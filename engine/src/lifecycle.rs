//! Engine lifecycle shared by native and network hosts. This module binds no listener.
use crate::AppState;
use std::sync::{Arc, atomic::Ordering};
pub async fn start(state: Arc<AppState>) {
    if state.runtime_started.swap(true, Ordering::SeqCst) {
        return;
    }
    if std::env::var("FLAMEL_PROVIDER_AUTO_UPDATE").as_deref() != Ok("0") {
        crate::provider_cli::start_updater();
    }

    let maintenance_state = Arc::clone(&state);
    tokio::spawn(async move {
        // Start shortly after the listener so old databases converge without
        // extending startup or blocking recovery of interrupted work.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        loop {
            if maintenance_state.is_shutting_down() {
                break;
            }
            let db = Arc::clone(&maintenance_state.db);
            let result = tokio::task::spawn_blocking(move || {
                let realtime = crate::db::realtime::maintain_realtime_journal(&db, 2_000);
                let context = crate::db::context::maintain_context_ledger(&db, 2_000);
                crate::db::realtime::checkpoint_realtime_wal(&db);
                (realtime, context)
            })
            .await;
            if let Ok((realtime, context)) = result
                && (realtime.superseded_messages > 0
                    || realtime.expired_events > 0
                    || realtime.expired_runs > 0
                    || context.archived_revisions > 0
                    || context.reclaimed_pages > 0)
            {
                tracing::info!(
                    superseded_messages = realtime.superseded_messages,
                    expired_events = realtime.expired_events,
                    expired_runs = realtime.expired_runs,
                    archived_context_revisions = context.archived_revisions,
                    reclaimed_database_pages = context.reclaimed_pages,
                    "trimmed transient persistence history"
                );
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });

    if std::env::var("FLAMEL_CATALOG_REFRESH").as_deref() == Ok("1") {
        let catalog_state = Arc::clone(&state);
        let mut catalog_shutdown = catalog_state.shutdown_receiver();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            loop {
                {
                    let _guard = catalog_state.runtime_catalog_sync.lock().await;
                    let status =
                        crate::db::runtime_catalog::runtime_catalog_status(&catalog_state.db);
                    let result = if status.frozen {
                        Ok(status)
                    } else {
                        match crate::catalog_trust::get_jwks(&catalog_state.jwks_cache).await {
                            Ok(jwks) => {
                                crate::db::runtime_catalog::sync_runtime_catalog(
                                    &catalog_state.db,
                                    &jwks,
                                )
                                .await
                            }
                            Err(error) => Err(error),
                        }
                    };
                    if let Err(error) = result {
                        crate::db::runtime_catalog::record_runtime_catalog_error(
                            &catalog_state.db,
                            &error,
                        );
                        tracing::warn!(%error, "Runtime Catalog refresh kept the last-known-good snapshot");
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(15 * 60)) => {}
                    changed = catalog_shutdown.changed() => {
                        if changed.is_err() || *catalog_shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
        });
    }
    // Reconcile only work left by the previous process, before serving can
    // admit a new request. Dispatch waits until the listener is being served.
    let interrupted = crate::db::realtime::reconcile_interrupted_runs(&state.db);
    crate::routes::chat::start_dispatcher(Arc::clone(&state));
    let recovery_state = Arc::clone(&state);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        crate::routes::ask::recover_answered_input_requests(Arc::clone(&recovery_state)).await;
        for run in interrupted {
            crate::routes::chat::submit_durable_chat_run(Arc::clone(&recovery_state), run).await;
        }
        // A process can exit after issuing a short live-delivery lease but before
        // acknowledging it. Recheck once after that lease expires.
        tokio::time::sleep(std::time::Duration::from_secs(
            crate::routes::ask::DELIVERY_LEASE_SECONDS as u64 + 1,
        ))
        .await;
        crate::routes::ask::recover_answered_input_requests(recovery_state).await;
    });

    crate::claude::agent_runner::start_agent_runner(Arc::clone(&state));
}

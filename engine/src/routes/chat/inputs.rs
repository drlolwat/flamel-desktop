use super::*;
use crate::claude::session_pool::SteeringResult;
use crate::db::chat_inputs::{self, InputAdmission};
use rusqlite::OptionalExtension;

pub async fn handle_inputs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Response {
    let Some(user) = crate::routes::admin::get_current_user_id(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if get_session_meta(&state.db, &session_id)
        .and_then(|s| s.user_id)
        .as_deref()
        != Some(&user)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match chat_inputs::list(&state.db, &session_id, &user) {
        Ok(inputs) => Json(json!({"inputs":inputs})).into_response(),
        Err(_) => storage_error(),
    }
}

fn storage_error() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"accepted":false,"code":"input_storage_unavailable",
        "error":"Your message could not be saved. It has not been sent; please retry."})),
    )
        .into_response()
}

fn settings_match(active: &Value, next: &Value) -> bool {
    [
        "workingDirectory",
        "model",
        "reasoningEffort",
        "permissionMode",
        "runnerId",
        "planningMode",
        "ponytailEnabled",
        "activePlanId",
    ]
    .iter()
    .all(|key| active.get(key) == next.get(key))
        && !next
            .get("images")
            .and_then(Value::as_array)
            .is_some_and(|images| !images.is_empty())
}

pub async fn handle_steer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if state.is_shutting_down() {
        return storage_error();
    }
    let Some(user) = crate::routes::admin::get_current_user_id(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(sid) = body
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(id) = body
        .get("requestId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 200)
        .map(str::to_owned)
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(session) =
        get_session_meta(&state.db, &sid).filter(|s| s.user_id.as_deref() == Some(&user))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if session
        .session_owner_kind
        .as_deref()
        .is_some_and(|kind| kind != "interactive")
        || body
            .get("message")
            .and_then(Value::as_str)
            .is_none_or(|message| message.trim().is_empty() || message.len() > 1_048_576)
        || body.get("workingDirectory").and_then(Value::as_str) != Some(&session.project_path)
    {
        return (StatusCode::BAD_REQUEST,Json(json!({"accepted":false,"error":"Use the current project and a non-empty message."}))).into_response();
    }
    match chat_inputs::admit(&state.db,&id,&sid,&user,&body) {
        Ok(InputAdmission::Existing(receipt)) => return (StatusCode::ACCEPTED,Json(receipt)).into_response(),
        Ok(InputAdmission::Conflict) => return (StatusCode::CONFLICT,Json(json!({"accepted":false,"code":"request_id_conflict","error":"This message ID already belongs to another input."}))).into_response(),
        Err(_) => return storage_error(),
        Ok(InputAdmission::New) => {}
    }
    let waiting = chat_inputs::receipt(
        &id,
        &sid,
        &body,
        "waiting",
        Some("Saved. Waiting to send your guidance."),
    );
    state.realtime.publish_value(&sid, None, &waiting);
    let (tx, rx) = tokio::sync::oneshot::channel();
    // The saved input outlives its HTTP connection. A disconnect never cancels
    // dispatch or asks the provider to execute it twice.
    tokio::spawn(async move {
        let active = {
            let conn = state.db.conn();
            conn.query_row(
                "SELECT id,request_json FROM chat_runs WHERE session_id=?1 AND user_id=?2
                AND state='running' ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![sid, user],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
        };
        let (delivery, note) = match active {
            Err(_) => (
                "failed",
                Some("Could not read the active work. Your message is saved; it was not sent."),
            ),
            Ok(active) => {
                let active = active.and_then(|(run_id, raw)| {
                    serde_json::from_str::<Value>(&raw)
                        .ok()
                        .map(|request| (run_id, request))
                });
                let target = if active
                    .as_ref()
                    .is_some_and(|(_, active)| settings_match(active, &body))
                {
                    state.session_pool.steering_target(&sid).await
                } else {
                    None
                };
                // Bind the pool snapshot to the run whose settings were checked.
                // The pool then rechecks the exact provider turn under its writer lock.
                let target = target.filter(|_| active.as_ref().is_some_and(|(run_id,_)| {
                    state.db.conn().query_row("SELECT EXISTS(SELECT 1 FROM chat_runs WHERE id=?1 AND session_id=?2 AND user_id=?3 AND state='running')",
                        rusqlite::params![run_id,sid,user],|row|row.get::<_,bool>(0)).unwrap_or(false)
                }));
                let result = if let Some(target) = target {
                    state
                        .session_pool
                        .steer_session(&sid, &target, &id, body["message"].as_str().unwrap())
                        .await
                } else {
                    SteeringResult::NotSent
                };
                match result {
                    SteeringResult::Sent => {
                        let saved = crate::db::sessions::try_persist_session_message(
                            &state.db,
                            &sid,
                            "user",
                            &json!({"role":"user",
                            "content":[{"type":"text","text":body["message"]}],
                            "flamel":{"requestId":id,"delivery":"steer"}})
                            .to_string(),
                        );
                        if saved.is_ok() {
                            ("sent", Some("Sent to the current run."))
                        } else {
                            (
                                "uncertain",
                                Some(
                                    "The provider received your guidance, but conversation history could not be updated. Your saved input is here; do not resend it.",
                                ),
                            )
                        }
                    }
                    SteeringResult::Uncertain => (
                        "uncertain",
                        Some(
                            "The connection changed before acknowledgement. Your message is saved; check the conversation before sending it again.",
                        ),
                    ),
                    SteeringResult::NotSent => {
                        match crate::db::realtime::queue_unsent_input(&state.db, &id, &sid, &user) {
                            Ok(next) => {
                                // Both the receipt and queued prompt are already
                                // durable before any provider can receive it.
                                if let Some(run) = next {
                                    super::submit_durable_chat_run(Arc::clone(&state), run).await;
                                }
                                (
                                    "queued",
                                    Some("Saved for the next turn; current work was not stopped."),
                                )
                            }
                            Err(error) => {
                                tracing::error!(%error, input_id=id, "unsent guidance could not be queued");
                                (
                                    "failed",
                                    Some(
                                        "Your guidance is saved but could not be queued. Check the active session before trying again.",
                                    ),
                                )
                            }
                        }
                    }
                }
            }
        };
        let receipt = if chat_inputs::complete(&state.db, &id, delivery, note).is_ok() {
            chat_inputs::receipt(&id, &sid, &body, delivery, note)
        } else {
            chat_inputs::receipt(
                &id,
                &sid,
                &body,
                "uncertain",
                Some(
                    "Delivery could not be recorded. Do not resend until you check the conversation.",
                ),
            )
        };
        state.realtime.publish_value(&sid, None, &receipt);
        let _ = tx.send(receipt);
    });
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(12), rx)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(waiting);
    (StatusCode::ACCEPTED, Json(receipt)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steering_never_changes_execution_settings() {
        let active =
            json!({"model":"gpt-6-astra","workingDirectory":"/a","permissionMode":"never"});
        assert!(settings_match(&active, &active));
        let mut changed = active.clone();
        changed["workingDirectory"] = json!("/b");
        assert!(!settings_match(&active, &changed));
        let mut changed = active.clone();
        changed["images"] = json!([{"data":"image"}]);
        assert!(!settings_match(&active, &changed));
    }
}

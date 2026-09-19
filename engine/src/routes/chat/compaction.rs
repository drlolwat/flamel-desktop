use super::*;

pub(super) async fn wait_for_compaction_or_abort<T, F>(
    future: F,
    abort_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<T, ()>
where
    F: Future<Output = T>,
{
    if *abort_rx.borrow() {
        return Err(());
    }

    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            changed = abort_rx.changed() => {
                match changed {
                    Ok(()) if *abort_rx.borrow_and_update() => return Err(()),
                    Ok(()) => continue,
                    Err(_) => return Ok(future.await),
                }
            }
        }
    }
}

pub(super) const MAX_ASSISTANT_ENVELOPES_PER_TURN: u32 = 1_000_000;
pub(super) const MAX_CONTENT_BLOCKS_PER_TURN: usize = 262_144;

pub(super) fn provider_output_exceeds_emergency_limit(
    assistant_envelopes: u32,
    content_blocks: usize,
) -> bool {
    assistant_envelopes > MAX_ASSISTANT_ENVELOPES_PER_TURN
        || content_blocks > MAX_CONTENT_BLOCKS_PER_TURN
}

pub(super) fn should_compact_before_turn(
    input_tokens: i64,
    model: Option<&str>,
    defer_proactive_compaction: bool,
) -> bool {
    // An explicit replacement prompt should interrupt immediately. Spending
    // 20-90 seconds summarizing the superseded turn before delivering the new
    // user message defeats interruption semantics. If the resumed provider is
    // genuinely out of room, the existing context-exhaustion recovery path
    // performs a checkpoint then.
    if defer_proactive_compaction {
        return false;
    }
    let threshold = crate::compaction::auto_compaction_threshold(model);
    crate::compaction::bounded_context_tokens(input_tokens, model) >= threshold
}

pub(super) fn is_provider_native_compaction_message(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some("system")
        && (matches!(
            message.get("status").and_then(Value::as_str),
            Some("compacting" | "compact_complete")
        ) || message.get("subtype").and_then(Value::as_str) == Some("compact_boundary"))
}

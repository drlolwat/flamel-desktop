//! Shared validation for persistable user-visible integration data.
use serde_json::Value;
pub fn validate_public_payload(value: &Value) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            for value in values {
                validate_public_payload(value)?;
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                if matches!(
                    normalized.as_str(),
                    "chain_of_thought"
                        | "hidden_reasoning"
                        | "raw_reasoning"
                        | "provider_token"
                        | "repository_token"
                ) {
                    return Err(format!(
                        "runtime event payload contains forbidden field {key:?}"
                    ));
                }
                validate_public_payload(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

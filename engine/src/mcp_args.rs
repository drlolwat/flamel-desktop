use serde_json::{Map, Number, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedToolArguments {
    pub arguments: Value,
    pub coerced_paths: Vec<String>,
}

pub fn normalize_tool_arguments(
    tool_name: &str,
    raw_arguments: Value,
    tools: &Value,
) -> Result<NormalizedToolArguments, String> {
    let Some(schema) = find_tool_input_schema(tool_name, tools) else {
        return Ok(NormalizedToolArguments {
            arguments: raw_arguments,
            coerced_paths: Vec::new(),
        });
    };

    let mut coerced_paths = Vec::new();
    let arguments = normalize_value(raw_arguments, schema, "", &mut coerced_paths)
        .map_err(|errors| format_argument_errors(tool_name, errors))?;
    coerced_paths.sort();
    coerced_paths.dedup();

    if !coerced_paths.is_empty() {
        log_normalization(tool_name, &coerced_paths);
    }

    Ok(NormalizedToolArguments {
        arguments,
        coerced_paths,
    })
}

fn find_tool_input_schema<'a>(tool_name: &str, tools: &'a Value) -> Option<&'a Value> {
    let tools = tools
        .as_array()
        .or_else(|| tools.get("tools").and_then(Value::as_array))?;
    tools.iter().find_map(|tool| {
        (tool.get("name").and_then(Value::as_str) == Some(tool_name))
            .then(|| tool.get("inputSchema").or_else(|| tool.get("input_schema")))
            .flatten()
    })
}

fn normalize_value(
    value: Value,
    schema: &Value,
    path: &str,
    coerced_paths: &mut Vec<String>,
) -> Result<Value, Vec<String>> {
    let mut errors = Vec::new();
    let normalized = if schema_is_object(schema) {
        normalize_object(value, schema, path, coerced_paths, &mut errors)
    } else if schema_allows_type(schema, "array") {
        normalize_array(value, schema, path, coerced_paths, &mut errors)
    } else if schema_allows_type(schema, "integer") {
        normalize_integer(value, path, coerced_paths, &mut errors)
    } else if schema_allows_type(schema, "number") {
        normalize_number(value, path, coerced_paths, &mut errors)
    } else if schema_allows_type(schema, "boolean") {
        normalize_boolean(value, path, coerced_paths, &mut errors)
    } else if schema_allows_type(schema, "string") {
        normalize_string(value, path, &mut errors)
    } else {
        value
    };

    if errors.is_empty() {
        validate_enum(&normalized, schema, path, &mut errors);
    }

    if errors.is_empty() {
        Ok(normalized)
    } else {
        Err(errors)
    }
}

fn normalize_object(
    value: Value,
    schema: &Value,
    path: &str,
    coerced_paths: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Value {
    let mut object = match value {
        Value::Object(object) => object,
        Value::Null if path.is_empty() => Map::new(),
        Value::String(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(object)) => {
                coerced_paths.push(path_label(path).to_string());
                object
            }
            _ => {
                errors.push(format!("{} must be an object", path_label(path)));
                return Value::Null;
            }
        },
        _ => {
            errors.push(format!("{} must be an object", path_label(path)));
            return Value::Null;
        }
    };

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        let keys: Vec<String> = properties.keys().cloned().collect();
        for key in keys {
            let Some(value) = object.remove(&key) else {
                continue;
            };
            let property_path = append_path(path, &key);
            match normalize_value(value, &properties[&key], &property_path, coerced_paths) {
                Ok(value) => {
                    object.insert(key, value);
                }
                Err(mut nested) => errors.append(&mut nested),
            }
        }
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(field) {
                errors.push(format!("{} is required", append_path(path, field)));
            }
        }
    }

    Value::Object(object)
}

fn normalize_array(
    value: Value,
    schema: &Value,
    path: &str,
    coerced_paths: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Value {
    let mut values = match value {
        Value::Array(values) => values,
        Value::String(text) => {
            if let Ok(Value::Array(values)) = serde_json::from_str::<Value>(&text) {
                coerced_paths.push(path_label(path).to_string());
                values
            } else if schema
                .get("items")
                .is_some_and(|items| schema_allows_type(items, "string"))
            {
                coerced_paths.push(path_label(path).to_string());
                vec![Value::String(text)]
            } else {
                errors.push(format!("{} must be an array", path_label(path)));
                return Value::Null;
            }
        }
        _ => {
            errors.push(format!("{} must be an array", path_label(path)));
            return Value::Null;
        }
    };

    if let Some(item_schema) = schema.get("items") {
        for (index, item) in values.iter_mut().enumerate() {
            let item_path = format!("{}[{}]", path_label(path), index);
            match normalize_value(item.take(), item_schema, &item_path, coerced_paths) {
                Ok(value) => *item = value,
                Err(mut nested) => errors.append(&mut nested),
            }
        }
    }

    Value::Array(values)
}

fn normalize_integer(
    value: Value,
    path: &str,
    coerced_paths: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Value {
    match value {
        Value::Number(number) if number.as_i64().is_some() || number.as_u64().is_some() => {
            Value::Number(number)
        }
        Value::Number(number) => match number.as_f64().and_then(integer_number_from_f64) {
            Some(number) => {
                coerced_paths.push(path_label(path).to_string());
                Value::Number(number)
            }
            None => {
                errors.push(format!("{} must be an integer", path_label(path)));
                Value::Null
            }
        },
        Value::String(text) => match parse_integer_string(&text) {
            Some(number) => {
                coerced_paths.push(path_label(path).to_string());
                Value::Number(number)
            }
            None => {
                errors.push(format!("{} must be an integer", path_label(path)));
                Value::Null
            }
        },
        _ => {
            errors.push(format!("{} must be an integer", path_label(path)));
            Value::Null
        }
    }
}

fn normalize_number(
    value: Value,
    path: &str,
    coerced_paths: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Value {
    match value {
        Value::Number(number) if number.as_f64().is_some_and(f64::is_finite) => {
            Value::Number(number)
        }
        Value::String(text) => match text.parse::<f64>().ok().filter(|value| value.is_finite()) {
            Some(value) => match Number::from_f64(value) {
                Some(number) => {
                    coerced_paths.push(path_label(path).to_string());
                    Value::Number(number)
                }
                None => {
                    errors.push(format!("{} must be a finite number", path_label(path)));
                    Value::Null
                }
            },
            None => {
                errors.push(format!("{} must be a finite number", path_label(path)));
                Value::Null
            }
        },
        _ => {
            errors.push(format!("{} must be a finite number", path_label(path)));
            Value::Null
        }
    }
}

fn normalize_boolean(
    value: Value,
    path: &str,
    coerced_paths: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Value {
    match value {
        Value::Bool(_) => value,
        Value::String(text) if text.eq_ignore_ascii_case("true") => {
            coerced_paths.push(path_label(path).to_string());
            Value::Bool(true)
        }
        Value::String(text) if text.eq_ignore_ascii_case("false") => {
            coerced_paths.push(path_label(path).to_string());
            Value::Bool(false)
        }
        _ => {
            errors.push(format!("{} must be a boolean", path_label(path)));
            Value::Null
        }
    }
}

fn normalize_string(value: Value, path: &str, errors: &mut Vec<String>) -> Value {
    if value.is_string() {
        value
    } else {
        errors.push(format!("{} must be a string", path_label(path)));
        Value::Null
    }
}

fn validate_enum(value: &Value, schema: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(values) = schema.get("enum").and_then(Value::as_array) else {
        return;
    };
    if values.iter().any(|allowed| allowed == value) {
        return;
    }
    let allowed = values
        .iter()
        .filter_map(Value::as_str)
        .take(12)
        .collect::<Vec<_>>()
        .join(", ");
    if allowed.is_empty() {
        errors.push(format!("{} has an unsupported value", path_label(path)));
    } else {
        errors.push(format!("{} must be one of: {}", path_label(path), allowed));
    }
}

fn schema_is_object(schema: &Value) -> bool {
    schema_allows_type(schema, "object") || schema.get("properties").is_some()
}

fn schema_allows_type(schema: &Value, target: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(value)) => value == target,
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(target)),
        _ => false,
    }
}

fn parse_integer_string(text: &str) -> Option<Number> {
    text.parse::<i64>()
        .ok()
        .map(Number::from)
        .or_else(|| text.parse::<u64>().ok().map(Number::from))
        .or_else(|| text.parse::<f64>().ok().and_then(integer_number_from_f64))
}

fn integer_number_from_f64(value: f64) -> Option<Number> {
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }
    if value >= 0.0 && value <= u64::MAX as f64 {
        Some(Number::from(value as u64))
    } else if value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        Some(Number::from(value as i64))
    } else {
        None
    }
}

fn append_path(path: &str, field: &str) -> String {
    if path.is_empty() {
        field.to_string()
    } else {
        format!("{}.{}", path, field)
    }
}

fn path_label(path: &str) -> &str {
    if path.is_empty() { "arguments" } else { path }
}

fn format_argument_errors(tool_name: &str, errors: Vec<String>) -> String {
    let mut message = format!("Invalid arguments for tool `{}`", sanitize_name(tool_name));
    if !errors.is_empty() {
        message.push_str(": ");
        message.push_str(&errors.join("; "));
    }
    message
}

fn log_normalization(tool_name: &str, paths: &[String]) {
    eprintln!(
        "flamel_mcp_args_normalized tool={} paths={}",
        sanitize_name(tool_name),
        paths.join(",")
    );
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_tools() -> Value {
        json!([
            {
                "name": "sample",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": {"type": "integer"},
                        "enabled": {"type": "boolean"},
                        "labels": {"type": "array", "items": {"type": "string"}},
                        "mode": {"type": "string", "enum": ["fast", "safe"]}
                    },
                    "required": ["limit", "mode"]
                }
            }
        ])
    }

    #[test]
    fn coerces_provider_numbers_and_strings_using_schema() {
        let normalized = normalize_tool_arguments(
            "sample",
            json!({"limit": 3.0, "enabled": "true", "labels": "bug", "mode": "fast"}),
            &test_tools(),
        )
        .expect("normalized");

        assert_eq!(normalized.arguments["limit"], json!(3));
        assert_eq!(normalized.arguments["enabled"], json!(true));
        assert_eq!(normalized.arguments["labels"], json!(["bug"]));
        assert_eq!(normalized.coerced_paths, vec!["enabled", "labels", "limit"]);
    }

    #[test]
    fn accepts_top_level_json_object_string() {
        let normalized = normalize_tool_arguments(
            "sample",
            Value::String("{\"limit\":\"2\",\"mode\":\"safe\"}".to_string()),
            &test_tools(),
        )
        .expect("normalized");

        assert_eq!(normalized.arguments["limit"], json!(2));
        assert_eq!(normalized.arguments["mode"], json!("safe"));
        assert!(normalized.coerced_paths.contains(&"arguments".to_string()));
    }

    #[test]
    fn rejects_fractional_integer_and_missing_required_without_echoing_values() {
        let error = normalize_tool_arguments(
            "sample",
            json!({"limit": 1.25, "password": "do-not-leak"}),
            &test_tools(),
        )
        .expect_err("invalid");

        assert!(error.contains("limit must be an integer"));
        assert!(error.contains("mode is required"));
        assert!(!error.contains("do-not-leak"));
    }

    #[test]
    fn validates_enum_after_coercion() {
        let error = normalize_tool_arguments(
            "sample",
            json!({"limit": "5", "mode": "reckless"}),
            &test_tools(),
        )
        .expect_err("invalid enum");

        assert!(error.contains("mode must be one of: fast, safe"));
        assert!(!error.contains("reckless"));
    }
}

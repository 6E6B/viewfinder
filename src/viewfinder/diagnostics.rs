//! Bounded, structural HTTP diagnostics. Never dump headers, cookies, form
//! variables, page HTML, or arbitrary response text (including error messages).
use serde_json::{Value, json};

pub(super) fn enabled() -> bool {
    std::env::var("VIEWFINDER_HTTP_DEBUG").as_deref() == Ok("1")
}

pub(super) fn path(path: &str) -> String {
    path.split('/')
        .map(|part| {
            if part.bytes().any(|b| b.is_ascii_digit()) {
                ":id"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn gateway_error(value: &Value) -> bool {
    match &value["error"] {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_u64() != Some(0),
        Value::String(text) => !text.is_empty() && text != "0",
        _ => true,
    }
}

pub(super) fn gateway_error_code(value: &Value) -> Option<u64> {
    value["error"]
        .as_u64()
        .or_else(|| value["error"].as_str()?.parse().ok())
        .filter(|code| *code != 0)
}

fn safe_error_text(text: &str) -> String {
    text.split_whitespace()
        .take(40)
        .map(|word| {
            let trimmed = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            if word.contains("://")
                || trimmed.len() > 40
                || (trimmed.len() >= 4 && trimmed.bytes().all(|b| b.is_ascii_digit()))
            {
                "<redacted>".to_owned()
            } else {
                word.chars()
                    .filter(|c| c.is_ascii() && !c.is_control())
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn shape(value: &Value) -> Value {
    fn walk(value: &Value, key: &str, depth: usize, budget: &mut usize) -> Value {
        if depth > 7 || *budget == 0 {
            return json!("<truncated>");
        }
        *budget -= 1;
        match value {
            Value::Null | Value::Bool(_) => value.clone(),
            Value::Number(_) if matches!(key, "error" | "code" | "error_code") => value.clone(),
            Value::Number(_) => json!("<number>"),
            Value::String(text) if key == "status" && matches!(text.as_str(), "ok" | "fail") => {
                value.clone()
            }
            Value::String(text)
                if key == "__typename"
                    && text.len() <= 100
                    && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') =>
            {
                value.clone()
            }
            Value::String(text)
                if matches!(key, "message" | "errorSummary" | "errorDescription") =>
            {
                json!(safe_error_text(text))
            }
            Value::String(_) => json!("<string withheld>"),
            Value::Array(values) => json!({
                "length": values.len(),
                "sample": values.iter().take(2).map(|v| walk(v, key, depth + 1, budget)).collect::<Vec<_>>()
            }),
            Value::Object(fields) => {
                let mut result = serde_json::Map::new();
                for (key, value) in fields.iter().take(24) {
                    // Dynamic keys may themselves contain IDs or user content.
                    let safe_key = if key.len() <= 100
                        && key.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_')
                    {
                        key.as_str()
                    } else {
                        "<key withheld>"
                    };
                    result.insert(safe_key.into(), walk(value, key, depth + 1, budget));
                    if *budget == 0 {
                        break;
                    }
                }
                if fields.len() > result.len() {
                    result.insert("<truncated>".into(), json!(true));
                }
                Value::Object(result)
            }
        }
    }
    walk(value, "", 0, &mut 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_preserve_failure_shape_without_private_values() {
        let response = json!({
            "error": 1357004,
            "errorDescription": "Request 987654321 failed at https://secret.test/private with TOKEN0123456789012345678901234567890123456789",
            "data": {"xig_media_like": {"media": {"has_liked": false, "id": 987654321}}},
            "sessionid": "secret-cookie",
            "messages": [{"text": "private message"}],
            "12345678": "dynamic-key"
        });
        let summary = shape(&response);
        assert_eq!(summary["error"], 1357004);
        assert_eq!(
            summary["data"]["xig_media_like"]["media"]["has_liked"],
            false
        );
        let output = summary.to_string();
        for private in [
            "987654321",
            "https://secret.test/private",
            "TOKEN0123456789012345678901234567890123456789",
            "secret-cookie",
            "private message",
            "987654321",
            "12345678",
            "dynamic-key",
        ] {
            assert!(!output.contains(private));
        }
        assert_eq!(
            path("/api/v1/media/123_456/like/"),
            "/api/:id/media/:id/like/"
        );
        assert_eq!(
            safe_error_text(
                "Invalid doc 12345678 at https://example.test/private TOKEN0123456789012345678901234567890123456789"
            ),
            "Invalid doc <redacted> at <redacted> <redacted>"
        );
    }

    #[test]
    fn detects_http_success_gateway_errors() {
        for response in [
            json!({"error":1357004}),
            json!({"error":"1357004"}),
            json!({"error":true}),
        ] {
            assert!(gateway_error(&response));
        }
        for response in [
            json!({}),
            json!({"error":0}),
            json!({"error":false}),
            json!({"error":"0"}),
        ] {
            assert!(!gateway_error(&response));
        }
        assert_eq!(
            gateway_error_code(&json!({"error": 1357054})),
            Some(1357054)
        );
        assert_eq!(gateway_error_code(&json!({"error": "42"})), Some(42));
        assert_eq!(gateway_error_code(&json!({"error": false})), None);
    }
}

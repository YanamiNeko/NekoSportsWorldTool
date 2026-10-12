//! 日志文本处理：着色前缀剥离、敏感字段脱敏与关键词归类，GUI/CLI 共用。

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogKind {
    Plain,
    Ok,
    Err,
    Warn,
}

/// 归类并剥离前缀：入站 √/×/⚠ 仅用于着色，不进入展示文本。
pub fn classify(t: &str) -> (LogKind, &str) {
    if let Some(rest) = t.strip_prefix('×') {
        return (LogKind::Err, rest.trim_start());
    }
    if let Some(rest) = t.strip_prefix('⚠') {
        return (LogKind::Warn, rest.trim_start());
    }
    if let Some(rest) = t.strip_prefix('√') {
        return (LogKind::Ok, rest.trim_start());
    }
    if t.contains("失败") || t.contains("错误") {
        return (LogKind::Err, t);
    }
    if t.contains("成功") {
        return (LogKind::Ok, t);
    }
    (LogKind::Plain, t)
}

const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "password",
    "passwd",
    "tokenSign",
    "passToken",
    "captchaOutput",
    "deviceId",
    "customDeviceId",
    "androidId",
    "wifiMac",
    "blMac",
    "macAddress",
    "imei",
    "idfa",
    "account",
    "username",
    "token",
    "uid",
    "session",
];

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn sensitive_key(key: &str) -> bool {
    let normalized = normalized_key(key);
    SENSITIVE_KEYS
        .iter()
        .any(|candidate| normalized == normalized_key(candidate))
}

fn redact_json(value: Value) -> Value {
    match value {
        Value::Object(mut object) => {
            for (key, value) in object.iter_mut() {
                if sensitive_key(key) {
                    *value = Value::String("***".into());
                } else {
                    *value = redact_json(std::mem::take(value));
                }
            }
            Value::Object(object)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(redact_json).collect()),
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                match parsed {
                    Value::String(inner) => Value::String(redact_pairs(&inner)),
                    other => serde_json::to_string(&redact_json(other))
                        .map(Value::String)
                        .unwrap_or_else(|_| Value::String("***".into())),
                }
            } else {
                Value::String(redact_pairs(&text))
            }
        }
        other => other,
    }
}

fn boundary(byte: Option<u8>) -> bool {
    !matches!(byte, Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_'))
}

/// Redact key/value pairs in non-JSON diagnostics such as `token=... uid=...`.
fn redact_pairs(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pos = 0;
    while pos < input.len() {
        let mut matched = None;
        for key in SENSITIVE_KEYS {
            let Some(candidate) = input.get(pos..pos + key.len()) else {
                continue;
            };
            if !candidate.eq_ignore_ascii_case(key)
                || !boundary(
                    pos.checked_sub(1)
                        .and_then(|i| input.as_bytes().get(i).copied()),
                )
                || !boundary(input.as_bytes().get(pos + key.len()).copied())
            {
                continue;
            }
            let mut delimiter = pos + key.len();
            if matches!(input.as_bytes().get(delimiter), Some(b'"' | b'\'')) {
                delimiter += 1;
            }
            while delimiter < input.len() && input.as_bytes()[delimiter].is_ascii_whitespace() {
                delimiter += 1;
            }
            if delimiter >= input.len() || !matches!(input.as_bytes()[delimiter], b':' | b'=') {
                continue;
            }
            let mut value_start = delimiter + 1;
            while value_start < input.len() && input.as_bytes()[value_start].is_ascii_whitespace() {
                value_start += 1;
            }
            matched = Some((value_start, normalized_key(key) == "authorization"));
            break;
        }

        let Some((value_start, is_authorization)) = matched else {
            let ch = input[pos..].chars().next().expect("valid UTF-8");
            out.push(ch);
            pos += ch.len_utf8();
            continue;
        };

        out.push_str(&input[pos..value_start]);
        if let Some(&quote @ (b'"' | b'\'')) = input.as_bytes().get(value_start) {
            out.push(quote as char);
            let mut end = value_start + 1;
            let mut escaped = false;
            while end < input.len() {
                let byte = input.as_bytes()[end];
                if byte == quote && !escaped {
                    break;
                }
                escaped = byte == b'\\' && !escaped;
                if byte != b'\\' {
                    escaped = false;
                }
                end += 1;
            }
            out.push_str("***");
            if end < input.len() {
                out.push(quote as char);
                pos = end + 1;
            } else {
                pos = input.len();
            }
        } else {
            let mut end = value_start;
            while end < input.len()
                && !matches!(
                    input.as_bytes()[end],
                    b',' | b';' | b'&' | b'\t' | b'\r' | b'\n' | b'}' | b']'
                )
                && (is_authorization || input.as_bytes()[end] != b' ')
            {
                end += 1;
            }
            out.push_str("***");
            pos = end;
        }
    }
    out
}

/// Redact credentials and device identifiers from JSON or key/value diagnostics.
pub fn redact_text(t: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(t) {
        return serde_json::to_string(&redact_json(value)).unwrap_or_else(|_| "***".into());
    }
    redact_pairs(t)
}

/// 只取展示文本（CLI 输出用），同时确保不会泄露敏感字段。
pub fn clean(t: &str) -> String {
    redact_text(classify(t).1)
}

/// 按字符截断（字节切片会劈开多字节字符导致 panic）。
pub fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 多字节字符中间截断不允许 panic。
    #[test]
    fn test_truncate_multibyte() {
        let s = "中文字符串测试";
        for n in 0..=s.len() + 4 {
            let out = truncate(s, n);
            assert!(out.chars().count() <= n.max(0));
        }
        assert_eq!(truncate("中文abc", 4), "中文ab");
    }

    #[test]
    fn redact_nested_json_credentials_and_device_identity() {
        let raw =
            r#"{"token":"secret","profile":{"uid":123,"name":"ok"},"items":[{"deviceId":"abc"}]}"#;
        let redacted = redact_text(raw);
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("123"));
        assert!(!redacted.contains("abc"));
        assert!(redacted.contains("\"name\":\"ok\""));
    }

    #[test]
    fn redact_unstructured_key_value_diagnostics() {
        let redacted = redact_text("token=secret uid:123 DeviceId=abc name=ok");
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("123"));
        assert!(!redacted.contains("abc"));
        assert!(redacted.contains("name=ok"));

        let truncated_json = redact_text(r#"{"token":"secret""#);
        assert!(!truncated_json.contains("secret"));
    }

    #[test]
    fn redact_nested_json_strings_and_authorization_values() {
        let nested = redact_text(r#"{"data":"{\"token\":\"nested-secret\",\"uid\":123}"}"#);
        assert!(!nested.contains("nested-secret"));
        assert!(!nested.contains("123"));

        let authorization = redact_text("authorization=Bearer SYNTHETIC_TOKEN WITH_SPACES; status=ok");
        assert!(!authorization.contains("SYNTHETIC_TOKEN"));
        assert!(authorization.contains("status=ok"));
    }

    #[test]
    fn clean_removes_prefix_and_redacts() {
        assert_eq!(clean("√ token=secret"), "token=***");
    }
}

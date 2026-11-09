use super::ResourceKind;
use serde::Serialize;
use serde_json::Value;

const TOTAL_BYTES: usize = 8 * 1024;
const FIELD_BYTES: usize = 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventDiagnostic {
    pub message: String,
    pub truncated: bool,
    pub source: Option<String>,
    pub field_path: Option<String>,
}

// Recognizable credential syntax only, never a guarantee about arbitrary server prose.
pub(super) fn credential_value(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if lower.contains("-----begin") && lower.contains("private key-----") {
        return true;
    }
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .filter(|s| !s.is_empty())
        .collect();
    if words.windows(2).any(|w| matches!(w[0], "bearer" | "basic")) {
        return true;
    }
    for (start, _) in lower.char_indices() {
        if start > 0 && lower.as_bytes()[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        for name in [
            "password",
            "passwd",
            "pwd",
            "token",
            "access_token",
            "client_secret",
            "authorization",
            "credential",
            "private_key",
            "api_key",
        ] {
            if let Some(tail) = lower[start..].strip_prefix(name)
                && tail
                    .trim_start_matches([' ', '\t', '\"', '\''])
                    .starts_with(['=', ':'])
            {
                return true;
            }
        }
    }
    false
}

fn clean(c: char) -> bool {
    (!c.is_control() || matches!(c, '\n' | '\t'))
        && !matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn bounded(s: &str, limit: usize, truncated: &mut bool) -> String {
    let mut output = String::new();
    let mut private_key = false;
    // Redact before truncation: a cap must not cut the credential indicator off its value.
    for (index, line) in s.split('\n').enumerate() {
        let line: String = line.chars().filter(|c| clean(*c)).collect();
        let lower = line.to_ascii_lowercase();
        let begins_key = lower.contains("-----begin") && lower.contains("private key-----");
        let ends_key = lower.contains("-----end") && lower.contains("private key-----");
        let redact = private_key || begins_key || credential_value(&line);
        private_key = (private_key || begins_key) && !ends_key;
        let line = if redact { "[redacted]" } else { &line };
        for c in (index > 0).then_some('\n').into_iter().chain(line.chars()) {
            if output.len() + c.len_utf8() > limit {
                *truncated = true;
                return output;
            }
            output.push(c);
        }
    }
    output
}

fn optional(value: Option<&Value>, truncated: &mut bool) -> Option<String> {
    let value = value.and_then(Value::as_str)?;
    let value = bounded(value, FIELD_BYTES, truncated);
    (!value.is_empty()).then_some(value)
}

pub(crate) fn project(kind: ResourceKind, value: &Value) -> Option<EventDiagnostic> {
    if kind != ResourceKind::Events {
        return None;
    }
    let mut truncated = false;
    let source = optional(value.get("reportingComponent"), &mut truncated)
        .or_else(|| optional(value.pointer("/source/component"), &mut truncated));
    let field_path = optional(value.pointer("/involvedObject/fieldPath"), &mut truncated);
    let remaining = TOTAL_BYTES
        - source.as_ref().map_or(0, String::len)
        - field_path.as_ref().map_or(0, String::len);
    let message = bounded(
        value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        remaining,
        &mut truncated,
    );
    Some(EventDiagnostic {
        message,
        truncated,
        source,
        field_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_only_and_allowlisted_fields() {
        for kind in [
            ResourceKind::Pods,
            ResourceKind::Secrets,
            ResourceKind::ConfigMaps,
            ResourceKind::Nodes,
        ] {
            assert!(project(kind, &json!({"message": "never"})).is_none());
        }
        let diagnostic = project(ResourceKind::Events, &json!({"message": "a".repeat(500), "reportingComponent": "scheduler", "source": {"component": "legacy", "host": "not copied"}, "involvedObject": {"fieldPath": "spec.containers{api}"}, "data": {"token": "never"}, "metadata": {"annotations": {"secret": "never"}}})).unwrap();
        assert_eq!(diagnostic.message.len(), 500);
        assert!(!diagnostic.truncated);
        assert_eq!(diagnostic.source.as_deref(), Some("scheduler"));
        let wire = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(wire.as_object().unwrap().len(), 4);
        assert_eq!(wire["fieldPath"], "spec.containers{api}");
        assert!(!wire.to_string().contains("never"));
        assert!(!wire.to_string().contains("not copied"));
    }

    #[test]
    fn utf8_and_cumulative_budget_are_bounded() {
        let diagnostic = project(ResourceKind::Events, &json!({"message": "😀".repeat(3000), "reportingComponent": "é".repeat(600), "involvedObject": {"fieldPath": "f".repeat(1200)}})).unwrap();
        assert!(diagnostic.truncated);
        assert_eq!(diagnostic.source.as_ref().unwrap().len(), FIELD_BYTES);
        assert_eq!(diagnostic.field_path.as_ref().unwrap().len(), FIELD_BYTES);
        assert_eq!(
            diagnostic.message.len()
                + diagnostic.source.unwrap().len()
                + diagnostic.field_path.unwrap().len(),
            TOTAL_BYTES
        );
        let exact = project(
            ResourceKind::Events,
            &json!({"message": "a".repeat(TOTAL_BYTES)}),
        )
        .unwrap();
        assert!(!exact.truncated);
        let over = project(
            ResourceKind::Events,
            &json!({"message": "a".repeat(TOTAL_BYTES + 1)}),
        )
        .unwrap();
        assert!(over.truncated);
    }

    #[test]
    fn preserves_multiline_errors_but_removes_controls_and_bidi() {
        let diagnostic = project(ResourceKind::Events, &json!({"message": "Failed\r\n\tvolume\u{0000}\u{001b}\u{061c}\u{202e}\u{2066} mount\nRetry"})).unwrap();
        assert_eq!(diagnostic.message, "Failed\n\tvolume mount\nRetry");
        assert!(!diagnostic.truncated);
    }

    #[test]
    fn redacts_entire_credential_lines_and_private_key_bodies() {
        let diagnostic = project(ResourceKind::Events, &json!({"message": "Failed mount\nAuthorization: Bearer fixture\n{\"client_secret\":\"fixture\"}\npassword = fixture\n-----BEGIN RSA PRIVATE KEY-----\nfixture-body\n-----END RSA PRIVATE KEY-----\nretry", "reportingComponent": "Basic fixture", "involvedObject": {"fieldPath": "token=fixture"}})).unwrap();
        assert_eq!(
            diagnostic.message,
            "Failed mount\n[redacted]\n[redacted]\n[redacted]\n[redacted]\n[redacted]\n[redacted]\nretry"
        );
        assert_eq!(diagnostic.source.as_deref(), Some("[redacted]"));
        assert_eq!(diagnostic.field_path.as_deref(), Some("[redacted]"));
        assert!(!diagnostic.truncated);
    }

    #[test]
    fn redaction_precedes_limit_and_control_removal_precedes_detection() {
        let diagnostic = project(
            ResourceKind::Events,
            &json!({"message": format!("{}password=fixture", "x ".repeat(5000))}),
        )
        .unwrap();
        assert_eq!(diagnostic.message, "[redacted]");
        let diagnostic = project(
            ResourceKind::Events,
            &json!({"message": "pass\u{202e}word=fixture"}),
        )
        .unwrap();
        assert_eq!(diagnostic.message, "[redacted]");
    }

    #[test]
    fn fallback_and_missing_message_are_explicit() {
        let diagnostic = project(
            ResourceKind::Events,
            &json!({"message": {}, "reportingComponent": "", "source": {"component": "legacy"}}),
        )
        .unwrap();
        assert_eq!(diagnostic.message, "");
        assert_eq!(diagnostic.source.as_deref(), Some("legacy"));
        assert_eq!(diagnostic.field_path, None);
        assert!(!diagnostic.truncated);
    }
}

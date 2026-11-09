use super::ResourceKind;
use serde::Serialize;
use serde_json::Value;

const TEXT_BYTES: usize = 1024;
const TOTAL_BYTES: usize = 32 * 1024;
const FIELD_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetailMetadata {
    pub labels: Vec<MetadataEntry>,
    pub annotations: Vec<MetadataEntry>,
    pub finalizers: Vec<String>,
    pub managed_fields: Vec<ManagedField>,
    pub deletion_timestamp: Option<String>,
    pub partial: bool,
}
fn timestamp(value: Option<&Value>, partial: &mut bool) -> Option<String> {
    let s = optional(value, partial)?;
    let b = s.as_bytes();
    let fixed = b.len() >= 20
        && b[4] == b'-'
        && b[7] == b'-'
        && matches!(b[10], b'T' | b't')
        && b[13] == b':'
        && b[16] == b':'
        && [0..4, 5..7, 8..10, 11..13, 14..16, 17..19]
            .iter()
            .all(|r| b[r.clone()].iter().all(u8::is_ascii_digit))
        && &s[17..19] <= "59";
    let mut end = 19;
    if fixed && b.get(end) == Some(&b'.') {
        end += 1;
        let start = end;
        while b.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if !(1..=9).contains(&(end - start)) {
            *partial = true;
            return None;
        }
    }
    let zone = b.get(end..).unwrap_or_default();
    let valid_zone = matches!(zone, [b'Z' | b'z'])
        || (zone.len() == 6
            && matches!(zone[0], b'+' | b'-')
            && zone[3] == b':'
            && zone[1..3].iter().all(u8::is_ascii_digit)
            && zone[4..6].iter().all(u8::is_ascii_digit)
            && &zone[1..3] <= b"23".as_slice()
            && &zone[4..6] <= b"59".as_slice());
    if fixed && valid_zone && s.parse::<jiff::Timestamp>().is_ok() {
        Some(s)
    } else {
        *partial = true;
        None
    }
}
// Heuristics catch recognizable credentials, not every possible secret in free text.
fn credential_value(s: &str) -> bool {
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
        let rest = &lower[start..];
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
            if let Some(tail) = rest.strip_prefix(name) {
                let tail = tail.trim_start_matches([' ', '\t', '\"', '\'']);
                if tail.starts_with(['=', ':']) {
                    return true;
                }
            }
        }
    }
    false
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedField {
    pub manager: String,
    pub operation: String,
    pub api_version: Option<String>,
    pub time: Option<String>,
    pub subresource: Option<String>,
    /// JSON array of ownership paths, never the original fieldsV1 payload.
    pub fields: String,
}
fn safe(s: &str) -> bool {
    s.len() <= TEXT_BYTES
        && !credential_value(s)
        && !s.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}
fn denied_annotation(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "token",
        "password",
        "credential",
        "secret",
        "authorization",
        "private",
        "config",
        "last-applied",
        "last_applied",
        "lastapplied",
    ]
    .iter()
    .any(|fragment| key.contains(fragment))
}
fn optional(value: Option<&Value>, partial: &mut bool) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() && safe(s) => Some(s.clone()),
        _ => {
            *partial = true;
            None
        }
    }
}
fn entries(value: Option<&Value>, limit: usize, partial: &mut bool) -> Vec<MetadataEntry> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(map) = value.as_object() else {
        *partial = true;
        return Vec::new();
    };
    let mut result = Vec::new();
    for (key, value) in map {
        if result.len() == limit || key.is_empty() || !safe(key) || denied_annotation(key) {
            *partial = true;
            continue;
        }
        if let Some(value) = value.as_str().filter(|s| safe(s)) {
            result.push(MetadataEntry {
                key: key.clone(),
                value: value.into(),
            });
        } else {
            *partial = true;
        }
    }
    result
}

// Only empty-object leaves express ownership. k:/v: content is an identity,
// not a field name, so it is never copied into the display projection.
fn ownership(
    value: &Value,
    path: &mut Vec<String>,
    paths: &mut Vec<Vec<String>>,
    nodes: &mut usize,
    partial: &mut bool,
) {
    *nodes += 1;
    if *nodes > 256 || path.len() > 12 {
        *partial = true;
        return;
    }
    let Some(map) = value.as_object() else {
        *partial = true;
        return;
    };
    if map.is_empty() {
        if !path.is_empty() {
            paths.push(path.clone());
        }
        return;
    }
    for (key, child) in map {
        if *nodes >= 256 {
            *partial = true;
            break;
        }
        if key == "." {
            if child.as_object().is_some_and(|m| m.is_empty()) {
                if !path.is_empty() {
                    paths.push(path.clone());
                }
            } else {
                *partial = true;
            }
            *nodes += 1;
            continue;
        }
        let segment = if key.starts_with("k:") || key.starts_with("v:") {
            *partial = true;
            "[keyed item]"
        } else if let Some(field) = key.strip_prefix("f:").filter(|s| {
            !s.is_empty()
                && safe(s)
                && !denied_annotation(s)
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b))
        }) {
            field
        } else {
            *partial = true;
            continue;
        };
        if path.len() == 12 {
            *partial = true;
            continue;
        }
        path.push(segment.into());
        ownership(child, path, paths, nodes, partial);
        path.pop();
    }
}
fn managed(value: Option<&Value>, partial: &mut bool) -> Vec<ManagedField> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(list) = value.as_array() else {
        *partial = true;
        return Vec::new();
    };
    if list.len() > 8 {
        *partial = true;
    }
    list.iter()
        .take(8)
        .filter_map(|entry| {
            let Some(map) = entry.as_object() else {
                *partial = true;
                return None;
            };
            let manager = optional(map.get("manager"), partial).filter(|s| !s.is_empty());
            let operation = optional(map.get("operation"), partial).filter(|s| !s.is_empty());
            let (Some(manager), Some(operation)) = (manager, operation) else {
                *partial = true;
                return None;
            };
            let mut paths = Vec::new();
            if map.get("fieldsType").and_then(Value::as_str) == Some("FieldsV1") {
                if let Some(tree) = map.get("fieldsV1") {
                    ownership(tree, &mut Vec::new(), &mut paths, &mut 0, partial);
                } else {
                    *partial = true;
                }
            } else {
                *partial = true;
            }
            paths.sort();
            paths.dedup();
            let mut fields = serde_json::to_string(&paths).expect("ownership paths serialize");
            while fields.len() > FIELD_BYTES {
                *partial = true;
                paths.pop();
                fields = serde_json::to_string(&paths).expect("ownership paths serialize");
            }
            Some(ManagedField {
                manager,
                operation,
                api_version: optional(map.get("apiVersion"), partial),
                time: timestamp(map.get("time"), partial),
                subresource: optional(map.get("subresource"), partial),
                fields,
            })
        })
        .collect()
}
pub(super) fn project(kind: ResourceKind, value: &Value) -> Option<DetailMetadata> {
    if matches!(kind, ResourceKind::Secrets | ResourceKind::ConfigMaps) {
        return None;
    }
    let mut partial = false;
    let meta = value.get("metadata").and_then(Value::as_object);
    if meta.is_none() {
        partial = true;
    }
    let get = |key: &str| meta.and_then(|m| m.get(key));
    let labels = entries(get("labels"), 32, &mut partial);
    let annotations = entries(get("annotations"), 32, &mut partial);
    let mut finalizers = Vec::new();
    if let Some(value) = get("finalizers") {
        if let Some(list) = value.as_array() {
            if list.len() > 16 {
                partial = true;
            }
            for item in list.iter().take(16) {
                if let Some(s) = item.as_str().filter(|s| !s.is_empty() && safe(s)) {
                    finalizers.push(s.into());
                } else {
                    partial = true;
                }
            }
        } else {
            partial = true;
        }
    }
    let managed_fields = managed(get("managedFields"), &mut partial);
    let deletion_timestamp = timestamp(get("deletionTimestamp"), &mut partial);
    let mut result = DetailMetadata {
        labels,
        annotations,
        finalizers,
        managed_fields,
        deletion_timestamp,
        partial,
    };
    while serde_json::to_vec(&result)
        .expect("metadata serializes")
        .len()
        > TOTAL_BYTES
    {
        result.partial = true;
        if result.managed_fields.pop().is_some()
            || result.annotations.pop().is_some()
            || result.labels.pop().is_some()
            || result.finalizers.pop().is_some()
        {
            continue;
        }
        result.deletion_timestamp = None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn wire_budget_counts_utf8_and_json_escapes() {
        for text in [
            "é".repeat(512),
            "😀".repeat(256),
            "\"\\".repeat(512),
            "\u{2028}".repeat(341),
        ] {
            let result = project(
                ResourceKind::Pods,
                &json!({"metadata":{"labels":{"app":text}}}),
            )
            .unwrap();
            assert_eq!(result.labels[0].value, text);
            assert!(!result.partial);
        }
        let labels: serde_json::Map<String, Value> = (0..32)
            .map(|i| (format!("label-{i}"), json!("\"\\".repeat(512))))
            .collect();
        let result = project(ResourceKind::Pods, &json!({"metadata":{"labels":labels}})).unwrap();
        assert!(result.partial);
        assert!(serde_json::to_vec(&result).unwrap().len() <= TOTAL_BYTES);
        assert!(serde_json::to_string(&result).unwrap().contains("\\\"\\\\"));
    }
    #[test]
    fn credentials_are_omitted_from_all_text_and_sensitive_ownership_names() {
        for value in [
            "Bearer abc",
            "Authorization: Basic abc",
            "password = abc",
            "{\"client_secret\":\"abc\"}",
            "-----BEGIN RSA PRIVATE KEY-----abc",
        ] {
            let result = project(
                ResourceKind::Pods,
                &json!({"metadata": {
                    "labels":{"app":value}, "annotations":{"example/note":value},
                    "finalizers":[value], "managedFields":[{"manager":value,"operation":"Update"}]
                }}),
            )
            .unwrap();
            assert!(result.partial);
            assert!(
                result.labels.is_empty()
                    && result.annotations.is_empty()
                    && result.finalizers.is_empty()
                    && result.managed_fields.is_empty()
            );
        }
        let result = project(ResourceKind::Pods, &json!({"metadata": {
            "labels":{"app":"my-app", "example/secret":"hidden"},
            "annotations":{"meta.helm.sh/release-name":"my-app", "kubectl.kubernetes.io/last-applied-configuration":"hidden"},
            "managedFields":[{"manager":"helm","operation":"Update","fieldsType":"FieldsV1","fieldsV1":{
                "f:metadata":{"f:labels":{"f:example/secret":{},"f:app":{}},"f:annotations":{"f:example/password":{},"f:meta.helm.sh/release-name":{}}},
                "f:spec":{"f:items":{"k:{\"value\":\"hidden\"}":{"f:name":{}}, "v:hidden":{}}}
            }}]
        }})).unwrap();
        assert!(result.partial);
        assert_eq!(result.labels[0].value, "my-app");
        assert_eq!(result.annotations[0].key, "meta.helm.sh/release-name");
        let wire = serde_json::to_string(&result).unwrap();
        for forbidden in ["hidden", "example/secret", "example/password", "k:", "v:"] {
            assert!(!wire.contains(forbidden));
        }
        assert!(result.managed_fields[0].fields.contains("[keyed item]"));
        assert!(
            result.managed_fields[0]
                .fields
                .contains("meta.helm.sh/release-name")
        );
    }
    #[test]
    fn timestamps_require_rfc3339_calendar_and_offset() {
        for value in [
            "2024-02-29T23:59:59.123456789+23:59",
            "2026-10-29t12:00:00z",
            "0000-01-01T00:00:00Z",
        ] {
            let mut partial = false;
            assert_eq!(
                timestamp(Some(&json!(value)), &mut partial).as_deref(),
                Some(value)
            );
            assert!(!partial);
        }
        for value in [
            "",
            "tomorrow",
            "2023-02-29T00:00:00Z",
            "2024-01-01 00:00:00Z",
            "2024-01-01T24:00:00Z",
            "2024-01-01T00:00:60Z",
            "2024-01-01T00:00:00+24:00",
            "2024-01-01T00:00:00.1234567890Z",
            "2024-01-01T00:00:00Z[UTC]",
        ] {
            let result = project(ResourceKind::Pods, &json!({"metadata":{"deletionTimestamp":value,"managedFields":[{"manager":"controller","operation":"Update","time":value,"fieldsType":"FieldsV1","fieldsV1":{}}]}})).unwrap();
            assert!(result.partial, "{value}");
            assert_eq!(result.deletion_timestamp, None, "{value}");
            assert_eq!(result.managed_fields[0].time, None, "{value}");
        }
    }
    #[test]
    fn sensitive_kinds_have_no_shared_metadata() {
        for kind in [ResourceKind::Secrets, ResourceKind::ConfigMaps] {
            assert!(project(kind, &json!({"metadata":{"labels":{"leak":"no"}}})).is_none());
        }
    }
    #[test]
    fn filters_annotations_controls_and_keyed_identifiers() {
        let result = project(ResourceKind::Pods, &json!({"metadata": {
            "labels":{"ok":"", "bad":"a\u{202e}b"},
            "annotations":{"example/note":"reviewed", "example/Token":"never", "kubectl.kubernetes.io/last-applied-configuration":"never"},
            "managedFields":[{"manager":"controller", "operation":"Update", "fieldsType":"FieldsV1", "fieldsV1":{"f:spec":{"f:containers":{"k:{\"secret\":\"never\"}":{"f:image":{}, "f:unknown":"never"}}}}}]
        }})).unwrap();
        assert!(result.partial);
        assert_eq!(result.labels.len(), 1);
        assert_eq!(result.annotations.len(), 1);
        assert_eq!(
            result.managed_fields[0].fields,
            r#"[["spec","containers","[keyed item]","image"]]"#
        );
        assert!(!serde_json::to_string(&result).unwrap().contains("never"));
    }
    #[test]
    fn enforces_serialized_budget_and_collection_limits() {
        let labels: serde_json::Map<String, Value> = (0..50)
            .map(|i| (format!("label-{i}"), json!("x".repeat(1024))))
            .collect();
        let result = project(
            ResourceKind::Pods,
            &json!({"metadata":{"labels":labels,"finalizers":vec!["a";20]}}),
        )
        .unwrap();
        assert!(result.partial);
        assert!(result.labels.len() <= 32);
        assert!(result.finalizers.len() <= 16);
        assert!(serde_json::to_vec(&result).unwrap().len() <= TOTAL_BYTES);
    }
    #[test]
    fn deep_and_unknown_fields_are_not_forwarded() {
        let mut tree = json!({"f:password":"never"});
        for _ in 0..20 {
            tree = json!({"f:nested":tree});
        }
        let result = project(ResourceKind::Pods, &json!({"metadata":{"managedFields":[{"manager":"controller","operation":"Update","fieldsType":"FieldsV1","fieldsV1":tree}]}})).unwrap();
        assert!(result.partial);
        assert_eq!(result.managed_fields[0].fields, "[]");
    }
}

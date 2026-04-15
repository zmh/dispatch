use serde_json::{Map, Value};

pub const MAX_LOG_ENTRIES: usize = 500;
pub const DEFAULT_LOG_FETCH_LIMIT: usize = 200;
pub const MAX_MESSAGE_CHARS: usize = 180;
pub const MAX_METADATA_CHARS: usize = 900;
pub const DEDUPE_WINDOW_SECONDS: i64 = 60;

#[derive(Debug, Clone)]
pub struct DiagnosticEventInput {
    pub run_id: Option<String>,
    pub scope: String,
    pub level: String,
    pub event: String,
    pub message: String,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct SanitizedDiagnosticEvent {
    pub run_id: Option<String>,
    pub scope: String,
    pub level: String,
    pub event: String,
    pub message: String,
    pub metadata: Map<String, Value>,
    pub error_code: Option<String>,
    pub provider_used: Option<String>,
}

pub fn sanitize_scope_filter(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim().to_ascii_lowercase();
    match value.as_str() {
        "refresh" | "categorization" => Some(value),
        _ => None,
    }
}

pub fn sanitize_diagnostic_event(input: DiagnosticEventInput) -> Option<SanitizedDiagnosticEvent> {
    let event = normalize_event(&input.event)?;
    let derived_scope = scope_for_event(&event).to_string();
    let _requested_scope = sanitize_scope_filter(Some(&input.scope));
    let level = normalize_level(&input.level, default_level_for_event(&event));
    let run_id = sanitize_run_id(input.run_id);

    let mut metadata = Map::new();
    let mut error_code: Option<String> = None;
    let mut provider_used: Option<String> = None;
    let allowed = allowed_metadata_keys(&event);

    for (raw_key, raw_value) in input.metadata {
        let key = normalize_metadata_key(&raw_key);
        if key.is_empty() || !allowed.contains(&key.as_str()) || is_sensitive_key(&key) {
            continue;
        }

        if key == "error" {
            if let Some(raw_err) = raw_value.as_str() {
                let code = normalize_error_code(raw_err);
                error_code = Some(code.clone());
                metadata.insert("error_code".to_string(), Value::String(code));
            }
            continue;
        }

        let Some(mut value) = sanitize_metadata_value(&key, &raw_value) else {
            continue;
        };

        if key == "error_code" {
            if let Some(code) = value.as_str() {
                let normalized = normalize_error_code(code);
                value = Value::String(normalized.clone());
                error_code = Some(normalized);
            }
        }

        if key == "provider_used" {
            provider_used = value.as_str().map(|v| v.to_string());
        }

        metadata.insert(key, value);
    }

    if provider_used.is_none() {
        provider_used = metadata
            .get("provider_requested")
            .and_then(Value::as_str)
            .map(normalize_provider)
            .map(str::to_string);
    }

    if let Some(ref used) = provider_used {
        metadata.insert("provider_used".to_string(), Value::String(used.clone()));
    }

    if level == "error" && error_code.is_none() {
        let code = "unknown_error".to_string();
        error_code = Some(code.clone());
        metadata
            .entry("error_code".to_string())
            .or_insert_with(|| Value::String(code));
    }

    clamp_metadata_size(&mut metadata);
    let message = sanitize_message(&input.message, &event);

    Some(SanitizedDiagnosticEvent {
        run_id,
        scope: derived_scope,
        level,
        event,
        message,
        metadata,
        error_code,
        provider_used,
    })
}

pub fn normalize_error_code(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return "unknown_error".to_string();
    }
    if lower.contains("429") || lower.contains("rate limit") {
        return "rate_limited".to_string();
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return "timeout".to_string();
    }
    if lower.contains("401") || lower.contains("unauthorized") {
        return "auth_unauthorized".to_string();
    }
    if lower.contains("403") || lower.contains("forbidden") {
        return "auth_forbidden".to_string();
    }
    if lower.contains("api key") || lower.contains("not configured") || lower.contains("missing") {
        return "missing_credentials".to_string();
    }
    if lower.contains("not installed") || lower.contains("not visible") {
        return "cli_unavailable".to_string();
    }
    if lower.contains("parse") {
        return "parse_error".to_string();
    }
    if lower.contains("network") || lower.contains("connect") {
        return "network_error".to_string();
    }
    if lower.contains("unknown ai provider") {
        return "provider_unknown".to_string();
    }
    "unknown_error".to_string()
}

fn normalize_event(event: &str) -> Option<String> {
    match event.trim().to_ascii_lowercase().as_str() {
        "refresh_started"
        | "refresh_completed"
        | "refresh_failed"
        | "classify_started"
        | "classify_completed"
        | "classify_warning"
        | "classify_failed"
        | "classify_skipped"
        | "codex_status_checked" => Some(event.trim().to_ascii_lowercase()),
        _ => None,
    }
}

fn scope_for_event(event: &str) -> &'static str {
    if event.starts_with("refresh_") {
        "refresh"
    } else {
        "categorization"
    }
}

fn default_level_for_event(event: &str) -> &'static str {
    if event.ends_with("_failed") {
        "error"
    } else if event.ends_with("_warning") {
        "warn"
    } else {
        "info"
    }
}

fn normalize_level(level: &str, fallback: &str) -> String {
    match level.trim().to_ascii_lowercase().as_str() {
        "info" | "warn" | "error" => level.trim().to_ascii_lowercase(),
        _ => fallback.to_string(),
    }
}

fn default_message_for_event(event: &str) -> &'static str {
    match event {
        "refresh_started" => "Refresh started",
        "refresh_completed" => "Refresh completed",
        "refresh_failed" => "Refresh failed",
        "classify_started" => "Classification started",
        "classify_completed" => "Classification completed",
        "classify_warning" => "Classification warning",
        "classify_failed" => "Classification failed",
        "classify_skipped" => "Classification skipped",
        "codex_status_checked" => "Codex status checked",
        _ => "Diagnostic event",
    }
}

fn sanitize_message(raw: &str, event: &str) -> String {
    let fallback = default_message_for_event(event);
    let trimmed = raw.trim();
    if trimmed.is_empty() || contains_sensitive_text(trimmed) {
        return fallback.to_string();
    }
    let compact = trimmed
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate_chars(&compact, MAX_MESSAGE_CHARS)
}

fn sanitize_run_id(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let filtered: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    let filtered = truncate_chars(filtered.trim(), 64);
    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

fn normalize_metadata_key(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace(' ', "_")
}

fn allowed_metadata_keys(event: &str) -> &'static [&'static str] {
    match event {
        "refresh_started" => &["provider_requested", "provider_used", "pending_before"],
        "refresh_completed" => &[
            "provider_requested",
            "provider_used",
            "new_messages",
            "classified_count",
            "pending_before",
            "pending_after",
            "duration_ms",
            "slack_fetch_ms",
            "db_write_ms",
            "classify_ms",
            "avatar_ms",
            "error_count",
        ],
        "refresh_failed" => &[
            "provider_requested",
            "provider_used",
            "pending_before",
            "pending_after",
            "duration_ms",
            "error",
            "error_code",
            "error_count",
        ],
        "classify_started" => &[
            "phase",
            "provider_requested",
            "provider_used",
            "rules_matched_count",
            "ai_attempted",
            "pending_before",
            "batch_size",
        ],
        "classify_completed" => &[
            "phase",
            "provider_requested",
            "provider_used",
            "rules_matched_count",
            "ai_attempted",
            "ai_succeeded",
            "classified_count",
            "pending_before",
            "pending_after",
            "duration_ms",
            "batch_size",
        ],
        "classify_warning" | "classify_failed" => &[
            "phase",
            "provider_requested",
            "provider_used",
            "rules_matched_count",
            "ai_attempted",
            "ai_succeeded",
            "classified_count",
            "pending_before",
            "pending_after",
            "duration_ms",
            "batch_size",
            "error",
            "error_code",
        ],
        "classify_skipped" => &[
            "phase",
            "provider_requested",
            "provider_used",
            "rules_matched_count",
            "ai_attempted",
            "reason",
            "pending_before",
            "pending_after",
            "batch_size",
        ],
        "codex_status_checked" => &[
            "provider_used",
            "installed",
            "authenticated",
            "auth_mode",
            "has_codex_subscription",
            "duration_ms",
            "error",
            "error_code",
        ],
        _ => &[],
    }
}

fn sanitize_metadata_value(key: &str, value: &Value) -> Option<Value> {
    match key {
        "provider_requested" | "provider_used" => value
            .as_str()
            .map(normalize_provider)
            .map(|s| Value::String(s.to_string())),
        "auth_mode" => value
            .as_str()
            .map(normalize_auth_mode)
            .map(|s| Value::String(s.to_string())),
        "reason" | "phase" => value
            .as_str()
            .map(|v| normalize_tag(v, 48))
            .filter(|v| !v.is_empty())
            .map(Value::String),
        "error_code" => value.as_str().map(normalize_error_code).map(Value::String),
        _ => match value {
            Value::Bool(v) => Some(Value::Bool(*v)),
            Value::Number(v) => Some(Value::Number(v.clone())),
            Value::String(v) => {
                if contains_sensitive_text(v) {
                    None
                } else {
                    Some(Value::String(truncate_chars(v.trim(), 96)))
                }
            }
            _ => None,
        },
    }
}

fn normalize_provider(raw: &str) -> &'static str {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "rules" || lower == "rules_only" {
        return "rules_only";
    }
    if lower.contains("codex") || lower.contains("chatgpt") {
        return "codex";
    }
    if lower.contains("openai") {
        return "openai";
    }
    if lower.contains("claude") {
        return "claude";
    }
    "unknown"
}

fn normalize_auth_mode(raw: &str) -> &'static str {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.contains("chatgpt") {
        "chatgpt"
    } else if lower.contains("api") {
        "api_key"
    } else {
        "unknown"
    }
}

fn normalize_tag(raw: &str, max_len: usize) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_' | '.' | ' ') {
            out.push('_');
        }
        if out.len() >= max_len {
            break;
        }
    }
    out.trim_matches('_').to_string()
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "token",
        "cookie",
        "api_key",
        "authorization",
        "prompt",
        "body",
        "subject",
        "sender",
        "channel",
        "message_text",
        "permalink",
        "slack_token",
        "slack_cookie",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn contains_sensitive_text(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "xoxc-",
        "xoxd-",
        "xoxa-",
        "xoxb-",
        "xoxp-",
        "sk-ant-",
        "sk-proj-",
        "bearer ",
        "authorization:",
        "document.cookie",
        "localstorage.localconfig_v2",
        "slack_token",
        "slack_cookie",
        "sender:",
        "subject:",
        "body:",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect::<String>()
}

fn clamp_metadata_size(metadata: &mut Map<String, Value>) {
    if metadata.is_empty() {
        return;
    }

    let mut keys: Vec<String> = metadata.keys().cloned().collect();
    keys.sort_by_key(|k| metadata_priority(k));

    let mut compact = Map::new();
    for key in keys {
        if let Some(value) = metadata.get(&key).cloned() {
            compact.insert(key.clone(), value);
            let len = serde_json::to_string(&compact)
                .map(|s| s.len())
                .unwrap_or(usize::MAX);
            if len > MAX_METADATA_CHARS {
                compact.remove(&key);
                break;
            }
        }
    }

    *metadata = compact;
}

fn metadata_priority(key: &str) -> usize {
    match key {
        "provider_used" => 0,
        "provider_requested" => 1,
        "error_code" => 2,
        "classified_count" => 3,
        "pending_before" => 4,
        "pending_after" => 5,
        "ai_attempted" => 6,
        "ai_succeeded" => 7,
        "rules_matched_count" => 8,
        "duration_ms" => 9,
        _ => 99,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitization_drops_disallowed_metadata_keys() {
        let mut metadata = Map::new();
        metadata.insert(
            "provider_used".to_string(),
            Value::String("codex".to_string()),
        );
        metadata.insert(
            "subject".to_string(),
            Value::String("Do not keep".to_string()),
        );
        metadata.insert("random".to_string(), Value::String("ignored".to_string()));

        let input = DiagnosticEventInput {
            run_id: Some("run-1".to_string()),
            scope: "categorization".to_string(),
            level: "info".to_string(),
            event: "classify_completed".to_string(),
            message: "Classification completed".to_string(),
            metadata,
        };

        let sanitized = sanitize_diagnostic_event(input).expect("sanitized event");
        assert_eq!(sanitized.scope, "categorization");
        assert_eq!(
            sanitized.metadata.get("provider_used"),
            Some(&json!("codex"))
        );
        assert!(!sanitized.metadata.contains_key("subject"));
        assert!(!sanitized.metadata.contains_key("random"));
    }

    #[test]
    fn sanitization_replaces_sensitive_message_and_normalizes_error_code() {
        let mut metadata = Map::new();
        metadata.insert(
            "error".to_string(),
            Value::String("401 Unauthorized: xoxc-secret".to_string()),
        );
        metadata.insert(
            "provider_used".to_string(),
            Value::String("codex".to_string()),
        );

        let input = DiagnosticEventInput {
            run_id: Some("run-2".to_string()),
            scope: "categorization".to_string(),
            level: "error".to_string(),
            event: "classify_failed".to_string(),
            message: "Bearer sk-ant-secret should never be logged".to_string(),
            metadata,
        };

        let sanitized = sanitize_diagnostic_event(input).expect("sanitized event");
        assert_eq!(sanitized.message, "Classification failed");
        assert_eq!(sanitized.error_code.as_deref(), Some("auth_unauthorized"));
        assert_eq!(
            sanitized.metadata.get("error_code"),
            Some(&Value::String("auth_unauthorized".to_string()))
        );
    }

    #[test]
    fn sanitization_redacts_xoxb_token_pattern() {
        let mut metadata = Map::new();
        metadata.insert(
            "provider_used".to_string(),
            Value::String("codex".to_string()),
        );
        metadata.insert(
            "provider_requested".to_string(),
            Value::String("codex".to_string()),
        );
        metadata.insert(
            "pending_before".to_string(),
            Value::Number(serde_json::Number::from(1)),
        );

        let input = DiagnosticEventInput {
            run_id: Some("run-token".to_string()),
            scope: "refresh".to_string(),
            level: "info".to_string(),
            event: "refresh_started".to_string(),
            message: "token leaked: xoxb-123456-abcdef".to_string(),
            metadata,
        };

        let sanitized = sanitize_diagnostic_event(input).expect("sanitized event");
        assert_eq!(sanitized.message, "Refresh started");
    }

    #[test]
    fn sanitize_scope_filter_only_allows_known_scopes() {
        assert_eq!(
            sanitize_scope_filter(Some("refresh")),
            Some("refresh".to_string())
        );
        assert_eq!(
            sanitize_scope_filter(Some("categorization")),
            Some("categorization".to_string())
        );
        assert_eq!(sanitize_scope_filter(Some("all")), None);
    }

    #[test]
    fn sanitization_keeps_slack_fetch_metric() {
        let mut metadata = Map::new();
        metadata.insert(
            "provider_used".to_string(),
            Value::String("codex".to_string()),
        );
        metadata.insert(
            "provider_requested".to_string(),
            Value::String("codex".to_string()),
        );
        metadata.insert(
            "slack_fetch_ms".to_string(),
            Value::Number(serde_json::Number::from(1234)),
        );

        let input = DiagnosticEventInput {
            run_id: Some("run-3".to_string()),
            scope: "refresh".to_string(),
            level: "info".to_string(),
            event: "refresh_completed".to_string(),
            message: "Refresh completed".to_string(),
            metadata,
        };

        let sanitized = sanitize_diagnostic_event(input).expect("sanitized event");
        assert_eq!(
            sanitized.metadata.get("slack_fetch_ms"),
            Some(&Value::Number(serde_json::Number::from(1234)))
        );
    }
}

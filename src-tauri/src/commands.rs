use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, State};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::RwLock;

use crate::classifier;
use crate::diagnostics::{DiagnosticEventInput, DEFAULT_LOG_FETCH_LIMIT, MAX_LOG_ENTRIES};
use crate::models::{
    Category, CategoryRule, CodexStatus, DiagnosticLogEntry, Message, MessageCounts,
    OnboardingSuggestions, RefreshResult, SaveSettingsResult, Settings, SlackCacheStatus,
    SlackChannel, SlackConnectionInfo, SlackUser,
};
use crate::slack;
use crate::storage::Database;

/// Build the AI classification system prompt from per-category descriptions.
fn build_classification_prompt(categories: &[Category]) -> String {
    let mut lines = vec![
        "You are a message classifier for a CEO's inbox.".to_string(),
        "Classify each message into exactly one of these categories:".to_string(),
        String::new(),
    ];

    for cat in categories {
        if cat.name == "other" {
            continue; // "other" is the hardcoded fallback, not listed with a description
        }
        let desc = cat
            .description
            .as_deref()
            .unwrap_or("(no description provided)");
        lines.push(format!("- \"{}\": {}", cat.name, desc));
    }

    lines.push(String::new());
    lines.push(
        "If a message doesn't clearly fit any category above, classify it as \"other\"."
            .to_string(),
    );
    lines.push(String::new());
    lines.push("Respond with ONLY a JSON array.".to_string());

    lines.join("\n")
}

pub struct AppState {
    pub db: Arc<Database>,
    pub refresh_in_progress: Arc<AtomicBool>,
    pub refresh_progress_percent: Arc<AtomicU8>,
    pub last_refresh_result: Arc<RwLock<RefreshResult>>,
    pub backlog_classify_in_progress: Arc<AtomicBool>,
}

fn rule_classifications_for_messages(
    messages: &[Message],
    categories: &[Category],
    rules: &[CategoryRule],
) -> Vec<(String, String)> {
    if messages.is_empty() || rules.is_empty() {
        return Vec::new();
    }

    let mut updates = Vec::new();
    let mut sorted_cats: Vec<&Category> = categories.iter().collect();
    sorted_cats.sort_by_key(|c| c.position);

    for msg in messages {
        let mut matched_category: Option<&str> = None;

        'outer: for cat in &sorted_cats {
            if cat.name == "other" {
                continue;
            }
            for rule in rules {
                if rule.category != cat.name {
                    continue;
                }
                let matches = match rule.rule_type.as_str() {
                    "keyword" => msg.body.to_lowercase().contains(&rule.value.to_lowercase()),
                    "sender" => msg.sender.to_lowercase() == rule.value.to_lowercase(),
                    "channel" => msg
                        .subject
                        .as_deref()
                        .map(|s| s.to_lowercase() == rule.value.to_lowercase())
                        .unwrap_or(false),
                    _ => false,
                };
                if matches {
                    matched_category = Some(&cat.name);
                    break 'outer;
                }
            }
        }

        if let Some(category) = matched_category {
            updates.push((msg.id.clone(), category.to_string()));
        }
    }

    updates
}

fn apply_rules_for_ids(
    db: &Database,
    categories: &[Category],
    rules: &[CategoryRule],
    ids: &[String],
) -> Result<usize, String> {
    let unclassified = db.get_unclassified_messages_by_ids(ids)?;
    let updates = rule_classifications_for_messages(&unclassified, categories, rules);
    db.update_classifications_batch(&updates)
}

const OVERLAP_SECONDS: i64 = 2 * 60 * 60;
const DEEP_SCAN_INTERVAL_SECONDS: u64 = 12 * 60 * 60;
const SLACK_PAGE_LIMIT: u32 = 3;
const SLACK_RETRY_ATTEMPTS: usize = 2;
const SLACK_CACHE_RETRY_ATTEMPTS: usize = 6;
const ONBOARDING_DM_SUGGESTION_LIMIT: usize = 20;
const ONBOARDING_CHANNEL_SUGGESTION_LIMIT: usize = 15;
const ONBOARDING_CHANNEL_SUGGESTION_PAGES: u32 = 10;
const AVATAR_FETCH_CONCURRENCY: usize = 4;
const FOREGROUND_CLASSIFY_LIMIT: usize = 300;
const BACKLOG_CLASSIFY_BATCH_SIZE: usize = 200;
const INCREMENTAL_STATE_KEY: &str = "slack_incremental_state_v1";
static RUN_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn now_epoch_seconds() -> Result<i64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64)
}

fn next_run_id() -> Result<String, String> {
    let ts = now_epoch_seconds()?;
    let seq = RUN_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    Ok(format!("refresh-{}-{}", ts, seq))
}

fn normalize_provider_label(provider: Option<&str>) -> String {
    let trimmed = provider.unwrap_or_default().trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        "rules_only".to_string()
    } else if matches!(trimmed.as_str(), "claude" | "openai" | "codex") {
        trimmed
    } else {
        "unknown".to_string()
    }
}

fn metadata_from_json(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn log_diagnostic(
    db: &Database,
    run_id: Option<&str>,
    scope: &str,
    level: &str,
    event: &str,
    message: &str,
    metadata: Map<String, Value>,
) {
    let _ = db.insert_diagnostic_log(DiagnosticEventInput {
        run_id: run_id.map(str::to_string),
        scope: scope.to_string(),
        level: level.to_string(),
        event: event.to_string(),
        message: message.to_string(),
        metadata,
    });
}

fn codex_status_log_metadata(status: &CodexStatus, duration_ms: u64) -> Map<String, Value> {
    let mut metadata = metadata_from_json(json!({
        "provider_used": "codex",
        "installed": status.installed,
        "authenticated": status.authenticated,
        "auth_mode": status.auth_mode.clone().unwrap_or_else(|| "unknown".to_string()),
        "has_codex_subscription": status.has_codex_subscription,
        "duration_ms": duration_ms
    }));
    if !status.installed || !status.authenticated {
        metadata.insert("error".to_string(), Value::String(status.message.clone()));
    }
    metadata
}

#[derive(Debug, Default, Clone)]
struct ClassifyOutcome {
    classified: usize,
    rules_matched_count: usize,
    ai_attempted: bool,
    ai_succeeded: bool,
    provider_requested: String,
    provider_used: String,
    warning: Option<String>,
    skipped_reason: Option<String>,
    batch_size: usize,
}

#[derive(Debug, Clone)]
struct ClassifyFailure {
    error: String,
    outcome: ClassifyOutcome,
}

fn classify_failed_metadata(
    phase: &str,
    pending_before: usize,
    pending_after: usize,
    duration_ms: u64,
    outcome: &ClassifyOutcome,
    error: &str,
) -> Map<String, Value> {
    metadata_from_json(json!({
        "phase": phase,
        "provider_requested": outcome.provider_requested.clone(),
        "provider_used": outcome.provider_used.clone(),
        "rules_matched_count": outcome.rules_matched_count,
        "ai_attempted": outcome.ai_attempted,
        "ai_succeeded": outcome.ai_succeeded,
        "classified_count": outcome.classified,
        "pending_before": pending_before,
        "pending_after": pending_after,
        "duration_ms": duration_ms,
        "batch_size": outcome.batch_size,
        "error": error
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedQueryState {
    checkpoint_ts: Option<i64>,
    last_deep_scan_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedIncrementalState {
    queries: HashMap<String, PersistedQueryState>,
}

fn workspace_key(token: &str, cookie: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update(b":");
    hasher.update(cookie.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

fn query_state_storage_key(workspace: &str, query_key: &str) -> String {
    format!("{}::{}", workspace, query_key)
}

fn load_incremental_state(db: &Database) -> Result<PersistedIncrementalState, String> {
    let raw = match db.get_setting(INCREMENTAL_STATE_KEY)? {
        Some(v) => v,
        None => return Ok(PersistedIncrementalState::default()),
    };
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

fn save_incremental_state(db: &Database, state: &PersistedIncrementalState) -> Result<(), String> {
    let json = serde_json::to_string(state).map_err(|e| e.to_string())?;
    db.set_setting(INCREMENTAL_STATE_KEY, &json)
}

fn unique_ids_with_limit<I>(ids: I, limit: usize) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    if limit == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if seen.insert(id.clone()) {
            out.push(id);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

fn now_epoch_u64() -> Result<u64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs())
}

async fn fetch_slack_cache_snapshot(
    token: &str,
    cookie: &str,
) -> Result<(Vec<SlackChannel>, Vec<SlackUser>), String> {
    let token2 = token.to_string();
    let cookie2 = cookie.to_string();
    let mut channels = Vec::new();
    let mut users = Vec::new();

    let (channels_result, users_result) = tokio::join!(
        slack::fetch_slack_channels_paged(token, cookie, SLACK_CACHE_RETRY_ATTEMPTS, |page| {
            channels.extend_from_slice(page);
            Ok(())
        }),
        slack::fetch_slack_users_paged(&token2, &cookie2, SLACK_CACHE_RETRY_ATTEMPTS, |page| {
            users.extend_from_slice(page);
            Ok(())
        },)
    );

    let mut preload_errors = Vec::new();
    if let Err(err) = channels_result {
        preload_errors.push(format!("channels cache: {}", err));
    }
    if let Err(err) = users_result {
        preload_errors.push(format!("users cache: {}", err));
    }
    if !preload_errors.is_empty() {
        return Err(preload_errors.join(" | "));
    }

    Ok((channels, users))
}

struct AtomicFlagGuard {
    flag: Arc<AtomicBool>,
}

impl AtomicFlagGuard {
    fn acquire(flag: Arc<AtomicBool>) -> Option<Self> {
        if flag
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(Self { flag })
        } else {
            None
        }
    }
}

impl Drop for AtomicFlagGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

async fn classify_message_ids(
    db: &Database,
    ids: &[String],
    categories: &[Category],
    rules: &[CategoryRule],
    ai_provider: Option<&str>,
    claude_api_key: Option<&str>,
    openai_api_key: Option<&str>,
    category_names: &[String],
    system_prompt: &str,
) -> Result<ClassifyOutcome, ClassifyFailure> {
    let mut outcome = ClassifyOutcome {
        provider_requested: normalize_provider_label(ai_provider),
        provider_used: normalize_provider_label(ai_provider),
        batch_size: ids.len(),
        ..ClassifyOutcome::default()
    };

    outcome.rules_matched_count =
        apply_rules_for_ids(db, categories, rules, ids).map_err(|e| ClassifyFailure {
            error: e,
            outcome: outcome.clone(),
        })?;
    outcome.classified += outcome.rules_matched_count;
    let remaining = db
        .get_unclassified_messages_by_ids(ids)
        .map_err(|e| ClassifyFailure {
            error: e,
            outcome: outcome.clone(),
        })?;
    if remaining.is_empty() {
        outcome.skipped_reason = Some("already_classified".to_string());
        return Ok(outcome);
    }

    let remaining_ids: Vec<String> = remaining.iter().map(|m| m.id.clone()).collect();
    let provider = ai_provider
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("");

    let classify_result = match provider {
        "claude" => match claude_api_key.map(str::trim).filter(|v| !v.is_empty()) {
            Some(api_key) => {
                outcome.ai_attempted = true;
                classifier::classify_messages_claude(
                    api_key,
                    system_prompt,
                    &remaining,
                    category_names,
                )
                .await
            }
            None => {
                Err("Claude provider selected but Claude API key is not configured".to_string())
            }
        },
        "openai" => match openai_api_key.map(str::trim).filter(|v| !v.is_empty()) {
            Some(api_key) => {
                outcome.ai_attempted = true;
                classifier::classify_messages_openai(
                    api_key,
                    system_prompt,
                    &remaining,
                    category_names,
                )
                .await
            }
            None => {
                Err("OpenAI provider selected but OpenAI API key is not configured".to_string())
            }
        },
        "codex" => {
            outcome.ai_attempted = true;
            classifier::classify_messages_codex(system_prompt, &remaining, category_names).await
        }
        "" => {
            outcome.provider_used = "rules_only".to_string();
            outcome.classified += db
                .set_messages_to_other_by_ids(&remaining_ids)
                .map_err(|e| ClassifyFailure {
                    error: e,
                    outcome: outcome.clone(),
                })?;
            outcome.skipped_reason = Some("rules_only_fallback".to_string());
            return Ok(outcome);
        }
        unknown => Err(format!("Unknown AI provider: {}", unknown)),
    };

    match classify_result {
        Ok(classifications) => {
            outcome.ai_succeeded = outcome.ai_attempted;
            outcome.classified +=
                db.update_classifications_batch(&classifications)
                    .map_err(|e| ClassifyFailure {
                        error: e,
                        outcome: outcome.clone(),
                    })?;
            Ok(outcome)
        }
        Err(err) => {
            outcome.warning = Some(err);
            Ok(outcome)
        }
    }
}

fn collect_delta_ids(upserted: &crate::storage::MessageUpsertResult) -> Vec<String> {
    let mut ids = Vec::with_capacity(upserted.new_ids.len() + upserted.changed_ids.len());
    let mut seen = HashSet::new();
    for id in upserted.new_ids.iter().chain(upserted.changed_ids.iter()) {
        if seen.insert(id.clone()) {
            ids.push(id.clone());
        }
    }
    ids
}

fn empty_refresh_result() -> RefreshResult {
    RefreshResult {
        new_messages: 0,
        classified: 0,
        pending_classification: 0,
        in_progress: false,
        progress_percent: 0,
        slack_fetch_ms: 0,
        db_write_ms: 0,
        classify_ms: 0,
        avatar_ms: 0,
        errors: vec![],
    }
}

fn set_refresh_progress(state: &AppState, progress_percent: u8) {
    state
        .refresh_progress_percent
        .store(progress_percent.min(100), Ordering::SeqCst);
}

async fn current_refresh_snapshot(state: &AppState) -> RefreshResult {
    let mut snapshot = state.last_refresh_result.read().await.clone();
    snapshot.in_progress = state.refresh_in_progress.load(Ordering::SeqCst);
    snapshot.progress_percent = state
        .refresh_progress_percent
        .load(Ordering::SeqCst)
        .min(100);
    snapshot
}

#[tauri::command]
pub async fn get_messages(
    state: State<'_, AppState>,
    classification: String,
    status: String,
) -> Result<Vec<Message>, String> {
    state.db.get_messages(&classification, &status)
}

#[tauri::command]
pub async fn get_messages_by_status(
    state: State<'_, AppState>,
    status: String,
) -> Result<Vec<Message>, String> {
    state.db.get_messages_by_status(&status)
}

#[tauri::command]
pub async fn get_message_counts(
    state: State<'_, AppState>,
    status: String,
) -> Result<MessageCounts, String> {
    state.db.get_message_counts(&status)
}

#[tauri::command]
pub async fn refresh_inbox(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    start_if_idle: Option<bool>,
) -> Result<RefreshResult, String> {
    let start_if_idle = start_if_idle.unwrap_or(true);
    if !start_if_idle {
        return Ok(current_refresh_snapshot(&state).await);
    }

    let Some(_refresh_guard) = AtomicFlagGuard::acquire(state.refresh_in_progress.clone()) else {
        return Ok(current_refresh_snapshot(&state).await);
    };

    set_refresh_progress(&state, 5);
    {
        let mut snapshot = state.last_refresh_result.write().await;
        *snapshot = RefreshResult {
            in_progress: true,
            progress_percent: 5,
            ..empty_refresh_result()
        };
    }

    let settings = state.db.get_settings()?;
    set_refresh_progress(&state, 12);
    let categories = settings.effective_categories();
    let rules = settings.effective_rules();
    let category_names: Vec<String> = categories.iter().map(|c| c.name.clone()).collect();
    let system_prompt = build_classification_prompt(&categories);
    let run_id = next_run_id()?;
    let provider_requested = normalize_provider_label(settings.ai_provider.as_deref());
    let pending_before_refresh = state.db.get_unclassified_inbox_count().unwrap_or(0);
    let refresh_started_at = Instant::now();
    let mut result = RefreshResult {
        in_progress: true,
        progress_percent: 12,
        ..empty_refresh_result()
    };
    let mut delta_ids: Vec<String> = Vec::new();
    log_diagnostic(
        &state.db,
        Some(&run_id),
        "refresh",
        "info",
        "refresh_started",
        "Refresh started",
        metadata_from_json(json!({
            "provider_requested": provider_requested.clone(),
            "provider_used": provider_requested.clone(),
            "pending_before": pending_before_refresh
        })),
    );

    // Fetch from Slack
    if let (Some(ref token), Some(ref cookie)) = (&settings.slack_token, &settings.slack_cookie) {
        set_refresh_progress(&state, 20);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        let ws_key = workspace_key(token, cookie);

        let mut persisted_state = match load_incremental_state(&state.db) {
            Ok(v) => v,
            Err(e) => {
                result.errors.push(format!("Incremental state: {}", e));
                PersistedIncrementalState::default()
            }
        };

        let mut sync_state = HashMap::new();
        for (key, value) in &persisted_state.queries {
            let Some((workspace, query_key)) = key.split_once("::") else {
                continue;
            };
            if workspace != ws_key {
                continue;
            }
            let deep_scan_due = value
                .last_deep_scan_at
                .map(|ts| now.saturating_sub(ts) >= DEEP_SCAN_INTERVAL_SECONDS)
                .unwrap_or(true);
            sync_state.insert(
                query_key.to_string(),
                slack::QuerySyncState {
                    checkpoint_ts: value.checkpoint_ts,
                    force_deep_scan: deep_scan_due,
                },
            );
        }

        let slack_fetch_start = Instant::now();
        let filters = settings.slack_filters.as_deref();
        match slack::fetch_slack_messages_with_sync(
            token,
            cookie,
            filters,
            &sync_state,
            OVERLAP_SECONDS,
            SLACK_PAGE_LIMIT,
            SLACK_RETRY_ATTEMPTS,
        )
        .await
        {
            Ok(mut fetched) => {
                result.slack_fetch_ms = slack_fetch_start.elapsed().as_millis() as u64;
                result
                    .errors
                    .extend(fetched.errors.into_iter().map(|e| format!("Slack: {}", e)));

                let avatar_lookup_start = Instant::now();
                let mut unique_user_ids = Vec::new();
                let mut seen_user_ids = HashSet::new();
                let mut message_ids_by_user: HashMap<String, Vec<String>> = HashMap::new();
                for item in &fetched.messages {
                    if let Some(user_id) = item.user_id.as_ref() {
                        if seen_user_ids.insert(user_id.clone()) {
                            unique_user_ids.push(user_id.clone());
                        }
                        message_ids_by_user
                            .entry(user_id.clone())
                            .or_default()
                            .push(item.message.id.clone());
                    }
                }

                let cached_avatars = state
                    .db
                    .get_slack_user_avatars(&unique_user_ids)
                    .unwrap_or_default();
                for item in &mut fetched.messages {
                    if let Some(user_id) = item.user_id.as_ref() {
                        if let Some(avatar) = cached_avatars.get(user_id) {
                            item.message.avatar_url = Some(avatar.clone());
                        }
                    }
                }
                let missing_avatar_users: Vec<String> = unique_user_ids
                    .into_iter()
                    .filter(|id| !cached_avatars.contains_key(id))
                    .collect();
                result.avatar_ms = avatar_lookup_start.elapsed().as_millis() as u64;

                let messages: Vec<Message> =
                    fetched.messages.into_iter().map(|m| m.message).collect();
                let db_write_start = Instant::now();
                let upserted = state.db.upsert_messages_batch(&messages)?;
                result.db_write_ms = db_write_start.elapsed().as_millis() as u64;
                result.new_messages = upserted.new_ids.len();
                delta_ids = collect_delta_ids(&upserted);
                if delta_ids.len() > FOREGROUND_CLASSIFY_LIMIT {
                    delta_ids.truncate(FOREGROUND_CLASSIFY_LIMIT);
                }

                for query_result in fetched.query_results {
                    if !query_result.success {
                        continue;
                    }
                    let _stopped_early = query_result.stopped_early;
                    let key = query_state_storage_key(&ws_key, &query_result.query_key);
                    let state = persisted_state.queries.entry(key).or_default();
                    if let Some(max_ts) = query_result.max_timestamp {
                        state.checkpoint_ts = Some(
                            state
                                .checkpoint_ts
                                .map(|prev| prev.max(max_ts))
                                .unwrap_or(max_ts),
                        );
                    }
                    if query_result.ran_deep_scan || state.last_deep_scan_at.is_none() {
                        state.last_deep_scan_at = Some(now);
                    }
                }
                if let Err(e) = save_incremental_state(&state.db, &persisted_state) {
                    result.errors.push(format!("Incremental state: {}", e));
                }

                if !missing_avatar_users.is_empty() {
                    let db = state.db.clone();
                    let token = token.clone();
                    let cookie = cookie.clone();
                    let message_ids_by_user = message_ids_by_user;
                    tokio::spawn(async move {
                        match slack::fetch_user_profiles_by_ids(
                            &token,
                            &cookie,
                            &missing_avatar_users,
                            AVATAR_FETCH_CONCURRENCY,
                            SLACK_RETRY_ATTEMPTS,
                        )
                        .await
                        {
                            Ok(users) => {
                                for user in users {
                                    if let Some(avatar_url) = user.avatar_url.as_deref() {
                                        let _ = db.upsert_slack_user_avatar(
                                            &user.id,
                                            Some(&user.name),
                                            Some(&user.real_name),
                                            avatar_url,
                                        );
                                        if let Some(message_ids) = message_ids_by_user.get(&user.id)
                                        {
                                            let _ = db.update_message_avatars_by_ids(
                                                message_ids,
                                                avatar_url,
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("[haystack] Avatar enrichment failed: {}", e);
                            }
                        }
                    });
                }
            }
            Err(e) => result.errors.push(format!("Slack: {}", e)),
        }
        set_refresh_progress(&state, 62);
    } else {
        result
            .errors
            .push("Slack credentials not configured".to_string());
        set_refresh_progress(&state, 62);
    }

    let mut classification_guard =
        AtomicFlagGuard::acquire(state.backlog_classify_in_progress.clone());

    let classify_start = Instant::now();
    let pending_before_classify = state.db.get_unclassified_inbox_count().unwrap_or(0);
    if delta_ids.is_empty() {
        log_diagnostic(
            &state.db,
            Some(&run_id),
            "categorization",
            "info",
            "classify_skipped",
            "Classification skipped",
            metadata_from_json(json!({
                "phase": "foreground",
                "provider_requested": provider_requested.clone(),
                "provider_used": provider_requested.clone(),
                "rules_matched_count": 0,
                "ai_attempted": false,
                "reason": "no_delta_ids",
                "pending_before": pending_before_classify,
                "pending_after": pending_before_classify,
                "batch_size": 0
            })),
        );
    } else if classification_guard.is_none() {
        log_diagnostic(
            &state.db,
            Some(&run_id),
            "categorization",
            "warn",
            "classify_skipped",
            "Classification skipped while another pass is active",
            metadata_from_json(json!({
                "phase": "foreground",
                "provider_requested": provider_requested.clone(),
                "provider_used": provider_requested.clone(),
                "rules_matched_count": 0,
                "ai_attempted": false,
                "reason": "classifier_busy",
                "pending_before": pending_before_classify,
                "pending_after": pending_before_classify,
                "batch_size": delta_ids.len()
            })),
        );
    } else {
        log_diagnostic(
            &state.db,
            Some(&run_id),
            "categorization",
            "info",
            "classify_started",
            "Classification started",
            metadata_from_json(json!({
                "phase": "foreground",
                "provider_requested": provider_requested.clone(),
                "provider_used": provider_requested.clone(),
                "rules_matched_count": 0,
                "ai_attempted": false,
                "pending_before": pending_before_classify,
                "batch_size": delta_ids.len()
            })),
        );

        match classify_message_ids(
            &state.db,
            &delta_ids,
            &categories,
            &rules,
            settings.ai_provider.as_deref(),
            settings.claude_api_key.as_deref(),
            settings.openai_api_key.as_deref(),
            &category_names,
            &system_prompt,
        )
        .await
        {
            Ok(outcome) => {
                result.classified += outcome.classified;
                let pending_after = state.db.get_unclassified_inbox_count().unwrap_or(0);
                let duration_ms = classify_start.elapsed().as_millis() as u64;

                if let Some(ref warning) = outcome.warning {
                    result.errors.push(format!("Classifier: {}", warning));
                    log_diagnostic(
                        &state.db,
                        Some(&run_id),
                        "categorization",
                        "warn",
                        "classify_warning",
                        "Classification completed with warning",
                        metadata_from_json(json!({
                            "phase": "foreground",
                            "provider_requested": outcome.provider_requested,
                            "provider_used": outcome.provider_used,
                            "rules_matched_count": outcome.rules_matched_count,
                            "ai_attempted": outcome.ai_attempted,
                            "ai_succeeded": outcome.ai_succeeded,
                            "classified_count": outcome.classified,
                            "pending_before": pending_before_classify,
                            "pending_after": pending_after,
                            "duration_ms": duration_ms,
                            "batch_size": outcome.batch_size,
                            "error": warning
                        })),
                    );
                } else if let Some(reason) = outcome.skipped_reason {
                    log_diagnostic(
                        &state.db,
                        Some(&run_id),
                        "categorization",
                        "info",
                        "classify_skipped",
                        "Classification skipped",
                        metadata_from_json(json!({
                            "phase": "foreground",
                            "provider_requested": outcome.provider_requested,
                            "provider_used": outcome.provider_used,
                            "rules_matched_count": outcome.rules_matched_count,
                            "ai_attempted": outcome.ai_attempted,
                            "reason": reason,
                            "pending_before": pending_before_classify,
                            "pending_after": pending_after,
                            "batch_size": outcome.batch_size
                        })),
                    );
                } else {
                    log_diagnostic(
                        &state.db,
                        Some(&run_id),
                        "categorization",
                        "info",
                        "classify_completed",
                        "Classification completed",
                        metadata_from_json(json!({
                            "phase": "foreground",
                            "provider_requested": outcome.provider_requested,
                            "provider_used": outcome.provider_used,
                            "rules_matched_count": outcome.rules_matched_count,
                            "ai_attempted": outcome.ai_attempted,
                            "ai_succeeded": outcome.ai_succeeded,
                            "classified_count": outcome.classified,
                            "pending_before": pending_before_classify,
                            "pending_after": pending_after,
                            "duration_ms": duration_ms,
                            "batch_size": outcome.batch_size
                        })),
                    );
                }
            }
            Err(failure) => {
                result.classified += failure.outcome.classified;
                result.errors.push(format!("Classifier: {}", failure.error));
                let pending_after = state.db.get_unclassified_inbox_count().unwrap_or(0);
                log_diagnostic(
                    &state.db,
                    Some(&run_id),
                    "categorization",
                    "error",
                    "classify_failed",
                    "Classification failed",
                    classify_failed_metadata(
                        "foreground",
                        pending_before_classify,
                        pending_after,
                        classify_start.elapsed().as_millis() as u64,
                        &failure.outcome,
                        &failure.error,
                    ),
                );
            }
        }
    }
    result.classify_ms = classify_start.elapsed().as_millis() as u64;
    set_refresh_progress(&state, 82);

    match state.db.get_unclassified_inbox_count() {
        Ok(pending) => result.pending_classification = pending,
        Err(e) => result.errors.push(format!("Classifier: {}", e)),
    }
    set_refresh_progress(&state, 88);

    if result.pending_classification > 0 {
        if let Some(backlog_guard) = classification_guard.take() {
            let db = state.db.clone();
            let app_handle = app.clone();
            let categories = categories.clone();
            let rules = rules.clone();
            let category_names = category_names.clone();
            let system_prompt = system_prompt.clone();
            let ai_provider = settings.ai_provider.clone();
            let claude_api_key = settings.claude_api_key.clone();
            let openai_api_key = settings.openai_api_key.clone();
            let run_id = run_id.clone();
            tokio::spawn(async move {
                let _guard = backlog_guard;
                let mut total_classified = 0usize;
                loop {
                    let batch =
                        match db.get_unclassified_messages_limited(BACKLOG_CLASSIFY_BATCH_SIZE) {
                            Ok(v) => v,
                            Err(e) => {
                                eprintln!("[haystack] Backlog classification load failed: {}", e);
                                break;
                            }
                        };
                    if batch.is_empty() {
                        break;
                    }

                    let ids: Vec<String> = batch.into_iter().map(|m| m.id).collect();
                    let pending_before = db.get_unclassified_inbox_count().unwrap_or(0);
                    let started_at = Instant::now();
                    let requested_provider = normalize_provider_label(ai_provider.as_deref());
                    log_diagnostic(
                        &db,
                        Some(&run_id),
                        "categorization",
                        "info",
                        "classify_started",
                        "Backlog classification started",
                        metadata_from_json(json!({
                            "phase": "backlog",
                            "provider_requested": requested_provider.clone(),
                            "provider_used": requested_provider.clone(),
                            "rules_matched_count": 0,
                            "ai_attempted": false,
                            "pending_before": pending_before,
                            "batch_size": ids.len()
                        })),
                    );
                    match classify_message_ids(
                        &db,
                        &ids,
                        &categories,
                        &rules,
                        ai_provider.as_deref(),
                        claude_api_key.as_deref(),
                        openai_api_key.as_deref(),
                        &category_names,
                        &system_prompt,
                    )
                    .await
                    {
                        Ok(outcome) => {
                            total_classified += outcome.classified;
                            let pending_after = db.get_unclassified_inbox_count().unwrap_or(0);
                            let duration_ms = started_at.elapsed().as_millis() as u64;
                            if let Some(err) = outcome.warning {
                                eprintln!("[haystack] Backlog classifier warning: {}", err);
                                log_diagnostic(
                                    &db,
                                    Some(&run_id),
                                    "categorization",
                                    "warn",
                                    "classify_warning",
                                    "Backlog classification warning",
                                    metadata_from_json(json!({
                                        "phase": "backlog",
                                        "provider_requested": outcome.provider_requested,
                                        "provider_used": outcome.provider_used,
                                        "rules_matched_count": outcome.rules_matched_count,
                                        "ai_attempted": outcome.ai_attempted,
                                        "ai_succeeded": outcome.ai_succeeded,
                                        "classified_count": outcome.classified,
                                        "pending_before": pending_before,
                                        "pending_after": pending_after,
                                        "duration_ms": duration_ms,
                                        "batch_size": outcome.batch_size,
                                        "error": err
                                    })),
                                );
                                break;
                            }
                            if let Some(reason) = outcome.skipped_reason {
                                log_diagnostic(
                                    &db,
                                    Some(&run_id),
                                    "categorization",
                                    "info",
                                    "classify_skipped",
                                    "Backlog classification skipped",
                                    metadata_from_json(json!({
                                        "phase": "backlog",
                                        "provider_requested": outcome.provider_requested,
                                        "provider_used": outcome.provider_used,
                                        "rules_matched_count": outcome.rules_matched_count,
                                        "ai_attempted": outcome.ai_attempted,
                                        "reason": reason,
                                        "pending_before": pending_before,
                                        "pending_after": pending_after,
                                        "batch_size": outcome.batch_size
                                    })),
                                );
                            } else {
                                log_diagnostic(
                                    &db,
                                    Some(&run_id),
                                    "categorization",
                                    "info",
                                    "classify_completed",
                                    "Backlog classification completed",
                                    metadata_from_json(json!({
                                        "phase": "backlog",
                                        "provider_requested": outcome.provider_requested,
                                        "provider_used": outcome.provider_used,
                                        "rules_matched_count": outcome.rules_matched_count,
                                        "ai_attempted": outcome.ai_attempted,
                                        "ai_succeeded": outcome.ai_succeeded,
                                        "classified_count": outcome.classified,
                                        "pending_before": pending_before,
                                        "pending_after": pending_after,
                                        "duration_ms": duration_ms,
                                        "batch_size": outcome.batch_size
                                    })),
                                );
                            }

                            if outcome.classified == 0 {
                                eprintln!(
                                    "[haystack] Backlog classification made no progress; stopping background pass"
                                );
                                break;
                            }
                        }
                        Err(failure) => {
                            eprintln!(
                                "[haystack] Backlog classification failed: {}",
                                failure.error
                            );
                            total_classified += failure.outcome.classified;
                            let pending_after = db.get_unclassified_inbox_count().unwrap_or(0);
                            log_diagnostic(
                                &db,
                                Some(&run_id),
                                "categorization",
                                "error",
                                "classify_failed",
                                "Backlog classification failed",
                                classify_failed_metadata(
                                    "backlog",
                                    pending_before,
                                    pending_after,
                                    started_at.elapsed().as_millis() as u64,
                                    &failure.outcome,
                                    &failure.error,
                                ),
                            );
                            break;
                        }
                    }

                    if ids.len() < BACKLOG_CLASSIFY_BATCH_SIZE {
                        break;
                    }
                }

                if total_classified > 0 {
                    let _ = app_handle.emit("messages-classified", total_classified);
                }
            });
        }
    }
    drop(classification_guard);

    // Refresh slack cache if stale (>24 hours old or missing)
    if let (Some(ref token), Some(ref cookie)) = (&settings.slack_token, &settings.slack_cookie) {
        let needs_refresh = match state.db.get_setting("cache_last_populated")? {
            Some(ts_str) => {
                let cached_at = ts_str.parse::<u64>().unwrap_or(0);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs();
                now.saturating_sub(cached_at) > 86400
            }
            None => true,
        };
        if needs_refresh {
            let db = state.db.clone();
            let token = token.clone();
            let cookie = cookie.clone();
            tokio::spawn(async move {
                match fetch_slack_cache_snapshot(&token, &cookie).await {
                    Ok((channels, users)) => {
                        if let Err(e) = db.replace_slack_cache(&channels, &users) {
                            eprintln!("[haystack] Slack cache replace failed: {}", e);
                            return;
                        }
                        if let Ok(now) = now_epoch_u64() {
                            let _ = db.set_setting("cache_last_populated", &now.to_string());
                        }
                    }
                    Err(e) => {
                        eprintln!("[haystack] Slack cache refresh failed: {}", e);
                    }
                }
            });
        }
    }
    set_refresh_progress(&state, 95);

    if result.new_messages > 0 {
        let enabled = settings.notifications_enabled.unwrap_or(true);
        if enabled {
            let (title, body) = if result.new_messages == 1 {
                (
                    "New message".to_string(),
                    "1 new message in your inbox".to_string(),
                )
            } else {
                (
                    format!("{} new messages", result.new_messages),
                    format!("{} new messages in your inbox", result.new_messages),
                )
            };
            let _ = app
                .notification()
                .builder()
                .title(&title)
                .body(&body)
                .show();
        }
    }

    set_refresh_progress(&state, 100);
    result.in_progress = false;
    result.progress_percent = 100;
    {
        let mut snapshot = state.last_refresh_result.write().await;
        *snapshot = result.clone();
    }

    let refresh_duration_ms = refresh_started_at.elapsed().as_millis() as u64;
    if result.errors.is_empty() {
        log_diagnostic(
            &state.db,
            Some(&run_id),
            "refresh",
            "info",
            "refresh_completed",
            "Refresh completed",
            metadata_from_json(json!({
                "provider_requested": provider_requested.clone(),
                "provider_used": provider_requested.clone(),
                "new_messages": result.new_messages,
                "classified_count": result.classified,
                "pending_before": pending_before_refresh,
                "pending_after": result.pending_classification,
                "duration_ms": refresh_duration_ms,
                "slack_fetch_ms": result.slack_fetch_ms,
                "db_write_ms": result.db_write_ms,
                "classify_ms": result.classify_ms,
                "avatar_ms": result.avatar_ms,
                "error_count": 0
            })),
        );
    } else {
        log_diagnostic(
            &state.db,
            Some(&run_id),
            "refresh",
            "error",
            "refresh_failed",
            "Refresh completed with errors",
            metadata_from_json(json!({
                "provider_requested": provider_requested.clone(),
                "provider_used": provider_requested.clone(),
                "pending_before": pending_before_refresh,
                "pending_after": result.pending_classification,
                "duration_ms": refresh_duration_ms,
                "error_count": result.errors.len(),
                "error": result.errors.first().cloned().unwrap_or_default()
            })),
        );
    }

    Ok(result)
}

#[tauri::command]
pub async fn get_starred_messages(state: State<'_, AppState>) -> Result<Vec<Message>, String> {
    state.db.get_starred_messages()
}

#[tauri::command]
pub async fn mark_done_message(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.db.mark_done_message(&id)
}

#[tauri::command]
pub async fn snooze_message(
    state: State<'_, AppState>,
    id: String,
    until: i64,
) -> Result<(), String> {
    state.db.snooze_message(&id, until)
}

#[tauri::command]
pub async fn star_message(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state.db.toggle_star(&id)
}

#[tauri::command]
pub async fn set_unread_message(
    state: State<'_, AppState>,
    id: String,
    unread: bool,
) -> Result<bool, String> {
    state.db.set_unread_message(&id, unread)
}

/// Convert a Slack permalink (https://workspace.slack.com/archives/C.../p...)
/// into a slack:// deep link that opens directly in the Slack desktop app.
fn slack_permalink_to_deeplink(url: &str, team_id: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let segments: Vec<&str> = parsed.path_segments()?.collect();

    // Expected: ["archives", "<channel_id>", "p<timestamp>"]
    if segments.len() < 3 || segments[0] != "archives" {
        return None;
    }

    let channel_id = segments[1];
    let ts_raw = segments[2].strip_prefix('p')?;

    // Slack timestamps: "1234567890123456" -> "1234567890.123456" (dot 6 from end)
    if ts_raw.len() <= 6 {
        return None;
    }
    let (secs, micros) = ts_raw.split_at(ts_raw.len() - 6);
    let message_ts = format!("{}.{}", secs, micros);

    // Check for thread_ts in query params
    let thread_ts = parsed
        .query_pairs()
        .find(|(k, _)| k == "thread_ts")
        .map(|(_, v)| v.to_string());

    let mut deep = format!(
        "slack://channel?team={}&id={}&message={}",
        team_id, channel_id, message_ts
    );
    if let Some(tts) = thread_ts {
        deep.push_str(&format!("&thread_ts={}", tts));
    }
    Some(deep)
}

/// Get team_id, using cached value from DB or fetching from Slack API.
async fn get_or_fetch_team_id(db: &crate::storage::Database) -> Result<String, String> {
    // Check cache first
    if let Some(cached) = db.get_setting("slack_team_id")? {
        if !cached.is_empty() {
            return Ok(cached);
        }
    }

    // Fetch from API
    let settings = db.get_settings()?;
    let token = settings
        .slack_token
        .as_deref()
        .ok_or("Slack token not configured")?;
    let cookie = settings
        .slack_cookie
        .as_deref()
        .ok_or("Slack cookie not configured")?;

    let team_id = slack::get_team_id(token, cookie).await?;
    db.set_setting("slack_team_id", &team_id)?;
    Ok(team_id)
}

#[tauri::command]
pub async fn open_link(
    state: State<'_, AppState>,
    url: String,
    use_slack_app: bool,
) -> Result<(), String> {
    if use_slack_app {
        let team_id = get_or_fetch_team_id(&state.db).await.unwrap_or_default();
        let link = slack_permalink_to_deeplink(&url, &team_id).unwrap_or_else(|| url.clone());
        open::that(&link).map_err(|e| e.to_string())
    } else {
        open::that(&url).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state.db.get_settings()
}

#[tauri::command]
pub async fn get_codex_status(state: State<'_, AppState>) -> Result<CodexStatus, String> {
    let started = Instant::now();
    let status = classifier::get_codex_status().await;
    log_diagnostic(
        &state.db,
        None,
        "categorization",
        "info",
        "codex_status_checked",
        "Codex status checked",
        codex_status_log_metadata(&status, started.elapsed().as_millis() as u64),
    );
    Ok(status)
}

#[tauri::command]
pub async fn get_diagnostic_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
    scope: Option<String>,
) -> Result<Vec<DiagnosticLogEntry>, String> {
    let limit = limit
        .unwrap_or(DEFAULT_LOG_FETCH_LIMIT)
        .max(1)
        .min(MAX_LOG_ENTRIES);
    let scope = scope.as_deref().map(str::trim).filter(|v| !v.is_empty());
    state.db.get_diagnostic_logs(limit, scope)
}

#[tauri::command]
pub async fn clear_diagnostic_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.db.clear_diagnostic_logs()
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<SaveSettingsResult, String> {
    state.db.save_settings(&settings)
}

#[tauri::command]
pub async fn test_slack_connection(
    token: String,
    cookie: String,
) -> Result<SlackConnectionInfo, String> {
    slack::test_connection(&token, &cookie).await
}

#[tauri::command]
pub async fn populate_slack_cache(state: State<'_, AppState>) -> Result<SlackCacheStatus, String> {
    let settings = state.db.get_settings()?;
    let token = settings
        .slack_token
        .as_deref()
        .ok_or("Slack token not configured")?
        .to_string();
    let cookie = settings
        .slack_cookie
        .as_deref()
        .ok_or("Slack cookie not configured")?
        .to_string();

    let token3 = token.clone();
    let cookie3 = cookie.clone();
    let token4 = token.clone();
    let cookie4 = cookie.clone();

    let (cache_snapshot_result, dms_result, channel_suggestions_result) = tokio::join!(
        fetch_slack_cache_snapshot(&token, &cookie),
        slack::fetch_recent_dm_user_ids(
            &token3,
            &cookie3,
            ONBOARDING_DM_SUGGESTION_LIMIT,
            SLACK_CACHE_RETRY_ATTEMPTS,
        ),
        slack::fetch_recent_to_me_channels(
            &token4,
            &cookie4,
            ONBOARDING_CHANNEL_SUGGESTION_LIMIT,
            ONBOARDING_CHANNEL_SUGGESTION_PAGES,
            SLACK_CACHE_RETRY_ATTEMPTS,
        )
    );

    let (channels, users) = cache_snapshot_result?;
    state.db.replace_slack_cache(&channels, &users)?;

    // Save DM/channel IDs for onboarding suggestions (best-effort).
    if let Ok(dm_ids) = dms_result {
        let dm_ids = unique_ids_with_limit(dm_ids, ONBOARDING_DM_SUGGESTION_LIMIT);
        let _ = state.db.save_suggested_dm_user_ids(&dm_ids);
    }
    let suggested_channel_ids = if let Ok(channel_suggestions) = channel_suggestions_result {
        unique_ids_with_limit(
            channel_suggestions
                .into_iter()
                .map(|channel| channel.id)
                .collect::<Vec<_>>(),
            ONBOARDING_CHANNEL_SUGGESTION_LIMIT,
        )
    } else {
        let mut fallback_channels = channels.clone();
        fallback_channels.sort_by(|a, b| {
            b.updated
                .partial_cmp(&a.updated)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        unique_ids_with_limit(
            fallback_channels
                .into_iter()
                .map(|channel| channel.id)
                .collect::<Vec<_>>(),
            ONBOARDING_CHANNEL_SUGGESTION_LIMIT,
        )
    };
    if !suggested_channel_ids.is_empty() {
        let _ = state.db.save_suggested_channel_ids(&suggested_channel_ids);
    }

    // Record cache timestamp
    let now = now_epoch_u64()?;
    state
        .db
        .set_setting("cache_last_populated", &now.to_string())?;

    state.db.slack_cache_count()
}

#[tauri::command]
pub async fn get_onboarding_suggestions(
    state: State<'_, AppState>,
) -> Result<OnboardingSuggestions, String> {
    let raw_dm_ids = state.db.get_suggested_dm_user_ids()?;
    let dm_user_ids = unique_ids_with_limit(raw_dm_ids.clone(), ONBOARDING_DM_SUGGESTION_LIMIT);
    if dm_user_ids != raw_dm_ids {
        let _ = state.db.save_suggested_dm_user_ids(&dm_user_ids);
    }

    let suggested_people = state.db.get_slack_users_by_ids(&dm_user_ids)?;

    let raw_channel_ids = state.db.get_suggested_channel_ids()?;
    let channel_ids =
        unique_ids_with_limit(raw_channel_ids.clone(), ONBOARDING_CHANNEL_SUGGESTION_LIMIT);
    if channel_ids != raw_channel_ids {
        let _ = state.db.save_suggested_channel_ids(&channel_ids);
    }

    let mut suggested_channels = state.db.get_slack_channels_by_ids(&channel_ids)?;
    if suggested_channels.len() < ONBOARDING_CHANNEL_SUGGESTION_LIMIT {
        let mut seen_channel_ids: HashSet<String> =
            suggested_channels.iter().map(|ch| ch.id.clone()).collect();
        for ch in state
            .db
            .get_suggested_channels(ONBOARDING_CHANNEL_SUGGESTION_LIMIT * 4)?
        {
            if seen_channel_ids.insert(ch.id.clone()) {
                suggested_channels.push(ch);
            }
            if suggested_channels.len() >= ONBOARDING_CHANNEL_SUGGESTION_LIMIT {
                break;
            }
        }
    }

    Ok(OnboardingSuggestions {
        suggested_people,
        suggested_channels,
    })
}

#[tauri::command]
pub async fn search_slack_users(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<SlackUser>, String> {
    state.db.search_slack_users(&query)
}

#[tauri::command]
pub async fn search_slack_channels(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<SlackChannel>, String> {
    // First try cache for instant results
    let cached = state.db.search_slack_channels(&query)?;

    // Also try live Slack search which finds ALL channels (not just member channels)
    let settings = state.db.get_settings()?;
    let token = settings.slack_token.as_deref().unwrap_or_default();
    let cookie = settings.slack_cookie.as_deref().unwrap_or_default();

    if token.is_empty() || cookie.is_empty() {
        return Ok(cached);
    }

    match slack::search_channels_live(token, cookie, &query).await {
        Ok(live) => {
            // Merge: live results first (deduped), then any cached results not in live
            let mut seen = std::collections::HashSet::new();
            let mut merged = Vec::new();
            for ch in live {
                if seen.insert(ch.id.clone()) {
                    merged.push(ch);
                }
            }
            for ch in cached {
                if seen.insert(ch.id.clone()) {
                    merged.push(ch);
                }
            }
            // Re-sort merged results by name relevance to the query
            let query_lower = query.to_lowercase();
            merged.sort_by_key(|ch| {
                let name_lower = ch.name.to_lowercase();
                if name_lower == query_lower {
                    0
                } else if name_lower.starts_with(&query_lower) {
                    1
                } else if name_lower.contains(&query_lower) {
                    2
                } else {
                    3
                }
            });
            Ok(merged)
        }
        Err(_) => Ok(cached), // Fall back to cache on error
    }
}

#[tauri::command]
pub async fn get_slack_cache_status(
    state: State<'_, AppState>,
) -> Result<SlackCacheStatus, String> {
    state.db.slack_cache_count()
}

#[tauri::command]
pub async fn set_window_theme(app: tauri::AppHandle, theme: String) -> Result<(), String> {
    use tauri::Manager;
    let window = app
        .get_webview_window("main")
        .ok_or("Main window not found")?;
    let tauri_theme = match theme.as_str() {
        "light" | "solarized-light" => Some(tauri::Theme::Light),
        _ => Some(tauri::Theme::Dark),
    };
    window.set_theme(tauri_theme).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_categories() -> Vec<Category> {
        vec![
            Category {
                name: "important".to_string(),
                builtin: true,
                position: 0,
                description: Some("Needs attention".to_string()),
            },
            Category {
                name: "other".to_string(),
                builtin: true,
                position: 1,
                description: None,
            },
        ]
    }

    fn sample_message(id: &str, body: &str) -> Message {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_secs() as i64;
        Message {
            id: id.to_string(),
            source: "slack".to_string(),
            sender: "alice".to_string(),
            subject: Some("general".to_string()),
            body: body.to_string(),
            body_html: None,
            permalink: None,
            avatar_url: None,
            timestamp: now,
            classification: "unclassified".to_string(),
            status: "inbox".to_string(),
            starred: false,
            unread: true,
            snoozed_until: None,
            created_at: now,
        }
    }

    #[test]
    fn codex_status_log_metadata_includes_error_when_unavailable() {
        let status = CodexStatus {
            installed: false,
            authenticated: false,
            auth_mode: None,
            has_codex_subscription: false,
            message: "Codex CLI is not installed".to_string(),
        };
        let metadata = codex_status_log_metadata(&status, 42);
        assert_eq!(metadata.get("provider_used"), Some(&json!("codex")));
        assert_eq!(metadata.get("installed"), Some(&json!(false)));
        assert_eq!(
            metadata.get("error"),
            Some(&json!("Codex CLI is not installed"))
        );
    }

    #[test]
    fn classify_failed_metadata_uses_real_outcome_fields() {
        let outcome = ClassifyOutcome {
            classified: 3,
            rules_matched_count: 1,
            ai_attempted: true,
            ai_succeeded: false,
            provider_requested: "codex".to_string(),
            provider_used: "codex".to_string(),
            warning: None,
            skipped_reason: None,
            batch_size: 5,
        };

        let metadata =
            classify_failed_metadata("foreground", 10, 7, 420, &outcome, "db_write_failed");

        assert_eq!(metadata.get("provider_requested"), Some(&json!("codex")));
        assert_eq!(metadata.get("provider_used"), Some(&json!("codex")));
        assert_eq!(metadata.get("rules_matched_count"), Some(&json!(1)));
        assert_eq!(metadata.get("ai_attempted"), Some(&json!(true)));
        assert_eq!(metadata.get("ai_succeeded"), Some(&json!(false)));
        assert_eq!(metadata.get("classified_count"), Some(&json!(3)));
        assert_eq!(metadata.get("pending_before"), Some(&json!(10)));
        assert_eq!(metadata.get("pending_after"), Some(&json!(7)));
        assert_eq!(metadata.get("duration_ms"), Some(&json!(420)));
        assert_eq!(metadata.get("batch_size"), Some(&json!(5)));
        assert_eq!(metadata.get("error"), Some(&json!("db_write_failed")));
    }

    #[test]
    fn diagnostic_events_share_run_id_across_refresh_cycle() {
        let db = Database::new(":memory:").expect("db init");
        let run_id = "refresh-1700000000-1";

        log_diagnostic(
            &db,
            Some(run_id),
            "refresh",
            "info",
            "refresh_started",
            "Refresh started",
            metadata_from_json(json!({
                "provider_requested": "codex",
                "provider_used": "codex",
                "pending_before": 2
            })),
        );
        log_diagnostic(
            &db,
            Some(run_id),
            "categorization",
            "info",
            "classify_started",
            "Classification started",
            metadata_from_json(json!({
                "phase": "foreground",
                "provider_requested": "codex",
                "provider_used": "codex",
                "rules_matched_count": 0,
                "ai_attempted": false,
                "pending_before": 2,
                "batch_size": 2
            })),
        );
        log_diagnostic(
            &db,
            Some(run_id),
            "categorization",
            "info",
            "classify_completed",
            "Classification completed",
            metadata_from_json(json!({
                "phase": "foreground",
                "provider_requested": "codex",
                "provider_used": "codex",
                "rules_matched_count": 0,
                "ai_attempted": true,
                "ai_succeeded": true,
                "classified_count": 2,
                "pending_before": 2,
                "pending_after": 0,
                "duration_ms": 120,
                "batch_size": 2
            })),
        );
        log_diagnostic(
            &db,
            Some(run_id),
            "refresh",
            "info",
            "refresh_completed",
            "Refresh completed",
            metadata_from_json(json!({
                "provider_requested": "codex",
                "provider_used": "codex",
                "new_messages": 2,
                "classified_count": 2,
                "pending_before": 2,
                "pending_after": 0,
                "duration_ms": 800,
                "slack_fetch_ms": 300,
                "db_write_ms": 120,
                "classify_ms": 180,
                "avatar_ms": 20,
                "error_count": 0
            })),
        );

        let logs = db.get_diagnostic_logs(20, None).expect("fetch logs");
        assert_eq!(logs.len(), 4);
        assert!(
            logs.iter().all(|log| log.run_id.as_deref() == Some(run_id)),
            "all logs should share the same run_id"
        );
    }

    #[test]
    fn classify_message_ids_rules_only_fallback_classifies_remaining_messages() {
        let db = Database::new(":memory:").expect("db init");
        let message = sample_message("m-rules-only", "normal update");
        db.insert_message(&message).expect("insert message");

        let categories = sample_categories();
        let rules: Vec<CategoryRule> = Vec::new();
        let ids = vec![message.id.clone()];
        let category_names: Vec<String> = categories.iter().map(|c| c.name.clone()).collect();
        let system_prompt = build_classification_prompt(&categories);
        let rt = tokio::runtime::Runtime::new().expect("runtime");

        let outcome = rt
            .block_on(classify_message_ids(
                &db,
                &ids,
                &categories,
                &rules,
                None,
                None,
                None,
                &category_names,
                &system_prompt,
            ))
            .expect("classify outcome");

        assert_eq!(outcome.classified, 1);
        assert_eq!(outcome.rules_matched_count, 0);
        assert!(!outcome.ai_attempted);
        assert!(!outcome.ai_succeeded);
        assert_eq!(outcome.provider_used, "rules_only");
        assert_eq!(
            outcome.skipped_reason.as_deref(),
            Some("rules_only_fallback")
        );
        assert!(outcome.warning.is_none());
        assert_eq!(db.get_unclassified_inbox_count().expect("count"), 0);
    }

    #[test]
    fn classify_message_ids_unknown_provider_reports_warning_without_ai_attempt() {
        let db = Database::new(":memory:").expect("db init");
        let message = sample_message("m-unknown-provider", "status update");
        db.insert_message(&message).expect("insert message");

        let categories = sample_categories();
        let rules: Vec<CategoryRule> = Vec::new();
        let ids = vec![message.id.clone()];
        let category_names: Vec<String> = categories.iter().map(|c| c.name.clone()).collect();
        let system_prompt = build_classification_prompt(&categories);
        let rt = tokio::runtime::Runtime::new().expect("runtime");

        let outcome = rt
            .block_on(classify_message_ids(
                &db,
                &ids,
                &categories,
                &rules,
                Some("mystery-provider"),
                None,
                None,
                &category_names,
                &system_prompt,
            ))
            .expect("classify outcome");

        assert_eq!(outcome.classified, 0);
        assert_eq!(outcome.rules_matched_count, 0);
        assert!(!outcome.ai_attempted);
        assert!(!outcome.ai_succeeded);
        assert!(outcome.skipped_reason.is_none());
        assert!(outcome
            .warning
            .as_deref()
            .unwrap_or_default()
            .contains("Unknown AI provider"));
        assert_eq!(db.get_unclassified_inbox_count().expect("count"), 1);
    }
}

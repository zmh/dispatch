use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, State};
use tauri_plugin_notification::NotificationExt;

use crate::classifier;
use crate::models::{
    Category, CategoryRule, CodexStatus, Message, MessageCounts, OnboardingSuggestions,
    RefreshResult, SaveSettingsResult, Settings, SlackCacheStatus, SlackChannel,
    SlackConnectionInfo, SlackUser,
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
const AVATAR_FETCH_CONCURRENCY: usize = 4;
const FOREGROUND_CLASSIFY_LIMIT: usize = 300;
const BACKLOG_CLASSIFY_BATCH_SIZE: usize = 200;
const INCREMENTAL_STATE_KEY: &str = "slack_incremental_state_v1";

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
) -> Result<(usize, Option<String>), String> {
    let mut classified = 0usize;

    classified += apply_rules_for_ids(db, categories, rules, ids)?;
    let remaining = db.get_unclassified_messages_by_ids(ids)?;
    if remaining.is_empty() {
        return Ok((classified, None));
    }

    let remaining_ids: Vec<String> = remaining.iter().map(|m| m.id.clone()).collect();
    let provider = ai_provider
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("");

    let classify_result = match provider {
        "claude" => match claude_api_key.map(str::trim).filter(|v| !v.is_empty()) {
            Some(api_key) => {
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
            classifier::classify_messages_codex(system_prompt, &remaining, category_names).await
        }
        "" => {
            classified += db.set_messages_to_other_by_ids(&remaining_ids)?;
            return Ok((classified, None));
        }
        unknown => Err(format!("Unknown AI provider: {}", unknown)),
    };

    match classify_result {
        Ok(classifications) => {
            classified += db.update_classifications_batch(&classifications)?;
            Ok((classified, None))
        }
        Err(err) => Ok((classified, Some(err))),
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
) -> Result<RefreshResult, String> {
    let Some(_refresh_guard) = AtomicFlagGuard::acquire(state.refresh_in_progress.clone()) else {
        return Ok(RefreshResult {
            new_messages: 0,
            classified: 0,
            pending_classification: 0,
            in_progress: true,
            slack_fetch_ms: 0,
            db_write_ms: 0,
            classify_ms: 0,
            avatar_ms: 0,
            errors: vec![],
        });
    };

    let settings = state.db.get_settings()?;
    let categories = settings.effective_categories();
    let rules = settings.effective_rules();
    let category_names: Vec<String> = categories.iter().map(|c| c.name.clone()).collect();
    let system_prompt = build_classification_prompt(&categories);
    let mut result = RefreshResult {
        new_messages: 0,
        classified: 0,
        pending_classification: 0,
        in_progress: false,
        slack_fetch_ms: 0,
        db_write_ms: 0,
        classify_ms: 0,
        avatar_ms: 0,
        errors: vec![],
    };
    let mut delta_ids: Vec<String> = Vec::new();

    // Fetch from Slack
    if let (Some(ref token), Some(ref cookie)) = (&settings.slack_token, &settings.slack_cookie) {
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
    } else {
        result
            .errors
            .push("Slack credentials not configured".to_string());
    }

    let mut classification_guard =
        AtomicFlagGuard::acquire(state.backlog_classify_in_progress.clone());

    let classify_start = Instant::now();
    if !delta_ids.is_empty() {
        if classification_guard.is_some() {
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
                Ok((n, err)) => {
                    result.classified += n;
                    if let Some(err) = err {
                        result.errors.push(format!("Classifier: {}", err));
                    }
                }
                Err(e) => result.errors.push(format!("Classifier: {}", e)),
            }
        }
    }
    result.classify_ms = classify_start.elapsed().as_millis() as u64;

    match state.db.get_unclassified_inbox_count() {
        Ok(pending) => result.pending_classification = pending,
        Err(e) => result.errors.push(format!("Classifier: {}", e)),
    }

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
                        Ok((n, err)) => {
                            total_classified += n;
                            if let Some(err) = err {
                                eprintln!("[haystack] Backlog classifier warning: {}", err);
                                break;
                            }
                            if n == 0 {
                                eprintln!(
                                    "[haystack] Backlog classification made no progress; stopping background pass"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("[haystack] Backlog classification failed: {}", e);
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
                now - cached_at > 86400
            }
            None => true,
        };
        if needs_refresh {
            let db_ch = state.db.clone();
            let db_us = state.db.clone();
            let db_ts = state.db.clone();
            let token = token.clone();
            let cookie = cookie.clone();
            let token2 = token.clone();
            let cookie2 = cookie.clone();
            tokio::spawn(async move {
                let (ch, us) = tokio::join!(
                    slack::fetch_slack_channels_paged(&token, &cookie, |page| {
                        db_ch.append_slack_channels(page)
                    }),
                    slack::fetch_slack_users_paged(&token2, &cookie2, |page| {
                        db_us.append_slack_users(page)
                    })
                );
                if let Err(ref e) = ch {
                    eprintln!("[haystack] Slack channel cache refresh failed: {}", e);
                }
                if let Err(ref e) = us {
                    eprintln!("[haystack] Slack user cache refresh failed: {}", e);
                }
                if ch.is_ok() && us.is_ok() {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = db_ts.set_setting("cache_last_populated", &now.to_string());
                }
            });
        }
    }

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
pub async fn get_codex_status() -> Result<CodexStatus, String> {
    Ok(classifier::get_codex_status().await)
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

    state.db.clear_slack_cache()?;

    let db_channels = state.db.clone();
    let db_users = state.db.clone();
    let db_dms = state.db.clone();
    let token2 = token.clone();
    let cookie2 = cookie.clone();
    let token3 = token.clone();
    let cookie3 = cookie.clone();

    let (channels_result, users_result, dms_result) = tokio::join!(
        slack::fetch_slack_channels_paged(&token, &cookie, |page| {
            db_channels.append_slack_channels(page)
        }),
        slack::fetch_slack_users_paged(&token2, &cookie2, |page| {
            db_users.append_slack_users(page)
        }),
        slack::fetch_recent_dm_user_ids(&token3, &cookie3, 20)
    );

    channels_result?;
    users_result?;

    // Save DM user IDs for onboarding suggestions (best-effort)
    if let Ok(dm_ids) = dms_result {
        let _ = db_dms.save_suggested_dm_user_ids(&dm_ids);
    }

    // Record cache timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    state
        .db
        .set_setting("cache_last_populated", &now.to_string())?;

    state.db.slack_cache_count()
}

#[tauri::command]
pub async fn get_onboarding_suggestions(
    state: State<'_, AppState>,
) -> Result<OnboardingSuggestions, String> {
    let dm_user_ids = state.db.get_suggested_dm_user_ids()?;
    let suggested_people = state.db.get_slack_users_by_ids(&dm_user_ids)?;
    let suggested_channels = state.db.get_suggested_channels(15)?;
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

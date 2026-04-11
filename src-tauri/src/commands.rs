use std::sync::Arc;
use tauri::State;
use tauri_plugin_notification::NotificationExt;

use crate::classifier;
use crate::models::{
    Category, CategoryRule, Message, MessageCounts, OnboardingSuggestions, RefreshResult,
    SaveSettingsResult, Settings, SlackCacheStatus, SlackChannel, SlackConnectionInfo, SlackUser,
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
        let desc = cat.description.as_deref().unwrap_or("(no description provided)");
        lines.push(format!("- \"{}\": {}", cat.name, desc));
    }

    lines.push(String::new());
    lines.push("If a message doesn't clearly fit any category above, classify it as \"other\".".to_string());
    lines.push(String::new());
    lines.push("Respond with ONLY a JSON array.".to_string());

    lines.join("\n")
}

pub struct AppState {
    pub db: Arc<Database>,
}

/// Apply rules to unclassified messages. Returns how many were classified by rules.
fn apply_rules(
    db: &Database,
    categories: &[Category],
    rules: &[CategoryRule],
) -> Result<usize, String> {
    let unclassified = db.get_unclassified_messages()?;
    if unclassified.is_empty() || rules.is_empty() {
        return Ok(0);
    }

    let mut classified = 0;

    // Sort categories by position — check rules for earlier categories first.
    // "other" (the last category) never has rules, it's the catch-all.
    let mut sorted_cats: Vec<&Category> = categories.iter().collect();
    sorted_cats.sort_by_key(|c| c.position);

    for msg in &unclassified {
        let mut matched_category: Option<&str> = None;

        'outer: for cat in &sorted_cats {
            if cat.name == "other" {
                continue; // other is catch-all, skip
            }
            for rule in rules {
                if rule.category != cat.name {
                    continue;
                }
                let matches = match rule.rule_type.as_str() {
                    "keyword" => msg.body.to_lowercase().contains(&rule.value.to_lowercase()),
                    "sender" => msg.sender.to_lowercase() == rule.value.to_lowercase(),
                    "channel" => {
                        msg.subject
                            .as_deref()
                            .map(|s| s.to_lowercase() == rule.value.to_lowercase())
                            .unwrap_or(false)
                    }
                    _ => false,
                };
                if matches {
                    matched_category = Some(&cat.name);
                    break 'outer;
                }
            }
        }

        if let Some(category) = matched_category {
            db.update_classification(&msg.id, category)?;
            classified += 1;
        }
    }

    Ok(classified)
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
pub async fn refresh_inbox(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<RefreshResult, String> {
    let settings = state.db.get_settings()?;
    let mut result = RefreshResult {
        new_messages: 0,
        classified: 0,
        errors: vec![],
    };

    // Fetch from Slack
    if let (Some(ref token), Some(ref cookie)) = (&settings.slack_token, &settings.slack_cookie) {
        let filters = settings.slack_filters.as_deref();
        match slack::fetch_slack_messages(token, cookie, filters).await {
            Ok(messages) => {
                for msg in &messages {
                    if state.db.insert_message(msg)? {
                        result.new_messages += 1;
                    }
                }
            }
            Err(e) => result.errors.push(format!("Slack: {}", e)),
        }
    } else {
        result.errors.push("Slack credentials not configured".to_string());
    }

    // Step 1: Apply rules to unclassified messages
    let categories = settings.effective_categories();
    let rules = settings.effective_rules();
    match apply_rules(&state.db, &categories, &rules) {
        Ok(n) => result.classified += n,
        Err(e) => result.errors.push(format!("Rules: {}", e)),
    }

    // Step 2: AI classification for remaining unclassified
    let category_names: Vec<String> = categories.iter().map(|c| c.name.clone()).collect();

    if let Some(ref api_key) = settings.claude_api_key {
        let unclassified = state.db.get_unclassified_messages()?;
        if !unclassified.is_empty() {
            let system_prompt = build_classification_prompt(&categories);

            match classifier::classify_messages(api_key, &system_prompt, &unclassified, &category_names).await {
                Ok(classifications) => {
                    for (id, class) in &classifications {
                        state.db.update_classification(id, class)?;
                        result.classified += 1;
                    }
                }
                Err(e) => result.errors.push(format!("Classifier: {}", e)),
            }
        }
    } else {
        // No API key: default classify as "other"
        let unclassified = state.db.get_unclassified_messages()?;
        for msg in &unclassified {
            state.db.update_classification(&msg.id, "other")?;
        }
    }

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

    // Send notification for new messages
    if result.new_messages > 0 {
        let enabled = settings.notifications_enabled.unwrap_or(true);
        if enabled {
            let (title, body) = if result.new_messages == 1 {
                ("New message".to_string(), "1 new message in your inbox".to_string())
            } else {
                (format!("{} new messages", result.new_messages), format!("{} new messages in your inbox", result.new_messages))
            };
            let _ = app.notification().builder().title(&title).body(&body).show();
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
        let link = slack_permalink_to_deeplink(&url, &team_id)
            .unwrap_or_else(|| url.clone());
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
    state.db.set_setting("cache_last_populated", &now.to_string())?;

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
    let token = settings
        .slack_token
        .as_deref()
        .unwrap_or_default();
    let cookie = settings
        .slack_cookie
        .as_deref()
        .unwrap_or_default();

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
                if name_lower == query_lower { 0 }
                else if name_lower.starts_with(&query_lower) { 1 }
                else if name_lower.contains(&query_lower) { 2 }
                else { 3 }
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
pub async fn set_window_theme(
    app: tauri::AppHandle,
    theme: String,
) -> Result<(), String> {
    use tauri::Manager;
    let window = app
        .get_webview_window("main")
        .ok_or("Main window not found")?;
    let tauri_theme = match theme.as_str() {
        "light" | "solarized-light" => Some(tauri::Theme::Light),
        _ => Some(tauri::Theme::Dark),
    };
    window
        .set_theme(tauri_theme)
        .map_err(|e| e.to_string())
}

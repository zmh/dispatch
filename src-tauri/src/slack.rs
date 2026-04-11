use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, COOKIE};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::{Message, SlackChannel, SlackFilter, SlackUser};

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil days algorithm from Howard Hinnant
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn build_headers(token: &str, cookie: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|e| e.to_string())?,
    );
    headers.insert(
        COOKIE,
        HeaderValue::from_str(&format!("d={}", cookie)).map_err(|e| e.to_string())?,
    );
    Ok(headers)
}

/// Cookie-only headers (for POST endpoints where token goes in form body).
fn build_cookie_header(cookie: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        COOKIE,
        HeaderValue::from_str(&format!("d={}", cookie)).map_err(|e| e.to_string())?,
    );
    Ok(headers)
}

#[derive(Debug, Deserialize)]
struct SlackSearchResponse {
    ok: bool,
    messages: Option<SlackMessages>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackMessages {
    matches: Vec<SlackMatch>,
    paging: Option<SlackPaging>,
}

#[derive(Debug, Deserialize)]
struct SlackPaging {
    pages: u32,
}

#[derive(Debug, Deserialize)]
struct SlackMatch {
    ts: String,
    text: String,
    channel: SlackChannelInfo,
    permalink: Option<String>,
    username: Option<String>,
    user: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackChannelInfo {
    name: Option<String>,
    #[allow(dead_code)]
    id: Option<String>,
}

// (User search uses serde_json::Value for flexible response parsing)

// -- Users list API types --

#[derive(Debug, Deserialize)]
struct UsersListResponse {
    ok: bool,
    members: Option<Vec<UserMember>>,
    response_metadata: Option<ResponseMetadata>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserMember {
    id: String,
    name: Option<String>,
    real_name: Option<String>,
    deleted: Option<bool>,
    is_bot: Option<bool>,
    profile: Option<UserProfile>,
}

#[derive(Debug, Deserialize)]
struct UserProfile {
    real_name: Option<String>,
}

// -- Conversations list API types --

#[derive(Debug, Deserialize)]
struct ConversationsListResponse {
    ok: bool,
    channels: Option<Vec<ConversationChannel>>,
    response_metadata: Option<ResponseMetadata>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConversationChannel {
    id: String,
    name: String,
    is_archived: Option<bool>,
    is_private: Option<bool>,
    #[serde(default)]
    updated: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ImConversation {
    #[allow(dead_code)]
    id: String,
    user: String,
}

#[derive(Debug, Deserialize)]
struct ImConversationsListResponse {
    ok: bool,
    channels: Option<Vec<ImConversation>>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseMetadata {
    next_cursor: Option<String>,
}

/// Slack emoji shortcodes that differ from the standard shortcodes in the `emojis` crate.
/// Maps Slack-specific names to either the standard shortcode (looked up via the crate)
/// or directly to a Unicode emoji string.
static SLACK_EMOJI_ALIASES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        // Thumbs
        ("thumbup", "thumbsup"),
        ("thumbdown", "thumbsdown"),
        // Faces
        ("thinking_face", "thinking"),
        ("simple_smile", "slightly_smiling_face"),
        ("grinning_face_with_smiling_eyes", "grinning"),
        ("face_with_rolling_eyes", "rolling_eyes"),
        ("upside_down_face", "upside_down"),
        ("nerd_face", "nerd"),
        ("face_with_thermometer", "thermometer_face"),
        ("slightly_frowning_face", "frowning"),
        ("zipper_mouth_face", "zipper_mouth"),
        ("money_mouth_face", "money_mouth"),
        ("face_with_head_bandage", "head_bandage"),
        ("hugging_face", "hugging"),
        // Gestures
        ("raised_hand_with_fingers_splayed", "hand_splayed"),
        ("reversed_hand_with_middle_finger_extended", "middle_finger"),
        ("sign_of_the_horns", "metal"),
        ("writing_hand", "writing"),
        // Objects / symbols
        ("lower_left_paintbrush", "paintbrush"),
        ("old_key", "key2"),
        ("memo", "pencil"),
        ("heavy_exclamation_mark", "exclamation"),
        // Misc commonly used in Slack
        ("male-technologist", "man_technologist"),
        ("female-technologist", "woman_technologist"),
        ("male-detective", "man_detective"),
        ("female-detective", "woman_detective"),
    ])
});

/// Skin tone modifier Unicode codepoints (Slack uses :skin-tone-2: through :skin-tone-6:).
fn skin_tone_modifier(n: u32) -> Option<char> {
    match n {
        2 => Some('\u{1F3FB}'), // light
        3 => Some('\u{1F3FC}'), // medium-light
        4 => Some('\u{1F3FD}'), // medium
        5 => Some('\u{1F3FE}'), // medium-dark
        6 => Some('\u{1F3FF}'), // dark
        _ => None,
    }
}

/// Convert Slack emoji shortcodes (e.g. `:wave:`, `:wave::skin-tone-3:`) to Unicode emoji.
fn convert_emoji_shortcodes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find(':') {
        result.push_str(&remaining[..start]);
        let after_colon = &remaining[start + 1..];

        if let Some(end) = after_colon.find(':') {
            let name = &after_colon[..end];
            // Skip if it looks like it contains spaces or is empty (not an emoji shortcode)
            if name.is_empty() || name.contains(' ') {
                result.push(':');
                remaining = after_colon;
                continue;
            }

            // Check for skin tone modifier
            if let Some(stripped) = name.strip_prefix("skin-tone-") {
                if let Ok(n) = stripped.parse::<u32>() {
                    if let Some(modifier) = skin_tone_modifier(n) {
                        // Append skin tone modifier to previous emoji
                        result.push(modifier);
                        remaining = &after_colon[end + 1..];
                        continue;
                    }
                }
            }

            // Look up the emoji by shortcode, falling back to Slack aliases
            let emoji = emojis::get_by_shortcode(name).or_else(|| {
                SLACK_EMOJI_ALIASES
                    .get(name)
                    .and_then(|alias| emojis::get_by_shortcode(alias))
            });
            if let Some(emoji) = emoji {
                result.push_str(emoji.as_str());
                remaining = &after_colon[end + 1..];
            } else {
                // Not a known emoji, keep the original text
                result.push(':');
                remaining = after_colon;
            }
        } else {
            // No closing colon found
            result.push(':');
            remaining = after_colon;
        }
    }

    result.push_str(remaining);
    result
}

/// Convert Slack mrkdwn to plain text.
fn slack_to_plain(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut inner = String::new();
            for c in chars.by_ref() {
                if c == '>' {
                    break;
                }
                inner.push(c);
            }
            if inner.starts_with('@') || inner.starts_with("!subteam") {
                if let Some(label) = inner.split('|').nth(1) {
                    result.push('@');
                    result.push_str(label);
                } else {
                    result.push_str(&inner);
                }
            } else if inner.starts_with('!') {
                let cmd = inner.trim_start_matches('!');
                let label = cmd.split('|').next().unwrap_or(cmd);
                result.push('@');
                result.push_str(label);
            } else if inner.starts_with('#') {
                if let Some(label) = inner.split('|').nth(1) {
                    result.push('#');
                    result.push_str(label);
                } else {
                    result.push_str(&inner);
                }
            } else {
                if let Some(label) = inner.split('|').nth(1) {
                    result.push_str(label);
                } else {
                    result.push_str(&inner);
                }
            }
        } else {
            result.push(ch);
        }
    }

    convert_emoji_shortcodes(&result)
}

/// Convert Slack mrkdwn to HTML for the preview panel.
fn slack_to_html(text: &str) -> String {
    let mut result = String::with_capacity(text.len() * 2);
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut inner = String::new();
            for c in chars.by_ref() {
                if c == '>' {
                    break;
                }
                inner.push(c);
            }
            if inner.starts_with('@') || inner.starts_with("!subteam") {
                let label = inner.split('|').nth(1).map(|l| format!("@{}", l))
                    .unwrap_or_else(|| inner.clone());
                result.push_str(&format!("<strong>{}</strong>", html_escape(&label)));
            } else if inner.starts_with('!') {
                let cmd = inner.trim_start_matches('!');
                let label = cmd.split('|').next().unwrap_or(cmd);
                result.push_str(&format!("<strong>@{}</strong>", html_escape(label)));
            } else if inner.starts_with('#') {
                let label = inner.split('|').nth(1).map(|l| format!("#{}", l))
                    .unwrap_or_else(|| inner.clone());
                result.push_str(&format!("<strong>{}</strong>", html_escape(&label)));
            } else {
                let parts: Vec<&str> = inner.splitn(2, '|').collect();
                let url = parts[0];
                let label = if parts.len() > 1 { parts[1] } else { url };
                result.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    html_escape(url),
                    html_escape(label)
                ));
            }
        } else if ch == '\n' {
            result.push_str("<br>");
        } else if ch == '&' {
            result.push_str("&amp;");
        } else if ch == '"' {
            result.push_str("&quot;");
        } else {
            result.push(ch);
        }
    }

    convert_emoji_shortcodes(&result)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Build individual search queries from filters (one per filter to avoid OR/grouping issues).
/// Always includes `to:me` to capture DMs and @-mentions directed at the user.
fn build_queries_from_filters(filters: &[SlackFilter], cutoff_date: &str) -> Vec<String> {
    let mut queries = vec![
        format!("to:me after:{}", cutoff_date), // Always fetch messages directed at user
    ];

    for f in filters {
        match f.filter_type.as_str() {
            "user" => queries.push(format!("from:<@{}> after:{}", f.id, cutoff_date)),
            "channel" => {
                let name = f.display_name.trim_start_matches('#');
                queries.push(format!("in:{} after:{}", name, cutoff_date));
            }
            "to" => queries.push(format!("to:{} after:{}", f.display_name, cutoff_date)),
            _ => {}
        }
    }

    queries
}

pub async fn fetch_slack_messages(
    token: &str,
    cookie: &str,
    filters: Option<&[SlackFilter]>,
) -> Result<Vec<Message>, String> {
    let client = reqwest::Client::new();
    let headers = build_headers(token, cookie)?;

    let thirty_days_ago = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System time error: {}", e))?
        .as_secs()
        - (30 * 86400);
    let cutoff_date = {
        let days_since_epoch = thirty_days_ago / 86400;
        let (y, m, d) = days_to_ymd(days_since_epoch);
        format!("{}-{:02}-{:02}", y, m, d)
    };

    let queries = match filters {
        Some(f) if !f.is_empty() => build_queries_from_filters(f, &cutoff_date),
        _ => build_queries_from_filters(&[], &cutoff_date),
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System time error: {}", e))?
        .as_secs() as i64;

    let mut all_messages = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut user_id_to_msg_indices: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();

    // Run each filter as a separate query to avoid OR/grouping issues
    for query in &queries {
        let mut page: u32 = 1;

        loop {
            let page_str = page.to_string();
            let response = client
                .get("https://slack.com/api/search.messages")
                .headers(headers.clone())
                .query(&[
                    ("query", query.as_str()),
                    ("sort", "timestamp"),
                    ("sort_dir", "desc"),
                    ("count", "100"),
                    ("page", page_str.as_str()),
                ])
                .send()
                .await
                .map_err(|e| format!("Slack request failed: {}", e))?;

            let data: SlackSearchResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse Slack response: {}", e))?;

            if !data.ok {
                break;
            }

            let slack_messages = match data.messages {
                Some(m) => m,
                None => break,
            };

            let total_pages = slack_messages.paging.map(|p| p.pages).unwrap_or(1);

            for m in slack_messages.matches {
                let msg_id = format!("slack:{}", m.ts);
                if !seen_ids.insert(msg_id.clone()) {
                    continue; // skip duplicates across queries
                }
                let ts_float: f64 = m.ts.parse().unwrap_or(0.0);
                let ts = ts_float as i64;
                let body = slack_to_plain(&m.text);
                let body_html = Some(slack_to_html(&m.text));
                // DM channels show up with user-ID-like names (e.g. "U0SCBPQTXML")
                // Multi-person DMs use "mpdm-" prefix (e.g. "mpdm-zach.hamed--matt--sophie-1")
                let channel_name = match &m.channel.name {
                    Some(name) if name.len() >= 9 && name.starts_with('U') && name.chars().all(|c| c.is_ascii_alphanumeric()) => {
                        Some("DM".to_string())
                    }
                    Some(name) if name.starts_with("mpdm-") => {
                        Some("Group DM".to_string())
                    }
                    other => other.clone(),
                };

                all_messages.push(Message {
                    id: msg_id,
                    source: "slack".to_string(),
                    sender: m.username.unwrap_or_else(|| "unknown".to_string()),
                    subject: channel_name,
                    body,
                    body_html,
                    permalink: m.permalink,
                    avatar_url: None, // filled in below via batch user lookup
                    timestamp: ts,
                    classification: "unclassified".to_string(),
                    status: "inbox".to_string(),
                    starred: false,
                    snoozed_until: None,
                    created_at: now,
                });
                // Track user ID for avatar lookup
                if let Some(ref uid) = m.user {
                    if !uid.is_empty() {
                        let idx = all_messages.len() - 1;
                        user_id_to_msg_indices
                            .entry(uid.clone())
                            .or_insert_with(Vec::new)
                            .push(idx);
                    }
                }
            }

            // Cap at 3 pages per query (300 messages) to keep things fast
            if page >= total_pages || page >= 3 {
                break;
            }
            page += 1;
        }
    }

    // Batch-fetch avatar URLs for unique user IDs
    if !user_id_to_msg_indices.is_empty() {
        let cookie_headers = build_cookie_header(cookie)?;
        let unique_ids: Vec<String> = user_id_to_msg_indices.keys().cloned().collect();
        for uid in &unique_ids {
            if let Ok(resp) = client
                .post("https://slack.com/api/users.info")
                .headers(cookie_headers.clone())
                .form(&[("token", token), ("user", uid.as_str())])
                .send()
                .await
            {
                if let Ok(text) = resp.text().await {
                    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(avatar) = raw
                            .get("user")
                            .and_then(|u| u.get("profile"))
                            .and_then(|p| p.get("image_72"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(indices) = user_id_to_msg_indices.get(uid) {
                                for &idx in indices {
                                    all_messages[idx].avatar_url = Some(avatar.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(all_messages)
}

/// Search Slack users by name via search.modules, plus a handle-based
/// fallback using search.messages with `from:@query` to find users by handle.
pub async fn search_users_live(
    token: &str,
    cookie: &str,
    query: &str,
) -> Result<Vec<SlackUser>, String> {
    let client = reqwest::Client::new();
    let headers = build_cookie_header(cookie)?;

    // 1) Name-based search via search.modules
    let modules_resp = client
        .post("https://slack.com/api/search.modules")
        .headers(headers.clone())
        .form(&[
            ("token", token),
            ("query", query),
            ("module", "people"),
            ("count", "20"),
        ])
        .send()
        .await
        .map_err(|e| format!("search.modules/people: {}", e))?;

    let text = modules_resp.text().await.map_err(|e| e.to_string())?;
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

    let mut users = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    if raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        if let Some(items) = raw.get("items").and_then(|v| v.as_array()) {
            for item in items {
                let user_obj = item.get("user").unwrap_or(item);
                if let Some(id) = user_obj.get("id").and_then(|v| v.as_str()) {
                    let profile = user_obj.get("profile");
                    let name = user_obj.get("name").and_then(|v| v.as_str())
                        .or_else(|| profile.and_then(|p| p.get("display_name")).and_then(|v| v.as_str()))
                        .unwrap_or("");
                    let real_name = profile.and_then(|p| p.get("real_name")).and_then(|v| v.as_str())
                        .or_else(|| user_obj.get("real_name").and_then(|v| v.as_str()))
                        .unwrap_or(name);
                    if seen_ids.insert(id.to_string()) {
                        users.push(SlackUser {
                            id: id.to_string(),
                            name: name.to_string(),
                            real_name: real_name.to_string(),
                        });
                    }
                }
            }
        }
    }

    // 2) Handle-based search: search messages from `from:@query` to find users by Slack handle.
    //    This catches users whose handle matches but whose display name doesn't rank high.
    let msg_query = format!("from:@{}", query);
    let msg_headers = build_headers(token, cookie)?;
    if let Ok(msg_resp) = client
        .get("https://slack.com/api/search.messages")
        .headers(msg_headers)
        .query(&[("query", msg_query.as_str()), ("count", "5"), ("sort", "timestamp"), ("sort_dir", "desc")])
        .send()
        .await
    {
        if let Ok(msg_text) = msg_resp.text().await {
            if let Ok(msg_raw) = serde_json::from_str::<serde_json::Value>(&msg_text) {
                if msg_raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                    if let Some(matches) = msg_raw
                        .get("messages")
                        .and_then(|m| m.get("matches"))
                        .and_then(|v| v.as_array())
                    {
                        for m in matches {
                            if let Some(username) = m.get("username").and_then(|v| v.as_str()) {
                                // Extract user_id from permalink or use username
                                let user_id = m.get("user").and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                if let Some(uid) = user_id {
                                    if seen_ids.insert(uid.clone()) {
                                        // We have handle but not real name — use handle as placeholder
                                        users.insert(0, SlackUser {
                                            id: uid,
                                            name: username.to_string(),
                                            real_name: username.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3) If we found a handle match via messages, try to get the real name via users.info
    for user in &mut users {
        if user.name == user.real_name && !user.id.is_empty() {
            // This user came from message search — enrich with users.info
            let info_headers = build_cookie_header(cookie)?;
            if let Ok(info_resp) = client
                .post("https://slack.com/api/users.info")
                .headers(info_headers)
                .form(&[("token", token), ("user", &user.id)])
                .send()
                .await
            {
                if let Ok(info_text) = info_resp.text().await {
                    if let Ok(info_raw) = serde_json::from_str::<serde_json::Value>(&info_text) {
                        if let Some(u) = info_raw.get("user") {
                            if let Some(rn) = u.get("profile").and_then(|p| p.get("real_name")).and_then(|v| v.as_str()) {
                                user.real_name = rn.to_string();
                            }
                            if let Some(n) = u.get("name").and_then(|v| v.as_str()) {
                                user.name = n.to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(users)
}

/// Search Slack channels live via the Slack API (searches ALL channels, not just member channels).
pub async fn search_channels_live(
    token: &str,
    cookie: &str,
    query: &str,
) -> Result<Vec<SlackChannel>, String> {
    let client = reqwest::Client::new();
    let headers = build_cookie_header(cookie)?;

    // Try conversations.search (internal Slack API)
    let response = client
        .post("https://slack.com/api/search.modules")
        .headers(headers)
        .form(&[
            ("token", token),
            ("query", query),
            ("module", "channels"),
            ("count", "20"),
        ])
        .send()
        .await
        .map_err(|e| format!("Slack search.modules failed: {}", e))?;

    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read search.modules response: {}", e))?;

    let raw: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        format!("Failed to parse search.modules JSON: {}", e)
    })?;

    let ok = raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let err = raw.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(format!("Slack search.modules error: {}", err));
    }

    let mut channels = Vec::new();

    // Try { items: [{ type: "channel", channel: { id, name, is_private } }] }
    if let Some(items) = raw.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let ch_obj = item.get("channel").unwrap_or(item);
            if let Some(id) = ch_obj.get("id").and_then(|v| v.as_str()) {
                let name = ch_obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let is_private = ch_obj
                    .get("is_private")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                channels.push(SlackChannel {
                    id: id.to_string(),
                    name: name.to_string(),
                    is_private,
                    updated: 0.0,
                });
            }
        }
    }
    // Try { channels: [...] } shape
    else if let Some(ch_arr) = raw.get("channels").and_then(|v| v.as_array()) {
        for ch_obj in ch_arr {
            if let Some(id) = ch_obj.get("id").and_then(|v| v.as_str()) {
                let name = ch_obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let is_private = ch_obj
                    .get("is_private")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                channels.push(SlackChannel {
                    id: id.to_string(),
                    name: name.to_string(),
                    is_private,
                    updated: 0.0,
                });
            }
        }
    }

    Ok(channels)
}

/// Test Slack connection and return workspace + user info.
pub async fn test_connection(token: &str, cookie: &str) -> Result<crate::models::SlackConnectionInfo, String> {
    let client = reqwest::Client::new();
    let headers = build_cookie_header(cookie)?;

    let response = client
        .post("https://slack.com/api/auth.test")
        .headers(headers)
        .form(&[("token", token)])
        .send()
        .await
        .map_err(|e| format!("auth.test failed: {}", e))?;

    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read auth.test response: {}", e))?;

    let raw: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse auth.test: {}", e))?;

    if !raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = raw
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(format!("auth.test error: {}", err));
    }

    let team = raw.get("team").and_then(|v| v.as_str()).unwrap_or("Unknown workspace").to_string();
    let user = raw.get("user").and_then(|v| v.as_str()).unwrap_or("Unknown user").to_string();
    let team_id = raw.get("team_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let user_id = raw.get("user_id").and_then(|v| v.as_str()).unwrap_or("").to_string();

    Ok(crate::models::SlackConnectionInfo { team, user, team_id, user_id })
}

/// Call auth.test to get the team ID for the current workspace.
pub async fn get_team_id(token: &str, cookie: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let headers = build_cookie_header(cookie)?;

    let response = client
        .post("https://slack.com/api/auth.test")
        .headers(headers)
        .form(&[("token", token)])
        .send()
        .await
        .map_err(|e| format!("auth.test failed: {}", e))?;

    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read auth.test response: {}", e))?;

    let raw: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse auth.test: {}", e))?;

    if !raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = raw
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(format!("auth.test error: {}", err));
    }

    raw.get("team_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "team_id missing from auth.test response".to_string())
}

/// Fetch Slack channels page by page, saving each page via callback.
/// Uses POST with token in form body (required for xoxc session tokens).
pub async fn fetch_slack_channels_paged<F>(
    token: &str,
    cookie: &str,
    mut on_page: F,
) -> Result<usize, String>
where
    F: FnMut(&[SlackChannel]) -> Result<(), String>,
{
    let client = reqwest::Client::new();
    let headers = build_cookie_header(cookie)?;

    let mut total = 0usize;
    let mut cursor = String::new();

    loop {
        let mut form_params = vec![
            ("token", token.to_string()),
            ("limit", "200".to_string()),
            ("types", "public_channel,private_channel".to_string()),
            ("exclude_archived", "true".to_string()),
        ];

        if !cursor.is_empty() {
            form_params.push(("cursor", cursor.clone()));
        }

        let response = client
            .post("https://slack.com/api/conversations.list")
            .headers(headers.clone())
            .form(&form_params)
            .send()
            .await
            .map_err(|e| format!("Slack conversations.list failed: {}", e))?;

        let text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read conversations.list response: {}", e))?;

        let data: ConversationsListResponse = serde_json::from_str(&text).map_err(|e| {
            let preview = if text.len() > 300 { &text[..300] } else { &text };
            format!("Failed to parse conversations.list: {} — response: {}", e, preview)
        })?;

        if !data.ok {
            return Err(format!(
                "Slack conversations.list error: {}",
                data.error.unwrap_or_else(|| "unknown".to_string())
            ));
        }

        if let Some(channels) = data.channels {
            let mut page_channels = Vec::new();
            for ch in channels {
                if ch.is_archived.unwrap_or(false) {
                    continue;
                }
                page_channels.push(SlackChannel {
                    id: ch.id,
                    name: ch.name,
                    is_private: ch.is_private.unwrap_or(false),
                    updated: ch.updated.unwrap_or(0.0),
                });
            }
            total += page_channels.len();
            on_page(&page_channels)?;
        }

        let next = data
            .response_metadata
            .and_then(|m| m.next_cursor)
            .unwrap_or_default();
        if next.is_empty() {
            break;
        }
        cursor = next;
    }

    Ok(total)
}

/// Fetch Slack users page by page, saving each page via callback.
/// Filters out bots and deactivated users.
pub async fn fetch_slack_users_paged<F>(
    token: &str,
    cookie: &str,
    mut on_page: F,
) -> Result<usize, String>
where
    F: FnMut(&[SlackUser]) -> Result<(), String>,
{
    let client = reqwest::Client::new();
    let headers = build_cookie_header(cookie)?;

    let mut total = 0usize;
    let mut cursor = String::new();

    loop {
        let mut form_params = vec![
            ("token", token.to_string()),
            ("limit", "200".to_string()),
        ];

        if !cursor.is_empty() {
            form_params.push(("cursor", cursor.clone()));
        }

        let response = client
            .post("https://slack.com/api/users.list")
            .headers(headers.clone())
            .form(&form_params)
            .send()
            .await
            .map_err(|e| format!("Slack users.list failed: {}", e))?;

        let text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read users.list response: {}", e))?;

        let data: UsersListResponse = serde_json::from_str(&text).map_err(|e| {
            let preview = if text.len() > 300 { &text[..300] } else { &text };
            format!("Failed to parse users.list: {} — response: {}", e, preview)
        })?;

        if !data.ok {
            return Err(format!(
                "Slack users.list error: {}",
                data.error.unwrap_or_else(|| "unknown".to_string())
            ));
        }

        if let Some(members) = data.members {
            let mut page_users = Vec::new();
            for m in members {
                if m.deleted.unwrap_or(false) || m.is_bot.unwrap_or(false) {
                    continue;
                }
                let real_name = m.profile
                    .as_ref()
                    .and_then(|p| p.real_name.clone())
                    .or(m.real_name)
                    .unwrap_or_default();
                let name = m.name.unwrap_or_default();
                if name.is_empty() && real_name.is_empty() {
                    continue;
                }
                page_users.push(SlackUser {
                    id: m.id,
                    name,
                    real_name,
                });
            }
            total += page_users.len();
            on_page(&page_users)?;
        }

        let next = data
            .response_metadata
            .and_then(|m| m.next_cursor)
            .unwrap_or_default();
        if next.is_empty() {
            break;
        }
        cursor = next;
    }

    Ok(total)
}

/// Fetch recent DM conversation partner user IDs (no pagination, single page).
/// Returns user IDs ordered by most recent activity.
pub async fn fetch_recent_dm_user_ids(
    token: &str,
    cookie: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();
    let headers = build_cookie_header(cookie)?;

    let form_params = vec![
        ("token", token.to_string()),
        ("limit", limit.to_string()),
        ("types", "im".to_string()),
        ("exclude_archived", "true".to_string()),
    ];

    let response = client
        .post("https://slack.com/api/conversations.list")
        .headers(headers)
        .form(&form_params)
        .send()
        .await
        .map_err(|e| format!("Slack conversations.list (im) failed: {}", e))?;

    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read conversations.list (im) response: {}", e))?;

    let data: ImConversationsListResponse = serde_json::from_str(&text).map_err(|e| {
        let preview = if text.len() > 300 { &text[..300] } else { &text };
        format!("Failed to parse conversations.list (im): {} — response: {}", e, preview)
    })?;

    if !data.ok {
        return Err(format!(
            "Slack conversations.list (im) error: {}",
            data.error.unwrap_or_else(|| "unknown".to_string())
        ));
    }

    let user_ids: Vec<String> = data
        .channels
        .unwrap_or_default()
        .into_iter()
        .map(|im| im.user)
        .filter(|uid| uid != "USLACKBOT")
        .collect();

    Ok(user_ids)
}

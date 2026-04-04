use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub source: String,
    pub sender: String,
    pub subject: Option<String>,
    pub body: String,
    pub body_html: Option<String>,
    pub permalink: Option<String>,
    pub timestamp: i64,
    pub classification: String,
    pub status: String,
    pub starred: bool,
    pub snoozed_until: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackUser {
    pub id: String,
    pub name: String,
    pub real_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackChannel {
    pub id: String,
    pub name: String,
    pub is_private: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackFilter {
    pub filter_type: String, // "user" or "channel"
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub builtin: bool,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRule {
    pub category: String,
    pub rule_type: String, // "keyword", "sender", "channel"
    pub value: String,
    pub id: Option<String>, // Slack ID for sender/channel rules
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub slack_token: Option<String>,
    pub slack_cookie: Option<String>,
    pub claude_api_key: Option<String>,
    pub classification_prompt: Option<String>,
    pub slack_filters: Option<Vec<SlackFilter>>,
    pub categories: Option<Vec<Category>>,
    pub category_rules: Option<Vec<CategoryRule>>,
    pub theme: Option<String>,
    pub font: Option<String>,
    pub open_in_slack_app: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            slack_token: None,
            slack_cookie: None,
            claude_api_key: None,
            classification_prompt: Some(
                "Classify each message. If it's relevant to Clay / Mesh, or asks to check a box or respond, classify as 'important'. Otherwise 'other'.".to_string()
            ),
            slack_filters: None,
            categories: None,
            category_rules: None,
            theme: Some("dark".to_string()),
            font: Some("system".to_string()),
            open_in_slack_app: Some(false),
        }
    }
}

impl Settings {
    pub fn effective_categories(&self) -> Vec<Category> {
        self.categories.clone().unwrap_or_else(|| vec![
            Category { name: "important".to_string(), builtin: true, position: 0 },
            Category { name: "other".to_string(), builtin: true, position: 1 },
        ])
    }

    pub fn effective_rules(&self) -> Vec<CategoryRule> {
        self.category_rules.clone().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResult {
    pub new_messages: usize,
    pub classified: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageCounts {
    pub counts: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackCacheStatus {
    pub user_count: usize,
    pub channel_count: usize,
}

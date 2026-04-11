use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_IMPORTANT_DESCRIPTION: &str = "Messages that require direct attention or action — decisions needed, urgent requests, escalations, and messages that need a response.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub source: String,
    pub sender: String,
    pub subject: Option<String>,
    pub body: String,
    pub body_html: Option<String>,
    pub permalink: Option<String>,
    pub avatar_url: Option<String>,
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
    #[serde(default)]
    pub updated: f64,
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
    #[serde(default)]
    pub description: Option<String>,
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
    pub slack_filters: Option<Vec<SlackFilter>>,
    pub categories: Option<Vec<Category>>,
    pub category_rules: Option<Vec<CategoryRule>>,
    pub theme: Option<String>,
    pub font: Option<String>,
    pub font_size: Option<String>,
    pub open_in_slack_app: Option<bool>,
    pub notifications_enabled: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            slack_token: None,
            slack_cookie: None,
            claude_api_key: None,
            slack_filters: None,
            categories: None,
            category_rules: None,
            theme: Some("dark".to_string()),
            font: Some("system".to_string()),
            font_size: Some("s".to_string()),
            open_in_slack_app: Some(false),
            notifications_enabled: Some(true),
        }
    }
}

impl Settings {
    pub fn effective_categories(&self) -> Vec<Category> {
        match self.categories.clone() {
            Some(mut cats) => {
                for cat in &mut cats {
                    if cat.name == "important" && cat.description.is_none() {
                        cat.description = Some(DEFAULT_IMPORTANT_DESCRIPTION.to_string());
                    }
                }
                cats
            }
            None => vec![
                Category {
                    name: "important".to_string(),
                    builtin: true,
                    position: 0,
                    description: Some(DEFAULT_IMPORTANT_DESCRIPTION.to_string()),
                },
                Category {
                    name: "other".to_string(),
                    builtin: true,
                    position: 1,
                    description: None,
                },
            ],
        }
    }

    pub fn effective_rules(&self) -> Vec<CategoryRule> {
        self.category_rules.clone().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveSettingsResult {
    pub classifications_reset: bool,
    pub filters_cleaned: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingSuggestions {
    pub suggested_people: Vec<SlackUser>,
    pub suggested_channels: Vec<SlackChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConnectionInfo {
    pub team: String,
    pub user: String,
    pub team_id: String,
    pub user_id: String,
}

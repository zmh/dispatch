use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::{Category, CategoryRule, Message, MessageCounts, Settings, SlackCacheStatus, SlackChannel, SlackFilter, SlackUser, DEFAULT_IMPORTANT_DESCRIPTION};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                sender TEXT NOT NULL,
                subject TEXT,
                body TEXT NOT NULL,
                body_html TEXT,
                permalink TEXT,
                timestamp INTEGER NOT NULL,
                classification TEXT DEFAULT 'other',
                status TEXT DEFAULT 'inbox',
                starred INTEGER DEFAULT 0,
                snoozed_until INTEGER,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS slack_users_cache (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                real_name TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS slack_channels_cache (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                is_private INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_messages_status ON messages(status);
            CREATE INDEX IF NOT EXISTS idx_messages_classification ON messages(classification);
            CREATE INDEX IF NOT EXISTS idx_messages_snoozed ON messages(snoozed_until);",
        )
        .map_err(|e| e.to_string())?;

        // Migration: add avatar_url column if missing
        let has_avatar_url: bool = conn
            .prepare("SELECT avatar_url FROM messages LIMIT 0")
            .is_ok();
        if !has_avatar_url {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN avatar_url TEXT;")
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    pub fn insert_message(&self, msg: &Message) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let result = conn.execute(
            "INSERT OR IGNORE INTO messages (id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, snoozed_until, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                msg.id,
                msg.source,
                msg.sender,
                msg.subject,
                msg.body,
                msg.body_html,
                msg.permalink,
                msg.avatar_url,
                msg.timestamp,
                msg.classification,
                msg.status,
                msg.starred as i32,
                msg.snoozed_until,
                msg.created_at,
            ],
        ).map_err(|e| e.to_string())?;
        Ok(result > 0)
    }

    pub fn get_messages(&self, classification: &str, status: &str) -> Result<Vec<Message>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // First, unsnoze any messages whose snooze time has passed
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "UPDATE messages SET status = 'inbox', snoozed_until = NULL WHERE status = 'snoozed' AND snoozed_until IS NOT NULL AND snoozed_until <= ?1",
            params![now],
        ).map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare(
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, snoozed_until, created_at
                 FROM messages
                 WHERE classification = ?1 AND status = ?2
                 ORDER BY timestamp DESC",
            )
            .map_err(|e| e.to_string())?;

        let messages = stmt
            .query_map(params![classification, status], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    sender: row.get(2)?,
                    subject: row.get(3)?,
                    body: row.get(4)?,
                    body_html: row.get(5)?,
                    permalink: row.get(6)?,
                    avatar_url: row.get(7)?,
                    timestamp: row.get(8)?,
                    classification: row.get(9)?,
                    status: row.get(10)?,
                    starred: row.get::<_, i32>(11)? != 0,
                    snoozed_until: row.get(12)?,
                    created_at: row.get(13)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(messages)
    }

    pub fn get_messages_by_status(&self, status: &str) -> Result<Vec<Message>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // Unsnoze any messages whose snooze time has passed
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "UPDATE messages SET status = 'inbox', snoozed_until = NULL WHERE status = 'snoozed' AND snoozed_until IS NOT NULL AND snoozed_until <= ?1",
            params![now],
        ).map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare(
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, snoozed_until, created_at
                 FROM messages
                 WHERE status = ?1
                 ORDER BY timestamp DESC",
            )
            .map_err(|e| e.to_string())?;

        let messages = stmt
            .query_map(params![status], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    sender: row.get(2)?,
                    subject: row.get(3)?,
                    body: row.get(4)?,
                    body_html: row.get(5)?,
                    permalink: row.get(6)?,
                    avatar_url: row.get(7)?,
                    timestamp: row.get(8)?,
                    classification: row.get(9)?,
                    status: row.get(10)?,
                    starred: row.get::<_, i32>(11)? != 0,
                    snoozed_until: row.get(12)?,
                    created_at: row.get(13)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(messages)
    }

    pub fn get_starred_messages(&self) -> Result<Vec<Message>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, snoozed_until, created_at
                 FROM messages
                 WHERE starred = 1
                 ORDER BY timestamp DESC",
            )
            .map_err(|e| e.to_string())?;

        let messages = stmt
            .query_map([], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    sender: row.get(2)?,
                    subject: row.get(3)?,
                    body: row.get(4)?,
                    body_html: row.get(5)?,
                    permalink: row.get(6)?,
                    avatar_url: row.get(7)?,
                    timestamp: row.get(8)?,
                    classification: row.get(9)?,
                    status: row.get(10)?,
                    starred: row.get::<_, i32>(11)? != 0,
                    snoozed_until: row.get(12)?,
                    created_at: row.get(13)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(messages)
    }

    pub fn get_starred_count(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM messages WHERE starred = 1", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count)
    }

    pub fn get_message_counts(&self, status: &str) -> Result<MessageCounts, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare(
                "SELECT classification, COUNT(*) FROM messages WHERE status = ?1 GROUP BY classification",
            )
            .map_err(|e| e.to_string())?;

        let mut counts = HashMap::new();
        let rows = stmt
            .query_map(params![status], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })
            .map_err(|e| e.to_string())?;

        for row in rows {
            let (classification, count) = row.map_err(|e| e.to_string())?;
            counts.insert(classification, count);
        }

        // Add starred count (across all statuses)
        let starred_count: usize = conn
            .query_row("SELECT COUNT(*) FROM messages WHERE starred = 1", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        counts.insert("starred".to_string(), starred_count);

        Ok(MessageCounts { counts })
    }

    pub fn mark_done_message(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET status = 'archived' WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn snooze_message(&self, id: &str, until: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET status = 'snoozed', snoozed_until = ?2 WHERE id = ?1",
            params![id, until],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn toggle_star(&self, id: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let current: i32 = conn
            .query_row(
                "SELECT starred FROM messages WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let new_val = if current == 0 { 1 } else { 0 };
        conn.execute(
            "UPDATE messages SET starred = ?2 WHERE id = ?1",
            params![id, new_val],
        )
        .map_err(|e| e.to_string())?;
        Ok(new_val != 0)
    }

    pub fn update_classification(&self, id: &str, classification: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET classification = ?2 WHERE id = ?1",
            params![id, classification],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_unclassified_messages(&self) -> Result<Vec<Message>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, snoozed_until, created_at
                 FROM messages
                 WHERE classification = 'unclassified'
                 ORDER BY timestamp DESC",
            )
            .map_err(|e| e.to_string())?;

        let messages = stmt
            .query_map([], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    sender: row.get(2)?,
                    subject: row.get(3)?,
                    body: row.get(4)?,
                    body_html: row.get(5)?,
                    permalink: row.get(6)?,
                    avatar_url: row.get(7)?,
                    timestamp: row.get(8)?,
                    classification: row.get(9)?,
                    status: row.get(10)?,
                    starred: row.get::<_, i32>(11)? != 0,
                    snoozed_until: row.get(12)?,
                    created_at: row.get(13)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(messages)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let result = conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        );
        match result {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<Settings, String> {
        let slack_filters: Option<Vec<SlackFilter>> = self
            .get_setting("slack_filters")?
            .and_then(|v| serde_json::from_str(&v).ok());
        let categories: Option<Vec<Category>> = self
            .get_setting("categories")?
            .and_then(|v| serde_json::from_str(&v).ok());
        let category_rules: Option<Vec<CategoryRule>> = self
            .get_setting("category_rules")?
            .and_then(|v| serde_json::from_str(&v).ok());

        // One-time migration: if old classification_prompt exists, copy it into
        // "important"'s description (if no category has a description yet), then delete the key.
        let old_prompt = self.get_setting("classification_prompt")?;
        let mut categories = categories;
        if let Some(ref prompt) = old_prompt {
            let needs_migration = categories.as_ref().map_or(true, |cats| {
                cats.iter().all(|c| c.description.is_none())
            });
            if needs_migration {
                let mut cats = categories.unwrap_or_else(|| vec![
                    Category {
                        name: "important".to_string(),
                        builtin: true,
                        position: 0,
                        description: None,
                    },
                    Category {
                        name: "other".to_string(),
                        builtin: true,
                        position: 1,
                        description: None,
                    },
                ]);
                for cat in &mut cats {
                    if cat.name == "important" {
                        cat.description = Some(prompt.clone());
                    }
                }
                let json = serde_json::to_string(&cats).map_err(|e| e.to_string())?;
                self.set_setting("categories", &json)?;
                categories = Some(cats);
            }
            // Always delete the old key so migration never re-triggers
            self.delete_setting("classification_prompt")?;
        }

        // Fill in default description for "important" if missing, and persist so
        // save_settings() won't detect a spurious "change" on first save.
        if let Some(ref mut cats) = categories {
            let mut patched = false;
            for cat in cats.iter_mut() {
                if cat.name == "important" && cat.description.is_none() {
                    cat.description = Some(DEFAULT_IMPORTANT_DESCRIPTION.to_string());
                    patched = true;
                }
            }
            if patched {
                let json = serde_json::to_string(&*cats).map_err(|e| e.to_string())?;
                self.set_setting("categories", &json)?;
            }
        }

        Ok(Settings {
            slack_token: self.get_setting("slack_token")?,
            slack_cookie: self.get_setting("slack_cookie")?,
            claude_api_key: self.get_setting("claude_api_key")?,
            slack_filters,
            categories,
            category_rules,
            theme: self.get_setting("theme")?.or_else(|| Some("dark".to_string())),
            font: self.get_setting("font")?.or_else(|| Some("system".to_string())),
            font_size: self.get_setting("font_size")?.or_else(|| Some("s".to_string())),
            open_in_slack_app: self.get_setting("open_in_slack_app")?.map(|v| v == "true").or(Some(false)),
            notifications_enabled: self.get_setting("notifications_enabled")?.map(|v| v == "true").or(Some(true)),
        })
    }

    /// Save settings. Returns `true` if classifications were reset (descriptions changed).
    pub fn save_settings(&self, settings: &Settings) -> Result<bool, String> {
        let mut classifications_reset = false;

        if let Some(ref val) = settings.slack_token {
            self.set_setting("slack_token", val)?;
        }
        if let Some(ref val) = settings.slack_cookie {
            self.set_setting("slack_cookie", val)?;
        }
        if let Some(ref val) = settings.claude_api_key {
            self.set_setting("claude_api_key", val)?;
        }

        // Save slack filters as JSON
        if let Some(ref filters) = settings.slack_filters {
            let json = serde_json::to_string(filters).map_err(|e| e.to_string())?;
            self.set_setting("slack_filters", &json)?;
        }

        // Save categories — detect description changes and reassign removed categories
        let old_categories = self.get_setting("categories")?;
        let old_rules = self.get_setting("category_rules")?;

        if let Some(ref cats) = settings.categories {
            let json = serde_json::to_string(cats).map_err(|e| e.to_string())?;
            if old_categories.as_deref() != Some(&json) {
                // Check if descriptions changed for existing categories (triggers reclassification).
                // New categories don't count — they have no messages yet.
                let descriptions_changed = if let Some(ref old_json) = old_categories {
                    if let Ok(old_cats) = serde_json::from_str::<Vec<Category>>(old_json) {
                        let old_descs: std::collections::HashMap<&str, Option<&str>> = old_cats
                            .iter()
                            .map(|c| (c.name.as_str(), c.description.as_deref()))
                            .collect();
                        cats.iter().any(|c| {
                            match old_descs.get(c.name.as_str()) {
                                Some(old_desc) => old_desc != &c.description.as_deref(),
                                None => false, // new category, not a description change
                            }
                        })
                    } else {
                        false
                    }
                } else {
                    false
                };

                // Find removed categories and reassign their messages to "other"
                if let Some(ref old_json) = old_categories {
                    if let Ok(old_cats) = serde_json::from_str::<Vec<Category>>(old_json) {
                        let new_names: Vec<String> = cats.iter().map(|c| c.name.clone()).collect();
                        for old_cat in &old_cats {
                            if !new_names.contains(&old_cat.name) {
                                self.reassign_category_to_other(&old_cat.name)?;
                            }
                        }
                    }
                }
                self.set_setting("categories", &json)?;

                if descriptions_changed {
                    self.reset_classifications()?;
                    classifications_reset = true;
                }
            }
        }

        // Save rules — only reclassify messages in categories whose rules changed
        if let Some(ref rules) = settings.category_rules {
            let json = serde_json::to_string(rules).map_err(|e| e.to_string())?;
            if old_rules.as_deref() != Some(&json) {
                self.set_setting("category_rules", &json)?;
            }
        }

        if let Some(ref val) = settings.theme {
            self.set_setting("theme", val)?;
        }
        if let Some(ref val) = settings.font {
            self.set_setting("font", val)?;
        }
        if let Some(ref val) = settings.font_size {
            self.set_setting("font_size", val)?;
        }
        if let Some(val) = settings.open_in_slack_app {
            self.set_setting("open_in_slack_app", if val { "true" } else { "false" })?;
        }
        if let Some(val) = settings.notifications_enabled {
            self.set_setting("notifications_enabled", if val { "true" } else { "false" })?;
        }

        Ok(classifications_reset)
    }

    pub fn unsnooze_due_messages(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let count = conn.execute(
            "UPDATE messages SET status = 'inbox', snoozed_until = NULL WHERE status = 'snoozed' AND snoozed_until IS NOT NULL AND snoozed_until <= ?1",
            params![now],
        ).map_err(|e| e.to_string())?;
        Ok(count)
    }

    pub fn reassign_category_to_other(&self, category: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET classification = 'other' WHERE classification = ?1",
            params![category],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn reset_classifications(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET classification = 'unclassified' WHERE status = 'inbox'",
            [],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    // Slack cache methods

    pub fn clear_slack_cache(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM slack_users_cache", [])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM slack_channels_cache", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn append_slack_users(&self, users: &[SlackUser]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("INSERT OR REPLACE INTO slack_users_cache (id, name, real_name) VALUES (?1, ?2, ?3)")
            .map_err(|e| e.to_string())?;
        for user in users {
            stmt.execute(params![user.id, user.name, user.real_name])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn append_slack_channels(&self, channels: &[SlackChannel]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("INSERT OR REPLACE INTO slack_channels_cache (id, name, is_private) VALUES (?1, ?2, ?3)")
            .map_err(|e| e.to_string())?;
        for ch in channels {
            stmt.execute(params![ch.id, ch.name, ch.is_private as i32])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn search_slack_users(&self, query: &str) -> Result<Vec<SlackUser>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let query_lower = query.to_lowercase();
        let pattern = format!("%{}%", query_lower);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, real_name FROM slack_users_cache
                 WHERE LOWER(name) LIKE ?1 OR LOWER(real_name) LIKE ?1
                 ORDER BY
                   CASE WHEN LOWER(name) = ?2 OR LOWER(real_name) = ?2 THEN 0
                        WHEN LOWER(name) LIKE ?2 || '%' OR LOWER(real_name) LIKE ?2 || '%' THEN 1
                        ELSE 2 END,
                   real_name ASC
                 LIMIT 20",
            )
            .map_err(|e| e.to_string())?;

        let users = stmt
            .query_map(params![pattern, query_lower], |row| {
                Ok(SlackUser {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    real_name: row.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(users)
    }

    pub fn search_slack_channels(&self, query: &str) -> Result<Vec<SlackChannel>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let query_lower = query.to_lowercase();
        let pattern = format!("%{}%", query_lower);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, is_private FROM slack_channels_cache
                 WHERE LOWER(name) LIKE ?1
                 ORDER BY
                   CASE WHEN LOWER(name) = ?2 THEN 0
                        WHEN LOWER(name) LIKE ?2 || '%' THEN 1
                        ELSE 2 END,
                   name ASC
                 LIMIT 20",
            )
            .map_err(|e| e.to_string())?;

        let channels = stmt
            .query_map(params![pattern, query_lower], |row| {
                Ok(SlackChannel {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_private: row.get::<_, i32>(2)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(channels)
    }

    pub fn slack_cache_count(&self) -> Result<SlackCacheStatus, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let user_count: usize = conn
            .query_row("SELECT COUNT(*) FROM slack_users_cache", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        let channel_count: usize = conn
            .query_row("SELECT COUNT(*) FROM slack_channels_cache", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(SlackCacheStatus { user_count, channel_count })
    }
}

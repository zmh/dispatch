use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::{
    Category, CategoryRule, Message, MessageCounts, SaveSettingsResult, Settings, SlackCacheStatus,
    SlackChannel, SlackFilter, SlackUser, DEFAULT_IMPORTANT_DESCRIPTION,
};

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Default, Clone)]
pub struct MessageUpsertResult {
    pub new_ids: Vec<String>,
    pub changed_ids: Vec<String>,
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
                unread INTEGER DEFAULT 0,
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
                real_name TEXT NOT NULL,
                avatar_url TEXT
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

        // Migration: add unread column if missing
        let has_unread: bool = conn
            .prepare("SELECT unread FROM messages LIMIT 0")
            .is_ok();
        if !has_unread {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN unread INTEGER DEFAULT 0;")
                .map_err(|e| e.to_string())?;
        }

        // Migration: add updated column to slack_channels_cache if missing
        let has_updated: bool = conn
            .prepare("SELECT updated FROM slack_channels_cache LIMIT 0")
            .is_ok();
        if !has_updated {
            conn.execute_batch(
                "ALTER TABLE slack_channels_cache ADD COLUMN updated REAL DEFAULT 0;",
            )
            .map_err(|e| e.to_string())?;
        }

        // Migration: add avatar_url column to slack_users_cache if missing
        let has_user_avatar_url: bool = conn
            .prepare("SELECT avatar_url FROM slack_users_cache LIMIT 0")
            .is_ok();
        if !has_user_avatar_url {
            conn.execute_batch("ALTER TABLE slack_users_cache ADD COLUMN avatar_url TEXT;")
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    pub fn insert_message(&self, msg: &Message) -> Result<bool, String> {
        let upserted = self.upsert_messages_batch(std::slice::from_ref(msg))?;
        Ok(!upserted.new_ids.is_empty())
    }

    pub fn upsert_messages_batch(
        &self,
        messages: &[Message],
    ) -> Result<MessageUpsertResult, String> {
        if messages.is_empty() {
            return Ok(MessageUpsertResult::default());
        }

        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut result = MessageUpsertResult::default();

        let mut select_stmt = tx
            .prepare(
                "SELECT source, sender, subject, body, body_html, permalink, avatar_url, timestamp
                 FROM messages
                 WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut upsert_stmt = tx
            .prepare(
                "INSERT INTO messages (id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(id) DO UPDATE SET
                    source = excluded.source,
                    sender = excluded.sender,
                    subject = excluded.subject,
                    body = excluded.body,
                    body_html = excluded.body_html,
                    permalink = COALESCE(excluded.permalink, messages.permalink),
                    avatar_url = COALESCE(excluded.avatar_url, messages.avatar_url),
                    timestamp = excluded.timestamp",
            )
            .map_err(|e| e.to_string())?;

        for msg in messages {
            let existing = select_stmt
                .query_row(params![msg.id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                })
                .optional()
                .map_err(|e| e.to_string())?;

            if let Some((
                source,
                sender,
                subject,
                body,
                body_html,
                permalink,
                avatar_url,
                timestamp,
            )) = existing
            {
                let expected_permalink = msg.permalink.clone().or(permalink.clone());
                let expected_avatar_url = msg.avatar_url.clone().or(avatar_url.clone());
                let changed = source != msg.source
                    || sender != msg.sender
                    || subject != msg.subject
                    || body != msg.body
                    || body_html != msg.body_html
                    || permalink != expected_permalink
                    || avatar_url != expected_avatar_url
                    || timestamp != msg.timestamp;
                if changed {
                    result.changed_ids.push(msg.id.clone());
                }
            } else {
                result.new_ids.push(msg.id.clone());
            }

            upsert_stmt
                .execute(params![
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
                    msg.unread as i32,
                    msg.snoozed_until,
                    msg.created_at,
                ])
                .map_err(|e| e.to_string())?;
        }

        drop(upsert_stmt);
        drop(select_stmt);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
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

        let sql = if classification == "other" {
            "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at
             FROM messages
             WHERE status = ?1 AND (classification = 'other' OR classification = 'unclassified')
             ORDER BY timestamp DESC"
        } else {
            "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at
             FROM messages
             WHERE classification = ?1 AND status = ?2
             ORDER BY timestamp DESC"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

        let messages = if classification == "other" {
            stmt.query_map(params![status], |row| {
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
                    unread: row.get::<_, i32>(12)? != 0,
                    snoozed_until: row.get(13)?,
                    created_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        } else {
            stmt.query_map(params![classification, status], |row| {
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
                    unread: row.get::<_, i32>(12)? != 0,
                    snoozed_until: row.get(13)?,
                    created_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };

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
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at
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
                    unread: row.get::<_, i32>(12)? != 0,
                    snoozed_until: row.get(13)?,
                    created_at: row.get(14)?,
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
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at
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
                    unread: row.get::<_, i32>(12)? != 0,
                    snoozed_until: row.get(13)?,
                    created_at: row.get(14)?,
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
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE starred = 1",
                [],
                |row| row.get(0),
            )
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

        if status == "inbox" {
            let unclassified = counts.remove("unclassified").unwrap_or(0);
            if unclassified > 0 {
                let other = counts.entry("other".to_string()).or_insert(0);
                *other += unclassified;
            }
        }

        // Add starred count (across all statuses)
        let starred_count: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE starred = 1",
                [],
                |row| row.get(0),
            )
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

    pub fn set_unread_message(&self, id: &str, unread: bool) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let current: i32 = conn
            .query_row(
                "SELECT unread FROM messages WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let target = if unread { 1 } else { 0 };
        if current != target {
            conn.execute(
                "UPDATE messages SET unread = ?2 WHERE id = ?1",
                params![id, target],
            )
            .map_err(|e| e.to_string())?;
        }

        Ok(unread)
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
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at
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
                    unread: row.get::<_, i32>(12)? != 0,
                    snoozed_until: row.get(13)?,
                    created_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(messages)
    }

    pub fn get_unclassified_messages_by_ids(&self, ids: &[String]) -> Result<Vec<Message>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at
             FROM messages
             WHERE classification = 'unclassified'
               AND status = 'inbox'
               AND id IN ({})
             ORDER BY timestamp DESC",
            placeholders.join(", ")
        );

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
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
                    unread: row.get::<_, i32>(12)? != 0,
                    snoozed_until: row.get(13)?,
                    created_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn get_unclassified_messages_limited(&self, limit: usize) -> Result<Vec<Message>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, source, sender, subject, body, body_html, permalink, avatar_url, timestamp, classification, status, starred, unread, snoozed_until, created_at
                 FROM messages
                 WHERE classification = 'unclassified' AND status = 'inbox'
                 ORDER BY timestamp DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![limit as i64], |row| {
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
                    unread: row.get::<_, i32>(12)? != 0,
                    snoozed_until: row.get(13)?,
                    created_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn get_unclassified_inbox_count(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let count: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE classification = 'unclassified' AND status = 'inbox'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count)
    }

    pub fn update_classifications_batch(
        &self,
        classifications: &[(String, String)],
    ) -> Result<usize, String> {
        if classifications.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut updated = 0usize;
        let mut stmt = tx
            .prepare("UPDATE messages SET classification = ?2 WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        for (id, class) in classifications {
            updated += stmt
                .execute(params![id, class])
                .map_err(|e| e.to_string())?;
        }
        drop(stmt);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(updated)
    }

    pub fn set_messages_to_other_by_ids(&self, ids: &[String]) -> Result<usize, String> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut updated = 0usize;
        let mut stmt = tx
            .prepare("UPDATE messages SET classification = 'other' WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        for id in ids {
            updated += stmt.execute(params![id]).map_err(|e| e.to_string())?;
        }
        drop(stmt);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(updated)
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
            let needs_migration = categories
                .as_ref()
                .map_or(true, |cats| cats.iter().all(|c| c.description.is_none()));
            if needs_migration {
                let mut cats = categories.unwrap_or_else(|| {
                    vec![
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
                    ]
                });
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
            theme: self
                .get_setting("theme")?
                .or_else(|| Some("dark".to_string())),
            font: self
                .get_setting("font")?
                .or_else(|| Some("system".to_string())),
            font_size: self
                .get_setting("font_size")?
                .or_else(|| Some("s".to_string())),
            open_in_slack_app: self
                .get_setting("open_in_slack_app")?
                .map(|v| v == "true")
                .or(Some(false)),
            notifications_enabled: self
                .get_setting("notifications_enabled")?
                .map(|v| v == "true")
                .or(Some(true)),
            beta_release_channel: self
                .get_setting("beta_release_channel")?
                .map(|v| v == "true")
                .or(Some(false)),
            after_archive: self
                .get_setting("after_archive")?
                .or_else(|| Some("newer".to_string())),
        })
    }

    /// Save settings. Returns result indicating if classifications were reset or filters were cleaned.
    pub fn save_settings(&self, settings: &Settings) -> Result<SaveSettingsResult, String> {
        let mut classifications_reset = false;
        let mut filters_cleaned = false;

        if let Some(ref val) = settings.slack_token {
            self.set_setting("slack_token", val)?;
        }
        if let Some(ref val) = settings.slack_cookie {
            self.set_setting("slack_cookie", val)?;
        }
        if let Some(ref val) = settings.claude_api_key {
            self.set_setting("claude_api_key", val)?;
        }

        // Save slack filters as JSON — archive messages from removed filters
        if let Some(ref filters) = settings.slack_filters {
            let json = serde_json::to_string(filters).map_err(|e| e.to_string())?;

            // Detect removed filters and archive their messages
            let old_filters_json = self.get_setting("slack_filters")?;
            if let Some(ref old_json) = old_filters_json {
                if let Ok(old_filters) = serde_json::from_str::<Vec<SlackFilter>>(old_json) {
                    let new_ids: Vec<&str> = filters.iter().map(|f| f.id.as_str()).collect();
                    for old_filter in &old_filters {
                        if !new_ids.contains(&old_filter.id.as_str()) {
                            if self.archive_messages_for_removed_filter(old_filter)? > 0 {
                                filters_cleaned = true;
                            }
                        }
                    }
                }
            }

            if old_filters_json.as_deref() != Some(&json) {
                self.set_setting("slack_filters", &json)?;
                // Filter changes invalidate query checkpoints.
                let _ = self.delete_setting("slack_incremental_state_v1");
            }
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
        if let Some(val) = settings.beta_release_channel {
            self.set_setting("beta_release_channel", if val { "true" } else { "false" })?;
        }
        if let Some(ref val) = settings.after_archive {
            self.set_setting("after_archive", val)?;
        }

        Ok(SaveSettingsResult {
            classifications_reset,
            filters_cleaned,
        })
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

    pub fn archive_messages_for_removed_filter(
        &self,
        filter: &SlackFilter,
    ) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let count = match filter.filter_type.as_str() {
            "channel" => {
                let name_without_hash = filter.display_name.trim_start_matches('#');
                let name_with_hash = format!("#{}", name_without_hash);
                conn.execute(
                    "UPDATE messages SET status = 'archived' WHERE status = 'inbox' AND (subject = ?1 OR subject = ?2)",
                    params![name_with_hash, name_without_hash],
                ).map_err(|e| e.to_string())?
            }
            "user" => {
                // Look up the Slack username from cache — sender stores username, not display_name
                let username: Option<String> = conn
                    .query_row(
                        "SELECT name FROM slack_users_cache WHERE id = ?1",
                        params![filter.id],
                        |row| row.get(0),
                    )
                    .ok();
                let sender_name = username.as_deref().unwrap_or(&filter.display_name);
                conn.execute(
                    "UPDATE messages SET status = 'archived' WHERE status = 'inbox' AND sender = ?1",
                    params![sender_name],
                ).map_err(|e| e.to_string())?
            }
            _ => 0,
        };
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

    pub fn append_slack_users(&self, users: &[SlackUser]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("INSERT OR REPLACE INTO slack_users_cache (id, name, real_name, avatar_url) VALUES (?1, ?2, ?3, ?4)")
            .map_err(|e| e.to_string())?;
        for u in users {
            stmt.execute(params![u.id, u.name, u.real_name, u.avatar_url])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn append_slack_channels(&self, channels: &[SlackChannel]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("INSERT OR REPLACE INTO slack_channels_cache (id, name, is_private, updated) VALUES (?1, ?2, ?3, ?4)")
            .map_err(|e| e.to_string())?;
        for ch in channels {
            stmt.execute(params![ch.id, ch.name, ch.is_private as i32, ch.updated])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn get_slack_user_avatars(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, String>, String> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT id, avatar_url FROM slack_users_cache WHERE id IN ({}) AND avatar_url IS NOT NULL",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, avatar) = row.map_err(|e| e.to_string())?;
            out.insert(id, avatar);
        }
        Ok(out)
    }

    pub fn upsert_slack_user_avatar(
        &self,
        user_id: &str,
        username: Option<&str>,
        real_name: Option<&str>,
        avatar_url: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let existing = conn
            .query_row(
                "SELECT name, real_name FROM slack_users_cache WHERE id = ?1",
                params![user_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let existing_name = existing.as_ref().map(|v| v.0.as_str()).unwrap_or_default();
        let existing_real_name = existing.as_ref().map(|v| v.1.as_str()).unwrap_or_default();
        let final_name = username.unwrap_or(existing_name);
        let final_real_name = real_name.unwrap_or(existing_real_name);

        conn.execute(
            "INSERT INTO slack_users_cache (id, name, real_name, avatar_url)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                name = COALESCE(NULLIF(excluded.name, ''), slack_users_cache.name),
                real_name = COALESCE(NULLIF(excluded.real_name, ''), slack_users_cache.real_name),
                avatar_url = excluded.avatar_url",
            params![user_id, final_name, final_real_name, avatar_url],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_message_avatars_by_ids(
        &self,
        ids: &[String],
        avatar_url: &str,
    ) -> Result<usize, String> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut updated = 0usize;
        let mut stmt = tx
            .prepare(
                "UPDATE messages
                 SET avatar_url = ?2
                 WHERE source = 'slack'
                   AND id = ?1
                   AND (avatar_url IS NULL OR avatar_url = '')",
            )
            .map_err(|e| e.to_string())?;
        for id in ids {
            updated += stmt
                .execute(params![id, avatar_url])
                .map_err(|e| e.to_string())?;
        }
        drop(stmt);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(updated)
    }

    pub fn search_slack_users(&self, query: &str) -> Result<Vec<SlackUser>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let query_lower = query.to_lowercase();
        let pattern = format!("%{}%", query_lower);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, real_name, avatar_url FROM slack_users_cache
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
                    avatar_url: row.get(3)?,
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
                    updated: 0.0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(channels)
    }

    pub fn get_slack_users_by_ids(&self, ids: &[String]) -> Result<Vec<SlackUser>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT id, name, real_name, avatar_url FROM slack_users_cache WHERE id IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(SlackUser {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    real_name: row.get(2)?,
                    avatar_url: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        // Preserve input ordering (DM recency)
        let map: HashMap<String, SlackUser> = rows.into_iter().map(|u| (u.id.clone(), u)).collect();
        Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
    }

    pub fn get_suggested_channels(&self, limit: usize) -> Result<Vec<SlackChannel>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, is_private, updated FROM slack_channels_cache
                 ORDER BY updated DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let channels = stmt
            .query_map(params![limit as i64], |row| {
                Ok(SlackChannel {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_private: row.get::<_, i32>(2)? != 0,
                    updated: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(channels)
    }

    pub fn save_suggested_dm_user_ids(&self, ids: &[String]) -> Result<(), String> {
        let json = serde_json::to_string(ids).map_err(|e| e.to_string())?;
        self.set_setting("suggested_dm_user_ids", &json)
    }

    pub fn get_suggested_dm_user_ids(&self) -> Result<Vec<String>, String> {
        match self.get_setting("suggested_dm_user_ids")? {
            Some(json) => serde_json::from_str(&json).map_err(|e| e.to_string()),
            None => Ok(Vec::new()),
        }
    }

    pub fn slack_cache_count(&self) -> Result<SlackCacheStatus, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let user_count: usize = conn
            .query_row("SELECT COUNT(*) FROM slack_users_cache", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        let channel_count: usize = conn
            .query_row("SELECT COUNT(*) FROM slack_channels_cache", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        Ok(SlackCacheStatus {
            user_count,
            channel_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_message(
        id: &str,
        classification: &str,
        status: &str,
        body: &str,
        starred: bool,
        unread: bool,
        snoozed_until: Option<i64>,
    ) -> Message {
        Message {
            id: id.to_string(),
            source: "slack".to_string(),
            sender: "alice".to_string(),
            subject: Some("general".to_string()),
            body: body.to_string(),
            body_html: Some(body.to_string()),
            permalink: Some(format!("https://example.com/{}", id)),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            timestamp: 1_700_000_000,
            classification: classification.to_string(),
            status: status.to_string(),
            starred,
            unread,
            snoozed_until,
            created_at: 1_700_000_000,
        }
    }

    #[test]
    fn other_tab_includes_unclassified_and_counts_fold() {
        let db = Database::new(":memory:").expect("db init");
        let m1 = sample_message("m1", "other", "inbox", "hello", false, false, None);
        let m2 = sample_message("m2", "unclassified", "inbox", "world", false, false, None);
        db.upsert_messages_batch(&[m1, m2]).expect("insert");

        let other = db.get_messages("other", "inbox").expect("load other");
        assert_eq!(other.len(), 2);

        let counts = db.get_message_counts("inbox").expect("counts");
        assert_eq!(counts.counts.get("other"), Some(&2usize));
        assert!(!counts.counts.contains_key("unclassified"));
    }

    #[test]
    fn upsert_updates_mutable_fields_without_clobbering_triage() {
        let db = Database::new(":memory:").expect("db init");

        let original = sample_message(
            "same-id",
            "important",
            "archived",
            "old body",
            true,
            true,
            Some(1_700_001_000),
        );
        db.upsert_messages_batch(&[original]).expect("seed");

        let mut updated =
            sample_message("same-id", "unclassified", "inbox", "new body", false, false, None);
        updated.avatar_url = None;
        db.upsert_messages_batch(&[updated]).expect("upsert");

        let rows = db
            .get_messages_by_status("archived")
            .expect("load archived rows");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.body, "new body");
        assert_eq!(row.classification, "important");
        assert!(row.starred);
        assert!(row.unread);
        assert_eq!(row.snoozed_until, Some(1_700_001_000));
        assert_eq!(
            row.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
    }

    #[test]
    fn set_unread_is_idempotent() {
        let db = Database::new(":memory:").expect("db init");
        let msg = sample_message("m1", "other", "inbox", "hello", false, false, None);
        db.upsert_messages_batch(&[msg]).expect("insert");

        db.set_unread_message("m1", true).expect("set unread");
        db.set_unread_message("m1", true).expect("set unread again");

        let row = db
            .get_messages("other", "inbox")
            .expect("load rows")
            .into_iter()
            .find(|m| m.id == "m1")
            .expect("message exists");
        assert!(row.unread);
    }

    #[test]
    fn set_unread_errors_for_missing_id() {
        let db = Database::new(":memory:").expect("db init");
        let err = db
            .set_unread_message("missing", true)
            .expect_err("missing id should error");
        assert!(err.contains("Query returned no rows"));
    }

    #[test]
    fn init_migrates_unread_column_on_existing_db() {
        let tmp = std::env::temp_dir().join(format!(
            "dispatch-unread-migration-{}.sqlite",
            std::process::id()
        ));
        if tmp.exists() {
            let _ = std::fs::remove_file(&tmp);
        }

        {
            let conn = Connection::open(&tmp).expect("open legacy db");
            conn.execute_batch(
                "CREATE TABLE messages (
                    id TEXT PRIMARY KEY,
                    source TEXT NOT NULL,
                    sender TEXT NOT NULL,
                    subject TEXT,
                    body TEXT NOT NULL,
                    body_html TEXT,
                    permalink TEXT,
                    avatar_url TEXT,
                    timestamp INTEGER NOT NULL,
                    classification TEXT DEFAULT 'other',
                    status TEXT DEFAULT 'inbox',
                    starred INTEGER DEFAULT 0,
                    snoozed_until INTEGER,
                    created_at INTEGER NOT NULL
                );",
            )
            .expect("create legacy schema");
        }

        let db = Database::new(tmp.to_str().expect("tmp path str")).expect("open migrated db");
        let migrated = sample_message("m1", "other", "inbox", "hello", false, true, None);
        db.upsert_messages_batch(&[migrated]).expect("insert");

        let row = db
            .get_messages("other", "inbox")
            .expect("load rows")
            .into_iter()
            .find(|m| m.id == "m1")
            .expect("message exists");
        assert!(row.unread);

        let _ = std::fs::remove_file(tmp);
    }
}

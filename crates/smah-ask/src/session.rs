//! SQLite session store (RA-03), mirroring the legacy Python schema
//! (`1730153754_history.py`): `chat_history`, `chat_history_details`,
//! `chat_history_message`, `settings` (`last_session`). Same tables, same
//! columns — a Python-era `smah.db` opens unchanged.
//!
//! Messages serialize as `{"role": "user"|"assistant", "content": "..."}`
//! JSON rows (Python parity).

use std::path::Path;

use rusqlite::Connection;
use serde_json::Value;

use smah_core::errors::SmahError;

use crate::backend::{ThreadMessage, ThreadRole};

/// One persisted session row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: i64,
    pub title: String,
}

/// Open/create the session database at `path`.
pub struct SessionStore {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chat_history(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT,
    created_on TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    modified_on TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS chat_history_details(
    chat_history_id INTEGER PRIMARY KEY,
    args JSON,
    plan JSON,
    pipe_input TEXT DEFAULT NULL,
    FOREIGN KEY(chat_history_id) REFERENCES chat_history(id)
);
CREATE TABLE IF NOT EXISTS chat_history_message(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_history_id INTEGER,
    message JSON,
    created_on TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(chat_history_id) REFERENCES chat_history(id)
);
CREATE TABLE IF NOT EXISTS settings(
    setting VARCHAR(32) PRIMARY KEY,
    setting_value VARCHAR(255)
);
";

fn db_error(path: &Path, detail: String) -> SmahError {
    SmahError::Database {
        path: path.display().to_string(),
        detail,
    }
}

fn message_json(message: &ThreadMessage) -> Value {
    serde_json::json!({
        "role": match message.role {
            ThreadRole::User => "user",
            ThreadRole::Assistant => "assistant",
        },
        "content": message.content,
    })
}

fn message_from_json(value: &Value) -> Option<ThreadMessage> {
    let role = match value.get("role")?.as_str()? {
        "user" => ThreadRole::User,
        "assistant" => ThreadRole::Assistant,
        _ => return None,
    };
    Some(ThreadMessage {
        role,
        content: value.get("content")?.as_str()?.to_string(),
    })
}

impl SessionStore {
    /// Open (creating dirs + schema as needed) at `path`.
    pub fn open(path: &Path) -> Result<Self, SmahError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| db_error(path, format!("cannot create dir: {err}")))?;
        }
        // F6: the session database holds the full conversation history —
        // create it owner-only (0600) when new; an existing file's mode is
        // left alone (SQLite may already have it open).
        smah_core::secrets::create_private_file(path)
            .map_err(|err| db_error(path, format!("cannot create private file: {err}")))?;
        let conn = Connection::open(path).map_err(|err| db_error(path, err.to_string()))?;
        conn.execute_batch(SCHEMA)
            .map_err(|err| db_error(path, err.to_string()))?;
        Ok(SessionStore { conn })
    }

    /// Start a new session; returns its id.
    pub fn create_session(&self, title: &str) -> Result<i64, SmahError> {
        self.conn
            .query_row(
                "INSERT INTO chat_history (title) VALUES (?1) RETURNING id",
                [&title],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|err| db_error(Path::new("chat_history"), err.to_string()))
    }

    /// Append one message to a session.
    pub fn append_message(&self, session: i64, message: &ThreadMessage) -> Result<(), SmahError> {
        let payload = message_json(message).to_string();
        let _ = self
            .conn
            .execute(
                "INSERT INTO chat_history_message (chat_history_id, message) VALUES (?1, ?2)",
                rusqlite::params![session, payload],
            )
            .map_err(|err| db_error(Path::new("chat_history_message"), err.to_string()))?;
        self.touch(session)
    }

    /// Bump `modified_on`.
    pub fn touch(&self, session: i64) -> Result<(), SmahError> {
        self.conn
            .execute(
                "UPDATE chat_history SET modified_on = strftime('%Y-%m-%d %H:%M:%f','now') WHERE id = ?1",
                rusqlite::params![session],
            )
            .map_err(|err| db_error(Path::new("chat_history"), err.to_string()))?;
        Ok(())
    }

    /// Load a session's messages, oldest first.
    pub fn load_session(&self, session: i64) -> Result<Vec<ThreadMessage>, SmahError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT message FROM chat_history_message \
                 WHERE chat_history_id = ?1 ORDER BY id ASC",
            )
            .map_err(|err| db_error(Path::new("chat_history_message"), err.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![session], |row| row.get::<_, String>(0))
            .map_err(|err| db_error(Path::new("chat_history_message"), err.to_string()))?;
        let mut messages = Vec::new();
        for row in rows {
            let payload =
                row.map_err(|err| db_error(Path::new("chat_history_message"), err.to_string()))?;
            if let Some(message) = serde_json::from_str::<Value>(&payload)
                .ok()
                .as_ref()
                .and_then(message_from_json)
            {
                messages.push(message);
            }
        }
        Ok(messages)
    }

    /// The most recently modified session, if any.
    pub fn latest_session(&self) -> Result<Option<SessionRow>, SmahError> {
        self.conn
            .query_row(
                "SELECT id, title FROM chat_history ORDER BY modified_on DESC, id DESC LIMIT 1",
                [],
                |row| {
                    Ok(SessionRow {
                        id: row.get(0)?,
                        title: row.get(1)?,
                    })
                },
            )
            .map(Some)
            .or_else(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(db_error(Path::new("chat_history"), other.to_string())),
            })
    }

    /// Record `last_session` (the `smah resume --last` pointer).
    pub fn set_last_session(&self, session: i64) -> Result<(), SmahError> {
        self.conn
            .execute(
                "INSERT INTO settings (setting, setting_value) VALUES ('last_session', ?1) \
                 ON CONFLICT(setting) DO UPDATE SET setting_value = ?1",
                rusqlite::params![session.to_string()],
            )
            .map_err(|err| db_error(Path::new("settings"), err.to_string()))?;
        Ok(())
    }

    /// The `last_session` pointer, when set.
    pub fn last_session(&self) -> Result<Option<i64>, SmahError> {
        self.conn
            .query_row(
                "SELECT setting_value FROM settings WHERE setting = 'last_session'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map(|value| value.parse::<i64>().ok())
            .or_else(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(db_error(Path::new("settings"), other.to_string())),
            })
    }

    /// Recent sessions, newest first.
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRow>, SmahError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, title FROM chat_history \
                 ORDER BY modified_on DESC, id DESC LIMIT ?1",
            )
            .map_err(|err| db_error(Path::new("chat_history"), err.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                })
            })
            .map_err(|err| db_error(Path::new("chat_history"), err.to_string()))?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|err| db_error(Path::new("chat_history"), err.to_string()))?);
        }
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().expect("tmp");
        let store = SessionStore::open(&dir.path().join("smah.db")).expect("open");
        (dir, store)
    }

    #[test]
    fn messages_round_trip_in_order() {
        let (_dir, store) = store();
        let session = store.create_session("disk cleanup").expect("create");
        store
            .append_message(session, &ThreadMessage::user("how to free disk?"))
            .expect("append");
        store
            .append_message(session, &ThreadMessage::assistant("try journalctl"))
            .expect("append");
        let loaded = store.load_session(session).expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, "how to free disk?");
        assert_eq!(loaded[1].role, ThreadRole::Assistant);
        assert_eq!(loaded[1].content, "try journalctl");
    }

    #[test]
    fn latest_session_tracks_modification() {
        let (_dir, store) = store();
        let first = store.create_session("one").expect("create");
        let second = store.create_session("two").expect("create");
        assert_eq!(
            store.latest_session().expect("latest").map(|s| s.id),
            Some(second)
        );
        store.touch(first).expect("touch");
        assert_eq!(
            store.latest_session().expect("latest").map(|s| s.id),
            Some(first)
        );
    }

    #[test]
    fn last_session_pointer_round_trips() {
        let (_dir, store) = store();
        assert_eq!(store.last_session().expect("pointer"), None);
        let session = store.create_session("x").expect("create");
        store.set_last_session(session).expect("set");
        assert_eq!(store.last_session().expect("pointer"), Some(session));
        let other = store.create_session("y").expect("create");
        store.set_last_session(other).expect("set");
        assert_eq!(store.last_session().expect("pointer"), Some(other));
    }

    #[test]
    fn empty_store_reports_nothing() {
        let (_dir, store) = store();
        assert_eq!(store.latest_session().expect("latest"), None);
        assert!(store.list_sessions(10).expect("list").is_empty());
    }

    #[test]
    fn python_schema_tables_exist() {
        let (_dir, store) = store();
        let mut stmt = store
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare");
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("query")
            .filter_map(|r| r.ok())
            .collect();
        for expected in [
            "chat_history",
            "chat_history_details",
            "chat_history_message",
            "settings",
        ] {
            assert!(tables.contains(&expected.to_string()), "missing {expected}");
        }
    }
}

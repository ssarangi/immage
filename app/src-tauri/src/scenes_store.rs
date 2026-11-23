// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Scenes store: `scenes.sqlite` under the app's runtime directory, kept apart
//! from the cull store. It holds the scene hash of each photo's cached thumbnail
//! (against the thumbnail's size and checksum, so a replaced thumbnail is hashed
//! again), the frames the user accepted in Scenes, and the splits and merges the
//! user made, replayed after every grouping. Photos are keyed by path and version,
//! as the cull store keys them.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;

pub struct Store {
    db: Connection,
}

/// A frame the user accepted in Scenes (the green label is written to the library
/// by the page; this is the memory of who wrote it).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneAccepted {
    pub path: String,
    #[serde(default)]
    pub version: i64,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// milliseconds, so two overrides made in the same second replay in the order they were made
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| format!("scenes store: {e}"))?;
        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
        // best effort: a read-only volume still reads
        let _ = db.pragma_update(None, "journal_mode", "WAL");
        let _ = db.pragma_update(None, "synchronous", "NORMAL");
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS scene_hash (
               path TEXT NOT NULL,
               version INTEGER NOT NULL,
               thumb_len INTEGER NOT NULL,
               thumb_sum INTEGER NOT NULL,
               hash TEXT NOT NULL,
               PRIMARY KEY (path, version)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS scene_accepted (
               path TEXT NOT NULL,
               version INTEGER NOT NULL,
               accepted_at INTEGER NOT NULL,
               PRIMARY KEY (path, version)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS scene_overrides (
               path TEXT NOT NULL,
               version INTEGER NOT NULL,
               action TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               PRIMARY KEY (path, version, action)
             ) WITHOUT ROWID;",
        )
        .map_err(|e| format!("scenes store: {e}"))?;
        Ok(Self { db })
    }

    /// the stored hash of a photo, if it was made from a thumbnail of this size and checksum
    pub fn hash_get(&self, path: &str, version: i64, len: i64, sum: i64) -> Option<String> {
        self.db
            .query_row(
                "SELECT hash FROM scene_hash WHERE path = ?1 AND version = ?2 AND thumb_len = ?3 AND thumb_sum = ?4",
                params![path, version, len, sum],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// (path, version, thumbnail length, checksum, hash) rows, in one transaction
    pub fn hash_put_many(&self, rows: &[(&str, i64, i64, i64, &str)]) -> Result<(), String> {
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        {
            let mut put = tx
                .prepare_cached("INSERT OR REPLACE INTO scene_hash(path, version, thumb_len, thumb_sum, hash) VALUES(?1, ?2, ?3, ?4, ?5)")
                .map_err(|e| e.to_string())?;
            for (path, version, len, sum, hash) in rows {
                put.execute(params![path, version, len, sum, hash]).map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn accepted_all(&self) -> Result<Vec<SceneAccepted>, String> {
        let mut stmt = self.db.prepare("SELECT path, version FROM scene_accepted").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok(SceneAccepted { path: row.get(0)?, version: row.get(1)? }))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn set_accepted(&self, path: &str, version: i64, accepted: bool) -> Result<(), String> {
        if accepted {
            self.db
                .execute("INSERT OR REPLACE INTO scene_accepted(path, version, accepted_at) VALUES(?1, ?2, ?3)", params![path, version, now()])
        } else {
            self.db.execute("DELETE FROM scene_accepted WHERE path = ?1 AND version = ?2", params![path, version])
        }
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// (path, version, "split" | "merge") of every stored override
    pub fn overrides(&self) -> Result<Vec<(String, i64, String)>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT path, version, action FROM scene_overrides ORDER BY created_at, path")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn override_put(&self, path: &str, version: i64, action: &str) -> Result<(), String> {
        self.db
            .execute("INSERT OR REPLACE INTO scene_overrides(path, version, action, created_at) VALUES(?1, ?2, ?3, ?4)", params![path, version, action, now_ms()])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("scenes.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn a_hash_is_kept_against_its_thumbnail() {
        let (_dir, store) = store();
        store.hash_put_many(&[("/a.jpg", 0, 100, 7, "abc"), ("/b.jpg", 1, 50, 9, "def")]).unwrap();
        assert_eq!(store.hash_get("/a.jpg", 0, 100, 7).as_deref(), Some("abc"));
        // a different thumbnail (size or checksum) or version is not this hash
        assert!(store.hash_get("/a.jpg", 0, 101, 7).is_none());
        assert!(store.hash_get("/a.jpg", 0, 100, 8).is_none());
        assert!(store.hash_get("/a.jpg", 1, 100, 7).is_none());
    }

    #[test]
    fn accepted_frames_and_overrides_persist() {
        let (_dir, store) = store();
        store.set_accepted("/a.jpg", 0, true).unwrap();
        store.set_accepted("/b.jpg", 0, true).unwrap();
        store.set_accepted("/b.jpg", 0, false).unwrap();
        assert_eq!(store.accepted_all().unwrap(), vec![SceneAccepted { path: "/a.jpg".into(), version: 0 }]);
        store.override_put("/a.jpg", 0, "split").unwrap();
        assert_eq!(store.overrides().unwrap(), vec![("/a.jpg".to_string(), 0, "split".to_string())]);
    }
}

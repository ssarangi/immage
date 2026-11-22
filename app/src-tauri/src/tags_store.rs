// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! `tags.sqlite`: what the Auto-keywords analysis found, per photo file and model version,
//! and which keywords it attached to the photo, so only those can be taken off again.
//! A photo's identity is its path and original fingerprint (size and time, search_cache.rs);
//! a changed file or a new model version makes the stored result stale, and the photo is
//! analysed again. Every tag above the model's own threshold is kept with its confidence,
//! so changing the confidence setting re-evaluates stored results without new inference.
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 1;

pub struct TagStore {
    db: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTag {
    pub name: String,
    pub confidence: f32,
    /// Immage attached this keyword to the photo (the user did not have it already).
    pub applied: bool,
    /// The user removed this AI keyword from the photo: it is never attached again, by
    /// a change of the confidence setting or by a new analysis.
    pub dismissed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct TagTotals {
    /// Photos with a stored result.
    pub photos: u32,
    /// Photos that have at least one keyword Immage attached.
    pub keyworded: u32,
    /// Keywords Immage attached, over all photos.
    pub applied: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct TagCount {
    pub name: String,
    /// Photos that carry this keyword because of the analysis.
    pub photos: u32,
}

impl TagStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let db = Connection::open(path).or_else(|_| Connection::open_in_memory()).map_err(|e| e.to_string())?;
        Self::from_connection(db)
    }
    pub fn from_connection(db: Connection) -> Result<Self, String> {
        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
        let _ = db.pragma_update(None, "journal_mode", "WAL");
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if version != SCHEMA_VERSION {
            // results are disposable: a newer or older layout is rebuilt by analysing again
            db.execute_batch("DROP TABLE IF EXISTS tag_rows; DROP TABLE IF EXISTS tag_photos;").map_err(|e| e.to_string())?;
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS tag_photos (
                 path TEXT PRIMARY KEY,
                 fingerprint TEXT NOT NULL,
                 model TEXT NOT NULL,
                 analysed_at INTEGER NOT NULL,
                 applied_pct INTEGER
             );
             CREATE TABLE IF NOT EXISTS tag_rows (
                 path TEXT NOT NULL REFERENCES tag_photos(path) ON DELETE CASCADE,
                 tag TEXT NOT NULL,
                 confidence REAL NOT NULL,
                 applied INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (path, tag)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS tag_rows_tag ON tag_rows(tag) WHERE applied=1;",
        )
        .map_err(|e| e.to_string())?;
        db.pragma_update(None, "user_version", SCHEMA_VERSION).map_err(|e| e.to_string())?;
        db.pragma_update(None, "foreign_keys", true).map_err(|e| e.to_string())?;
        Ok(Self { db: Mutex::new(db) })
    }
    fn db(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.db.lock().map_err(|_| "tag store lock poisoned".to_string())
    }

    /// Whether this exact file has been analysed by this model version.
    pub fn is_current(&self, path: &str, fingerprint: &str, model: &str) -> Result<bool, String> {
        self.db()?
            .query_row(
                "SELECT 1 FROM tag_photos WHERE path=?1 AND fingerprint=?2 AND model=?3",
                params![path, fingerprint, model],
                |_| Ok(()),
            )
            .optional()
            .map(|r| r.is_some())
            .map_err(|e| e.to_string())
    }

    /// Replaces a photo's result. The keywords Immage had attached for an earlier result
    /// and that the photo still has are carried over (`applied` stays), so a re-analysis
    /// never forgets what it can take off.
    pub fn put(&self, path: &str, fingerprint: &str, model: &str, tags: &[(String, f32)], now: i64) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let before: Vec<(String, i64)> = {
            let mut q = tx.prepare("SELECT tag,applied FROM tag_rows WHERE path=?1 AND applied>0").map_err(|e| e.to_string())?;
            let rows = q.query_map(params![path], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        let state_of = |name: &str| before.iter().find(|(n, _)| n == name).map_or(0, |(_, a)| *a);
        tx.execute("DELETE FROM tag_photos WHERE path=?1", params![path]).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO tag_photos(path,fingerprint,model,analysed_at,applied_pct) VALUES(?1,?2,?3,?4,NULL)",
            params![path, fingerprint, model, now],
        )
        .map_err(|e| e.to_string())?;
        {
            let mut insert = tx
                .prepare("INSERT OR REPLACE INTO tag_rows(path,tag,confidence,applied) VALUES(?1,?2,?3,?4)")
                .map_err(|e| e.to_string())?;
            for (name, confidence) in tags {
                insert
                    .execute(params![path, name, f64::from(*confidence), state_of(name)])
                    .map_err(|e| e.to_string())?;
            }
            // applied keywords the new result no longer lists stay known, at zero confidence
            for (name, state) in before.iter().filter(|(b, _)| !tags.iter().any(|(n, _)| n == b)) {
                insert.execute(params![path, name, 0.0f64, state]).map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn tags(&self, path: &str) -> Result<Vec<StoredTag>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT tag,confidence,applied FROM tag_rows WHERE path=?1 ORDER BY confidence DESC, tag")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map(params![path], |r| {
                let state: i64 = r.get(2)?;
                Ok(StoredTag { name: r.get(0)?, confidence: r.get::<_, f64>(1)? as f32, applied: state == 1, dismissed: state == 2 })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
    /// The confidence percent the photo's keywords were last decided at, or None.
    #[cfg(test)]
    pub fn applied_pct(&self, path: &str) -> Result<Option<u8>, String> {
        let v: Option<Option<i64>> = self
            .db()?
            .query_row("SELECT applied_pct FROM tag_photos WHERE path=?1", params![path], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(v.flatten().and_then(|n| u8::try_from(n).ok()))
    }
    /// Records that these keywords were attached and those taken off, at this confidence.
    pub fn record_applied(&self, path: &str, pct: u8, attached: &[String], detached: &[String]) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for name in attached {
            tx.execute("UPDATE tag_rows SET applied=1 WHERE path=?1 AND tag=?2", params![path, name]).map_err(|e| e.to_string())?;
        }
        for name in detached {
            tx.execute("UPDATE tag_rows SET applied=0 WHERE path=?1 AND tag=?2", params![path, name]).map_err(|e| e.to_string())?;
        }
        tx.execute("UPDATE tag_photos SET applied_pct=?2 WHERE path=?1", params![path, i64::from(pct)]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    /// Photos whose keywords were decided at another confidence than `pct`, up to `limit`.
    pub fn needing_reapply(&self, pct: u8, limit: u32) -> Result<Vec<String>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT path FROM tag_photos WHERE applied_pct IS NOT ?1 ORDER BY path LIMIT ?2")
            .map_err(|e| e.to_string())?;
        let rows = q.query_map(params![i64::from(pct), limit], |r| r.get(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn totals(&self) -> Result<TagTotals, String> {
        self.db()?
            .query_row(
                "SELECT (SELECT count(*) FROM tag_photos),
                        (SELECT count(DISTINCT path) FROM tag_rows WHERE applied=1),
                        (SELECT count(*) FROM tag_rows WHERE applied=1)",
                [],
                |r| Ok(TagTotals { photos: r.get(0)?, keyworded: r.get(1)?, applied: r.get(2)? }),
            )
            .map_err(|e| e.to_string())
    }
    /// The keywords Immage attached, most used first, for the review list and Find.
    pub fn applied_counts(&self, limit: u32) -> Result<Vec<TagCount>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT tag,count(*) n FROM tag_rows WHERE applied=1 GROUP BY tag ORDER BY n DESC, tag LIMIT ?1")
            .map_err(|e| e.to_string())?;
        let rows = q.query_map(params![limit], |r| Ok(TagCount { name: r.get(0)?, photos: r.get(1)? })).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
    /// Photos carrying an applied keyword (all of `names`, or any when empty), up to `limit`.
    pub fn applied_photos(&self, names: &[String], limit: u32) -> Result<Vec<(String, Vec<String>)>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT path,tag FROM tag_rows WHERE applied=1 ORDER BY path,tag")
            .map_err(|e| e.to_string())?;
        let mut rows = q.query([]).map_err(|e| e.to_string())?;
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let (path, tag): (String, String) = (r.get(0).map_err(|e| e.to_string())?, r.get(1).map_err(|e| e.to_string())?);
            if !names.is_empty() && !names.contains(&tag) {
                continue;
            }
            match out.last_mut() {
                Some((p, tags)) if *p == path => tags.push(tag),
                _ => {
                    if out.len() as u32 >= limit {
                        break;
                    }
                    out.push((path, vec![tag]));
                }
            }
        }
        Ok(out)
    }
    /// The keywords the analysis attached to this photo.
    pub fn applied_for(&self, path: &str) -> Result<Vec<String>, String> {
        let db = self.db()?;
        let mut q = db.prepare("SELECT tag FROM tag_rows WHERE path=?1 AND applied=1 ORDER BY tag").map_err(|e| e.to_string())?;
        let rows = q.query_map(params![path], |r| r.get(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
    /// These keywords were taken off the photo by the user: they stop being Immage's and are
    /// remembered as dismissed, so changing the confidence setting does not attach them again.
    pub fn forget_applied(&self, path: &str, names: &[String]) -> Result<(), String> {
        let db = self.db()?;
        for name in names {
            db.execute("UPDATE tag_rows SET applied=2 WHERE path=?1 AND tag=?2 AND applied=1", params![path, name]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    /// Drops every stored result (Remove model and results). Keywords already on photos stay.
    pub fn clear(&self) -> Result<(), String> {
        self.db()?.execute_batch("DELETE FROM tag_rows; DELETE FROM tag_photos;").map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> TagStore {
        TagStore::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn tags(list: &[(&str, f32)]) -> Vec<(String, f32)> {
        list.iter().map(|(n, c)| (n.to_string(), *c)).collect()
    }

    #[test]
    fn a_result_is_current_for_its_file_and_model_only() {
        let s = store();
        assert!(!s.is_current("/p/a.jpg", "f1", "m1").unwrap());
        s.put("/p/a.jpg", "f1", "m1", &tags(&[("beach", 0.9)]), 1).unwrap();
        assert!(s.is_current("/p/a.jpg", "f1", "m1").unwrap());
        assert!(!s.is_current("/p/a.jpg", "f2", "m1").unwrap(), "a changed file is analysed again");
        assert!(!s.is_current("/p/a.jpg", "f1", "m2").unwrap(), "a new model version is analysed again");
    }

    #[test]
    fn every_tag_is_kept_with_its_confidence_strongest_first() {
        let s = store();
        s.put("/p/a.jpg", "f", "m", &tags(&[("sea", 0.55), ("beach", 0.93)]), 1).unwrap();
        let got = s.tags("/p/a.jpg").unwrap();
        assert_eq!(got.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["beach", "sea"]);
        assert!(got.iter().all(|t| !t.applied));
    }

    #[test]
    fn applied_keywords_are_tracked_and_survive_a_new_analysis() {
        let s = store();
        s.put("/p/a.jpg", "f", "m", &tags(&[("beach", 0.9), ("sea", 0.8)]), 1).unwrap();
        s.record_applied("/p/a.jpg", 70, &["beach".into(), "sea".into()], &[]).unwrap();
        assert_eq!(s.applied_pct("/p/a.jpg").unwrap(), Some(70));
        s.put("/p/a.jpg", "f2", "m2", &tags(&[("beach", 0.95)]), 2).unwrap();
        let got = s.tags("/p/a.jpg").unwrap();
        assert!(got.iter().find(|t| t.name == "beach").unwrap().applied);
        let sea = got.iter().find(|t| t.name == "sea").unwrap();
        assert!(sea.applied && sea.confidence == 0.0, "a keyword the new result dropped can still be taken off");
        assert_eq!(s.applied_pct("/p/a.jpg").unwrap(), None, "a new result is decided afresh");
    }

    #[test]
    fn photos_decided_at_another_confidence_are_listed_for_reapply() {
        let s = store();
        s.put("/p/a.jpg", "f", "m", &tags(&[("x", 0.9)]), 1).unwrap();
        s.put("/p/b.jpg", "f", "m", &tags(&[("x", 0.9)]), 1).unwrap();
        s.record_applied("/p/a.jpg", 70, &[], &[]).unwrap();
        assert_eq!(s.needing_reapply(70, 10).unwrap(), ["/p/b.jpg"]);
        assert_eq!(s.needing_reapply(80, 10).unwrap(), ["/p/a.jpg", "/p/b.jpg"]);
        assert_eq!(s.needing_reapply(70, 0).unwrap().len(), 0);
    }

    #[test]
    fn totals_counts_and_photo_lists_cover_only_applied_keywords() {
        let s = store();
        s.put("/p/a.jpg", "f", "m", &tags(&[("beach", 0.9), ("sea", 0.8), ("dog", 0.7)]), 1).unwrap();
        s.put("/p/b.jpg", "f", "m", &tags(&[("beach", 0.9)]), 1).unwrap();
        s.put("/p/c.jpg", "f", "m", &tags(&[("cat", 0.9)]), 1).unwrap();
        s.record_applied("/p/a.jpg", 70, &["beach".into(), "sea".into()], &[]).unwrap();
        s.record_applied("/p/b.jpg", 70, &["beach".into()], &[]).unwrap();
        assert_eq!(s.totals().unwrap(), TagTotals { photos: 3, keyworded: 2, applied: 3 });
        let counts = s.applied_counts(10).unwrap();
        assert_eq!(counts[0], TagCount { name: "beach".into(), photos: 2 });
        assert_eq!(counts.len(), 2);
        assert_eq!(s.applied_photos(&[], 10).unwrap().len(), 2);
        assert_eq!(s.applied_photos(&["sea".into()], 10).unwrap(), vec![("/p/a.jpg".to_string(), vec!["sea".to_string()])]);
        assert_eq!(s.applied_photos(&[], 1).unwrap().len(), 1);
        assert_eq!(s.applied_for("/p/a.jpg").unwrap(), ["beach", "sea"]);
        assert!(s.applied_for("/p/c.jpg").unwrap().is_empty());
        s.forget_applied("/p/b.jpg", &["beach".into()]).unwrap();
        assert_eq!(s.totals().unwrap().applied, 2);
        let b = s.tags("/p/b.jpg").unwrap();
        assert!(!b[0].applied && b[0].dismissed, "a removed AI keyword is remembered as dismissed");
    }

    #[test]
    fn clear_drops_results_and_a_changed_schema_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tags.sqlite");
        {
            let s = TagStore::open(&path).unwrap();
            s.put("/p/a.jpg", "f", "m", &tags(&[("x", 0.9)]), 1).unwrap();
            s.clear().unwrap();
            assert_eq!(s.totals().unwrap().photos, 0);
            s.put("/p/a.jpg", "f", "m", &tags(&[("x", 0.9)]), 1).unwrap();
        }
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 99).unwrap();
        let s = TagStore::open(&path).unwrap();
        assert_eq!(s.totals().unwrap().photos, 0, "an unknown layout is dropped, not misread");
    }
}

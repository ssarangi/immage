// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The auto-cull store: a small SQLite file of its own under the app's runtime
//! directory (`cull.sqlite`), holding past sessions, their decisions, every
//! override and a cache of each photo's analysis. It is not darktable's
//! library: a decision only reaches ratings, flags and XMP sidecars when it
//! is applied (cull_cmds.rs). Photos are keyed by file identity, the path, the
//! version (virtual copy) and the file's size and modification time, so an
//! edited or replaced file is measured again and never applied to by mistake.
use crate::cull::{CullAnalysis, CullBlurRegion};
use crate::cull_plan::{CullSettings, CullFactor, Planned, CullReason, CullStatus, FEATURE_SET_VERSION};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;

/// A photo, as the library names it: its file and which version of the file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullTarget {
    pub path: String,
    #[serde(default)]
    pub version: i64,
}

/// What a photo's library entry says now: the three things a cull writes.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullLibraryState {
    /// 0-5; 0 when rejected
    pub rating: u8,
    pub rejected: bool,
    /// bits: red 1, yellow 2, green 4, blue 8, purple 16
    pub labels: u32,
}

/// One photo's library entry before and after an apply, kept so Undo can put
/// it back (and tell when something else changed it since).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullApplied {
    pub target: CullTarget,
    pub before: CullLibraryState,
    pub after: CullLibraryState,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub enum CullSessionState {
    Planned,
    Applied,
    Undone,
}

impl CullSessionState {
    fn as_str(self) -> &'static str {
        match self {
            CullSessionState::Planned => "planned",
            CullSessionState::Applied => "applied",
            CullSessionState::Undone => "undone",
        }
    }
    fn parse(text: &str) -> Self {
        match text {
            "applied" => CullSessionState::Applied,
            "undone" => CullSessionState::Undone,
            _ => CullSessionState::Planned,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullFailure {
    pub path: String,
    pub error: String,
}

/// One past or current cull run.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullSession {
    pub id: i64,
    /// what was culled, for the History list (a folder's name, "Selection")
    pub scope_label: String,
    pub state: CullSessionState,
    pub settings: CullSettings,
    pub total_count: i64,
    pub keep_count: i64,
    pub reject_count: i64,
    pub highlight_count: i64,
    pub group_count: i64,
    /// seconds since the epoch
    pub created_at: i64,
    pub updated_at: i64,
    /// photos that could not be analysed, left out of the decisions
    pub failed: Vec<CullFailure>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullDecision {
    pub id: i64,
    pub session_id: i64,
    pub path: String,
    pub version: i64,
    /// other files that take this photo's decision (a RAW's JPEG, say)
    pub backing: Vec<CullTarget>,
    /// the planner's verdict
    pub proposed: CullStatus,
    /// the user's override, if any
    pub final_status: Option<CullStatus>,
    /// the verdict that counts: the override, else the proposal
    pub status: CullStatus,
    pub reason: CullReason,
    /// 1-based set number, when the photo is one of a set of look-alikes
    pub group: Option<u32>,
    /// the set's best frame (for a duplicate, the one it lost to)
    pub best_path: Option<String>,
    /// the set's best frame is this photo
    pub best: bool,
    pub highlight: bool,
    pub quality_score: f64,
    pub factors: Vec<CullFactor>,
    pub blurry_region: Option<CullBlurRegion>,
    pub analysis: CullAnalysis,
    /// the file's size and modification time (ms) when it was analysed
    pub file_size: i64,
    pub file_modified_ms: i64,
    /// written to the library by Apply and not yet undone
    pub applied: bool,
}

/// A photo with what the planner needs about its file.
pub struct Item {
    pub target: CullTarget,
    pub backing: Vec<CullTarget>,
    pub size: i64,
    pub modified_ms: i64,
    pub planned: Planned,
}

pub struct Store {
    db: Connection,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

fn from_json<T: for<'a> Deserialize<'a>>(text: &str) -> Result<T, rusqlite::Error> {
    serde_json::from_str(text).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

const DECISION_COLUMNS: &str = "id, session_id, path, version, backing_json, proposed, final_status, reason, group_no, best_path, \
     best, highlight, quality_score, factors_json, blur_json, analysis_json, file_size, file_modified_ms, applied";

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| format!("culling store: {e}"))?;
        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
        // best effort: a read-only volume still reads
        let _ = db.pragma_update(None, "journal_mode", "WAL");
        db.pragma_update(None, "foreign_keys", true).map_err(|e| e.to_string())?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS cull_sessions (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               scope_label TEXT NOT NULL,
               state TEXT NOT NULL,
               settings_json TEXT NOT NULL,
               feature_set_version TEXT NOT NULL,
               total_count INTEGER NOT NULL,
               keep_count INTEGER NOT NULL,
               reject_count INTEGER NOT NULL,
               highlight_count INTEGER NOT NULL,
               group_count INTEGER NOT NULL,
               failed_json TEXT NOT NULL DEFAULT '[]',
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS cull_decisions (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id INTEGER NOT NULL REFERENCES cull_sessions(id) ON DELETE CASCADE,
               path TEXT NOT NULL,
               version INTEGER NOT NULL DEFAULT 0,
               backing_json TEXT NOT NULL DEFAULT '[]',
               proposed TEXT NOT NULL,
               final_status TEXT,
               reason TEXT NOT NULL,
               group_no INTEGER,
               best_path TEXT,
               best INTEGER NOT NULL DEFAULT 0,
               highlight INTEGER NOT NULL DEFAULT 0,
               quality_score REAL NOT NULL,
               factors_json TEXT NOT NULL DEFAULT '[]',
               blur_json TEXT,
               analysis_json TEXT NOT NULL,
               file_size INTEGER NOT NULL,
               file_modified_ms INTEGER NOT NULL,
               applied INTEGER NOT NULL DEFAULT 0,
               applied_json TEXT NOT NULL DEFAULT '[]'
             );
             CREATE INDEX IF NOT EXISTS idx_cull_decisions_session ON cull_decisions(session_id, proposed);
             CREATE TABLE IF NOT EXISTS cull_decision_events (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               decision_id INTEGER NOT NULL REFERENCES cull_decisions(id) ON DELETE CASCADE,
               previous_status TEXT NOT NULL,
               next_status TEXT NOT NULL,
               source TEXT NOT NULL,
               created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_cull_events_decision ON cull_decision_events(decision_id, created_at DESC);
             CREATE TABLE IF NOT EXISTS cull_analysis_cache (
               path TEXT NOT NULL,
               version INTEGER NOT NULL,
               file_size INTEGER NOT NULL,
               file_modified_ms INTEGER NOT NULL,
               feature_set_version TEXT NOT NULL,
               hash TEXT NOT NULL,
               analysis_json TEXT NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (path, version)
             );",
        )
        .map_err(|e| format!("culling store: {e}"))?;
        Ok(Self { db })
    }

    /// The cached measurements of a photo, if its file is the one measured.
    pub fn cached(&self, target: &CullTarget, size: i64, modified_ms: i64) -> Option<(CullAnalysis, String)> {
        self.db
            .query_row(
                "SELECT analysis_json, hash FROM cull_analysis_cache
                 WHERE path = ?1 AND version = ?2 AND file_size = ?3 AND file_modified_ms = ?4 AND feature_set_version = ?5",
                params![target.path, target.version, size, modified_ms, FEATURE_SET_VERSION],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|(analysis, hash)| serde_json::from_str(&analysis).ok().map(|a| (a, hash)))
    }

    /// The cached measurement of each photo, in the order asked, by path and version alone: what the Scenes timeline shows (a photo changed since it was measured shows its old score until it is
    /// scored again) and what a rules plan records. No file is looked at, so on a network share this is free.
    pub fn analyses(&self, items: &[(&str, i64)]) -> Vec<Option<CullAnalysis>> {
        let Ok(mut stmt) = self.db.prepare_cached(
            "SELECT analysis_json FROM cull_analysis_cache WHERE path = ?1 AND version = ?2 AND feature_set_version = ?3",
        ) else {
            return vec![None; items.len()];
        };
        items
            .iter()
            .map(|(path, version)| {
                let text: Option<String> = stmt.query_row(params![path, version, FEATURE_SET_VERSION], |row| row.get(0)).optional().ok().flatten();
                text.and_then(|t| serde_json::from_str::<CullAnalysis>(&t).ok())
            })
            .collect()
    }

    pub fn cache_put(&self, target: &CullTarget, size: i64, modified_ms: i64, hash: &str, analysis: &CullAnalysis) -> Result<(), String> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO cull_analysis_cache(path, version, file_size, file_modified_ms, feature_set_version, hash, analysis_json, updated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![target.path, target.version, size, modified_ms, FEATURE_SET_VERSION, hash, json(analysis), now()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Drops cached measurements, for one folder prefix or all.
    pub fn cache_clear(&self) -> Result<usize, String> {
        self.db.execute("DELETE FROM cull_analysis_cache", []).map_err(|e| e.to_string())
    }

    /// Records a plan as a new session and returns it with its decisions.
    pub fn create_session(
        &mut self,
        scope_label: &str,
        settings: &CullSettings,
        items: Vec<Item>,
        failed: Vec<CullFailure>,
    ) -> Result<(CullSession, Vec<CullDecision>), String> {
        let total = items.len() as i64;
        let rejects = items.iter().filter(|i| i.planned.proposed == CullStatus::Reject).count() as i64;
        let highlights = items.iter().filter(|i| i.planned.highlight).count() as i64;
        let groups = items.iter().filter_map(|i| i.planned.group).max().unwrap_or(0) as i64;
        let when = now();
        let tx = self.db.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO cull_sessions(scope_label, state, settings_json, feature_set_version, total_count, keep_count, reject_count,
                                       highlight_count, group_count, failed_json, created_at, updated_at)
             VALUES(?1, 'planned', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![scope_label, json(settings), FEATURE_SET_VERSION, total, total - rejects, rejects, highlights, groups, json(&failed), when],
        )
        .map_err(|e| e.to_string())?;
        let session_id = tx.last_insert_rowid();
        // the set's best frame is the one a duplicate lost to, or a BestInGroup
        for item in &items {
            let p = &item.planned;
            tx.execute(
                "INSERT INTO cull_decisions(session_id, path, version, backing_json, proposed, reason, group_no, best_path, best, highlight,
                                            quality_score, factors_json, blur_json, analysis_json, file_size, file_modified_ms)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    session_id,
                    item.target.path,
                    item.target.version,
                    json(&item.backing),
                    p.proposed.as_str(),
                    p.reason.as_str(),
                    p.group,
                    p.best_path,
                    p.reason == CullReason::BestInGroup,
                    p.highlight,
                    p.quality_score,
                    json(&p.factors),
                    p.blurry_region.as_ref().map(json),
                    json(&p.analysis),
                    item.size,
                    item.modified_ms
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok((self.session(session_id)?.ok_or("session vanished")?, self.decisions(session_id)?))
    }

    fn read_session(row: &rusqlite::Row<'_>) -> Result<CullSession, rusqlite::Error> {
        Ok(CullSession {
            id: row.get(0)?,
            scope_label: row.get(1)?,
            state: CullSessionState::parse(&row.get::<_, String>(2)?),
            settings: from_json(&row.get::<_, String>(3)?)?,
            total_count: row.get(4)?,
            keep_count: row.get(5)?,
            reject_count: row.get(6)?,
            highlight_count: row.get(7)?,
            group_count: row.get(8)?,
            failed: from_json(&row.get::<_, String>(9)?)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }

    const SESSION_COLUMNS: &'static str = "id, scope_label, state, settings_json, total_count, keep_count, reject_count, highlight_count, \
         group_count, failed_json, created_at, updated_at";

    pub fn session(&self, id: i64) -> Result<Option<CullSession>, String> {
        self.db
            .query_row(&format!("SELECT {} FROM cull_sessions WHERE id = ?1", Self::SESSION_COLUMNS), [id], Self::read_session)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Newest first.
    pub fn sessions(&self) -> Result<Vec<CullSession>, String> {
        let mut statement = self
            .db
            .prepare(&format!("SELECT {} FROM cull_sessions ORDER BY updated_at DESC, id DESC LIMIT 200", Self::SESSION_COLUMNS))
            .map_err(|e| e.to_string())?;
        let rows = statement.query_map([], Self::read_session).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn delete_session(&self, id: i64) -> Result<bool, String> {
        self.db.execute("DELETE FROM cull_sessions WHERE id = ?1", [id]).map(|n| n > 0).map_err(|e| e.to_string())
    }

    fn read_decision(row: &rusqlite::Row<'_>) -> Result<CullDecision, rusqlite::Error> {
        let proposed = CullStatus::parse(&row.get::<_, String>(5)?).unwrap_or(CullStatus::Keep);
        let final_status = row.get::<_, Option<String>>(6)?.and_then(|s| CullStatus::parse(&s));
        let blur: Option<String> = row.get(14)?;
        Ok(CullDecision {
            id: row.get(0)?,
            session_id: row.get(1)?,
            path: row.get(2)?,
            version: row.get(3)?,
            backing: from_json(&row.get::<_, String>(4)?)?,
            proposed,
            final_status,
            status: final_status.unwrap_or(proposed),
            reason: CullReason::parse(&row.get::<_, String>(7)?).unwrap_or(CullReason::Unique),
            group: row.get(8)?,
            best_path: row.get(9)?,
            best: row.get(10)?,
            highlight: row.get(11)?,
            quality_score: row.get(12)?,
            factors: from_json(&row.get::<_, String>(13)?)?,
            blurry_region: blur.map(|b| from_json(&b)).transpose()?,
            analysis: from_json(&row.get::<_, String>(15)?)?,
            file_size: row.get(16)?,
            file_modified_ms: row.get(17)?,
            applied: row.get(18)?,
        })
    }

    /// Best score first within the session's own order (path).
    pub fn decisions(&self, session_id: i64) -> Result<Vec<CullDecision>, String> {
        let mut statement = self
            .db
            .prepare(&format!("SELECT {DECISION_COLUMNS} FROM cull_decisions WHERE session_id = ?1 ORDER BY path, version"))
            .map_err(|e| e.to_string())?;
        let rows = statement.query_map([session_id], Self::read_decision).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn decision(&self, id: i64) -> Result<Option<CullDecision>, String> {
        self.db
            .query_row(&format!("SELECT {DECISION_COLUMNS} FROM cull_decisions WHERE id = ?1"), [id], Self::read_decision)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Records the user's call on a photo, and the event. `make_best` also
    /// takes the set's best mark from the frame that had it. Returns the
    /// previous effective status.
    pub fn override_decision(&mut self, id: i64, status: CullStatus, make_best: bool, source: &str) -> Result<Option<CullStatus>, String> {
        let Some(current) = self.decision(id)? else { return Ok(None) };
        let tx = self.db.transaction().map_err(|e| e.to_string())?;
        tx.execute("UPDATE cull_decisions SET final_status = ?2 WHERE id = ?1", params![id, status.as_str()]).map_err(|e| e.to_string())?;
        if make_best {
            if let Some(group) = current.group {
                tx.execute("UPDATE cull_decisions SET best = 0 WHERE session_id = ?1 AND group_no = ?2", params![current.session_id, group])
                    .map_err(|e| e.to_string())?;
            }
            tx.execute("UPDATE cull_decisions SET best = 1 WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT INTO cull_decision_events(decision_id, previous_status, next_status, source, created_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![id, current.status.as_str(), status.as_str(), source, now()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE cull_sessions SET reject_count = (SELECT COUNT(*) FROM cull_decisions d WHERE d.session_id = cull_sessions.id
                                                      AND COALESCE(d.final_status, d.proposed) = 'reject'),
                                      keep_count = (SELECT COUNT(*) FROM cull_decisions d WHERE d.session_id = cull_sessions.id
                                                    AND COALESCE(d.final_status, d.proposed) = 'keep'),
                                      updated_at = ?2
             WHERE id = ?1",
            params![current.session_id, now()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some(current.status))
    }

    /// The previously recorded library entries of one decision.
    pub fn applied_records(&self, id: i64) -> Result<Vec<CullApplied>, String> {
        let text: Option<String> = self
            .db
            .query_row("SELECT applied_json FROM cull_decisions WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
    }

    /// Notes what Apply wrote for a decision, or (empty) that Undo took it back.
    pub fn set_applied(&self, id: i64, records: &[CullApplied]) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE cull_decisions SET applied = ?2, applied_json = ?3 WHERE id = ?1",
                params![id, !records.is_empty(), json(&records)],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn set_state(&self, id: i64, state: CullSessionState) -> Result<(), String> {
        self.db
            .execute("UPDATE cull_sessions SET state = ?2, updated_at = ?3 WHERE id = ?1", params![id, state.as_str(), now()])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    #[cfg(test)]
    pub fn event_count(&self, decision_id: i64) -> i64 {
        self.db.query_row("SELECT COUNT(*) FROM cull_decision_events WHERE decision_id = ?1", [decision_id], |r| r.get(0)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cull::{analyze, tests::sharp_scene};
    use crate::cull_plan::{plan, Input};

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cull.sqlite")).unwrap();
        (dir, store)
    }

    fn items() -> Vec<Item> {
        let frame = analyze(&sharp_scene(160, 120, 0));
        let other = analyze(&sharp_scene(160, 120, 1));
        let inputs = vec![
            Input { path: "/p/a.jpg".into(), analysis: frame.result.clone(), hash: frame.hash },
            Input { path: "/p/b.jpg".into(), analysis: other.result.clone(), hash: other.hash },
        ];
        let settings = CullSettings { group_similar: false, ..Default::default() };
        plan(&inputs, &settings)
            .into_iter()
            .map(|planned| Item {
                target: CullTarget { path: planned.path.clone(), version: 0 },
                backing: vec![],
                size: 10,
                modified_ms: 20,
                planned,
            })
            .collect()
    }

    #[test]
    fn a_session_persists_with_its_decisions() {
        let (_dir, mut store) = store();
        let failed = vec![CullFailure { path: "/p/bad.jpg".into(), error: "cannot decode".into() }];
        let (session, decisions) = store.create_session("Folder", &CullSettings::default(), items(), failed).unwrap();
        assert_eq!((session.state, session.total_count, session.failed.len()), (CullSessionState::Planned, 2, 1));
        assert_eq!(decisions.len(), 2);
        assert!(decisions.iter().all(|d| d.final_status.is_none() && d.status == d.proposed && !d.applied));
        assert_eq!(store.sessions().unwrap().len(), 1);
        assert_eq!(store.decisions(session.id).unwrap()[0].path, "/p/a.jpg");
        assert!(store.delete_session(session.id).unwrap());
        assert!(store.decisions(session.id).unwrap().is_empty(), "decisions cascade");
        assert!(!store.delete_session(session.id).unwrap());
    }

    #[test]
    fn an_override_updates_counts_and_records_an_event() {
        let (_dir, mut store) = store();
        let (session, decisions) = store.create_session("Folder", &CullSettings::default(), items(), vec![]).unwrap();
        let first = &decisions[0];
        assert_eq!(first.status, CullStatus::Keep);
        let before = store.override_decision(first.id, CullStatus::Reject, false, "user").unwrap();
        assert_eq!(before, Some(CullStatus::Keep));
        let after = store.decision(first.id).unwrap().unwrap();
        assert_eq!((after.proposed, after.final_status, after.status), (CullStatus::Keep, Some(CullStatus::Reject), CullStatus::Reject));
        let s = store.session(session.id).unwrap().unwrap();
        assert_eq!((s.keep_count, s.reject_count), (1, 1));
        assert_eq!(store.event_count(first.id), 1);
        assert_eq!(store.override_decision(9999, CullStatus::Keep, false, "user").unwrap(), None);
    }

    #[test]
    fn make_best_moves_the_mark_within_a_set() {
        let (_dir, mut store) = store();
        let mut grouped = items();
        for (i, item) in grouped.iter_mut().enumerate() {
            item.planned.group = Some(1);
            item.planned.reason = if i == 0 { CullReason::BestInGroup } else { CullReason::DuplicateOf };
        }
        let (session, decisions) = store.create_session("Folder", &CullSettings::default(), grouped, vec![]).unwrap();
        assert!(decisions[0].best && !decisions[1].best);
        store.override_decision(decisions[1].id, CullStatus::Keep, true, "user").unwrap();
        let now = store.decisions(session.id).unwrap();
        assert!(!now[0].best && now[1].best);
    }

    #[test]
    fn applied_records_round_trip_and_clear() {
        let (_dir, mut store) = store();
        let (_, decisions) = store.create_session("Folder", &CullSettings::default(), items(), vec![]).unwrap();
        let id = decisions[0].id;
        let record = CullApplied {
            target: CullTarget { path: "/p/a.jpg".into(), version: 0 },
            before: CullLibraryState { rating: 3, rejected: false, labels: 2 },
            after: CullLibraryState { rating: 0, rejected: true, labels: 2 },
        };
        store.set_applied(id, std::slice::from_ref(&record)).unwrap();
        assert!(store.decision(id).unwrap().unwrap().applied);
        assert_eq!(store.applied_records(id).unwrap(), vec![record]);
        store.set_applied(id, &[]).unwrap();
        assert!(!store.decision(id).unwrap().unwrap().applied);
        assert!(store.applied_records(id).unwrap().is_empty());
    }

    #[test]
    fn the_analysis_cache_is_keyed_by_file_identity() {
        let (_dir, store) = store();
        let target = CullTarget { path: "/p/a.jpg".into(), version: 0 };
        let frame = analyze(&sharp_scene(64, 64, 0));
        store.cache_put(&target, 100, 200, &crate::cull::hash_to_string(&frame.hash), &frame.result).unwrap();
        let (analysis, hash) = store.cached(&target, 100, 200).unwrap();
        assert_eq!(analysis.sharpness_metric, frame.result.sharpness_metric);
        assert!(crate::cull::hash_from_string(&hash).is_some());
        assert!(store.cached(&target, 101, 200).is_none(), "size changed");
        assert!(store.cached(&target, 100, 201).is_none(), "modified");
        assert!(store.cached(&CullTarget { version: 1, ..target.clone() }, 100, 200).is_none(), "another version");
        assert_eq!(store.cache_clear().unwrap(), 1);
        assert!(store.cached(&target, 100, 200).is_none());
    }

    #[test]
    fn state_changes_stick() {
        let (_dir, mut store) = store();
        let (session, _) = store.create_session("Folder", &CullSettings::default(), items(), vec![]).unwrap();
        store.set_state(session.id, CullSessionState::Applied).unwrap();
        assert_eq!(store.session(session.id).unwrap().unwrap().state, CullSessionState::Applied);
        store.set_state(session.id, CullSessionState::Undone).unwrap();
        assert_eq!(store.session(session.id).unwrap().unwrap().state, CullSessionState::Undone);
    }
}

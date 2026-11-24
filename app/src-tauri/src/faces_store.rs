// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! `faces.sqlite`: the faces found in the user's photos, who they were named as, and how the
//! unnamed ones group. This is biometric data and lives only here, in the app's data folder.
//!
//! A photo is its path and original fingerprint (as in tags.sqlite); a changed file or a new model
//! version makes its faces stale and the photo is scanned again. A scan keeps what the user decided:
//! a new face that overlaps (IoU >= 0.5) a reviewed one of the same photo inherits its person and
//! state. Every change returns an undo entry that restores the rows it touched.
//! RAW+JPG companions (same folder and stem) are scanned once: the second file's faces are copies
//! (`companion_of`), and a review decision on one face is applied to its twin in the companion.
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 3;
pub const MAX_NAME: usize = 80;
const UNDO_DEPTH: usize = 20;

/// One migration per schema version; `LADDER[i]` takes the database from version i to i + 1.
const LADDER: &[&str] = &["
CREATE TABLE photos (
    path TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    model TEXT NOT NULL,
    scanned_at INTEGER NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'done',
    error TEXT,
    companion_of TEXT
);
CREATE TABLE people (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','ignored','merged')),
    merged_into INTEGER,
    cover_face_id INTEGER,
    cover_manual INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX people_name ON people(name COLLATE NOCASE) WHERE state = 'active';
CREATE TABLE faces (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL REFERENCES photos(path) ON DELETE CASCADE,
    x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
    score REAL NOT NULL,
    lm BLOB NOT NULL,
    frontal REAL NOT NULL,
    roll REAL NOT NULL,
    aligned INTEGER NOT NULL,
    emb BLOB NOT NULL,
    model TEXT NOT NULL,
    person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
    state TEXT NOT NULL DEFAULT 'unreviewed' CHECK (state IN ('unreviewed','confirmed','rejected')),
    suggested_person INTEGER,
    suggested_sim REAL,
    cluster_id INTEGER,
    CHECK (x >= 0 AND y >= 0 AND w > 0 AND h > 0 AND x + w <= 1.0001 AND y + h <= 1.0001)
);
CREATE INDEX faces_path ON faces(path);
CREATE INDEX faces_person ON faces(person_id);
CREATE INDEX faces_cluster ON faces(cluster_id);
CREATE TABLE person_rejections (
    face_id INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL,
    PRIMARY KEY (face_id, person_id)
) WITHOUT ROWID;
CREATE TABLE clusters (
    id INTEGER PRIMARY KEY,
    rep_face_id INTEGER NOT NULL,
    size INTEGER NOT NULL
);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
", "
-- the user's answers about pairs: groups are recomputed, so a group pair is remembered by faces of each
CREATE TABLE groups_not_same (
    face_a INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    face_b INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    PRIMARY KEY (face_a, face_b)
) WITHOUT ROWID;
CREATE TABLE face_links (
    face_a INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    face_b INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    PRIMARY KEY (face_a, face_b)
) WITHOUT ROWID;
CREATE TABLE people_not_same (
    person_a INTEGER NOT NULL,
    person_b INTEGER NOT NULL,
    PRIMARY KEY (person_a, person_b)
) WITHOUT ROWID;
", "
-- the shot a file belongs to (shot_key), so counts are of photos and not of files
ALTER TABLE photos ADD COLUMN shot TEXT;
CREATE INDEX photos_shot ON photos(shot);
"];

/// The shot a file belongs to: its folder (a trailing `Processed` folder is the folder it was made from) and its
/// file name without extension, lower case. A RAW, its JPG and the same-named copy in `Processed/` are one shot;
/// a derivative with another name (`IMG_1-edit.jpg`) is not matched, since nothing says it is the same shot.
pub fn shot_key(path: &str) -> String {
    let p = Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let mut dir = p.parent().unwrap_or(Path::new(""));
    if dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("processed")) {
        dir = dir.parent().unwrap_or(dir);
    }
    format!("{}/{}", dir.to_string_lossy(), stem)
}

pub struct FaceStore {
    db: Mutex<Connection>,
    undo: Mutex<Vec<Undo>>,
    path: Option<std::path::PathBuf>,
}

/// A face as the scan produces it.
#[derive(Debug, Clone)]
pub struct NewFace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub lm: [f32; 10],
    pub frontal: f32,
    pub roll: f32,
    pub aligned: bool,
    pub emb: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct FaceRow {
    pub id: i64,
    pub path: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub frontal: f32,
    pub person_id: Option<i64>,
    pub state: String,
    pub suggested_person: Option<i64>,
    pub suggested_sim: Option<f32>,
    pub cluster_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct PersonRow {
    pub id: i64,
    pub name: String,
    pub state: String,
    pub faces: u32,
    pub photos: u32,
    pub cover_face_id: Option<i64>,
    pub cover_manual: bool,
    /// Unreviewed faces the analysis suggests for this person.
    pub suggestions: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct ClusterRow {
    pub id: i64,
    pub rep_face_id: i64,
    pub size: u32,
    /// The photos (shots) the group's faces are in.
    pub photos: u32,
    /// Up to four faces of the group for a collage, the representative first.
    pub members: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct FaceTotals {
    pub photos: u32,
    pub faces: u32,
    pub people: u32,
    pub unreviewed: u32,
    pub groups: u32,
    pub suggestions: u32,
    /// Unreviewed faces nobody is suggested for and no group holds (the loose ones).
    pub unknown: u32,
    /// Faces marked "not a face" (the Ignored section).
    pub ignored_faces: u32,
    /// People who were removed and can be restored.
    pub ignored_people: u32,
    /// Photos the scan gave up on (could not be read).
    pub failed_photos: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct ModelCount {
    pub model: String,
    pub faces: u32,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct StoreStats {
    /// Files scanned, and the photos (shots) they make up.
    pub files: u32,
    pub photos: u32,
    pub faces: u32,
    pub people: u32,
    pub ignored_people: u32,
    pub groups: u32,
    pub unassigned: u32,
    pub rejected: u32,
    pub failed: u32,
    pub db_bytes: u64,
    /// Faces per model version stored (more than one while a recognizer change is being re-scanned).
    pub models: Vec<ModelCount>,
}

/// The photos of a person (confirmed first), one page of them and the total.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct PersonPhotos {
    pub paths: Vec<String>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct EyePair {
    pub left_open: f32,
    pub right_open: f32,
}

/// A face of one photo for the editor's overlay. The box is in the picture's displayed orientation
/// (faces are found on the oriented picture) and does not follow crops or rotations made in the editor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct PhotoFace {
    pub face: i64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub person: Option<i64>,
    pub name: Option<String>,
    /// "confirmed", "suggested", "unassigned" or "rejected"
    pub state: String,
    pub detector_confidence: f32,
    /// Only with the optional eye model, which this read never runs (`faces_eye_state` does).
    pub eyes: Option<EyePair>,
}

/// Which photos a People filter allows: `allow` (when the filter names a positive condition) and `deny`.
#[derive(Debug, Clone, Default)]
pub struct PathSets {
    pub allow: Option<std::collections::HashSet<String>>,
    pub deny: std::collections::HashSet<String>,
}
impl PathSets {
    pub fn passes(&self, path: &str) -> bool {
        self.allow.as_ref().is_none_or(|a| a.contains(path)) && !self.deny.contains(path)
    }
}

/// One side of a suggested pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Side {
    Group { id: i64 },
    Person { id: i64 },
}

/// The pairs the suggestions skip, by current group and person ids (each pair ordered low, high).
#[derive(Debug, Clone, Default)]
pub struct Skips {
    pub groups: std::collections::HashSet<(i64, i64)>,
    pub people: std::collections::HashSet<(i64, i64)>,
    /// (group, person): the user said the group is not them.
    pub group_person: std::collections::HashSet<(i64, i64)>,
}

/// An unnamed face with what the grouping judges it by: size in pixels of the analysed picture, detector confidence, how frontal.
#[derive(Debug, Clone)]
pub struct Unnamed {
    pub id: i64,
    pub emb: Vec<f32>,
    /// People the user said this face is not.
    pub skip: Vec<i64>,
    pub px: f32,
    pub score: f32,
    pub frontal: f32,
}

/// Where a list of faces comes from.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FaceFilter {
    Person { id: i64 },
    Cluster { id: i64 },
    /// Unreviewed faces suggested for this person.
    Suggested { id: i64 },
    /// Unreviewed faces with no suggestion and no group.
    Unknown,
    Photo { path: String },
    /// Faces marked "not a face".
    Ignored,
}

#[derive(Debug, Clone)]
struct FaceState {
    id: i64,
    person_id: Option<i64>,
    state: String,
    suggested_person: Option<i64>,
    suggested_sim: Option<f64>,
    cluster_id: Option<i64>,
}
#[derive(Debug, Clone)]
struct PersonState {
    id: i64,
    name: String,
    state: String,
    merged_into: Option<i64>,
    cover_face_id: Option<i64>,
    cover_manual: i64,
}
/// What one change touched, before it: restoring these rows undoes it.
#[derive(Debug, Clone, Default)]
pub struct Undo {
    pub label: String,
    faces: Vec<FaceState>,
    people: Vec<PersonState>,
    rejections_added: Vec<(i64, i64)>,
    rejections_removed: Vec<(i64, i64)>,
    created_person: Option<i64>,
    /// Group rows as they were (id, representative, size), put back by an undo.
    clusters: Vec<(i64, i64, i64)>,
    links_added: Vec<(i64, i64)>,
    groups_not_same_added: Vec<(i64, i64)>,
    people_not_same_added: Vec<(i64, i64)>,
}

fn emb_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
pub fn emb_from(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn clean_name(name: &str) -> Result<String, String> {
    let n: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() || n.chars().count() > MAX_NAME || n.chars().any(char::is_control) {
        return Err(format!("A name needs 1 to {MAX_NAME} characters"));
    }
    Ok(n)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn face_from(r: &rusqlite::Row) -> rusqlite::Result<FaceRow> {
    Ok(FaceRow {
        id: r.get(0)?,
        path: r.get(1)?,
        x: r.get(2)?,
        y: r.get(3)?,
        w: r.get(4)?,
        h: r.get(5)?,
        score: r.get(6)?,
        frontal: r.get(7)?,
        person_id: r.get(8)?,
        state: r.get(9)?,
        suggested_person: r.get(10)?,
        suggested_sim: r.get::<_, Option<f64>>(11)?.map(|v| v as f32),
        cluster_id: r.get(12)?,
    })
}
const FACE_COLS: &str = "id,path,x,y,w,h,score,frontal,person_id,state,suggested_person,suggested_sim,cluster_id";

fn iou(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
    let iw = ((a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0)).max(0.0);
    let ih = ((a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1)).max(0.0);
    let inter = iw * ih;
    let union = a.2 * a.3 + b.2 * b.3 - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

impl FaceStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        Self::from_connection(db).map(|s| FaceStore { path: Some(path.to_path_buf()), ..s })
    }
    pub fn from_connection(db: Connection) -> Result<Self, String> {
        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
        let _ = db.pragma_update(None, "journal_mode", "WAL");
        db.pragma_update(None, "foreign_keys", true).map_err(|e| e.to_string())?;
        let mut version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if version > SCHEMA_VERSION {
            return Err("faces.sqlite was written by a newer Immage".into());
        }
        while (version as usize) < LADDER.len() {
            db.execute_batch(&format!("BEGIN; {} PRAGMA user_version = {}; COMMIT;", LADDER[version as usize], version + 1))
                .map_err(|e| e.to_string())?;
            version += 1;
        }
        // files scanned before shots were recorded get theirs now
        {
            let mut q = db.prepare("SELECT path FROM photos WHERE shot IS NULL").map_err(|e| e.to_string())?;
            let paths: Vec<String> = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
            drop(q);
            for p in paths {
                db.execute("UPDATE photos SET shot=?2 WHERE path=?1", params![p, shot_key(&p)]).map_err(|e| e.to_string())?;
            }
        }
        Ok(Self { db: Mutex::new(db), undo: Mutex::new(Vec::new()), path: None })
    }
    fn db(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.db.lock().map_err(|_| "face store lock poisoned".to_string())
    }
    pub fn schema_version(&self) -> Result<i64, String> {
        self.db()?.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())
    }

    // ---- scanning ---------------------------------------------------------------

    pub fn is_current(&self, path: &str, fingerprint: &str, model: &str) -> Result<bool, String> {
        self.db()?
            .query_row("SELECT 1 FROM photos WHERE path=?1 AND fingerprint=?2 AND model=?3", params![path, fingerprint, model], |_| Ok(()))
            .optional()
            .map(|r| r.is_some())
            .map_err(|e| e.to_string())
    }

    /// The photo of the same folder and stem that was scanned already (a RAW and its JPG), with
    /// the same picture shape, if any.
    pub fn companion(&self, path: &str, width: u32, height: u32, model: &str) -> Result<Option<String>, String> {
        let p = Path::new(path);
        let (Some(dir), Some(stem)) = (p.parent().and_then(|d| d.to_str()), p.file_stem().and_then(|s| s.to_str())) else {
            return Ok(None);
        };
        let like = format!("{}/{}.%", dir.replace('%', "\\%").replace('_', "\\_"), stem.replace('%', "\\%").replace('_', "\\_"));
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT path,width,height FROM photos WHERE path LIKE ?1 ESCAPE '\\' AND path<>?2 AND model=?3 AND status='done' AND companion_of IS NULL AND fingerprint<>'' ORDER BY path")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map(params![like, path, model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (other, w, h) = row.map_err(|e| e.to_string())?;
            // the same stem but another shape is another picture; the aspect ratio must agree within 1 %
            let (a, b) = (width as f64 / height as f64, w as f64 / h as f64);
            if (a / b - 1.0).abs() < 0.01 {
                return Ok(Some(other));
            }
        }
        Ok(None)
    }

    /// Stores a photo's faces, replacing the earlier result, keeping what the user decided about
    /// faces that are still there.
    #[allow(clippy::too_many_arguments)]
    pub fn put_scan(&self, path: &str, fingerprint: &str, model: &str, width: u32, height: u32, faces: &[NewFace], companion_of: Option<&str>) -> Result<Vec<i64>, String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        struct Old {
            id: i64,
            bbox: (f64, f64, f64, f64),
            person: Option<i64>,
            state: String,
        }
        let mut old: Vec<Old> = Vec::new();
        {
            let mut q = tx.prepare("SELECT id,x,y,w,h,person_id,state FROM faces WHERE path=?1").map_err(|e| e.to_string())?;
            let rows = q
                .query_map([path], |r| Ok(Old { id: r.get(0)?, bbox: (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?), person: r.get(5)?, state: r.get(6)? }))
                .map_err(|e| e.to_string())?;
            for row in rows {
                old.push(row.map_err(|e| e.to_string())?);
            }
        }
        // what hangs on the old faces: "not this person" answers, joined or separate groups, covers
        let pairs_of = |table: &str, a: &str, b: &str| -> Result<Vec<(i64, i64)>, String> {
            let mut out = Vec::new();
            let mut q = tx.prepare(&format!("SELECT {a},{b} FROM {table}")).map_err(|e| e.to_string())?;
            for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())? {
                let (x, y) = row.map_err(|e| e.to_string())?;
                if old.iter().any(|o| o.id == x || o.id == y) {
                    out.push((x, y));
                }
            }
            Ok(out)
        };
        let links = pairs_of("face_links", "face_a", "face_b")?;
        let apart = pairs_of("groups_not_same", "face_a", "face_b")?;
        let rejections = pairs_of("person_rejections", "face_id", "person_id")?;
        let covers: Vec<(i64, i64)> = {
            let mut q = tx.prepare("SELECT id,cover_face_id FROM people WHERE cover_face_id IS NOT NULL").map_err(|e| e.to_string())?;
            let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?.into_iter().filter(|(_, f)| old.iter().any(|o| o.id == *f)).collect()
        };
        tx.execute("DELETE FROM faces WHERE path=?1", [path]).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO photos(path,fingerprint,model,scanned_at,width,height,status,error,companion_of,shot) VALUES(?1,?2,?3,?4,?5,?6,'done',NULL,?7,?8)
             ON CONFLICT(path) DO UPDATE SET fingerprint=?2,model=?3,scanned_at=?4,width=?5,height=?6,status='done',error=NULL,companion_of=?7,shot=?8",
            params![path, fingerprint, model, now(), width, height, companion_of, shot_key(path)],
        )
        .map_err(|e| e.to_string())?;
        // a new face is the old one it overlaps most (IoU >= 0.5), each old face at most once: a face is known by
        // where it is in its photo, so a re-embed by another model keeps who it was named as
        let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
        for (n, f) in faces.iter().enumerate() {
            let bbox = (f64::from(f.x), f64::from(f.y), f64::from(f.w), f64::from(f.h));
            for (k, o) in old.iter().enumerate() {
                let v = iou(o.bbox, bbox);
                if v >= 0.5 {
                    candidates.push((v, n, k));
                }
            }
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut matched: Vec<Option<usize>> = vec![None; faces.len()];
        let mut taken = vec![false; old.len()];
        for (_, n, k) in candidates {
            if matched[n].is_none() && !taken[k] {
                matched[n] = Some(k);
                taken[k] = true;
            }
        }
        let mut ids = Vec::new();
        let mut renamed: HashMap<i64, i64> = HashMap::new();
        for (n, f) in faces.iter().enumerate() {
            let inherit = matched[n].map(|k| &old[k]);
            let (person, state) = match inherit {
                Some(o) if o.state != "unreviewed" => (o.person, o.state.clone()),
                _ => (None, "unreviewed".to_string()),
            };
            tx.execute(
                "INSERT INTO faces(path,x,y,w,h,score,lm,frontal,roll,aligned,emb,model,person_id,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![path, f.x, f.y, f.w, f.h, f.score, emb_blob(&f.lm), f.frontal, f.roll, f.aligned, emb_blob(&f.emb), model, person, state],
            )
            .map_err(|e| e.to_string())?;
            let id = tx.last_insert_rowid();
            if let Some(o) = inherit {
                renamed.insert(o.id, id);
            }
            ids.push(id);
        }
        let new_id = |x: i64| -> Option<i64> { if old.iter().any(|o| o.id == x) { renamed.get(&x).copied() } else { Some(x) } };
        for (f, p) in rejections {
            if let Some(f) = new_id(f) {
                tx.execute("INSERT OR IGNORE INTO person_rejections(face_id,person_id) VALUES(?1,?2)", params![f, p]).map_err(|e| e.to_string())?;
            }
        }
        for (table, rows) in [("face_links", links), ("groups_not_same", apart)] {
            for (x, y) in rows {
                if let (Some(x), Some(y)) = (new_id(x), new_id(y)) {
                    tx.execute(&format!("INSERT OR IGNORE INTO {table}(face_a,face_b) VALUES(?1,?2)"), params![x.min(y), x.max(y)]).map_err(|e| e.to_string())?;
                }
            }
        }
        for (person, cover) in covers {
            match renamed.get(&cover) {
                Some(n) => {
                    tx.execute("UPDATE people SET cover_face_id=?2 WHERE id=?1", params![person, n]).map_err(|e| e.to_string())?;
                }
                None => {
                    tx.execute("UPDATE people SET cover_face_id=NULL,cover_manual=0 WHERE id=?1", [person]).map_err(|e| e.to_string())?;
                }
            }
        }
        // a person who lost a face (it is no longer found) may need another cover
        let people: Vec<i64> = old.iter().filter_map(|o| o.person).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        for p in people {
            Self::refresh_cover(&tx, p)?;
        }
        if let Some(primary) = companion_of {
            // a mirror starts with what the user already decided about the primary's faces
            for id in &ids {
                tx.execute(
                    "UPDATE faces SET person_id=t.person_id, state=t.state FROM (
                         SELECT person_id,state FROM faces p
                         WHERE p.path=?2 AND p.state<>'unreviewed'
                           AND ABS((p.x+p.w/2)-(SELECT x+w/2 FROM faces WHERE id=?1)) < 0.05
                           AND ABS((p.y+p.h/2)-(SELECT y+h/2 FROM faces WHERE id=?1)) < 0.05
                         LIMIT 1) AS t WHERE faces.id=?1",
                    params![id, primary],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(ids)
    }

    /// The faces of a photo as a scan would store them (for a RAW+JPG companion to copy).
    pub fn new_faces_of(&self, path: &str) -> Result<Vec<NewFace>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT x,y,w,h,score,lm,frontal,roll,aligned,emb FROM faces WHERE path=?1 AND state<>'rejected' ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([path], |r| {
                let lm: Vec<f32> = emb_from(&r.get::<_, Vec<u8>>(5)?);
                Ok(NewFace {
                    x: r.get(0)?,
                    y: r.get(1)?,
                    w: r.get(2)?,
                    h: r.get(3)?,
                    score: r.get(4)?,
                    lm: lm.try_into().unwrap_or([0.0; 10]),
                    frontal: r.get(6)?,
                    roll: r.get(7)?,
                    aligned: r.get::<_, i64>(8)? != 0,
                    emb: emb_from(&r.get::<_, Vec<u8>>(9)?),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// Marks every scanned photo stale (its file fingerprint is cleared), so the scan reads them all again. People,
    /// review decisions, covers and pair answers stay: a re-scan matches new faces to old ones by where they are in the
    /// photo. Returns how many photos were marked.
    pub fn mark_all_stale(&self) -> Result<u32, String> {
        let db = self.db()?;
        db.execute("UPDATE photos SET fingerprint='' WHERE fingerprint<>''", []).map(|n| n as u32).map_err(|e| e.to_string())
    }

    /// A photo the scan could not process: remembered so it is not retried at every pass.
    pub fn put_failure(&self, path: &str, fingerprint: &str, model: &str, status: &str, error: &str) -> Result<(), String> {
        let db = self.db()?;
        db.execute("DELETE FROM faces WHERE path=?1", [path]).map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO photos(path,fingerprint,model,scanned_at,width,height,status,error,companion_of,shot) VALUES(?1,?2,?3,?4,0,0,?5,?6,NULL,?7)
             ON CONFLICT(path) DO UPDATE SET fingerprint=?2,model=?3,scanned_at=?4,width=0,height=0,status=?5,error=?6,companion_of=NULL,shot=?7",
            params![path, fingerprint, model, now(), status, error, shot_key(path)],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Forgets a photo that no longer exists (its faces go with it; people stay).
    pub fn forget_photo(&self, path: &str) -> Result<(), String> {
        self.db()?.execute("DELETE FROM photos WHERE path=?1", [path]).map(|_| ()).map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---- reading ------------------------------------------------------------------

    pub fn face(&self, id: i64) -> Result<Option<FaceRow>, String> {
        self.db()?
            .query_row(&format!("SELECT {FACE_COLS} FROM faces WHERE id=?1"), [id], face_from)
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn embedding(&self, id: i64) -> Result<Option<Vec<f32>>, String> {
        self.db()?
            .query_row("SELECT emb FROM faces WHERE id=?1", [id], |r| r.get::<_, Vec<u8>>(0))
            .optional()
            .map(|o| o.map(|b| emb_from(&b)))
            .map_err(|e| e.to_string())
    }
    pub fn landmarks(&self, id: i64) -> Result<Option<[f32; 10]>, String> {
        let blob: Option<Vec<u8>> = self.db()?.query_row("SELECT lm FROM faces WHERE id=?1", [id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        Ok(blob.and_then(|b| emb_from(&b).try_into().ok()))
    }
    pub fn faces_of_photo(&self, path: &str) -> Result<Vec<FaceRow>, String> {
        self.list_faces(&FaceFilter::Photo { path: path.to_string() }, 0, 10_000)
    }

    pub fn list_faces(&self, filter: &FaceFilter, offset: u32, limit: u32) -> Result<Vec<FaceRow>, String> {
        let (clause, arg): (&str, rusqlite::types::Value) = match filter {
            FaceFilter::Person { id } => ("person_id=?1 AND state<>'rejected'", (*id).into()),
            FaceFilter::Cluster { id } => ("cluster_id=?1 AND state='unreviewed' AND person_id IS NULL", (*id).into()),
            FaceFilter::Suggested { id } => ("suggested_person=?1 AND state='unreviewed' AND person_id IS NULL", (*id).into()),
            FaceFilter::Unknown => ("state='unreviewed' AND person_id IS NULL AND suggested_person IS NULL AND cluster_id IS NULL AND ?1=0", 0i64.into()),
            FaceFilter::Photo { path } => ("path=?1 AND state<>'rejected'", path.clone().into()),
            FaceFilter::Ignored => ("state='rejected' AND ?1=0", 0i64.into()),
        };
        let order = match filter {
            FaceFilter::Suggested { .. } => "suggested_sim DESC, id",
            _ => "score DESC, id",
        };
        let db = self.db()?;
        let mut q = db
            .prepare(&format!("SELECT {FACE_COLS} FROM faces WHERE {clause} ORDER BY {order} LIMIT ?2 OFFSET ?3"))
            .map_err(|e| e.to_string())?;
        let rows = q.query_map(params![arg, limit.min(10_000), offset], face_from).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn list_people(&self, include_ignored: bool) -> Result<Vec<PersonRow>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare(
                "SELECT p.id,p.name,p.state,
                   (SELECT COUNT(*) FROM faces f WHERE f.person_id=p.id AND f.state='confirmed'),
                   (SELECT COUNT(DISTINCT ph.shot) FROM faces f JOIN photos ph ON ph.path=f.path WHERE f.person_id=p.id AND f.state='confirmed'),
                   p.cover_face_id,p.cover_manual,
                   (SELECT COUNT(*) FROM faces f WHERE f.suggested_person=p.id AND f.state='unreviewed' AND f.person_id IS NULL)
                 FROM people p WHERE p.state='active' OR (?1 AND p.state='ignored')
                 ORDER BY p.name COLLATE NOCASE",
            )
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([include_ignored], |r| {
                Ok(PersonRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    state: r.get(2)?,
                    faces: r.get(3)?,
                    photos: r.get(4)?,
                    cover_face_id: r.get(5)?,
                    cover_manual: r.get::<_, i64>(6)? != 0,
                    suggestions: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn list_clusters(&self) -> Result<Vec<ClusterRow>, String> {
        let db = self.db()?;
        let mut members: HashMap<i64, Vec<i64>> = HashMap::new();
        {
            let mut q = db
                .prepare(
                    "SELECT cluster_id,id FROM (
                         SELECT f.cluster_id,f.id,ROW_NUMBER() OVER (PARTITION BY f.cluster_id ORDER BY (f.id=c.rep_face_id) DESC, f.score DESC, f.id) AS rn
                         FROM faces f JOIN clusters c ON c.id=f.cluster_id WHERE f.state='unreviewed' AND f.person_id IS NULL)
                     WHERE rn<=4 ORDER BY cluster_id, rn",
                )
                .map_err(|e| e.to_string())?;
            for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())? {
                let (c, f) = row.map_err(|e| e.to_string())?;
                members.entry(c).or_default().push(f);
            }
        }
        let mut shots: HashMap<i64, u32> = HashMap::new();
        {
            let mut q = db
                .prepare("SELECT f.cluster_id,COUNT(DISTINCT p.shot) FROM faces f JOIN photos p ON p.path=f.path WHERE f.cluster_id IS NOT NULL AND f.state='unreviewed' AND f.person_id IS NULL GROUP BY f.cluster_id")
                .map_err(|e| e.to_string())?;
            for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?))).map_err(|e| e.to_string())? {
                let (c, n) = row.map_err(|e| e.to_string())?;
                shots.insert(c, n);
            }
        }
        let mut q = db.prepare("SELECT id,rep_face_id,size FROM clusters ORDER BY size DESC, id").map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, u32>(2)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (id, rep, size) = row.map_err(|e| e.to_string())?;
            let photos = shots.get(&id).copied().unwrap_or(0);
            out.push(ClusterRow { id, rep_face_id: rep, size, photos, members: members.remove(&id).unwrap_or_default() });
        }
        Ok(out)
    }

    /// Photos the scan gave up on, for the strip's "could not be read".
    pub fn failed_photos(&self, limit: u32) -> Result<Vec<String>, String> {
        let db = self.db()?;
        let mut q = db.prepare("SELECT path FROM photos WHERE status='failed' ORDER BY path LIMIT ?1").map_err(|e| e.to_string())?;
        let rows = q.query_map([limit], |r| r.get(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// Takes faces back from "not a face".
    pub fn unignore_faces(&self, faces: &[i64]) -> Result<(), String> {
        let db = self.db()?;
        let ids = Self::with_twins(&db, faces)?;
        let undo = Undo { label: "Restore".into(), faces: Self::snapshot_faces(&db, &ids)?, ..Undo::default() };
        for id in &ids {
            db.execute("UPDATE faces SET state='unreviewed' WHERE id=?1 AND state='rejected'", [id]).map_err(|e| e.to_string())?;
        }
        drop(db);
        self.push_undo(undo);
        Ok(())
    }

    /// Small durable values of the service (the folders waiting for a one-off scan).
    pub fn meta_get(&self, key: &str) -> Result<Option<String>, String> {
        self.db()?.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0)).optional().map_err(|e| e.to_string())
    }
    pub fn meta_set(&self, key: &str, value: &str) -> Result<(), String> {
        self.db()?
            .execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2", params![key, value])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn totals(&self) -> Result<FaceTotals, String> {
        let db = self.db()?;
        let one = |sql: &str| -> Result<u32, String> { db.query_row(sql, [], |r| r.get(0)).map_err(|e| e.to_string()) };
        Ok(FaceTotals {
            photos: one("SELECT COUNT(DISTINCT shot) FROM photos WHERE status='done'")?,
            faces: one("SELECT COUNT(*) FROM faces WHERE state<>'rejected'")?,
            people: one("SELECT COUNT(*) FROM people WHERE state='active'")?,
            unreviewed: one("SELECT COUNT(*) FROM faces WHERE state='unreviewed' AND person_id IS NULL")?,
            groups: one("SELECT COUNT(*) FROM clusters")?,
            suggestions: one("SELECT COUNT(*) FROM faces WHERE state='unreviewed' AND person_id IS NULL AND suggested_person IS NOT NULL")?,
            unknown: one("SELECT COUNT(*) FROM faces WHERE state='unreviewed' AND person_id IS NULL AND suggested_person IS NULL AND cluster_id IS NULL")?,
            ignored_faces: one("SELECT COUNT(*) FROM faces WHERE state='rejected'")?,
            ignored_people: one("SELECT COUNT(*) FROM people WHERE state='ignored'")?,
            failed_photos: one("SELECT COUNT(*) FROM photos WHERE status='failed'")?,
        })
    }

    /// The files of a person's photos, confirmed first, then those with only a suggested face of theirs. All files are
    /// returned (a RAW, its JPG and a same-named `Processed/` copy each appear), but files of one shot sit together, and
    /// `total` counts shots, not files. Within a group of equal rank, path order. `offset` and `limit` count files.
    pub fn person_photos(&self, person: i64, offset: u32, limit: u32) -> Result<PersonPhotos, String> {
        let db = self.db()?;
        let inner = "SELECT f.path AS path, ph.shot AS shot, MAX(f.person_id=?1 AND f.state='confirmed') AS c
                     FROM faces f JOIN photos ph ON ph.path=f.path
                     WHERE (f.person_id=?1 AND f.state='confirmed') OR (f.suggested_person=?1 AND f.state='unreviewed' AND f.person_id IS NULL)
                     GROUP BY f.path";
        let total: u32 = db.query_row(&format!("SELECT COUNT(DISTINCT shot) FROM ({inner})"), [person], |r| r.get(0)).map_err(|e| e.to_string())?;
        let mut q = db
            .prepare(&format!("SELECT path FROM (SELECT path, shot, MAX(c) OVER (PARTITION BY shot) AS sc FROM ({inner})) ORDER BY sc DESC, shot, path LIMIT ?2 OFFSET ?3"))
            .map_err(|e| e.to_string())?;
        let rows = q.query_map(params![person, limit.min(10_000), offset], |r| r.get(0)).map_err(|e| e.to_string())?;
        Ok(PersonPhotos { paths: rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?, total })
    }

    /// The faces of one photo, left to right: one indexed read, no model and no file access.
    pub fn photo_faces(&self, path: &str) -> Result<Vec<PhotoFace>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare(
                "SELECT f.id,f.x,f.y,f.w,f.h,f.person_id,f.suggested_person,f.state,f.score,pc.name,ps.name
                 FROM faces f LEFT JOIN people pc ON pc.id=f.person_id LEFT JOIN people ps ON ps.id=f.suggested_person
                 WHERE f.path=?1 ORDER BY f.x, f.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([path], |r| {
                let (person, suggested, state): (Option<i64>, Option<i64>, String) = (r.get(5)?, r.get(6)?, r.get(7)?);
                let (name, sname): (Option<String>, Option<String>) = (r.get(9)?, r.get(10)?);
                let (state, person, name) = if state == "rejected" {
                    ("rejected", None, None)
                } else if person.is_some() {
                    ("confirmed", person, name)
                } else if suggested.is_some() {
                    ("suggested", suggested, sname)
                } else {
                    ("unassigned", None, None)
                };
                Ok(PhotoFace { face: r.get(0)?, x: r.get(1)?, y: r.get(2)?, w: r.get(3)?, h: r.get(4)?, person, name, state: state.to_string(), detector_confidence: r.get(8)?, eyes: None })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// An id as it stands now: a merged person is the one they were merged into, a removed or unknown one nobody.
    fn live_person(db: &Connection, mut id: i64) -> Result<Option<i64>, String> {
        for _ in 0..16 {
            let row: Option<(String, Option<i64>)> =
                db.query_row("SELECT state,merged_into FROM people WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(|e| e.to_string())?;
            match row {
                Some((s, _)) if s == "active" => return Ok(Some(id)),
                Some((s, Some(next))) if s == "merged" => id = next,
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    /// The photos a People filter allows or denies (see `PeopleFilter`). Merged people count as who they became;
    /// removed or unknown people are ignored, so a saved filter with one still works.
    pub fn filter_paths(&self, f: &crate::search_filters::PeopleFilter) -> Result<PathSets, String> {
        use std::collections::HashSet;
        let max = crate::search_filters::MAX_FILTER_PEOPLE;
        if f.include.len() > max || f.exclude.len() > max {
            return Err("Too many people in the filter".into());
        }
        let db = self.db()?;
        let live = |ids: &[i64]| -> Result<Vec<i64>, String> {
            let mut v = Vec::new();
            for id in ids {
                if let Some(p) = Self::live_person(&db, *id)? {
                    if !v.contains(&p) {
                        v.push(p);
                    }
                }
            }
            Ok(v)
        };
        let (include, exclude) = (live(&f.include)?, live(&f.exclude)?);
        let paths = |sql: &str, args: Vec<rusqlite::types::Value>| -> Result<HashSet<String>, String> {
            let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
            let rows = q.query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
            rows.collect::<Result<HashSet<_>, _>>().map_err(|e| e.to_string())
        };
        let marks = |n: usize| vec!["?"; n].join(",");
        let ids = |v: &[i64]| -> Vec<rusqlite::types::Value> { v.iter().map(|i| (*i).into()).collect() };
        let mut sets = PathSets::default();
        let mut narrow = |sets: &mut PathSets, s: HashSet<String>| {
            sets.allow = Some(match sets.allow.take() {
                Some(a) => a.intersection(&s).cloned().collect(),
                None => s,
            });
        };
        if !include.is_empty() {
            let having = if f.together { format!(" GROUP BY path HAVING COUNT(DISTINCT person_id)={}", include.len()) } else { " GROUP BY path".into() };
            narrow(&mut sets, paths(&format!("SELECT path FROM faces WHERE state='confirmed' AND person_id IN ({}){having}", marks(include.len())), ids(&include))?);
        }
        if !exclude.is_empty() {
            sets.deny = paths(&format!("SELECT DISTINCT path FROM faces WHERE state='confirmed' AND person_id IN ({})", marks(exclude.len())), ids(&exclude))?;
        }
        if f.unnamed {
            narrow(&mut sets, paths("SELECT DISTINCT path FROM faces WHERE state='unreviewed' AND person_id IS NULL", vec![])?);
        }
        if f.none {
            narrow(&mut sets, paths("SELECT path FROM photos p WHERE status='done' AND NOT EXISTS (SELECT 1 FROM faces f WHERE f.path=p.path AND f.state<>'rejected')", vec![])?);
        }
        Ok(sets)
    }

    /// Read-only aggregates for the Faces card in Preferences.
    pub fn stats(&self) -> Result<StoreStats, String> {
        let t = self.totals()?;
        let db = self.db()?;
        let one = |sql: &str| -> Result<u32, String> { db.query_row(sql, [], |r| r.get(0)).map_err(|e| e.to_string()) };
        let files = one("SELECT COUNT(*) FROM photos WHERE status='done'")?;
        let mut q = db.prepare("SELECT model,COUNT(*) FROM faces GROUP BY model ORDER BY COUNT(*) DESC").map_err(|e| e.to_string())?;
        let models: Vec<(String, u32)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        let db_bytes = self.path.as_ref().map_or(0, |p| {
            ["", "-wal", "-shm"].iter().map(|x| std::fs::metadata(format!("{}{x}", p.display())).map_or(0, |m| m.len())).sum()
        });
        Ok(StoreStats { files, photos: t.photos, faces: t.faces, people: t.people, ignored_people: t.ignored_people, groups: t.groups, unassigned: t.unreviewed, rejected: t.ignored_faces, failed: t.failed_photos, db_bytes, models: models.into_iter().map(|(model, faces)| ModelCount { model, faces }).collect() })
    }

    /// Every confirmed face of an active person made by this model: what new faces are compared with.
    pub fn exemplars(&self, model: &str) -> Result<Vec<(i64, i64, Vec<f32>)>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT f.person_id,f.id,f.emb FROM faces f JOIN people p ON p.id=f.person_id WHERE f.state='confirmed' AND p.state='active' AND f.model=?1 ORDER BY f.id")
            .map_err(|e| e.to_string())?;
        let rows = q.query_map([model], |r| Ok((r.get(0)?, r.get(1)?, emb_from(&r.get::<_, Vec<u8>>(2)?)))).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
    /// The unreviewed faces nobody is named for (made by this model), with what the grouping needs to judge them.
    pub fn unnamed(&self, model: &str) -> Result<Vec<Unnamed>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare(
                "SELECT f.id,f.emb,MAX(f.w*p.width, f.h*p.height),f.score,f.frontal FROM faces f JOIN photos p ON p.path=f.path
                 WHERE f.state='unreviewed' AND f.person_id IS NULL AND f.model=?1 ORDER BY f.id",
            )
            .map_err(|e| e.to_string())?;
        let faces: Vec<Unnamed> = q
            .query_map([model], |r| {
                Ok(Unnamed { id: r.get(0)?, emb: emb_from(&r.get::<_, Vec<u8>>(1)?), skip: Vec::new(), px: r.get::<_, f64>(2)? as f32, score: r.get::<_, f64>(3)? as f32, frontal: r.get::<_, f64>(4)? as f32 })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let mut rej: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut q = db.prepare("SELECT face_id,person_id FROM person_rejections").map_err(|e| e.to_string())?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())? {
            let (f, p) = row.map_err(|e| e.to_string())?;
            rej.entry(f).or_default().push(p);
        }
        Ok(faces.into_iter().map(|mut u| { u.skip = rej.remove(&u.id).unwrap_or_default(); u }).collect())
    }
    /// The embeddings of each group (made by this model), for comparing groups with each other and with people.
    pub fn group_members(&self, model: &str) -> Result<Vec<(i64, Vec<Vec<f32>>)>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT cluster_id,emb FROM faces WHERE cluster_id IS NOT NULL AND state='unreviewed' AND person_id IS NULL AND model=?1 ORDER BY cluster_id,id")
            .map_err(|e| e.to_string())?;
        let mut out: Vec<(i64, Vec<Vec<f32>>)> = Vec::new();
        for row in q.query_map([model], |r| Ok((r.get::<_, i64>(0)?, emb_from(&r.get::<_, Vec<u8>>(1)?)))).map_err(|e| e.to_string())? {
            let (c, e) = row.map_err(|e| e.to_string())?;
            match out.last_mut() {
                Some(l) if l.0 == c => l.1.push(e),
                _ => out.push((c, vec![e])),
            }
        }
        Ok(out)
    }
    /// The confirmed faces of each active person (made by this model).
    pub fn people_members(&self, model: &str) -> Result<Vec<(i64, Vec<Vec<f32>>)>, String> {
        let mut out: Vec<(i64, Vec<Vec<f32>>)> = Vec::new();
        for (p, _, e) in self.exemplars(model)? {
            match out.iter_mut().find(|x| x.0 == p) {
                Some(l) => l.1.push(e),
                None => out.push((p, vec![e])),
            }
        }
        Ok(out)
    }

    /// Replaces every suggestion and group of the unreviewed faces: `suggestions` is (face, person, cosine),
    /// `groups` the face ids of each cluster with its representative.
    pub fn set_analysis(&self, suggestions: &[(i64, i64, f32)], groups: &[(i64, Vec<i64>)]) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute("UPDATE faces SET suggested_person=NULL,suggested_sim=NULL,cluster_id=NULL WHERE state='unreviewed' AND person_id IS NULL", []).map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM clusters", []).map_err(|e| e.to_string())?;
        for (face, person, sim) in suggestions {
            tx.execute("UPDATE faces SET suggested_person=?2,suggested_sim=?3 WHERE id=?1 AND state='unreviewed' AND person_id IS NULL", params![face, person, f64::from(*sim)])
                .map_err(|e| e.to_string())?;
        }
        for (rep, members) in groups {
            tx.execute("INSERT INTO clusters(rep_face_id,size) VALUES(?1,?2)", params![rep, members.len() as i64]).map_err(|e| e.to_string())?;
            let id = tx.last_insert_rowid();
            for m in members {
                tx.execute("UPDATE faces SET cluster_id=?2 WHERE id=?1", params![m, id]).map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }

    // ---- people and review --------------------------------------------------------

    fn snapshot_faces(db: &Connection, ids: &[i64]) -> Result<Vec<FaceState>, String> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(s) = db
                .query_row("SELECT id,person_id,state,suggested_person,suggested_sim,cluster_id FROM faces WHERE id=?1", [id], |r| {
                    Ok(FaceState { id: r.get(0)?, person_id: r.get(1)?, state: r.get(2)?, suggested_person: r.get(3)?, suggested_sim: r.get(4)?, cluster_id: r.get(5)? })
                })
                .optional()
                .map_err(|e| e.to_string())?
            {
                out.push(s);
            }
        }
        Ok(out)
    }
    fn snapshot_person(db: &Connection, id: i64) -> Result<Option<PersonState>, String> {
        db.query_row("SELECT id,name,state,merged_into,cover_face_id,cover_manual FROM people WHERE id=?1", [id], |r| {
            Ok(PersonState { id: r.get(0)?, name: r.get(1)?, state: r.get(2)?, merged_into: r.get(3)?, cover_face_id: r.get(4)?, cover_manual: r.get(5)? })
        })
        .optional()
        .map_err(|e| e.to_string())
    }
    /// The faces themselves and their twins in a RAW+JPG companion.
    fn with_twins(db: &Connection, ids: &[i64]) -> Result<Vec<i64>, String> {
        let mut all: Vec<i64> = ids.to_vec();
        for id in ids {
            let Some((path, x, y, w, h)) = db
                .query_row("SELECT path,x,y,w,h FROM faces WHERE id=?1", [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?, r.get::<_, f64>(4)?)))
                .optional()
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            // the photos linked to this one: the one it mirrors, and the ones mirroring it
            let mut linked: Vec<String> = db
                .prepare("SELECT companion_of FROM photos WHERE path=?1 AND companion_of IS NOT NULL UNION SELECT path FROM photos WHERE companion_of=?1")
                .and_then(|mut s| s.query_map([&path], |r| r.get(0)).and_then(|m| m.collect()))
                .map_err(|e| e.to_string())?;
            if let Some(primary) = linked.first().cloned() {
                // a mirror of a primary also links its other mirrors
                let siblings: Vec<String> = db
                    .prepare("SELECT path FROM photos WHERE companion_of=?1 AND path<>?2")
                    .and_then(|mut s| s.query_map(params![primary, path], |r| r.get(0)).and_then(|m| m.collect()))
                    .map_err(|e| e.to_string())?;
                linked.extend(siblings);
            }
            for other in linked {
                let mut q = db.prepare("SELECT id,x,y,w,h FROM faces WHERE path=?1").map_err(|e| e.to_string())?;
                let rows = q
                    .query_map([&other], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?, r.get::<_, f64>(4)?)))
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    let (oid, ox, oy, ow, oh) = row.map_err(|e| e.to_string())?;
                    let centre = ((x + w / 2.0) - (ox + ow / 2.0)).hypot((y + h / 2.0) - (oy + oh / 2.0));
                    if centre < 0.05 && (w / ow - 1.0).abs() < 0.25 && !all.contains(&oid) {
                        all.push(oid);
                    }
                }
            }
        }
        Ok(all)
    }
    fn push_undo(&self, u: Undo) {
        let mut stack = self.undo.lock().unwrap();
        stack.push(u);
        if stack.len() > UNDO_DEPTH {
            stack.remove(0);
        }
    }
    pub fn can_undo(&self) -> Option<String> {
        self.undo.lock().unwrap().last().map(|u| u.label.clone())
    }

    fn check_person(db: &Connection, id: i64) -> Result<PersonState, String> {
        match Self::snapshot_person(db, id)? {
            Some(p) if p.state == "active" => Ok(p),
            Some(_) => Err("That person is not active".into()),
            None => Err("No such person".into()),
        }
    }
    fn name_taken(db: &Connection, name: &str, except: Option<i64>) -> Result<bool, String> {
        db.query_row("SELECT 1 FROM people WHERE state='active' AND name=?1 COLLATE NOCASE AND id<>?2", params![name, except.unwrap_or(-1)], |_| Ok(()))
            .optional()
            .map(|r| r.is_some())
            .map_err(|e| e.to_string())
    }

    /// Creates a person, optionally naming faces after them at once.
    pub fn create_person(&self, name: &str, faces: &[i64]) -> Result<i64, String> {
        let name = clean_name(name)?;
        let id = {
            let db = self.db()?;
            if Self::name_taken(&db, &name, None)? {
                return Err(format!("There is already a person called {name}"));
            }
            db.execute("INSERT INTO people(name,created_at) VALUES(?1,?2)", params![name, now()]).map_err(|e| e.to_string())?;
            db.last_insert_rowid()
        };
        let mut undo = Undo { label: format!("Create {name}"), created_person: Some(id), ..Undo::default() };
        if !faces.is_empty() {
            let inner = self.assign_inner(id, faces)?;
            undo.faces = inner.faces;
            undo.rejections_removed = inner.rejections_removed;
        }
        self.push_undo(undo);
        Ok(id)
    }

    pub fn rename_person(&self, id: i64, name: &str) -> Result<(), String> {
        let name = clean_name(name)?;
        let db = self.db()?;
        let before = Self::check_person(&db, id)?;
        if Self::name_taken(&db, &name, Some(id))? {
            return Err(format!("There is already a person called {name}"));
        }
        db.execute("UPDATE people SET name=?2 WHERE id=?1", params![id, name]).map_err(|e| e.to_string())?;
        drop(db);
        self.push_undo(Undo { label: format!("Rename {}", before.name), people: vec![before], ..Undo::default() });
        Ok(())
    }

    fn assign_inner(&self, person: i64, faces: &[i64]) -> Result<Undo, String> {
        let db = self.db()?;
        let target = Self::check_person(&db, person)?;
        let ids = Self::with_twins(&db, faces)?;
        let mut undo = Undo { label: format!("Name as {}", target.name), faces: Self::snapshot_faces(&db, &ids)?, people: vec![target], ..Undo::default() };
        for id in &ids {
            let had: bool = db.query_row("SELECT COUNT(*) FROM person_rejections WHERE face_id=?1 AND person_id=?2", params![id, person], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())? > 0;
            if had {
                undo.rejections_removed.push((*id, person));
            }
            db.execute("DELETE FROM person_rejections WHERE face_id=?1 AND person_id=?2", params![id, person]).map_err(|e| e.to_string())?;
            db.execute("UPDATE faces SET person_id=?2,state='confirmed',suggested_person=NULL,suggested_sim=NULL,cluster_id=NULL WHERE id=?1", params![id, person]).map_err(|e| e.to_string())?;
        }
        Self::refresh_cover(&db, person)?;
        Ok(undo)
    }
    /// Names faces after a person (confirms them), also their twins in a companion file.
    pub fn assign(&self, person: i64, faces: &[i64]) -> Result<(), String> {
        let undo = self.assign_inner(person, faces)?;
        self.push_undo(undo);
        Ok(())
    }

    /// Takes faces off the person they were named or suggested as and makes them unreviewed again.
    pub fn unassign(&self, faces: &[i64]) -> Result<(), String> {
        let db = self.db()?;
        let ids = Self::with_twins(&db, faces)?;
        let mut undo = Undo { label: "Remove from person".into(), faces: Self::snapshot_faces(&db, &ids)?, ..Undo::default() };
        let mut touched = std::collections::BTreeSet::new();
        for s in &undo.faces {
            if let Some(p) = s.person_id {
                touched.insert(p);
            }
        }
        for id in &ids {
            db.execute("UPDATE faces SET person_id=NULL,state='unreviewed',suggested_person=NULL,suggested_sim=NULL,cluster_id=NULL WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        }
        for p in touched {
            if let Some(ps) = Self::snapshot_person(&db, p)? {
                undo.people.push(ps);
            }
            Self::refresh_cover(&db, p)?;
        }
        drop(db);
        self.push_undo(undo);
        Ok(())
    }

    /// "Not this person": the faces leave that person (or its suggestion) and are never suggested for it again.
    pub fn reject_for_person(&self, person: i64, faces: &[i64]) -> Result<(), String> {
        let db = self.db()?;
        let ids = Self::with_twins(&db, faces)?;
        let mut undo = Undo { label: "Not this person".into(), faces: Self::snapshot_faces(&db, &ids)?, people: Self::snapshot_person(&db, person)?.into_iter().collect(), ..Undo::default() };
        for id in &ids {
            let had: bool = db.query_row("SELECT COUNT(*) FROM person_rejections WHERE face_id=?1 AND person_id=?2", params![id, person], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())? > 0;
            if !had {
                undo.rejections_added.push((*id, person));
            }
            db.execute("INSERT OR IGNORE INTO person_rejections(face_id,person_id) VALUES(?1,?2)", params![id, person]).map_err(|e| e.to_string())?;
            db.execute(
                "UPDATE faces SET person_id=NULL,state='unreviewed',cluster_id=NULL WHERE id=?1 AND person_id=?2",
                params![id, person],
            )
            .map_err(|e| e.to_string())?;
            db.execute("UPDATE faces SET suggested_person=NULL,suggested_sim=NULL WHERE id=?1 AND suggested_person=?2", params![id, person]).map_err(|e| e.to_string())?;
        }
        Self::refresh_cover(&db, person)?;
        drop(db);
        self.push_undo(undo);
        Ok(())
    }

    /// "Not a face" (a false detection): hidden everywhere, kept so a rescan does not bring it back.
    pub fn reject_faces(&self, faces: &[i64]) -> Result<(), String> {
        let db = self.db()?;
        let ids = Self::with_twins(&db, faces)?;
        let mut undo = Undo { label: "Not a face".into(), faces: Self::snapshot_faces(&db, &ids)?, ..Undo::default() };
        let mut touched = std::collections::BTreeSet::new();
        for s in &undo.faces {
            if let Some(p) = s.person_id {
                touched.insert(p);
            }
        }
        for id in &ids {
            db.execute("UPDATE faces SET state='rejected',person_id=NULL,suggested_person=NULL,suggested_sim=NULL,cluster_id=NULL WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        }
        for p in touched {
            if let Some(ps) = Self::snapshot_person(&db, p)? {
                undo.people.push(ps);
            }
            Self::refresh_cover(&db, p)?;
        }
        drop(db);
        self.push_undo(undo);
        Ok(())
    }

    /// Joins `from` into `into`: its faces move over and it disappears from the lists.
    pub fn merge(&self, from: i64, into: i64) -> Result<(), String> {
        if from == into {
            return Err("Choose two different people".into());
        }
        let db = self.db()?;
        let (a, b) = (Self::check_person(&db, from)?, Self::check_person(&db, into)?);
        let moved: Vec<i64> = db
            .prepare("SELECT id FROM faces WHERE person_id=?1")
            .and_then(|mut s| s.query_map([from], |r| r.get(0)).and_then(|m| m.collect()))
            .map_err(|e| e.to_string())?;
        let undo = Undo { label: format!("Merge {} into {}", a.name, b.name), faces: Self::snapshot_faces(&db, &moved)?, people: vec![a, b], ..Undo::default() };
        db.execute("UPDATE faces SET person_id=?2 WHERE person_id=?1", params![from, into]).map_err(|e| e.to_string())?;
        db.execute("UPDATE people SET state='merged',merged_into=?2,cover_face_id=NULL WHERE id=?1", params![from, into]).map_err(|e| e.to_string())?;
        db.execute("UPDATE faces SET suggested_person=?2 WHERE suggested_person=?1", params![from, into]).map_err(|e| e.to_string())?;
        // answers about a person who no longer exists go with them
        db.execute("DELETE FROM people_not_same WHERE person_a=?1 OR person_b=?1", [from]).map_err(|e| e.to_string())?;
        Self::refresh_cover(&db, into)?;
        drop(db);
        self.push_undo(undo);
        Ok(())
    }

    /// "Remove person": hides them and keeps their faces out of suggestions and groups; restorable.
    pub fn set_ignored(&self, id: i64, ignored: bool) -> Result<(), String> {
        let db = self.db()?;
        let before = Self::snapshot_person(&db, id)?.ok_or("No such person")?;
        if before.state == "merged" {
            return Err("That person was merged".into());
        }
        if !ignored && Self::name_taken(&db, &before.name, Some(id))? {
            return Err(format!("There is already a person called {}", before.name));
        }
        db.execute("UPDATE people SET state=?2 WHERE id=?1", params![id, if ignored { "ignored" } else { "active" }]).map_err(|e| e.to_string())?;
        if ignored {
            db.execute("UPDATE faces SET suggested_person=NULL,suggested_sim=NULL WHERE suggested_person=?1", [id]).map_err(|e| e.to_string())?;
        }
        let label = format!("{} {}", if ignored { "Remove" } else { "Restore" }, before.name);
        drop(db);
        self.push_undo(Undo { label, people: vec![before], ..Undo::default() });
        Ok(())
    }

    pub fn set_cover(&self, person: i64, face: i64) -> Result<(), String> {
        let db = self.db()?;
        let before = Self::check_person(&db, person)?;
        let ok: bool = db
            .query_row("SELECT COUNT(*) FROM faces WHERE id=?1 AND person_id=?2 AND state='confirmed'", params![face, person], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            > 0;
        if !ok {
            return Err("The cover must be one of the person's faces".into());
        }
        db.execute("UPDATE people SET cover_face_id=?2,cover_manual=1 WHERE id=?1", params![person, face]).map_err(|e| e.to_string())?;
        drop(db);
        self.push_undo(Undo { label: "Set cover".into(), people: vec![before], ..Undo::default() });
        Ok(())
    }

    /// An automatic cover is the person's best face (sharp, frontal, large); a manual one stays while it is theirs.
    fn refresh_cover(db: &Connection, person: i64) -> Result<(), String> {
        let (cover, manual): (Option<i64>, i64) = db.query_row("SELECT cover_face_id,cover_manual FROM people WHERE id=?1", [person], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        if manual != 0 {
            if let Some(c) = cover {
                let still: bool = db.query_row("SELECT COUNT(*) FROM faces WHERE id=?1 AND person_id=?2 AND state='confirmed'", params![c, person], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())? > 0;
                if still {
                    return Ok(());
                }
            }
        }
        let best: Option<i64> = db
            .query_row(
                "SELECT id FROM faces WHERE person_id=?1 AND state='confirmed' ORDER BY (0.5*score + 0.3*frontal + 0.2*MIN(1.0, w*h*20)) DESC, id LIMIT 1",
                [person],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        db.execute("UPDATE people SET cover_face_id=?2,cover_manual=0 WHERE id=?1", params![person, best]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Reverses the latest change; returns its label.
    pub fn undo_last(&self) -> Result<Option<String>, String> {
        let Some(u) = self.undo.lock().unwrap().pop() else { return Ok(None) };
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for f in &u.faces {
            tx.execute(
                "UPDATE faces SET person_id=?2,state=?3,suggested_person=?4,suggested_sim=?5,cluster_id=?6 WHERE id=?1",
                params![f.id, f.person_id, f.state, f.suggested_person, f.suggested_sim, f.cluster_id],
            )
            .map_err(|e| e.to_string())?;
        }
        for (f, p) in &u.rejections_added {
            tx.execute("DELETE FROM person_rejections WHERE face_id=?1 AND person_id=?2", params![f, p]).map_err(|e| e.to_string())?;
        }
        for (f, p) in &u.rejections_removed {
            tx.execute("INSERT OR IGNORE INTO person_rejections(face_id,person_id) VALUES(?1,?2)", params![f, p]).map_err(|e| e.to_string())?;
        }
        for (id, rep, size) in &u.clusters {
            tx.execute("INSERT OR REPLACE INTO clusters(id,rep_face_id,size) VALUES(?1,?2,?3)", params![id, rep, size]).map_err(|e| e.to_string())?;
        }
        for (a, b) in &u.links_added {
            tx.execute("DELETE FROM face_links WHERE face_a=?1 AND face_b=?2", params![a, b]).map_err(|e| e.to_string())?;
        }
        for (a, b) in &u.groups_not_same_added {
            tx.execute("DELETE FROM groups_not_same WHERE face_a=?1 AND face_b=?2", params![a, b]).map_err(|e| e.to_string())?;
        }
        for (a, b) in &u.people_not_same_added {
            tx.execute("DELETE FROM people_not_same WHERE person_a=?1 AND person_b=?2", params![a, b]).map_err(|e| e.to_string())?;
        }
        if let Some(id) = u.created_person {
            tx.execute("DELETE FROM people WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        }
        for p in &u.people {
            tx.execute(
                "UPDATE people SET name=?2,state=?3,merged_into=?4,cover_face_id=?5,cover_manual=?6 WHERE id=?1",
                params![p.id, p.name, p.state, p.merged_into, p.cover_face_id, p.cover_manual],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some(u.label))
    }

    // ---- groups and pairs ------------------------------------------------------------

    /// The group's best faces (its representative first), to remember it by.
    fn group_keys(db: &Connection, group: i64) -> Result<Vec<i64>, String> {
        db.prepare(
            "SELECT f.id FROM faces f JOIN clusters c ON c.id=f.cluster_id WHERE f.cluster_id=?1 AND f.state='unreviewed' AND f.person_id IS NULL
             ORDER BY (f.id=c.rep_face_id) DESC, f.score DESC, f.id LIMIT 3",
        )
        .and_then(|mut s| s.query_map([group], |r| r.get(0)).and_then(|m| m.collect()))
        .map_err(|e| e.to_string())
    }
    fn cluster_row(db: &Connection, group: i64) -> Result<Option<(i64, i64, i64)>, String> {
        db.query_row("SELECT id,rep_face_id,size FROM clusters WHERE id=?1", [group], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(|e| e.to_string())
    }

    /// Joins two unnamed groups without naming them. The decision is remembered (links between their best faces), so
    /// the next analysis keeps them together; undo splits them again.
    pub fn merge_groups(&self, from: i64, into: i64) -> Result<(), String> {
        if from == into {
            return Err("Choose two different groups".into());
        }
        let db = self.db()?;
        let (a, b) = (Self::cluster_row(&db, from)?.ok_or("No such group")?, Self::cluster_row(&db, into)?.ok_or("No such group")?);
        let (ka, kb) = (Self::group_keys(&db, from)?, Self::group_keys(&db, into)?);
        let moved: Vec<i64> = db
            .prepare("SELECT id FROM faces WHERE cluster_id=?1 OR cluster_id=?2")
            .and_then(|mut s| s.query_map(params![from, into], |r| r.get(0)).and_then(|m| m.collect()))
            .map_err(|e| e.to_string())?;
        let mut undo = Undo { label: "Merge groups".into(), faces: Self::snapshot_faces(&db, &moved)?, clusters: vec![a, b], ..Undo::default() };
        for (x, y) in ka.iter().zip(&kb) {
            let (lo, hi) = (*x.min(y), *x.max(y));
            let had = db.execute("INSERT OR IGNORE INTO face_links(face_a,face_b) VALUES(?1,?2)", params![lo, hi]).map_err(|e| e.to_string())? > 0;
            if had {
                undo.links_added.push((lo, hi));
            }
        }
        db.execute("UPDATE faces SET cluster_id=?2 WHERE cluster_id=?1", params![from, into]).map_err(|e| e.to_string())?;
        db.execute("DELETE FROM clusters WHERE id=?1", [from]).map_err(|e| e.to_string())?;
        db.execute("UPDATE clusters SET size=(SELECT COUNT(*) FROM faces WHERE cluster_id=?1) WHERE id=?1", [into]).map_err(|e| e.to_string())?;
        drop(db);
        self.push_undo(undo);
        Ok(())
    }

    /// "Not the same" for two groups, or two people: remembered, so neither a suggestion nor the automatic merge joins them.
    pub fn not_same(&self, a: Side, b: Side) -> Result<(), String> {
        let db = self.db()?;
        let mut undo = Undo { label: "Not the same".into(), ..Undo::default() };
        match (a, b) {
            (Side::Group { id: x }, Side::Group { id: y }) => {
                if x == y {
                    return Err("Choose two different groups".into());
                }
                Self::cluster_row(&db, x)?.ok_or("No such group")?;
                Self::cluster_row(&db, y)?.ok_or("No such group")?;
                let (kx, ky) = (Self::group_keys(&db, x)?, Self::group_keys(&db, y)?);
                for f in &kx {
                    for g in &ky {
                        let (lo, hi) = (*f.min(g), *f.max(g));
                        if db.execute("INSERT OR IGNORE INTO groups_not_same(face_a,face_b) VALUES(?1,?2)", params![lo, hi]).map_err(|e| e.to_string())? > 0 {
                            undo.groups_not_same_added.push((lo, hi));
                        }
                    }
                }
            }
            (Side::Person { id: x }, Side::Person { id: y }) => {
                if x == y {
                    return Err("Choose two different people".into());
                }
                Self::check_person(&db, x)?;
                Self::check_person(&db, y)?;
                let (lo, hi) = (x.min(y), x.max(y));
                if db.execute("INSERT OR IGNORE INTO people_not_same(person_a,person_b) VALUES(?1,?2)", params![lo, hi]).map_err(|e| e.to_string())? > 0 {
                    undo.people_not_same_added.push((lo, hi));
                }
            }
            (Side::Group { id: g }, Side::Person { id: p }) | (Side::Person { id: p }, Side::Group { id: g }) => {
                // a group that is not them: every face of it is "not this person"
                Self::check_person(&db, p)?;
                let faces: Vec<i64> = db
                    .prepare("SELECT id FROM faces WHERE cluster_id=?1 AND state='unreviewed' AND person_id IS NULL")
                    .and_then(|mut s| s.query_map([g], |r| r.get(0)).and_then(|m| m.collect()))
                    .map_err(|e| e.to_string())?;
                drop(db);
                return self.reject_for_person(p, &faces);
            }
        }
        drop(db);
        self.push_undo(undo);
        Ok(())
    }

    /// The pairs of faces the user joined: the analysis keeps their groups together.
    pub fn face_links(&self) -> Result<Vec<(i64, i64)>, String> {
        let db = self.db()?;
        let mut q = db.prepare("SELECT face_a,face_b FROM face_links").map_err(|e| e.to_string())?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
    /// The pairs of faces whose groups the user said are not the same person.
    pub fn groups_not_same(&self) -> Result<Vec<(i64, i64)>, String> {
        let db = self.db()?;
        let mut q = db.prepare("SELECT face_a,face_b FROM groups_not_same").map_err(|e| e.to_string())?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
    /// What the suggestions must not offer, in current group and person ids.
    pub fn suggestion_skips(&self) -> Result<Skips, String> {
        let db = self.db()?;
        let mut skips = Skips::default();
        let owner = |db: &Connection| -> Result<HashMap<i64, i64>, String> {
            let mut q = db.prepare("SELECT id,cluster_id FROM faces WHERE cluster_id IS NOT NULL AND state='unreviewed' AND person_id IS NULL").map_err(|e| e.to_string())?;
            let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())?;
            rows.collect::<Result<HashMap<_, _>, _>>().map_err(|e| e.to_string())
        };
        let of = owner(&db)?;
        let mut q = db.prepare("SELECT face_a,face_b FROM groups_not_same").map_err(|e| e.to_string())?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())? {
            let (a, b) = row.map_err(|e| e.to_string())?;
            if let (Some(x), Some(y)) = (of.get(&a), of.get(&b)) {
                skips.groups.insert((*x.min(y), *x.max(y)));
            }
        }
        let mut q = db.prepare("SELECT person_a,person_b FROM people_not_same").map_err(|e| e.to_string())?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())? {
            skips.people.insert(row.map_err(|e| e.to_string())?);
        }
        let mut q = db.prepare("SELECT face_id,person_id FROM person_rejections").map_err(|e| e.to_string())?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(|e| e.to_string())? {
            let (f, p) = row.map_err(|e| e.to_string())?;
            if let Some(g) = of.get(&f) {
                skips.group_person.insert((*g, p));
            }
        }
        Ok(skips)
    }

    // ---- privacy ------------------------------------------------------------------

    /// Every face, person, group and scan record: gone, and the file shrunk.
    pub fn delete_all(&self) -> Result<(), String> {
        let db = self.db()?;
        db.execute_batch("DELETE FROM person_rejections; DELETE FROM groups_not_same; DELETE FROM face_links; DELETE FROM people_not_same; DELETE FROM clusters; DELETE FROM faces; DELETE FROM people; DELETE FROM photos; DELETE FROM meta;")
            .map_err(|e| e.to_string())?;
        let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;");
        self.undo.lock().unwrap().clear();
        Ok(())
    }
    /// Scan records and faces only, keeping the people (a re-scan after a model change).
    pub fn clear_index(&self) -> Result<(), String> {
        let db = self.db()?;
        db.execute_batch("DELETE FROM person_rejections; DELETE FROM groups_not_same; DELETE FROM face_links; DELETE FROM clusters; DELETE FROM faces; DELETE FROM photos; UPDATE people SET cover_face_id=NULL;")
            .map_err(|e| e.to_string())?;
        self.undo.lock().unwrap().clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> FaceStore {
        FaceStore::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn nf(x: f32, e0: f32) -> NewFace {
        let mut emb = vec![0.0; 128];
        emb[0] = e0;
        emb[1] = (1.0 - e0 * e0).max(0.0).sqrt();
        NewFace { x, y: 0.2, w: 0.2, h: 0.25, score: 0.9, lm: [0.0; 10], frontal: 0.8, roll: 1.0, aligned: true, emb }
    }
    fn scan(s: &FaceStore, path: &str, faces: &[NewFace]) -> Vec<i64> {
        s.put_scan(path, "fp", "m1", 1000, 800, faces, None).unwrap()
    }

    #[test]
    fn the_schema_is_created_once_and_a_newer_one_is_refused() {
        let s = store();
        assert_eq!(s.schema_version().unwrap(), 3);
        // a database of the first version gains the pair tables and keeps its data
        let old = Connection::open_in_memory().unwrap();
        old.execute_batch(&format!("BEGIN; {} PRAGMA user_version = 1; COMMIT;", LADDER[0])).unwrap();
        old.execute("INSERT INTO people(name,created_at) VALUES('Ann',1)", []).unwrap();
        let migrated = FaceStore::from_connection(old).unwrap();
        assert_eq!(migrated.schema_version().unwrap(), 3);
        assert_eq!(migrated.list_people(false).unwrap().len(), 1);
        assert!(migrated.face_links().unwrap().is_empty());
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 99).unwrap();
        assert!(FaceStore::from_connection(db).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("faces.sqlite");
        {
            let s = FaceStore::open(&path).unwrap();
            scan(&s, "/a.jpg", &[nf(0.1, 1.0)]);
        }
        let s = FaceStore::open(&path).unwrap();
        assert_eq!(s.totals().unwrap().faces, 1, "reopening keeps the data and does not migrate again");
    }

    #[test]
    fn a_scan_replaces_the_photo_and_is_current_only_for_its_model_and_file() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.0)]);
        assert_eq!(ids.len(), 2);
        assert!(s.is_current("/a.jpg", "fp", "m1").unwrap());
        assert!(!s.is_current("/a.jpg", "other", "m1").unwrap() && !s.is_current("/a.jpg", "fp", "m2").unwrap());
        scan(&s, "/a.jpg", &[nf(0.1, 1.0)]);
        assert_eq!(s.faces_of_photo("/a.jpg").unwrap().len(), 1);
        assert_eq!(s.embedding(s.faces_of_photo("/a.jpg").unwrap()[0].id).unwrap().unwrap().len(), 128);
    }

    #[test]
    fn the_bounding_box_check_rejects_nonsense() {
        let s = store();
        let mut bad = nf(0.9, 1.0);
        bad.w = 0.3; // runs off the picture
        assert!(s.put_scan("/a.jpg", "fp", "m1", 10, 10, &[bad], None).is_err());
        assert_eq!(s.totals().unwrap().photos, 0, "the failed scan left nothing behind");
    }

    #[test]
    fn people_are_created_named_uniquely_and_renamed() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let ann = s.create_person("  Ann   Lee ", &[ids[0]]).unwrap();
        assert_eq!(s.list_people(false).unwrap()[0].name, "Ann Lee");
        assert_eq!(s.list_people(false).unwrap()[0].faces, 1);
        assert!(s.create_person("ann lee", &[]).is_err(), "names are unique, ignoring case");
        assert!(s.create_person("   ", &[]).is_err());
        assert!(s.create_person(&"x".repeat(81), &[]).is_err());
        let bob = s.create_person("Bob", &[ids[1]]).unwrap();
        assert!(s.rename_person(bob, "ANN LEE").is_err());
        s.rename_person(bob, "Robert").unwrap();
        assert_eq!(s.list_people(false).unwrap().iter().map(|p| p.name.clone()).collect::<Vec<_>>(), ["Ann Lee", "Robert"]);
        assert_ne!(ann, bob);
    }

    #[test]
    fn assigning_confirms_sets_an_automatic_cover_and_undo_restores() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let p = s.create_person("Ann", &[]).unwrap();
        s.assign(p, &ids).unwrap();
        let people = s.list_people(false).unwrap();
        assert_eq!((people[0].faces, people[0].photos), (2, 1));
        assert!(people[0].cover_face_id.is_some() && !people[0].cover_manual);
        assert_eq!(s.face(ids[0]).unwrap().unwrap().state, "confirmed");
        assert_eq!(s.can_undo().as_deref(), Some("Name as Ann"));
        s.undo_last().unwrap();
        let f = s.face(ids[0]).unwrap().unwrap();
        assert_eq!((f.person_id, f.state.as_str()), (None, "unreviewed"));
        s.undo_last().unwrap(); // the creation
        assert!(s.list_people(false).unwrap().is_empty());
        assert_eq!(s.undo_last().unwrap(), None);
    }

    #[test]
    fn a_manual_cover_stays_while_the_face_is_theirs() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let p = s.create_person("Ann", &ids).unwrap();
        s.set_cover(p, ids[1]).unwrap();
        assert!(s.list_people(false).unwrap()[0].cover_manual);
        s.assign(p, &[ids[0]]).unwrap();
        assert_eq!(s.list_people(false).unwrap()[0].cover_face_id, Some(ids[1]));
        s.unassign(&[ids[1]]).unwrap();
        let after = &s.list_people(false).unwrap()[0];
        assert_eq!((after.cover_face_id, after.cover_manual), (Some(ids[0]), false), "the cover fell back to a face they still have");
        assert!(s.set_cover(p, ids[1]).is_err(), "not their face any more");
    }

    #[test]
    fn merge_moves_faces_hides_the_source_and_undo_brings_it_back() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let a = s.create_person("Ann", &[ids[0]]).unwrap();
        let b = s.create_person("Anne", &[ids[1]]).unwrap();
        assert!(s.merge(a, a).is_err());
        s.merge(b, a).unwrap();
        let people = s.list_people(false).unwrap();
        assert_eq!((people.len(), people[0].faces), (1, 2));
        assert!(s.merge(b, a).is_err(), "a merged person cannot be merged again");
        assert_eq!(s.face(ids[1]).unwrap().unwrap().person_id, Some(a));
        s.undo_last().unwrap();
        assert_eq!(s.list_people(false).unwrap().len(), 2);
        assert_eq!(s.face(ids[1]).unwrap().unwrap().person_id, Some(b));
    }

    #[test]
    fn ignored_people_leave_the_lists_and_suggestions_and_can_be_restored() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let a = s.create_person("Ann", &[ids[0]]).unwrap();
        s.set_analysis(&[(ids[1], a, 0.6)], &[]).unwrap();
        assert_eq!(s.list_people(false).unwrap()[0].suggestions, 1);
        s.set_ignored(a, true).unwrap();
        assert!(s.list_people(false).unwrap().is_empty());
        assert_eq!(s.list_people(true).unwrap().len(), 1);
        assert!(s.exemplars("m1").unwrap().is_empty(), "an ignored person is nobody's exemplar");
        assert!(s.face(ids[1]).unwrap().unwrap().suggested_person.is_none());
        let again = s.create_person("Ann", &[]).unwrap(); // the name is free while they are removed
        assert!(s.set_ignored(a, false).is_err(), "restoring would duplicate a name");
        s.rename_person(again, "Ann 2").unwrap();
        s.set_ignored(a, false).unwrap();
        assert_eq!(s.list_people(false).unwrap().len(), 2);
    }

    #[test]
    fn rejecting_for_a_person_keeps_the_face_out_of_their_suggestions() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let a = s.create_person("Ann", &[ids[0]]).unwrap();
        s.assign(a, &[ids[1]]).unwrap();
        s.reject_for_person(a, &[ids[1]]).unwrap();
        let f = s.face(ids[1]).unwrap().unwrap();
        assert_eq!((f.person_id, f.state.as_str()), (None, "unreviewed"));
        let unnamed = s.unnamed("m1").unwrap();
        assert_eq!(unnamed.iter().find(|u| u.id == ids[1]).unwrap().skip, vec![a], "the analysis is told who it must not suggest");
        s.undo_last().unwrap();
        assert!(s.unnamed("m1").unwrap().iter().all(|u| u.id != ids[1]));
        assert_eq!(s.face(ids[1]).unwrap().unwrap().person_id, Some(a));
        // naming the face after them again clears the rejection
        s.reject_for_person(a, &[ids[1]]).unwrap();
        s.assign(a, &[ids[1]]).unwrap();
        s.unassign(&[ids[1]]).unwrap();
        assert!(s.unnamed("m1").unwrap().iter().find(|u| u.id == ids[1]).unwrap().skip.is_empty());
    }

    #[test]
    fn not_a_face_hides_it_and_survives_a_rescan() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        s.reject_faces(&[ids[0]]).unwrap();
        assert_eq!(s.faces_of_photo("/a.jpg").unwrap().len(), 1);
        scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let faces = s.faces_of_photo("/a.jpg").unwrap();
        assert_eq!(faces.len(), 1, "the same box is still rejected after the photo is scanned again");
        assert!(s.unnamed("m1").unwrap().len() == 1);
    }

    #[test]
    fn a_rescan_keeps_the_review_of_faces_that_are_still_there() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let a = s.create_person("Ann", &[ids[0]]).unwrap();
        s.reject_for_person(a, &[ids[1]]).unwrap();
        // the detector moves the boxes a little and finds a third face
        let mut moved = nf(0.11, 1.0);
        moved.w = 0.19;
        let again = s.put_scan("/a.jpg", "fp2", "m2", 1000, 800, &[moved, nf(0.5, 0.2), nf(0.8, 0.5)], None).unwrap();
        assert_eq!(s.face(again[0]).unwrap().unwrap().person_id, Some(a));
        assert_eq!(s.face(again[0]).unwrap().unwrap().state, "confirmed");
        assert_eq!(s.face(again[2]).unwrap().unwrap().state, "unreviewed");
        assert_eq!(s.list_people(false).unwrap()[0].faces, 1);
    }

    #[test]
    fn analysis_results_are_listed_by_group_suggestion_and_unknown() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.3, 0.9), nf(0.5, 0.2), nf(0.7, 0.1)]);
        let a = s.create_person("Ann", &[ids[0]]).unwrap();
        s.set_analysis(&[(ids[3], a, 0.52)], &[(ids[1], vec![ids[1], ids[2]])]).unwrap();
        let t = s.totals().unwrap();
        assert_eq!((t.faces, t.people, t.groups, t.suggestions, t.unreviewed), (4, 1, 1, 1, 3));
        let g = s.list_clusters().unwrap();
        assert_eq!((g[0].size, g[0].rep_face_id), (2, ids[1]));
        assert_eq!(s.list_faces(&FaceFilter::Cluster { id: g[0].id }, 0, 10).unwrap().len(), 2);
        assert_eq!(s.list_faces(&FaceFilter::Suggested { id: a }, 0, 10).unwrap()[0].id, ids[3]);
        assert!(s.list_faces(&FaceFilter::Unknown, 0, 10).unwrap().is_empty());
        assert_eq!(s.list_faces(&FaceFilter::Person { id: a }, 0, 10).unwrap().len(), 1);
        // confirming a grouped face takes it out of the group listing
        s.assign(a, &[ids[1]]).unwrap();
        assert_eq!(s.list_faces(&FaceFilter::Cluster { id: g[0].id }, 0, 10).unwrap().len(), 1);
        // a new analysis replaces the old one and leaves the named alone
        s.set_analysis(&[], &[]).unwrap();
        assert_eq!(s.totals().unwrap().groups, 0);
        assert_eq!(s.face(ids[1]).unwrap().unwrap().person_id, Some(a));
        assert_eq!(s.list_faces(&FaceFilter::Unknown, 0, 10).unwrap().len(), 2);
    }

    #[test]
    fn raw_and_jpg_companions_share_faces_and_review_decisions() {
        let s = store();
        assert_eq!(s.companion("/d/IMG_1.jpg", 4000, 3000, "m1").unwrap(), None);
        let a = scan(&s, "/d/IMG_1.cr3", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        assert_eq!(s.companion("/d/IMG_1.jpg", 1250, 1000, "m1").unwrap().as_deref(), Some("/d/IMG_1.cr3"));
        assert_eq!(s.companion("/d/IMG_1.jpg", 1200, 1600, "m1").unwrap(), None, "another shape is another picture");
        assert_eq!(s.companion("/d/IMG_10.jpg", 1250, 1000, "m1").unwrap(), None, "another stem");
        let b = s.put_scan("/d/IMG_1.jpg", "fp", "m1", 1250, 1000, &[nf(0.1, 1.0), nf(0.5, 0.2)], Some("/d/IMG_1.cr3")).unwrap();
        let p = s.create_person("Ann", &[b[0]]).unwrap();
        assert_eq!(s.face(a[0]).unwrap().unwrap().person_id, Some(p), "naming one names its twin");
        // a mirror scanned later starts with the primary's decisions
        let copy = s.new_faces_of("/d/IMG_1.cr3").unwrap();
        assert_eq!(copy.len(), 2);
        let c = s.put_scan("/d/IMG_1.tif", "fp", "m1", 1250, 1000, &copy, Some("/d/IMG_1.cr3")).unwrap();
        assert_eq!(s.face(c[0]).unwrap().unwrap().person_id, Some(p));
        assert_eq!(s.face(c[1]).unwrap().unwrap().state, "unreviewed");
        assert_eq!(s.face(a[1]).unwrap().unwrap().person_id, None);
        s.reject_faces(&[a[1]]).unwrap();
        assert_eq!(s.face(b[1]).unwrap().unwrap().state, "rejected");
        assert_eq!(s.face(c[1]).unwrap().unwrap().state, "rejected", "every mirror of the primary follows");
        assert_eq!(s.list_people(false).unwrap()[0].faces, 3);
    }

    #[test]
    fn a_failed_photo_is_remembered_without_faces() {
        let s = store();
        scan(&s, "/a.jpg", &[nf(0.1, 1.0)]);
        s.put_failure("/a.jpg", "fp", "m1", "failed", "cannot decode").unwrap();
        assert!(s.is_current("/a.jpg", "fp", "m1").unwrap());
        assert_eq!(s.totals().unwrap().faces, 0);
        s.forget_photo("/a.jpg").unwrap();
        assert!(!s.is_current("/a.jpg", "fp", "m1").unwrap());
    }

    #[test]
    fn counts_ignored_faces_collage_members_and_failed_photos() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.05, 1.0), nf(0.2, 0.9), nf(0.35, 0.8), nf(0.5, 0.7), nf(0.65, 0.6), nf(0.8, 0.1)]);
        s.set_analysis(&[], &[(ids[2], ids[..5].to_vec())]).unwrap();
        let g = s.list_clusters().unwrap();
        assert_eq!(g[0].members.len(), 4);
        assert_eq!(g[0].members[0], ids[2], "the representative comes first");
        assert_eq!(g[0].size, 5);
        let t = s.totals().unwrap();
        assert_eq!((t.unknown, t.ignored_faces, t.ignored_people, t.failed_photos), (1, 0, 0, 0));
        s.reject_faces(&[ids[5]]).unwrap();
        let p = s.create_person("Ann", &[ids[0]]).unwrap();
        s.set_ignored(p, true).unwrap();
        s.put_failure("/bad.jpg", "fp", "m1", "failed", "cannot decode").unwrap();
        let t = s.totals().unwrap();
        assert_eq!((t.unknown, t.ignored_faces, t.ignored_people, t.failed_photos), (0, 1, 1, 1));
        assert_eq!(s.failed_photos(10).unwrap(), ["/bad.jpg"]);
        assert_eq!(s.list_faces(&FaceFilter::Ignored, 0, 10).unwrap()[0].id, ids[5]);
        assert_eq!(s.list_faces(&FaceFilter::Person { id: p }, 0, 10).unwrap().len(), 1, "a removed person's faces stay listable");
        s.unignore_faces(&[ids[5]]).unwrap();
        assert!(s.list_faces(&FaceFilter::Ignored, 0, 10).unwrap().is_empty());
        assert_eq!(s.face(ids[5]).unwrap().unwrap().state, "unreviewed");
        s.undo_last().unwrap();
        assert_eq!(s.face(ids[5]).unwrap().unwrap().state, "rejected");
    }

    #[test]
    fn unnamed_faces_carry_quality_and_only_this_models_faces_are_compared() {
        let s = store();
        // picture 1000 x 800: a face of w 0.2, h 0.25 is max(200, 200) = 200 px
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        s.put_scan("/b.jpg", "fp", "m2", 1000, 800, &[nf(0.1, 0.5)], None).unwrap();
        let u = s.unnamed("m1").unwrap();
        assert_eq!(u.len(), 2);
        assert!((u[0].px - 200.0).abs() < 0.5 && (u[0].score - 0.9).abs() < 1e-6 && (u[0].frontal - 0.8).abs() < 1e-6);
        assert_eq!(s.unnamed("m2").unwrap().len(), 1);
        let p = s.create_person("Ann", &[ids[0]]).unwrap();
        assert_eq!(s.exemplars("m2").unwrap().len(), 0, "another model's faces are not exemplars");
        s.set_analysis(&[], &[(ids[1], vec![ids[1]])]).unwrap();
        assert_eq!(s.group_members("m1").unwrap().len(), 1);
        assert_eq!(s.people_members("m1").unwrap()[0].0, p);
    }

    fn grouped(s: &FaceStore) -> (Vec<i64>, i64, i64) {
        let ids = scan(s, "/a.jpg", &[nf(0.05, 1.0), nf(0.2, 0.9), nf(0.35, 0.8), nf(0.5, 0.7), nf(0.65, 0.6), nf(0.8, 0.1)]);
        s.set_analysis(&[], &[(ids[0], ids[..2].to_vec()), (ids[2], ids[2..5].to_vec())]).unwrap();
        let of = |f: i64| s.face(f).unwrap().unwrap().cluster_id.unwrap();
        let (a, b) = (of(ids[0]), of(ids[2]));
        (ids, a, b)
    }

    #[test]
    fn two_groups_join_without_a_name_and_undo_splits_them_again() {
        let s = store();
        let (ids, a, b) = grouped(&s);
        assert!(s.merge_groups(a, a).is_err() && s.merge_groups(a, 999).is_err());
        s.merge_groups(a, b).unwrap();
        let g = s.list_clusters().unwrap();
        assert_eq!((g.len(), g[0].size, g[0].id), (1, 5, b));
        assert_eq!(s.list_faces(&FaceFilter::Cluster { id: b }, 0, 10).unwrap().len(), 5);
        assert!(s.face_links().unwrap().len() >= 1, "the join is remembered for the next analysis");
        assert_eq!(s.can_undo().as_deref(), Some("Merge groups"));
        s.undo_last().unwrap();
        let g = s.list_clusters().unwrap();
        assert_eq!(g.iter().map(|c| c.size).collect::<Vec<_>>(), [3, 2]);
        assert_eq!(s.face(ids[0]).unwrap().unwrap().cluster_id, Some(a));
        assert!(s.face_links().unwrap().is_empty());
    }

    #[test]
    fn not_the_same_is_remembered_for_groups_and_people_and_cleaned_up() {
        let s = store();
        let (ids, a, b) = grouped(&s);
        assert!(s.not_same(Side::Group { id: a }, Side::Group { id: a }).is_err());
        s.not_same(Side::Group { id: a }, Side::Group { id: b }).unwrap();
        assert!(!s.groups_not_same().unwrap().is_empty());
        let skips = s.suggestion_skips().unwrap();
        assert!(skips.groups.contains(&(a.min(b), a.max(b))));
        // groups are rebuilt with new ids: the pair is still known through its faces
        s.set_analysis(&[], &[(ids[1], ids[..2].to_vec()), (ids[3], ids[2..5].to_vec())]).unwrap();
        let (a2, b2) = (s.face(ids[0]).unwrap().unwrap().cluster_id.unwrap(), s.face(ids[2]).unwrap().unwrap().cluster_id.unwrap());
        assert!(s.suggestion_skips().unwrap().groups.contains(&(a2.min(b2), a2.max(b2))));
        // undo takes it back
        s.undo_last().unwrap();
        assert!(s.groups_not_same().unwrap().is_empty() && s.suggestion_skips().unwrap().groups.is_empty());
        // people
        let p = s.create_person("Ann", &[ids[0]]).unwrap();
        let q = s.create_person("Anne", &[ids[1]]).unwrap();
        s.not_same(Side::Person { id: p }, Side::Person { id: q }).unwrap();
        assert!(s.suggestion_skips().unwrap().people.contains(&(p.min(q), p.max(q))));
        // merging a person away cleans their pairs
        s.merge(q, p).unwrap();
        assert!(s.suggestion_skips().unwrap().people.is_empty());
        // a group marked not them is never offered to that person
        s.set_analysis(&[], &[(ids[3], ids[2..5].to_vec())]).unwrap();
        let g = s.face(ids[3]).unwrap().unwrap().cluster_id.unwrap();
        s.not_same(Side::Group { id: g }, Side::Person { id: p }).unwrap();
        assert!(s.suggestion_skips().unwrap().group_person.contains(&(g, p)));
        // a rescan of the photo drops the faces and so the group answers
        s.not_same(Side::Group { id: g }, Side::Group { id: g + 100 }).unwrap_err();
        s.delete_all().unwrap();
        assert!(s.groups_not_same().unwrap().is_empty() && s.face_links().unwrap().is_empty() && s.suggestion_skips().unwrap().people.is_empty());
    }

    #[test]
    fn a_rescan_that_replaces_the_faces_removes_their_pair_answers() {
        let s = store();
        let (_, a, b) = grouped(&s);
        s.not_same(Side::Group { id: a }, Side::Group { id: b }).unwrap();
        s.merge_groups(a, b).unwrap();
        assert!(!s.groups_not_same().unwrap().is_empty() && !s.face_links().unwrap().is_empty());
        scan(&s, "/a.jpg", &[nf(0.4, 0.5)]);
        assert!(s.groups_not_same().unwrap().is_empty() && s.face_links().unwrap().is_empty());
    }

    #[test]
    fn a_persons_photos_list_confirmed_first_with_a_total_and_pages() {
        let s = store();
        let a = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]);
        let b = scan(&s, "/b.jpg", &[nf(0.1, 0.9)]);
        let c = scan(&s, "/c.jpg", &[nf(0.1, 0.8)]);
        scan(&s, "/d.jpg", &[nf(0.1, 0.1)]);
        let p = s.create_person("Ann", &[c[0]]).unwrap();
        s.set_analysis(&[(a[0], p, 0.6), (b[0], p, 0.5)], &[]).unwrap();
        let all = s.person_photos(p, 0, 10).unwrap();
        assert_eq!((all.total, all.paths.clone()), (3, vec!["/c.jpg".to_string(), "/a.jpg".into(), "/b.jpg".into()]), "confirmed first, then suggested, by path");
        let page = s.person_photos(p, 1, 1).unwrap();
        assert_eq!((page.total, page.paths), (3, vec!["/a.jpg".to_string()]));
        s.reject_for_person(p, &[a[0]]).unwrap();
        assert_eq!(s.person_photos(p, 0, 10).unwrap().total, 2);
        assert_eq!(s.person_photos(999, 0, 10).unwrap(), PersonPhotos { paths: vec![], total: 0 });
    }

    #[test]
    fn a_photos_faces_carry_name_state_and_confidence_and_read_fast() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.6, 1.0), nf(0.1, 0.9), nf(0.35, 0.5), nf(0.8, 0.3)]);
        let p = s.create_person("Ann", &[ids[1]]).unwrap();
        let q = s.create_person("Bob", &[]).unwrap();
        s.set_analysis(&[(ids[2], q, 0.5)], &[]).unwrap();
        s.reject_faces(&[ids[3]]).unwrap();
        let faces = s.photo_faces("/a.jpg").unwrap();
        let by: Vec<(String, Option<String>)> = faces.iter().map(|f| (f.state.clone(), f.name.clone())).collect();
        assert_eq!(faces.iter().map(|f| f.face).collect::<Vec<_>>(), [ids[1], ids[2], ids[0], ids[3]], "left to right");
        assert_eq!(by, [("confirmed".into(), Some("Ann".into())), ("suggested".into(), Some("Bob".into())), ("unassigned".into(), None), ("rejected".into(), None)]);
        assert_eq!((faces[0].person, faces[1].person), (Some(p), Some(q)));
        assert!((faces[0].detector_confidence - 0.9).abs() < 1e-6 && faces[0].eyes.is_none());
        assert!(s.photo_faces("/none.jpg").unwrap().is_empty());
        // a library of 2000 photos: one photo's faces come back in well under 5 ms
        for i in 0..2000 {
            scan(&s, &format!("/lib/{i}.jpg"), &[nf(0.1, 1.0), nf(0.5, 0.2), nf(0.8, 0.1)]);
        }
        let started = std::time::Instant::now();
        for i in 0..200 {
            assert_eq!(s.photo_faces(&format!("/lib/{}.jpg", i * 9)).unwrap().len(), 3);
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / 200.0;
        assert!(each < 5.0, "{each} ms per photo");
    }

    #[test]
    fn the_people_filter_includes_excludes_and_finds_photos_without_or_with_unnamed_faces() {
        use crate::search_filters::PeopleFilter;
        let s = store();
        let a = scan(&s, "/a.jpg", &[nf(0.1, 1.0), nf(0.5, 0.2)]); // Ann and Bob
        let b = scan(&s, "/b.jpg", &[nf(0.1, 0.9)]); // Ann
        let c = scan(&s, "/c.jpg", &[nf(0.1, 0.8), nf(0.5, 0.4)]); // Bob and an unnamed face
        scan(&s, "/d.jpg", &[]); // no faces
        scan(&s, "/e.jpg", &[nf(0.1, 0.7)]); // an unnamed face only
        let ann = s.create_person("Ann", &[a[0], b[0]]).unwrap();
        let bob = s.create_person("Bob", &[a[1], c[0]]).unwrap();
        let ok = |f: PeopleFilter| -> Vec<&'static str> {
            let sets = s.filter_paths(&f).unwrap();
            ["/a.jpg", "/b.jpg", "/c.jpg", "/d.jpg", "/e.jpg"].into_iter().filter(|p| sets.passes(p)).collect()
        };
        assert_eq!(ok(PeopleFilter::default()), ["/a.jpg", "/b.jpg", "/c.jpg", "/d.jpg", "/e.jpg"], "an empty filter allows everything");
        assert_eq!(ok(PeopleFilter { include: vec![ann], ..Default::default() }), ["/a.jpg", "/b.jpg"]);
        assert_eq!(ok(PeopleFilter { include: vec![ann, bob], ..Default::default() }), ["/a.jpg", "/b.jpg", "/c.jpg"], "any of them");
        assert_eq!(ok(PeopleFilter { include: vec![ann, bob], together: true, ..Default::default() }), ["/a.jpg"], "all together");
        assert_eq!(ok(PeopleFilter { exclude: vec![ann], ..Default::default() }), ["/c.jpg", "/d.jpg", "/e.jpg"]);
        assert_eq!(ok(PeopleFilter { include: vec![bob], exclude: vec![ann], ..Default::default() }), ["/c.jpg"]);
        assert_eq!(ok(PeopleFilter { none: true, ..Default::default() }), ["/d.jpg"]);
        assert_eq!(ok(PeopleFilter { unnamed: true, ..Default::default() }), ["/c.jpg", "/e.jpg"]);
        assert_eq!(ok(PeopleFilter { none: true, unnamed: true, ..Default::default() }), Vec::<&str>::new());
        // a merged person is who they became; a removed or unknown one is ignored
        s.merge(bob, ann).unwrap();
        assert_eq!(ok(PeopleFilter { include: vec![bob], ..Default::default() }), ["/a.jpg", "/b.jpg", "/c.jpg"]);
        assert_eq!(ok(PeopleFilter { include: vec![999], ..Default::default() }), ["/a.jpg", "/b.jpg", "/c.jpg", "/d.jpg", "/e.jpg"]);
        s.set_ignored(ann, true).unwrap();
        assert_eq!(ok(PeopleFilter { include: vec![ann], ..Default::default() }).len(), 5, "a removed person no longer filters");
        let too_many = PeopleFilter { include: (0..501).collect(), ..Default::default() };
        assert!(s.filter_paths(&too_many).is_err());
    }

    #[test]
    fn a_shot_is_one_photo_whatever_files_it_has() {
        assert_eq!(shot_key("/d/IMG_1.CR3"), shot_key("/d/img_1.jpg"));
        assert_eq!(shot_key("/d/IMG_1.jpg"), shot_key("/d/Processed/IMG_1.jpg"));
        assert_eq!(shot_key("/d/IMG_1.jpg"), shot_key("/d/processed/IMG_1.tif"));
        assert_ne!(shot_key("/d/IMG_1.jpg"), shot_key("/d/IMG_10.jpg"));
        assert_ne!(shot_key("/d/IMG_1.jpg"), shot_key("/e/IMG_1.jpg"), "another folder is another shot");
        assert_ne!(shot_key("/d/IMG_1.jpg"), shot_key("/d/IMG_1-edit.jpg"), "a differently named derivative is not matched");
        assert_ne!(shot_key("/d/IMG_1.jpg"), shot_key("/d/sub/Processed/IMG_1.jpg"));
    }

    #[test]
    fn counts_are_of_shots_not_files() {
        let s = store();
        // two shots: a RAW + JPG + Processed triple, and a single JPG
        let raw = scan(&s, "/d/A.arw", &[nf(0.1, 1.0)]);
        let jpg = scan(&s, "/d/A.jpg", &[nf(0.1, 1.0)]);
        let pro = scan(&s, "/d/Processed/A.jpg", &[nf(0.1, 1.0)]);
        let other = scan(&s, "/d/B.jpg", &[nf(0.1, 0.9)]);
        let p = s.create_person("Ann", &[raw[0], jpg[0], pro[0], other[0]]).unwrap();
        let row = s.list_people(false).unwrap().into_iter().find(|r| r.id == p).unwrap();
        assert_eq!((row.faces, row.photos), (4, 2), "four faces in two photos");
        assert_eq!(s.totals().unwrap().photos, 2);
        let pp = s.person_photos(p, 0, 10).unwrap();
        assert_eq!(pp.total, 2);
        assert_eq!(pp.paths.len(), 4, "every file is still returned");
        // the files of a shot sit together
        let at = |name: &str| pp.paths.iter().position(|x| x == name).unwrap();
        let triple = [at("/d/A.arw"), at("/d/A.jpg"), at("/d/Processed/A.jpg")];
        assert_eq!(triple.iter().max().unwrap() - triple.iter().min().unwrap(), 2, "{:?}", pp.paths);
        // a group's photos too
        let (x, y) = (scan(&s, "/e/C.arw", &[nf(0.1, 0.3)]), scan(&s, "/e/C.jpg", &[nf(0.1, 0.3)]));
        s.set_analysis(&[], &[(x[0], vec![x[0], y[0]])]).unwrap();
        let g = &s.list_clusters().unwrap()[0];
        assert_eq!((g.size, g.photos), (2, 1));
        let st = s.stats().unwrap();
        assert_eq!((st.files, st.photos, st.faces, st.people, st.groups, st.unassigned, st.rejected, st.failed), (6, 3, 6, 1, 1, 2, 0, 0));
        assert_eq!(st.models.len(), 1);
    }

    #[test]
    fn a_version_two_store_gets_its_shots_filled_in() {
        let old = Connection::open_in_memory().unwrap();
        old.execute_batch(&format!("BEGIN; {} {} PRAGMA user_version = 2; COMMIT;", LADDER[0], LADDER[1])).unwrap();
        old.execute("INSERT INTO photos(path,fingerprint,model,scanned_at,width,height) VALUES('/d/Processed/X.jpg','f','m',1,10,10)", []).unwrap();
        let s = FaceStore::from_connection(old).unwrap();
        assert_eq!(s.schema_version().unwrap(), 3);
        assert_eq!(s.totals().unwrap().photos, 1);
        assert!(s.stats().unwrap().db_bytes == 0, "an in-memory store has no file");
    }

    #[test]
    fn reprocessing_with_another_model_keeps_who_each_face_was_named_as() {
        let s = store();
        // a photo with three faces: Ann (cover), Bob, and one that is not Ann
        let a = scan(&s, "/a.jpg", &[nf(0.05, 1.0), nf(0.35, 0.8), nf(0.65, 0.5)]);
        let other = scan(&s, "/b.jpg", &[nf(0.05, 0.9), nf(0.4, 0.2)]);
        let ann = s.create_person("Ann", &[a[0]]).unwrap();
        let bob = s.create_person("Bob", &[a[1]]).unwrap();
        s.set_cover(ann, a[0]).unwrap();
        s.reject_for_person(ann, &[a[2]]).unwrap();
        s.set_analysis(&[], &[(a[2], vec![a[2], other[1]])]).unwrap();
        s.reject_faces(&[other[0]]).unwrap();
        assert_eq!(s.mark_all_stale().unwrap(), 2);
        assert!(!s.is_current("/a.jpg", "fp", "m1").unwrap(), "stale photos are scanned again");
        // the new model finds the same faces a little differently, in another order, plus a new one, and loses face 2
        let mut moved = nf(0.07, 0.3);
        moved.w = 0.19;
        let again = s.put_scan("/a.jpg", "fp", "m2", 1000, 800, &[nf(0.8, 0.1), moved, nf(0.36, 0.7)], None).unwrap();
        let f = |i: usize| s.face(again[i]).unwrap().unwrap();
        assert_eq!((f(1).person_id, f(1).state.as_str()), (Some(ann), "confirmed"), "Ann's face is found at the same place");
        assert_eq!((f(2).person_id, f(2).state.as_str()), (Some(bob), "confirmed"));
        assert_eq!((f(0).person_id, f(0).state.as_str()), (None, "unreviewed"), "a face new to the model is unreviewed");
        // the cover follows the face, the answer about the lost face (not Ann) went with it
        assert_eq!(s.list_people(false).unwrap().iter().find(|p| p.id == ann).unwrap().cover_face_id, Some(again[1]));
        assert_eq!(s.exemplars("m2").unwrap().len(), 2);
        assert!(s.unnamed("m2").unwrap().iter().all(|u| u.skip.is_empty()));
        // a rejected "not a face" photo is untouched and still hidden
        assert_eq!(s.face(other[0]).unwrap().unwrap().state, "rejected");
    }

    #[test]
    fn a_reprocess_keeps_joined_groups_and_not_this_person_answers_by_where_the_faces_are() {
        let s = store();
        let (ids, a, b) = grouped(&s);
        let p = s.create_person("Ann", &[ids[5]]).unwrap();
        s.merge_groups(a, b).unwrap();
        s.reject_for_person(p, &[ids[0]]).unwrap();
        let (links, rej) = (s.face_links().unwrap().len(), 1);
        assert!(links > 0);
        s.mark_all_stale().unwrap();
        // the same six faces, re-embedded
        let faces: Vec<NewFace> = [(0.05, 1.0), (0.2, 0.9), (0.35, 0.8), (0.5, 0.7), (0.65, 0.6), (0.8, 0.1)].iter().map(|(x, e)| nf(*x, *e)).collect();
        let again = s.put_scan("/a.jpg", "fp", "m2", 1000, 800, &faces, None).unwrap();
        assert_eq!(s.face_links().unwrap().len(), links, "joined groups stay joined");
        let u = s.unnamed("m2").unwrap();
        assert_eq!(u.iter().find(|x| x.id == again[0]).unwrap().skip, vec![p; rej]);
        assert_eq!(s.face(again[5]).unwrap().unwrap().person_id, Some(p));
    }

    #[test]
    fn meta_values_are_kept() {
        let s = store();
        assert_eq!(s.meta_get("k").unwrap(), None);
        s.meta_set("k", "1").unwrap();
        s.meta_set("k", "2").unwrap();
        assert_eq!(s.meta_get("k").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn delete_all_leaves_nothing_and_clear_index_keeps_people() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 1.0)]);
        s.create_person("Ann", &ids).unwrap();
        s.clear_index().unwrap();
        let t = s.totals().unwrap();
        assert_eq!((t.photos, t.faces, t.people), (0, 0, 1));
        scan(&s, "/a.jpg", &[nf(0.1, 1.0)]);
        s.delete_all().unwrap();
        let t = s.totals().unwrap();
        assert_eq!((t.photos, t.faces, t.people, t.groups), (0, 0, 0, 0));
        assert!(s.list_people(true).unwrap().is_empty() && s.can_undo().is_none());
    }
}

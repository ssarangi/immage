// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Applying a cull to the library, and taking it back. A session's decisions
//! are written through `Library`, which in the app is darktable's own rating,
//! reject and color-label path (so they show in the filmstrip, in Find and in
//! XMP sidecars). Rejects are flagged in place: nothing is moved or deleted.
//! Each decision records what every file looked like before and after, so
//! Undo restores the previous entry, and leaves alone any photo something
//! else has changed since.
use crate::cull_plan::CullStatus;
use crate::cull_store::{CullApplied, CullDecision, CullFailure, CullSession, CullTarget, CullLibraryState, CullSessionState, Store};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;

/// Reads and writes a photo's rating, reject flag and labels.
pub trait Library {
    fn read(&self, target: &CullTarget) -> Result<CullLibraryState, String>;
    fn write(&self, target: &CullTarget, state: &CullLibraryState) -> Result<(), String>;
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullApplyOptions {
    /// flag the rejects (default). Off applies only the highlight options.
    #[serde(default = "default_true")]
    pub reject: bool,
    /// give each highlight at least this many stars (1-5); never lowers one
    #[serde(default)]
    pub highlight_rating: Option<u8>,
    /// add this color label (one of the bits red 1, yellow 2, green 4, blue 8,
    /// purple 16) to each highlight
    #[serde(default)]
    pub highlight_label: Option<u32>,
}

impl Default for CullApplyOptions {
    fn default() -> Self {
        Self { reject: true, highlight_rating: None, highlight_label: None }
    }
}

/// A photo's entry after an apply or undo.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullChange {
    pub target: CullTarget,
    pub state: CullLibraryState,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullApplyResult {
    pub session: CullSession,
    /// every photo whose entry now differs, with it, so the page can update
    /// its thumbnails without reading the library again
    pub changes: Vec<CullChange>,
    /// decisions not applied (or not undone), each with the reason
    pub skipped: Vec<CullFailure>,
    /// photos flagged as rejected, and photos given a highlight rating or label
    pub rejected: i64,
    pub highlighted: i64,
}

/// the file as it is now, as the analysis saw it: size and mtime in ms
pub fn file_identity(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((meta.len() as i64, modified))
}

fn targets(decision: &CullDecision) -> Vec<CullTarget> {
    let mut all = vec![CullTarget { path: decision.path.clone(), version: decision.version }];
    all.extend(decision.backing.iter().cloned());
    all
}

fn wanted(before: CullLibraryState, decision: &CullDecision, status: CullStatus, options: &CullApplyOptions) -> CullLibraryState {
    let mut after = before;
    match status {
        CullStatus::Reject if options.reject => {
            after.rejected = true;
            after.rating = 0;
        }
        CullStatus::Keep if decision.highlight => {
            if let Some(stars) = options.highlight_rating.filter(|s| (1..=5).contains(s)) {
                if !before.rejected {
                    after.rating = before.rating.max(stars);
                }
            }
            if let Some(bit) = options.highlight_label.filter(|b| *b != 0 && *b < 32) {
                after.labels |= bit;
            }
        }
        _ => {}
    }
    after
}

/// whether `wanted` could differ from the photo's state, whatever that state is
fn can_change(decision: &CullDecision, options: &CullApplyOptions) -> bool {
    match decision.status {
        CullStatus::Reject => options.reject,
        CullStatus::Keep => {
            decision.highlight
                && (options.highlight_rating.is_some_and(|s| (1..=5).contains(&s)) || options.highlight_label.is_some_and(|b| b != 0 && b < 32))
        }
    }
}

/// Writes one decision's effect, returning what it recorded. A changed file
/// is refused: the verdict was for a different picture.
fn apply_decision(
    store: &Store,
    decision: &CullDecision,
    options: &CullApplyOptions,
    library: &dyn Library,
    check_file: bool,
) -> Result<Vec<CullApplied>, String> {
    if check_file && file_identity(Path::new(&decision.path)).is_some_and(|now| now != (decision.file_size, decision.file_modified_ms)) {
        return Err("the file changed after it was analysed".into());
    }
    // a decision that cannot change a photo (a plain keep) must not touch the library: each read imports
    // the photo into the engine, which made a 400-keep Scenes Apply take minutes for no change
    if !can_change(decision, options) {
        return Ok(Vec::new());
    }
    let mut records: Vec<CullApplied> = Vec::new();
    for target in targets(decision) {
        let before = library.read(&target)?;
        let after = wanted(before, decision, decision.status, options);
        if after == before {
            continue;
        }
        if let Err(error) = library.write(&target, &after) {
            // keep what did change undoable
            let _ = store.set_applied(decision.id, &records);
            return Err(error);
        }
        records.push(CullApplied { target, before, after });
    }
    store.set_applied(decision.id, &records)?;
    Ok(records)
}

fn change_list(records: impl IntoIterator<Item = (CullTarget, CullLibraryState)>) -> Vec<CullChange> {
    records.into_iter().map(|(target, state)| CullChange { target, state }).collect()
}

/// Applies a session's decisions. A session applies once; use `override_applied`
/// for later changes and `undo_session` to take it back.
pub fn apply_session(store: &Store, session_id: i64, options: &CullApplyOptions, library: &dyn Library) -> Result<CullApplyResult, String> {
    apply_session_with(store, session_id, options, library, &|_, _| {})
}

/// `apply_session`, calling `progress(done, total)` after each decision.
pub fn apply_session_with(
    store: &Store,
    session_id: i64,
    options: &CullApplyOptions,
    library: &dyn Library,
    progress: &dyn Fn(usize, usize),
) -> Result<CullApplyResult, String> {
    let session = store.session(session_id)?.ok_or("no such cull session")?;
    if session.state == CullSessionState::Applied {
        return Err("this cull is already applied; undo it first".into());
    }
    let (mut changes, mut skipped, mut rejected, mut highlighted) = (Vec::new(), Vec::new(), 0, 0);
    let all = store.decisions(session_id)?;
    let total = all.len();
    for (n, decision) in all.into_iter().enumerate() {
        progress(n, total);
        match apply_decision(store, &decision, options, library, true) {
            Ok(records) => {
                for record in &records {
                    if record.after.rejected && !record.before.rejected {
                        rejected += 1;
                    } else if record.after.rating != record.before.rating || record.after.labels != record.before.labels {
                        highlighted += 1;
                    }
                }
                changes.extend(change_list(records.into_iter().map(|r| (r.target, r.after))));
            }
            Err(error) => skipped.push(CullFailure { path: decision.path.clone(), error }),
        }
    }
    progress(total, total);
    store.set_state(session_id, CullSessionState::Applied)?;
    Ok(CullApplyResult { session: store.session(session_id)?.ok_or("session vanished")?, changes, skipped, rejected, highlighted })
}

/// Restores what the session wrote. Photos that something else changed since
/// are left as they are and reported.
pub fn undo_session(store: &Store, session_id: i64, library: &dyn Library) -> Result<CullApplyResult, String> {
    let session = store.session(session_id)?.ok_or("no such cull session")?;
    if session.state != CullSessionState::Applied {
        return Err("this cull is not applied".into());
    }
    let (mut changes, mut skipped) = (Vec::new(), Vec::new());
    for decision in store.decisions(session_id)?.into_iter().filter(|d| d.applied) {
        match restore(store, &decision, library) {
            Ok(restored) => changes.extend(change_list(restored)),
            Err(error) => skipped.push(CullFailure { path: decision.path.clone(), error }),
        }
    }
    store.set_state(session_id, CullSessionState::Undone)?;
    Ok(CullApplyResult { session: store.session(session_id)?.ok_or("session vanished")?, changes, skipped, rejected: 0, highlighted: 0 })
}

fn restore(store: &Store, decision: &CullDecision, library: &dyn Library) -> Result<Vec<(CullTarget, CullLibraryState)>, String> {
    let records = store.applied_records(decision.id)?;
    let mut restored = Vec::new();
    let mut left = Vec::new();
    let mut problem = None;
    for record in records {
        match library.read(&record.target) {
            Ok(now) if now == record.after => match library.write(&record.target, &record.before) {
                Ok(()) => restored.push((record.target, record.before)),
                Err(error) => {
                    problem = Some(error);
                    left.push(record);
                }
            },
            Ok(_) => {
                problem = Some("it was changed after the cull, so it was left as it is".to_string());
                left.push(record);
            }
            Err(error) => {
                problem = Some(error);
                left.push(record);
            }
        }
    }
    store.set_applied(decision.id, &left)?;
    match problem {
        Some(error) if restored.is_empty() => Err(error),
        _ => Ok(restored),
    }
}

/// A call on one photo of a cull that is already applied: puts the photo
/// back as it was and, for a reject, flags it again.
pub fn override_applied(store: &Store, decision_id: i64, library: &dyn Library) -> Result<Vec<CullChange>, String> {
    let decision = store.decision(decision_id)?.ok_or("no such decision")?;
    let mut changes = Vec::new();
    if decision.applied {
        changes.extend(change_list(restore(store, &decision, library)?));
    }
    if decision.status == CullStatus::Reject {
        let options = CullApplyOptions { reject: true, ..Default::default() };
        let records = apply_decision(store, &decision, &options, library, false)?;
        changes.extend(change_list(records.into_iter().map(|r| (r.target, r.after))));
    }
    Ok(changes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cull::{analyze, tests::sharp_scene};
    use crate::cull_plan::{plan, CullSettings, Input};
    use crate::cull_store::Item;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Fake(pub Mutex<HashMap<String, CullLibraryState>>);

    impl Library for Fake {
        fn read(&self, t: &CullTarget) -> Result<CullLibraryState, String> {
            self.0.lock().unwrap().get(&t.path).copied().ok_or_else(|| format!("{} is not in the library", t.path))
        }
        fn write(&self, t: &CullTarget, s: &CullLibraryState) -> Result<(), String> {
            self.0.lock().unwrap().insert(t.path.clone(), *s);
            Ok(())
        }
    }

    fn state(rating: u8, rejected: bool, labels: u32) -> CullLibraryState {
        CullLibraryState { rating, rejected, labels }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        library: Fake,
        session: i64,
        sharp: String,
        soft: String,
    }

    /// two files on disk: a sharp frame and a blurred twin, so the twin is a duplicate reject
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("cull.sqlite")).unwrap();
        let (sharp, soft) = (dir.path().join("sharp.jpg"), dir.path().join("soft.jpg"));
        let scene = sharp_scene(320, 240, 0);
        scene.save(&sharp).unwrap();
        image::imageops::blur(&scene, 1.5).save(&soft).unwrap();
        let (a, b) = (analyze(&scene), analyze(&image::imageops::blur(&scene, 1.5)));
        let inputs = vec![
            Input { path: sharp.to_string_lossy().into(), analysis: a.result, hash: a.hash },
            Input { path: soft.to_string_lossy().into(), analysis: b.result, hash: b.hash },
        ];
        let settings = CullSettings { duplicate_ratio: crate::cull_plan::DuplicateRatio::Less, highlight_percent: 100, ..Default::default() };
        let items = plan(&inputs, &settings)
            .into_iter()
            .map(|planned| {
                let (size, modified_ms) = file_identity(Path::new(&planned.path)).unwrap();
                Item { target: CullTarget { path: planned.path.clone(), version: 0 }, backing: vec![], size, modified_ms, planned }
            })
            .collect();
        let (session, _) = store.create_session("Test", &settings, items, vec![]).unwrap();
        let library = Fake::default();
        let (sharp, soft): (String, String) = (sharp.to_string_lossy().into(), soft.to_string_lossy().into());
        library.0.lock().unwrap().insert(sharp.clone(), state(3, false, 2));
        library.0.lock().unwrap().insert(soft.clone(), state(2, false, 0));
        Fixture { _dir: dir, store, library, session: session.id, sharp, soft }
    }

    fn lib(f: &Fixture, path: &str) -> CullLibraryState {
        f.library.0.lock().unwrap()[path]
    }

    #[test]
    fn apply_flags_rejects_in_place_and_undo_restores_them() {
        let f = fixture();
        let result = apply_session(&f.store, f.session, &CullApplyOptions::default(), &f.library).unwrap();
        assert_eq!((result.rejected, result.highlighted, result.skipped.len()), (1, 0, 0));
        assert_eq!(lib(&f, &f.soft), state(0, true, 0));
        assert_eq!(lib(&f, &f.sharp), state(3, false, 2), "the keeper is untouched");
        assert_eq!(result.session.state, CullSessionState::Applied);
        assert!(std::path::Path::new(&f.soft).exists(), "nothing is moved or deleted");
        assert_eq!(result.changes, vec![CullChange { target: CullTarget { path: f.soft.clone(), version: 0 }, state: state(0, true, 0) }]);

        assert!(apply_session(&f.store, f.session, &CullApplyOptions::default(), &f.library).is_err(), "applies once");

        let undone = undo_session(&f.store, f.session, &f.library).unwrap();
        assert_eq!(lib(&f, &f.soft), state(2, false, 0));
        assert_eq!((undone.session.state, undone.changes.len(), undone.skipped.len()), (CullSessionState::Undone, 1, 0));
        assert!(undo_session(&f.store, f.session, &f.library).is_err());
        // an undone session can be applied again
        assert_eq!(apply_session(&f.store, f.session, &CullApplyOptions::default(), &f.library).unwrap().rejected, 1);
    }

    #[test]
    fn highlights_raise_ratings_and_add_labels_without_lowering() {
        let f = fixture();
        let options = CullApplyOptions { reject: true, highlight_rating: Some(4), highlight_label: Some(4) };
        let result = apply_session(&f.store, f.session, &options, &f.library).unwrap();
        assert_eq!(result.highlighted, 1);
        assert_eq!(lib(&f, &f.sharp), state(4, false, 2 | 4));
        undo_session(&f.store, f.session, &f.library).unwrap();
        assert_eq!(lib(&f, &f.sharp), state(3, false, 2));
        // a 5-star photo keeps its stars
        f.library.0.lock().unwrap().insert(f.sharp.clone(), state(5, false, 0));
        apply_session(&f.store, f.session, &options, &f.library).unwrap();
        assert_eq!(lib(&f, &f.sharp).rating, 5);
    }

    /// a library that fails the test if it is touched
    struct Untouchable;
    impl Library for Untouchable {
        fn read(&self, t: &CullTarget) -> Result<CullLibraryState, String> {
            panic!("a keep with nothing to write read {}", t.path)
        }
        fn write(&self, t: &CullTarget, _: &CullLibraryState) -> Result<(), String> {
            panic!("a keep with nothing to write wrote {}", t.path)
        }
    }

    #[test]
    fn plain_keeps_never_touch_the_library_and_progress_reaches_the_total() {
        let f = fixture();
        let keep_only = f.store.decisions(f.session).unwrap().into_iter().filter(|d| d.status == CullStatus::Keep).count();
        assert!(keep_only >= 1);
        // reject = false and no highlight: every decision is a no-op
        let options = CullApplyOptions { reject: false, highlight_rating: None, highlight_label: None };
        let seen = Mutex::new(Vec::new());
        let result = apply_session_with(&f.store, f.session, &options, &Untouchable, &|d, t| seen.lock().unwrap().push((d, t))).unwrap();
        assert_eq!((result.rejected, result.highlighted, result.changes.len(), result.skipped.len()), (0, 0, 0, 0));
        let seen = seen.into_inner().unwrap();
        let total = f.store.decisions(f.session).unwrap().len();
        assert_eq!(seen.last(), Some(&(total, total)));
        assert_eq!(f.store.session(f.session).unwrap().unwrap().state, CullSessionState::Applied);
        // an applied session can be undone, and undoing touches nothing either
        undo_session(&f.store, f.session, &Untouchable).unwrap();
    }

    #[test]
    fn a_changed_file_is_not_applied_to() {
        let f = fixture();
        std::fs::write(&f.soft, b"replaced with something else").unwrap();
        let result = apply_session(&f.store, f.session, &CullApplyOptions::default(), &f.library).unwrap();
        assert_eq!(result.rejected, 0);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(lib(&f, &f.soft), state(2, false, 0));
    }

    #[test]
    fn undo_leaves_photos_changed_since() {
        let f = fixture();
        apply_session(&f.store, f.session, &CullApplyOptions::default(), &f.library).unwrap();
        // the user rated it again after the cull
        f.library.0.lock().unwrap().insert(f.soft.clone(), state(5, false, 0));
        let undone = undo_session(&f.store, f.session, &f.library).unwrap();
        assert_eq!(undone.skipped.len(), 1);
        assert_eq!(lib(&f, &f.soft), state(5, false, 0));
    }

    #[test]
    fn a_missing_library_entry_is_reported_not_fatal() {
        let f = fixture();
        f.library.0.lock().unwrap().remove(&f.soft);
        let result = apply_session(&f.store, f.session, &CullApplyOptions::default(), &f.library).unwrap();
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.session.state, CullSessionState::Applied);
    }

    #[test]
    fn overriding_after_apply_rewrites_just_that_photo() {
        let mut f = fixture();
        apply_session(&f.store, f.session, &CullApplyOptions::default(), &f.library).unwrap();
        let soft = f.store.decisions(f.session).unwrap().into_iter().find(|d| d.path == f.soft).unwrap();
        // keep the rejected twin after all
        f.store.override_decision(soft.id, CullStatus::Keep, false, "user").unwrap();
        let changes = override_applied(&f.store, soft.id, &f.library).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(lib(&f, &f.soft), state(2, false, 0));
        assert!(!f.store.decision(soft.id).unwrap().unwrap().applied);
        // and reject the keeper
        let sharp = f.store.decisions(f.session).unwrap().into_iter().find(|d| d.path == f.sharp).unwrap();
        f.store.override_decision(sharp.id, CullStatus::Reject, false, "user").unwrap();
        override_applied(&f.store, sharp.id, &f.library).unwrap();
        assert_eq!(lib(&f, &f.sharp), state(0, true, 2));
        assert!(f.store.decision(sharp.id).unwrap().unwrap().applied);
        // undo still restores it
        undo_session(&f.store, f.session, &f.library).unwrap();
        assert_eq!(lib(&f, &f.sharp), state(3, false, 2));
    }

    #[test]
    fn backing_files_take_the_same_decision() {
        let mut f = fixture();
        let extra = format!("{}.raw", f.soft);
        f.library.0.lock().unwrap().insert(extra.clone(), state(1, false, 0));
        // a second session whose reject has a backing file
        let inputs: Vec<_> = [&f.sharp, &f.soft]
            .into_iter()
            .map(|p| {
                let a = analyze(&image::open(p).unwrap().to_rgb8());
                Input { path: p.clone(), analysis: a.result, hash: a.hash }
            })
            .collect();
        let settings = CullSettings { duplicate_ratio: crate::cull_plan::DuplicateRatio::Less, ..Default::default() };
        let items = plan(&inputs, &settings)
            .into_iter()
            .map(|planned| {
                let (size, modified_ms) = file_identity(Path::new(&planned.path)).unwrap();
                let backing = if planned.path == f.soft { vec![CullTarget { path: extra.clone(), version: 0 }] } else { vec![] };
                Item { target: CullTarget { path: planned.path.clone(), version: 0 }, backing, size, modified_ms, planned }
            })
            .collect();
        let (session, _) = f.store.create_session("Pair", &settings, items, vec![]).unwrap();
        let result = apply_session(&f.store, session.id, &CullApplyOptions::default(), &f.library).unwrap();
        assert_eq!(result.rejected, 2);
        assert_eq!(lib(&f, &extra), state(0, true, 0));
    }
}

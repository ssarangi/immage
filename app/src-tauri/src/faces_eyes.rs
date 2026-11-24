// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Eye verdicts for culling: computed lazily for the photos a cull or a Scenes view needs, stored per face in
//! faces.sqlite (schema v4), read back by `faces_culling::assess`. The eye model is the user's own OCEC file
//! (faces_models.rs `eyes()`); without it every verdict stays unknown and culling uses pose alone. Nothing
//! here finds faces: a photo Faces has not scanned simply has none.
use crate::faces_culling::{self as fc, CullFace, EyeLabel, EyeMode, FaceQuality};
use crate::faces_detect as det;
use crate::faces_models::{self as fm, Model};
use crate::faces_store::{FaceStore, PathSets};
use crate::pack_cmds::AiState;
use crate::AppState;
use rusqlite::params;
use serde::Serialize;
use specta::Type;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

/// The most paths one `faces_eyes_for` call takes.
pub const MAX_PATHS: usize = 200;
/// `faces_eyes_for` stops starting new photos after this long and returns what it has.
const BUDGET: std::time::Duration = std::time::Duration::from_secs(20);
/// The long edge of the picture the eyes are cropped from: more pixels than detection used, since an eye is small.
const EYE_EDGE: u32 = 2048;

/// What `eye_model` holds for verdicts made by the pinned file.
pub fn model_tag() -> String {
    // the suffix names the crop recipe and the verdict rule (faces_detect::EYE_CROPS, `decide`): a change of either makes stored verdicts stale
    format!("ocec-s:{}:rule2", &fm::EYES_SHA256[..8])
}

/// A face that still needs a verdict.
#[derive(Debug, Clone)]
pub struct EyeTask {
    pub id: i64,
    pub face: det::Face,
    /// wide enough, frontal enough and not tilted: the only faces whose eyes are judged (`CullFace::eligible`, `MAX_TILT`)
    pub eligible: bool,
}

/// One face's verdict to store; `None` for a face that cannot be judged.
pub type EyeResult = (i64, Option<(f32, EyeLabel)>);

impl FaceStore {
    /// The stored faces of a photo as culling sees them, left to right (faces marked "not a face" are left out).
    pub fn cull_faces(&self, path: &str) -> Result<Vec<CullFace>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare(
                "SELECT f.id,f.w,f.h,f.w*p.width,f.frontal,f.eye_state FROM faces f JOIN photos p ON p.path=f.path
                 WHERE f.path=?1 AND f.state<>'rejected' ORDER BY f.x, f.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([path], |r| {
                let label: Option<String> = r.get(5)?;
                Ok(CullFace { id: r.get(0)?, w: r.get(1)?, h: r.get(2)?, px: r.get::<_, f64>(3)? as f32, frontal: r.get(4)?, eye: label.as_deref().and_then(EyeLabel::parse) })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// The faces of a photo with no verdict from `model` yet.
    pub fn eye_tasks(&self, path: &str, model: &str) -> Result<Vec<EyeTask>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare(
                "SELECT f.id,f.x,f.y,f.w,f.h,f.score,f.lm,f.frontal,f.w*p.width,f.roll FROM faces f JOIN photos p ON p.path=f.path
                 WHERE f.path=?1 AND f.state<>'rejected' AND (f.eye_model IS NULL OR f.eye_model<>?2) ORDER BY f.x, f.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map(params![path, model], |r| {
                let lm: Vec<u8> = r.get(6)?;
                let lm: Vec<f32> = lm.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
                Ok((r.get::<_, i64>(0)?, [r.get::<_, f32>(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?], lm, r.get::<_, f32>(7)?, r.get::<_, f64>(8)? as f32, r.get::<_, f32>(9)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (id, b, lm, frontal, px, roll) = row.map_err(|e| e.to_string())?;
            let eligible = px >= fc::MIN_EYE_PX && frontal >= fc::MIN_EYE_FRONTAL && lm.len() == 10 && (roll <= MAX_TILT || roll >= 180.0 - MAX_TILT);
            let lm: [(f32, f32); 5] = std::array::from_fn(|i| (lm.get(2 * i).copied().unwrap_or(0.0), lm.get(2 * i + 1).copied().unwrap_or(0.0)));
            out.push(EyeTask { id, face: det::Face { x: b[0], y: b[1], w: b[2], h: b[3], score: b[4], lm }, eligible });
        }
        Ok(out)
    }

    /// Stores verdicts made by `model`. A face with `None` is stored as "unknown" so it is not tried again.
    pub fn put_eyes(&self, model: &str, results: &[EyeResult]) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for (id, verdict) in results {
            let (open, label) = match verdict {
                Some((p, l)) => (Some(f64::from(*p)), l.as_str()),
                None => (None, "unknown"),
            };
            tx.execute("UPDATE faces SET eye_open=?2, eye_state=?3, eye_model=?4 WHERE id=?1", params![id, open, label, model]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// Photos whose verdict (by `mode`) is closed or half closed, among the photos whose eyes are computed.
    pub fn closed_eye_paths(&self, mode: EyeMode) -> Result<HashSet<String>, String> {
        let candidates: Vec<String> = {
            let db = self.db()?;
            let mut q = db.prepare("SELECT DISTINCT path FROM faces WHERE eye_state IN ('closed','semi') AND state<>'rejected'").map_err(|e| e.to_string())?;
            let rows = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        let mut out = HashSet::new();
        for path in candidates {
            if fc::eye_of(&self.cull_faces(&path)?, mode).is_some_and(|(label, _)| label != EyeLabel::Open) {
                out.insert(path);
            }
        }
        Ok(out)
    }

    /// Find's closed-eyes condition as an allow list.
    pub fn closed_eye_sets(&self, mode: EyeMode) -> Result<PathSets, String> {
        Ok(PathSets { allow: Some(self.closed_eye_paths(mode)?), deny: HashSet::new() })
    }
}

/// Everything the eye step needs, from the app's state.
#[derive(Clone)]
pub struct EyeCtx {
    pub store: Arc<FaceStore>,
    pub models: Arc<fm::Models>,
    pub runtime: PathBuf,
    pub output: Arc<crate::frame_output::FrameOutput>,
    pub engine: crate::engine::Tagged,
}

impl EyeCtx {
    pub fn from_app(app: &AppHandle) -> Option<Self> {
        let ai = app.try_state::<AiState>()?;
        let state = app.state::<AppState>();
        Some(Self { store: ai.faces_store.clone(), models: ai.faces.clone(), runtime: ai.faces_runtime.clone(), output: ai.faces_output.clone(), engine: state.tagged.clone() })
    }

    /// Faces is on, so culling may use what it stored.
    pub fn faces_on(&self) -> bool {
        self.models.enabled()
    }
}

/// The open probability in the OCEC outputs (`prob_open`, 1 is an open eye), clamped to 0..1.
pub fn open_prob(tensors: &[fm::Tensor]) -> Option<f32> {
    tensors.iter().find(|t| t.name == "prob_open").or(tensors.first()).and_then(|t| t.data.first().copied()).filter(|p| p.is_finite()).map(|p| p.clamp(0.0, 1.0))
}

/// One eye's probability of being open from each crop of `faces_detect::EYE_CROPS`.
/// `Ok(None)` when the eye cannot be cropped or a crop gave no answer.
pub fn eye_crops(engine: &crate::engine::Tagged, runtime: &Path, eyes_root: &Path, img: &image::RgbImage, face: &det::Face, right: bool) -> Result<Option<Vec<f32>>, String> {
    let mut probs = Vec::new();
    for (scale, down) in det::EYE_CROPS {
        let Some(input) = det::eye_input_with(img, face, right, scale, down, false) else { return Ok(None) };
        let (tensors, _) = fm::run(engine, runtime, eyes_root, Model::Eyes, &[1, 3, 24, 40], &input)?;
        let Some(p) = open_prob(&tensors) else { return Ok(None) };
        probs.push(p);
    }
    Ok(Some(probs))
}

/// One eye's probability of being open: the mean over its crops.
pub fn eye_probability(engine: &crate::engine::Tagged, runtime: &Path, eyes_root: &Path, img: &image::RgbImage, face: &det::Face, right: bool) -> Result<Option<f32>, String> {
    Ok(eye_crops(engine, runtime, eyes_root, img, face, right)?.map(|c| c.iter().sum::<f32>() / c.len() as f32))
}

/// A closed verdict needs the worse eye under this (mean of its crops) and both eyes under `CLOSED_BOTH`.
pub const CLOSED_WORSE: f32 = 0.15;
pub const CLOSED_BOTH: f32 = 0.25;
/// Below this the face is "half closed", a verdict that is only shown.
pub const SEMI_BELOW: f32 = 0.4;
/// The crops of one eye may disagree by this much (largest minus smallest) before the verdict is unknown.
pub const MAX_SPREAD: f32 = 0.5;
/// Faces tilted more than this (degrees of the eye line) get no verdict.
pub const MAX_TILT: f32 = 25.0;

/// The conservative verdict of a face from the crop probabilities of its right and left eye. A false "closed"
/// hides a good photo, a missed one costs little, so: closed only when both eyes are confidently closed,
/// half closed (shown, never scored) when the worse eye is under 0.4, unknown when the crops of an eye disagree.
/// Eyes cast down look closed to the model and are the main source of closed verdicts on portraits.
pub fn decide(eyes: [Option<&[f32]>; 2]) -> Option<(f32, EyeLabel)> {
    let mut means = Vec::new();
    for crops in eyes.into_iter().flatten() {
        let (lo, hi) = crops.iter().fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(*p), h.max(*p)));
        if crops.is_empty() || hi - lo > MAX_SPREAD {
            return None;
        }
        means.push(crops.iter().sum::<f32>() / crops.len() as f32);
    }
    let worse = means.iter().copied().fold(None, |m: Option<f32>, p| Some(m.map_or(p, |m| m.min(p))))?;
    let both_low = means.len() == 2 && means.iter().all(|m| *m < CLOSED_BOTH);
    let label = if worse < CLOSED_WORSE && both_low {
        EyeLabel::Closed
    } else if worse < SEMI_BELOW {
        EyeLabel::Semi
    } else {
        EyeLabel::Open
    };
    Some((worse, label))
}

/// A face's probability of open eyes: the worse eye decides, and one eye is enough.
pub fn face_probability(eyes: [Option<f32>; 2]) -> Option<f32> {
    eyes.into_iter().flatten().fold(None, |m: Option<f32>, p| Some(m.map_or(p, |m| m.min(p))))
}

/// The verdicts of one photo's pending faces: the picture is read once, and only when some face can be judged.
fn judge(ctx: &EyeCtx, eyes_root: &Path, path: &str, tasks: &[EyeTask]) -> Result<Vec<EyeResult>, String> {
    if !tasks.iter().any(|t| t.eligible) {
        return Ok(tasks.iter().map(|t| (t.id, None)).collect());
    }
    let img = crate::faces_cmds::load_picture(&ctx.engine, &ctx.output, path, EYE_EDGE)?;
    let mut out = Vec::new();
    for t in tasks {
        if !t.eligible {
            out.push((t.id, None));
            continue;
        }
        let [right, left] = [true, false].map(|right| eye_crops(&ctx.engine, &ctx.runtime, eyes_root, &img, &t.face, right));
        let (right, left) = (right?, left?);
        out.push((t.id, decide([right.as_deref(), left.as_deref()])));
    }
    Ok(out)
}

/// Computes and stores the missing verdicts of `path`. Returns whether anything was computed. Nothing happens
/// without the eye model, for a network photo or one Faces has not scanned.
pub fn ensure_photo(ctx: &EyeCtx, path: &str) -> Result<bool, String> {
    let Some(eyes_root) = ctx.models.eyes() else { return Ok(false) };
    if crate::volumes::on_network(Path::new(path)) {
        return Ok(false);
    }
    let model = model_tag();
    let tasks = ctx.store.eye_tasks(path, &model)?;
    if tasks.is_empty() {
        return Ok(false);
    }
    let results = judge(ctx, &eyes_root, path, &tasks)?;
    ctx.store.put_eyes(&model, &results)?;
    Ok(true)
}

/// The culling adjustment of a photo from what is stored now (never computes anything).
pub fn quality_of(store: &FaceStore, path: &str, mode: EyeMode) -> FaceQuality {
    match store.cull_faces(path) {
        Ok(faces) if !faces.is_empty() => fc::assess(&faces, mode),
        _ => FaceQuality::none(),
    }
}

/// One photo's eye summary for the page.
#[derive(Serialize, Debug, Clone, PartialEq, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhotoEyes {
    pub path: String,
    /// the verdict that counts (see `EyeMode`), None while unknown
    pub state: Option<EyeLabel>,
    /// the face the verdict came from, else the primary face
    pub primary_face: Option<i64>,
    /// faces stored for the photo (not counting "not a face")
    pub faces: u32,
}

pub fn photo_eyes(store: &FaceStore, path: &str, mode: EyeMode) -> PhotoEyes {
    let q = quality_of(store, path, mode);
    PhotoEyes { path: path.to_string(), state: q.eye, primary_face: q.eye_face.or(q.primary), faces: q.faces }
}

/// The eye verdicts of up to 200 photos, computing the missing ones (see the notes: faces-p4-backend-notes.md).
/// Stops starting photos after about 20 s; the rest come back unknown, so a second call finishes them.
#[tauri::command]
#[specta::specta]
pub async fn faces_eyes_for(app: AppHandle, paths: Vec<String>, mode: Option<EyeMode>) -> Result<Vec<PhotoEyes>, String> {
    if paths.len() > MAX_PATHS {
        return Err(format!("at most {MAX_PATHS} photos per call"));
    }
    let mode = mode.unwrap_or_default();
    let ctx = EyeCtx::from_app(&app).ok_or("Faces is not ready")?;
    if !ctx.faces_on() {
        return Ok(paths.into_iter().map(|path| PhotoEyes { path, state: None, primary_face: None, faces: 0 }).collect());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let mut computed = false;
        let mut seen: HashMap<String, bool> = HashMap::new();
        for path in &paths {
            if started.elapsed() > BUDGET {
                break;
            }
            if seen.contains_key(path) {
                continue;
            }
            // one photo's failure leaves it unknown; it must not stop the rest
            let did = ensure_photo(&ctx, path).unwrap_or(false);
            computed |= did;
            seen.insert(path.clone(), did);
        }
        if computed {
            let _ = app.emit("faces-changed", ());
        }
        Ok(paths.iter().map(|p| photo_eyes(&ctx.store, p, mode)).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Whether `faces_eyes_for` would be able to judge eyes at all.
#[allow(dead_code)]
pub fn available(state: &State<'_, AiState>) -> bool {
    state.faces.enabled() && state.faces.eyes().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faces_store::NewFace;

    fn store() -> FaceStore {
        FaceStore::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap()
    }

    fn nf(x: f32, w: f32, frontal: f32) -> NewFace {
        let mut lm = [0.0f32; 10];
        for (i, v) in [(0, x + w * 0.3), (1, 0.3), (2, x + w * 0.7), (3, 0.3), (4, x + w * 0.5), (5, 0.4), (6, x + w * 0.35), (7, 0.5), (8, x + w * 0.65), (9, 0.5)] {
            lm[i] = v;
        }
        NewFace { x, y: 0.2, w, h: w * 1.3, score: 0.9, lm, frontal, roll: 0.0, aligned: true, emb: vec![0.0; 128] }
    }

    fn scan(s: &FaceStore, path: &str, faces: &[NewFace]) -> Vec<i64> {
        s.put_scan(path, "fp", "m1", 1000, 800, faces, None).unwrap()
    }

    fn tensor(name: &str, v: f32) -> fm::Tensor {
        fm::Tensor { name: name.into(), shape: vec![1, 1], data: vec![v] }
    }

    #[test]
    fn a_high_open_probability_is_an_open_eye_and_a_low_one_is_closed() {
        // the OCEC output is the probability that the eye is OPEN: 1 means open (checked on real portraits)
        assert_eq!(open_prob(&[tensor("prob_open", 0.97)]).map(|p| det::eye_verdict(p).into()), Some(EyeLabel::Open));
        assert_eq!(open_prob(&[tensor("prob_open", 0.02)]).map(|p| det::eye_verdict(p).into()), Some(EyeLabel::Closed));
        assert_eq!(open_prob(&[tensor("prob_open", 0.5)]).map(|p| det::eye_verdict(p).into()), Some(EyeLabel::Semi));
        // the named output wins over the first one, an unnamed single output is used, junk is refused
        assert_eq!(open_prob(&[tensor("other", 0.1), tensor("prob_open", 0.9)]), Some(0.9));
        assert_eq!(open_prob(&[tensor("x", 0.8)]), Some(0.8));
        assert_eq!(open_prob(&[tensor("prob_open", f32::NAN)]), None);
        assert_eq!(open_prob(&[tensor("prob_open", 1.7)]), Some(1.0));
        assert_eq!(open_prob(&[]), None);
    }

    #[test]
    fn the_verdict_is_conservative() {
        let l = |c: [f32; 3]| decide([Some(&c[..]), Some(&[0.9f32, 0.9, 0.9][..])]).map(|v| v.1);
        // closed needs both eyes confidently closed
        assert_eq!(decide([Some(&[0.02f32, 0.05, 0.0][..]), Some(&[0.01f32, 0.0, 0.03][..])]).map(|v| v.1), Some(EyeLabel::Closed));
        // one closed eye and one open eye is not a closed face
        assert_eq!(l([0.0, 0.0, 0.0]), Some(EyeLabel::Semi));
        // a worse eye of 0.15 to 0.4 is half closed
        assert_eq!(decide([Some(&[0.3f32, 0.3, 0.3][..]), Some(&[0.2f32, 0.2, 0.2][..])]).map(|v| v.1), Some(EyeLabel::Semi));
        // both low but the worse eye above 0.15 is not closed
        assert_eq!(decide([Some(&[0.2f32, 0.2, 0.2][..]), Some(&[0.2f32, 0.2, 0.2][..])]).map(|v| v.1), Some(EyeLabel::Semi));
        assert_eq!(decide([Some(&[0.95f32, 0.9, 0.99][..]), Some(&[0.8f32, 0.9, 1.0][..])]).map(|v| v.1), Some(EyeLabel::Open));
        // crops that disagree by more than 0.5 make the verdict unknown, whatever the mean
        assert_eq!(l([0.0, 0.0, 0.9]), None);
        assert_eq!(decide([Some(&[0.0f32, 0.1, 0.7][..]), Some(&[0.0f32, 0.0, 0.0][..])]), None);
        // a single eye is never enough for closed, and nothing is unknown
        assert_eq!(decide([Some(&[0.0f32, 0.0, 0.0][..]), None]).map(|v| v.1), Some(EyeLabel::Semi));
        assert_eq!(decide([None, None]), None);
    }

    #[test]
    fn the_worse_eye_decides_and_one_eye_is_enough() {
        assert_eq!(face_probability([Some(0.95), Some(0.1)]), Some(0.1));
        assert_eq!(face_probability([None, Some(0.8)]), Some(0.8));
        assert_eq!(face_probability([None, None]), None);
        assert_eq!(det::eye_verdict(face_probability([Some(0.95), Some(0.1)]).unwrap()), det::EyeVerdict::Closed);
    }

    #[test]
    fn the_stored_model_tag_names_the_crop_recipe() {
        assert!(model_tag().starts_with("ocec-s:9a346a08:rule"), "{}", model_tag());
    }

    #[test]
    fn a_new_face_has_no_verdict_and_a_stored_one_comes_back_through_the_store() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 0.2, 0.9), nf(0.6, 0.03, 0.9), nf(0.4, 0.2, 0.2)]);
        let tag = model_tag();
        let tasks = s.eye_tasks("/a.jpg", &tag).unwrap();
        assert_eq!(tasks.len(), 3);
        // 200 px frontal: eligible; 30 px: tiny; frontal 0.2: profile
        assert_eq!(tasks.iter().map(|t| t.eligible).collect::<Vec<_>>(), vec![true, false, false]);
        assert!(s.cull_faces("/a.jpg").unwrap().iter().all(|f| f.eye.is_none()));
        let results: Vec<EyeResult> = vec![(ids[0], Some((0.1, EyeLabel::Closed))), (ids[1], None), (ids[2], None)];
        s.put_eyes(&tag, &results).unwrap();
        assert!(s.eye_tasks("/a.jpg", &tag).unwrap().is_empty(), "unknown is not tried again");
        assert_eq!(s.eye_tasks("/a.jpg", "other-model").unwrap().len(), 3, "another eye model redoes them");
        let faces = s.cull_faces("/a.jpg").unwrap();
        assert_eq!(faces[0].eye, Some(EyeLabel::Closed));
        assert_eq!(faces[1].eye, None);
        assert!((faces[0].px - 200.0).abs() < 0.5);
        let q = quality_of(&s, "/a.jpg", EyeMode::MainSubject);
        assert_eq!((q.eye, q.faces, q.primary), (Some(EyeLabel::Closed), 3, Some(ids[0])));
        assert!(q.adjust < -0.1);
    }

    #[test]
    fn a_rescan_clears_the_verdicts_and_a_rejected_face_does_not_count() {
        let s = store();
        let ids = scan(&s, "/a.jpg", &[nf(0.1, 0.2, 0.9), nf(0.6, 0.2, 0.9)]);
        s.put_eyes(&model_tag(), &[(ids[0], Some((0.05, EyeLabel::Closed))), (ids[1], Some((0.9, EyeLabel::Open)))]).unwrap();
        s.reject_faces(&[ids[0]]).unwrap();
        let faces = s.cull_faces("/a.jpg").unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].id, ids[1]);
        assert!(s.closed_eye_paths(EyeMode::MainSubject).unwrap().is_empty(), "the closed face was marked not a face");
        // the other face is found again: its old verdict is gone with its row
        let again = scan(&s, "/a.jpg", &[nf(0.6, 0.2, 0.9)]);
        assert_eq!(s.eye_tasks("/a.jpg", &model_tag()).unwrap().len(), again.len());
    }

    #[test]
    fn find_keeps_photos_whose_main_or_any_face_is_closed() {
        let s = store();
        let a = scan(&s, "/closed.jpg", &[nf(0.1, 0.3, 0.9)]);
        let b = scan(&s, "/open.jpg", &[nf(0.1, 0.3, 0.9)]);
        // a big open main face with a small closed one beside it
        let c = scan(&s, "/group.jpg", &[nf(0.1, 0.4, 0.95), nf(0.6, 0.12, 0.8)]);
        let t = model_tag();
        s.put_eyes(&t, &[(a[0], Some((0.1, EyeLabel::Closed))), (b[0], Some((0.95, EyeLabel::Open))), (c[0], Some((0.9, EyeLabel::Open))), (c[1], Some((0.4, EyeLabel::Semi)))]).unwrap();
        let main = s.closed_eye_paths(EyeMode::MainSubject).unwrap();
        assert_eq!(main, ["/closed.jpg".to_string()].into_iter().collect());
        let any = s.closed_eye_paths(EyeMode::AnyFace).unwrap();
        assert!(any.contains("/closed.jpg") && any.contains("/group.jpg") && !any.contains("/open.jpg"));
        let sets = s.closed_eye_sets(EyeMode::MainSubject).unwrap();
        assert!(sets.passes("/closed.jpg") && !sets.passes("/open.jpg"));
    }

    #[test]
    fn the_photo_summary_names_the_face_the_verdict_came_from() {
        let s = store();
        let ids = scan(&s, "/g.jpg", &[nf(0.1, 0.4, 0.95), nf(0.6, 0.2, 0.8)]);
        s.put_eyes(&model_tag(), &[(ids[0], Some((0.9, EyeLabel::Open))), (ids[1], Some((0.05, EyeLabel::Closed)))]).unwrap();
        let main = photo_eyes(&s, "/g.jpg", EyeMode::MainSubject);
        assert_eq!((main.state, main.primary_face, main.faces), (Some(EyeLabel::Open), Some(ids[0]), 2));
        let any = photo_eyes(&s, "/g.jpg", EyeMode::AnyFace);
        assert_eq!((any.state, any.primary_face), (Some(EyeLabel::Closed), Some(ids[1])));
        let none = photo_eyes(&s, "/unknown.jpg", EyeMode::MainSubject);
        assert_eq!((none.state, none.primary_face, none.faces), (None, None, 0));
    }

    /// The eye step on real photos (read only), printing counts and timings, never pictures:
    /// IMMAGE_FACES_ROOT=<folder with models/ and engine/> IMMAGE_FACES_EYES=<folder with models/ocec_s.onnx>
    /// IMMAGE_TEST_ENGINE=<darktable-engine> IMMAGE_TEST_ORT=<libonnxruntime.so> IMMAGE_TEST_PHOTOS=<folder of JPEGs>
    /// [IMMAGE_FACES_LIMIT=20]   cargo test opt_in_real_eyes -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs the Faces models, the eye model, the built engine, ONNX Runtime and photos"]
    fn opt_in_real_eyes() {
        let var = |k: &str| PathBuf::from(std::env::var_os(k).unwrap_or_else(|| panic!("set {k}")));
        let (root, eyes, engine_path, ort, photos) = (var("IMMAGE_FACES_ROOT"), var("IMMAGE_FACES_EYES"), var("IMMAGE_TEST_ENGINE"), var("IMMAGE_TEST_ORT"), var("IMMAGE_TEST_PHOTOS"));
        let limit: usize = std::env::var("IMMAGE_FACES_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        fm::write_manifests(&root, &[("detector", "immage-yunet"), ("recognizer", "immage-sface")]).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let wrapper = runtime.join("engine-with-test-ort");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexec '{}' \"$@\" --conf 'plugins/ai/ort_library_path={}'\n", engine_path.display(), ort.display())).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let engine = crate::engine::Engine::spawn(&wrapper, &runtime, &runtime).expect("start the isolated engine");
        let tagged = engine.tagged();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&photos)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")))
            .collect();
        files.sort();
        files.truncate(limit);
        let store = Arc::new(store());
        let ctx = EyeCtx {
            store: store.clone(),
            models: Arc::new(fm::Models::new(root.clone(), eyes.clone())),
            runtime: runtime.clone(),
            output: Arc::new(crate::frame_output::FrameOutput::new(&runtime)),
            engine: tagged.clone(),
        };
        assert!(ctx.models.eyes().is_some(), "the eye file is missing or has another hash");
        let (mut with_faces, mut faces_total, mut detect_ms) = (0u32, 0u32, 0f64);
        for (i, p) in files.iter().enumerate() {
            let t0 = std::time::Instant::now();
            let Some(pic) = crate::analysis_image::picture(p) else { continue };
            let (faces, _) = crate::faces_service::detect_and_embed(&tagged, &runtime, &root, None, &pic.rgb).expect("faces");
            detect_ms += t0.elapsed().as_secs_f64() * 1000.0;
            faces_total += faces.len() as u32;
            with_faces += u32::from(!faces.is_empty());
            store.put_scan(&p.to_string_lossy(), "fp", fm::MODEL_VERSION, pic.rgb.width(), pic.rgb.height(), &faces, None).unwrap();
            let _ = i;
        }
        let (mut walls, mut eligible, mut judged, mut counts) = (Vec::new(), 0u32, 0u32, [0u32; 3]);
        for p in &files {
            let path = p.to_string_lossy().to_string();
            let tasks = store.eye_tasks(&path, &model_tag()).unwrap();
            if tasks.is_empty() {
                continue;
            }
            let n = tasks.iter().filter(|t| t.eligible).count() as u32;
            let t0 = std::time::Instant::now();
            ensure_photo(&ctx, &path).unwrap();
            let wall = t0.elapsed().as_secs_f64() * 1000.0;
            eligible += n;
            if n > 0 {
                walls.push((wall, n));
            }
            for f in store.cull_faces(&path).unwrap() {
                if let Some(e) = f.eye {
                    judged += 1;
                    counts[e.rank() as usize] += 1;
                }
            }
        }
        // the first photo pays for loading the eye model; the rest are the steady cost
        let steady: Vec<f64> = walls.iter().skip(1).map(|w| w.0).collect();
        let mean = |v: &[f64]| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
        let per_face: Vec<f64> = walls.iter().skip(1).map(|w| w.0 / f64::from(w.1)).collect();
        let mut sorted = steady.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        println!("eyes real: photos {}, with faces {with_faces}, faces {faces_total}, eligible faces {eligible}, judged {judged} (open {}, semi {}, closed {})", files.len(), counts[0], counts[1], counts[2]);
        println!("eyes real: detection (not the eye step) mean {:.0} ms per photo", detect_ms / files.len().max(1) as f64);
        println!(
            "eyes real: eye step first photo {:.0} ms (model load), then mean {:.0} ms, median {:.0} ms per photo, mean {:.0} ms per eligible face ({} photos)",
            walls.first().map_or(0.0, |w| w.0),
            mean(&steady),
            sorted.get(sorted.len() / 2).copied().unwrap_or(0.0),
            mean(&per_face),
            walls.len()
        );
        assert!(!walls.is_empty());
    }
}

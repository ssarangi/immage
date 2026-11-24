// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Faces commands: people and their faces, the review decisions (name, merge, remove,
//! "not this person", "not a face"), covers, face crops, eye state and deleting everything. The
//! switch itself is `analysis_set` for `AnalysisId::Faces` (pack_cmds.rs), off until the user
//! turns it on. Every change asks the service to recompute names and groups and tells the page
//! with the `faces-changed` event; scan progress arrives as `faces-progress`.
use crate::faces_detect as det;
use crate::faces_models::{self as fm, Model};
use crate::faces_store::{ClusterRow, FaceFilter, FaceRow, FaceTotals, PersonRow};
use crate::pack_cmds::AiState;
use crate::settings::Settings;
use crate::AppState;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

#[derive(serde::Serialize, specta::Type)]
pub struct FacesStatus {
    /// The Faces switch (Preferences > AI models).
    pub enabled: bool,
    /// The YuNet and SFace pack is installed.
    pub installed: bool,
    /// The user has supplied the eye-state model (optional, never downloaded).
    pub eyes_available: bool,
    pub totals: FaceTotals,
    /// Photos scanned in this session and the faces found in them.
    pub scanned: u32,
    pub faces_found: u32,
    /// Photos still to scan, once counted.
    pub pending: Option<u32>,
    pub folders_waiting: u32,
    /// Library folders that do not answer now (an unplugged drive, a share that is down).
    pub offline_folders: u32,
    /// Library folders on a network share that Faces does not scan (not allowed for Faces).
    pub skipped_network_sources: u32,
    /// Milliseconds of the last photo's detection and embedding, once measured.
    pub infer_ms: Option<u32>,
    /// "ok", "retrying" or "blocked", with the reason once not ok.
    pub health: String,
    pub failure: Option<String>,
    pub undo: Option<String>,
}

fn changed(app: &AppHandle, ai: &AiState) {
    ai.faces_service.request_analysis();
    let _ = app.emit("faces-changed", ());
}

#[tauri::command]
#[specta::specta]
pub fn faces_status(ai: State<'_, AiState>) -> Result<FacesStatus, String> {
    let progress = ai.faces_service.progress();
    let (infer_ms, _) = ai.faces_service.speed();
    let health = ai.faces_service.health();
    let offline_folders = if ai.faces.enabled() { offline_count(&ai.faces_runtime) } else { 0 };
    Ok(FacesStatus {
        enabled: ai.faces.enabled(),
        installed: ai.faces.installed(),
        eyes_available: ai.faces.eyes().is_some(),
        totals: ai.faces_store.totals()?,
        scanned: progress.scanned as u32,
        faces_found: progress.faces_found as u32,
        pending: progress.pending,
        folders_waiting: progress.folder_pending,
        offline_folders,
        skipped_network_sources: crate::faces_service::skipped_network_sources(&ai.faces_runtime, crate::volumes::MountSnapshot::take()),
        infer_ms,
        health: health.state.to_string(),
        failure: health.reason.map(str::to_string),
        undo: ai.faces_store.can_undo(),
    })
}

/// Library folders that do not answer. Folders on a network share are not asked at all (the rule is
/// to keep off shares unless told otherwise), and the answer is kept for 30 s so the status poll never
/// waits on a slow drive more than once in that time.
fn offline_count(runtime: &Path) -> u32 {
    static LAST: std::sync::Mutex<Option<(std::time::Instant, u32)>> = std::sync::Mutex::new(None);
    if let Some((at, n)) = *LAST.lock().unwrap() {
        if at.elapsed() < std::time::Duration::from_secs(30) {
            return n;
        }
    }
    let n = offline_with(runtime, &crate::volumes::MountSnapshot::take(), &|p| crate::volumes::answering(p));
    *LAST.lock().unwrap() = Some((std::time::Instant::now(), n));
    n
}

fn offline_with(runtime: &Path, snapshot: &crate::volumes::MountSnapshot, answers: &dyn Fn(&Path) -> bool) -> u32 {
    let selection = crate::search_analysis::library_selection(runtime);
    selection["folders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .filter(|f| !snapshot.is_network(Path::new(f)) && !answers(Path::new(f)))
        .count() as u32
}

/// Scans a folder and its subfolders once, whether or not it is in the Catalog. Needs the pack.
#[tauri::command]
#[specta::specta]
pub fn faces_scan_folder(ai: State<'_, AiState>, path: String) -> Result<(), String> {
    scan_folder(&ai, path, true, false)
}

/// `faces_scan_folder` with the choice of subfolders (only that folder when `subfolders` is false) and of
/// network shares: a folder on one is refused unless `allow_network` is true.
#[tauri::command]
#[specta::specta]
pub fn faces_scan_folder_with(ai: State<'_, AiState>, path: String, subfolders: bool, allow_network: bool) -> Result<(), String> {
    scan_folder(&ai, path, subfolders, allow_network)
}

fn refuse_network(folder: &Path, allow_network: bool, snapshot: &crate::volumes::MountSnapshot) -> Result<(), String> {
    if !allow_network && snapshot.is_network(folder) {
        return Err("That folder is on a network share. Faces does not read network shares unless you allow it for that folder.".into());
    }
    Ok(())
}

/// Allows (or stops allowing) Faces to scan a library source on a network share. Off for every source until then.
#[tauri::command]
#[specta::specta]
pub fn faces_network_source(app: AppHandle, ai: State<'_, AiState>, settings: State<'_, Settings>, path: String, allowed: bool) -> Result<Vec<String>, String> {
    let p = PathBuf::from(&path);
    if !p.is_absolute() {
        return Err("Choose a folder".into());
    }
    let mut list: Vec<String> = crate::faces_service::allowed_sources(&ai.faces_runtime).iter().filter_map(|p| p.to_str().map(str::to_string)).collect();
    list.retain(|x| *x != path);
    if allowed {
        list.push(path);
    }
    settings.set(crate::faces_service::NETWORK_SOURCES_KEY, &serde_json::to_string(&list).map_err(|e| e.to_string())?)?;
    changed(&app, &ai);
    Ok(list)
}

fn scan_folder(ai: &AiState, path: String, subfolders: bool, allow_network: bool) -> Result<(), String> {
    if !ai.faces.installed() {
        return Err("Install the Faces model first".into());
    }
    let folder = PathBuf::from(path);
    if !folder.is_absolute() {
        return Err("Choose a folder".into());
    }
    // judged from the mount table before the folder is looked at: nothing on a share is touched
    refuse_network(&folder, allow_network, &crate::volumes::MountSnapshot::take())?;
    if !folder.is_dir() {
        return Err("Choose a folder".into());
    }
    ai.faces_service.scan_folder(folder, subfolders, allow_network);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn faces_retry(ai: State<'_, AiState>) {
    ai.faces_service.retry();
}

#[tauri::command]
#[specta::specta]
pub fn faces_people(ai: State<'_, AiState>, include_removed: bool) -> Result<Vec<PersonRow>, String> {
    ai.faces_store.list_people(include_removed)
}

#[tauri::command]
#[specta::specta]
pub fn faces_groups(ai: State<'_, AiState>) -> Result<Vec<ClusterRow>, String> {
    ai.faces_store.list_clusters()
}

#[tauri::command]
#[specta::specta]
pub fn faces_list(ai: State<'_, AiState>, filter: FaceFilter, offset: u32, limit: u32) -> Result<Vec<FaceRow>, String> {
    ai.faces_store.list_faces(&filter, offset, limit.clamp(1, 2000))
}

#[tauri::command]
#[specta::specta]
pub fn faces_person_create(app: AppHandle, ai: State<'_, AiState>, name: String, faces: Vec<i64>) -> Result<i64, String> {
    let id = ai.faces_store.create_person(&name, &faces)?;
    changed(&app, &ai);
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub fn faces_person_rename(app: AppHandle, ai: State<'_, AiState>, id: i64, name: String) -> Result<(), String> {
    ai.faces_store.rename_person(id, &name)?;
    changed(&app, &ai);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn faces_person_merge(app: AppHandle, ai: State<'_, AiState>, from: i64, into: i64) -> Result<(), String> {
    ai.faces_store.merge(from, into)?;
    changed(&app, &ai);
    Ok(())
}

/// "Remove person" (hide and exclude from suggestions) or restore.
#[tauri::command]
#[specta::specta]
pub fn faces_person_remove(app: AppHandle, ai: State<'_, AiState>, id: i64, removed: bool) -> Result<(), String> {
    ai.faces_store.set_ignored(id, removed)?;
    changed(&app, &ai);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn faces_person_cover(app: AppHandle, ai: State<'_, AiState>, id: i64, face: i64) -> Result<(), String> {
    ai.faces_store.set_cover(id, face)?;
    changed(&app, &ai);
    Ok(())
}

/// Names faces after a person: the bulk assign and the confirm of a suggestion.
#[tauri::command]
#[specta::specta]
pub fn faces_assign(app: AppHandle, ai: State<'_, AiState>, person: i64, faces: Vec<i64>) -> Result<(), String> {
    ai.faces_store.assign(person, &faces)?;
    changed(&app, &ai);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn faces_unassign(app: AppHandle, ai: State<'_, AiState>, faces: Vec<i64>) -> Result<(), String> {
    ai.faces_store.unassign(&faces)?;
    changed(&app, &ai);
    Ok(())
}

/// "Not this person": rejects a suggestion or a naming, for good.
#[tauri::command]
#[specta::specta]
pub fn faces_not_person(app: AppHandle, ai: State<'_, AiState>, person: i64, faces: Vec<i64>) -> Result<(), String> {
    ai.faces_store.reject_for_person(person, &faces)?;
    changed(&app, &ai);
    Ok(())
}

/// "Not a face": a false detection.
#[tauri::command]
#[specta::specta]
pub fn faces_not_face(app: AppHandle, ai: State<'_, AiState>, faces: Vec<i64>) -> Result<(), String> {
    ai.faces_store.reject_faces(&faces)?;
    changed(&app, &ai);
    Ok(())
}

/// Takes faces back from "not a face".
#[tauri::command]
#[specta::specta]
pub fn faces_unignore(app: AppHandle, ai: State<'_, AiState>, faces: Vec<i64>) -> Result<(), String> {
    ai.faces_store.unignore_faces(&faces)?;
    changed(&app, &ai);
    Ok(())
}

/// The photos the scan could not read, for the strip's "Show".
#[tauri::command]
#[specta::specta]
pub fn faces_failed_photos(ai: State<'_, AiState>, limit: u32) -> Result<Vec<String>, String> {
    ai.faces_store.failed_photos(limit.clamp(1, 1000))
}

/// Reverses the latest change; returns what it was.
#[tauri::command]
#[specta::specta]
pub fn faces_undo(app: AppHandle, ai: State<'_, AiState>) -> Result<Option<String>, String> {
    let label = ai.faces_store.undo_last()?;
    if label.is_some() {
        changed(&app, &ai);
    }
    Ok(label)
}

/// Deletes every face, person, group and crop, and turns the switch off so nothing is scanned again unasked.
#[tauri::command]
#[specta::specta]
pub async fn faces_delete_all(app: AppHandle, ai: State<'_, AiState>, state: State<'_, AppState>, settings: State<'_, Settings>) -> Result<(), String> {
    ai.faces.enabled_flag().store(false, Ordering::Release);
    settings.set(crate::analysis_settings::enabled_key(crate::analysis_settings::AnalysisId::Faces), "0")?;
    state.analysis_queue.cancel();
    ai.faces_store.delete_all()?;
    let dir = ai.faces_crops.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(dir)).await;
    let _ = app.emit("faces-changed", ());
    Ok(())
}

// ---- crops -------------------------------------------------------------------

/// The square, in pixels, around a face: its longer side times `margin`, moved inside the picture
/// and no bigger than it. Returns (x, y, side).
pub fn crop_rect(f: &FaceRow, img_w: u32, img_h: u32, margin: f32) -> (u32, u32, u32) {
    let (w, h) = (img_w as f32, img_h as f32);
    let side = ((f.w * w).max(f.h * h) * margin).clamp(8.0, w.min(h));
    let (cx, cy) = ((f.x + f.w / 2.0) * w, (f.y + f.h / 2.0) * h);
    let x = (cx - side / 2.0).clamp(0.0, w - side);
    let y = (cy - side / 2.0).clamp(0.0, h - side);
    (x.round() as u32, y.round() as u32, (side.round() as u32).min(img_w).min(img_h).max(1))
}

const CROP_EDGE: u32 = 2048;

fn load_picture(engine: &crate::engine::Tagged, output: &crate::frame_output::FrameOutput, path: &str, edge: u32) -> Result<image::RgbImage, String> {
    if let Some(p) = crate::analysis_image::picture_at(Path::new(path), edge) {
        return Ok(p.rgb);
    }
    let tag = engine.tag();
    let out = output.path(&format!("faces-crop-{tag}.jpg"));
    let imgid = engine.cached_photo(path, 0);
    let preview = (|| {
        engine.request(tag, serde_json::json!({"cmd":"analysis_preview","imgid":imgid,"subject":"original","out":out}))?;
        crate::frame_output::FrameOutput::read(&out).map_err(|e| e.to_string())
    })();
    output.remove(&out);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(preview?));
    reader.set_format(image::ImageFormat::Jpeg);
    reader.decode().map(|i| i.to_rgb8()).map_err(|e| e.to_string())
}

fn write_crop(img: &image::RgbImage, f: &FaceRow, size: u32, file: &Path) -> Result<(), String> {
    let (x, y, side) = crop_rect(f, img.width(), img.height(), 1.6);
    let crop = image::imageops::crop_imm(img, x, y, side, side).to_image();
    let small = image::imageops::resize(&crop, size, size, image::imageops::FilterType::Lanczos3);
    let tmp = file.with_extension("tmp");
    let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&small).map_err(|e| e.to_string())?;
    drop(out);
    std::fs::rename(&tmp, file).map_err(|e| e.to_string())
}

/// A face's picture as a JPEG file in the local cache, made from the source photo the first time it
/// is asked for (`size` 32 to 512). The cache holds no more than a few thousand and is emptied by
/// `faces_delete_all`.
#[tauri::command]
#[specta::specta]
pub async fn faces_crop(ai: State<'_, AiState>, state: State<'_, AppState>, face: i64, size: u32) -> Result<String, String> {
    let engine = state.engine("faces_crop").tagged();
    let file = crop_file(&ai, engine, face, size).await?;
    Ok(file.to_string_lossy().into_owned())
}

/// `faces_crop` as the JPEG bytes (the page cannot read a path: the CSP has no file scheme), like `thumb_cached`.
#[tauri::command]
#[specta::specta]
pub async fn faces_crop_bytes(ai: State<'_, AiState>, state: State<'_, AppState>, face: i64, size: u32) -> Result<crate::bindings::Frame, String> {
    let engine = state.engine("faces_crop").tagged();
    let file = crop_file(&ai, engine, face, size).await?;
    let bytes = tauri::async_runtime::spawn_blocking(move || std::fs::read(file)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    Ok(crate::bindings::Frame(tauri::ipc::Response::new(bytes)))
}

async fn crop_file(ai: &AiState, engine: crate::engine::Tagged, face: i64, size: u32) -> Result<PathBuf, String> {
    let size = size.clamp(32, 512);
    let row = ai.faces_store.face(face)?.ok_or("No such face")?;
    let file = ai.faces_crops.join(format!("{face}-{size}.jpg"));
    if file.is_file() {
        return Ok(file);
    }
    let output = ai.faces_output.clone();
    let dir = ai.faces_crops.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let img = load_picture(&engine, &output, &row.path, CROP_EDGE)?;
        write_crop(&img, &row, size, &file)?;
        prune(&dir, 4000);
        Ok(file)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Keeps at most `keep` files, dropping the oldest.
fn prune(dir: &Path, keep: usize) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = read.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= keep {
        return;
    }
    files.sort();
    for (_, p) in files.iter().take(files.len() - keep) {
        let _ = std::fs::remove_file(p);
    }
}

// ---- eye state -----------------------------------------------------------------

#[derive(serde::Serialize, specta::Type)]
pub struct EyeState {
    /// False when the user has not supplied the eye model: nothing was computed.
    pub available: bool,
    /// Probability each eye is open (0 to 1), the person's right eye first (the left one in the picture).
    pub right: Option<f32>,
    pub left: Option<f32>,
    /// The worse eye decides.
    pub verdict: Option<det::EyeVerdict>,
}

#[tauri::command]
#[specta::specta]
pub async fn faces_eye_state(ai: State<'_, AiState>, state: State<'_, AppState>, face: i64) -> Result<EyeState, String> {
    let none = EyeState { available: false, right: None, left: None, verdict: None };
    let Some(eyes_root) = ai.faces.eyes() else { return Ok(none) };
    let row = ai.faces_store.face(face)?.ok_or("No such face")?;
    let lm = ai.faces_store.landmarks(face)?.ok_or("No landmarks")?;
    let (engine, output) = (state.engine("faces_eye").tagged(), ai.faces_output.clone());
    let runtime = ai.faces_runtime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let img = load_picture(&engine, &output, &row.path, CROP_EDGE)?;
        let f = det::Face { x: row.x, y: row.y, w: row.w, h: row.h, score: row.score, lm: std::array::from_fn(|i| (lm[2 * i], lm[2 * i + 1])) };
        let mut probs = [None, None];
        for (k, right) in [true, false].into_iter().enumerate() {
            let Some(input) = det::eye_input(&img, &f, right) else { continue };
            let (out, _) = fm::run(&engine, &runtime, &eyes_root, Model::Eyes, &[1, 3, 24, 40], &input)?;
            probs[k] = out.iter().find(|t| t.name == "prob_open").or(out.first()).and_then(|t| t.data.first().copied()).map(|p| p.clamp(0.0, 1.0));
        }
        let worst = probs.iter().flatten().copied().fold(None, |m: Option<f32>, p| Some(m.map_or(p, |m| m.min(p))));
        Ok(EyeState { available: true, right: probs[0], left: probs[1], verdict: worst.map(det::eye_verdict) })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(x: f32, y: f32, w: f32, h: f32) -> FaceRow {
        FaceRow { id: 1, path: "/a.jpg".into(), x, y, w, h, score: 0.9, frontal: 0.9, person_id: None, state: "unreviewed".into(), suggested_person: None, suggested_sim: None, cluster_id: None }
    }

    const TABLE: &str = "/dev/sda2 / ext4 rw 0 0\n//nas/photos /mnt/synology cifs rw 0 0\n";

    #[test]
    fn a_network_folder_is_refused_without_allow_network() {
        let snap = crate::volumes::MountSnapshot::from_table(TABLE);
        let nas = Path::new("/mnt/synology/photos");
        assert!(refuse_network(nas, false, &snap).unwrap_err().contains("network share"));
        assert!(refuse_network(nas, true, &snap).is_ok());
        assert!(refuse_network(Path::new("/home/me/pics"), false, &snap).is_ok());
    }

    #[test]
    fn offline_folders_never_ask_a_network_folder() {
        let dir = tempfile::tempdir().unwrap();
        Settings::load(dir.path()).set("immage.library", r#"{"folders":["/home/me/a","/home/me/gone","/mnt/synology/photos"]}"#).unwrap();
        let asked = std::sync::Mutex::new(Vec::new());
        let n = offline_with(dir.path(), &crate::volumes::MountSnapshot::from_table(TABLE), &|p| {
            asked.lock().unwrap().push(p.to_path_buf());
            !p.ends_with("gone")
        });
        assert_eq!(n, 1);
        assert!(asked.lock().unwrap().iter().all(|p| !p.starts_with("/mnt/synology")), "{:?}", asked.lock().unwrap());
    }

    #[test]
    fn the_crop_is_a_square_around_the_face_kept_inside_the_picture() {
        // a face of 100 x 120 px centred at (500, 400) in a 1000 x 800 picture: side 120 * 1.6 = 192
        let (x, y, side) = crop_rect(&row(0.45, 0.4 - 0.075, 0.1, 0.15), 1000, 800, 1.6);
        assert_eq!(side, 192);
        assert_eq!((x, y), (404, 224));
        // at a corner it slides inside instead of leaving the picture
        let (x, y, side) = crop_rect(&row(0.0, 0.0, 0.1, 0.1), 1000, 800, 1.6);
        assert_eq!((x, y), (0, 0));
        assert!(side >= 100);
        let (x, y, side) = crop_rect(&row(0.95, 0.9, 0.05, 0.1), 1000, 800, 1.6);
        assert!(x + side <= 1000 && y + side <= 800);
        // never bigger than the picture
        let (_, _, side) = crop_rect(&row(0.0, 0.0, 1.0, 1.0), 400, 300, 1.6);
        assert_eq!(side, 300);
    }

    #[test]
    fn a_crop_is_written_atomically_at_the_requested_size() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = image::RgbImage::from_pixel(400, 300, image::Rgb([10, 10, 10]));
        for y in 100..200 {
            for x in 150..250 {
                img.put_pixel(x, y, image::Rgb([250, 250, 250]));
            }
        }
        let file = dir.path().join("1-64.jpg");
        write_crop(&img, &row(0.375, 0.333, 0.25, 0.333), 64, &file).unwrap();
        let out = image::open(&file).unwrap().to_rgb8();
        assert_eq!((out.width(), out.height()), (64, 64));
        assert!(out.get_pixel(32, 32)[0] > 200, "the face is in the middle");
        assert!(!file.with_extension("tmp").exists());
    }

    #[test]
    fn the_crop_cache_keeps_the_newest_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..6 {
            let p = dir.path().join(format!("{i}.jpg"));
            std::fs::write(&p, b"x").unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_secs(100 - i);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        prune(dir.path(), 3);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["3.jpg", "4.jpg", "5.jpg"]);
    }
}

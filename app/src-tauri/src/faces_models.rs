// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Faces model pack (YuNet 2023mar detector and SFace 2021dec recognizer, from the OpenCV
//! model zoo) and the optional eye-state model, plus the call that runs one of their graphs in
//! the engine (engine/faces.c). The pack is installed by the generic pipeline in packs.rs.
//!
//! Eye state (OCEC, `ocec_s.onnx`) is deliberately not a registry pack: its weights carry no
//! licence statement. It is used only when the user has put that exact file (by SHA-256) into the
//! eyes folder; nothing here can download it and no release list names it.
use crate::analysis_settings::AnalysisId;
use crate::engine::Tagged;
use crate::packs::{PackFile, PackHooks, PackId, PackSpec, PackStore};
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// opencv_zoo at this commit; the files are Git LFS objects, served by the media host.
pub const REVISION: &str = "47534e27c9851bb1128ccc0102f1145e27f23f98";
/// Recorded with every stored face: a new model makes stored faces stale.
pub const MODEL_VERSION: &str = "yunet-2023mar+sface-2021dec";
/// Which network makes the embeddings. SFace is the default and the only one that ships; ArcFace
/// (InsightFace w600k_r50, non-commercial weights) is for local testing, used only when `faces.recognizer`
/// is "arcface" AND the user put the exact file in `<data>/faces-arcface/models/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recognizer {
    Sface,
    Arcface,
}
pub const ARCFACE_FILE: &str = "w600k_r50.onnx";
/// SHA-256 of the InsightFace buffalo_l w600k_r50.onnx (174383860 bytes), as the old app's installed manifest recorded it.
pub const ARCFACE_SHA256: &str = "4c06341c33c2ca1f86781dab0e829f88ad5b64be9fba56e56bc9ebdefc619e43";
pub const ARCFACE_DIMS: usize = 512;
const ARCFACE_ID: &str = "immage-arcface";

/// The stored model version of faces made by this recognizer: a change makes stored faces stale.
pub fn model_version(r: Recognizer) -> &'static str {
    match r {
        Recognizer::Sface => MODEL_VERSION,
        Recognizer::Arcface => "yunet-2023mar+arcface-w600k_r50",
    }
}
/// The long edge the photo is decoded at for detection (`faces.detect_size`): 1024 (default) or, finer and about four times
/// slower per photo, 1600 or 2048, which also looks at the picture in 640 px tiles to find small faces (the detector takes
/// 640 px, so a face under about 50 px in a 1024 picture is lost). On 40 of the owner's family photos: 54 faces at 1024,
/// 55 at 1600 whole, 72 at 1600 tiled, 76 at 2048 tiled.
pub const DETECT_SIZES: [u32; 3] = [1024, 1600, 2048];
pub fn wanted_detect_size(runtime: &Path) -> u32 {
    crate::settings::read_key(runtime, "faces.detect_size").and_then(|s| s.parse::<u32>().ok()).filter(|n| DETECT_SIZES.contains(n)).unwrap_or(1024)
}
/// The model version new scans store: a change of recognizer or of detection size makes stored faces stale.
pub fn version_for(runtime: &Path, r: Recognizer) -> String {
    match wanted_detect_size(runtime) {
        1024 => model_version(r).to_string(),
        n => format!("{}+d{n}", model_version(r)),
    }
}
/// The hidden setting `faces.recognizer`: "sface" (default) or "arcface".
pub fn wanted_recognizer(runtime: &Path) -> Recognizer {
    match crate::settings::read_key(runtime, "faces.recognizer").as_deref() {
        Some("arcface") => Recognizer::Arcface,
        _ => Recognizer::Sface,
    }
}
/// The thresholds that go with a recognizer. SFace's are the old app's (suggest 0.45, edge 0.48) plus the
/// attach and merge values derived from the owner's data; ArcFace's cosines sit lower (other people
/// 95th percentile 0.13 in a 60 photo sample, the same person's median 0.72), so its values are lower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub suggest: f32,
    pub cluster: f32,
    pub attach: f32,
    pub merge: f32,
}
pub fn thresholds(r: Recognizer) -> Thresholds {
    match r {
        Recognizer::Sface => Thresholds { suggest: 0.45, cluster: 0.48, attach: 0.42, merge: 0.60 },
        Recognizer::Arcface => Thresholds { suggest: 0.40, cluster: 0.35, attach: 0.30, merge: 0.55 },
    }
}

/// How readily faces are called the same person (`faces.strictness`): loose joins more (and mistakes more), strict
/// keeps more groups apart. The steps are 0.04 on each threshold: on the owner's SFace data the other people's
/// nearest faces reach 0.47 at most (95th percentile) while one person's faces have a 5th percentile of 0.63 to each
/// other, so a shift of 0.04 either way stays inside that gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strictness {
    Loose,
    Balanced,
    Strict,
}
impl Strictness {
    pub fn name(self) -> &'static str {
        match self {
            Strictness::Loose => "loose",
            Strictness::Balanced => "balanced",
            Strictness::Strict => "strict",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "loose" => Some(Strictness::Loose),
            "balanced" => Some(Strictness::Balanced),
            "strict" => Some(Strictness::Strict),
            _ => None,
        }
    }
}
pub fn wanted_strictness(runtime: &Path) -> Strictness {
    crate::settings::read_key(runtime, "faces.strictness").and_then(|s| Strictness::parse(&s)).unwrap_or(Strictness::Balanced)
}
pub fn thresholds_for(r: Recognizer, s: Strictness) -> Thresholds {
    let t = thresholds(r);
    let d = match s {
        Strictness::Loose => -0.04,
        Strictness::Balanced => 0.0,
        Strictness::Strict => 0.04,
    };
    Thresholds { suggest: t.suggest + d, cluster: t.cluster + d, attach: t.attach + d, merge: t.merge + d }
}

pub const DETECTOR_FILE: &str = "face_detection_yunet/face_detection_yunet_2023mar.onnx";
pub const RECOGNIZER_FILE: &str = "face_recognition_sface/face_recognition_sface_2021dec.onnx";
const ASSETS: &[PackFile] = &[
    PackFile {
        path: "models/face_detection_yunet/face_detection_yunet_2023mar.onnx",
        size: 232589,
        digest: "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
        git_blob: false,
    },
    PackFile {
        path: "models/face_recognition_sface/face_recognition_sface_2021dec.onnx",
        size: 38696353,
        digest: "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79",
        git_blob: false,
    },
];
const BASE: &str = "https://media.githubusercontent.com/media/opencv/opencv_zoo/47534e27c9851bb1128ccc0102f1145e27f23f98";

const LICENSES: &str = "YuNet (face_detection_yunet_2023mar.onnx): MIT License, Copyright (c) 2020 Shiqi Yu, from the OpenCV model zoo\nhttps://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet\n\nSFace (face_recognition_sface_2021dec.onnx): Apache License, Version 2.0, from the OpenCV model zoo\nhttps://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface\nYou may obtain a copy of the License at http://www.apache.org/licenses/LICENSE-2.0. The software is distributed on an \"AS IS\" BASIS, without warranties or conditions of any kind.\n\nThe models run on this computer only. Face embeddings are biometric data: they are stored in faces.sqlite in the app's data folder and nowhere else.\n";

pub static SPEC: PackSpec = PackSpec {
    id: PackId::Faces,
    name: "Faces (YuNet + SFace)",
    license: "MIT, Apache-2.0",
    license_url: "https://github.com/opencv/opencv_zoo",
    license_text: LICENSES,
    source_host: "media.githubusercontent.com",
    base_url: BASE,
    fallback_base_url: None,
    revision: REVISION,
    files: ASSETS,
    analyses: &[AnalysisId::Faces],
    hooks: PackHooks { prepare: write_engine_manifest, validate: validate_package },
};

/// One config.json per model folder: darktable's AI layer finds a model by the id in it.
pub fn write_manifests(root: &Path, models: &[(&str, &str)]) -> Result<(), String> {
    for (folder, id) in models {
        let dir = root.join("engine").join(folder);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(
            dir.join("config.json"),
            format!(r#"{{"id":"{id}","name":"Immage {id}","backend":"onnx","num_inputs":1}}"#),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn write_engine_manifest(root: &Path) -> Result<(), String> {
    write_manifests(root, &[("detector", DETECTOR_ID), ("recognizer", RECOGNIZER_ID)])
}
fn validate_package(root: &Path) -> Result<(), String> {
    for a in ASSETS {
        if !root.join(a.path).is_file() {
            return Err(format!("{} is missing", a.path));
        }
    }
    Ok(())
}
const DETECTOR_ID: &str = "immage-yunet";
const RECOGNIZER_ID: &str = "immage-sface";

/// The eye-state model: SHA-256 of the file the app accepts.
pub const EYES_SHA256: &str = "9a346a08b256ad70725044cd2aa582858e108c6f45d42a9c3415afc604ba9b64";
pub const EYES_FILE: &str = "ocec_s.onnx";
const EYES_ID: &str = "immage-ocec";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Detector,
    Recognizer,
    Eyes,
    Arcface,
}
impl Model {
    fn id_file(self) -> (&'static str, &'static str) {
        match self {
            Model::Detector => (DETECTOR_ID, DETECTOR_FILE),
            Model::Recognizer => (RECOGNIZER_ID, RECOGNIZER_FILE),
            Model::Eyes => (EYES_ID, EYES_FILE),
            Model::Arcface => (ARCFACE_ID, ARCFACE_FILE),
        }
    }
}

type Stamp = Vec<(u64, std::time::SystemTime)>;
pub struct Models {
    store: Arc<PackStore>,
    enabled: Arc<AtomicBool>,
    eyes_root: PathBuf,
    arcface_root: PathBuf,
    prepared: Mutex<Option<Stamp>>,
    eyes_ok: Mutex<Option<(u64, std::time::SystemTime, bool)>>,
    arcface_ok: Mutex<Option<(u64, std::time::SystemTime, bool)>>,
}

fn stamps(root: &Path) -> Result<Stamp, String> {
    ASSETS
        .iter()
        .map(|a| {
            let m = std::fs::symlink_metadata(root.join(a.path)).map_err(|_| "The Faces model is not installed".to_string())?;
            if !m.is_file() || m.len() != a.size {
                return Err("The Faces model changed; verify the download".into());
            }
            Ok((m.len(), m.modified().map_err(|e| e.to_string())?))
        })
        .collect()
}

impl Models {
    pub fn new(root: PathBuf, eyes_root: PathBuf) -> Self {
        let arcface_root = eyes_root.with_file_name("faces-arcface");
        Self {
            store: Arc::new(PackStore::new(&SPEC, root)),
            enabled: Arc::new(AtomicBool::new(false)),
            eyes_root,
            arcface_root,
            prepared: Mutex::new(None),
            eyes_ok: Mutex::new(None),
            arcface_ok: Mutex::new(None),
        }
    }
    pub fn store(&self) -> Arc<PackStore> {
        Arc::clone(&self.store)
    }
    pub fn enabled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.enabled)
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
    pub fn installed(&self) -> bool {
        self.store.installed()
    }
    /// The package folder, once its files are whole (hashed again only when they changed). Never downloads.
    pub fn package(&self) -> Result<PathBuf, String> {
        if self.store.is_downloading() {
            return Err("The Faces model download is still running".into());
        }
        let root = self.store.final_dir();
        let now = stamps(&root)?;
        let mut prepared = self.prepared.lock().unwrap();
        if prepared.as_ref() != Some(&now) {
            *prepared = None;
            self.store.set_verified(false);
            crate::packs::verify_package(&root, ASSETS).map_err(|e| e.to_string())?;
            write_engine_manifest(&root)?;
            self.store.record_verified();
            *prepared = Some(now);
        }
        self.store.set_verified(true);
        Ok(root)
    }
    /// The eyes folder, when it holds the exact OCEC file. The user supplies it; there is no download.
    pub fn eyes(&self) -> Option<PathBuf> {
        let file = self.eyes_root.join("models").join(EYES_FILE);
        let m = std::fs::metadata(&file).ok().filter(|m| m.is_file())?;
        let stamp = (m.len(), m.modified().ok()?);
        let mut ok = self.eyes_ok.lock().unwrap();
        if ok.as_ref().is_none_or(|o| (o.0, o.1) != stamp) {
            let digest = sha256_file(&file).ok();
            let good = digest.as_deref() == Some(EYES_SHA256);
            if good && write_manifests(&self.eyes_root, &[("eyes", EYES_ID)]).is_err() {
                return None;
            }
            *ok = Some((stamp.0, stamp.1, good));
        }
        ok.as_ref().is_some_and(|o| o.2).then(|| self.eyes_root.clone())
    }
    /// The ArcFace folder, when it holds the exact w600k_r50 file (never downloaded, never in a pack).
    pub fn arcface(&self) -> Option<PathBuf> {
        let file = self.arcface_root.join("models").join(ARCFACE_FILE);
        let m = std::fs::metadata(&file).ok().filter(|m| m.is_file())?;
        let stamp = (m.len(), m.modified().ok()?);
        let mut ok = self.arcface_ok.lock().unwrap();
        if ok.as_ref().is_none_or(|o| (o.0, o.1) != stamp) {
            let good = sha256_file(&file).ok().as_deref() == Some(ARCFACE_SHA256);
            if good && write_manifests(&self.arcface_root, &[("arcface", ARCFACE_ID)]).is_err() {
                return None;
            }
            *ok = Some((stamp.0, stamp.1, good));
        }
        ok.as_ref().is_some_and(|o| o.2).then(|| self.arcface_root.clone())
    }
    /// The recognizer to use now: the setting, but ArcFace only while its file is there.
    pub fn recognizer(&self, runtime: &Path) -> Recognizer {
        if wanted_recognizer(runtime) == Recognizer::Arcface && self.arcface().is_some() {
            Recognizer::Arcface
        } else {
            Recognizer::Sface
        }
    }
    pub fn eyes_root(&self) -> &Path {
        &self.eyes_root
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// One output tensor of a graph.
#[derive(Debug, Clone)]
pub struct Tensor {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

/// Splits an output file by the shapes the engine reported.
pub fn split_outputs(bytes: &[u8], shapes: &[(String, Vec<usize>)]) -> Result<Vec<Tensor>, String> {
    let mut at = 0usize;
    let mut out = Vec::new();
    for (name, shape) in shapes {
        let n: usize = shape.iter().product();
        let end = at + n * 4;
        if n == 0 || end > bytes.len() {
            return Err("The face model's output is shorter than its shapes".into());
        }
        let data: Vec<f32> = bytes[at..end].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        if data.iter().any(|v| !v.is_finite()) {
            return Err("The face model produced an invalid value".into());
        }
        out.push(Tensor { name: name.clone(), shape: shape.clone(), data });
        at = end;
    }
    if at != bytes.len() {
        return Err("The face model's output is longer than its shapes".into());
    }
    Ok(out)
}

fn private_file(dir: &Path, name: &str) -> Result<(PathBuf, std::fs::File), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path).map(|f| (path, f)).map_err(|e| e.to_string())
}

/// Runs one graph in the engine's face worker. `root` is the pack folder (or the eyes folder), and
/// `runtime` is where the short-lived tensor files go (0600, removed after the call).
pub fn run(engine: &Tagged, runtime: &Path, root: &Path, model: Model, shape: &[usize], input: &[f32]) -> Result<(Vec<Tensor>, f64), String> {
    if shape.iter().product::<usize>() != input.len() || input.iter().any(|v| !v.is_finite()) {
        return Err("Invalid face tensor".into());
    }
    let dir = runtime.join("faces-io");
    let id = uuid::Uuid::new_v4();
    let (in_path, mut file) = private_file(&dir, &format!("{id}.in"))?;
    let bytes: Vec<u8> = input.iter().flat_map(|v| v.to_le_bytes()).collect();
    let written = file.write_all(&bytes).and_then(|()| file.flush());
    drop(file);
    let out_path = dir.join(format!("{id}.out"));
    let (model_id, model_file) = model.id_file();
    let reply = written.map_err(|e| e.to_string()).and_then(|()| {
        engine.request(
            engine.tag(),
            json!({"cmd":"faces_infer","model_root":root,"model_id":model_id,"model_file":model_file,"input":in_path,"shape":shape,"output":out_path}),
        )
    });
    let _ = std::fs::remove_file(&in_path);
    let result = (|| {
        let reply = reply?;
        if reply["ok"].as_bool() != Some(true) {
            return Err(reply["error"].as_str().unwrap_or("face inference failed").to_string());
        }
        let shapes: Vec<(String, Vec<usize>)> = reply["outputs"]
            .as_array()
            .ok_or("the face model reported no outputs")?
            .iter()
            .map(|o| {
                let shape = o["shape"].as_array().map(|s| s.iter().filter_map(|d| d.as_u64().map(|d| d as usize)).collect()).unwrap_or_default();
                (o["name"].as_str().unwrap_or("").to_string(), shape)
            })
            .collect();
        let bytes = std::fs::read(&out_path).map_err(|e| e.to_string())?;
        Ok((split_outputs(&bytes, &shapes)?, reply["ms"].as_f64().unwrap_or(0.0)))
    })();
    let _ = std::fs::remove_file(&out_path);
    result
}

/// Asks the engine to free the face models (the service does it after a quiet spell).
pub fn unload(engine: &Tagged) {
    let _ = engine.request(engine.tag(), json!({"cmd":"faces_infer","unload":true}));
}

pub fn find<'a>(outputs: &'a [Tensor], name: &str) -> Option<&'a [f32]> {
    outputs.iter().find(|t| t.name == name).map(|t| t.data.as_slice())
}

/// The three feature maps' outputs of YuNet in the order `faces_detect::DetOutputs` wants.
pub fn detector_outputs(outputs: &[Tensor]) -> Result<crate::faces_detect::DetOutputs<'_>, String> {
    let get = |prefix: &str| -> Result<[&[f32]; 3], String> {
        let mut a = [&[][..]; 3];
        for (i, s) in [8, 16, 32].iter().enumerate() {
            a[i] = find(outputs, &format!("{prefix}_{s}")).ok_or_else(|| format!("YuNet output {prefix}_{s} is missing"))?;
        }
        Ok(a)
    };
    Ok(crate::faces_detect::DetOutputs { cls: get("cls")?, obj: get("obj")?, bbox: get("bbox")?, kps: get("kps")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_size_is_part_of_the_stored_version() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(wanted_detect_size(dir.path()), 1024);
        assert_eq!(version_for(dir.path(), Recognizer::Sface), MODEL_VERSION, "the default keeps the version stored so far");
        crate::settings::Settings::load(dir.path()).set("faces.detect_size", "1600").unwrap();
        assert_eq!(wanted_detect_size(dir.path()), 1600);
        assert_eq!(version_for(dir.path(), Recognizer::Sface), format!("{MODEL_VERSION}+d1600"));
        assert_ne!(version_for(dir.path(), Recognizer::Arcface), version_for(dir.path(), Recognizer::Sface));
        crate::settings::Settings::load(dir.path()).set("faces.detect_size", "999").unwrap();
        assert_eq!(wanted_detect_size(dir.path()), 1024, "only the offered sizes");
    }

    #[test]
    fn strictness_shifts_every_threshold_by_a_step_for_both_recognizers() {
        for r in [Recognizer::Sface, Recognizer::Arcface] {
            let (b, l, s) = (thresholds_for(r, Strictness::Balanced), thresholds_for(r, Strictness::Loose), thresholds_for(r, Strictness::Strict));
            assert_eq!(b, thresholds(r));
            for (lo, mid, hi) in [(l.suggest, b.suggest, s.suggest), (l.cluster, b.cluster, s.cluster), (l.attach, b.attach, s.attach), (l.merge, b.merge, s.merge)] {
                assert!((mid - lo - 0.04).abs() < 1e-6 && (hi - mid - 0.04).abs() < 1e-6);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(wanted_strictness(dir.path()), Strictness::Balanced);
        crate::settings::Settings::load(dir.path()).set("faces.strictness", "strict").unwrap();
        assert_eq!(wanted_strictness(dir.path()), Strictness::Strict);
        crate::settings::Settings::load(dir.path()).set("faces.strictness", "nonsense").unwrap();
        assert_eq!(wanted_strictness(dir.path()), Strictness::Balanced);
        assert_eq!(Strictness::parse("loose").map(Strictness::name), Some("loose"));
    }

    #[test]
    fn arcface_is_used_only_when_asked_for_and_its_exact_file_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = dir.path().join("rt");
        std::fs::create_dir_all(&runtime).unwrap();
        let models = Models::new(runtime.join("faces-models"), runtime.join("faces-eyes"));
        assert_eq!(models.recognizer(&runtime), Recognizer::Sface);
        crate::settings::Settings::load(&runtime).set("faces.recognizer", "arcface").unwrap();
        assert_eq!(wanted_recognizer(&runtime), Recognizer::Arcface);
        assert_eq!(models.recognizer(&runtime), Recognizer::Sface, "no file, no ArcFace");
        std::fs::create_dir_all(runtime.join("faces-arcface/models")).unwrap();
        std::fs::write(runtime.join("faces-arcface/models").join(ARCFACE_FILE), b"another file").unwrap();
        assert_eq!(models.recognizer(&runtime), Recognizer::Sface, "a file with another hash is refused");
        assert_ne!(model_version(Recognizer::Sface), model_version(Recognizer::Arcface));
        assert!(thresholds(Recognizer::Arcface).suggest < thresholds(Recognizer::Sface).suggest);
        assert!(!ASSETS.iter().any(|a| a.path.contains("w600k")), "ArcFace is in no pack");
    }


    #[test]
    fn the_pin_lists_two_files_with_known_sizes_and_both_licences() {
        assert_eq!(ASSETS.len(), 2);
        assert_eq!(SPEC.bytes(), 232589 + 38696353);
        assert!(ASSETS.iter().all(|a| a.digest.len() == 64 && !a.git_blob));
        assert!(BASE.ends_with(REVISION) && BASE.starts_with("https://") && !BASE.ends_with('/'));
        assert!(SPEC.license_text.contains("MIT") && SPEC.license_text.contains("Apache"));
        // every pinned path is `models/<file the engine loads>`
        assert!(ASSETS.iter().any(|a| a.path == format!("models/{DETECTOR_FILE}")));
        assert!(ASSETS.iter().any(|a| a.path == format!("models/{RECOGNIZER_FILE}")));
    }

    #[test]
    fn the_eye_model_is_in_no_release_list() {
        assert!(ASSETS.iter().all(|a| !a.path.contains("ocec")));
        assert!(!SPEC.license_text.contains("OCEC") && !SPEC.name.contains("OCEC"));
        assert_eq!(EYES_SHA256.len(), 64);
    }

    #[test]
    fn manifests_name_one_id_per_model_folder() {
        let dir = tempfile::tempdir().unwrap();
        write_engine_manifest(dir.path()).unwrap();
        let det = std::fs::read_to_string(dir.path().join("engine/detector/config.json")).unwrap();
        let rec = std::fs::read_to_string(dir.path().join("engine/recognizer/config.json")).unwrap();
        assert!(det.contains("\"id\":\"immage-yunet\"") && rec.contains("\"id\":\"immage-sface\""));
        assert!(serde_json::from_str::<serde_json::Value>(&det).is_ok());
    }

    #[test]
    fn outputs_are_split_by_shape_and_checked() {
        let mut bytes = Vec::new();
        for v in [1.0f32, 2.0, 3.0, 4.0, 5.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let shapes = vec![("a".to_string(), vec![1, 2]), ("b".to_string(), vec![1, 3])];
        let t = split_outputs(&bytes, &shapes).unwrap();
        assert_eq!((t[0].data.clone(), t[1].data.clone()), (vec![1.0, 2.0], vec![3.0, 4.0, 5.0]));
        assert!(split_outputs(&bytes[..16], &shapes).is_err(), "too short");
        assert!(split_outputs(&bytes, &shapes[..1]).is_err(), "longer than the shapes");
        let nan = f32::NAN.to_le_bytes();
        assert!(split_outputs(&nan, &[("a".into(), vec![1])]).is_err());
        assert!(find(&t, "b").is_some() && find(&t, "z").is_none());
    }

    #[test]
    fn a_missing_detector_output_is_named() {
        let tensors = vec![Tensor { name: "cls_8".into(), shape: vec![1], data: vec![0.0] }];
        assert!(detector_outputs(&tensors).err().unwrap().contains("cls_16"));
        assert!(detector_outputs(&tensors).is_err());
    }

    #[test]
    fn the_eye_file_is_used_only_when_its_hash_matches() {
        let dir = tempfile::tempdir().unwrap();
        let models = Models::new(dir.path().join("faces"), dir.path().join("eyes"));
        assert!(models.eyes().is_none());
        std::fs::create_dir_all(dir.path().join("eyes/models")).unwrap();
        std::fs::write(dir.path().join("eyes/models").join(EYES_FILE), b"not the model").unwrap();
        assert!(models.eyes().is_none(), "a file with another hash is refused");
    }
}

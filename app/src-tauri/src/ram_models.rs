// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Auto-keywords model pack: RAM++ (Recognize Anything Plus) as an ONNX graph that
//! returns one logit per tag, with its tag list and per-tag thresholds. The pins below
//! are SHA-256 over whole files. The pack is installed by the generic pipeline in
//! packs.rs; this file keeps the pins, the engine manifest, the image preprocessing
//! (384 px, ImageNet mean and std, as the model was exported) and the decision of which
//! tags a set of logits yields. Inference itself runs in the engine (engine/tags.c).
//!
//! Source: the owner's earlier pin (benjaminjonard/ram-plus-onnx) answers 401 since
//! 2026-09-22, so the pack is fetched from WC-ECL/ram-plus-onnx at a fixed commit. It is
//! an independent export of the official Apache-2.0 weights, not a byte copy of the old
//! file; its hashes below are what this app checks.
use crate::analysis_settings::AnalysisId;
use crate::packs::{PackFile, PackHooks, PackId, PackSpec, PackStore};
use image::imageops::FilterType;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

pub const REVISION: &str = "ce61dc6d6cc47769e69fc8b7ad7bfdfaf8c4e65b";
/// Recorded with every analysis; a new model or vocabulary invalidates stored tags.
pub const MODEL_VERSION: &str = "ram-plus-ce61dc6d";
pub const CLASSES: usize = 4585;
pub const EDGE: u32 = 384;
pub const TAGS_FILE: &str = "ram_tag_list.txt";
pub const THRESHOLDS_FILE: &str = "ram_tag_list_threshold.txt";
const ASSETS: &[PackFile] = &[
    PackFile {
        path: "model.onnx",
        size: 1849852180,
        digest: "f9c32ce00da39310ac9c1dda6298ef8fb3d3e9e413bf1b09ee853c4661b261e9",
        git_blob: false,
    },
    PackFile {
        path: TAGS_FILE,
        size: 46488,
        digest: "1ce432a23770e6a8c908e36a085f3435b9bfa2dfe3bcae70537adcf3c50e3a7a",
        git_blob: false,
    },
    PackFile {
        path: THRESHOLDS_FILE,
        size: 26601,
        digest: "cc8ea160ef5e37940d31f0e4714aab498c50bba9c6af3a324a6d85ef519f3334",
        git_blob: false,
    },
];

/// The Hugging Face address the files are fetched from today (WC-ECL's export at REVISION).
const ORIGINAL_BASE: &str = "https://huggingface.co/WC-ECL/ram-plus-onnx/resolve/ce61dc6d6cc47769e69fc8b7ad7bfdfaf8c4e65b";
/// The owner's GitHub release holding the same three files, named exactly as in ASSETS (the
/// folder `{RELEASE_BASE}/{path}`), built and checked by scripts/models-release.sh. Stays `None`
/// until the release exists; then set it to
/// `Some("https://github.com/ssarangi/immage/releases/download/models-ram-plus-v1")`. The release is
/// tried first and ORIGINAL_BASE second, with the same size and SHA-256 pins either way.
const RELEASE_BASE: Option<&str> = None;
const PRIMARY_BASE: &str = match RELEASE_BASE {
    Some(release) => release,
    None => ORIGINAL_BASE,
};
const FALLBACK_BASE: Option<&str> = match RELEASE_BASE {
    Some(_) => Some(ORIGINAL_BASE),
    None => None,
};

const APACHE: &str = "Apache License, Version 2.0\n\nRAM++ (Recognize Anything Plus) was created by the OPPO research team and is published at\nhttps://huggingface.co/xinyu1205/recognize-anything-plus-model under the Apache License 2.0.\nThe ONNX graph in this pack was exported from those official weights without changing them.\n\nLicensed under the Apache License, Version 2.0 (the \"License\"); you may not use this file\nexcept in compliance with the License. You may obtain a copy of the License at\n\n    http://www.apache.org/licenses/LICENSE-2.0\n\nUnless required by applicable law or agreed to in writing, software distributed under the\nLicense is distributed on an \"AS IS\" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND,\neither express or implied. See the License for the specific language governing permissions\nand limitations under the License.\n";

/// The registry entry of the RAM++ pack.
pub static SPEC: PackSpec = PackSpec {
    id: PackId::Ram,
    name: "Auto-keywords (RAM++)",
    license: "Apache-2.0",
    license_url: "https://huggingface.co/xinyu1205/recognize-anything-plus-model",
    license_text: APACHE,
    source_host: "huggingface.co",
    base_url: PRIMARY_BASE,
    fallback_base_url: FALLBACK_BASE,
    revision: REVISION,
    files: ASSETS,
    analyses: &[AnalysisId::Keywords],
    hooks: PackHooks { prepare: write_engine_manifest, validate: validate_package },
};

fn write_engine_manifest(root: &Path) -> Result<(), String> {
    let folder = root.join("engine");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    std::fs::write(
        folder.join("config.json"),
        r#"{"id":"immage-ram-plus","name":"Immage RAM++","backend":"onnx","num_inputs":1}"#,
    )
    .map_err(|e| e.to_string())
}
fn validate_package(root: &Path) -> Result<(), String> {
    Vocabulary::load(root).map(|_| ())
}

/// A tag the model found, with its probability.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoredTag {
    pub name: String,
    pub confidence: f32,
}

pub struct Vocabulary {
    tags: Vec<String>,
    thresholds: Vec<f32>,
}
impl Vocabulary {
    pub fn load(root: &Path) -> Result<Self, String> {
        let tags: Vec<String> = std::fs::read_to_string(root.join(TAGS_FILE))
            .map_err(|e| e.to_string())?
            .lines()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        let thresholds: Vec<f32> = std::fs::read_to_string(root.join(THRESHOLDS_FILE))
            .map_err(|e| e.to_string())?
            .lines()
            .filter_map(|l| l.trim().parse::<f32>().ok())
            .collect();
        Self::new(tags, thresholds)
    }
    pub fn new(tags: Vec<String>, thresholds: Vec<f32>) -> Result<Self, String> {
        if tags.len() != CLASSES
            || thresholds.len() != CLASSES
            || thresholds.iter().any(|t| !(0.0..=1.0).contains(t))
            || tags.iter().any(|t| t.contains('|') || t.contains(','))
        {
            return Err("RAM++ tag list is invalid or incomplete".into());
        }
        Ok(Self { tags, thresholds })
    }
    /// The tags whose probability passes the model's own threshold for that tag, strongest
    /// first. Stored whole, so a later change of the user's confidence setting needs no
    /// new analysis.
    pub fn select(&self, logits: &[f32]) -> Result<Vec<ScoredTag>, String> {
        if logits.len() != self.tags.len() {
            return Err(format!("RAM++ output has {} logits but {} tags", logits.len(), self.tags.len()));
        }
        let mut found: Vec<ScoredTag> = logits
            .iter()
            .zip(self.tags.iter().zip(&self.thresholds))
            .filter_map(|(logit, (name, threshold))| {
                let confidence = 1.0 / (1.0 + (-logit.clamp(-30.0, 30.0)).exp());
                (confidence >= *threshold).then(|| ScoredTag { name: name.clone(), confidence })
            })
            .collect();
        found.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
        Ok(found)
    }
}

/// 384 x 384 RGB, bilinear, ImageNet mean and std, CHW floats.
pub fn preprocess(rgb: &[u8], width: u32, height: u32) -> Result<Vec<f32>, String> {
    if width == 0 || height == 0 || rgb.len() != width as usize * height as usize * 3 {
        return Err("Invalid RGB image for RAM++".into());
    }
    let source = image::RgbImage::from_raw(width, height, rgb.to_vec()).ok_or("Invalid RGB image for RAM++")?;
    let resized = image::imageops::resize(&source, EDGE, EDGE, FilterType::Triangle);
    let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
    let plane = (EDGE * EDGE) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, p) in resized.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (f32::from(p[c]) / 255.0 - mean[c]) / std[c];
        }
    }
    Ok(out)
}

struct Prepared {
    stamps: Vec<(u64, std::time::SystemTime)>,
    vocabulary: Arc<Vocabulary>,
}

/// The pack, its switch and the confidence setting.
pub struct Models {
    store: Arc<PackStore>,
    /// Preferences > AI models > Add keywords automatically.
    enabled: Arc<AtomicBool>,
    /// Minimum confidence, percent, for a tag to become a keyword.
    threshold: Arc<AtomicU8>,
    /// Photos that look like one already analysed in their folder take its tags (a burst).
    burst: Arc<AtomicBool>,
    prepared: Mutex<Option<Prepared>>,
}
pub const DEFAULT_THRESHOLD: u8 = 70;
pub const MIN_THRESHOLD: u8 = 40;
pub const MAX_THRESHOLD: u8 = 95;

fn stamps(root: &Path) -> Result<Vec<(u64, std::time::SystemTime)>, String> {
    ASSETS
        .iter()
        .map(|asset| {
            let m = std::fs::symlink_metadata(root.join(asset.path))
                .map_err(|_| "The Auto-keywords model is not installed".to_string())?;
            if !m.is_file() || m.len() != asset.size {
                return Err("The Auto-keywords model changed; verify the download".into());
            }
            Ok((m.len(), m.modified().map_err(|e| e.to_string())?))
        })
        .collect()
}

impl Models {
    pub fn new(root: PathBuf) -> Self {
        Self {
            store: Arc::new(PackStore::new(&SPEC, root)),
            enabled: Arc::new(AtomicBool::new(false)),
            threshold: Arc::new(AtomicU8::new(DEFAULT_THRESHOLD)),
            burst: Arc::new(AtomicBool::new(true)),
            prepared: Mutex::new(None),
        }
    }
    pub fn store(&self) -> Arc<PackStore> {
        Arc::clone(&self.store)
    }
    pub fn enabled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.enabled)
    }
    pub fn threshold_flag(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.threshold)
    }
    pub fn burst_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.burst)
    }
    pub fn burst(&self) -> bool {
        self.burst.load(Ordering::Acquire)
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
    pub fn threshold(&self) -> u8 {
        self.threshold.load(Ordering::Acquire)
    }
    pub fn installed(&self) -> bool {
        self.store.installed()
    }
    /// Verifies the files by size and time, and by hash whenever they changed since
    /// the last check (about 6 s for the 1.8 GB graph, once). Never downloads.
    pub fn package(&self) -> Result<(PathBuf, Arc<Vocabulary>), String> {
        if self.store.is_downloading() {
            return Err("The Auto-keywords download is still running".into());
        }
        let root = self.store.final_dir();
        let now = stamps(&root)?;
        let mut prepared = self.prepared.lock().unwrap();
        if prepared.as_ref().is_none_or(|p| p.stamps != now) {
            *prepared = None;
            self.store.set_verified(false);
            crate::packs::verify_package(&root, ASSETS).map_err(|e| e.to_string())?;
            let vocabulary = Arc::new(Vocabulary::load(&root)?);
            if stamps(&root)? != now {
                return Err("The Auto-keywords model changed during verification".into());
            }
            write_engine_manifest(&root)?;
            self.store.record_verified();
            *prepared = Some(Prepared { stamps: now, vocabulary });
        }
        self.store.set_verified(true);
        Ok((root, Arc::clone(&prepared.as_ref().unwrap().vocabulary)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocabulary() -> Vocabulary {
        let tags = (0..CLASSES).map(|i| format!("tag{i}")).collect();
        Vocabulary::new(tags, vec![0.5; CLASSES]).unwrap()
    }

    #[test]
    fn the_pin_lists_three_files_and_a_known_size() {
        assert_eq!(ASSETS.len(), 3);
        assert_eq!(SPEC.bytes(), 1849852180 + 46488 + 26601);
        assert!(ASSETS.iter().all(|a| a.digest.len() == 64 && !a.git_blob));
        assert_eq!(SPEC.license, "Apache-2.0");
        assert!(ORIGINAL_BASE.ends_with(REVISION));
    }

    #[test]
    fn the_source_order_is_the_original_host_until_a_release_is_set() {
        match RELEASE_BASE {
            None => assert_eq!((SPEC.base_url, SPEC.fallback_base_url), (ORIGINAL_BASE, None)),
            Some(release) => {
                assert!(release.starts_with("https://github.com/ssarangi/immage/releases/download/"));
                assert_eq!((SPEC.base_url, SPEC.fallback_base_url), (release, Some(ORIGINAL_BASE)));
            }
        }
        // every source is https and ends without a slash, so `{base}/{path}` is well formed
        for base in [SPEC.base_url].into_iter().chain(SPEC.fallback_base_url) {
            assert!(base.starts_with("https://") && !base.ends_with('/'));
        }
    }

    #[test]
    fn select_applies_the_sigmoid_and_each_tags_own_threshold_strongest_first() {
        let mut logits = vec![-10.0f32; CLASSES];
        logits[3] = 0.0; // 0.5 passes a 0.5 threshold exactly
        logits[7] = 4.0; // 0.982
        logits[9] = -0.5; // 0.378 fails
        let found = vocabulary().select(&logits).unwrap();
        assert_eq!(found.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["tag7", "tag3"]);
        assert!((found[0].confidence - 0.9820138).abs() < 1e-5);
        assert!((found[1].confidence - 0.5).abs() < 1e-6);
    }

    #[test]
    fn select_survives_extreme_logits_and_rejects_a_wrong_length() {
        let mut logits = vec![f32::MIN; CLASSES];
        logits[0] = f32::MAX;
        let found = vocabulary().select(&logits).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].confidence <= 1.0);
        assert!(vocabulary().select(&logits[..10]).is_err());
    }

    #[test]
    fn a_vocabulary_must_match_the_model() {
        assert!(Vocabulary::new(vec!["a".into()], vec![0.5]).is_err());
        let mut tags: Vec<String> = (0..CLASSES).map(|i| format!("t{i}")).collect();
        tags[2] = "a|b".into();
        assert!(Vocabulary::new(tags, vec![0.5; CLASSES]).is_err());
        let mut bad = vec![0.5; CLASSES];
        bad[0] = 1.5;
        assert!(Vocabulary::new((0..CLASSES).map(|i| format!("t{i}")).collect(), bad).is_err());
    }

    #[test]
    fn preprocess_is_chw_normalized_and_checks_its_input() {
        let rgb = vec![255u8; 8 * 4 * 3];
        let t = preprocess(&rgb, 8, 4).unwrap();
        assert_eq!(t.len(), 3 * 384 * 384);
        assert!((t[0] - (1.0 - 0.485) / 0.229).abs() < 1e-5);
        assert!((t[384 * 384] - (1.0 - 0.456) / 0.224).abs() < 1e-5);
        assert!((t[2 * 384 * 384] - (1.0 - 0.406) / 0.225).abs() < 1e-5);
        assert!(t.iter().all(|v| v.is_finite()));
        assert!(preprocess(&rgb, 8, 5).is_err());
        assert!(preprocess(&[], 0, 0).is_err());
    }

    #[test]
    fn the_engine_manifest_names_the_graph_the_engine_loads() {
        let dir = tempfile::tempdir().unwrap();
        write_engine_manifest(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("engine/config.json")).unwrap();
        assert!(text.contains("\"id\":\"immage-ram-plus\"") && text.contains("onnx"));
        // engine/tags.c loads "../model.onnx" relative to that folder
        assert_eq!(ASSETS[0].path, "model.onnx");
    }

    /// Installs the real pack from the network through the pack pipeline into the folder named by
    /// IMMAGE_RAM_INSTALL_ROOT (a dev profile, never the owner's): `cargo test real_install -- --ignored`.
    /// Files already whole under `.partial-<revision>/` are verified and kept, the rest are fetched.
    #[test]
    #[ignore = "downloads about 1.8 GB"]
    fn real_install_through_the_pack_pipeline() {
        let root = std::env::var("IMMAGE_RAM_INSTALL_ROOT").expect("IMMAGE_RAM_INSTALL_ROOT");
        let store = PackStore::new(&SPEC, PathBuf::from(&root));
        store.install_blocking().unwrap();
        assert!(store.installed());
        assert_eq!(store.info().state, crate::packs::PackState::Installed);
        let models = Models::new(PathBuf::from(&root));
        let (_, vocabulary) = models.package().unwrap();
        assert_eq!(vocabulary.select(&vec![-30.0; CLASSES]).unwrap().len(), 0);
    }
}

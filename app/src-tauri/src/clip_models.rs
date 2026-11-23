// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Opt-in, immutable CLIP package provisioning. Opening the app never fetches assets.
//! The graph hashes are LFS SHA-256; small assets use their pinned Git blob IDs
//! (including the Git blob header), never an unverified checksum from a download.
//! The download, check, repair and removal pipeline is the generic one in packs.rs
//! (CLIP is its first pack); this file keeps the pins, the on-disk layout
//! (`<root>/<REVISION>/...`, unchanged since the first release) and the
//! CLIP-specific preparation of text and images.
use crate::analysis_settings::AnalysisId;
use crate::packs::{PackFile, PackHooks, PackId, PackSpec, PackStore};
#[cfg(test)]
use crate::packs::{copy_verified, verify_package, Cancellable, PackError, PackFile as Asset};
use crate::AppState;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::State;

pub const REVISION: &str = "6b7c523703ae26e39be4b03774966f8571c7f523";
#[cfg(test)]
const REPOSITORY: &str = "Xenova/clip-vit-base-patch32";
const ASSETS: &[PackFile] = &[
    PackFile {
        path: "onnx/text_model.onnx",
        size: 254058553,
        digest: "3f6571f5bad13a97c469c1622e1cfc4d9aef78b79fdbfcff804ca357bfada8cc",
        git_blob: false,
    },
    PackFile {
        path: "onnx/vision_model.onnx",
        size: 351685709,
        digest: "fd6e1402a588279d1723c7534d4bcba5bc0b14b47dfab0e46f8c47b8270d7d40",
        git_blob: false,
    },
    PackFile {
        path: "tokenizer.json",
        size: 2224119,
        digest: "bc1f77d20440541dd073ebae6f6c401087c7d34e",
        git_blob: true,
    },
    PackFile {
        path: "preprocessor_config.json",
        size: 520,
        digest: "e0675735fd0052745f2f7b7291e4aa7859389998",
        git_blob: true,
    },
    PackFile {
        path: "tokenizer_config.json",
        size: 775,
        digest: "c9b2a711cbdd039529fa5a66f1965e13eb14451d",
        git_blob: true,
    },
    PackFile {
        path: "special_tokens_map.json",
        size: 472,
        digest: "2c2130b544c0c5a72d5d00da071ba130a9800fb2",
        git_blob: true,
    },
    PackFile {
        path: "config.json",
        size: 4524,
        digest: "e79bad158b04c235740f9b2ec320b17f4030f7a5",
        git_blob: true,
    },
];

const MIT: &str = "MIT License\n\nCopyright (c) 2021 OpenAI\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files (the \"Software\"), to deal\nin the Software without restriction, including without limitation the rights\nto use, copy, modify, merge, publish, distribute, sublicense, and/or sell\ncopies of the Software, and to permit persons to whom the Software is\nfurnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all\ncopies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\nFITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\nAUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\nOUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\nSOFTWARE.\n";

/// The registry entry of the CLIP pack.
pub static SPEC: PackSpec = PackSpec {
    id: PackId::Clip,
    name: "Smart search (CLIP)",
    license: "MIT",
    license_url: "https://huggingface.co/Xenova/clip-vit-base-patch32",
    license_text: MIT,
    source_host: "huggingface.co",
    base_url: "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/6b7c523703ae26e39be4b03774966f8571c7f523",
    fallback_base_url: None,
    revision: REVISION,
    files: ASSETS,
    analyses: &[AnalysisId::SmartSearch],
    hooks: PackHooks { prepare: write_engine_manifest, validate: validate_package },
};
fn validate_package(root: &Path) -> Result<(), String> {
    crate::clip_image::validate_config(root)?;
    crate::clip_text::TextProcessor::load(root).map(|_| ())
}

#[derive(Clone, serde::Serialize, specta::Type)]
pub struct ModelStatus {
    pub revision: String,
    pub bytes: u64,
    pub downloading: bool,
    /// Installation verified in this process; not an inference availability claim.
    pub verified: bool,
    pub error: Option<String>,
}
pub struct Models {
    #[cfg_attr(not(test), allow(dead_code))]
    root: PathBuf,
    store: Arc<PackStore>,
    /// Preferences > AI models > Analyse photos for Smart search. Photos are
    /// analysed only while it is on; Smart search itself needs only the model.
    analysis: Arc<AtomicBool>,
    prepared: Mutex<Option<Prepared>>,
}
type FileStamps = Vec<(u64, std::time::SystemTime)>;
struct Prepared {
    stamps: FileStamps,
    text: Arc<crate::clip_text::TextProcessor>,
}
fn package_stamps(root: &Path) -> Result<FileStamps, String> {
    ASSETS
        .iter()
        .map(|asset| {
            let m = std::fs::symlink_metadata(root.join(asset.path))
                .map_err(|_| "Optional CLIP package is not installed".to_string())?;
            if !m.is_file() || m.len() != asset.size {
                return Err("CLIP package changed; verify the model download".into());
            }
            Ok((m.len(), m.modified().map_err(|e| e.to_string())?))
        })
        .collect()
}
impl Models {
    pub fn new(root: PathBuf) -> Self {
        Self {
            store: Arc::new(PackStore::new(&SPEC, root.clone())),
            root,
            analysis: Arc::new(AtomicBool::new(true)),
            prepared: Mutex::new(None),
        }
    }
    /// The pack entry for the pack manager; shares this struct's state.
    pub fn store(&self) -> Arc<PackStore> {
        Arc::clone(&self.store)
    }
    pub fn analysis_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.analysis)
    }
    pub fn analysis_enabled(&self) -> bool {
        self.analysis.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn installed(&self) -> bool { self.store.installed() }
    /// Lazy local verification only; never downloads. Rehash after any file stamp change.
    fn package(&self) -> Result<(PathBuf, Arc<crate::clip_text::TextProcessor>), String> {
        if self.store.is_downloading() {
            return Err("CLIP model verification is already running".into());
        }
        let root = self.store.final_dir();
        let stamps = package_stamps(&root)?;
        let mut prepared = self.prepared.lock().unwrap();
        if prepared.as_ref().is_none_or(|p| p.stamps != stamps) {
            *prepared = None;
            self.store.set_verified(false);
            crate::packs::verify_package(&root, ASSETS).map_err(|e| e.to_string())?;
            crate::clip_image::validate_config(&root)?;
            let processor = Arc::new(crate::clip_text::TextProcessor::load(&root)?);
            if package_stamps(&root)? != stamps {
                return Err("CLIP package changed during verification".into());
            }
            write_engine_manifest(&root)?;
            self.store.record_verified();
            *prepared = Some(Prepared {
                stamps,
                text: processor,
            });
        }
        self.store.set_verified(true);
        Ok((root, Arc::clone(&prepared.as_ref().unwrap().text)))
    }
    pub fn prepare_text(
        &self,
        text: &str,
    ) -> Result<(PathBuf, crate::clip_text::TextInput), String> {
        let (root, processor) = self.package()?;
        Ok((root, processor.encode(text)?))
    }
    pub fn prepare_image(
        &self,
        rgb: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(PathBuf, Vec<f32>), String> {
        let (root, _) = self.package()?;
        Ok((root, crate::clip_image::prepare_rgb(rgb, width, height)?))
    }
    pub fn status(&self) -> ModelStatus {
        ModelStatus {
            revision: REVISION.into(),
            bytes: SPEC.bytes(),
            downloading: self.store.is_downloading(),
            verified: self.store.verified(),
            error: self.store.error().map(|e| e.detail),
        }
    }
    pub fn cancel(&self) {
        self.store.cancel();
    }
    fn install(&self, consent: bool) -> Result<(), String> {
        // Check before any directory creation, client construction or network access.
        if !consent {
            return Err("Model download requires explicit opt-in".into());
        }
        self.store.install_blocking().map_err(|e| e.to_string())?;
        // a fresh install is read again by the next lazy check
        *self.prepared.lock().unwrap() = None;
        Ok(())
    }
    #[cfg(test)]
    fn install_with(
        &self,
        assets: &[Asset],
        fetch: impl FnMut(&Asset, &Path) -> Result<(), PackError>,
    ) -> Result<(), PackError> {
        self.store.install_with(assets, fetch)
    }
}
fn write_engine_manifest(root: &Path) -> Result<(), String> {
    let folder = root.join("engine");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    std::fs::write(
        folder.join("config.json"),
        r#"{"id":"immage-clip","name":"Immage CLIP ViT-B/32","backend":"onnx","num_inputs":1}"#,
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
#[specta::specta]
pub fn search_model_status(state: State<'_, AppState>) -> ModelStatus {
    state.clip_models.status()
}
#[tauri::command]
#[specta::specta]
pub async fn search_model_download(
    state: State<'_, AppState>,
    consent: bool,
) -> Result<(), String> {
    let models = state.clip_models.clone();
    tauri::async_runtime::spawn_blocking(move || models.install(consent))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub fn search_model_cancel(state: State<'_, AppState>) {
    state.clip_models.cancel();
}

#[cfg(test)]
mod tests {
    use super::*;
    const SHA: Asset = Asset {
        path: "model",
        size: 3,
        digest: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        git_blob: false,
    };
    const GIT: Asset = Asset {
        path: "tokenizer",
        size: 3,
        digest: "f2ba8f84ab5c1bce84a7b441cb1959cfc7093b7f",
        git_blob: true,
    };
    #[test]
    fn exact_pins_reject_truncation_corruption_and_oversize() {
        for asset in [&SHA, &GIT] {
            assert!(copy_verified(&mut &b"abc"[..], &mut Vec::new(), asset).is_ok());
            for bytes in [&b"ab"[..], &b"abd"[..], &b"abcd"[..]] {
                assert!(copy_verified(&mut &bytes[..], &mut Vec::new(), asset).is_err());
            }
        }
        let mut input = &b"abcdef"[..];
        assert!(copy_verified(&mut input, &mut Vec::new(), &SHA).is_err());
        assert_eq!(input, b"ef");
    }
    #[test]
    fn denied_consent_has_no_filesystem_or_network_effects() {
        let dir = tempfile::tempdir().unwrap();
        let models = Models::new(dir.path().join("absent"));
        assert!(models.install(false).unwrap_err().contains("opt-in"));
        assert!(models.prepare_text("cat").is_err());
        assert!(!models.root.exists());
        assert!(!models.status().downloading);
        assert!(!models.status().verified);
    }
    #[test]
    fn failed_install_removes_staging_preserves_previous_package() {
        let dir = tempfile::tempdir().unwrap();
        let models = Models::new(dir.path().into());
        let old = dir.path().join(REVISION);
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("keep"), "previous").unwrap();
        let result = models.install_with(ASSETS, |_, path| {
            std::fs::write(path, "partial").unwrap();
            Err("interrupted".into())
        });
        assert_eq!(result.unwrap_err().to_string(), "interrupted");
        assert_eq!(
            std::fs::read_to_string(old.join("keep")).unwrap(),
            "previous"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn publishes_complete_package_and_reuses_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let models = Models::new(dir.path().into());
        let assets = [SHA, GIT];
        models
            .install_with(&assets, |_, path| {
                std::fs::write(path, "abc").map_err(|e| PackError::from(e.to_string()))
            })
            .unwrap();
        assert!(verify_package(&dir.path().join(REVISION), &assets).is_ok());
        models
            .install_with(&assets, |_, _| panic!("valid install must not fetch"))
            .unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn cancellation_blocks_streams_and_publication() {
        let dir = tempfile::tempdir().unwrap();
        let models = Models::new(dir.path().into());
        let result = models.install_with(&[SHA], |_, path| {
            std::fs::write(path, "abc").unwrap();
            models.cancel();
            Ok(())
        });
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert!(!dir.path().join(REVISION).exists());
        // Cancel keeps the files fetched so far (to Resume); nothing is published
        // and no staging folder from an old attempt is left
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec![format!(".partial-{REVISION}")]);
        let mut source = &b"abc"[..];
        assert!(copy_verified(
            &mut Cancellable {
                reader: &mut source,
                cancelled: models.store.cancel_flag()
            },
            &mut Vec::new(),
            &SHA
        )
        .is_err());
        assert_eq!(source, b"abc");
    }
    #[test]
    fn package_requires_every_matching_asset() {
        let dir = tempfile::tempdir().unwrap();
        let assets = [SHA, GIT];
        std::fs::write(dir.path().join("model"), "abc").unwrap();
        assert!(verify_package(dir.path(), &assets).is_err());
        std::fs::write(dir.path().join("tokenizer"), "abc").unwrap();
        assert!(verify_package(dir.path(), &assets).is_ok());
        std::fs::write(dir.path().join("model"), "abd").unwrap();
        assert!(verify_package(dir.path(), &assets).is_err());
    }

    #[test]
    fn pins_layout_and_source_are_those_of_the_first_release() {
        // the on-disk layout and every pinned file are what existing installs hold
        let paths: Vec<_> = ASSETS.iter().map(|a| a.path).collect();
        assert_eq!(paths, ["onnx/text_model.onnx", "onnx/vision_model.onnx", "tokenizer.json",
            "preprocessor_config.json", "tokenizer_config.json", "special_tokens_map.json", "config.json"]);
        assert_eq!(ASSETS[0].digest, "3f6571f5bad13a97c469c1622e1cfc4d9aef78b79fdbfcff804ca357bfada8cc");
        assert_eq!(ASSETS[1].digest, "fd6e1402a588279d1723c7534d4bcba5bc0b14b47dfab0e46f8c47b8270d7d40");
        assert_eq!(SPEC.bytes(), 607_974_672);
        assert_eq!(SPEC.base_url, format!("https://huggingface.co/{REPOSITORY}/resolve/{REVISION}"));
        let dir = tempfile::tempdir().unwrap();
        let models = Models::new(dir.path().into());
        assert_eq!(models.store().final_dir(), dir.path().join(REVISION));
    }
    #[test]
    fn an_existing_install_is_recognised_as_installed_and_keeps_working() {
        // files of exactly the pinned sizes (sparse, no weights), as an install by an earlier version leaves them
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(REVISION);
        for a in ASSETS {
            let f = root.join(a.path);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::File::create(&f).unwrap().set_len(a.size).unwrap();
        }
        let models = Models::new(dir.path().into());
        assert!(models.installed(), "the analysis service's test");
        let info = models.store().info();
        assert_eq!(info.state, crate::packs::PackState::Installed);
        assert_eq!((info.files, info.files_ok), (7, 7));
        assert!(!models.status().downloading);
        // nothing was fetched or written beside the pack
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "downloads pinned CLIP assets; requires explicit opt-in, engine, and CPU ONNX Runtime"]
    fn opt_in_real_cpu_text_and_vision_inference() {
        // Check both independent consent controls before inspecting model files,
        // creating the install directory, or allowing Models::install to fetch.
        let opt_in = std::env::var("IMMAGE_CLIP_MODEL_OPT_IN").unwrap_or_default();
        assert_eq!(opt_in, "1", "set IMMAGE_CLIP_MODEL_OPT_IN=1 to authorize the pinned model install/check");
        let model_dir = std::env::var_os("IMMAGE_CLIP_MODEL_DIR")
            .map(PathBuf::from)
            .expect("set IMMAGE_CLIP_MODEL_DIR to an absolute persistent model directory");
        assert!(model_dir.is_absolute(), "IMMAGE_CLIP_MODEL_DIR must be absolute");

        let engine_path = std::env::var_os("IMMAGE_TEST_ENGINE")
            .map(PathBuf::from)
            .expect("set IMMAGE_TEST_ENGINE to the absolute built darktable-engine executable");
        assert!(engine_path.is_absolute(), "IMMAGE_TEST_ENGINE must be absolute");
        assert!(engine_path.is_file(), "IMMAGE_TEST_ENGINE does not exist: {}", engine_path.display());
        let ort_path = std::env::var_os("IMMAGE_TEST_ORT").map(PathBuf::from);
        if let Some(path) = &ort_path {
            assert!(path.is_absolute(), "IMMAGE_TEST_ORT must be absolute when set");
            assert!(path.is_file(), "IMMAGE_TEST_ORT does not exist: {}", path.display());
        }

        let models = Models::new(model_dir);
        models.install(true).expect("verify or install pinned CLIP package");
        let known = models.prepare_text("a photo of a cat").unwrap().1;
        assert_eq!(known.ids[..7], [49406, 320, 1125, 539, 320, 2368, 49407]);
        assert_eq!(known.attention.iter().sum::<i64>(), 7);

        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        // The test-only wrapper passes ORT's documented darktable preference
        // on the command line; Engine writes it only into this temporary configdir.
        let bin = if let Some(ort) = ort_path.as_ref() {
            use std::os::unix::fs::PermissionsExt;
            fn quote(path: &Path) -> String {
                format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
            }
            let wrapper = runtime.join("engine-with-test-ort");
            let script = format!(
                "#!/bin/sh\nexec {} \"$@\" --conf {}\n",
                quote(&engine_path),
                quote(Path::new(&format!("plugins/ai/ort_library_path={}", ort.display())))
            );
            std::fs::write(&wrapper, script).unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
            wrapper
        } else {
            engine_path
        };

        let mut engine = crate::engine::Engine::spawn(&bin, &runtime, &runtime)
            .expect("start isolated built engine");
        let tagged = engine.tagged();
        let timed = |run: &mut dyn FnMut() -> Vec<f32>| {
            let started = std::time::Instant::now();
            let values = run();
            (values, started.elapsed().as_secs_f64() * 1000.0)
        };
        let query = "a photo of a cat";
        let (text_first, text_cold_ms) = timed(&mut || {
            crate::clip_infer::text(&models, &tagged, &runtime, query).unwrap()
        });
        let (text_warm, text_warm_ms) = timed(&mut || {
            crate::clip_infer::text(&models, &tagged, &runtime, query).unwrap()
        });
        let photo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../app/sample/stairs-to-mountain.jpg");
        let rgb = image::ImageReader::open(photo)
            .unwrap()
            .decode()
            .unwrap()
            .to_rgb8();
        let (image_first, image_cold_ms) = timed(&mut || {
            crate::clip_infer::image(
                &models,
                &tagged,
                &runtime,
                rgb.as_raw(),
                rgb.width() as usize,
                rgb.height() as usize,
            )
            .unwrap()
        });
        let (image_warm, image_warm_ms) = timed(&mut || {
            crate::clip_infer::image(
                &models,
                &tagged,
                &runtime,
                rgb.as_raw(),
                rgb.width() as usize,
                rgb.height() as usize,
            )
            .unwrap()
        });
        let relevant_captions = [
            "a photo of snowy mountain peaks above a rocky hiking trail and purple wildflowers",
            "a mountain landscape with a stone path through pink flowers",
        ];
        let unrelated_captions = [
            "a close-up photo of sushi on a plate at a restaurant",
            "a brown dog swimming in a blue pool",
        ];
        let mut caption_scores = Vec::new();
        for caption in relevant_captions.iter().chain(unrelated_captions.iter()) {
            let embedding = crate::clip_infer::text(&models, &tagged, &runtime, caption).unwrap();
            let dot = image_first.iter().zip(&embedding)
                .map(|(image, text)| f64::from(*image) * f64::from(*text))
                .sum::<f64>();
            let image_norm = image_first.iter().map(|value| f64::from(*value).powi(2)).sum::<f64>().sqrt();
            let text_norm = embedding.iter().map(|value| f64::from(*value).powi(2)).sum::<f64>().sqrt();
            let cosine = dot / (image_norm * text_norm);
            caption_scores.push((caption, cosine));
        }
        let relevant_floor = caption_scores[..relevant_captions.len()].iter()
            .map(|(_, score)| *score).fold(f64::INFINITY, f64::min);
        let unrelated_ceiling = caption_scores[relevant_captions.len()..].iter()
            .map(|(_, score)| *score).fold(f64::NEG_INFINITY, f64::max);
        assert!(relevant_floor > unrelated_ceiling,
            "relevant captions must rank above unrelated ones: {caption_scores:?}");
        engine.quit();

        for embedding in [&text_first, &text_warm, &image_first, &image_warm] {
            assert_eq!(embedding.len(), 512);
            assert!(embedding.iter().all(|v| v.is_finite()));
            let norm = embedding.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
            assert!((norm - 1.0).abs() <= 1e-4, "projection is not normalized: {norm}");
        }
        println!(
            "REAL_CLIP_CPU_SMOKE {}",
            serde_json::json!({
                "text": "a photo of a cat",
                "textDimensions": text_first.len(),
                "imageDimensions": image_first.len(),
                "textColdMs": text_cold_ms,
                "textWarmMs": text_warm_ms,
                "imageColdMs": image_cold_ms,
                "imageWarmMs": image_warm_ms,
                "captionScores": caption_scores.iter().map(|(caption, score)| (*caption, *score)).collect::<Vec<_>>(),
                "relevantCaptionFloor": relevant_floor,
                "unrelatedCaptionCeiling": unrelated_ceiling,
                "provider": "CPU",
                "semanticQualityValidated": "single-sample ranking smoke only",
                "osCachesCleared": false,
                "modelDirectory": models.root,
                "onnxRuntimeOverride": ort_path.as_ref().map(|p| p.display().to_string()),
            })
        );
    }
}

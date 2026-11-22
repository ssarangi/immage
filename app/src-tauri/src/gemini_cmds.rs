// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Gemini commands (gemini.rs): the key in the keychain, a photo critique and
//! natural-language search. The key never reaches the page: `gemini_key_set`
//! takes it once and `gemini_key_status` only says whether one is saved.

use crate::gemini::{self, CritiqueRegion, GeminiSearch, Identity, Store};
use crate::{frame_output, AppState};
use serde::Serialize;
use serde_json::json;
use tauri::State;

async fn worker<T: Send + 'static>(run: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(run).await.map_err(|_| "The Gemini operation failed".to_string())?
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiStatus {
    /// a key is saved in the system keychain
    has_key: bool,
    /// the keychain could not be read
    keychain_error: bool,
    /// Preferences > Culling's AI grading switch
    enabled: bool,
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiCritique {
    pub imgid: i64,
    pub model: String,
    /// a verdict of a few words
    pub headline: String,
    pub overall_summary: String,
    pub regions: Vec<CritiqueRegion>,
    /// answered from the cache, with no upload
    pub cached: bool,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn gemini_key_status(state: State<'_, AppState>) -> Result<GeminiStatus, String> {
    let runtime = state.runtime.clone();
    worker(move || {
        let key = gemini::stored_key();
        Ok(GeminiStatus {
            has_key: matches!(key, Ok(Some(_))),
            keychain_error: key.is_err(),
            enabled: gemini::enabled(&runtime),
        })
    })
    .await
}

/// Saves the key in the keychain. It is not echoed back or logged.
#[tauri::command]
#[specta::specta]
pub(crate) async fn gemini_key_set(key: String) -> Result<(), String> {
    worker(move || {
        let key = gemini::clean_key(&key)?;
        gemini::entry()?.set_password(&key).map_err(|_| "Could not save the key in the system keychain".to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn gemini_key_clear() -> Result<(), String> {
    worker(|| match gemini::entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Could not remove the key from the system keychain".to_string()),
    })
    .await
}

/// Tests the typed key, or the saved one when `key` is None.
#[tauri::command]
#[specta::specta]
pub(crate) async fn gemini_key_test(key: Option<String>) -> Result<(), String> {
    worker(move || {
        let key = match key.filter(|k| !k.trim().is_empty()) {
            Some(key) => key,
            None => gemini::stored_key()?.ok_or("Add a Gemini API key first")?,
        };
        gemini::test_key(gemini::BASE_URL, &key)
    })
    .await
}

fn require_key(runtime: &std::path::Path) -> Result<String, String> {
    // a click on Critique or Ask Gemini is the user's own request: only a saved
    // key is needed. The automatic-grading switch is for grading that runs unasked
    let _ = runtime;
    gemini::stored_key()?.ok_or_else(|| "Add a Gemini API key in Preferences > Culling first".to_string())
}

/// Critiques a photo, from the cache unless `force`. A miss uploads one JPEG of
/// the photo (long edge 1024) to Google. `path` is the photo's file, which
/// identifies it in the cache; `imgid` is its id in the engine's library.
#[tauri::command]
#[specta::specta]
pub(crate) async fn gemini_critique(
    state: State<'_, AppState>,
    imgid: i64,
    path: String,
    force: Option<bool>,
) -> Result<GeminiCritique, String> {
    let runtime = state.runtime.clone();
    let (key, id) = {
        let runtime = runtime.clone();
        let path = path.clone();
        worker(move || Ok((require_key(&runtime)?, Identity::of(&path)?))).await?
    };
    let store_file = runtime.join("gemini-critiques.sqlite");
    if !force.unwrap_or(false) {
        let (store_file, id) = (store_file.clone(), id.clone());
        let hit = worker(move || Ok(Store::open(&store_file).ok().and_then(|s| s.get(&id)))).await?;
        if let Some((model, body)) = hit {
            return Ok(GeminiCritique { imgid, model, headline: body.headline, overall_summary: body.overall_summary, regions: body.regions, cached: true });
        }
    }
    // the photo as the library shows it, from the engine's own preview path
    let thumbs = state.tagged.clone();
    let tag = thumbs.tag();
    let storage = state._frame_output.clone();
    let out = storage.path(&format!("gemini-{tag}.rgba"));
    let (reply, pixels) = worker(move || {
        let got = thumbs
            .request(tag, json!({ "cmd": "thumb", "imgid": imgid, "size": gemini::MAX_EDGE, "proof": false,
                                  "foreground": true, "prefer_source": true, "camera_preview": false, "out": out }))
            .and_then(|reply| Ok((reply, frame_output::FrameOutput::read(&out).map_err(|e| format!("preview: {e}"))?)));
        storage.remove(&out);
        got
    })
    .await?;
    let (model, body) = worker(move || {
        let (width, height, rgba) = if reply.get("encoding").and_then(|v| v.as_str()) == Some("jpeg") {
            let image = image::load_from_memory(&pixels).map_err(|e| format!("preview: {e}"))?.to_rgba8();
            (image.width(), image.height(), image.into_raw())
        } else {
            let width = reply["width"].as_u64().ok_or("preview: unexpected reply")? as u32;
            let height = reply["height"].as_u64().ok_or("preview: unexpected reply")? as u32;
            (width, height, pixels)
        };
        let jpeg = gemini::prepare_jpeg(rgba, width, height)?;
        let (model, body) = gemini::critique(gemini::BASE_URL, &key, &jpeg)?;
        Ok((model, body))
    })
    .await?;
    {
        let (store_file, id, model, body) = (store_file, id, model.clone(), body.clone());
        // a cache that cannot be written costs only the next request
        let _ = worker(move || Store::open(&store_file)?.put(&id, &model, &body)).await;
    }
    Ok(GeminiCritique { imgid, model, headline: body.headline, overall_summary: body.overall_summary, regions: body.regions, cached: false })
}

/// Turns a sentence into Find filters. Only the sentence and today's date are
/// sent; no photo names, paths or catalog contents.
#[tauri::command]
#[specta::specta]
pub(crate) async fn gemini_search(state: State<'_, AppState>, query: String) -> Result<GeminiSearch, String> {
    let runtime = state.runtime.clone();
    worker(move || {
        let key = require_key(&runtime)?;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        gemini::search(gemini::BASE_URL, &key, &query, &today)
    })
    .await
}

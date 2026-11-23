// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Auto-keywords commands of Preferences > General > AI models and Find: the confidence
//! setting, what the analysis has found, the review list and the bulk removal of the
//! keywords it attached. The switch itself is `analysis_set` (pack_cmds.rs).
use crate::pack_cmds::AiState;
use crate::ram_models::{MAX_THRESHOLD, MIN_THRESHOLD};
use crate::settings::Settings;
use crate::tags_store::{PhotoKeywordState, TagCount};
use crate::AppState;
use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(serde::Serialize, specta::Type)]
pub struct KeywordsSummary {
    /// Photos with a stored result.
    pub photos: u32,
    /// Photos carrying at least one keyword the analysis attached.
    pub keyworded: u32,
    /// Keywords the analysis attached, over all photos.
    pub keywords: u32,
    /// The confidence setting, percent.
    pub threshold: u8,
    pub min_threshold: u8,
    pub max_threshold: u8,
    /// Inference time of the last photo, milliseconds, once one has been analysed.
    pub infer_ms: Option<u32>,
    /// The engine's memory while it held the model, megabytes, once measured.
    pub memory_mb: Option<u32>,
    /// Photos share a burst mate's tags instead of being analysed (setting).
    pub burst: bool,
    /// Photos that carry tags copied from a burst mate.
    pub copied: u32,
    /// Why photos wait without being analysed: "level" (the Memory level cannot hold the model)
    /// or "free" (too little memory is free now).
    pub blocked: Option<String>,
    /// Whether analysis works: "ok", "retrying" (a failure, waiting to try again) or "blocked"
    /// (it failed repeatedly and stays stopped until Retry).
    pub health: String,
    /// Why, once not ok: "model" (the model or the AI runtime cannot load), "engine" (the helper
    /// cannot start), "nocpu" (this build has no CPU AI support), "crash" (the helper keeps
    /// stopping) or "other".
    pub failure: Option<String>,
    /// Seconds until the next try while retrying.
    pub retry_in_s: Option<u32>,
    /// The engine's own words for the failure, for a tooltip.
    pub failure_detail: Option<String>,
    /// Photos left alone after failing on their own (unreadable pictures).
    pub failed: u32,
}

#[tauri::command]
#[specta::specta]
pub fn keywords_summary(ai: State<'_, AiState>) -> Result<KeywordsSummary, String> {
    let totals = ai.tags.totals()?;
    let (infer_ms, kb) = ai.tags_service.speed();
    let health = ai.tags_service.health();
    Ok(KeywordsSummary {
        photos: totals.photos,
        keyworded: totals.keyworded,
        keywords: totals.applied,
        threshold: ai.keywords.threshold(),
        min_threshold: MIN_THRESHOLD,
        max_threshold: MAX_THRESHOLD,
        infer_ms,
        memory_mb: kb.map(|k| (k / 1024) as u32),
        burst: ai.keywords.burst(),
        copied: ai.tags.propagated_count()?,
        blocked: match ai.tags_service.blocked() {
            crate::tags_service::BLOCK_LEVEL => Some("level".into()),
            crate::tags_service::BLOCK_FREE => Some("free".into()),
            _ => None,
        },
        health: health.state.into(),
        failure: health.reason.map(Into::into),
        retry_in_s: health.retry_in_s,
        failure_detail: (!health.detail.is_empty()).then_some(health.detail),
        failed: ai.tags_service.failed(),
    })
}

/// Retry after Auto-keywords stopped on a failure: forgets the failures and tries again at once.
/// Fixing the cause (the AI runtime, the model's files via Check files) is the user's.
#[tauri::command]
#[specta::specta]
pub fn keywords_retry(ai: State<'_, AiState>) {
    ai.tags_service.retry();
}

/// Sets the confidence a tag needs to become a keyword. Stored results are re-decided in
/// the background without analysing again.
#[tauri::command]
#[specta::specta]
pub fn keywords_threshold_set(ai: State<'_, AiState>, settings: State<'_, Settings>, percent: u8) -> Result<u8, String> {
    let pct = percent.clamp(MIN_THRESHOLD, MAX_THRESHOLD);
    settings.set(crate::analysis_settings::KEYWORDS_THRESHOLD_KEY, &pct.to_string())?;
    ai.keywords.threshold_flag().store(pct, Ordering::Release);
    Ok(pct)
}

/// Turns burst sharing on or off: with it on, a photo that looks like one already analysed in its folder
/// takes that photo's tags instead of being analysed itself.
#[tauri::command]
#[specta::specta]
pub fn keywords_burst_set(ai: State<'_, AiState>, settings: State<'_, Settings>, on: bool) -> Result<bool, String> {
    settings.set(crate::analysis_settings::KEYWORDS_BURST_KEY, if on { "1" } else { "0" })?;
    ai.keywords.burst_flag().store(on, Ordering::Release);
    Ok(on)
}

/// The keywords the analysis attached, most used first: the review list and Find's AI tags.
#[tauri::command]
#[specta::specta]
pub fn keywords_ai_list(ai: State<'_, AiState>, limit: u32) -> Result<Vec<TagCount>, String> {
    ai.tags.applied_counts(limit.clamp(1, 5000))
}

/// The keywords the analysis attached to one photo (Keywording marks them AI).
#[tauri::command]
#[specta::specta]
pub fn keywords_ai_of(ai: State<'_, AiState>, path: String) -> Result<Vec<String>, String> {
    ai.tags.applied_for(&path)
}

/// The analysis's state for each of these photos (the open one, or a selection): whether it has a
/// result and its AI keywords with their confidence. The page combines it with the library-wide
/// `analysis_status` to say "analysing" or "nothing found".
#[tauri::command]
#[specta::specta]
pub fn keywords_photo_states(ai: State<'_, AiState>, paths: Vec<String>) -> Result<Vec<PhotoKeywordState>, String> {
    paths.iter().take(2000).map(|p| ai.tags.photo_state(p)).collect()
}

/// `keywords_photo_states` for one photo.
#[tauri::command]
#[specta::specta]
pub fn keywords_photo_state(ai: State<'_, AiState>, path: String) -> Result<PhotoKeywordState, String> {
    ai.tags.photo_state(&path)
}

/// Keep: the named AI keywords of these photos (all of each photo's when `names` is empty) become the
/// user's own. They stay in the sidecar; only Immage's record of having added them changes.
#[tauri::command]
#[specta::specta]
pub fn keywords_ai_keep(ai: State<'_, AiState>, paths: Vec<String>, names: Vec<String>) -> Result<u32, String> {
    let mut n = 0;
    for p in &paths {
        n += ai.tags.keep_applied(p, &names)?;
    }
    Ok(n)
}

/// Takes the named AI keywords (all when `names` is empty) off these photos only, not the library.
/// The user's own keywords are never touched, and a removed keyword is not added to the photo again.
#[tauri::command]
#[specta::specta]
pub async fn keywords_ai_remove_photos(
    state: State<'_, AppState>,
    ai: State<'_, AiState>,
    paths: Vec<String>,
    names: Vec<String>,
) -> Result<RemoveReport, String> {
    let (store, tagged, apply) = (ai.tags.clone(), state.tagged.clone(), ai.apply.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let photos = store.applied_photos_in(&paths, &names)?;
        detach(&store, &tagged, &apply, photos)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize, specta::Type)]
pub struct RemoveReport {
    pub photos: u32,
    pub keywords: u32,
    /// Photos that could not be reached (missing, offline); their keywords are kept and still listed.
    pub failed: u32,
}

/// Takes off the keywords the analysis attached: the named ones, or all when `names` is empty.
/// Keywords the user typed, or had before the analysis, are never touched: only what the
/// store recorded as attached by the analysis is detached.
#[tauri::command]
#[specta::specta]
pub async fn keywords_ai_remove(
    state: State<'_, AppState>,
    ai: State<'_, AiState>,
    names: Vec<String>,
) -> Result<RemoveReport, String> {
    let (store, tagged, apply) = (ai.tags.clone(), state.tagged.clone(), ai.apply.clone());
    tauri::async_runtime::spawn_blocking(move || remove_applied(&store, &tagged, &apply, &names))
        .await
        .map_err(|e| e.to_string())?
}

pub fn remove_applied(
    store: &crate::tags_store::TagStore,
    tagged: &crate::engine::Tagged,
    apply: &crate::tags_service::Applier,
    names: &[String],
) -> Result<RemoveReport, String> {
    let photos = store.applied_photos(names, u32::MAX)?;
    detach(store, tagged, apply, photos)
}

fn detach(
    store: &crate::tags_store::TagStore,
    tagged: &crate::engine::Tagged,
    apply: &crate::tags_service::Applier,
    photos: Vec<(String, Vec<String>)>,
) -> Result<RemoveReport, String> {
    let mut report = RemoveReport { photos: 0, keywords: 0, failed: 0 };
    for (path, tags) in photos {
        if !std::path::Path::new(&path).is_file() {
            report.failed += 1;
            continue;
        }
        let imgid = tagged.cached_photo(&path, 0);
        match (apply)(json!({"cmd":"tags_apply","imgid":imgid,"add":[],"remove":tags})) {
            Ok(reply) => {
                // what the engine detached, or what was already gone, is no longer Immage's
                store.forget_applied(&path, &tags)?;
                report.photos += 1;
                report.keywords += reply["removed"].as_array().map_or(0, |a| a.len() as u32);
            }
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Auto-keywords commands of Preferences > General > AI models and Find: the confidence
//! setting, what the analysis has found, the review list and the bulk removal of the
//! keywords it attached. The switch itself is `analysis_set` (pack_cmds.rs).
use crate::pack_cmds::AiState;
use crate::ram_models::{MAX_THRESHOLD, MIN_THRESHOLD};
use crate::settings::Settings;
use crate::tags_store::TagCount;
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
}

#[tauri::command]
#[specta::specta]
pub fn keywords_summary(ai: State<'_, AiState>) -> Result<KeywordsSummary, String> {
    let totals = ai.tags.totals()?;
    let (infer_ms, kb) = ai.tags_service.speed();
    Ok(KeywordsSummary {
        photos: totals.photos,
        keyworded: totals.keyworded,
        keywords: totals.applied,
        threshold: ai.keywords.threshold(),
        min_threshold: MIN_THRESHOLD,
        max_threshold: MAX_THRESHOLD,
        infer_ms,
        memory_mb: kb.map(|k| (k / 1024) as u32),
    })
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

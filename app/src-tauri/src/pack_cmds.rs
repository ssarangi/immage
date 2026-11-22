// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The model pack commands (Preferences > General > AI models). Every download
//! starts from `pack_download`, which the page calls only from the consent
//! dialog, Resume or Repair; nothing here runs at start-up except reading
//! state.
use crate::analysis_settings::{self as ai, AnalysisId, AnalysisSettings};
use crate::packs::{PackCheck, PackId, PackInfo, PackManager, PackState};
use crate::settings::Settings;
use crate::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;

pub struct AiState {
    pub packs: Arc<PackManager>,
    /// The Smart search analysis switch; the analysis service reads the same flag.
    pub smart_search: Arc<AtomicBool>,
}

fn with_consents(mut info: PackInfo, settings: &Settings) -> PackInfo {
    info.download_consent = settings.get(ai::download_consent_key(info.id)).is_some();
    info.analysis_consent = info.analyses.iter().all(|a| settings.get(ai::analysis_consent_key(*a)).is_some());
    info
}
fn store(ai: &AiState, id: PackId) -> Result<&Arc<crate::packs::PackStore>, String> {
    ai.packs.get(id).ok_or_else(|| "Unknown model pack".to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn pack_list(ai: State<'_, AiState>, settings: State<'_, Settings>) -> Result<Vec<PackInfo>, String> {
    let packs = ai.packs.clone();
    let list = tauri::async_runtime::spawn_blocking(move || {
        for p in packs.stores() {
            p.ensure_checked();
        }
        packs.list()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(list.into_iter().map(|i| with_consents(i, &settings)).collect())
}

/// Starts (or resumes, or repairs) a download. `consent` is true only from the
/// consent dialog, which records it; Resume and Repair pass false and are
/// accepted only for a pack the user already consented to.
#[tauri::command]
#[specta::specta]
pub async fn pack_download(
    ai: State<'_, AiState>,
    settings: State<'_, Settings>,
    id: PackId,
    consent: bool,
) -> Result<PackInfo, String> {
    let s = store(&ai, id)?.clone();
    let key = crate::analysis_settings::download_consent_key(id);
    if consent {
        crate::analysis_settings::record_download_consent(&settings, id)?;
    } else if settings.get(key).is_none() {
        return Err("Downloading a model requires your consent in the Download dialog".into());
    }
    s.start()?;
    Ok(with_consents(s.info(), &settings))
}

#[tauri::command]
#[specta::specta]
pub fn pack_cancel(ai: State<'_, AiState>, id: PackId) -> Result<(), String> {
    store(&ai, id)?.cancel();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn pack_verify(ai: State<'_, AiState>, id: PackId) -> Result<PackCheck, String> {
    let s = store(&ai, id)?.clone();
    tauri::async_runtime::spawn_blocking(move || s.check_files())
        .await
        .map_err(|e| e.to_string())?
}

/// Removes the installed model and its kept partial files (or only the partial
/// files). Removing an installed model turns its analyses off; the results
/// stay unless `keep_results` is false.
#[tauri::command]
#[specta::specta]
pub async fn pack_remove(
    ai: State<'_, AiState>,
    state: State<'_, AppState>,
    settings: State<'_, Settings>,
    id: PackId,
    keep_results: bool,
    partial_only: bool,
) -> Result<PackInfo, String> {
    let s = store(&ai, id)?.clone();
    let analyses = s.spec().analyses;
    {
        let s = s.clone();
        tauri::async_runtime::spawn_blocking(move || s.remove(partial_only))
            .await
            .map_err(|e| e.to_string())??;
    }
    if !partial_only {
        for a in analyses {
            settings.set(ai::enabled_key(*a), "0")?;
            if *a == AnalysisId::SmartSearch {
                ai.smart_search.store(false, Ordering::Release);
            }
        }
        state.analysis_queue.cancel();
        if !keep_results {
            if let Some(cache) = &state.search_cache {
                cache.clear_analyses()?;
            }
        }
    }
    Ok(with_consents(s.info(), &settings))
}

#[tauri::command]
#[specta::specta]
pub fn pack_license(ai: State<'_, AiState>, id: PackId) -> Result<String, String> {
    Ok(store(&ai, id)?.spec().license_text.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn analysis_get(settings: State<'_, Settings>) -> Vec<AnalysisSettings> {
    vec![AnalysisSettings {
        id: AnalysisId::SmartSearch,
        enabled: settings.get(ai::enabled_key(AnalysisId::SmartSearch)).as_deref() == Some("1"),
    }]
}

/// Turns an analysis on or off. Turning one on is refused in Rust unless its
/// pack is installed and its files are whole (the page also disables the switch).
#[tauri::command]
#[specta::specta]
pub async fn analysis_set(
    ai: State<'_, AiState>,
    state: State<'_, AppState>,
    settings: State<'_, Settings>,
    setting: AnalysisSettings,
) -> Result<Vec<AnalysisSettings>, String> {
    let AnalysisSettings { id, enabled } = setting;
    if enabled {
        let pack = ai
            .packs
            .stores()
            .iter()
            .find(|p| p.spec().analyses.contains(&id))
            .ok_or_else(|| "Unknown analysis".to_string())?;
        if pack.info().state != PackState::Installed {
            return Err("Install the model before turning this on".into());
        }
        ai::record_analysis_consent(&settings, id)?;
    }
    settings.set(ai::enabled_key(id), if enabled { "1" } else { "0" })?;
    match id {
        AnalysisId::SmartSearch => ai.smart_search.store(enabled, Ordering::Release),
    }
    if !enabled {
        // queued photos are dropped; the one in flight finishes and is kept
        state.analysis_queue.cancel();
    }
    Ok(analysis_get(settings))
}

#[derive(serde::Serialize, specta::Type)]
pub struct AnalysisRow {
    pub id: AnalysisId,
    pub enabled: bool,
    pub installed: bool,
    pub pending: Option<u32>,
    pub running: u32,
    pub active: bool,
}
#[derive(serde::Serialize, specta::Type)]
pub struct AnalysisStatusAll {
    pub paused: bool,
    pub rows: Vec<AnalysisRow>,
}

/// Memory only, like `search_analysis_status`, which it generalises: the same
/// 2 s poll feeds the summary strip, the CLIP row and the Find icon.
#[tauri::command]
#[specta::specta]
pub fn analysis_status(state: State<'_, AppState>) -> AnalysisStatusAll {
    let paused = state.analysis_queue.status().paused;
    let p = state.search_analysis.status(paused);
    AnalysisStatusAll {
        paused,
        rows: vec![AnalysisRow {
            id: AnalysisId::SmartSearch,
            enabled: p.enabled,
            installed: p.installed,
            pending: p.pending.map(u64::from).map(|n| n as u32),
            running: p.running,
            active: p.active,
        }],
    }
}

/// The same pause as `search_pause` and the Find icon.
#[tauri::command]
#[specta::specta]
pub async fn analysis_pause(
    state: State<'_, AppState>,
    preferences: State<'_, Settings>,
    paused: bool,
) -> Result<(), String> {
    crate::search_cmds::search_pause(state, preferences, paused).await
}

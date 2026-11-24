// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The per-analysis switches (Preferences > General > AI models) and the
//! consents behind them, kept in settings.json under `ai.*`. A switch is off
//! until the user turns it on, except for Smart search (CLIP) analysis, which
//! is on for a user who already had the model installed when the switches were
//! introduced, so their library keeps being analysed as before.
//!
//! Keys (strings, like every settings value):
//!   ai.smart_search.enabled           "1" | "0"
//!   ai.smart_search.analysis_consent  unix seconds, or "migrated"
//!   ai.keywords.enabled               "1" | "0" (always off until the user turns it on)
//!   ai.keywords.analysis_consent      unix seconds
//!   ai.faces.enabled                  "1" | "0" (always off until the user turns it on)
//!   ai.faces.analysis_consent         unix seconds
//!   ai.keywords.threshold             percent, 40..=95 (70 when absent)
//!   ai.<pack>.download_consent        unix seconds, or "migrated"
use crate::settings::Settings;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AnalysisId {
    SmartSearch,
    Keywords,
    Faces,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct AnalysisSettings {
    pub id: AnalysisId,
    pub enabled: bool,
}

pub fn enabled_key(id: AnalysisId) -> &'static str {
    match id {
        AnalysisId::SmartSearch => "ai.smart_search.enabled",
        AnalysisId::Keywords => "ai.keywords.enabled",
        AnalysisId::Faces => "ai.faces.enabled",
    }
}
pub fn analysis_consent_key(id: AnalysisId) -> &'static str {
    match id {
        AnalysisId::SmartSearch => "ai.smart_search.analysis_consent",
        AnalysisId::Keywords => "ai.keywords.analysis_consent",
        AnalysisId::Faces => "ai.faces.analysis_consent",
    }
}
pub fn download_consent_key(pack: crate::packs::PackId) -> &'static str {
    match pack {
        crate::packs::PackId::Clip => "ai.clip.download_consent",
        crate::packs::PackId::Ram => "ai.ram.download_consent",
        crate::packs::PackId::Faces => "ai.faces.download_consent",
    }
}
fn stamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}
pub fn record_download_consent(settings: &Settings, pack: crate::packs::PackId) -> Result<(), String> {
    settings.set(download_consent_key(pack), &stamp())
}
pub fn record_analysis_consent(settings: &Settings, id: AnalysisId) -> Result<(), String> {
    settings.set(analysis_consent_key(id), &stamp())
}

/// First start with the switches: an existing CLIP install keeps analysing
/// (and counts as consented), a new user starts with everything off. Returns
/// the Smart search switch. Idempotent: a stored value always wins.
pub fn load_smart_search(settings: &Settings, clip_installed: bool) -> bool {
    let download = download_consent_key(crate::packs::PackId::Clip);
    if clip_installed && settings.get(download).is_none() {
        let _ = settings.set(download, "migrated");
    }
    match settings.get(enabled_key(AnalysisId::SmartSearch)) {
        Some(v) => v == "1",
        None => {
            let _ = settings.set(enabled_key(AnalysisId::SmartSearch), if clip_installed { "1" } else { "0" });
            if clip_installed {
                let _ = settings.set(analysis_consent_key(AnalysisId::SmartSearch), "migrated");
            }
            clip_installed
        }
    }
}

pub const KEYWORDS_THRESHOLD_KEY: &str = "ai.keywords.threshold";
pub const KEYWORDS_BURST_KEY: &str = "ai.keywords.burst";

/// Whether a photo that looks like one already analysed in its folder takes its tags: on unless turned off.
pub fn load_keywords_burst(settings: &Settings) -> bool {
    settings.get(KEYWORDS_BURST_KEY).as_deref() != Some("0")
}

/// The Auto-keywords switch: off until the user turns it on, whatever else is stored.
pub fn load_keywords(settings: &Settings) -> bool {
    settings.get(enabled_key(AnalysisId::Keywords)).as_deref() == Some("1")
}
/// The Faces switch: off until the user turns it on, whatever else is stored (embeddings are biometric).
pub fn load_faces(settings: &Settings) -> bool {
    settings.get(enabled_key(AnalysisId::Faces)).as_deref() == Some("1")
}
/// The confidence setting in percent, kept inside the range the page offers.
pub fn load_keywords_threshold(settings: &Settings) -> u8 {
    use crate::ram_models::{DEFAULT_THRESHOLD, MAX_THRESHOLD, MIN_THRESHOLD};
    settings
        .get(KEYWORDS_THRESHOLD_KEY)
        .and_then(|v| v.parse::<u8>().ok())
        .map_or(DEFAULT_THRESHOLD, |n| n.clamp(MIN_THRESHOLD, MAX_THRESHOLD))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> (tempfile::TempDir, Settings) {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::load(dir.path());
        (dir, s)
    }
    #[test]
    fn existing_clip_users_keep_analysis_on() {
        let (dir, s) = settings();
        assert!(load_smart_search(&s, true));
        assert_eq!(s.get("ai.smart_search.enabled").as_deref(), Some("1"));
        assert_eq!(s.get("ai.clip.download_consent").as_deref(), Some("migrated"));
        assert_eq!(s.get("ai.smart_search.analysis_consent").as_deref(), Some("migrated"));
        // persisted, and a later load reads it back
        assert!(load_smart_search(&Settings::load(dir.path()), false));
    }
    #[test]
    fn new_users_start_with_everything_off() {
        let (_dir, s) = settings();
        assert!(!load_smart_search(&s, false));
        assert_eq!(s.get("ai.smart_search.enabled").as_deref(), Some("0"));
        assert!(s.get("ai.clip.download_consent").is_none());
        assert!(s.get("ai.smart_search.analysis_consent").is_none());
    }
    #[test]
    fn a_stored_choice_wins_over_the_default() {
        let (_dir, s) = settings();
        s.set("ai.smart_search.enabled", "0").unwrap();
        assert!(!load_smart_search(&s, true));
        // a user who turned it on later stays on after the model is removed and reinstalled
        s.set("ai.smart_search.enabled", "1").unwrap();
        assert!(load_smart_search(&s, false));
    }
    #[test]
    fn keywords_stay_off_until_turned_on_and_the_threshold_is_clamped() {
        let (_dir, s) = settings();
        assert!(!load_keywords(&s));
        assert_eq!(load_keywords_threshold(&s), 70);
        s.set("ai.keywords.enabled", "1").unwrap();
        assert!(load_keywords(&s));
        s.set(KEYWORDS_THRESHOLD_KEY, "5").unwrap();
        assert_eq!(load_keywords_threshold(&s), 40);
        s.set(KEYWORDS_THRESHOLD_KEY, "200").unwrap();
        assert_eq!(load_keywords_threshold(&s), 95);
        s.set(KEYWORDS_THRESHOLD_KEY, "junk").unwrap();
        assert_eq!(load_keywords_threshold(&s), 70);
        assert!(load_keywords_burst(&s), "burst sharing is on unless turned off");
        s.set(KEYWORDS_BURST_KEY, "0").unwrap();
        assert!(!load_keywords_burst(&s));
        s.set(KEYWORDS_THRESHOLD_KEY, "85").unwrap();
        assert_eq!(load_keywords_threshold(&s), 85);
    }
}

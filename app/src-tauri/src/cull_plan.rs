// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The auto-cull planner, ported from the RapidRAW fork's auto_cull.rs: from
//! each photo's analysis it decides groups, keepers, rejects (with a reason
//! and plain decision factors) and highlights. Pure: no files, engine or
//! database, so it is tested with hand-made analyses. Nothing here changes a
//! photo; `cull_cmds` applies a plan through the library's own rating and
//! reject paths.
use crate::cull::{self, CullAnalysis, CullBlurRegion};
use image_hasher::ImageHash;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::path::Path;

/// Bumped when the analysis changes what it stores, so cached numbers from an
/// older build are measured again.
pub const FEATURE_SET_VERSION: &str = "culling-v2-immage";

/// How many frames of each duplicate set stay: More keeps the top 35 %
/// (at least one), Moderate the top 20 %, Less only the best.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub enum DuplicateRatio {
    More,
    Moderate,
    Less,
}

impl DuplicateRatio {
    fn share(self) -> Option<f64> {
        match self {
            DuplicateRatio::More => Some(0.35),
            DuplicateRatio::Moderate => Some(0.20),
            DuplicateRatio::Less => None,
        }
    }

    /// how many of a set of `n` stay
    pub fn keepers(self, n: usize) -> usize {
        match self.share() {
            Some(share) => ((n as f64 * share).ceil() as usize).clamp(1, n.max(1)),
            None => 1,
        }
    }
}

fn default_similarity() -> u32 {
    28
}
fn default_blur() -> f64 {
    100.0
}
fn default_true() -> bool {
    true
}
fn default_ratio() -> DuplicateRatio {
    DuplicateRatio::Moderate
}
fn default_highlights() -> u8 {
    15
}
fn default_shoot() -> String {
    "general".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullSettings {
    /// "general", "portrait" or "wildlife". Kept with the session; the
    /// technical pass is the same for every type until subject detection exists.
    #[serde(default = "default_shoot")]
    pub shoot_type: String,
    /// perceptual-hash distance (of 512 bits) up to which two frames are one set
    #[serde(default = "default_similarity")]
    pub similarity_threshold: u32,
    /// sharpness (Laplacian variance) below which a single frame is blurry
    #[serde(default = "default_blur")]
    pub blur_threshold: f64,
    #[serde(default = "default_true")]
    pub group_similar: bool,
    #[serde(default = "default_true")]
    pub filter_blurry: bool,
    #[serde(default = "default_ratio")]
    pub duplicate_ratio: DuplicateRatio,
    /// the share of the keepers flagged as highlights, 0-100
    #[serde(default = "default_highlights")]
    pub highlight_percent: u8,
}

impl Default for CullSettings {
    fn default() -> Self {
        Self {
            shoot_type: default_shoot(),
            similarity_threshold: default_similarity(),
            blur_threshold: default_blur(),
            group_similar: true,
            filter_blurry: true,
            duplicate_ratio: DuplicateRatio::Moderate,
            highlight_percent: default_highlights(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub enum CullStatus {
    Keep,
    Reject,
}

impl CullStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CullStatus::Keep => "keep",
            CullStatus::Reject => "reject",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "keep" => Some(CullStatus::Keep),
            "reject" => Some(CullStatus::Reject),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub enum CullReason {
    /// not in a set and sharp enough
    Unique,
    /// the best frame of a duplicate set
    BestInGroup,
    /// a lower-ranked frame of a set that the duplicate ratio keeps as well
    KeptInGroup,
    /// a single frame under the sharpness threshold
    Blurry,
    /// a lower-ranked frame of a set (`bestPath` names the frame it lost to)
    DuplicateOf,
    /// a frame under the quality a Scenes rule asks for
    LowQuality,
}

impl CullReason {
    pub fn as_str(self) -> &'static str {
        match self {
            CullReason::Unique => "unique",
            CullReason::BestInGroup => "bestInGroup",
            CullReason::KeptInGroup => "keptInGroup",
            CullReason::Blurry => "blurry",
            CullReason::DuplicateOf => "duplicateOf",
            CullReason::LowQuality => "lowQuality",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "unique" => CullReason::Unique,
            "bestInGroup" => CullReason::BestInGroup,
            "keptInGroup" => CullReason::KeptInGroup,
            "blurry" => CullReason::Blurry,
            "duplicateOf" => CullReason::DuplicateOf,
            "lowQuality" => CullReason::LowQuality,
            _ => return None,
        })
    }
}

/// One line of "why this decision". `impact` is "reject", "supporting" or
/// "context".
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullFactor {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub impact: String,
}

/// One photo to plan, with its measurements.
pub struct Input {
    pub path: String,
    pub analysis: CullAnalysis,
    pub hash: ImageHash,
}

/// A planned decision, before it has a database id.
#[derive(Debug, Clone)]
pub struct Planned {
    /// the input it was planned from (its order is the input's)
    #[allow(dead_code)]
    pub path: String,
    pub proposed: CullStatus,
    pub reason: CullReason,
    /// the 1-based set this frame belongs to, if any
    pub group: Option<u32>,
    /// the frame this one lost to (`DuplicateOf`), or the set's best
    pub best_path: Option<String>,
    pub highlight: bool,
    pub quality_score: f64,
    pub factors: Vec<CullFactor>,
    pub blurry_region: Option<CullBlurRegion>,
    pub analysis: CullAnalysis,
}

fn name(path: &str) -> &str {
    Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path)
}

/// Plans keepers, rejects and highlights over every analysed photo.
/// Order: the input's, so a plan is stable for the same scope.
pub fn plan(inputs: &[Input], settings: &CullSettings) -> Vec<Planned> {
    let scored: Vec<(ImageHash, f64)> = inputs.iter().map(|i| (i.hash.clone(), i.analysis.quality_score)).collect();
    let groups = if settings.group_similar { cull::group_similar(&scored, settings.similarity_threshold) } else { Vec::new() };

    // per input: (group number, rank in its group, group size, best path)
    let mut placement: HashMap<usize, (u32, usize, usize, usize)> = HashMap::new();
    for (g, group) in groups.iter().enumerate() {
        for (rank, &member) in group.members.iter().enumerate() {
            placement.insert(member, (g as u32 + 1, rank, group.members.len(), group.members[0]));
        }
    }

    let mut planned: Vec<Planned> = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let a = &input.analysis;
            let mut factors = Vec::new();
            let (proposed, reason, group, best_path) = match placement.get(&index) {
                Some(&(group, rank, size, best)) => {
                    let best_input = &inputs[best];
                    if rank == 0 {
                        (CullStatus::Keep, CullReason::BestInGroup, Some(group), None)
                    } else if rank < settings.duplicate_ratio.keepers(size) {
                        (CullStatus::Keep, CullReason::KeptInGroup, Some(group), Some(best_input.path.clone()))
                    } else {
                        factors.push(CullFactor {
                            id: "duplicate".into(),
                            label: "Near duplicate".into(),
                            detail: format!(
                                "Lower quality score ({:.2}) than {} ({:.2})",
                                a.quality_score,
                                name(&best_input.path),
                                best_input.analysis.quality_score
                            ),
                            impact: "reject".into(),
                        });
                        (CullStatus::Reject, CullReason::DuplicateOf, Some(group), Some(best_input.path.clone()))
                    }
                }
                None if settings.filter_blurry && a.sharpness_metric < settings.blur_threshold => {
                    factors.push(CullFactor {
                        id: "sharpness".into(),
                        label: "Low sharpness".into(),
                        detail: format!("Laplacian sharpness {:.1}; rejection threshold {:.1}", a.sharpness_metric, settings.blur_threshold),
                        impact: "reject".into(),
                    });
                    (CullStatus::Reject, CullReason::Blurry, None, None)
                }
                None => (CullStatus::Keep, CullReason::Unique, None, None),
            };
            let keep = proposed == CullStatus::Keep;
            factors.push(CullFactor {
                id: "technical_quality".into(),
                label: "Technical quality score".into(),
                detail: format!(
                    "Score {:.2}: sharpness {:.1}, focus {:.1}, exposure {:.2}",
                    a.quality_score,
                    a.sharpness_metric,
                    a.subject_focus_metric.unwrap_or(a.center_focus_metric),
                    a.exposure_metric
                ),
                impact: if keep { "context" } else { "supporting" }.into(),
            });
            if let Some(composition) = a.subject_composition_metric {
                factors.push(CullFactor {
                    id: "subject_geometry".into(),
                    label: "Subject placement cue".into(),
                    detail: format!(
                        "Subject placement score {composition:.2}; subject edge contact {:.0}%. A minor tie-breaker, not an aesthetic verdict.",
                        a.subject_edge_contact_ratio.unwrap_or_default() * 100.0
                    ),
                    impact: "context".into(),
                });
            }
            Planned {
                path: input.path.clone(),
                proposed,
                reason,
                group,
                best_path,
                highlight: false,
                quality_score: a.quality_score,
                factors,
                blurry_region: if reason == CullReason::Blurry { a.blurry_region } else { None },
                analysis: a.clone(),
            }
        })
        .collect();

    // highlights: the best-scoring share of the keepers
    let mut keepers: Vec<usize> = (0..planned.len()).filter(|&i| planned[i].proposed == CullStatus::Keep).collect();
    keepers.sort_by(|&a, &b| planned[b].quality_score.partial_cmp(&planned[a].quality_score).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
    let percent = f64::from(settings.highlight_percent.min(100));
    let count = if percent == 0.0 { 0 } else { ((keepers.len() as f64 * percent / 100.0).ceil() as usize).min(keepers.len()) };
    for &i in keepers.iter().take(count) {
        planned[i].highlight = true;
    }
    planned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cull::tests::sharp_scene;
    use crate::cull::{analyze, Analysed};
    use image::{imageops, Rgb, RgbImage};

    fn input(path: &str, frame: Analysed) -> Input {
        Input { path: path.into(), analysis: frame.result, hash: frame.hash }
    }

    fn other_scene() -> RgbImage {
        RgbImage::from_fn(320, 240, |x, y| Rgb([(y * 255 / 240) as u8, (255 - x * 255 / 320) as u8, 60]))
    }

    fn burst() -> Vec<Input> {
        let sharp = sharp_scene(320, 240, 0);
        vec![
            input("/s/a_soft.jpg", analyze(&imageops::blur(&sharp, 1.2))),
            input("/s/a_sharp.jpg", analyze(&sharp)),
            input("/s/a_softer.jpg", analyze(&imageops::blur(&sharp, 2.0))),
            input("/s/unique.jpg", analyze(&other_scene())),
        ]
    }

    fn by_path<'a>(plan: &'a [Planned], path: &str) -> &'a Planned {
        plan.iter().find(|p| p.path == path).unwrap()
    }

    #[test]
    fn duplicate_ratio_counts() {
        assert_eq!(DuplicateRatio::Less.keepers(10), 1);
        assert_eq!(DuplicateRatio::Moderate.keepers(10), 2);
        assert_eq!(DuplicateRatio::More.keepers(10), 4);
        assert_eq!(DuplicateRatio::More.keepers(2), 1);
        assert_eq!(DuplicateRatio::Moderate.keepers(3), 1);
        assert_eq!(DuplicateRatio::More.keepers(1), 1);
    }

    #[test]
    fn a_burst_keeps_its_best_frame_and_rejects_the_rest() {
        let inputs = burst();
        let settings = CullSettings { duplicate_ratio: DuplicateRatio::Less, filter_blurry: false, ..Default::default() };
        let plan = plan(&inputs, &settings);
        let best = by_path(&plan, "/s/a_sharp.jpg");
        assert_eq!((best.proposed, best.reason), (CullStatus::Keep, CullReason::BestInGroup));
        for soft in ["/s/a_soft.jpg", "/s/a_softer.jpg"] {
            let p = by_path(&plan, soft);
            assert_eq!((p.proposed, p.reason), (CullStatus::Reject, CullReason::DuplicateOf), "{soft}");
            assert_eq!(p.best_path.as_deref(), Some("/s/a_sharp.jpg"));
            assert_eq!(p.group, best.group);
            assert!(p.factors.iter().any(|f| f.id == "duplicate" && f.impact == "reject"));
        }
        let unique = by_path(&plan, "/s/unique.jpg");
        assert_eq!((unique.proposed, unique.reason, unique.group), (CullStatus::Keep, CullReason::Unique, None));
    }

    #[test]
    fn more_keeps_a_second_frame_of_a_big_enough_set() {
        // ten near-identical frames: More keeps ceil(3.5) = 4
        let sharp = sharp_scene(320, 240, 0);
        let inputs: Vec<Input> = (0..10)
            .map(|i| input(&format!("/s/f{i}.jpg"), analyze(&imageops::blur(&sharp, 0.2 + i as f32 * 0.1))))
            .collect();
        let settings = CullSettings { duplicate_ratio: DuplicateRatio::More, filter_blurry: false, similarity_threshold: 60, ..Default::default() };
        let plan = plan(&inputs, &settings);
        assert_eq!(plan.iter().filter(|p| p.proposed == CullStatus::Keep).count(), 4);
        assert_eq!(plan.iter().filter(|p| p.reason == CullReason::KeptInGroup).count(), 3);
        assert_eq!(plan.iter().filter(|p| p.reason == CullReason::BestInGroup).count(), 1);
    }

    #[test]
    fn a_single_blurry_frame_is_rejected_with_its_region() {
        let heavy = imageops::blur(&sharp_scene(320, 240, 0), 6.0);
        let inputs = vec![input("/s/blur.jpg", analyze(&heavy)), input("/s/ok.jpg", analyze(&other_scene()))];
        // the second scene is smooth: lower the bar so only the textured frame is judged
        let settings = CullSettings { blur_threshold: 100.0, ..Default::default() };
        let plan = plan(&inputs, &settings);
        let p = by_path(&plan, "/s/blur.jpg");
        assert_eq!((p.proposed, p.reason), (CullStatus::Reject, CullReason::Blurry));
        assert!(p.factors.iter().any(|f| f.id == "sharpness"));
        let off = super::plan(&inputs, &CullSettings { filter_blurry: false, ..settings });
        assert_eq!(by_path(&off, "/s/blur.jpg").proposed, CullStatus::Keep);
    }

    #[test]
    fn grouping_can_be_turned_off() {
        let settings = CullSettings { group_similar: false, filter_blurry: false, ..Default::default() };
        assert!(plan(&burst(), &settings).iter().all(|p| p.proposed == CullStatus::Keep && p.group.is_none()));
    }

    #[test]
    fn highlights_are_the_best_share_of_the_keepers() {
        let sharp = sharp_scene(160, 120, 0);
        let inputs: Vec<Input> = (0..10)
            .map(|i| {
                // ten different scenes, so none groups
                let scene = RgbImage::from_fn(160, 120, |x, y| {
                    let cell = (4 + i * 3) as u32;
                    let v = if ((x / cell) + (y / cell)) % 2 == 0 { 30 } else { 220 };
                    Rgb([v, v, v])
                });
                let _ = &sharp;
                input(&format!("/s/h{i}.jpg"), analyze(&scene))
            })
            .collect();
        let settings = CullSettings { group_similar: false, filter_blurry: false, highlight_percent: 30, ..Default::default() };
        let plan = plan(&inputs, &settings);
        let highlights: Vec<&Planned> = plan.iter().filter(|p| p.highlight).collect();
        assert_eq!(highlights.len(), 3);
        let lowest = highlights.iter().map(|p| p.quality_score).fold(f64::INFINITY, f64::min);
        assert!(plan.iter().filter(|p| !p.highlight).all(|p| p.quality_score <= lowest));
        let none = super::plan(&inputs, &CullSettings { highlight_percent: 0, ..settings });
        assert!(none.iter().all(|p| !p.highlight));
    }

    #[test]
    fn rejects_are_never_highlights() {
        let settings = CullSettings { highlight_percent: 100, duplicate_ratio: DuplicateRatio::Less, ..Default::default() };
        let plan = plan(&burst(), &settings);
        assert!(plan.iter().all(|p| p.highlight == (p.proposed == CullStatus::Keep)));
    }

    #[test]
    fn older_settings_fill_their_defaults() {
        let s: CullSettings = serde_json::from_str(r#"{"similarityThreshold":40}"#).unwrap();
        assert_eq!(s.similarity_threshold, 40);
        assert_eq!(s.duplicate_ratio, DuplicateRatio::Moderate);
        assert_eq!(s.highlight_percent, 15);
        assert_eq!(s.shoot_type, "general");
    }

    #[test]
    fn status_and_reason_round_trip_text() {
        for r in [CullReason::Unique, CullReason::BestInGroup, CullReason::KeptInGroup, CullReason::Blurry, CullReason::DuplicateOf] {
            assert_eq!(CullReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(CullStatus::parse("reject"), Some(CullStatus::Reject));
        assert_eq!(CullStatus::parse("x"), None);
    }
}

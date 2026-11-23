// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Scenes' keep rules: from each scene's frames, their scores, ratings and flags, the
//! verdict of every frame (keep, reject or leave alone), and the cull plan items that
//! record those verdicts so `cull_apply` and `cull_undo` handle them like an Auto Cull
//! session. Pure: no files, engine or database, so it is tested with hand-made scenes.
//! The commands are in scenes_cmds.rs.
//!
//! The rules, per scene (the spec's section 3.4):
//! - frames already rejected stay as they are and take no part in the ranking;
//! - frames the user accepted in Scenes, or rated at least the "always keep" rating, are
//!   kept, and count toward the keep-best number;
//! - the other frames are ranked by score; the best are kept up to the keep-best number
//!   (0 means no cap), the rest are rejected as lower-ranked frames of their scene;
//! - a frame under the minimum quality is rejected, except that a scene's best frame is
//!   never rejected for its score (a scene never disappears by a rule);
//! - a scene whose ranked frames are not all scored yet is left undecided;
//! - a scene of one frame is kept or left undecided, as asked;
//! - a frame the user moved to the Keep or Reject pile by hand always wins.
use crate::cull::CullAnalysis;
use crate::cull_plan::{CullFactor, CullReason, CullStatus, Planned};
use crate::cull_store::{CullTarget, Item};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    /// frames kept per scene, best first; 0 keeps every frame (no cap)
    pub keep_best: u32,
    /// frames scoring under this (0-100) are rejected; 0 is off
    pub min_quality: u32,
    /// frames rated at least `rated_at_least` are always kept
    pub always_keep: bool,
    pub rated_at_least: u32,
    /// a scene of one frame is kept (else left undecided)
    pub singles_keep: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self { keep_best: 2, min_quality: 0, always_keep: true, rated_at_least: 3, singles_keep: false }
    }
}

/// A hand-made call on one frame, from the piles review.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub enum Force {
    Keep,
    Reject,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuleFrame {
    pub path: String,
    #[serde(default)]
    pub version: i64,
    /// the cached quality, 0-1; None while unscored
    #[serde(default)]
    pub quality: Option<f64>,
    #[serde(default)]
    pub rating: u32,
    #[serde(default)]
    pub rejected: bool,
    /// Scenes accepted it
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub force: Option<Force>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuleScene {
    pub frames: Vec<RuleFrame>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// the scene's best frame
    KeepBest,
    /// ranked high enough to stay
    KeepKept,
    /// accepted in Scenes, rated high enough, or put in the Keep pile
    KeepProtected,
    /// the only frame of its scene, and singles are kept
    KeepSingle,
    /// ranked below the keep-best number
    RejectDuplicate,
    RejectLowQuality,
    /// put in the Reject pile by hand
    RejectManual,
    /// already rejected in the library
    RejectedAlready,
    /// some frame of the scene has no score yet
    UndecidedUnscored,
    /// the only frame of its scene, and singles are skipped
    UndecidedSingle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keep,
    Reject,
    Undecided,
}

impl Verdict {
    pub fn kind(self) -> Kind {
        match self {
            Verdict::KeepBest | Verdict::KeepKept | Verdict::KeepProtected | Verdict::KeepSingle => Kind::Keep,
            Verdict::RejectDuplicate | Verdict::RejectLowQuality | Verdict::RejectManual | Verdict::RejectedAlready => Kind::Reject,
            Verdict::UndecidedUnscored | Verdict::UndecidedSingle => Kind::Undecided,
        }
    }

    /// whether the plan records the frame: an already rejected frame is left out, so Apply never touches it
    fn in_plan(self) -> bool {
        self.kind() != Kind::Undecided && self != Verdict::RejectedAlready
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuleCounts {
    pub keep: u32,
    pub reject: u32,
    pub undecided: u32,
}

pub fn counts(verdicts: &[Vec<Verdict>]) -> RuleCounts {
    let mut out = RuleCounts::default();
    for v in verdicts.iter().flatten() {
        match v.kind() {
            Kind::Keep => out.keep += 1,
            Kind::Reject => out.reject += 1,
            Kind::Undecided => out.undecided += 1,
        }
    }
    out
}

fn percent(q: f64) -> u32 {
    (q * 100.0).round().clamp(0.0, 100.0) as u32
}

/// The verdict of every frame of every scene, in the scenes' order.
pub fn evaluate(scenes: &[RuleScene], rules: &Rules) -> Vec<Vec<Verdict>> {
    scenes.iter().map(|scene| evaluate_scene(&scene.frames, rules)).collect()
}

fn evaluate_scene(frames: &[RuleFrame], rules: &Rules) -> Vec<Verdict> {
    let mut out = vec![Verdict::UndecidedUnscored; frames.len()];
    if frames.len() == 1 {
        let f = &frames[0];
        out[0] = if f.rejected && f.force != Some(Force::Keep) {
            Verdict::RejectedAlready
        } else if f.force == Some(Force::Reject) {
            Verdict::RejectManual
        } else if f.force == Some(Force::Keep) || f.accepted {
            Verdict::KeepProtected
        } else if rules.singles_keep {
            Verdict::KeepSingle
        } else {
            Verdict::UndecidedSingle
        };
        return out;
    }
    let mut protected = 0usize;
    let mut ranked: Vec<usize> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        out[i] = match f.force {
            Some(Force::Reject) => if f.rejected { Verdict::RejectedAlready } else { Verdict::RejectManual },
            Some(Force::Keep) => Verdict::KeepProtected,
            None if f.rejected => Verdict::RejectedAlready,
            None if f.accepted || (rules.always_keep && f.rating >= rules.rated_at_least) => Verdict::KeepProtected,
            None => {
                ranked.push(i);
                continue;
            }
        };
        if out[i] == Verdict::KeepProtected {
            protected += 1;
        }
    }
    if ranked.iter().any(|&i| frames[i].quality.is_none()) {
        return out;   // the ranked frames stay UndecidedUnscored
    }
    ranked.sort_by(|&a, &b| frames[b].quality.partial_cmp(&frames[a].quality).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
    let slots = if rules.keep_best == 0 { usize::MAX } else { (rules.keep_best as usize).saturating_sub(protected) };
    for (rank, &i) in ranked.iter().enumerate() {
        let low = rules.min_quality > 0 && percent(frames[i].quality.unwrap_or(0.0)) < rules.min_quality;
        // the scene's best frame is never rejected for its score, so a scene does not vanish
        let guarded = rank == 0 && protected == 0;
        out[i] = if guarded {
            Verdict::KeepBest
        } else if rank >= slots {
            Verdict::RejectDuplicate
        } else if low {
            Verdict::RejectLowQuality
        } else {
            Verdict::KeepKept
        };
    }
    out
}

/// A neutral measurement for a frame that was never scored (a protected or forced frame can be
/// recorded without one).
fn blank(quality: f64) -> CullAnalysis {
    CullAnalysis {
        quality_score: quality, sharpness_metric: 0.0, center_focus_metric: 0.0, subject_focus_metric: None, subject_composition_metric: None,
        subject_edge_contact_ratio: None, exposure_metric: 0.0, width: 0, height: 0, blurry_region: None, sharp_region: None,
    }
}

/// What the file looks like now, for the plan's record of it, and the frame's cached measurement.
pub struct Known {
    pub analysis: Option<CullAnalysis>,
    pub size: i64,
    pub modified_ms: i64,
}

/// The plan items of `verdicts` (from `evaluate` over the same `scenes`): one per frame the rules
/// keep or reject, in scene order, with the scene number as its set. `known` returns what is known of a
/// frame's file, or None when the file is gone (the frame is left out of the plan and listed in the
/// returned failures).
pub fn plan_items(
    scenes: &[RuleScene], verdicts: &[Vec<Verdict>], known: &dyn Fn(&RuleFrame) -> Option<Known>,
) -> (Vec<Item>, Vec<(String, String)>) {
    let mut items = Vec::new();
    let mut failed = Vec::new();
    for (n, (scene, verdict)) in scenes.iter().zip(verdicts).enumerate() {
        let best = scene.frames.iter().zip(verdict).find(|(_, v)| **v == Verdict::KeepBest).map(|(f, _)| f.path.clone());
        for (frame, &v) in scene.frames.iter().zip(verdict) {
            if !v.in_plan() {
                continue;
            }
            let Some(k) = known(frame) else {
                failed.push((frame.path.clone(), "the file is missing or unreadable".to_string()));
                continue;
            };
            let quality = frame.quality.unwrap_or(0.0);
            let analysis = k.analysis.unwrap_or_else(|| blank(quality));
            let keep = v.kind() == Kind::Keep;
            let (reason, text) = match v {
                Verdict::KeepBest => (CullReason::BestInGroup, "The best frame of its scene".to_string()),
                Verdict::KeepKept => (CullReason::KeptInGroup, "Ranked high enough to stay".to_string()),
                Verdict::KeepProtected => (CullReason::KeptInGroup, "Accepted, rated high enough, or kept by you".to_string()),
                Verdict::KeepSingle => (CullReason::Unique, "The only frame of its scene".to_string()),
                Verdict::RejectLowQuality => (CullReason::LowQuality, format!("Quality {} is under the minimum", percent(quality))),
                Verdict::RejectManual => (CullReason::DuplicateOf, "Moved to the Reject pile by you".to_string()),
                _ => (
                    CullReason::DuplicateOf,
                    format!("Ranked below the frames kept from its scene (quality {})", percent(quality)),
                ),
            };
            let factor = CullFactor {
                id: "scene_rule".into(),
                label: "Scenes rule".into(),
                detail: text,
                impact: if keep { "context" } else { "reject" }.into(),
            };
            let in_set = reason != CullReason::Unique;
            items.push(Item {
                target: CullTarget { path: frame.path.clone(), version: frame.version },
                backing: Vec::new(),
                size: k.size,
                modified_ms: k.modified_ms,
                planned: Planned {
                    path: frame.path.clone(),
                    proposed: if keep { CullStatus::Keep } else { CullStatus::Reject },
                    reason,
                    group: if in_set { Some(n as u32 + 1) } else { None },
                    best_path: if in_set && v != Verdict::KeepBest { best.clone() } else { None },
                    highlight: false,
                    quality_score: quality,
                    factors: vec![factor],
                    blurry_region: None,
                    analysis,
                },
            });
        }
    }
    (items, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, q: Option<f64>) -> RuleFrame {
        RuleFrame { path: format!("/s/{name}"), version: 0, quality: q, rating: 0, rejected: false, accepted: false, force: None }
    }
    fn scene(frames: Vec<RuleFrame>) -> RuleScene {
        RuleScene { frames }
    }
    fn rules(keep_best: u32) -> Rules {
        Rules { keep_best, min_quality: 0, always_keep: false, rated_at_least: 3, singles_keep: false }
    }
    use Verdict::*;

    #[test]
    fn keep_best_n_keeps_the_top_n_by_score() {
        let s = scene(vec![f("a", Some(0.4)), f("b", Some(0.9)), f("c", Some(0.7)), f("d", Some(0.1))]);
        assert_eq!(evaluate(&[s.clone()], &rules(2))[0], vec![RejectDuplicate, KeepBest, KeepKept, RejectDuplicate]);
        assert_eq!(evaluate(&[s.clone()], &rules(1))[0], vec![RejectDuplicate, KeepBest, RejectDuplicate, RejectDuplicate]);
        // 0 is no cap: nothing is rejected
        assert_eq!(evaluate(&[s], &rules(0))[0], vec![KeepKept, KeepBest, KeepKept, KeepKept]);
    }

    #[test]
    fn ties_keep_capture_order() {
        let s = scene(vec![f("a", Some(0.5)), f("b", Some(0.5)), f("c", Some(0.5))]);
        assert_eq!(evaluate(&[s], &rules(1))[0], vec![KeepBest, RejectDuplicate, RejectDuplicate]);
    }

    #[test]
    fn minimum_quality_rejects_but_never_a_scenes_best_frame() {
        let s = scene(vec![f("a", Some(0.30)), f("b", Some(0.20)), f("c", Some(0.10))]);
        let r = Rules { min_quality: 50, ..rules(0) };
        assert_eq!(evaluate(&[s], &r)[0], vec![KeepBest, RejectLowQuality, RejectLowQuality]);
        let good = scene(vec![f("a", Some(0.9)), f("b", Some(0.49)), f("c", Some(0.5))]);
        assert_eq!(evaluate(&[good], &r)[0], vec![KeepBest, RejectLowQuality, KeepKept]);
    }

    #[test]
    fn rated_frames_win_and_count_toward_the_number() {
        let mut a = f("a", Some(0.1));
        a.rating = 4;
        let s = scene(vec![a, f("b", Some(0.9)), f("c", Some(0.8))]);
        let r = Rules { always_keep: true, ..rules(2) };
        // a is kept by its rating and fills one slot; the best of the rest fills the other
        assert_eq!(evaluate(&[s.clone()], &r)[0], vec![KeepProtected, KeepKept, RejectDuplicate]);
        // with the option off the rating counts for nothing
        assert_eq!(evaluate(&[s], &rules(2))[0], vec![RejectDuplicate, KeepBest, KeepKept]);
        // more rated frames than slots: all stay
        let many = scene((0..3).map(|i| RuleFrame { rating: 5, ..f(&i.to_string(), Some(0.5)) }).chain([f("z", Some(0.99))]).collect());
        assert_eq!(evaluate(&[many], &Rules { always_keep: true, ..rules(2) })[0], vec![KeepProtected, KeepProtected, KeepProtected, RejectDuplicate]);
    }

    #[test]
    fn accepted_and_hand_picked_frames_win_over_the_rules() {
        let mut a = f("a", Some(0.1));
        a.accepted = true;
        let mut b = f("b", Some(0.2));
        b.force = Some(Force::Keep);
        let mut c = f("c", Some(0.99));
        c.force = Some(Force::Reject);
        let s = scene(vec![a, b, c, f("d", Some(0.5))]);
        assert_eq!(evaluate(&[s], &rules(1))[0], vec![KeepProtected, KeepProtected, RejectManual, RejectDuplicate]);
    }

    #[test]
    fn rejected_frames_are_left_alone_and_unscored_scenes_wait() {
        let mut a = f("a", Some(0.9));
        a.rejected = true;
        let s = scene(vec![a, f("b", Some(0.2)), f("c", Some(0.1))]);
        assert_eq!(evaluate(&[s], &rules(1))[0], vec![RejectedAlready, KeepBest, RejectDuplicate]);
        let unscored = scene(vec![f("a", Some(0.9)), f("b", None)]);
        assert_eq!(evaluate(&[unscored], &rules(1))[0], vec![UndecidedUnscored, UndecidedUnscored]);
        // a rated frame is kept even while its scene is unscored
        let mut r = f("r", None);
        r.rating = 5;
        let waiting = scene(vec![r, f("b", None)]);
        assert_eq!(evaluate(&[waiting], &Rules { always_keep: true, ..rules(1) })[0], vec![KeepProtected, UndecidedUnscored]);
    }

    #[test]
    fn singles_are_kept_or_skipped_as_asked() {
        let s = scene(vec![f("a", None)]);
        assert_eq!(evaluate(&[s.clone()], &rules(1))[0], vec![UndecidedSingle]);
        assert_eq!(evaluate(&[s], &Rules { singles_keep: true, ..rules(1) })[0], vec![KeepSingle]);
        let mut r = f("r", Some(0.1));
        r.rejected = true;
        assert_eq!(evaluate(&[scene(vec![r])], &Rules { singles_keep: true, ..rules(1) })[0], vec![RejectedAlready]);
    }

    #[test]
    fn counts_add_up_over_scenes() {
        let scenes = vec![scene(vec![f("a", Some(0.9)), f("b", Some(0.1))]), scene(vec![f("c", None)])];
        let c = counts(&evaluate(&scenes, &rules(1)));
        assert_eq!((c.keep, c.reject, c.undecided), (1, 1, 1));
    }

    #[test]
    fn plan_items_record_keeps_and_rejects_with_the_scene_as_the_set() {
        let mut gone = f("gone", Some(0.0));
        gone.rejected = false;
        let mut already = f("already", Some(0.5));
        already.rejected = true;
        let scenes = vec![scene(vec![f("a", Some(0.9)), f("b", Some(0.1)), already]), scene(vec![f("only", None)]), scene(vec![gone, f("z", Some(0.8))])];
        let verdicts = evaluate(&scenes, &Rules { singles_keep: false, ..rules(1) });
        let known = |fr: &RuleFrame| (!fr.path.ends_with("gone")).then(|| Known { analysis: None, size: 10, modified_ms: 20 });
        let (items, failed) = plan_items(&scenes, &verdicts, &known);
        let got: Vec<(&str, CullStatus, CullReason, Option<u32>)> = items.iter().map(|i| (i.target.path.as_str(), i.planned.proposed, i.planned.reason, i.planned.group)).collect();
        assert_eq!(got, vec![
            ("/s/a", CullStatus::Keep, CullReason::BestInGroup, Some(1)),
            ("/s/b", CullStatus::Reject, CullReason::DuplicateOf, Some(1)),
            ("/s/z", CullStatus::Keep, CullReason::BestInGroup, Some(3)),
        ]);
        assert_eq!(items[1].planned.best_path.as_deref(), Some("/s/a"));
        assert!(items.iter().all(|i| !i.planned.highlight));
        // the already rejected frame and the skipped single are not in the plan; the missing file is listed
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "/s/gone");
    }

    #[test]
    fn a_plan_applies_and_undoes_through_cull_apply() {
        use crate::cull_apply::{apply_session, tests::Fake, undo_session, CullApplyOptions};
        use crate::cull_plan::CullSettings;
        use crate::cull_store::{CullLibraryState, Store};
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("cull.sqlite")).unwrap();
        let scenes = vec![scene(vec![f("a", Some(0.9)), f("b", Some(0.4)), f("c", Some(0.2))])];
        let verdicts = evaluate(&scenes, &rules(1));
        let (items, _) = plan_items(&scenes, &verdicts, &|_| Some(Known { analysis: None, size: 1, modified_ms: 1 }));
        let (session, _) = store.create_session("Scenes: test", &CullSettings::default(), items, vec![]).unwrap();
        assert_eq!((session.keep_count, session.reject_count), (1, 2));
        let library = Fake::default();
        for n in ["a", "b", "c"] {
            library.0.lock().unwrap().insert(format!("/s/{n}"), CullLibraryState { rating: 2, rejected: false, labels: 0 });
        }
        // the files do not exist, so the check against a changed file passes (apply refuses only a file that changed)
        let applied = apply_session(&store, session.id, &CullApplyOptions::default(), &library).unwrap();
        assert_eq!(applied.rejected, 2);
        assert!(library.0.lock().unwrap()["/s/b"].rejected && !library.0.lock().unwrap()["/s/a"].rejected);
        undo_session(&store, session.id, &library).unwrap();
        assert!(!library.0.lock().unwrap()["/s/b"].rejected);
        assert_eq!(library.0.lock().unwrap()["/s/b"].rating, 2);
    }
}

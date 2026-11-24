// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! What a photo's faces add to its culling score: a small pose adjustment and an eye-state adjustment,
//! from the faces Faces already stored (no model, no file). Pure, so it is tested with made-up faces.
//!
//! The rules are the old app's (notes/faces-port-inventory.md): the adjustment is
//! `clamp(pose + eyes, -0.2, +0.04)`, `pose = clamp((frontal - 0.5) * 0.08, +-0.04)` of the primary face
//! (the one with the highest `0.8 * frontal + 0.2 * frame fraction`), closed eyes -0.15 (half closed eyes are shown and cost nothing),
//! the worse eye deciding. Only a face of at least 80 px that looks at the camera (frontal >= 0.5) can
//! have an eye verdict (eyes cast down look closed to the model, so the rule in faces_eyes.rs is deliberately conservative). A closed-eye verdict only lowers a score; it never rejects a photo by itself.
use crate::cull_plan::CullFactor;
use crate::faces_detect::EyeVerdict;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Narrowest face, in pixels of the picture it was found in, that may have an eye verdict.
pub const MIN_EYE_PX: f32 = 80.0;
/// Least frontal score a face may have to get an eye verdict (a profile face has no usable eyes).
pub const MIN_EYE_FRONTAL: f32 = 0.5;
/// The adjustment's limits.
pub const MIN_ADJUST: f32 = -0.2;
pub const MAX_ADJUST: f32 = 0.04;

/// A face's eye verdict as the page sees it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum EyeLabel {
    Open,
    Semi,
    Closed,
}

impl EyeLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            EyeLabel::Open => "open",
            EyeLabel::Semi => "semi",
            EyeLabel::Closed => "closed",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "open" => Some(EyeLabel::Open),
            "semi" => Some(EyeLabel::Semi),
            "closed" => Some(EyeLabel::Closed),
            _ => None,
        }
    }
    /// 0 open, 1 semi, 2 closed: the larger is the worse
    pub fn rank(self) -> u8 {
        match self {
            EyeLabel::Open => 0,
            EyeLabel::Semi => 1,
            EyeLabel::Closed => 2,
        }
    }
}

impl From<EyeVerdict> for EyeLabel {
    fn from(v: EyeVerdict) -> Self {
        match v {
            EyeVerdict::Open => EyeLabel::Open,
            EyeVerdict::SemiClosed => EyeLabel::Semi,
            EyeVerdict::Closed => EyeLabel::Closed,
        }
    }
}

/// Whose eyes count in a photo with several faces.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Type)]
#[serde(rename_all = "camelCase")]
pub enum EyeMode {
    /// only the primary face decides (default)
    #[default]
    MainSubject,
    /// any eligible face with closed eyes counts, at a smaller weight
    AnyFace,
}

/// One stored face, as the culling score needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct CullFace {
    pub id: i64,
    /// the box's size as a share of the picture's width and height
    pub w: f32,
    pub h: f32,
    /// the box's width in pixels of the picture the face was found in
    pub px: f32,
    pub frontal: f32,
    /// the stored eye verdict, None while unknown
    pub eye: Option<EyeLabel>,
}

impl CullFace {
    /// Big enough and facing the camera: the only faces whose eyes are judged.
    pub fn eligible(&self) -> bool {
        self.px >= MIN_EYE_PX && self.frontal >= MIN_EYE_FRONTAL
    }
    fn primary_score(&self) -> f32 {
        0.8 * self.frontal + 0.2 * self.w * self.h
    }
}

/// The result for one photo.
#[derive(Debug, Clone)]
pub struct FaceQuality {
    pub faces: u32,
    /// the face id of the primary face
    pub primary: Option<i64>,
    /// the primary face's frontal score
    pub frontal: Option<f32>,
    /// the eye verdict that counts (the primary face's, or the worst eligible face's with `AnyFace`)
    pub eye: Option<EyeLabel>,
    /// the face that verdict came from
    pub eye_face: Option<i64>,
    /// added to the quality score, -0.2 to +0.04
    pub adjust: f32,
    /// the pose part of `adjust` alone: what applies while eyes do not rank (`CullSettings.eyes_rank` off)
    pub pose_adjust: f32,
    pub factors: Vec<CullFactor>,
}

impl FaceQuality {
    /// What is added to the quality score: pose alone, or with the eye penalty when eyes rank.
    pub fn adjust_with(&self, rank_eyes: bool) -> f32 {
        if rank_eyes { self.adjust } else { self.pose_adjust }
    }

    /// The decision factors: with eyes not ranking the eye factor is only context and says the score is unchanged.
    pub fn factors_with(&self, rank_eyes: bool) -> Vec<CullFactor> {
        self.factors
            .iter()
            .map(|f| {
                let mut f = f.clone();
                if f.id == "eye_state" && !rank_eyes && f.impact == "supporting" {
                    f.impact = "context".into();
                    f.detail = format!("{} - shown only, the score is not changed", f.label);
                }
                f
            })
            .collect()
    }

    /// Nothing known: no faces, no change.
    pub fn none() -> Self {
        Self { faces: 0, primary: None, frontal: None, eye: None, eye_face: None, adjust: 0.0, pose_adjust: 0.0, factors: Vec::new() }
    }
}

fn eye_penalty(label: EyeLabel, mode: EyeMode) -> f32 {
    match (mode, label) {
        (_, EyeLabel::Open) => 0.0,
        (_, EyeLabel::Semi) => 0.0,
        (EyeMode::MainSubject, EyeLabel::Closed) => -0.15,
        (EyeMode::AnyFace, EyeLabel::Closed) => -0.08,
    }
}

/// The primary face of a photo: the best `0.8 * frontal + 0.2 * frame fraction`, the first on a tie.
pub fn primary_of(faces: &[CullFace]) -> Option<&CullFace> {
    faces.iter().fold(None, |best: Option<&CullFace>, f| match best {
        Some(b) if b.primary_score() >= f.primary_score() => Some(b),
        _ => Some(f),
    })
}

/// The eye verdict that counts for a photo, and the face it came from.
pub fn eye_of(faces: &[CullFace], mode: EyeMode) -> Option<(EyeLabel, i64)> {
    match mode {
        EyeMode::MainSubject => primary_of(faces).filter(|f| f.eligible()).and_then(|f| Some((f.eye?, f.id))),
        EyeMode::AnyFace => faces
            .iter()
            .filter(|f| f.eligible())
            .filter_map(|f| Some((f.eye?, f.id)))
            // the worst wins, the first face on a tie
            .fold(None, |best: Option<(EyeLabel, i64)>, c| match best {
                Some(b) if b.0.rank() >= c.0.rank() => Some(b),
                _ => Some(c),
            }),
    }
}

/// The culling adjustment of a photo from its stored faces (the faces marked "not a face" are left out
/// by the caller).
pub fn assess(faces: &[CullFace], mode: EyeMode) -> FaceQuality {
    let Some(primary) = primary_of(faces) else { return FaceQuality::none() };
    let pose = ((primary.frontal - 0.5) * 0.08).clamp(-0.04, 0.04);
    let eye = eye_of(faces, mode);
    let eye_adjust = eye.map_or(0.0, |(label, _)| eye_penalty(label, mode));
    let adjust = (pose + eye_adjust).clamp(MIN_ADJUST, MAX_ADJUST);
    let mut factors = vec![CullFactor {
        id: "face_pose".into(),
        label: "Face pose".into(),
        detail: if pose >= 0.0 {
            format!("The main face looks toward the camera (frontal {:.2}): {:+.2}", primary.frontal, pose)
        } else {
            format!("The main face is turned away (frontal {:.2}): {:+.2}", primary.frontal, pose)
        },
        impact: "context".into(),
    }];
    if let Some((label, _)) = eye {
        let who = match mode {
            EyeMode::MainSubject => "main subject",
            EyeMode::AnyFace => "any face",
        };
        let (name, detail, impact) = match label {
            EyeLabel::Open => ("Eyes open", format!("Eyes open (counted: {who}): no change"), "context"),
            EyeLabel::Semi => ("Eyes half closed or looking down", format!("Half closed or looking down (counted: {who}): shown only, no effect on the score"), "context"),
            EyeLabel::Closed => ("Eyes closed or looking down", format!("Eyes closed or looking down: {eye_adjust:+.2} when eyes rank, not a hard rule (counted: {who})"), "supporting"),
        };
        factors.push(CullFactor { id: "eye_state".into(), label: name.into(), detail, impact: impact.into() });
    }
    FaceQuality { faces: faces.len() as u32, primary: Some(primary.id), frontal: Some(primary.frontal), eye: eye.map(|e| e.0), eye_face: eye.map(|e| e.1), adjust, pose_adjust: pose.clamp(MIN_ADJUST, MAX_ADJUST), factors }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(id: i64, px: f32, frontal: f32, eye: Option<EyeLabel>) -> CullFace {
        // a box whose width and height follow its pixel width in a 1000 px picture
        let w = px / 1000.0;
        CullFace { id, w, h: w * 1.3, px, frontal, eye }
    }

    #[test]
    fn no_faces_change_nothing() {
        let q = assess(&[], EyeMode::MainSubject);
        assert_eq!((q.faces, q.primary, q.adjust, q.eye), (0, None, 0.0, None));
        assert!(q.factors.is_empty());
    }

    #[test]
    fn the_primary_face_is_the_most_frontal_and_biggest() {
        let faces = [face(1, 120.0, 0.6, None), face(2, 100.0, 0.95, None), face(3, 300.0, 0.3, None)];
        assert_eq!(primary_of(&faces).unwrap().id, 2);
        // frontal ties go to the larger face, a full tie to the first
        let tie = [face(7, 100.0, 0.8, None), face(8, 200.0, 0.8, None), face(9, 200.0, 0.8, None)];
        assert_eq!(primary_of(&tie).unwrap().id, 8);
    }

    #[test]
    fn pose_alone_is_small_and_signed() {
        let front = assess(&[face(1, 200.0, 1.0, None)], EyeMode::MainSubject);
        assert!((front.adjust - 0.04).abs() < 1e-6);
        let side = assess(&[face(1, 200.0, 0.0, None)], EyeMode::MainSubject);
        assert!((side.adjust + 0.04).abs() < 1e-6);
        let mid = assess(&[face(1, 200.0, 0.5, None)], EyeMode::MainSubject);
        assert!(mid.adjust.abs() < 1e-6);
        assert_eq!(front.factors.len(), 1, "no eye verdict, no eye factor");
        assert_eq!(front.factors[0].id, "face_pose");
    }

    #[test]
    fn closed_and_half_closed_eyes_of_the_main_subject_lower_the_score_within_the_limits() {
        let closed = assess(&[face(1, 200.0, 1.0, Some(EyeLabel::Closed))], EyeMode::MainSubject);
        assert!((closed.adjust - (0.04 - 0.15)).abs() < 1e-6, "{}", closed.adjust);
        let semi = assess(&[face(1, 200.0, 0.5, Some(EyeLabel::Semi))], EyeMode::MainSubject);
        assert!(semi.adjust.abs() < 1e-6, "half closed eyes never lower a score");
        let worst = assess(&[face(1, 200.0, 0.5, Some(EyeLabel::Closed))], EyeMode::MainSubject);
        assert!((worst.adjust + 0.15).abs() < 1e-6);
        for q in [&closed, &worst] {
            assert!(q.adjust >= MIN_ADJUST && q.adjust <= MAX_ADJUST);
            let eye = q.factors.iter().find(|f| f.id == "eye_state").unwrap();
            assert_eq!(eye.impact, "supporting");
        }
        let open = assess(&[face(1, 200.0, 0.5, Some(EyeLabel::Open))], EyeMode::MainSubject);
        assert_eq!(open.adjust, 0.0);
        assert_eq!(open.factors.iter().find(|f| f.id == "eye_state").unwrap().impact, "context");
    }

    #[test]
    fn the_adjustment_is_clamped() {
        // every input range together stays inside the limits
        for frontal in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for eye in [None, Some(EyeLabel::Open), Some(EyeLabel::Semi), Some(EyeLabel::Closed)] {
                for mode in [EyeMode::MainSubject, EyeMode::AnyFace] {
                    let q = assess(&[face(1, 200.0, frontal, eye)], mode);
                    assert!(q.adjust >= MIN_ADJUST && q.adjust <= MAX_ADJUST, "{frontal} {eye:?} {mode:?}: {}", q.adjust);
                }
            }
        }
    }

    #[test]
    fn tiny_and_profile_faces_never_have_an_eye_verdict() {
        let tiny = assess(&[face(1, 47.0, 0.9, Some(EyeLabel::Closed))], EyeMode::MainSubject);
        assert_eq!(tiny.eye, None);
        let profile = assess(&[face(1, 200.0, 0.49, Some(EyeLabel::Closed))], EyeMode::MainSubject);
        assert_eq!(profile.eye, None);
        assert!(!profile.factors.iter().any(|f| f.id == "eye_state"));
        for mode in [EyeMode::MainSubject, EyeMode::AnyFace] {
            let both = [face(1, 40.0, 0.9, Some(EyeLabel::Closed)), face(2, 200.0, 0.3, Some(EyeLabel::Closed))];
            assert_eq!(eye_of(&both, mode), None, "{mode:?}");
        }
        // exactly at the limits is eligible
        assert_eq!(eye_of(&[face(1, 80.0, 0.5, Some(EyeLabel::Closed))], EyeMode::MainSubject).map(|e| e.0), Some(EyeLabel::Closed));
    }

    #[test]
    fn main_subject_ignores_a_closed_background_face_and_any_face_counts_it_less() {
        // the primary face is open and large; a smaller frontal face beside it is closed
        let group = [face(1, 300.0, 0.95, Some(EyeLabel::Open)), face(2, 120.0, 0.8, Some(EyeLabel::Closed))];
        let main = assess(&group, EyeMode::MainSubject);
        assert_eq!((main.primary, main.eye, main.eye_face), (Some(1), Some(EyeLabel::Open), Some(1)));
        let any = assess(&group, EyeMode::AnyFace);
        assert_eq!((any.eye, any.eye_face), (Some(EyeLabel::Closed), Some(2)));
        let pose = ((0.95f32 - 0.5) * 0.08).clamp(-0.04, 0.04);
        assert!((any.adjust - (pose - 0.08)).abs() < 1e-6, "{}", any.adjust);
        assert!(any.adjust > main.adjust - 0.2 && any.adjust < main.adjust);
        // half closed counts nothing in any mode (the primary face here is the second, frontal 0.8: pose +0.024)
        let semi = assess(&[face(1, 300.0, 0.5, Some(EyeLabel::Open)), face(2, 120.0, 0.8, Some(EyeLabel::Semi))], EyeMode::AnyFace);
        assert!((semi.adjust - 0.024).abs() < 1e-6, "{}", semi.adjust);
    }

    #[test]
    fn a_non_primary_face_gets_no_verdict_in_main_subject_mode() {
        let group = [face(1, 300.0, 0.95, None), face(2, 120.0, 0.8, Some(EyeLabel::Closed))];
        let q = assess(&group, EyeMode::MainSubject);
        assert_eq!(q.eye, None, "the primary face has no verdict, the other must not stand in");
    }

    #[test]
    fn any_face_takes_the_worst_of_the_eligible_faces() {
        let group = [face(1, 100.0, 0.9, Some(EyeLabel::Semi)), face(2, 100.0, 0.9, Some(EyeLabel::Closed)), face(3, 100.0, 0.9, Some(EyeLabel::Open))];
        assert_eq!(eye_of(&group, EyeMode::AnyFace), Some((EyeLabel::Closed, 2)));
    }

    #[test]
    fn labels_round_trip_text_and_serialize_lower_case() {
        for l in [EyeLabel::Open, EyeLabel::Semi, EyeLabel::Closed] {
            assert_eq!(EyeLabel::parse(l.as_str()), Some(l));
            assert_eq!(serde_json::to_string(&l).unwrap(), format!("\"{}\"", l.as_str()));
        }
        assert_eq!(EyeLabel::parse("unknown"), None);
        assert_eq!(serde_json::from_str::<EyeMode>("\"anyFace\"").unwrap(), EyeMode::AnyFace);
    }
}

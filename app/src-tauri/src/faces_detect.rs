// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The pure part of face detection and recognition: everything around the three ONNX graphs that
//! the engine runs (engine/faces.c). YuNet 2023mar finds faces and five landmarks on a 640 px
//! letterbox; SFace turns a face aligned to the ArcFace 112 px template into a 128-d embedding;
//! OCEC says whether an eye is open. The constants are those of the old app (notes/faces-port-inventory.md).
use image::{imageops::FilterType, RgbImage};

pub const DET_EDGE: u32 = 640;
pub const DET_SCORE: f32 = 0.65;
pub const NMS_IOU: f32 = 0.30;
pub const MIN_BOX_PX: f32 = 16.0;
pub const MIN_ASPECT: f32 = 0.30;
pub const MAX_ASPECT: f32 = 1.90;
pub const MIN_EYE_FRACTION: f32 = 0.07;
pub const ALIGN_MIN_EYE_FRACTION: f32 = 0.12;
pub const EMBED_EDGE: u32 = 112;
pub const EMBED_DIMS: usize = 128;
pub const EYE_W: u32 = 40;
pub const EYE_H: u32 = 24;

/// A detected face; every coordinate is normalised to the picture (0..1, top-left origin).
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    /// right eye, left eye (of the person, so the image-left one first), nose, right mouth corner, left mouth corner
    pub lm: [(f32, f32); 5],
}

/// How the picture sits in the 640 px square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    pub scale: f32,
    pub off_x: u32,
    pub off_y: u32,
    pub w: u32,
    pub h: u32,
}

pub fn letterbox_for(w: u32, h: u32) -> Letterbox {
    let scale = (DET_EDGE as f32 / w as f32).min(DET_EDGE as f32 / h as f32);
    let nw = ((w as f32 * scale).round() as u32).clamp(1, DET_EDGE);
    let nh = ((h as f32 * scale).round() as u32).clamp(1, DET_EDGE);
    Letterbox { scale, off_x: (DET_EDGE - nw) / 2, off_y: (DET_EDGE - nh) / 2, w: nw, h: nh }
}

/// The YuNet input: 1x3x640x640, BGR, raw 0..255, the picture scaled to fit and centred on zeros.
pub fn detector_input(img: &RgbImage) -> (Vec<f32>, Letterbox) {
    let lb = letterbox_for(img.width(), img.height());
    let small = image::imageops::resize(img, lb.w, lb.h, FilterType::Triangle);
    let plane = (DET_EDGE * DET_EDGE) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (x, y, p) in small.enumerate_pixels() {
        let i = ((y + lb.off_y) * DET_EDGE + x + lb.off_x) as usize;
        out[i] = f32::from(p[2]);
        out[plane + i] = f32::from(p[1]);
        out[2 * plane + i] = f32::from(p[0]);
    }
    (out, lb)
}

/// The twelve YuNet outputs by name (cls_8 ... kps_32), each flat.
pub struct DetOutputs<'a> {
    pub cls: [&'a [f32]; 3],
    pub obj: [&'a [f32]; 3],
    pub bbox: [&'a [f32]; 3],
    pub kps: [&'a [f32]; 3],
}

const STRIDES: [u32; 3] = [8, 16, 32];

/// Decodes the three feature maps into boxes of the original picture (before NMS and filters).
pub fn decode(out: &DetOutputs, lb: &Letterbox, img_w: u32, img_h: u32, min_score: f32) -> Vec<Face> {
    let mut faces = Vec::new();
    for (level, &stride) in STRIDES.iter().enumerate() {
        let cols = DET_EDGE / stride;
        let n = (cols * cols) as usize;
        if out.cls[level].len() < n || out.obj[level].len() < n || out.bbox[level].len() < 4 * n || out.kps[level].len() < 10 * n {
            return Vec::new();
        }
        for idx in 0..n {
            let cls = out.cls[level][idx].clamp(0.0, 1.0);
            let obj = out.obj[level][idx].clamp(0.0, 1.0);
            let score = (cls * obj).sqrt();
            if score < min_score {
                continue;
            }
            let (r, c) = ((idx as u32 / cols) as f32, (idx as u32 % cols) as f32);
            let s = stride as f32;
            let b = &out.bbox[level][idx * 4..idx * 4 + 4];
            let (cx, cy) = ((c + b[0]) * s, (r + b[1]) * s);
            let (bw, bh) = (b[2].exp() * s, b[3].exp() * s);
            let un = |x: f32, y: f32| ((x - lb.off_x as f32) / lb.scale / img_w as f32, (y - lb.off_y as f32) / lb.scale / img_h as f32);
            let (x0, y0) = un(cx - bw / 2.0, cy - bh / 2.0);
            let (x1, y1) = un(cx + bw / 2.0, cy + bh / 2.0);
            let k = &out.kps[level][idx * 10..idx * 10 + 10];
            let mut lm = [(0f32, 0f32); 5];
            for (i, p) in lm.iter_mut().enumerate() {
                *p = un((k[2 * i] + c) * s, (k[2 * i + 1] + r) * s);
            }
            faces.push(Face { x: x0, y: y0, w: x1 - x0, h: y1 - y0, score, lm });
        }
    }
    faces
}

pub fn iou(a: &Face, b: &Face) -> f32 {
    let (ax1, ay1, bx1, by1) = (a.x + a.w, a.y + a.h, b.x + b.w, b.y + b.h);
    let iw = (ax1.min(bx1) - a.x.max(b.x)).max(0.0);
    let ih = (ay1.min(by1) - a.y.max(b.y)).max(0.0);
    let inter = iw * ih;
    let union = a.w * a.h + b.w * b.h - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Greedy non-maximum suppression: highest score first, a box survives when its IoU with every kept one is below `NMS_IOU`.
pub fn nms(mut faces: Vec<Face>) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for f in faces {
        if kept.iter().all(|k| iou(k, &f) < NMS_IOU) {
            kept.push(f);
        }
    }
    kept
}

pub fn eye_distance_px(f: &Face, img_w: u32, img_h: u32) -> f32 {
    let (dx, dy) = ((f.lm[1].0 - f.lm[0].0) * img_w as f32, (f.lm[1].1 - f.lm[0].1) * img_h as f32);
    dx.hypot(dy)
}

/// Drops boxes that are too small, too thin or too wide, or whose eyes are too close together for the box.
pub fn valid(f: &Face, img_w: u32, img_h: u32) -> bool {
    let (w, h) = (f.w * img_w as f32, f.h * img_h as f32);
    if !(w.is_finite() && h.is_finite()) || w < MIN_BOX_PX || h < MIN_BOX_PX {
        return false;
    }
    let aspect = w / h;
    if !(MIN_ASPECT..=MAX_ASPECT).contains(&aspect) {
        return false;
    }
    eye_distance_px(f, img_w, img_h) >= MIN_EYE_FRACTION * w
}

/// Decode, suppress, filter, clamp to the picture: the faces of one YuNet run.
pub fn faces_from_outputs(out: &DetOutputs, lb: &Letterbox, img_w: u32, img_h: u32) -> Vec<Face> {
    nms(decode(out, lb, img_w, img_h, DET_SCORE))
        .into_iter()
        .filter(|f| valid(f, img_w, img_h))
        .map(|mut f| {
            let (x1, y1) = ((f.x + f.w).min(1.0), (f.y + f.h).min(1.0));
            f.x = f.x.max(0.0);
            f.y = f.y.max(0.0);
            f.w = x1 - f.x;
            f.h = y1 - f.y;
            f
        })
        .filter(|f| f.w > 0.0 && f.h > 0.0)
        .collect()
}

/// How frontal the face is (1 looks into the camera), from the nose against the eyes' midpoint.
pub fn frontal_score(f: &Face) -> f32 {
    let (re, le, nose) = (f.lm[0], f.lm[1], f.lm[2]);
    let eye_dist = (le.0 - re.0).hypot(le.1 - re.1);
    if eye_dist <= 1e-6 {
        return 0.0;
    }
    let mid = (re.0 + le.0) / 2.0;
    (1.0 - 1.35 * (nose.0 - mid).abs() / eye_dist).clamp(0.0, 1.0)
}

/// The tilt of the eye line in degrees, 0 to 180. Needs the picture's size: the landmarks are normalised.
pub fn roll_degrees(f: &Face, img_w: u32, img_h: u32) -> f32 {
    let dx = (f.lm[1].0 - f.lm[0].0) * img_w as f32;
    let dy = (f.lm[1].1 - f.lm[0].1) * img_h as f32;
    dy.atan2(dx).to_degrees().abs()
}

// ---- alignment ----------------------------------------------------------------

/// The ArcFace 112 x 112 landmark template (same order as `Face::lm`).
pub const TEMPLATE: [(f32, f32); 5] = [(38.2946, 51.6963), (73.5318, 51.5014), (56.0252, 71.7366), (41.5493, 92.3655), (70.7299, 92.2041)];

/// A similarity transform `p -> [[a, -b], [b, a]] p + t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Similarity {
    pub a: f64,
    pub b: f64,
    pub tx: f64,
    pub ty: f64,
}
impl Similarity {
    pub fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        (self.a * p.0 - self.b * p.1 + self.tx, self.b * p.0 + self.a * p.1 + self.ty)
    }
    pub fn invert(&self) -> Option<Similarity> {
        let d = self.a * self.a + self.b * self.b;
        if d < 1e-12 {
            return None;
        }
        let (ia, ib) = (self.a / d, -self.b / d);
        Some(Similarity { a: ia, b: ib, tx: -(ia * self.tx - ib * self.ty), ty: -(ib * self.tx + ia * self.ty) })
    }
}

/// The least-squares similarity transform taking `src` onto `dst` (Umeyama, 2-D, rotation and uniform scale).
pub fn fit_similarity(src: &[(f64, f64)], dst: &[(f64, f64)]) -> Option<Similarity> {
    let n = src.len();
    if n < 2 || n != dst.len() {
        return None;
    }
    let (mut sx, mut sy, mut dx, mut dy) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..n {
        sx += src[i].0;
        sy += src[i].1;
        dx += dst[i].0;
        dy += dst[i].1;
    }
    let nf = n as f64;
    let (sx, sy, dx, dy) = (sx / nf, sy / nf, dx / nf, dy / nf);
    let (mut num_a, mut num_b, mut den) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (px, py) = (src[i].0 - sx, src[i].1 - sy);
        let (qx, qy) = (dst[i].0 - dx, dst[i].1 - dy);
        num_a += px * qx + py * qy;
        num_b += px * qy - py * qx;
        den += px * px + py * py;
    }
    if den < 1e-12 {
        return None;
    }
    let (a, b) = (num_a / den, num_b / den);
    Some(Similarity { a, b, tx: dx - (a * sx - b * sy), ty: dy - (b * sx + a * sy) })
}

fn sample(img: &RgbImage, x: f64, y: f64) -> [f32; 3] {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let at = |ix: i64, iy: i64| {
        let p = img.get_pixel(ix.clamp(0, w - 1) as u32, iy.clamp(0, h - 1) as u32);
        [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])]
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (p00, p10, p01, p11) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let mut out = [0f32; 3];
    for c in 0..3 {
        let top = p00[c] * (1.0 - fx) + p10[c] * fx;
        let bottom = p01[c] * (1.0 - fx) + p11[c] * fx;
        out[c] = top * (1.0 - fy) + bottom * fy;
    }
    out
}

/// The 112 x 112 face for SFace as a BGR 0..255 NCHW tensor, and whether it was aligned by the
/// landmarks (the eyes far enough apart and every landmark inside the picture) or cropped by the box.
pub fn embedding_input(img: &RgbImage, f: &Face) -> (Vec<f32>, bool) {
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    let box_w = f64::from(f.w) * iw;
    let landmarks_ok = f.lm.iter().all(|p| (0.0..=1.0).contains(&p.0) && (0.0..=1.0).contains(&p.1));
    let aligned = landmarks_ok && f64::from(eye_distance_px(f, img.width(), img.height())) >= f64::from(ALIGN_MIN_EYE_FRACTION) * box_w;
    let src: Vec<(f64, f64)> = f.lm.iter().map(|p| (f64::from(p.0) * iw, f64::from(p.1) * ih)).collect();
    let dst: Vec<(f64, f64)> = TEMPLATE.iter().map(|p| (f64::from(p.0), f64::from(p.1))).collect();
    let to_src = if aligned { fit_similarity(&src, &dst).and_then(|t| t.invert()) } else { None };
    let edge = EMBED_EDGE as usize;
    let plane = edge * edge;
    let mut out = vec![0f32; 3 * plane];
    let used = to_src.is_some();
    // crop fallback: the box with a 10 % margin onto the whole 112 x 112 square
    let (bx, by, bw, bh) = (f64::from(f.x) * iw, f64::from(f.y) * ih, box_w, f64::from(f.h) * ih);
    let m = 0.10;
    let (cx0, cy0, cw, ch) = (bx - bw * m, by - bh * m, bw * (1.0 + 2.0 * m), bh * (1.0 + 2.0 * m));
    for v in 0..edge {
        for u in 0..edge {
            let (px, py) = (u as f64 + 0.5, v as f64 + 0.5);
            let (sx, sy) = match &to_src {
                Some(t) => t.apply((px, py)),
                None => (cx0 + px / edge as f64 * cw, cy0 + py / edge as f64 * ch),
            };
            let rgb = sample(img, sx, sy);
            let i = v * edge + u;
            out[i] = rgb[2];
            out[plane + i] = rgb[1];
            out[2 * plane + i] = rgb[0];
        }
    }
    (out, used)
}

/// The ArcFace input from the aligned 112 px face of `embedding_input` (BGR, 0..255): RGB planes,
/// (v - 127.5) / 127.5, which is how InsightFace feeds w600k_r50 (a swapped-channel run separated
/// the same person from others slightly worse on the owner's photos).
pub fn arcface_input(bgr_0_255: &[f32]) -> Vec<f32> {
    let plane = (EMBED_EDGE * EMBED_EDGE) as usize;
    let mut out = vec![0f32; 3 * plane];
    for c in 0..3 {
        for i in 0..plane {
            out[c * plane + i] = (bgr_0_255[(2 - c) * plane + i] - 127.5) / 127.5;
        }
    }
    out
}

pub fn l2_normalise(v: &mut [f32]) -> bool {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !n.is_finite() || n < 1e-9 {
        return false;
    }
    for x in v.iter_mut() {
        *x /= n;
    }
    true
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ---- eye state ------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum EyeVerdict {
    Open,
    SemiClosed,
    Closed,
}

pub const OPEN_AT: f32 = 0.7;
pub const SEMI_AT: f32 = 0.3;

/// The verdict for one eye, or for a face from the worse of its two eyes.
pub fn eye_verdict(prob_open: f32) -> EyeVerdict {
    if prob_open >= OPEN_AT {
        EyeVerdict::Open
    } else if prob_open >= SEMI_AT {
        EyeVerdict::SemiClosed
    } else {
        EyeVerdict::Closed
    }
}

/// One eye as the OCEC input: 1x3x24x40, RGB, 0..1. The crop is centred on the landmark, 0.55 of the
/// eyes' distance wide and 24/40 of that tall.
pub fn eye_input(img: &RgbImage, f: &Face, right: bool) -> Option<Vec<f32>> {
    let ied = f64::from(eye_distance_px(f, img.width(), img.height()));
    if ied < 4.0 {
        return None;
    }
    let p = f.lm[usize::from(!right)];
    let (cx, cy) = (f64::from(p.0) * img.width() as f64, f64::from(p.1) * img.height() as f64);
    let cw = 0.55 * ied;
    let ch = cw * f64::from(EYE_H) / f64::from(EYE_W);
    let (w, h) = (EYE_W as usize, EYE_H as usize);
    let mut out = vec![0f32; 3 * w * h];
    for v in 0..h {
        for u in 0..w {
            let sx = cx - cw / 2.0 + (u as f64 + 0.5) / w as f64 * cw;
            let sy = cy - ch / 2.0 + (v as f64 + 0.5) / h as f64 * ch;
            let rgb = sample(img, sx, sy);
            for c in 0..3 {
                out[c * w * h + v * w + u] = rgb[c] / 255.0;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(x: f32, y: f32, w: f32, h: f32, score: f32) -> Face {
        let mid = (x + w / 2.0, y + h * 0.4);
        Face {
            x,
            y,
            w,
            h,
            score,
            lm: [(mid.0 - w * 0.2, mid.1), (mid.0 + w * 0.2, mid.1), (mid.0, y + h * 0.6), (mid.0 - w * 0.15, y + h * 0.8), (mid.0 + w * 0.15, y + h * 0.8)],
        }
    }

    #[test]
    fn letterbox_fits_and_centres() {
        let lb = letterbox_for(1024, 512);
        assert_eq!((lb.w, lb.h, lb.off_x, lb.off_y), (640, 320, 0, 160));
        assert!((lb.scale - 0.625).abs() < 1e-6);
        let tall = letterbox_for(300, 600);
        assert_eq!((tall.w, tall.h, tall.off_x, tall.off_y), (320, 640, 160, 0));
    }

    #[test]
    fn detector_input_is_bgr_raw_and_pads_with_zeros() {
        let mut img = RgbImage::new(64, 32);
        for p in img.pixels_mut() {
            *p = image::Rgb([10, 20, 30]);
        }
        let (t, lb) = detector_input(&img);
        assert_eq!(t.len(), 3 * 640 * 640);
        assert_eq!(lb.off_y, 160);
        assert_eq!(t[0], 0.0, "padding is zero");
        let i = (320 * 640 + 320) as usize;
        assert_eq!((t[i], t[640 * 640 + i], t[2 * 640 * 640 + i]), (30.0, 20.0, 10.0), "B, G, R planes, 0..255");
    }

    fn r(v: &[Vec<f32>]) -> [&[f32]; 3] {
        [v[0].as_slice(), v[1].as_slice(), v[2].as_slice()]
    }

    fn maps(level: usize, idx: usize, score_sq: f32) -> (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<Vec<f32>>) {
        let sizes = [6400usize, 1600, 400];
        let cls: Vec<Vec<f32>> = sizes.iter().enumerate().map(|(l, &n)| { let mut v = vec![0.0; n]; if l == level { v[idx] = score_sq; } v }).collect();
        let obj: Vec<Vec<f32>> = sizes.iter().enumerate().map(|(l, &n)| { let mut v = vec![0.0; n]; if l == level { v[idx] = score_sq; } v }).collect();
        let bbox: Vec<Vec<f32>> = sizes.iter().map(|&n| vec![0.0; n * 4]).collect();
        let kps: Vec<Vec<f32>> = sizes.iter().map(|&n| vec![0.0; n * 10]).collect();
        (cls, obj, bbox, kps)
    }

    #[test]
    fn decode_maps_a_cell_back_through_the_letterbox() {
        // stride 16, cell (row 10, col 20) of a 40-wide map; a 4 cell wide and tall box with centre offset (0.5, 0.5)
        let idx = 10 * 40 + 20;
        let (cls, obj, mut bbox, mut kps) = maps(1, idx, 0.9);
        bbox[1][idx * 4..idx * 4 + 4].copy_from_slice(&[0.5, 0.5, (4.0f32).ln(), (4.0f32).ln()]);
        kps[1][idx * 10..idx * 10 + 10].copy_from_slice(&[-1.0, 0.0, 1.0, 0.0, 0.0, 1.0, -0.5, 2.0, 0.5, 2.0]);
        let out = DetOutputs { cls: r(&cls), obj: r(&obj), bbox: r(&bbox), kps: r(&kps) };
        let lb = letterbox_for(1280, 1280); // scale 0.5, no offset
        let faces = decode(&out, &lb, 1280, 1280, DET_SCORE);
        assert_eq!(faces.len(), 1);
        let f = &faces[0];
        // centre (20.5*16, 10.5*16) = (328, 168) in the square, size 64: x 296..360 -> picture x2
        assert!((f.x - 592.0 / 1280.0).abs() < 1e-5 && (f.w - 128.0 / 1280.0).abs() < 1e-5, "{f:?}");
        assert!((f.y - 272.0 / 1280.0).abs() < 1e-5 && (f.h - 128.0 / 1280.0).abs() < 1e-5);
        assert!((f.lm[0].0 - (19.0 * 16.0 * 2.0) / 1280.0).abs() < 1e-5 && (f.lm[1].0 - (21.0 * 16.0 * 2.0) / 1280.0).abs() < 1e-5);
        assert!((f.score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn decode_drops_scores_below_the_review_threshold_and_clamps_outputs() {
        let (cls, obj, bbox, kps) = maps(0, 100, 0.64);
        let out = DetOutputs { cls: r(&cls), obj: r(&obj), bbox: r(&bbox), kps: r(&kps) };
        assert!(decode(&out, &letterbox_for(640, 640), 640, 640, DET_SCORE).is_empty());
        let (mut cls, obj, bbox, kps) = maps(0, 100, 0.9);
        cls[0][100] = 7.0; // clamped to 1: sqrt(1 * 0.9)
        let out = DetOutputs { cls: r(&cls), obj: r(&obj), bbox: r(&bbox), kps: r(&kps) };
        let f = decode(&out, &letterbox_for(640, 640), 640, 640, DET_SCORE);
        assert!((f[0].score - (0.9f32).sqrt()).abs() < 1e-6);
    }

    #[test]
    fn nms_keeps_the_best_of_overlapping_boxes_and_separate_ones() {
        let a = face(0.1, 0.1, 0.2, 0.2, 0.9);
        let b = face(0.11, 0.11, 0.2, 0.2, 0.8); // iou ~ 0.8
        let c = face(0.6, 0.6, 0.2, 0.2, 0.7);
        let d = face(0.25, 0.1, 0.2, 0.2, 0.6); // overlaps a a little: iou ~ 0.14 < 0.30 stays
        let kept = nms(vec![b.clone(), c.clone(), a.clone(), d.clone()]);
        assert_eq!(kept.iter().map(|f| f.score).collect::<Vec<_>>(), vec![0.9, 0.7, 0.6]);
        assert!((iou(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn validity_filters() {
        let ok = face(0.4, 0.4, 0.1, 0.12, 0.9);
        assert!(valid(&ok, 1000, 1000));
        assert!(!valid(&ok, 100, 100), "10 px wide is below 16 px");
        let thin = face(0.4, 0.4, 0.02, 0.12, 0.9);
        assert!(!valid(&thin, 1000, 1000), "aspect 0.17 < 0.30");
        let wide = face(0.2, 0.4, 0.3, 0.1, 0.9);
        assert!(!valid(&wide, 1000, 1000), "aspect 3 > 1.90");
        let mut close = ok.clone();
        close.lm[0].0 = close.lm[1].0 - 0.001; // eyes 1 px apart in 1000 px: < 0.07 of the box
        assert!(!valid(&close, 1000, 1000));
    }

    #[test]
    fn pose_scores() {
        let f = face(0.4, 0.4, 0.2, 0.2, 0.9);
        assert!((frontal_score(&f) - 1.0).abs() < 1e-6, "nose between the eyes");
        let mut turned = f.clone();
        turned.lm[2].0 += (turned.lm[1].0 - turned.lm[0].0) * 0.5;
        assert!((frontal_score(&turned) - (1.0 - 1.35 * 0.5)).abs() < 1e-5, "{}", frontal_score(&turned));
        let mut profile = f.clone();
        profile.lm[2].0 += (profile.lm[1].0 - profile.lm[0].0) * 2.0;
        assert_eq!(frontal_score(&profile), 0.0);
        assert!(roll_degrees(&f, 1000, 1000) < 1e-3);
        let mut tilted = f.clone();
        tilted.lm[1].1 += (tilted.lm[1].0 - tilted.lm[0].0);
        assert!((roll_degrees(&tilted, 1000, 1000) - 45.0).abs() < 1e-3);
    }

    #[test]
    fn similarity_recovers_a_known_transform_and_inverts() {
        let truth = Similarity { a: 0.8 * 0.5f64.cos(), b: 0.8 * 0.5f64.sin(), tx: 12.0, ty: -7.0 };
        let src: Vec<(f64, f64)> = vec![(10.0, 20.0), (50.0, 22.0), (30.0, 40.0), (15.0, 60.0), (45.0, 61.0)];
        let dst: Vec<(f64, f64)> = src.iter().map(|&p| truth.apply(p)).collect();
        let fit = fit_similarity(&src, &dst).unwrap();
        for (x, y) in [(fit.a - truth.a, 0.0), (fit.b - truth.b, 0.0), (fit.tx - truth.tx, 0.0), (fit.ty - truth.ty, 0.0)] {
            assert!(x.abs() < 1e-9 && y == 0.0);
        }
        let back = fit.invert().unwrap().apply(fit.apply((3.0, 4.0)));
        assert!((back.0 - 3.0).abs() < 1e-9 && (back.1 - 4.0).abs() < 1e-9);
        assert!(fit_similarity(&[(1.0, 1.0)], &[(1.0, 1.0)]).is_none());
        assert!(fit_similarity(&[(1.0, 1.0), (1.0, 1.0)], &[(0.0, 0.0), (1.0, 1.0)]).is_none());
    }

    #[test]
    fn alignment_puts_the_landmarks_on_the_template() {
        // a picture whose pixel (x, y) encodes its position; the face is tilted and scaled
        let (w, h) = (400u32, 300u32);
        let mut img = RgbImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, 0]);
        }
        let t = Similarity { a: 1.5 * 0.2f64.cos(), b: 1.5 * 0.2f64.sin(), tx: 120.0, ty: 60.0 };
        let mut f = face(0.3, 0.2, 0.3, 0.4, 0.9);
        for (i, p) in TEMPLATE.iter().enumerate() {
            let q = t.apply((f64::from(p.0), f64::from(p.1)));
            f.lm[i] = ((q.0 / f64::from(w)) as f32, (q.1 / f64::from(h)) as f32);
        }
        f.x = 0.2;
        f.y = 0.1;
        f.w = 0.5;
        f.h = 0.7;
        let (tensor, aligned) = embedding_input(&img, &f);
        assert!(aligned);
        // the aligned crop at template point k samples the picture at landmark k: check the red (x) plane at the nose
        let plane = 112 * 112;
        let (u, v) = (TEMPLATE[2].0 as usize, TEMPLATE[2].1 as usize);
        let red = tensor[2 * plane + v * 112 + u];
        let expect = f.lm[2].0 * 255.0;
        assert!((red - expect).abs() < 2.5, "red {red} vs {expect}");
        // BGR order: blue plane is zero, red plane carries x
        assert!(tensor[0].abs() < 1e-3);
    }

    #[test]
    fn a_face_with_close_eyes_or_landmarks_outside_is_cropped_by_its_box() {
        let img = RgbImage::from_pixel(200, 200, image::Rgb([9, 9, 9]));
        let mut f = face(0.3, 0.3, 0.4, 0.4, 0.9);
        f.lm[0].0 = f.lm[1].0 - 0.001;
        assert!(!embedding_input(&img, &f).1);
        let mut g = face(0.3, 0.3, 0.4, 0.4, 0.9);
        g.lm[4].1 = 1.2;
        let (t, aligned) = embedding_input(&img, &g);
        assert!(!aligned && t.len() == 3 * 112 * 112 && (t[5] - 9.0).abs() < 1e-3);
    }

    #[test]
    fn arcface_input_is_rgb_and_scaled() {
        let plane = 112 * 112;
        let mut bgr = vec![0f32; 3 * plane];
        bgr[0] = 255.0; // blue of pixel 0
        bgr[plane] = 127.5; // green
        bgr[2 * plane] = 0.0; // red
        let t = arcface_input(&bgr);
        assert_eq!((t[0], t[plane], t[2 * plane]), (-1.0, 0.0, 1.0), "R, G, B planes in (v - 127.5) / 127.5");
    }

    #[test]
    fn l2_and_cosine() {
        let mut v = vec![3.0, 4.0];
        assert!(l2_normalise(&mut v));
        assert!((v[0] - 0.6).abs() < 1e-6 && (cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert!(!l2_normalise(&mut [0.0, 0.0]));
        assert!(!l2_normalise(&mut [f32::NAN, 1.0]));
    }

    #[test]
    fn eye_crops_and_verdicts() {
        let mut img = RgbImage::from_pixel(300, 300, image::Rgb([0, 0, 0]));
        // a white patch where the right eye is
        let f = face(0.3, 0.3, 0.4, 0.4, 0.9);
        let (ex, ey) = ((f.lm[0].0 * 300.0) as u32, (f.lm[0].1 * 300.0) as u32);
        for y in ey - 3..ey + 3 {
            for x in ex - 6..ex + 6 {
                img.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
        let t = eye_input(&img, &f, true).unwrap();
        assert_eq!(t.len(), 3 * 24 * 40);
        assert!(t[12 * 40 + 20] > 0.9, "centre of the crop is the patch");
        let left = eye_input(&img, &f, false).unwrap();
        assert!(left[12 * 40 + 20] < 0.1);
        assert_eq!((eye_verdict(0.9), eye_verdict(0.7), eye_verdict(0.5), eye_verdict(0.3), eye_verdict(0.29)),
            (EyeVerdict::Open, EyeVerdict::Open, EyeVerdict::SemiClosed, EyeVerdict::SemiClosed, EyeVerdict::Closed));
        let mut tiny = f.clone();
        tiny.lm[0].0 = tiny.lm[1].0;
        assert!(eye_input(&img, &tiny, true).is_none());
    }
}

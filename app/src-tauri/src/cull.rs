// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Auto-culling image analysis: sharpness, subject focus, exposure, the
//! blurriest region and a perceptual hash, ported from the RapidRAW fork's
//! culling.rs. Pure functions over decoded pixels, with no engine, file or
//! GUI access, so they run on any worker thread and are tested with
//! synthetic images. Subject detection (U-2-Net) is not part of this build:
//! the subject fields stay `Option` so a later detector can fill them
//! without changing the stored format.
use image::{imageops, DynamicImage, GrayImage, RgbImage};
use image_hasher::{HashAlg, HasherConfig, ImageHash};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::VecDeque;

/// The longest edge the metrics are measured at, as the old app did, so the
/// sharpness thresholds mean the same here.
pub const ANALYSIS_EDGE: u32 = 720;

const WEIGHT_SHARPNESS: f64 = 0.38;
const WEIGHT_CENTER_FOCUS: f64 = 0.32;
const WEIGHT_EXPOSURE: f64 = 0.22;
// A minor cue: rule-of-thirds placement is not a universal rule, so it only
// breaks ties between technically comparable frames with a confident subject.
const WEIGHT_SUBJECT_COMPOSITION: f64 = 0.08;

/// A normalized (0-1) rectangle.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullBlurRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What one photo measured. `Option` fields need a subject mask and are
/// `None` until one is available.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullAnalysis {
    pub quality_score: f64,
    pub sharpness_metric: f64,
    pub center_focus_metric: f64,
    pub subject_focus_metric: Option<f64>,
    pub subject_composition_metric: Option<f64>,
    pub subject_edge_contact_ratio: Option<f64>,
    pub exposure_metric: f64,
    /// the pixel size of the image that was analysed, not of the original
    pub width: u32,
    pub height: u32,
    /// the softest textured part of the frame (see `find_blurriest_region`)
    pub blurry_region: Option<CullBlurRegion>,
    /// the crispest textured part of the frame (see `find_sharpest_region`); None in an
    /// analysis cached before it was measured
    #[serde(default)]
    pub sharp_region: Option<CullBlurRegion>,
}

/// An analysis and the perceptual hash that groups it with look-alikes.
#[derive(Clone, Debug)]
pub struct Analysed {
    pub hash: ImageHash,
    pub result: CullAnalysis,
}

pub(crate) fn hasher() -> image_hasher::Hasher {
    HasherConfig::new().hash_alg(HashAlg::DoubleGradient).hash_size(16, 16).to_hasher()
}

pub fn hash_to_string(hash: &ImageHash) -> String {
    hash.to_base64()
}

pub fn hash_from_string(text: &str) -> Option<ImageHash> {
    ImageHash::from_base64(text).ok()
}

fn laplacian_values(image: &GrayImage, keep: impl Fn(u32, u32) -> bool) -> Vec<f64> {
    let (width, height) = image.dimensions();
    if width < 3 || height < 3 {
        return Vec::new();
    }
    let px = |x: u32, y: u32| i32::from(image.get_pixel(x, y)[0]);
    let mut values = Vec::with_capacity(((width - 2) * (height - 2)) as usize);
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            if keep(x, y) {
                values.push(f64::from(px(x, y - 1) + px(x, y + 1) + px(x - 1, y) + px(x + 1, y) - 4 * px(x, y)));
            }
        }
    }
    values
}

fn variance(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
}

/// The variance of the Laplacian: high for crisp edges, low for blur.
pub fn laplacian_variance(image: &GrayImage) -> f64 {
    variance(&laplacian_values(image, |_, _| true))
}

/// The region most likely behind a "blurry" verdict: the softest cell of a
/// 4x4 grid among those with enough texture to judge focus at all (a flat sky
/// is always low-variance without being out of focus). `None` for images too
/// small to grid, or where every cell is flat.
pub fn find_blurriest_region(image: &GrayImage) -> Option<CullBlurRegion> {
    const GRID: u32 = 4;
    const MIN_TEXTURE_STDDEV: f64 = 4.0;
    let (width, height) = image.dimensions();
    if width < GRID * 4 || height < GRID * 4 {
        return None;
    }
    let (cell_w, cell_h) = (width / GRID, height / GRID);
    let mut softest: Option<(f64, u32, u32)> = None;
    for row in 0..GRID {
        for col in 0..GRID {
            let (x, y) = (col * cell_w, row * cell_h);
            let w = if col == GRID - 1 { width - x } else { cell_w };
            let h = if row == GRID - 1 { height - y } else { cell_h };
            let cell = imageops::crop_imm(image, x, y, w, h).to_image();
            let pixels: Vec<f64> = cell.pixels().map(|p| f64::from(p[0])).collect();
            if pixels.is_empty() || variance(&pixels).sqrt() < MIN_TEXTURE_STDDEV {
                continue;
            }
            let sharpness = laplacian_variance(&cell);
            if softest.is_none_or(|(best, _, _)| sharpness < best) {
                softest = Some((sharpness, col, row));
            }
        }
    }
    softest.map(|(_, col, row)| CullBlurRegion {
        x: f64::from(col * cell_w) / f64::from(width),
        y: f64::from(row * cell_h) / f64::from(height),
        width: f64::from(cell_w) / f64::from(width),
        height: f64::from(cell_h) / f64::from(height),
    })
}

/// The part of the frame to look at for focus: the crispest cell of an 8x8 grid (the highest
/// Laplacian variance, among cells with enough texture to judge), widened to the 3x3 cells around
/// it and kept inside the frame, as a rectangle of fractions of the image. Ties go to the first
/// cell in reading order, so the same pixels always give the same rectangle. `None` for images too
/// small to grid, or where every cell is flat.
pub fn find_sharpest_region(image: &GrayImage) -> Option<CullBlurRegion> {
    const GRID: u32 = 8;
    const SPAN: u32 = 3;
    const MIN_TEXTURE_STDDEV: f64 = 4.0;
    let (width, height) = image.dimensions();
    if width < GRID * 4 || height < GRID * 4 {
        return None;
    }
    let (cell_w, cell_h) = (width / GRID, height / GRID);
    let mut sharpest: Option<(f64, u32, u32)> = None;
    for row in 0..GRID {
        for col in 0..GRID {
            let (x, y) = (col * cell_w, row * cell_h);
            let w = if col == GRID - 1 { width - x } else { cell_w };
            let h = if row == GRID - 1 { height - y } else { cell_h };
            let cell = imageops::crop_imm(image, x, y, w, h).to_image();
            let pixels: Vec<f64> = cell.pixels().map(|p| f64::from(p[0])).collect();
            if pixels.is_empty() || variance(&pixels).sqrt() < MIN_TEXTURE_STDDEV {
                continue;
            }
            let crisp = laplacian_variance(&cell);
            if sharpest.is_none_or(|(best, _, _)| crisp > best) {
                sharpest = Some((crisp, col, row));
            }
        }
    }
    sharpest.map(|(_, col, row)| {
        let left = col.saturating_sub(SPAN / 2).min(GRID - SPAN);
        let top = row.saturating_sub(SPAN / 2).min(GRID - SPAN);
        CullBlurRegion {
            x: f64::from(left * cell_w) / f64::from(width),
            y: f64::from(top * cell_h) / f64::from(height),
            width: f64::from(SPAN * cell_w) / f64::from(width),
            height: f64::from(SPAN * cell_h) / f64::from(height),
        }
    })
}

/// 1 for a frame without clipped shadows or highlights, falling as the
/// share of near-black (under 5) and near-white (250 and up) pixels grows.
pub fn exposure_metric(image: &GrayImage) -> f64 {
    let total = u64::from(image.width()) * u64::from(image.height());
    if total == 0 {
        return 0.0;
    }
    let (mut dark, mut bright) = (0u64, 0u64);
    for p in image.pixels() {
        if p[0] < 5 {
            dark += 1;
        } else if p[0] >= 250 {
            bright += 1;
        }
    }
    (1.0 - (dark as f64 / total as f64) * 5.0 - (bright as f64 / total as f64) * 5.0).max(0.0)
}

// Subject-mask helpers. Nothing calls them until a detector exists, but they
// are ported with their tests so the contract (a 0-255 mask the size of the
// image, 128 and up is subject) is kept.
const SUBJECT_MASK_THRESHOLD: u8 = 128;
const MIN_SUBJECT_PIXELS: usize = 64;

/// Laplacian variance over the mask's foreground only; `None` when the mask
/// is a different size or too small to trust.
#[allow(dead_code)]
pub fn masked_laplacian_variance(image: &GrayImage, mask: &GrayImage) -> Option<f64> {
    if image.dimensions() != mask.dimensions() {
        return None;
    }
    let values = laplacian_values(image, |x, y| mask.get_pixel(x, y)[0] >= SUBJECT_MASK_THRESHOLD);
    (values.len() >= MIN_SUBJECT_PIXELS).then(|| variance(&values))
}

/// Exposure measured inside the subject only, so a dark background does not
/// cost a well-exposed subject its score.
#[allow(dead_code)]
pub fn masked_exposure_metric(image: &GrayImage, mask: &GrayImage) -> Option<f64> {
    if image.dimensions() != mask.dimensions() {
        return None;
    }
    let (mut subject, mut dark, mut bright) = (0usize, 0usize, 0usize);
    for (pixel, m) in image.pixels().zip(mask.pixels()) {
        if m[0] < SUBJECT_MASK_THRESHOLD {
            continue;
        }
        subject += 1;
        if pixel[0] < 5 {
            dark += 1;
        } else if pixel[0] >= 250 {
            bright += 1;
        }
    }
    (subject >= MIN_SUBJECT_PIXELS)
        .then(|| (1.0 - (dark as f64 / subject as f64) * 5.0 - (bright as f64 / subject as f64) * 5.0).max(0.0))
}

/// A thirds-placement score and the share of the subject touching the frame
/// edge, from a subject mask. A minor cue, not an aesthetic classifier.
#[allow(dead_code)]
pub fn subject_composition_metrics(mask: &GrayImage) -> Option<(f64, f64)> {
    let (width, height) = mask.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let margin_x = ((width as f32 * 0.04).round() as u32).max(1);
    let margin_y = ((height as f32 * 0.04).round() as u32).max(1);
    let (mut pixels, mut edge, mut sum_x, mut sum_y) = (0usize, 0usize, 0f64, 0f64);
    for y in 0..height {
        for x in 0..width {
            if mask.get_pixel(x, y)[0] < SUBJECT_MASK_THRESHOLD {
                continue;
            }
            pixels += 1;
            sum_x += f64::from(x);
            sum_y += f64::from(y);
            if x < margin_x || x >= width.saturating_sub(margin_x) || y < margin_y || y >= height.saturating_sub(margin_y) {
                edge += 1;
            }
        }
    }
    if pixels < MIN_SUBJECT_PIXELS {
        return None;
    }
    let cx = sum_x / pixels as f64 / f64::from(width);
    let cy = sum_y / pixels as f64 / f64::from(height);
    let nearest = [1.0 / 3.0, 2.0 / 3.0]
        .into_iter()
        .flat_map(|tx| [1.0 / 3.0, 2.0 / 3.0].into_iter().map(move |ty| ((cx - tx).powi(2) + (cy - ty).powi(2)).sqrt()))
        .fold(f64::INFINITY, f64::min);
    let thirds = (1.0 - nearest / 0.5).clamp(0.0, 1.0);
    let contact = edge as f64 / pixels as f64;
    Some(((thirds * 0.7 + (1.0 - contact) * 0.3).clamp(0.0, 1.0), contact))
}

/// Measures one decoded photo. The image is shrunk to `ANALYSIS_EDGE` (never
/// enlarged), so the numbers do not depend on the size the preview came in.
pub fn analyze(image: &RgbImage) -> Analysed {
    let dynamic = DynamicImage::ImageRgb8(image.clone());
    let (w, h) = image.dimensions();
    let thumbnail = if w.max(h) > ANALYSIS_EDGE { dynamic.thumbnail(ANALYSIS_EDGE, ANALYSIS_EDGE) } else { dynamic };
    let gray = thumbnail.to_luma8();
    let sharpness_metric = laplacian_variance(&gray);
    let exposure = exposure_metric(&gray);
    let blurry_region = find_blurriest_region(&gray);
    let sharp_region = find_sharpest_region(&gray);
    let (tw, th) = gray.dimensions();
    let center = imageops::crop_imm(&gray, tw / 4, th / 4, tw / 2, th / 2).to_image();
    let center_focus_metric = laplacian_variance(&center);
    let normalize = |metric: f64| ((metric + 1.0).log10() / 3.5).min(1.0);
    let baseline = normalize(sharpness_metric) * WEIGHT_SHARPNESS
        + normalize(center_focus_metric) * WEIGHT_CENTER_FOCUS
        + exposure * WEIGHT_EXPOSURE;
    // without a subject mask the composition weight is spread over the rest,
    // so a landscape is not penalized for having no compact subject
    let quality_score = baseline / (1.0 - WEIGHT_SUBJECT_COMPOSITION);
    Analysed {
        hash: hasher().hash_image(&thumbnail),
        result: CullAnalysis {
            quality_score,
            sharpness_metric,
            center_focus_metric,
            subject_focus_metric: None,
            subject_composition_metric: None,
            subject_edge_contact_ratio: None,
            exposure_metric: exposure,
            width: tw,
            height: th,
            blurry_region,
            sharp_region,
        },
    }
}

/// One group of look-alikes: indices into the analysed list, best first.
pub struct Group {
    pub members: Vec<usize>,
}

/// Groups photos whose hashes are within `threshold` bits of each other,
/// transitively (a chain of near-matches is one burst), best quality first.
/// Singletons are not groups.
pub fn group_similar(items: &[(ImageHash, f64)], threshold: u32) -> Vec<Group> {
    let mut done = vec![false; items.len()];
    let mut groups = Vec::new();
    for start in 0..items.len() {
        if done[start] {
            continue;
        }
        done[start] = true;
        let mut members = vec![start];
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            for other in 0..items.len() {
                if !done[other] && items[current].0.dist(&items[other].0) <= threshold {
                    done[other] = true;
                    members.push(other);
                    queue.push_back(other);
                }
            }
        }
        if members.len() > 1 {
            members.sort_by(|&a, &b| items[b].1.partial_cmp(&items[a].1).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
            groups.push(Group { members });
        }
    }
    groups
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{GrayImage, Luma, Rgb};

    /// a checkerboard-and-gradient scene with plenty of edges
    pub fn sharp_scene(w: u32, h: u32, seed: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            let cell = ((x / 4 + y / 4 + seed) % 2) as u8;
            let v: u8 = if cell == 0 { 40 } else { 215 };
            let g = ((x * 255 / w) as u8) / 4;
            Rgb([v.saturating_add(g / 2), v, v.saturating_sub(g / 2)])
        })
    }

    #[test]
    fn subject_geometry_penalizes_edge_contact() {
        let mut centered = GrayImage::new(90, 90);
        let mut edge = GrayImage::new(90, 90);
        for y in 25..45 {
            for x in 25..45 {
                centered.put_pixel(x, y, Luma([255]));
            }
            for x in 0..20 {
                edge.put_pixel(x, y, Luma([255]));
            }
        }
        let (centered_score, centered_contact) = subject_composition_metrics(&centered).unwrap();
        let (edge_score, edge_contact) = subject_composition_metrics(&edge).unwrap();
        assert!(edge_contact > centered_contact);
        assert!(centered_score > edge_score);
    }

    #[test]
    fn blur_lowers_sharpness_and_score() {
        let sharp = sharp_scene(400, 300, 0);
        let blurred = image::imageops::blur(&sharp, 4.0);
        let (a, b) = (analyze(&sharp), analyze(&blurred));
        assert!(
            a.result.sharpness_metric > b.result.sharpness_metric * 5.0,
            "{} vs {}",
            a.result.sharpness_metric,
            b.result.sharpness_metric
        );
        assert!(a.result.quality_score > b.result.quality_score);
        assert!(b.result.subject_focus_metric.is_none());
    }

    #[test]
    fn exposure_penalizes_clipping() {
        let ok = GrayImage::from_pixel(20, 20, Luma([128]));
        let black = GrayImage::from_pixel(20, 20, Luma([0]));
        assert_eq!(exposure_metric(&ok), 1.0);
        assert_eq!(exposure_metric(&black), 0.0);
        assert_eq!(exposure_metric(&GrayImage::new(0, 0)), 0.0);
    }

    #[test]
    fn blurriest_region_ignores_flat_cells() {
        // left half textured, right half flat: the region must be on the left
        let mut image = GrayImage::from_pixel(160, 160, Luma([128]));
        for y in 0..160 {
            for x in 0..80 {
                image.put_pixel(x, y, Luma([if (x / 2 + y / 2) % 2 == 0 { 30 } else { 220 }]));
            }
        }
        let region = find_blurriest_region(&image).unwrap();
        assert!(region.x < 0.5);
        assert!(find_blurriest_region(&GrayImage::from_pixel(160, 160, Luma([128]))).is_none());
        assert!(find_blurriest_region(&GrayImage::new(8, 8)).is_none());
    }

    #[test]
    fn sharpest_region_finds_the_crisp_corner_and_stays_inside_the_frame() {
        // a soft frame (blurred checkerboard) with one crisp patch in the lower right
        let mut image = imageops::blur(&GrayImage::from_fn(320, 320, |x, y| Luma([if (x / 3 + y / 3) % 2 == 0 { 40 } else { 215 }])), 3.0);
        for y in 230..310 {
            for x in 230..310 {
                image.put_pixel(x, y, Luma([if (x / 2 + y / 2) % 2 == 0 { 0 } else { 255 }]));
            }
        }
        let region = find_sharpest_region(&image).unwrap();
        assert!(region.x + region.width / 2.0 > 0.6 && region.y + region.height / 2.0 > 0.6, "{region:?}");
        assert!(region.x >= 0.0 && region.y >= 0.0 && region.x + region.width <= 1.0001 && region.y + region.height <= 1.0001);
        // the same pixels give the same rectangle
        let again = find_sharpest_region(&image).unwrap();
        assert_eq!((region.x, region.y, region.width, region.height), (again.x, again.y, again.width, again.height));
        // negative controls: a flat frame and a frame too small to grid have none
        assert!(find_sharpest_region(&GrayImage::from_pixel(160, 160, Luma([128]))).is_none());
        assert!(find_sharpest_region(&GrayImage::new(16, 16)).is_none());
    }

    #[test]
    fn an_analysis_cached_before_the_sharp_region_still_decodes() {
        let old = r#"{"qualityScore":0.5,"sharpnessMetric":1.0,"centerFocusMetric":1.0,"subjectFocusMetric":null,"subjectCompositionMetric":null,"subjectEdgeContactRatio":null,"exposureMetric":1.0,"width":10,"height":10,"blurryRegion":null}"#;
        assert!(serde_json::from_str::<CullAnalysis>(old).unwrap().sharp_region.is_none());
    }

    #[test]
    fn masked_metrics_need_a_large_enough_matching_mask() {
        let image = GrayImage::from_pixel(40, 40, Luma([100]));
        assert!(masked_laplacian_variance(&image, &GrayImage::new(40, 40)).is_none());
        assert!(masked_exposure_metric(&image, &GrayImage::from_pixel(10, 10, Luma([255]))).is_none());
        let full = GrayImage::from_pixel(40, 40, Luma([255]));
        assert!(masked_laplacian_variance(&image, &full).is_some());
        assert_eq!(masked_exposure_metric(&image, &full), Some(1.0));
    }

    #[test]
    fn duplicate_burst_groups_and_the_unrelated_frame_stays_out() {
        let sharp = sharp_scene(320, 240, 0);
        let blurred = image::imageops::blur(&sharp, 1.5);
        let other = RgbImage::from_fn(320, 240, |x, y| Rgb([(y * 255 / 240) as u8, (255 - x * 255 / 320) as u8, 60]));
        let frames = [analyze(&sharp), analyze(&blurred), analyze(&other)];
        let items: Vec<_> = frames.iter().map(|f| (f.hash.clone(), f.result.quality_score)).collect();
        let groups = group_similar(&items, 28);
        assert_eq!(groups.len(), 1);
        // the sharper frame leads its group
        assert_eq!(groups[0].members, vec![0, 1]);
    }

    #[test]
    fn hash_round_trips_through_text() {
        let a = analyze(&sharp_scene(64, 64, 1));
        let again = hash_from_string(&hash_to_string(&a.hash)).unwrap();
        assert_eq!(a.hash.dist(&again), 0);
        assert!(hash_from_string("not a hash !!").is_none());
    }

    #[test]
    fn small_images_do_not_panic() {
        let a = analyze(&RgbImage::from_pixel(2, 2, Rgb([9, 9, 9])));
        assert_eq!(a.result.sharpness_metric, 0.0);
        assert!(a.result.blurry_region.is_none());
    }
}

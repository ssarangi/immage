// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors
use crate::{immich_search::SearchArgs, typed_library::LibraryImage};
use chrono::NaiveDate;
/// Photos by the people in them. `include` matches photos with any of those people (all of them when
/// `together`), `exclude` photos with none of them, `none` photos with no faces, `unnamed` photos with a face
/// nobody is named for. Only confirmed faces count as a person being in a photo.
#[derive(Default, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct PeopleFilter {
    pub include: Vec<i64>,
    pub exclude: Vec<i64>,
    pub together: bool,
    pub none: bool,
    pub unnamed: bool,
}
/// The most people one filter may name (they travel as SQL parameters).
pub const MAX_FILTER_PEOPLE: usize = 500;

#[derive(Default, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct GridFilter {
    /// Find's People row (faces_store.rs `filter_paths`); ignored while Faces is off.
    pub people: Option<PeopleFilter>,
    /// Keep photos whose main face has closed or half closed eyes (faces_eyes.rs `closed_eye_paths`); ignored while
    /// Faces is off or the eye model is missing, and it sees only photos whose eye verdicts are already computed.
    pub closed_eyes: bool,
    pub min_rating: u8,
    pub rejected: String,
    pub edited: String,
    pub labels: u32,
    pub unlabeled: bool,
    pub kinds: Vec<String>,
    pub cameras: Vec<String>,
    pub bodies: Vec<String>,
    pub lenses: Vec<String>,
    pub keywords: Vec<String>,
    pub taken_from: String,
    pub taken_to: String,
    pub iso: Vec<String>,
    pub aperture: Vec<String>,
    pub focal: Vec<String>,
    pub shutter: Vec<String>,
}
fn day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(&value.get(..10)?.replace(':', "-"), "%Y-%m-%d").ok()
}
fn selected(values: &[String], value: &str) -> bool {
    values.is_empty() || (!value.is_empty() && values.iter().any(|v| v == value))
}
fn exact(expected: &Option<String>, value: &str) -> bool {
    expected
        .as_ref()
        .is_none_or(|s| s.trim().is_empty() || (!value.is_empty() && s == value))
}
fn date_range(value: Option<NaiveDate>, from: &str, to: &str) -> bool {
    if from.is_empty() && to.is_empty() {
        return true;
    }
    value.is_some_and(|value| {
        (from.is_empty() || day(from).is_some_and(|d| value >= d))
            && (to.is_empty() || day(to).is_some_and(|d| value <= d))
    })
}
pub fn validate(f: &GridFilter, r: &SearchArgs) -> Result<(), String> {
    if f.min_rating > 5
        || r.min_rating.is_some_and(|v| v > 5)
        || !matches!(f.rejected.as_str(), "" | "all" | "hide" | "only")
        || !matches!(f.edited.as_str(), "" | "all" | "edited" | "unedited")
        || f.labels > 31
        || f.people.as_ref().is_some_and(|p| p.include.len() > MAX_FILTER_PEOPLE || p.exclude.len() > MAX_FILTER_PEOPLE)
    {
        return Err("Invalid local search filter".into());
    }
    for date in [
        &f.taken_from,
        &f.taken_to,
        r.taken_from.as_deref().unwrap_or(""),
        r.taken_to.as_deref().unwrap_or(""),
    ] {
        if !date.is_empty() && (date.len() != 10 || day(date).is_none()) {
            return Err("Invalid capture date filter".into());
        }
    }
    Ok(())
}
fn kind(name: &str) -> &str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" | "jpe" => "JPEG",
        "tif" | "tiff" => "TIFF",
        "heic" | "heif" | "hif" => "HEIF",
        "png" => "PNG",
        "avif" => "AVIF",
        "webp" => "WEBP",
        "jxl" => "JXL",
        "exr" => "EXR",
        "pfm" => "PFM",
        "ppm" | "pgm" | "pnm" => "PNM",
        "gif" => "GIF",
        "j2k" | "jp2" => "JPEG 2000",
        "qoi" => "QOI",
        "bmp" => "BMP",
        _ => "RAW",
    }
}
// JavaScript toFixed for positive finite camera values: round the exact binary
// significand to decimal, with ties up. This avoids 2.55*10 rounding drift.
fn decimal(value: f64, places: u32) -> Option<String> {
    if !value.is_finite() || value <= 0.0 || value > 1e9 {
        return None;
    }
    let bits = value.to_bits();
    let exp = ((bits >> 52) & 2047) as i32 - 1023 - 52;
    let significand = u128::from((bits & ((1u64 << 52) - 1)) | (1u64 << 52)) * 10u128.pow(places);
    let rounded = if exp >= 0 {
        significand.checked_shl(exp as u32)?
    } else {
        let shift = (-exp) as u32;
        if shift >= 128 {
            0
        } else {
            (significand >> shift)
                + u128::from((significand & ((1u128 << shift) - 1)) >= (1u128 << (shift - 1)))
        }
    };
    let scale = 10u128.pow(places);
    let whole = rounded / scale;
    let fraction = rounded % scale;
    Some(if places == 0 || fraction == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{fraction:0width$}", width = places as usize)
            .trim_end_matches('0')
            .to_string()
    })
}
pub fn passes(i: &LibraryImage, capture: Option<&str>, f: &GridFilter, r: &SearchArgs) -> bool {
    // These server identities/fields have no verified local mapping. Unknown
    // never satisfies a selected filter; five stars are not an Immich favorite.
    if r.favorites
        || !r.person_ids.is_empty()
        || !r.tag_ids.is_empty()
        || !r.album_ids.is_empty()
        || r.country.as_ref().is_some_and(|s| !s.is_empty())
        || r.city.as_ref().is_some_and(|s| !s.is_empty())
        || !matches!(r.library.as_str(), "" | "all" | "external")
    {
        return false;
    }
    let rating = f.min_rating.max(r.min_rating.unwrap_or(0));
    if (rating > 0 && (i.rejected || i.rating < rating))
        || (f.rejected == "hide" && i.rejected)
        || (f.rejected == "only" && !i.rejected)
        || (f.edited == "edited" && !i.edited)
        || (f.edited == "unedited" && i.edited)
    {
        return false;
    }
    if (f.labels != 0 || f.unlabeled)
        && (i.labels & f.labels) == 0
        && !(f.unlabeled && i.labels == 0)
    {
        return false;
    }
    let raw = std::path::Path::new(&i.filename)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| {
            crate::immich_search::RAW_EXTENSIONS.contains(&s.to_ascii_lowercase().as_str())
        });
    if (r.raw_only && !raw) || !selected(&f.kinds, kind(&i.filename)) {
        return false;
    }
    let camera = format!("{} {}", i.exif.maker, i.exif.model)
        .trim()
        .to_string();
    let body = if i.exif.body_serial.trim().is_empty() {
        String::new()
    } else {
        serde_json::to_string(&[&i.exif.maker, &i.exif.model, i.exif.body_serial.trim()])
            .unwrap_or_default()
    };
    if !selected(&f.cameras, &camera)
        || !selected(&f.bodies, &body)
        || !selected(&f.lenses, &i.exif.lens)
        || !exact(&r.make, &i.exif.maker)
        || !exact(&r.model, &i.exif.model)
        || !exact(&r.lens, &i.exif.lens)
    {
        return false;
    }
    if !f.keywords.iter().all(|k| {
        i.keywords
            .iter()
            .any(|v| v == k || v.starts_with(&format!("{k}|")))
    }) {
        return false;
    }
    // Capture dates never fall back to filesystem modification dates here.
    let date = capture.and_then(day).or_else(|| day(&i.exif.datetime));
    if !date_range(date, &f.taken_from, &f.taken_to)
        || !date_range(
            date,
            r.taken_from.as_deref().unwrap_or(""),
            r.taken_to.as_deref().unwrap_or(""),
        )
    {
        return false;
    }
    let iso = i
        .exif
        .iso
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| v.round().to_string())
        .unwrap_or_default();
    let aperture = i
        .exif
        .aperture
        .as_f64()
        .and_then(|v| decimal(v, 1))
        .map(|s| format!("f/{s}"))
        .unwrap_or_default();
    let focal = i
        .exif
        .focal_length
        .as_f64()
        .and_then(|v| decimal(v, 0))
        .map(|s| format!("{s} mm"))
        .unwrap_or_default();
    let shutter = i
        .exif
        .exposure
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
        .and_then(|v| {
            if v >= 0.3 {
                decimal(v, 1).map(|s| format!("{s}s"))
            } else {
                Some(format!("1/{}", (1.0 / v).round()))
            }
        })
        .unwrap_or_default();
    selected(&f.iso, &iso)
        && selected(&f.aperture, &aperture)
        && selected(&f.focal, &focal)
        && selected(&f.shutter, &shutter)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_matches_camera_filter_rounding() {
        assert_eq!(decimal(2.55, 1).as_deref(), Some("2.5"));
        assert_eq!(decimal(2.25, 1).as_deref(), Some("2.3"));
        assert_eq!(decimal(35.0, 0).as_deref(), Some("35"));
        assert!(decimal(f64::NAN, 1).is_none());
    }

    #[test]
    fn the_closed_eyes_flag_defaults_off_and_parses() {
        assert!(!serde_json::from_str::<GridFilter>("{}").unwrap().closed_eyes);
        assert!(serde_json::from_str::<GridFilter>(r#"{"closedEyes":true}"#).unwrap().closed_eyes);
    }

    #[test]
    fn the_people_filter_parses_with_defaults_and_is_capped() {
        let f: GridFilter = serde_json::from_str(r#"{"people":{"include":[1,2],"together":true}}"#).unwrap();
        let p = f.people.clone().unwrap();
        assert_eq!((p.include, p.exclude, p.together, p.none, p.unnamed), (vec![1, 2], vec![], true, false, false));
        assert!(serde_json::from_str::<GridFilter>(r#"{"people":{"nobody":1}}"#).is_err(), "unknown fields are refused");
        assert!(serde_json::from_str::<GridFilter>("{}").unwrap().people.is_none());
        assert!(validate(&f, &SearchArgs::default()).is_ok());
        let big = GridFilter { people: Some(PeopleFilter { include: (0..501).collect(), ..Default::default() }), ..Default::default() };
        assert!(validate(&big, &SearchArgs::default()).is_err());
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Google Gemini, for AI grading: a photo critique (a summary and boxed
//! regions) and a natural-language search turned into Find filters.
//!
//! The API key lives in the OS keychain (service `app.immage`), never in
//! settings.json, and is sent in the `x-goog-api-key` header, never in a URL.
//! A critique uploads one downscaled JPEG of the photo to Google; a search
//! sends only the typed sentence. The model never sees catalog contents and
//! never emits SQL: its answer is a fixed schema that `sanitize_search` clamps
//! and `to_find` maps onto Find's filter model (search_filters.rs `GridFilter`).
//!
//! Network calls take the base URL, so tests run against a mock server.

use crate::search_filters::GridFilter;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::Client;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

pub(crate) const BASE_URL: &str = "https://generativelanguage.googleapis.com";
/// the settings.json key of Preferences > Culling's AI grading switch
pub(crate) const ENABLED: &str = "immage.aiGrading";
/// bump when the critique prompt or schema changes, so old answers are not reused
pub(crate) const PROMPT_VERSION: u32 = 1;
/// long edge of the JPEG sent for a critique
pub(crate) const MAX_EDGE: u32 = 1024;
/// the most a critique upload may weigh
pub(crate) const MAX_JPEG_BYTES: usize = 2 * 1024 * 1024;
const JPEG_QUALITY: u8 = 82;
const MAX_MODELS: usize = 4;
const TIMEOUT: Duration = Duration::from_secs(60);
const KEY_HEADER: &str = "x-goog-api-key";

// ---- the key ---------------------------------------------------------

pub(crate) fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new("app.immage", "gemini-api-key")
        .map_err(|_| "The system keychain is unavailable".to_string())
}

/// the stored key, or None when there is none
pub(crate) fn stored_key() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(key) if !key.trim().is_empty() => Ok(Some(key)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("The system keychain is unavailable".into()),
    }
}

/// a key as typed: trimmed, and fit for a header
pub(crate) fn clean_key(key: &str) -> Result<String, String> {
    let key = key.trim();
    if key.is_empty() || key.len() > 256 || reqwest::header::HeaderValue::from_str(key).is_err() {
        return Err("That does not look like a Gemini API key".into());
    }
    Ok(key.to_string())
}

pub(crate) fn enabled(runtime: &Path) -> bool {
    crate::settings::read_key(runtime, ENABLED).as_deref() == Some("1")
}

// ---- HTTP ------------------------------------------------------------

fn client() -> Result<Client, String> {
    Client::builder().timeout(TIMEOUT).build().map_err(|_| "Could not start the network client".to_string())
}

/// an error body, shortened, with the key removed
fn brief(body: &str, key: &str) -> String {
    let body = if key.is_empty() { body.to_string() } else { body.replace(key, "<key>") };
    body.chars().take(200).collect()
}

enum CallError {
    /// the model is overloaded or limited (429, 5xx) or unreachable: try the next
    Retryable(String),
    /// another model will not help (a bad key or request)
    Fatal(String),
}

fn status_error(status: reqwest::StatusCode, body: &str, key: &str, what: &str) -> CallError {
    let message = format!("Gemini ({what}) returned HTTP {status}: {}", brief(body, key));
    if status.as_u16() == 429 || status.is_server_error() {
        CallError::Retryable(message)
    } else {
        CallError::Fatal(message)
    }
}

/// Checks a key by listing models, the cheapest authenticated call.
pub(crate) fn test_key(base: &str, key: &str) -> Result<(), String> {
    let key = clean_key(key)?;
    let response = client()?
        .get(format!("{base}/v1beta/models"))
        .query(&[("pageSize", "1")])
        .header(KEY_HEADER, &key)
        .send()
        .map_err(|_| "Could not reach Gemini".to_string())?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        400 | 401 | 403 => Err("Gemini rejected this key. Check that it was copied correctly.".into()),
        n => Err(format!("Gemini returned an unexpected error (HTTP {n})")),
    }
}

/// Sort key of a flash-tier model name: newest version first, then the
/// cheaper flash-lite, then a stable release over a preview. None for anything
/// that is not a flash or lite text-and-vision model.
fn parse_flash_candidate(model_name: &str) -> Option<(u32, u32, bool, bool)> {
    let short = model_name.rsplit('/').next().unwrap_or(model_name);
    if !short.contains("flash") {
        return None;
    }
    if ["tts", "embedding", "image", "live", "audio"].iter().any(|w| short.contains(w)) {
        return None;
    }
    let is_lite = short.contains("flash-lite");
    let is_preview = short.contains("preview") || short.contains("exp");
    let version = short.strip_prefix("gemini-")?.split("-flash").next()?;
    let mut parts = version.splitn(2, '.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((major, minor, is_lite, !is_preview))
}

/// the usable flash/lite models of this key, best first
fn resolve_models(base: &str, key: &str) -> Result<Vec<String>, String> {
    let response = client()?
        .get(format!("{base}/v1beta/models"))
        .query(&[("pageSize", "200")])
        .header(KEY_HEADER, key)
        .send()
        .map_err(|_| "Could not reach Gemini".to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(match status.as_u16() {
            400 | 401 | 403 => "Gemini rejected the saved key".to_string(),
            _ => format!("Gemini returned HTTP {status} while listing models: {}", brief(&body, key)),
        });
    }
    let payload: Value = response.json().map_err(|_| "Could not read Gemini's model list".to_string())?;
    let models = payload["models"].as_array().ok_or("Gemini's model list was missing 'models'")?;
    let mut found: Vec<(String, (u32, u32, bool, bool))> = Vec::new();
    for model in models {
        let Some(name) = model["name"].as_str() else { continue };
        let generates = model["supportedGenerationMethods"]
            .as_array()
            .is_some_and(|m| m.iter().any(|v| v.as_str() == Some("generateContent")));
        if !generates {
            continue;
        }
        if let Some(sort) = parse_flash_candidate(name) {
            found.push((name.rsplit('/').next().unwrap_or(name).to_string(), sort));
        }
    }
    if found.is_empty() {
        return Err("No flash or lite Gemini model is available for this key".into());
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(found.into_iter().map(|(name, _)| name).take(MAX_MODELS).collect())
}

fn call_model(base: &str, key: &str, model: &str, body: &Value) -> Result<Value, CallError> {
    let client = client().map_err(CallError::Fatal)?;
    let response = client
        .post(format!("{base}/v1beta/models/{model}:generateContent"))
        .header(KEY_HEADER, key)
        .json(body)
        .send()
        .map_err(|_| CallError::Retryable(format!("Could not reach Gemini ({model})")))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().unwrap_or_default();
        return Err(status_error(status, &text, key, model));
    }
    let payload: Value = response
        .json()
        .map_err(|_| CallError::Fatal("Could not read Gemini's response".into()))?;
    let text = payload["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| CallError::Fatal("Gemini's response had no text part".into()))?;
    serde_json::from_str(text).map_err(|_| CallError::Fatal("Gemini's answer was not the expected JSON".into()))
}

/// Sends `body` to the best model, falling through to the next on 429, 5xx or
/// a network failure. Returns the model that answered and its JSON.
pub(crate) fn generate(base: &str, key: &str, body: &Value) -> Result<(String, Value), String> {
    let mut last = None;
    for model in resolve_models(base, key)? {
        match call_model(base, key, &model, body) {
            Ok(answer) => return Ok((model, answer)),
            Err(CallError::Retryable(message)) => last = Some(message),
            Err(CallError::Fatal(message)) => return Err(message),
        }
    }
    Err(last.unwrap_or_else(|| "No Gemini model answered".into()))
}

// ---- critique --------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CritiqueRegion {
    pub label: String,
    pub positive: bool,
    pub note: String,
    /// [x, y, width, height], each 0 to 1 from the top left
    #[serde(rename = "box")]
    pub bbox: [f32; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CritiqueBody {
    pub overall_summary: String,
    #[serde(default)]
    pub regions: Vec<CritiqueRegion>,
}

pub(crate) fn critique_request(image_base64: &str) -> Value {
    let prompt = "You are a photography mentor helping someone cull a shoot. Look at this photo \
        and identify up to 4 notable regions: genuine strengths (sharp eye, good light, clean \
        background) or weaknesses (obstructing branch, blown highlight, distracting element, soft \
        focus area). For each region give a short 2-4 word label, whether it is positive, a \
        normalized bounding box [x, y, width, height] in the 0-1 range with the origin at the \
        top-left, and a one-sentence note. Also give a 1-2 sentence overall critique. Be specific \
        to what is actually in this image; do not give generic photography advice.";
    json!({
        "contents": [{ "parts": [
            { "inline_data": { "mime_type": "image/jpeg", "data": image_base64 } },
            { "text": prompt }
        ]}],
        "generationConfig": {
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "OBJECT",
                "properties": {
                    "overallSummary": { "type": "STRING" },
                    "regions": { "type": "ARRAY", "items": {
                        "type": "OBJECT",
                        "properties": {
                            "label": { "type": "STRING" },
                            "positive": { "type": "BOOLEAN" },
                            "note": { "type": "STRING" },
                            "box": { "type": "ARRAY", "items": { "type": "NUMBER" } }
                        },
                        "required": ["label", "positive", "note", "box"]
                    }}
                },
                "required": ["overallSummary", "regions"]
            }
        }
    })
}

fn clip(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

/// A model's answer made safe to draw: text bounded, at most 6 regions, boxes
/// clamped inside the frame, regions without a usable box dropped.
pub(crate) fn sanitize_critique(answer: Value) -> Result<CritiqueBody, String> {
    let overall = clip(answer["overallSummary"].as_str().unwrap_or(""), 600);
    if overall.is_empty() {
        return Err("Gemini's critique had no summary".into());
    }
    let mut regions = Vec::new();
    for region in answer["regions"].as_array().into_iter().flatten().take(12) {
        // the box is read loosely, as a model may send fewer or more numbers
        let nums: Vec<f32> = region["box"]
            .as_array()
            .map(|a| a.iter().filter_map(|n| n.as_f64().map(|n| n as f32)).collect())
            .unwrap_or_default();
        if nums.len() != 4 || nums.iter().any(|n| !n.is_finite()) {
            continue;
        }
        let x = nums[0].clamp(0.0, 1.0);
        let y = nums[1].clamp(0.0, 1.0);
        let w = nums[2].clamp(0.0, 1.0 - x);
        let h = nums[3].clamp(0.0, 1.0 - y);
        if w < 0.01 || h < 0.01 {
            continue;
        }
        let label = clip(region["label"].as_str().unwrap_or(""), 60);
        if label.is_empty() {
            continue;
        }
        regions.push(CritiqueRegion {
            label,
            positive: region["positive"].as_bool().unwrap_or(false),
            note: clip(region["note"].as_str().unwrap_or(""), 300),
            bbox: [x, y, w, h],
        });
        if regions.len() == 6 {
            break;
        }
    }
    Ok(CritiqueBody { overall_summary: overall, regions })
}

/// A downscaled JPEG (long edge `MAX_EDGE`) of RGBA pixels, for upload.
pub(crate) fn prepare_jpeg(rgba: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let image = image::RgbaImage::from_raw(width, height, rgba).ok_or("The preview has an unexpected size")?;
    let mut rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
    let long = rgb.width().max(rgb.height());
    if long > MAX_EDGE {
        let scale = f64::from(MAX_EDGE) / f64::from(long);
        let w = ((f64::from(rgb.width()) * scale).round() as u32).max(1);
        let h = ((f64::from(rgb.height()) * scale).round() as u32).max(1);
        rgb = image::imageops::resize(&rgb, w, h, image::imageops::FilterType::Triangle);
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    if out.len() > MAX_JPEG_BYTES {
        return Err("The preview is too large to send".into());
    }
    Ok(out)
}

pub(crate) fn critique(base: &str, key: &str, jpeg: &[u8]) -> Result<(String, CritiqueBody), String> {
    let (model, answer) = generate(base, key, &critique_request(&STANDARD.encode(jpeg)))?;
    Ok((model, sanitize_critique(answer)?))
}

// ---- critique cache --------------------------------------------------

/// a photo as the cache knows it: path, size and modified time, so an edited
/// or replaced file is critiqued again
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Identity {
    pub path: String,
    pub size: i64,
    pub mtime: i64,
}

impl Identity {
    pub(crate) fn of(path: &str) -> Result<Identity, String> {
        let meta = std::fs::metadata(path).map_err(|_| "The photo is not available".to_string())?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        Ok(Identity { path: path.to_string(), size: meta.len() as i64, mtime })
    }
}

/// One small sqlite file, gemini-critiques.sqlite in the runtime folder, keyed
/// by file identity, model and prompt version. A lookup accepts any model
/// (a newer model alone is no reason to spend another request); a stored
/// answer replaces older ones of the same photo and prompt.
pub(crate) struct Store(Connection);

impl Store {
    pub(crate) fn open(file: &Path) -> Result<Store, String> {
        let db = Connection::open(file).map_err(|e| e.to_string())?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS critiques(
               path TEXT NOT NULL, size INTEGER NOT NULL, mtime INTEGER NOT NULL,
               model TEXT NOT NULL, prompt INTEGER NOT NULL,
               body TEXT NOT NULL, created INTEGER NOT NULL,
               PRIMARY KEY(path, size, mtime, model, prompt));",
        )
        .map_err(|e| e.to_string())?;
        Ok(Store(db))
    }

    pub(crate) fn get(&self, id: &Identity) -> Option<(String, CritiqueBody)> {
        let (model, body): (String, String) = self
            .0
            .query_row(
                "SELECT model, body FROM critiques WHERE path=?1 AND size=?2 AND mtime=?3 AND prompt=?4
                 ORDER BY created DESC LIMIT 1",
                params![id.path, id.size, id.mtime, PROMPT_VERSION],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        Some((model, serde_json::from_str(&body).ok()?))
    }

    pub(crate) fn put(&self, id: &Identity, model: &str, body: &CritiqueBody) -> Result<(), String> {
        let json = serde_json::to_string(body).map_err(|e| e.to_string())?;
        self.0
            .execute("DELETE FROM critiques WHERE path=?1 AND prompt=?2", params![id.path, PROMPT_VERSION])
            .map_err(|e| e.to_string())?;
        self.0
            .execute(
                "INSERT OR REPLACE INTO critiques(path,size,mtime,model,prompt,body,created)
                 VALUES(?1,?2,?3,?4,?5,?6,strftime('%s','now'))",
                params![id.path, id.size, id.mtime, model, PROMPT_VERSION, json],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---- catalog search --------------------------------------------------

/// What the model fills in. Only fields Find can use; scope and paging belong
/// to the app.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct SearchQuery {
    pub text: Option<String>,
    pub rating: Option<i64>,
    pub min_rating: Option<i64>,
    pub tags: Option<Vec<String>>,
    pub ai_tags: Option<Vec<String>>,
    pub tag_mode: Option<String>,
    pub year: Option<i64>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub person: Option<String>,
    pub people: Option<Vec<String>>,
    pub excluded_people: Option<Vec<String>>,
    pub excluded_tags: Option<Vec<String>>,
    pub excluded_ai_tags: Option<Vec<String>>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub color: Option<String>,
    pub is_raw: Option<bool>,
    pub is_edited: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SearchAnswer {
    summary: String,
    query: SearchQuery,
}

fn trim_opt(value: &mut Option<String>, max: usize) {
    if let Some(text) = value {
        let t = text.trim();
        if t.is_empty() {
            *value = None;
        } else {
            *text = t.chars().take(max).collect();
        }
    }
}

fn clean_list(list: &mut Option<Vec<String>>, each: usize, count: usize) {
    if let Some(items) = list {
        let cleaned: Vec<String> = items.iter().map(|i| clip(i, each)).filter(|i| !i.is_empty()).take(count).collect();
        *list = (!cleaned.is_empty()).then_some(cleaned);
    }
}

/// bounds every field the model sent; a value out of range is dropped
pub(crate) fn sanitize_search(q: &mut SearchQuery) {
    q.rating = q.rating.filter(|r| (0..=5).contains(r));
    q.min_rating = q.min_rating.filter(|r| (0..=5).contains(r));
    q.year = q.year.filter(|y| (1900..=2100).contains(y));
    trim_opt(&mut q.text, 200);
    trim_opt(&mut q.camera, 120);
    trim_opt(&mut q.lens, 120);
    trim_opt(&mut q.person, 120);
    trim_opt(&mut q.color, 32);
    // "A and B" in the singular field is a conjunction of two people
    if q.people.as_ref().is_none_or(Vec::is_empty) {
        if let Some(person) = &q.person {
            let split: Vec<String> = person
                .split(',')
                .flat_map(|p| p.split(" and "))
                .flat_map(|p| p.split(" & "))
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            if split.len() > 1 {
                q.people = Some(split);
                q.person = None;
            }
        }
    }
    for list in [&mut q.people, &mut q.excluded_people, &mut q.excluded_tags, &mut q.excluded_ai_tags] {
        clean_list(list, 120, 12);
    }
    for list in [&mut q.tags, &mut q.ai_tags] {
        clean_list(list, 80, 12);
    }
    q.date_from = q.date_from.filter(|d| (0..=4_102_444_800).contains(d));
    q.date_to = q.date_to.filter(|d| (0..=4_102_444_800).contains(d));
    if q.date_from.zip(q.date_to).is_some_and(|(a, b)| a > b) {
        std::mem::swap(&mut q.date_from, &mut q.date_to);
    }
    if !matches!(q.tag_mode.as_deref(), Some("AND" | "OR")) {
        q.tag_mode = None;
    }
}

pub(crate) fn search_request(sentence: &str, today: &str) -> Value {
    let prompt = format!(
        "Convert the user's request into a photo catalog search. Today is {today}. Return only the \
         JSON schema below. Use only filters the schema supports. Do not invent names, tags, cameras, \
         dates, ratings or file properties; omit a filter that is not stated. Put remaining \
         filename or caption words in query.text. Ratings are integers 0 through 5; 'at least N stars' \
         is minRating. RAW or not RAW and edited or unedited are booleans. For relative years, work \
         out the calendar year from today's date. Names after 'not', 'without' or 'excluding' go in \
         excludedPeople; do the same for tags with excludedTags and excludedAiTags. Convert date \
         ranges to inclusive Unix timestamps in dateFrom and dateTo. User request: {sentence}"
    );
    let strings = json!({ "type": "ARRAY", "items": { "type": "STRING" } });
    json!({
        "contents": [{ "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "temperature": 0,
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "OBJECT",
                "properties": {
                    "summary": { "type": "STRING" },
                    "query": { "type": "OBJECT", "properties": {
                        "text": { "type": "STRING" },
                        "rating": { "type": "INTEGER" },
                        "minRating": { "type": "INTEGER" },
                        "tags": strings, "aiTags": strings,
                        "tagMode": { "type": "STRING", "enum": ["AND", "OR"] },
                        "year": { "type": "INTEGER" },
                        "camera": { "type": "STRING" }, "lens": { "type": "STRING" },
                        "person": { "type": "STRING" }, "people": strings,
                        "excludedPeople": strings, "excludedTags": strings, "excludedAiTags": strings,
                        "dateFrom": { "type": "INTEGER" }, "dateTo": { "type": "INTEGER" },
                        "color": { "type": "STRING" },
                        "isRaw": { "type": "BOOLEAN" }, "isEdited": { "type": "BOOLEAN" }
                    }}
                },
                "required": ["summary", "query"]
            }
        }
    })
}

/// A search as Find can run it: the words for its search box, a folder
/// Filter, and what Find has no field for.
#[derive(Clone, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GeminiSearch {
    /// what Gemini understood, one sentence
    pub summary: String,
    /// for Find's search box (file names, and content words for Smart search)
    pub text: String,
    /// for Find's Filter: rating, edited, file kind, keywords, capture dates
    pub filter: GridFilter,
    /// a camera or lens named, to be matched against the folder's cameras and lenses
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// color label bits (red 1, yellow 2, green 4, blue 8, purple 16)
    pub labels: u32,
    /// parts of the request Find cannot apply, in words
    pub unmapped: Vec<String>,
}

fn date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).map_or_else(String::new, |d| d.format("%Y-%m-%d").to_string())
}

const NON_RAW_KINDS: [&str; 13] =
    ["JPEG", "TIFF", "HEIF", "PNG", "AVIF", "WEBP", "JXL", "EXR", "PFM", "PNM", "GIF", "JPEG 2000", "BMP"];

/// Maps a sanitized query onto Find's filter model.
pub(crate) fn to_find(summary: &str, q: &SearchQuery) -> GeminiSearch {
    let mut out = GeminiSearch { summary: clip(summary, 240), ..Default::default() };
    if out.summary.is_empty() {
        out.summary = "Interpreted your search".into();
    }
    let mut words: Vec<String> = q.text.iter().cloned().collect();
    // Smart search matches photo content by words
    words.extend(q.ai_tags.iter().flatten().cloned());
    out.text = words.join(" ");
    out.filter.min_rating = q.min_rating.or(q.rating).unwrap_or(0) as u8;
    if let Some(r) = q.rating.filter(|r| (1..5).contains(r) && q.min_rating.is_none()) {
        out.unmapped.push(format!("exactly {r} stars (applied as {r} or more)"));
    }
    if let Some(edited) = q.is_edited {
        out.filter.edited = if edited { "edited" } else { "unedited" }.into();
    }
    match q.is_raw {
        Some(true) => out.filter.kinds = vec!["RAW".into()],
        Some(false) => out.filter.kinds = NON_RAW_KINDS.iter().map(|k| k.to_string()).collect(),
        None => {}
    }
    if let Some(tags) = &q.tags {
        if q.tag_mode.as_deref() == Some("OR") && tags.len() > 1 {
            out.unmapped.push(format!("any of the keywords {}", tags.join(", ")));
        } else {
            out.filter.keywords = tags.clone();
        }
    }
    if let Some(year) = q.year {
        out.filter.taken_from = format!("{year:04}-01-01");
        out.filter.taken_to = format!("{year:04}-12-31");
    }
    if let Some(from) = q.date_from {
        out.filter.taken_from = date(from);
    }
    if let Some(to) = q.date_to {
        out.filter.taken_to = date(to);
    }
    out.camera = q.camera.clone();
    out.lens = q.lens.clone();
    if let Some(color) = &q.color {
        match color.to_ascii_lowercase().as_str() {
            "red" => out.labels = 1,
            "yellow" => out.labels = 2,
            "green" => out.labels = 4,
            "blue" => out.labels = 8,
            "purple" => out.labels = 16,
            _ => out.unmapped.push(format!("color {color}")),
        }
    }
    let people: Vec<String> = q.people.iter().flatten().chain(q.person.iter()).cloned().collect();
    if !people.is_empty() {
        out.unmapped.push(format!("people {}", people.join(", ")));
    }
    for (what, list) in [
        ("without people", &q.excluded_people),
        ("without keywords", &q.excluded_tags),
        ("without content", &q.excluded_ai_tags),
    ] {
        if let Some(items) = list {
            out.unmapped.push(format!("{what} {}", items.join(", ")));
        }
    }
    out
}

/// Interprets a sentence. Only the sentence and today's date are sent.
pub(crate) fn search(base: &str, key: &str, sentence: &str, today: &str) -> Result<GeminiSearch, String> {
    let sentence = sentence.trim();
    if sentence.chars().count() < 2 {
        return Err("Describe what you want to find first".into());
    }
    let sentence: String = sentence.chars().take(500).collect();
    let (_, answer) = generate(base, key, &search_request(&sentence, today))?;
    let mut answer: SearchAnswer =
        serde_json::from_value(answer).map_err(|_| "Gemini's search answer was not the expected JSON".to_string())?;
    sanitize_search(&mut answer.query);
    Ok(to_find(&answer.summary, &answer.query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    #[test]
    fn flash_candidates_rank_newest_lite_stable() {
        assert!(parse_flash_candidate("models/gemini-2.5-pro").is_none());
        assert!(parse_flash_candidate("models/gemini-2.5-flash-preview-tts").is_none());
        assert!(parse_flash_candidate("models/gemini-2.0-flash-live-001").is_none());
        let mut names =
            ["gemini-2.5-flash", "gemini-3.0-flash", "gemini-2.5-flash-lite", "gemini-2.5-flash-lite-preview-04-17"];
        names.sort_by(|a, b| parse_flash_candidate(b).cmp(&parse_flash_candidate(a)));
        assert_eq!(
            names,
            ["gemini-3.0-flash", "gemini-2.5-flash-lite", "gemini-2.5-flash-lite-preview-04-17", "gemini-2.5-flash"]
        );
    }

    #[test]
    fn critique_is_clamped_and_bounded() {
        let body = sanitize_critique(json!({
            "overallSummary": "  Sharp but cluttered. ",
            "regions": [
                { "label": "Eye", "positive": true, "note": "Crisp", "box": [0.2, 0.3, 0.1, 0.1] },
                { "label": "Edge", "positive": false, "note": "x", "box": [0.9, -1.0, 0.5, 2.0] },
                { "label": "Bad", "positive": false, "note": "x", "box": [0.1, 0.1] },
                { "label": "", "positive": false, "note": "x", "box": [0.1, 0.1, 0.2, 0.2] },
                { "label": "Zero", "positive": false, "note": "x", "box": [0.1, 0.1, 0.0, 0.2] }
            ]
        }))
        .unwrap();
        assert_eq!(body.overall_summary, "Sharp but cluttered.");
        assert_eq!(body.regions.len(), 2);
        let edge = &body.regions[1].bbox;
        assert!((edge[0] - 0.9).abs() < 1e-6 && edge[1] == 0.0 && (edge[2] - 0.1).abs() < 1e-6 && edge[3] == 1.0);
        assert!(sanitize_critique(json!({ "overallSummary": " ", "regions": [] })).is_err());
        let many: Vec<Value> = (0..20)
            .map(|i| json!({ "label": format!("r{i}"), "positive": true, "note": "n", "box": [0.1, 0.1, 0.2, 0.2] }))
            .collect();
        assert_eq!(sanitize_critique(json!({ "overallSummary": "s", "regions": many })).unwrap().regions.len(), 6);
    }

    #[test]
    fn search_is_sanitized() {
        let mut q = SearchQuery {
            text: Some("  ".into()),
            rating: Some(9),
            min_rating: Some(4),
            year: Some(1800),
            person: Some("Swasti and Saket".into()),
            date_from: Some(2_000_000_000),
            date_to: Some(1_000_000_000),
            tag_mode: Some("XOR".into()),
            tags: Some(vec![" a ".into(), "".into()]),
            ..Default::default()
        };
        sanitize_search(&mut q);
        assert_eq!((q.text, q.rating, q.min_rating, q.year), (None, None, Some(4), None));
        assert_eq!(q.people, Some(vec!["Swasti".into(), "Saket".into()]));
        assert_eq!(q.person, None);
        assert!(q.date_from < q.date_to);
        assert_eq!(q.tag_mode, None);
        assert_eq!(q.tags, Some(vec!["a".into()]));
    }

    #[test]
    fn search_maps_onto_find() {
        let q = SearchQuery {
            text: Some("beach".into()),
            ai_tags: Some(vec!["sunset".into()]),
            rating: Some(3),
            is_raw: Some(true),
            is_edited: Some(false),
            year: Some(2024),
            color: Some("Red".into()),
            tags: Some(vec!["trip".into()]),
            excluded_tags: Some(vec!["blurry".into()]),
            camera: Some("Sony A7".into()),
            ..Default::default()
        };
        let s = to_find("", &q);
        assert_eq!(s.text, "beach sunset");
        assert_eq!(s.filter.min_rating, 3);
        assert_eq!(s.filter.kinds, vec!["RAW"]);
        assert_eq!(s.filter.edited, "unedited");
        assert_eq!((s.filter.taken_from.as_str(), s.filter.taken_to.as_str()), ("2024-01-01", "2024-12-31"));
        assert_eq!(s.filter.keywords, vec!["trip"]);
        assert_eq!(s.labels, 1);
        assert_eq!(s.camera.as_deref(), Some("Sony A7"));
        assert_eq!(s.unmapped.len(), 2);
        assert!(s.summary.contains("Interpreted"));
        // the filter is one Find accepts
        let args = crate::immich_search::SearchArgs::default();
        assert!(crate::search_filters::validate(&s.filter, &args).is_ok());
    }

    #[test]
    fn jpeg_is_downscaled_under_the_cap() {
        let (w, h) = (3000u32, 2000u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 13) as u8 * 19, 80, 255]).collect();
        let jpeg = prepare_jpeg(rgba, w, h).unwrap();
        assert!(jpeg.len() <= MAX_JPEG_BYTES);
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1024, 683));
        assert!(prepare_jpeg(vec![0; 3], 2, 2).is_err());
    }

    #[test]
    fn cache_keys_on_identity_and_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("c.sqlite")).unwrap();
        let id = Identity { path: "/p/a.arw".into(), size: 10, mtime: 5 };
        let body = CritiqueBody { overall_summary: "ok".into(), regions: vec![] };
        assert!(store.get(&id).is_none());
        store.put(&id, "gemini-2.5-flash-lite", &body).unwrap();
        assert_eq!(store.get(&id).unwrap().0, "gemini-2.5-flash-lite");
        assert!(store.get(&Identity { mtime: 6, ..id.clone() }).is_none());
        store.put(&id, "gemini-3.0-flash", &body).unwrap();
        assert_eq!(store.get(&id).unwrap().0, "gemini-3.0-flash");
        let rows: i64 = store.0.query_row("SELECT count(*) FROM critiques", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    fn models() -> String {
        json!({ "models": [
            { "name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.0-flash-lite", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/text-embedding", "supportedGenerationMethods": ["embedContent"] }
        ]})
        .to_string()
    }

    fn reply(text: &Value) -> String {
        json!({ "candidates": [{ "content": { "parts": [{ "text": text.to_string() }] } }] }).to_string()
    }

    #[test]
    fn key_goes_in_the_header_and_models_fall_back() {
        let mut server = Server::new();
        let list = server
            .mock("GET", "/v1beta/models")
            .match_query(Matcher::Any)
            .match_header("x-goog-api-key", "AIzaSECRET")
            .with_body(models())
            .create();
        let down = server
            .mock("POST", "/v1beta/models/gemini-3.0-flash-lite:generateContent")
            .match_header("x-goog-api-key", "AIzaSECRET")
            .with_status(503)
            .with_body("overloaded AIzaSECRET")
            .create();
        let good = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_header("x-goog-api-key", "AIzaSECRET")
            .match_body(Matcher::Regex("inline_data".into()))
            .with_body(reply(&json!({ "overallSummary": "Nice", "regions": [
                { "label": "Eye", "positive": true, "note": "n", "box": [0.1, 0.1, 0.2, 0.2] }] })))
            .create();
        let (model, body) = critique(&server.url(), "AIzaSECRET", b"jpeg").unwrap();
        assert_eq!(model, "gemini-2.5-flash");
        assert_eq!(body.regions.len(), 1);
        list.assert();
        down.assert();
        good.assert();
    }

    #[test]
    fn a_bad_request_does_not_fall_back_and_hides_the_key() {
        let mut server = Server::new();
        server.mock("GET", "/v1beta/models").match_query(Matcher::Any).with_body(models()).create();
        server
            .mock("POST", Matcher::Regex("generateContent".into()))
            .with_status(400)
            .with_body("bad key AIzaSECRET")
            .expect(1)
            .create();
        let error = critique(&server.url(), "AIzaSECRET", b"jpeg").unwrap_err();
        assert!(error.contains("400") && !error.contains("AIzaSECRET"), "{error}");
    }

    #[test]
    fn every_model_down_reports_the_last_error() {
        let mut server = Server::new();
        server.mock("GET", "/v1beta/models").match_query(Matcher::Any).with_body(models()).create();
        server.mock("POST", Matcher::Regex("generateContent".into())).with_status(429).create();
        assert!(critique(&server.url(), "k", b"jpeg").unwrap_err().contains("429"));
    }

    #[test]
    fn search_runs_against_the_mock() {
        let mut server = Server::new();
        server.mock("GET", "/v1beta/models").match_query(Matcher::Any).with_body(models()).create();
        let m = server
            .mock("POST", Matcher::Regex("generateContent".into()))
            .match_body(Matcher::Regex("red photos from 2023".into()))
            .with_body(reply(
                &json!({ "summary": "Red labeled, 2023", "query": { "year": 2023, "color": "red", "rating": 12 } }),
            ))
            .create();
        let s = search(&server.url(), "k", "red photos from 2023", "2026-11-20").unwrap();
        m.assert();
        assert_eq!((s.labels, s.filter.taken_from.as_str(), s.filter.min_rating), (1, "2023-01-01", 0));
        assert!(search(&server.url(), "k", " ", "2026-11-20").is_err());
    }

    #[test]
    fn test_key_maps_statuses() {
        let mut server = Server::new();
        server
            .mock("GET", "/v1beta/models")
            .match_query(Matcher::Any)
            .match_header("x-goog-api-key", "good")
            .with_body("{}")
            .create();
        server.mock("GET", "/v1beta/models").match_query(Matcher::Any).with_status(400).create();
        assert!(test_key(&server.url(), "good").is_ok());
        assert!(test_key(&server.url(), "bad").unwrap_err().contains("rejected"));
        assert!(test_key(&server.url(), "  ").is_err());
    }
}

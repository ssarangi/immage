// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! XMP, for the catalog: the rating, colour label, keywords and "has darktable
//! edits" of a sidecar (darktable's `name.ext.xmp`, Lightroom and other
//! `name.xmp`) or of an XMP packet embedded in a file.
//!
//! A deliberately small tolerant scanner rather than an XML crate: it only has
//! to find a handful of attributes and list items, it must never fail on the
//! damaged packets real files carry, and its limits are fixed (input size,
//! nesting, item counts). It does not validate XML and does not resolve
//! namespaces: the prefixes darktable, Adobe and Apple use are matched by name.

use serde::{Deserialize, Serialize};

/// Sidecars larger than this are not read (darktable's longest histories are
/// well under it).
pub const MAX_SIDECAR: u64 = 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_KEYWORDS: usize = 1000;
const MAX_KEYWORD_LEN: usize = 256;
const MAX_HISTORY: usize = 4096;

/// Modules darktable applies by itself to every raw (its default workflow);
/// a history made only of them is not an edit.
const AUTO_APPLIED: &[&str] = &[
    "rawprepare", "temperature", "highlights", "demosaic", "colorin", "colorout", "gamma", "flip", "dither", "finalscale",
    "hotpixels", "exposure", "filmicrgb", "sigmoid", "channelmixerrgb", "basecurve",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Xmp {
    /// 0-5; -1 is a reject in darktable's and Adobe's files.
    pub rating: Option<i32>,
    /// Bit n for colour n: red 1, yellow 2, green 4, blue 8, purple 16.
    pub labels: Option<u32>,
    pub keywords: Vec<String>,
    /// darktable's `history_end`; Some only in a darktable sidecar.
    pub history_end: Option<i64>,
    /// Whether the history holds an operation beyond the automatic ones.
    pub edited: Option<bool>,
}

fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Attributes of a start tag's text (everything after the name).
fn attributes(s: &str) -> Vec<(String, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() && out.len() < 64 {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let name = &s[start..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || (b[i] != b'"' && b[i] != b'\'') {
            continue;
        }
        let q = b[i];
        i += 1;
        let vs = i;
        while i < b.len() && b[i] != q {
            i += 1;
        }
        if !name.is_empty() {
            out.push((name.to_string(), decode(&s[vs..i.min(b.len())])));
        }
        i += 1;
    }
    out
}

fn label_bit(name: &str) -> Option<u32> {
    match name.trim().to_ascii_lowercase().as_str() {
        "red" => Some(1),
        "yellow" => Some(2),
        "green" => Some(4),
        "blue" => Some(8),
        "purple" | "magenta" => Some(16),
        _ => None,
    }
}

struct Frame {
    name: String,
    text: String,
}

/// Reads the XMP in `data` (a packet or a whole sidecar). None when it holds
/// nothing of interest, so an unrelated XML file adds no data.
pub fn parse(data: &[u8]) -> Option<Xmp> {
    if data.len() as u64 > MAX_SIDECAR {
        return None;
    }
    let text = String::from_utf8_lossy(data);
    let s: &str = &text;
    let mut x = Xmp::default();
    let mut hier: Vec<String> = Vec::new();
    let mut history: Vec<(Option<i64>, String)> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut found = false;
    let mut pos = 0;
    while let Some(lt) = s[pos..].find('<') {
        let at = pos + lt;
        // text before the tag belongs to the open element
        if let Some(f) = stack.last_mut() {
            if f.text.len() < 512 {
                f.text.push_str(&s[pos..at]);
            }
        }
        let rest = &s[at..];
        if rest.starts_with("<!--") {
            pos = rest.find("-->").map_or(s.len(), |e| at + e + 3);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").map_or(s.len(), |e| at + e + 3);
            if let Some(f) = stack.last_mut() {
                f.text.push_str(&s[(at + 9).min(end)..end.saturating_sub(3).max(at + 9).min(s.len())]);
            }
            pos = end;
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            pos = rest.find('>').map_or(s.len(), |e| at + e + 1);
            continue;
        }
        let Some(gt) = tag_end(rest) else { break };
        let inner = &rest[1..gt];
        pos = at + gt + 1;
        if let Some(name) = inner.strip_prefix('/') {
            // an end tag: close back to the matching frame
            let name = name.trim();
            if let Some(i) = stack.iter().rposition(|f| f.name == name) {
                while stack.len() > i {
                    let f = stack.pop().unwrap();
                    let value = decode(f.text.trim());
                    let in_list = |list: &str| stack.iter().any(|a| local(&a.name) == list);
                    match local(&f.name) {
                        "Rating" => {
                            if let Ok(v) = value.parse::<f64>() {
                                x.rating = Some(v as i32);
                                found = true;
                            }
                        }
                        "Label" => {
                            if let Some(b) = label_bit(&value) {
                                *x.labels.get_or_insert(0) |= b;
                                found = true;
                            }
                        }
                        "history_end" => {
                            if let Ok(v) = value.parse() {
                                x.history_end = Some(v);
                                found = true;
                            }
                        }
                        "li" if !value.is_empty() => {
                            if in_list("subject") && !in_list("hierarchicalSubject") {
                                push_keyword(&mut x.keywords, &value);
                                found = true;
                            } else if in_list("hierarchicalSubject") {
                                push_keyword(&mut hier, &value);
                            } else if in_list("colorlabels") {
                                if let Ok(n) = value.parse::<u32>() {
                                    if n < 5 {
                                        *x.labels.get_or_insert(0) |= 1 << n;
                                        found = true;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }
        let self_closing = inner.ends_with('/');
        let body = inner.trim_end_matches('/');
        let (name, attrs) = match body.find(|c: char| c.is_ascii_whitespace()) {
            Some(i) => (&body[..i], &body[i..]),
            None => (body, ""),
        };
        let attrs = attributes(attrs);
        let is_li = local(name) == "li";
        let in_history = stack.iter().any(|a| local(&a.name) == "history");
        for (k, v) in &attrs {
            match local(k) {
                "Rating" if is_xmp_ns(k) => {
                    if let Ok(r) = v.parse::<f64>() {
                        x.rating = Some(r as i32);
                        found = true;
                    }
                }
                "Label" if is_xmp_ns(k) => {
                    if let Some(b) = label_bit(v) {
                        *x.labels.get_or_insert(0) |= b;
                        found = true;
                    }
                }
                "history_end" => {
                    if let Ok(n) = v.trim().parse() {
                        x.history_end = Some(n);
                        found = true;
                    }
                }
                _ => {}
            }
        }
        if is_li && in_history && history.len() < MAX_HISTORY {
            let get = |n: &str| attrs.iter().find(|(k, _)| local(k) == n).map(|(_, v)| v.as_str());
            if let Some(op) = get("operation") {
                // disabled steps do not count either
                if get("enabled") != Some("0") {
                    history.push((get("num").and_then(|n| n.trim().parse().ok()), op.to_string()));
                }
            }
        }
        if !self_closing && stack.len() < MAX_DEPTH {
            stack.push(Frame { name: name.to_string(), text: String::new() });
        }
    }
    if x.keywords.is_empty() && !hier.is_empty() {
        x.keywords = hier;
        found = true;
    }
    if let Some(end) = x.history_end {
        x.edited = Some(end > 0 && history.iter().any(|(n, op)| n.is_none_or(|n| n < end) && !AUTO_APPLIED.contains(&op.as_str())));
    }
    found.then_some(x)
}

fn is_xmp_ns(attr: &str) -> bool {
    matches!(attr.rsplit_once(':').map(|(p, _)| p), Some("xmp" | "xap"))
}

fn push_keyword(v: &mut Vec<String>, s: &str) {
    let s: String = s.trim().chars().take(MAX_KEYWORD_LEN).collect();
    if !s.is_empty() && v.len() < MAX_KEYWORDS && !v.contains(&s) {
        v.push(s);
    }
}

/// The end of a tag (the index of its `>`), skipping `>` inside quotes.
fn tag_end(s: &str) -> Option<usize> {
    let mut q: Option<u8> = None;
    for (i, b) in s.bytes().enumerate().skip(1) {
        match (q, b) {
            (Some(c), b) if b == c => q = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => q = Some(b),
            (None, b'>') => return Some(i),
            _ => {}
        }
    }
    None
}

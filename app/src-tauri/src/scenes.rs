// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Scenes: a folder's photos as a timeline of bursts. A scene is a run of frames
//! close in time that look alike; a time block is a run of scenes with no long gap
//! between them. This file is the pure part: parsing capture times, hashing a
//! cached thumbnail, and the time-sorted sweep that cuts scenes and blocks. It
//! reads no file but the thumbnail bytes it is handed, so it is tested with
//! synthetic frames. The commands, the store and the progress events are
//! scenes_cmds.rs and scenes_store.rs.
//!
//! The sweep is O(n * window): frames are sorted by capture time and each is
//! compared with the last few frames of the scene being built, never with every
//! other frame (`cull::group_similar` is O(n^2) and not usable on 20,000 photos).
use crate::scenes_store::Store;
use image_hasher::ImageHash;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// The thumbnails the grid caches are this wide (library.ts THUMB_SIZE); a scene
/// hash is made from one, never from the photo.
pub const THUMB_SIZE: u32 = 240;
/// How many of the scene's latest frames a new frame is compared with, so a slow
/// pan or a walking subject stays one scene.
const REFERENCE_FRAMES: usize = 3;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneParams {
    /// a gap above this many seconds starts a new scene
    pub gap_sec: u32,
    /// a gap above this many seconds starts a new time block
    pub block_gap_sec: u32,
    /// a frame this many bits (or more) away from the scene's recent frames starts a new scene
    pub similarity: u32,
}

impl Default for SceneParams {
    fn default() -> Self {
        Self { gap_sec: 60, block_gap_sec: 20 * 60, similarity: 28 }
    }
}

/// One photo as the page lists it.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneItem {
    pub path: String,
    #[serde(default)]
    pub version: i64,
    pub name: String,
    /// "YYYY:MM:DD HH:MM:SS", with a fraction when the camera recorded one; empty when there is no time
    #[serde(default)]
    pub taken_at: String,
    /// the time is the file's modified time, not the camera's
    #[serde(default)]
    pub from_file: bool,
}

/// A split or merge the user made, replayed after every grouping.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Override {
    /// start a new scene at this frame
    Split(usize),
    /// join the scene holding this frame to the next scene of its block
    Merge(usize),
}

/// The wall-clock time of a "YYYY:MM:DD HH:MM:SS[.fff]" text as milliseconds since
/// 1970 as if it were UTC (there is no zone, so only differences and the date
/// matter). None for an empty or unreadable text.
pub fn parse_taken(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() < 19 {
        return None;
    }
    chrono::NaiveDateTime::parse_from_str(text, "%Y:%m:%d %H:%M:%S%.f").ok().map(|t| t.and_utc().timestamp_millis())
}

/// What the sweep needs of one frame.
#[derive(Clone)]
pub struct Frame {
    pub t_ms: Option<i64>,
    pub name: String,
    pub hash: Option<ImageHash>,
    /// the folder the photo is in: a scene never holds photos of two folders, so a recursive
    /// timeline never merges two folders' bursts that happen to overlap in time
    pub folder: u32,
}

/// A run of scenes. `scenes` hold indexes into the input, in capture order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub no_time: bool,
    pub scenes: Vec<Vec<usize>>,
}

fn distance(a: &ImageHash, b: &ImageHash) -> u32 {
    if a.as_bytes().len() == b.as_bytes().len() { a.dist(b) } else { u32::MAX }
}

/// whether `frame` looks like any of the scene's last frames; frames without a
/// hash (no cached thumbnail yet) look like whatever they are next to in time
fn alike(scene: &[usize], frames: &[Frame], hash: &Option<ImageHash>, limit: u32) -> bool {
    let Some(hash) = hash else { return true };
    let mut compared = false;
    for &other in scene.iter().rev().take(REFERENCE_FRAMES) {
        if let Some(h) = &frames[other].hash {
            compared = true;
            if distance(hash, h) < limit {
                return true;
            }
        }
    }
    !compared
}

/// Cuts the frames into time blocks and scenes (the rules of the spec, section 2),
/// then replays the user's overrides. Frames with a capture time are sorted by it
/// (ties by file name, then input order); the rest go to a last block that is cut
/// by look alone and, for frames with no hash either, left single. Blocks follow
/// the time of all the photos together; scenes are cut folder by folder inside
/// them, so photos of two folders interleave by time but never share a scene.
pub fn group(frames: &[Frame], params: &SceneParams, overrides: &[Override]) -> Vec<Block> {
    let mut timed: Vec<usize> = (0..frames.len()).filter(|&i| frames[i].t_ms.is_some()).collect();
    timed.sort_by(|&a, &b| (frames[a].t_ms, &frames[a].name, a).cmp(&(frames[b].t_ms, &frames[b].name, b)));
    let (gap, block_gap) = (i64::from(params.gap_sec) * 1000, i64::from(params.block_gap_sec) * 1000);
    // the block of each timed frame, by the gaps between all the photos in time order
    let mut block_of = vec![0usize; frames.len()];
    let mut blocks: Vec<Block> = Vec::new();
    let mut previous: Option<i64> = None;
    for &i in &timed {
        let t = frames[i].t_ms.unwrap_or(0);
        // a new day always starts a new block, so the timeline reads by day
        if previous.is_none_or(|p| t - p > block_gap || p.div_euclid(86_400_000) != t.div_euclid(86_400_000)) {
            blocks.push(Block { no_time: false, scenes: Vec::new() });
        }
        block_of[i] = blocks.len() - 1;
        previous = Some(t);
    }
    // scenes, folder by folder, each frame compared only with its own folder's last frames
    let mut open: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();   // folder -> its open scene
    let mut scenes: Vec<(usize, Vec<usize>)> = Vec::new();   // (block, frames)
    for &i in &timed {
        let t = frames[i].t_ms.unwrap_or(0);
        let cut = open.get(&frames[i].folder).is_none_or(|&s| {
            let scene = &scenes[s].1;
            let last = *scene.last().expect("a scene has a frame");
            scenes[s].0 != block_of[i] || t - frames[last].t_ms.unwrap_or(t) > gap || !alike(scene, frames, &frames[i].hash, params.similarity)
        });
        if cut {
            open.insert(frames[i].folder, scenes.len());
            scenes.push((block_of[i], vec![i]));
        } else {
            let s = open[&frames[i].folder];
            scenes[s].1.push(i);
        }
    }
    // scenes land in their block in the order they began
    scenes.sort_by_key(|(block, frames_)| (*block, frames[frames_[0]].t_ms, frames[frames_[0]].name.clone(), frames_[0]));
    for (block, members) in scenes {
        blocks[block].scenes.push(members);
    }
    let mut untimed: Vec<usize> = (0..frames.len()).filter(|&i| frames[i].t_ms.is_none()).collect();
    untimed.sort_by(|&a, &b| (frames[a].folder, &frames[a].name, a).cmp(&(frames[b].folder, &frames[b].name, b)));
    if !untimed.is_empty() {
        let mut scenes: Vec<Vec<usize>> = Vec::new();
        for i in untimed {
            // with no time, a frame joins the scene before it only when both have a hash and look alike
            let joins = scenes.last().is_some_and(|s| {
                frames[s[0]].folder == frames[i].folder && frames[i].hash.is_some() && s.iter().any(|&o| frames[o].hash.is_some()) && alike(s, frames, &frames[i].hash, params.similarity)
            });
            if joins {
                scenes.last_mut().expect("checked").push(i);
            } else {
                scenes.push(vec![i]);
            }
        }
        blocks.push(Block { no_time: true, scenes });
    }
    apply_overrides(&mut blocks, overrides);
    blocks
}

/// Replays splits and merges on a grouping. A split at a scene's first frame and a
/// merge of a block's last scene change nothing.
pub fn apply_overrides(blocks: &mut [Block], overrides: &[Override]) {
    for action in overrides {
        match *action {
            Override::Split(frame) => {
                for block in blocks.iter_mut() {
                    if let Some(s) = block.scenes.iter().position(|s| s.contains(&frame)) {
                        let at = block.scenes[s].iter().position(|&f| f == frame).unwrap_or(0);
                        if at > 0 {
                            let tail = block.scenes[s].split_off(at);
                            block.scenes.insert(s + 1, tail);
                        }
                        break;
                    }
                }
            }
            Override::Merge(frame) => {
                for block in blocks.iter_mut() {
                    if let Some(s) = block.scenes.iter().position(|s| s.contains(&frame)) {
                        if s + 1 < block.scenes.len() {
                            let next = block.scenes.remove(s + 1);
                            block.scenes[s].extend(next);
                        }
                        break;
                    }
                }
            }
        }
    }
}

/// The scene hash of a thumbnail's JPEG bytes: the same 16x16 DoubleGradient as the
/// cull hash (cull.rs `hasher`), so distances are of the same size.
pub fn hash_thumbnail(jpeg: &[u8]) -> Option<ImageHash> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(jpeg));
    reader.set_format(image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    Some(crate::cull::hasher().hash_image(&decoded))
}

/// a cheap checksum of the thumbnail's bytes (FNV-1a), the cache's guard that the
/// thumbnail is still the one that was hashed
pub fn checksum(bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h as i64
}

pub struct Hashed {
    pub hashes: Vec<Option<ImageHash>>,
    /// frames hashed from a stored hash, not decoded
    pub reused: usize,
}

/// The scene hash of every item, from its cached thumbnail only (`read` returns the
/// JPEG, or None while there is none). A hash is kept in the store against the
/// thumbnail's checksum, so a second run decodes nothing. Runs on `workers` threads,
/// each with a store connection of its own; `step` hears each item as it finishes.
/// Returns None when `cancel` is set.
pub fn hash_all(
    items: &[SceneItem], read: &(dyn Fn(&SceneItem) -> Option<Vec<u8>> + Sync), store_path: &Path, workers: usize,
    cancel: &AtomicBool, step: &(dyn Fn(usize) + Sync),
) -> Option<Hashed> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let reused = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<ImageHash>>> = Mutex::new(vec![None; items.len()]);
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, 16) {
            scope.spawn(|| {
                let store = Store::open(store_path).ok();
                let mut fresh: Vec<(usize, i64, i64, String)> = Vec::new();
                let flush = |fresh: &mut Vec<(usize, i64, i64, String)>| {
                    if let Some(store) = &store {
                        let rows: Vec<_> = fresh.iter().map(|(i, len, sum, hash)| (items[*i].path.as_str(), items[*i].version, *len, *sum, hash.as_str())).collect();
                        let _ = store.hash_put_many(&rows);
                    }
                    fresh.clear();
                };
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= items.len() || cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let item = &items[i];
                    if let Some(bytes) = read(item) {
                        let (len, sum) = (bytes.len() as i64, checksum(&bytes));
                        let stored = store.as_ref().and_then(|s| s.hash_get(&item.path, item.version, len, sum));
                        let hash = match stored.and_then(|text| ImageHash::from_base64(&text).ok()) {
                            Some(hash) => {
                                reused.fetch_add(1, Ordering::SeqCst);
                                Some(hash)
                            }
                            None => hash_thumbnail(&bytes).inspect(|hash| fresh.push((i, len, sum, hash.to_base64()))),
                        };
                        out.lock().unwrap_or_else(|e| e.into_inner())[i] = hash;
                        if fresh.len() >= 256 {
                            flush(&mut fresh);
                        }
                    }
                    step(done.fetch_add(1, Ordering::SeqCst) + 1);
                }
                flush(&mut fresh);
            });
        }
    });
    if cancel.load(Ordering::SeqCst) {
        return None;
    }
    Some(Hashed { hashes: out.into_inner().unwrap_or_else(|e| e.into_inner()), reused: reused.load(Ordering::SeqCst) })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    pub fn at(hms: &str) -> i64 {
        parse_taken(&format!("2026:03:05 {hms}")).unwrap()
    }

    /// a distinct-looking 240px picture per seed, as JPEG
    pub fn jpeg(seed: u32) -> Vec<u8> {
        let image = RgbImage::from_fn(240, 160, |x, y| {
            let cell = ((x / (8 + seed % 5 * 6) + y / (8 + seed % 7 * 4) + seed) % 2) as u8;
            let v = if cell == 0 { 30 + (seed % 9) as u8 * 8 } else { 220 - (seed % 11) as u8 * 6 };
            Rgb([v, v.wrapping_add((seed * 37) as u8), 255 - v])
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80).encode_image(&image).unwrap();
        out
    }

    fn frame(hms: &str, seed: Option<u32>) -> Frame {
        Frame { t_ms: Some(at(hms)), name: format!("{hms}.jpg"), hash: seed.and_then(|s| hash_thumbnail(&jpeg(s))), folder: 0 }
    }

    fn untimed(name: &str, seed: Option<u32>) -> Frame {
        Frame { t_ms: None, name: name.into(), hash: seed.and_then(|s| hash_thumbnail(&jpeg(s))), folder: 0 }
    }

    #[test]
    fn times_parse_with_and_without_a_fraction() {
        assert_eq!(parse_taken("2026:03:05 07:10:04.500").unwrap() - parse_taken("2026:03:05 07:10:04").unwrap(), 500);
        assert!(parse_taken("").is_none());
        assert!(parse_taken("not a time at all!").is_none());
        assert!(parse_taken("2026:13:45 25:61:61").is_none());
    }

    #[test]
    fn a_time_gap_cuts_a_scene_and_a_long_gap_cuts_a_block() {
        // same picture throughout, so only time cuts
        let frames = [frame("07:10:00", Some(1)), frame("07:10:05", Some(1)), frame("07:12:00", Some(1)), frame("08:00:00", Some(1))];
        let blocks = group(&frames, &SceneParams::default(), &[]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].scenes, vec![vec![0, 1], vec![2]]);
        assert_eq!(blocks[1].scenes, vec![vec![3]]);
    }

    #[test]
    fn a_different_look_cuts_a_scene_inside_the_gap() {
        let frames = [frame("07:10:00", Some(1)), frame("07:10:02", Some(1)), frame("07:10:04", Some(2)), frame("07:10:06", Some(2))];
        let blocks = group(&frames, &SceneParams::default(), &[]);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].scenes, vec![vec![0, 1], vec![2, 3]]);
    }

    #[test]
    fn frames_without_a_thumbnail_follow_time_alone() {
        let frames = [frame("07:10:00", Some(1)), frame("07:10:02", None), frame("07:10:04", Some(2))];
        let blocks = group(&frames, &SceneParams::default(), &[]);
        // the unhashed frame sits with its neighbour; the third is compared with the hashed ones
        assert_eq!(blocks[0].scenes, vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn input_order_does_not_matter_and_ties_break_by_name() {
        let a = Frame { t_ms: Some(at("07:10:00")), name: "b.jpg".into(), hash: None, folder: 0 };
        let b = Frame { t_ms: Some(at("07:10:00")), name: "a.jpg".into(), hash: None, folder: 0 };
        let c = Frame { t_ms: Some(at("07:08:00")), name: "z.jpg".into(), hash: None, folder: 0 };
        let blocks = group(&[a, b, c], &SceneParams::default(), &[]);
        assert_eq!(blocks[0].scenes, vec![vec![2], vec![1, 0]]);
    }

    #[test]
    fn two_folders_interleaved_in_time_never_share_a_scene() {
        let mut frames = vec![frame("07:10:00", Some(1)), frame("07:10:01", Some(1)), frame("07:10:02", Some(1)), frame("07:10:03", Some(1))];
        frames[1].folder = 1;
        frames[3].folder = 1;
        let blocks = group(&frames, &SceneParams::default(), &[]);
        // one block (they are close in time), two scenes: folder 0's frames 0 and 2, folder 1's 1 and 3
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].scenes, vec![vec![0, 2], vec![1, 3]]);
    }

    #[test]
    fn midnight_starts_a_new_block() {
        let late = Frame { t_ms: parse_taken("2026:03:05 23:59:50"), name: "a".into(), hash: None, folder: 0 };
        let early = Frame { t_ms: parse_taken("2026:03:06 00:00:05"), name: "b".into(), hash: None, folder: 0 };
        let blocks = group(&[late, early], &SceneParams::default(), &[]);
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn photos_with_no_time_go_last_and_are_cut_by_look_only() {
        let frames = [frame("07:10:00", Some(1)), untimed("b.jpg", Some(3)), untimed("a.jpg", Some(3)), untimed("c.jpg", Some(4)), untimed("d.jpg", None)];
        let blocks = group(&frames, &SceneParams::default(), &[]);
        assert_eq!(blocks.len(), 2);
        assert!(!blocks[0].no_time && blocks[1].no_time);
        // a.jpg and b.jpg look alike; c.jpg differs; d.jpg has no hash so is single
        assert_eq!(blocks[1].scenes, vec![vec![2, 1], vec![3], vec![4]]);
    }

    #[test]
    fn overrides_split_and_merge_and_survive_a_regroup() {
        let frames = [frame("07:10:00", Some(1)), frame("07:10:02", Some(1)), frame("07:10:04", Some(1)), frame("07:12:00", Some(1))];
        let mut blocks = group(&frames, &SceneParams::default(), &[Override::Split(2)]);
        assert_eq!(blocks[0].scenes, vec![vec![0, 1], vec![2], vec![3]]);
        blocks = group(&frames, &SceneParams::default(), &[Override::Split(2), Override::Merge(2)]);
        assert_eq!(blocks[0].scenes, vec![vec![0, 1], vec![2, 3]]);
        // a split at a scene's first frame and a merge of the last scene change nothing
        blocks = group(&frames, &SceneParams::default(), &[Override::Split(0), Override::Merge(3)]);
        assert_eq!(blocks[0].scenes, vec![vec![0, 1, 2], vec![3]]);
    }

    #[test]
    fn a_slow_pan_stays_one_scene_by_chaining() {
        // each frame differs a little from the last, far from the first
        let hashes: Vec<_> = (0..4).map(|k| hash_thumbnail(&jpeg(10 + k))).collect();
        let near = |a: &Option<ImageHash>, b: &Option<ImageHash>| a.as_ref().unwrap().dist(b.as_ref().unwrap());
        // the fixture is only meaningful if the pictures differ at all
        assert!(near(&hashes[0], &hashes[1]) > 0);
        let params = SceneParams { similarity: u32::MAX, ..SceneParams::default() };
        let frames: Vec<_> = (0..4).map(|k| Frame { t_ms: Some(at("07:10:00") + k * 2000), name: k.to_string(), hash: hashes[k as usize].clone(), folder: 0 }).collect();
        assert_eq!(group(&frames, &params, &[])[0].scenes.len(), 1);
    }

    #[test]
    fn twenty_thousand_time_only_frames_group_fast() {
        // 20,000 frames, a burst of 8 every 100 s, a new block every 400 frames
        let frames: Vec<Frame> = (0..20_000i64)
            .map(|i| Frame { t_ms: Some(at("00:00:00") + (i / 8) * 100_000 + (i / 400) * 3_000_000 + (i % 8) * 400), name: format!("{i}"), hash: None, folder: 0 })
            .collect();
        let started = std::time::Instant::now();
        let blocks = group(&frames, &SceneParams::default(), &[]);
        let took = started.elapsed();
        let total: usize = blocks.iter().flat_map(|b| &b.scenes).map(Vec::len).sum();
        assert_eq!(total, 20_000);
        assert!(took.as_millis() < 1500, "grouping took {took:?}");
    }
}

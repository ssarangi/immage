// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Reply and argument types of the Catalog commands (catalog_cmds.rs). The page
//! gets them as generated bindings; see notes/catalog-ux-spec.md section 10.

use crate::catalog_meta_job::{MetaCoverage, MetaReport};
use crate::catalog_scan::ScanReport;
use crate::catalog_thumb_job::ThumbReport;
use crate::catalog_thumbs::ThumbCoverage;
use serde::{Deserialize, Serialize};
use specta::Type;

/// What `source_detect` learned of a folder before it is added.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SourceDetect {
    /// "Network share, SMB (host, share)", "Drive (exfat)" or "Local disk".
    pub storage: String,
    pub kind: String,
    pub protocol: Option<String>,
    pub host: Option<String>,
    pub share: Option<String>,
    pub fs_type: String,
    pub mount_path: String,
    /// The folder answered and is a directory.
    pub readable: bool,
    /// Behind an idle automount: adding it is allowed offline.
    pub asleep: bool,
    /// An existing source this one would overlap.
    pub overlaps: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SpeedTest {
    pub listing_per_s: f64,
    /// Reading headers is measured by the metadata phase; always null for now.
    pub reading_per_s: Option<f64>,
    /// "fast", "good", "slow", "very_slow" or "failed".
    pub verdict: String,
    pub sampled: u64,
    pub elapsed_s: f64,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct SourceAdd {
    pub path: String,
    pub name: Option<String>,
    /// "launch", "manual" or "every=6h".
    pub policy: String,
    pub include_subfolders: bool,
    pub index_now: bool,
    pub allow_offline: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct SourcePatch {
    pub name: Option<String>,
    pub policy: Option<String>,
    pub enabled: Option<bool>,
    /// "auto", "yes" or "no".
    pub treat_as_network: Option<String>,
    pub dir_skip: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ScanSummary {
    pub source_id: i64,
    pub job_id: i64,
    /// complete | incomplete | offline | cancelled | interrupted | failed
    pub state: String,
    pub resumed: bool,
    pub elapsed_s: f64,
    pub found: u64,
    pub folders: u64,
    pub new: u64,
    pub changed: u64,
    pub unchanged: u64,
    pub missing: u64,
    pub unreadable: u64,
    pub incomplete_dirs: Vec<String>,
    pub files_per_s_list: f64,
    pub error: Option<String>,
}

impl From<&ScanReport> for ScanSummary {
    fn from(r: &ScanReport) -> Self {
        Self {
            source_id: r.source_id,
            job_id: r.job_id,
            state: r.state.to_string(),
            resumed: r.resumed,
            elapsed_s: r.elapsed.as_secs_f64(),
            found: r.totals.found,
            folders: r.totals.dirs,
            new: r.totals.new,
            changed: r.totals.changed,
            unchanged: r.totals.unchanged,
            missing: r.totals.missing,
            unreadable: r.totals.unreadable,
            incomplete_dirs: r.totals.incomplete_dirs.iter().map(|d| d.0.clone()).collect(),
            files_per_s_list: r.files_per_s,
            error: r.error.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CatalogStatus {
    pub path: String,
    pub schema: i64,
    pub bytes: u64,
    pub photos: i64,
    pub missing: i64,
    pub sources: i64,
    pub online: i64,
    pub offline: i64,
    pub jobs_running: i64,
    /// Photos by metadata state: read, waiting, failed (retried by the next run) and
    /// unparsed (a format the Rust extractor cannot read).
    pub metadata: MetaCoverage,
    /// Photos by thumbnail state (catalog_thumbs.rs).
    pub thumbnails: ThumbCoverage,
}

/// What a finished metadata run reports (the `catalog://metadata-finished` event).
#[derive(Debug, Clone, Serialize, Type)]
pub struct MetaSummary {
    pub source_id: i64,
    pub job_id: i64,
    /// complete | cancelled | interrupted | offline | failed
    pub state: String,
    pub resumed: bool,
    pub elapsed_s: f64,
    pub total: u64,
    pub done: u64,
    pub sidecar_only: u64,
    pub failed: u64,
    pub unparsed: u64,
    pub skipped: u64,
    pub reads: u64,
    pub escalations: u64,
    pub bytes_read: u64,
    pub per_s: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    /// Renamed or moved photos matched to their old rows.
    pub relinked: u64,
    pub ambiguous: u64,
    pub error: Option<String>,
}

impl From<&MetaReport> for MetaSummary {
    fn from(r: &MetaReport) -> Self {
        Self {
            source_id: r.source_id,
            job_id: r.job_id,
            state: r.state.to_string(),
            resumed: r.resumed,
            elapsed_s: r.elapsed.as_secs_f64(),
            total: r.total,
            done: r.done,
            sidecar_only: r.sidecar_only,
            failed: r.failed,
            unparsed: r.unparsed,
            skipped: r.skipped,
            reads: r.reads,
            escalations: r.escalations,
            bytes_read: r.bytes_read,
            per_s: r.per_s,
            p50_ms: r.p50_ms,
            p95_ms: r.p95_ms,
            relinked: r.relink.relinked,
            ambiguous: r.relink.ambiguous,
            error: r.error.clone(),
        }
    }
}

/// What a finished thumbnail run reports (the `catalog://thumbnails-finished` event).
#[derive(Debug, Clone, Serialize, Type)]
pub struct ThumbSummary {
    pub source_id: i64,
    pub job_id: i64,
    /// complete | cancelled | interrupted | offline | disk_full | failed
    pub state: String,
    pub resumed: bool,
    pub elapsed_s: f64,
    pub total: u64,
    pub done: u64,
    pub failed: u64,
    /// Left for the app's engine, or for a policy that allows whole JPEGs.
    pub deferred: u64,
    pub skipped: u64,
    pub from_preview: u64,
    pub from_exif: u64,
    pub from_jpeg: u64,
    pub from_engine: u64,
    /// Thumbnails linked into the library thumbnail cache.
    pub linked: u64,
    pub reads: u64,
    pub bytes_read: u64,
    pub thumbnail_bytes: u64,
    pub per_s: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub error: Option<String>,
}

impl From<&ThumbReport> for ThumbSummary {
    fn from(r: &ThumbReport) -> Self {
        Self {
            source_id: r.source_id,
            job_id: r.job_id,
            state: r.state.to_string(),
            resumed: r.resumed,
            elapsed_s: r.elapsed.as_secs_f64(),
            total: r.total,
            done: r.done,
            failed: r.failed,
            deferred: r.deferred,
            skipped: r.skipped,
            from_preview: r.from_preview,
            from_exif: r.from_exif,
            from_jpeg: r.from_jpeg,
            from_engine: r.from_engine,
            linked: r.linked,
            reads: r.reads,
            bytes_read: r.bytes_read,
            thumbnail_bytes: r.thumb_bytes,
            per_s: r.per_s,
            p50_ms: r.p50_ms,
            p95_ms: r.p95_ms,
            error: r.error.clone(),
        }
    }
}

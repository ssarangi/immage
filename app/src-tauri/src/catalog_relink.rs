// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Rename and move detection. A file renamed or moved outside Immage looks, to a
//! scan, like a photo that went missing plus a new photo. They are the same
//! photo when their `content_key` (size, modified time, a hash of the first
//! 64 KB, capture time and body serial) is equal; relinking keeps the old row's
//! id, so tags, analyses and thumbnails keyed by it survive, and gives it the
//! new path and the freshly read metadata.
//!
//! The key exists only once the metadata phase has read the new file, so this
//! runs at the end of a metadata run (see `catalog_meta_job`), not inside the
//! scan. Rules, all within one source:
//!
//! * a missing row and a present row share a key, and each is the only one with
//!   that key in its state: exactly one missing and exactly one present. Two
//!   equal keys on either side is ambiguity (identical copies), and nothing is
//!   done;
//! * a present twin that was not created after the missing row (a copy that
//!   already existed) is not a move: the original stays missing;
//! * a copy (the original still present) therefore is never relinked, because
//!   the original is present, not missing.

use crate::catalog_db::CatalogDb;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct RelinkReport {
    pub relinked: u64,
    /// Keys with more than one candidate on a side: left alone.
    pub ambiguous: u64,
}

/// Relinks what can be relinked in `source_id`. Runs on the writer, in order
/// with the metadata batches before it.
pub fn relink_source(db: &CatalogDb, source_id: i64) -> Result<RelinkReport, String> {
    db.exec(move |c| relink(c, source_id))
}

pub fn relink(c: &mut Connection, source_id: i64) -> Result<RelinkReport, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let tx = c.transaction().map_err(e)?;
    // keys that are missing on one side and present on the other
    let pairs: Vec<(String, i64, i64, i64)> = {
        let mut st = tx
            .prepare(
                "SELECT m.content_key,
                        (SELECT COUNT(*) FROM photos x WHERE x.source_id=?1 AND x.status='missing' AND x.content_key=m.content_key),
                        (SELECT COUNT(*) FROM photos x WHERE x.source_id=?1 AND x.status='present' AND x.content_key=m.content_key),
                        MIN(m.id)
                 FROM photos m WHERE m.source_id=?1 AND m.status='missing' AND m.content_key IS NOT NULL
                 GROUP BY m.content_key",
            )
            .map_err(e)?;
        let rows = st.query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(e)?;
        rows.collect::<Result<_, _>>().map_err(e)?
    };
    let mut report = RelinkReport::default();
    for (key, missing, present, missing_id) in pairs {
        if present == 0 {
            continue;
        }
        if missing != 1 || present != 1 {
            report.ambiguous += 1;
            continue;
        }
        let new_id: i64 = tx
            .query_row("SELECT id FROM photos WHERE source_id=?1 AND status='present' AND content_key=?2", params![source_id, key], |r| r.get(0))
            .map_err(e)?;
        // a move creates the new row after the old one
        if new_id < missing_id {
            continue;
        }
        let (rel, now): (String, i64) = tx
            .query_row("SELECT rel_path, strftime('%s','now') FROM photos WHERE id=?1", [new_id], |r| Ok((r.get(0)?, r.get::<_, String>(1)?.parse().unwrap_or(0))))
            .map_err(e)?;
        // free the unique path, move the new row's content onto the old row, drop the new row
        tx.execute("UPDATE photos SET rel_path='\u{1}relink-'||id WHERE id=?1", [new_id]).map_err(e)?;
        tx.execute(
            "UPDATE photos SET
               dir_id=n.dir_id, name=n.name, rel_path=?3, ext=n.ext, size=n.size, mtime_ns=n.mtime_ns,
               status='present', seen_at=?4, changed_run=n.first_run,
               xmp_kind=n.xmp_kind, xmp_size=n.xmp_size, xmp_mtime_ns=n.xmp_mtime_ns,
               xmp_seen_size=n.xmp_seen_size, xmp_seen_mtime_ns=n.xmp_seen_mtime_ns,
               meta_state=n.meta_state, meta_error=n.meta_error, meta_at=n.meta_at,
               taken_at=n.taken_at, subsec_ms=n.subsec_ms, tz_offset=n.tz_offset, make=n.make, model=n.model,
               body_serial=n.body_serial, lens=n.lens, focal=n.focal, focal35=n.focal35, aperture=n.aperture,
               iso=n.iso, exposure=n.exposure, exposure_bias=n.exposure_bias, width=n.width, height=n.height,
               raw_width=n.raw_width, raw_height=n.raw_height, orientation=n.orientation,
               preview_offset=n.preview_offset, preview_len=n.preview_len, preview_w=n.preview_w, preview_h=n.preview_h,
               gps_lat=n.gps_lat, gps_lon=n.gps_lon, gps_alt=n.gps_alt, software=n.software, container=n.container,
               is_raw=n.is_raw, rating=n.rating, label=n.label, rejected=n.rejected, edited=n.edited,
               content_key=n.content_key, extra_json=n.extra_json, head_bytes=n.head_bytes
             FROM (SELECT * FROM photos WHERE id=?2) AS n WHERE photos.id=?1",
            params![missing_id, new_id, rel, now],
        )
        .map_err(e)?;
        // keywords read from the files come from the new row; the ones a person or an analysis added stay
        tx.execute("DELETE FROM keywords WHERE photo_id=?1 AND src IN ('xmp','sidecar')", [missing_id]).map_err(e)?;
        tx.execute("UPDATE OR IGNORE keywords SET photo_id=?1 WHERE photo_id=?2", params![missing_id, new_id]).map_err(e)?;
        tx.execute("DELETE FROM photos WHERE id=?1", [new_id]).map_err(e)?;
        report.relinked += 1;
    }
    tx.commit().map_err(e)?;
    Ok(report)
}

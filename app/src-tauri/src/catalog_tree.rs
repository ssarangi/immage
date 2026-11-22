// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Catalog's folder tree: a source's directories and how many present photos each holds,
//! answered from `catalog.sqlite` alone. It never opens the source's path, so it works with a NAS
//! asleep, unmounted or gone (notes/catalog-folder-tree-spec.md section 8).
//!
//! A [`SourceTree`] is built once per source by two queries and kept in memory, with the subtree
//! rolled up, until the database says the source's directories or photos changed (the shape counter
//! in `CatalogDb`, bumped by the writer after each scan batch). An expand is then a lookup.

use crate::catalog_db::CatalogDb;
use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// One directory of a source, as the page draws it.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct TreeNode {
    /// The directory this one is under (its rel path; "" is the source root).
    pub parent: String,
    /// Under the source root, '/' separated. "" is the source root itself.
    pub rel: String,
    pub name: String,
    pub has_children: bool,
    /// Present photos directly in it.
    pub direct: u32,
    /// Present photos in it and everything below.
    pub subtree: u32,
    /// "ok", "stalled", "denied", "error" or "gone" (gone only while photos remain below it).
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct TreeReply {
    /// What the page passes next time: unchanged while nothing about this source moved.
    pub generation: u32,
    /// The page's generation is still current: `nodes` and `root` are empty and its rows stand.
    pub unchanged: bool,
    /// The source root's own counts and whether it has folders under it. None when unchanged.
    pub root: Option<TreeNode>,
    /// The children of each asked folder, each in name order.
    pub nodes: Vec<TreeNode>,
}

struct Node {
    rel: String,
    /// Byte offset of the last path segment in `rel`.
    name_at: u32,
    status: u8,
    direct: u32,
    subtree: u32,
    kids: Vec<u32>,
}

const STATUS: [&str; 5] = ["ok", "stalled", "denied", "error", "gone"];

fn status_code(s: &str) -> u8 {
    STATUS.iter().position(|x| *x == s).unwrap_or(0) as u8
}

/// Nodes in `rel` byte order (what `ORDER BY rel_path` gives), so a rel is found by binary search
/// and no second copy of every path is kept.
pub struct SourceTree {
    shape: u64,
    nodes: Vec<Node>,
}

impl SourceTree {
    fn find(&self, rel: &str) -> Option<usize> {
        self.nodes.binary_search_by(|n| n.rel.as_str().cmp(rel)).ok()
    }

    fn node(&self, i: usize, parent: &str) -> TreeNode {
        let n = &self.nodes[i];
        TreeNode {
            parent: parent.to_string(),
            rel: n.rel.clone(),
            name: n.rel[n.name_at as usize..].to_string(),
            has_children: !n.kids.is_empty(),
            direct: n.direct,
            subtree: n.subtree,
            status: STATUS[n.status as usize].to_string(),
        }
    }

    fn children(&self, rel: &str) -> Vec<TreeNode> {
        match self.find(rel) {
            Some(i) => self.nodes[i].kids.iter().map(|k| self.node(*k as usize, rel)).collect(),
            None => Vec::new(),
        }
    }

    /// Approximate bytes held, for the memory test.
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.nodes.iter().map(|n| std::mem::size_of::<Node>() + n.rel.capacity() + n.kids.capacity() * 4).sum()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}

fn parent_rel(rel: &str) -> &str {
    rel.rfind('/').map(|i| &rel[..i]).unwrap_or("")
}

/// Builds a source's tree from two queries. Reads no file.
pub fn build(conn: &Connection, source_id: i64, shape: u64) -> Result<SourceTree, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let mut nodes: Vec<Node> = Vec::new();
    let mut by_id: HashMap<i64, u32> = HashMap::new();
    {
        let mut st = conn.prepare("SELECT id, rel_path, status FROM dirs WHERE source_id=?1 ORDER BY rel_path").map_err(e)?;
        let mut rows = st.query([source_id]).map_err(e)?;
        while let Some(r) = rows.next().map_err(e)? {
            let rel: String = r.get(1).map_err(e)?;
            let status: String = r.get(2).map_err(e)?;
            by_id.insert(r.get(0).map_err(e)?, nodes.len() as u32);
            let name_at = rel.rfind('/').map(|i| i + 1).unwrap_or(0) as u32;
            nodes.push(Node { rel, name_at, status: status_code(&status), direct: 0, subtree: 0, kids: Vec::new() });
        }
    }
    {
        let mut st = conn
            .prepare("SELECT dir_id, COUNT(*) FROM photos WHERE source_id=?1 AND status='present' GROUP BY dir_id")
            .map_err(e)?;
        let mut rows = st.query([source_id]).map_err(e)?;
        while let Some(r) = rows.next().map_err(e)? {
            if let Some(i) = by_id.get(&r.get::<_, i64>(0).map_err(e)?) {
                let n: i64 = r.get(1).map_err(e)?;
                nodes[*i as usize].direct = n.clamp(0, u32::MAX as i64) as u32;
            }
        }
    }
    drop(by_id);
    let mut tree = SourceTree { shape, nodes };
    // parent of each node by rel (a missing parent, from a scan still in flight, is the root)
    let parents: Vec<Option<usize>> = (0..tree.nodes.len())
        .map(|i| {
            let rel = &tree.nodes[i].rel;
            if rel.is_empty() { None } else { Some(tree.find(parent_rel(rel)).unwrap_or(0)) }
        })
        .collect();
    // a descendant sorts after its ancestor, so walking backwards finishes a node before its parent
    for i in 0..tree.nodes.len() {
        tree.nodes[i].subtree = tree.nodes[i].direct;
    }
    for i in (0..tree.nodes.len()).rev() {
        if let Some(p) = parents[i] {
            if p != i {
                tree.nodes[p].subtree = tree.nodes[p].subtree.saturating_add(tree.nodes[i].subtree);
            }
        }
    }
    for i in 0..tree.nodes.len() {
        let n = &tree.nodes[i];
        // a directory gone from the disk stays only while photos remain under it
        let shown = n.status != 4 || n.subtree > 0;
        if let (Some(p), true) = (parents[i], shown) {
            if p != i {
                tree.nodes[p].kids.push(i as u32);
            }
        }
    }
    for i in 0..tree.nodes.len() {
        let mut kids = std::mem::take(&mut tree.nodes[i].kids);
        kids.sort_by(|a, b| {
            let (x, y) = (&tree.nodes[*a as usize], &tree.nodes[*b as usize]);
            let (xn, yn) = (&x.rel[x.name_at as usize..], &y.rel[y.name_at as usize..]);
            xn.to_lowercase().cmp(&yn.to_lowercase()).then_with(|| xn.cmp(yn))
        });
        kids.shrink_to_fit();
        tree.nodes[i].kids = kids;
    }
    Ok(tree)
}

/// The trees kept, by source.
#[derive(Default)]
pub struct TreeCache {
    by_source: Mutex<HashMap<i64, Arc<SourceTree>>>,
}

impl TreeCache {
    fn tree(&self, db: &CatalogDb, source_id: i64) -> Result<Arc<SourceTree>, String> {
        // read the counter before the rows: a write landing in between only makes the next ask rebuild
        let shape = db.shape(source_id);
        if let Some(t) = self.by_source.lock().unwrap().get(&source_id) {
            if t.shape == shape {
                return Ok(t.clone());
            }
        }
        let conn = db.read().map_err(|e| e.to_string())?;
        let tree = Arc::new(build(&conn, source_id, shape)?);
        self.by_source.lock().unwrap().insert(source_id, tree.clone());
        Ok(tree)
    }

    /// The children of each of `parents` ("" is the source root), unless `generation` is still current.
    pub fn children(&self, db: &CatalogDb, source_id: i64, parents: &[String], generation: Option<u32>) -> Result<TreeReply, String> {
        let shape = db.shape(source_id);
        let current = shape as u32;
        if generation == Some(current) && self.by_source.lock().unwrap().get(&source_id).is_some_and(|t| t.shape == shape) {
            return Ok(TreeReply { generation: current, unchanged: true, root: None, nodes: Vec::new() });
        }
        let tree = self.tree(db, source_id)?;
        let mut nodes = Vec::new();
        for p in parents {
            nodes.extend(tree.children(p));
        }
        let root = tree.find("").map(|i| tree.node(i, ""));
        Ok(TreeReply { generation: tree.shape as u32, unchanged: false, root, nodes })
    }
}

pub fn cache() -> &'static TreeCache {
    static CACHE: OnceLock<TreeCache> = OnceLock::new();
    CACHE.get_or_init(TreeCache::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_db::CatalogDb;
    use rusqlite::params;

    struct Fixture {
        _dir: std::path::PathBuf,
        db: CatalogDb,
    }

    fn fixture(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("immage-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = CatalogDb::open(&dir.join("catalog.sqlite")).unwrap();
        Fixture { _dir: dir, db }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self._dir);
        }
    }

    /// Adds a source whose mount path does not exist, so any disk read would fail.
    fn source(f: &Fixture, dirs: &[(&str, &str, u32, u32)]) -> i64 {
        let dirs: Vec<(String, String, u32, u32)> = dirs.iter().map(|(a, b, c, d)| (a.to_string(), b.to_string(), *c, *d)).collect();
        f.db.exec(move |c| {
            c.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('S','nas','vol:x','/nonexistent/nas','',1)", [])
                .map_err(|e| e.to_string())?;
            let sid = c.last_insert_rowid();
            for (rel, status, present, missing) in &dirs {
                c.execute("INSERT INTO dirs(source_id,rel_path,status) VALUES(?1,?2,?3)", params![sid, rel, status]).map_err(|e| e.to_string())?;
                let did = c.last_insert_rowid();
                for (n, st) in std::iter::repeat_n("present", *present as usize).chain(std::iter::repeat_n("missing", *missing as usize)).enumerate() {
                    let name = format!("p{n}.jpg");
                    let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                    c.execute(
                        "INSERT INTO photos(source_id,dir_id,name,rel_path,ext,size,mtime_ns,status,discovered_at,seen_at) VALUES(?1,?2,?3,?4,'jpg',1,1,?5,1,1)",
                        params![sid, did, name, path, st],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok(sid)
        })
        .unwrap()
    }

    fn names(v: &[TreeNode]) -> Vec<String> {
        v.iter().map(|n| n.name.clone()).collect()
    }

    #[test]
    fn rolls_subtrees_up_and_orders_by_name() {
        let f = fixture("roll");
        let sid = source(&f, &[("", "ok", 1, 0), ("b", "ok", 2, 0), ("B2", "ok", 0, 0), ("a", "ok", 0, 0), ("a/x", "ok", 3, 0), ("a/x/y", "ok", 4, 0), ("a/z", "ok", 0, 0)]);
        let cache = TreeCache::default();
        let r = cache.children(&f.db, sid, &["".into(), "a".into()], None).unwrap();
        assert!(!r.unchanged);
        let root = r.root.unwrap();
        assert_eq!((root.direct, root.subtree, root.has_children), (1, 10, true));
        let top: Vec<_> = r.nodes.iter().filter(|n| n.parent.is_empty()).cloned().collect();
        assert_eq!(names(&top), ["a", "b", "B2"]);
        assert_eq!(top[0].subtree, 7);
        assert_eq!(top[0].direct, 0);
        assert!(top[0].has_children && !top[1].has_children);
        let under: Vec<_> = r.nodes.iter().filter(|n| n.parent == "a").cloned().collect();
        assert_eq!(names(&under), ["x", "z"]);
        assert_eq!(under[0].rel, "a/x");
        assert_eq!(under[0].subtree, 7);
    }

    #[test]
    fn missing_photos_are_not_counted_and_gone_folders_show_only_while_photos_remain() {
        let f = fixture("gone");
        let sid = source(&f, &[("", "ok", 0, 0), ("old", "gone", 0, 5), ("kept", "gone", 2, 0), ("kept/in", "ok", 1, 0), ("deny", "denied", 0, 0)]);
        let cache = TreeCache::default();
        let r = cache.children(&f.db, sid, &["".into()], None).unwrap();
        assert_eq!(names(&r.nodes), ["deny", "kept"]);
        assert_eq!(r.nodes[0].status, "denied");
        assert_eq!(r.nodes[1].subtree, 3);
        assert_eq!(r.root.unwrap().subtree, 3);
    }

    #[test]
    fn an_offline_source_answers_without_its_path() {
        let f = fixture("offline");
        let sid = source(&f, &[("", "ok", 0, 0), ("2019", "ok", 2, 0)]);
        assert!(!std::path::Path::new("/nonexistent/nas").exists());
        let r = TreeCache::default().children(&f.db, sid, &["".into()], None).unwrap();
        assert_eq!(r.nodes.len(), 1);
        assert_eq!(r.nodes[0].subtree, 2);
    }

    #[test]
    fn unchanged_until_a_scan_batch_lands() {
        let f = fixture("gen");
        let sid = source(&f, &[("", "ok", 0, 0), ("a", "ok", 1, 0)]);
        let cache = TreeCache::default();
        let first = cache.children(&f.db, sid, &["".into()], None).unwrap();
        let again = cache.children(&f.db, sid, &["".into()], Some(first.generation)).unwrap();
        assert!(again.unchanged && again.nodes.is_empty());
        f.db.touch_shape(sid);
        let moved = cache.children(&f.db, sid, &["".into()], Some(first.generation)).unwrap();
        assert!(!moved.unchanged);
        assert_ne!(moved.generation, first.generation);
        // another source's writes do not invalidate this one
        let before = f.db.shape(sid);
        f.db.touch_shape(sid + 100);
        assert_eq!(f.db.shape(sid), before);
    }

    #[test]
    fn a_scan_batch_through_the_writer_changes_the_shape() {
        use crate::catalog_db::{DirBatch, DirStatus};
        let f = fixture("writer");
        let sid = source(&f, &[("", "ok", 0, 0)]);
        let before = f.db.shape(sid);
        f.db
            .send_dir(DirBatch {
                source_id: sid,
                run_id: 1,
                rel_path: "new".into(),
                parent_rel: Some("".into()),
                mtime_ns: None,
                entry_count: 0,
                status: DirStatus::Ok,
                skipped: false,
                redo: false,
                counters: Default::default(),
                error: None,
                files: Vec::new(),
            })
            .unwrap();
        f.db.sync().unwrap();
        assert!(f.db.shape(sid) > before);
        let r = TreeCache::default().children(&f.db, sid, &["".into()], None).unwrap();
        assert_eq!(names(&r.nodes), ["new"]);
    }

    #[test]
    fn a_hundred_thousand_folders_build_fast_and_small() {
        let f = fixture("big");
        // 100 x 100 x 10 = 100k leaf folders, 3 photos each
        f.db.exec(|c| {
            let tx = c.transaction().map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('S','nas','vol:x','/nonexistent/nas','',1)", [])
                .map_err(|e| e.to_string())?;
            let sid = tx.last_insert_rowid();
            let mut d = tx.prepare("INSERT INTO dirs(source_id,rel_path,status) VALUES(?1,?2,'ok')").map_err(|e| e.to_string())?;
            let mut p = tx
                .prepare("INSERT INTO photos(source_id,dir_id,name,rel_path,ext,size,mtime_ns,status,discovered_at,seen_at) VALUES(?1,?2,'p.jpg',?3,'jpg',1,1,'present',1,1)")
                .map_err(|e| e.to_string())?;
            d.execute(params![sid, ""]).map_err(|e| e.to_string())?;
            for a in 0..100 {
                d.execute(params![sid, format!("year{a:03}")]).map_err(|e| e.to_string())?;
                for b in 0..100 {
                    d.execute(params![sid, format!("year{a:03}/event{b:03}")]).map_err(|e| e.to_string())?;
                    for c2 in 0..10 {
                        let rel = format!("year{a:03}/event{b:03}/shoot{c2}");
                        d.execute(params![sid, rel]).map_err(|e| e.to_string())?;
                        let did = tx.last_insert_rowid();
                        for k in 0..3 {
                            p.execute(params![sid, did, format!("{rel}/p{k}.jpg")]).map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
            drop((d, p));
            tx.commit().map_err(|e| e.to_string())
        })
        .unwrap();
        let sid: i64 = f.db.read().unwrap().query_row("SELECT id FROM sources", [], |r| r.get(0)).unwrap();
        let cache = TreeCache::default();
        let t = std::time::Instant::now();
        let r = cache.children(&f.db, sid, &["".into()], None).unwrap();
        let build = t.elapsed();
        assert_eq!(r.nodes.len(), 100);
        assert_eq!(r.root.as_ref().unwrap().subtree, 300_000);
        let tree = cache.tree(&f.db, sid).unwrap();
        assert!(tree.len() > 110_000);
        let t = std::time::Instant::now();
        for i in 0..1000 {
            let rel = format!("year{:03}/event{:03}", i % 100, (i * 7) % 100);
            assert_eq!(cache.children(&f.db, sid, &[rel], None).unwrap().nodes.len(), 10);
        }
        let expand = t.elapsed() / 1000;
        eprintln!("100k folders: build {build:?}, expand {expand:?}, memory about {} MB", tree.bytes() / 1_000_000);
        assert!(build.as_millis() < 2000, "first build took {build:?}");
        assert!(expand.as_micros() < 4000, "an expand took {expand:?}");
        assert!(tree.bytes() < 25_000_000, "{} bytes", tree.bytes());
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! `immage faces ...`: People from a terminal, headless like `immage catalog`. It uses the app's own
//! `faces.sqlite` (SQLite WAL with a busy timeout, so it is safe beside a running app) and the same detection and
//! recognition pipeline as the background service. It never writes into photo folders, never uploads anything, and
//! prints counts, names and paths, never embeddings or face pictures.
//!
//! Exit codes: 0 success, 1 failure, 2 usage, 3 store problem (locked, newer schema, another scan running),
//! 4 the Faces model or the engine is not available, 5 finished but some photos could not be read.

use crate::faces_cluster::MergeSuggestion;
use crate::faces_models::{self as fm, Models, Recognizer, Strictness};
use crate::faces_store::{FaceFilter, FaceStore};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAIL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_STORE: i32 = 3;
pub const EXIT_UNAVAILABLE: i32 = 4;
pub const EXIT_PARTIAL: i32 = 5;

const HELP: &str = "immage faces: People from a terminal

  immage faces status [--json]
  immage faces scan <folder> [--no-subfolders] [--thorough] [--recognizer sface|arcface] [--allow-network] [--wait]
  immage faces detect <image> [--thorough] [--json]
  immage faces groups [--json]
  immage faces people [--all] [--json]
  immage faces person <name|id> [--images] [--json]
  immage faces name <group-id> <name>        name a group (a new person, or an existing one of that name)
  immage faces rename <person> <name>
  immage faces merge <from> <into>
  immage faces ignore <person> [--restore]
  immage faces confirm <person> (<face-id>... | --suggested)
  immage faces reject [--person <person>] <face-id>...    not a face, or not that person
  immage faces suggestions [--json]
  immage faces export [--json]               people with counts, no embeddings and no pictures
  immage faces strictness <loose|balanced|strict>
  immage faces delete-all --yes

Options: --runtime <dir> (the app's data folder), --engine <path>, --ort <libonnxruntime>, --json, --help.
Network paths are refused unless --allow-network. Exit codes: 0 ok, 1 failure, 2 usage, 3 store problem,
4 model or engine unavailable, 5 done with unreadable photos.
";

/// Called first thing in `main()`: runs the CLI when the first argument is `faces`.
pub fn maybe_run() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("faces") {
        return None;
    }
    Some(run(&crate::catalog_cli::runtime_dir(), args.collect(), &mut std::io::stdout(), &mut std::io::stderr()))
}

#[derive(Default, Debug)]
struct Args {
    flags: Vec<String>,
    values: Vec<(String, String)>,
    positional: Vec<String>,
}
impl Args {
    fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
    fn value(&self, f: &str) -> Option<&str> {
        self.values.iter().find(|(k, _)| k == f).map(|(_, v)| v.as_str())
    }
}
const WITH_VALUE: &[&str] = &["--recognizer", "--engine", "--ort", "--runtime", "--person"];
const BOOLEAN: &[&str] = &["--json", "--yes", "--no-subfolders", "--thorough", "--wait", "--allow-network", "--images", "--all", "--help", "--suggested", "--restore"];

fn parse(raw: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        if arg.starts_with("--") {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) => (f, Some(v.to_string())),
                None => (arg.as_str(), None),
            };
            if WITH_VALUE.contains(&flag) {
                let v = match inline {
                    Some(v) => v,
                    None => it.next().cloned().ok_or_else(|| format!("{flag} needs a value"))?,
                };
                a.values.push((flag.to_string(), v));
            } else if BOOLEAN.contains(&flag) && inline.is_none() {
                a.flags.push(flag.to_string());
            } else {
                return Err(format!("unknown option {arg}"));
            }
        } else {
            a.positional.push(arg.clone());
        }
    }
    Ok(a)
}

struct Out<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    json: bool,
}
impl Out<'_> {
    fn result(&mut self, value: Value, human: impl FnOnce() -> String) {
        let _ = if self.json { writeln!(self.out, "{}", serde_json::to_string_pretty(&value).unwrap_or_default()) } else { writeln!(self.out, "{}", human()) };
    }
    fn note(&mut self, line: &str) {
        let _ = writeln!(self.err, "{line}");
    }
}

pub fn run(runtime: &Path, raw: Vec<String>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let args = match parse(&raw) {
        Ok(a) => a,
        Err(e) => {
            let _ = writeln!(err, "{e}\n\n{HELP}");
            return EXIT_USAGE;
        }
    };
    let runtime = args.value("--runtime").map_or_else(|| runtime.to_path_buf(), PathBuf::from);
    let mut o = Out { out, err, json: args.flag("--json") };
    let Some(cmd) = args.positional.first().cloned() else {
        let _ = write!(o.out, "{HELP}");
        return if args.flag("--help") { EXIT_OK } else { EXIT_USAGE };
    };
    if args.flag("--help") {
        let _ = write!(o.out, "{HELP}");
        return EXIT_OK;
    }
    let rest: Vec<String> = args.positional[1..].to_vec();
    let usage = |o: &mut Out, line: &str| {
        o.note(&format!("usage: immage faces {line}"));
        EXIT_USAGE
    };
    // commands that need no store
    if cmd == "detect" {
        return match rest.as_slice() {
            [image] => detect(&runtime, &args, &mut o, image),
            _ => usage(&mut o, "detect <image> [--thorough] [--json]"),
        };
    }
    let store = match FaceStore::open(&runtime.join("faces.sqlite")) {
        Ok(s) => s,
        Err(e) => {
            o.note(&format!("cannot open faces.sqlite in {}: {e}", runtime.display()));
            return EXIT_STORE;
        }
    };
    let models = Models::new(runtime.join("faces-models"), runtime.join("faces-eyes"));
    let result = match (cmd.as_str(), rest.as_slice()) {
        ("status", []) => status(&runtime, &store, &models, &mut o),
        ("scan", [folder]) => return scan(&runtime, &args, &mut o, &store, &models, folder),
        ("groups", []) => groups(&store, &mut o),
        ("people", []) => people(&store, &mut o, args.flag("--all")),
        ("person", [who]) => person(&store, &mut o, who, args.flag("--images")),
        ("name", [group, name]) => name_group(&runtime, &store, &models, &mut o, group, name),
        ("rename", [who, name]) => rename(&runtime, &store, &models, &mut o, who, name),
        ("merge", [from, into]) => merge(&runtime, &store, &models, &mut o, from, into),
        ("ignore", [who]) => ignore(&runtime, &store, &models, &mut o, who, args.flag("--restore")),
        ("confirm", [who, ids @ ..]) => confirm(&runtime, &store, &models, &mut o, who, ids, args.flag("--suggested")),
        ("reject", ids) if !ids.is_empty() => reject(&runtime, &store, &models, &mut o, args.value("--person"), ids),
        ("suggestions", []) => suggestions(&runtime, &store, &models, &mut o),
        ("export", []) => export(&store, &mut o),
        ("strictness", [level]) => strictness(&runtime, &store, &models, &mut o, level),
        ("delete-all", []) => delete_all(&runtime, &store, &mut o, args.flag("--yes")),
        ("status" | "groups" | "people" | "export" | "suggestions" | "delete-all", _) => return usage(&mut o, &format!("{cmd} takes no arguments")),
        ("person" | "name" | "rename" | "merge" | "ignore" | "confirm" | "reject" | "strictness" | "scan", _) => {
            return usage(&mut o, &format!("{cmd} ... (see --help)"))
        }
        _ => {
            o.note(&format!("unknown command {cmd}\n\n{HELP}"));
            return EXIT_USAGE;
        }
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(Failure::Usage(m)) => {
            o.note(&m);
            EXIT_USAGE
        }
        Err(Failure::Other(m)) => {
            o.note(&m);
            EXIT_FAIL
        }
    }
}

enum Failure {
    Usage(String),
    Other(String),
}
impl From<String> for Failure {
    fn from(m: String) -> Self {
        Failure::Other(m)
    }
}
type R = Result<(), Failure>;

fn recognizer_of(runtime: &Path, models: &Models) -> Recognizer {
    models.recognizer(runtime)
}

/// A person by id or by name (any case).
fn resolve(store: &FaceStore, who: &str) -> Result<i64, Failure> {
    let people = store.list_people(true)?;
    if let Ok(id) = who.parse::<i64>() {
        if people.iter().any(|p| p.id == id) {
            return Ok(id);
        }
    }
    let by_name: Vec<_> = people.iter().filter(|p| p.name.eq_ignore_ascii_case(who)).collect();
    match by_name.as_slice() {
        [p] => Ok(p.id),
        [] => Err(Failure::Usage(format!("no person {who}"))),
        _ => Err(Failure::Usage(format!("{who} names more than one person: use the id"))),
    }
}

/// Names and groups follow a change at once, as in the app.
fn regroup(runtime: &Path, store: &FaceStore, models: &Models) -> Result<(), String> {
    let r = recognizer_of(runtime, models);
    let model = fm::version_for(runtime, r);
    let decisions = crate::faces_service::Decisions { joined: store.face_links()?, apart: store.groups_not_same()? };
    let (suggestions, groups) = crate::faces_service::analysis(
        &store.exemplars(&model)?,
        &store.unnamed(&model)?,
        &fm::thresholds_for(r, fm::wanted_strictness(runtime)),
        &crate::faces_cluster::Params::default(),
        &decisions,
    );
    store.set_analysis(&suggestions, &groups)
}

fn setting_enabled(runtime: &Path) -> bool {
    crate::settings::read_key(runtime, "ai.faces.enabled").as_deref() == Some("1")
}

fn status(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out) -> R {
    let s = store.stats()?;
    let r = recognizer_of(runtime, models);
    let scope = if crate::faces_service::scope_is_folder(runtime) { "folder" } else { "catalog" };
    let v = json!({
        "enabled": setting_enabled(runtime),
        "pack_installed": models.installed(),
        "recognizer": if r == Recognizer::Arcface { "arcface" } else { "sface" },
        "arcface_available": models.arcface().is_some(),
        "strictness": fm::wanted_strictness(runtime).name(),
        "detect_size": fm::wanted_detect_size(runtime),
        "scope": scope,
        "model_version": fm::version_for(runtime, r),
        "files": s.files, "photos": s.photos, "faces": s.faces, "people": s.people, "ignored_people": s.ignored_people,
        "groups": s.groups, "unassigned": s.unassigned, "rejected": s.rejected, "failed": s.failed, "db_bytes": s.db_bytes,
        "models": s.models.iter().map(|m| json!({"model": m.model, "faces": m.faces})).collect::<Vec<_>>(),
    });
    o.result(v.clone(), || {
        format!(
            "Faces {}, model {} ({}), strictness {}, detection {} px, scope {}\n{} photos ({} files), {} faces, {} people, {} groups, {} unassigned, {} not a face, {} unreadable, database {} bytes",
            if v["enabled"].as_bool() == Some(true) { "on" } else { "off" },
            if v["pack_installed"].as_bool() == Some(true) { "installed" } else { "not installed" },
            v["recognizer"].as_str().unwrap_or(""),
            v["strictness"].as_str().unwrap_or(""),
            v["detect_size"], scope, s.photos, s.files, s.faces, s.people, s.groups, s.unassigned, s.rejected, s.failed, s.db_bytes
        )
    });
    Ok(())
}

fn groups(store: &FaceStore, o: &mut Out) -> R {
    let g = store.list_clusters()?;
    let v = json!(g.iter().map(|c| json!({"id": c.id, "faces": c.size, "photos": c.photos, "representative_face": c.rep_face_id, "members": c.members})).collect::<Vec<_>>());
    o.result(v, || {
        if g.is_empty() {
            "no groups".to_string()
        } else {
            g.iter().map(|c| format!("group {}: {} faces in {} photos", c.id, c.size, c.photos)).collect::<Vec<_>>().join("\n")
        }
    });
    Ok(())
}

fn people(store: &FaceStore, o: &mut Out, all: bool) -> R {
    let p = store.list_people(all)?;
    let v = json!(p.iter().map(|x| json!({"id": x.id, "name": x.name, "state": x.state, "faces": x.faces, "photos": x.photos, "suggestions": x.suggestions})).collect::<Vec<_>>());
    o.result(v, || {
        if p.is_empty() {
            "no people".to_string()
        } else {
            p.iter().map(|x| format!("{}\t{}\t{} photos\t{} faces{}", x.id, x.name, x.photos, x.faces, if x.state == "ignored" { "\t(removed)" } else { "" })).collect::<Vec<_>>().join("\n")
        }
    });
    Ok(())
}

fn person(store: &FaceStore, o: &mut Out, who: &str, images: bool) -> R {
    let id = resolve(store, who)?;
    let row = store.list_people(true)?.into_iter().find(|p| p.id == id).ok_or_else(|| Failure::Other("no such person".into()))?;
    let mut paths: Vec<String> = Vec::new();
    if images {
        loop {
            let page = store.person_photos(id, paths.len() as u32, 1000)?;
            if page.paths.is_empty() {
                break;
            }
            paths.extend(page.paths);
        }
    }
    let v = json!({"id": row.id, "name": row.name, "state": row.state, "faces": row.faces, "photos": row.photos, "suggestions": row.suggestions, "paths": if images { json!(paths) } else { Value::Null }});
    o.result(v, || {
        if images {
            paths.join("\n")
        } else {
            format!("{} (id {}): {} photos, {} faces, {} suggested", row.name, row.id, row.photos, row.faces, row.suggestions)
        }
    });
    Ok(())
}

fn name_group(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out, group: &str, name: &str) -> R {
    let gid: i64 = group.parse().map_err(|_| Failure::Usage("the group is its id (see `immage faces groups`)".into()))?;
    if !store.list_clusters()?.iter().any(|c| c.id == gid) {
        return Err(Failure::Usage(format!("no group {gid}")));
    }
    let faces: Vec<i64> = store.list_faces(&FaceFilter::Cluster { id: gid }, 0, 10_000)?.iter().map(|f| f.id).collect();
    let existing = store.list_people(false)?.into_iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()));
    let id = match existing {
        Some(p) => {
            store.assign(p.id, &faces)?;
            p.id
        }
        None => store.create_person(name, &faces)?,
    };
    regroup(runtime, store, models)?;
    o.result(json!({"person": id, "faces": faces.len()}), || format!("{} faces named (person {id})", faces.len()));
    Ok(())
}

fn rename(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out, who: &str, name: &str) -> R {
    let id = resolve(store, who)?;
    store.rename_person(id, name)?;
    regroup(runtime, store, models)?;
    o.result(json!({"person": id, "name": name}), || format!("person {id} is now {name}"));
    Ok(())
}

fn merge(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out, from: &str, into: &str) -> R {
    let (a, b) = (resolve(store, from)?, resolve(store, into)?);
    store.merge(a, b)?;
    regroup(runtime, store, models)?;
    o.result(json!({"from": a, "into": b}), || format!("person {a} merged into {b}"));
    Ok(())
}

fn ignore(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out, who: &str, restore: bool) -> R {
    let id = resolve(store, who)?;
    store.set_ignored(id, !restore)?;
    regroup(runtime, store, models)?;
    o.result(json!({"person": id, "removed": !restore}), || format!("person {id} {}", if restore { "restored" } else { "removed" }));
    Ok(())
}

fn face_ids(ids: &[String]) -> Result<Vec<i64>, Failure> {
    ids.iter().map(|s| s.parse::<i64>().map_err(|_| Failure::Usage(format!("{s} is not a face id")))).collect()
}

fn confirm(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out, who: &str, ids: &[String], suggested: bool) -> R {
    let id = resolve(store, who)?;
    let faces = if suggested {
        store.list_faces(&FaceFilter::Suggested { id }, 0, 10_000)?.iter().map(|f| f.id).collect()
    } else {
        face_ids(ids)?
    };
    if faces.is_empty() {
        return Err(Failure::Usage("no faces to confirm: give face ids or --suggested".into()));
    }
    store.assign(id, &faces)?;
    regroup(runtime, store, models)?;
    o.result(json!({"person": id, "confirmed": faces.len()}), || format!("{} faces confirmed for person {id}", faces.len()));
    Ok(())
}

fn reject(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out, person: Option<&str>, ids: &[String]) -> R {
    let faces = face_ids(ids)?;
    match person {
        Some(p) => store.reject_for_person(resolve(store, p)?, &faces)?,
        None => store.reject_faces(&faces)?,
    }
    regroup(runtime, store, models)?;
    o.result(json!({"rejected": faces.len()}), || format!("{} faces rejected", faces.len()));
    Ok(())
}

fn suggestions(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out) -> R {
    let r = recognizer_of(runtime, models);
    let s: Vec<MergeSuggestion> = crate::faces_cmds::compute_suggestions(store, runtime, r, &fm::thresholds_for(r, fm::wanted_strictness(runtime)))?;
    let v = json!(s.iter().map(|x| json!({"kind": x.kind, "a": x.a, "b": x.b, "similarity": x.similarity, "confidence": x.confidence})).collect::<Vec<_>>());
    o.result(v, || {
        if s.is_empty() {
            "no suggestions".to_string()
        } else {
            s.iter().map(|x| format!("{}: {} and {} ({:.2}, {})", x.kind, x.a, x.b, x.similarity, x.confidence)).collect::<Vec<_>>().join("\n")
        }
    });
    Ok(())
}

fn export(store: &FaceStore, o: &mut Out) -> R {
    let t = store.totals()?;
    let p = store.list_people(true)?;
    let v = json!({
        "photos": t.photos, "faces": t.faces, "people_count": t.people, "groups": t.groups, "unassigned": t.unreviewed,
        "people": p.iter().map(|x| json!({"id": x.id, "name": x.name, "state": x.state, "photos": x.photos, "faces": x.faces})).collect::<Vec<_>>(),
    });
    o.result(v, || p.iter().map(|x| format!("{}\t{}\t{}", x.name, x.photos, x.faces)).collect::<Vec<_>>().join("\n"));
    Ok(())
}

fn strictness(runtime: &Path, store: &FaceStore, models: &Models, o: &mut Out, level: &str) -> R {
    let s = Strictness::parse(level).ok_or_else(|| Failure::Usage("strictness is loose, balanced or strict".into()))?;
    crate::settings::Settings::load(runtime).set("faces.strictness", s.name())?;
    regroup(runtime, store, models)?;
    o.result(json!({"strictness": s.name()}), || format!("strictness {} (groups recomputed; an open app reads it at its next change, set it in Preferences while the app runs)", s.name()));
    Ok(())
}

fn delete_all(runtime: &Path, store: &FaceStore, o: &mut Out, yes: bool) -> R {
    if !yes {
        return Err(Failure::Usage("this deletes every face, person and group: add --yes".into()));
    }
    store.delete_all()?;
    let _ = std::fs::remove_dir_all(runtime.join("faces-crops"));
    crate::settings::Settings::load(runtime).set("ai.faces.enabled", "0")?;
    o.result(json!({"deleted": true}), || "all face data deleted, Faces switched off".to_string());
    Ok(())
}

// ---- engine ------------------------------------------------------------------

/// Where the engine is: --engine, DT_ENGINE_BIN, a dev checkout's build, or next to the program in a bundle.
fn engine_path(args: &Args) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "darktable-engine.exe" } else { "darktable-engine" };
    if let Some(p) = args.value("--engine") {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("DT_ENGINE_BIN") {
        return Some(PathBuf::from(p));
    }
    let mut candidates = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join("build/bin").join(exe)];
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join("darktable/bin").join(exe));
        candidates.push(dir.join("../lib/Immage/darktable/bin").join(exe));
        candidates.push(dir.join("../Resources/darktable/bin").join(exe));
    }
    candidates.into_iter().find(|c| c.is_file())
}

/// The engine through a one-line script that names ONNX Runtime when asked (--ort, or a dev build's own).
fn start_engine(runtime: &Path, args: &Args) -> Result<crate::engine::Engine, String> {
    let bin = engine_path(args).ok_or("the engine (darktable-engine) was not found: use --engine or DT_ENGINE_BIN")?;
    let ort = args.value("--ort").map(PathBuf::from).or_else(|| std::env::var_os("IMMAGE_ORT").map(PathBuf::from)).or_else(|| {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/_deps/onnxruntime/lib/libonnxruntime.so");
        (cfg!(debug_assertions) && dev.is_file()).then_some(dev)
    });
    let bin = match ort {
        #[cfg(unix)]
        Some(ort) => {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(runtime).map_err(|e| e.to_string())?;
            let wrapper = runtime.join(format!("faces-cli-engine-{}", std::process::id()));
            std::fs::write(&wrapper, format!("#!/bin/sh\nexec '{}' \"$@\" --conf 'plugins/ai/ort_library_path={}'\n", bin.display(), ort.display())).map_err(|e| e.to_string())?;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
            wrapper
        }
        _ => bin,
    };
    let engine = crate::engine::Engine::spawn(&bin, runtime, &runtime.join("models")).map_err(|e| format!("cannot start the engine: {e}"));
    if bin.file_name().is_some_and(|n| n.to_string_lossy().starts_with("faces-cli-engine-")) {
        let _ = std::fs::remove_file(&bin);
    }
    engine
}

fn apply_overrides(args: &Args, o: &mut Out, runtime: &Path) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if args.flag("--thorough") {
        fm::DETECT_OVERRIDE.store(1600, Ordering::Release);
        if fm::wanted_detect_size(runtime) != 1600 && fm::DETECT_OVERRIDE.load(Ordering::Acquire) == 1600 {
            o.note("note: --thorough (1600 px with tiles) differs from the app's detection size; the app will scan these photos again unless it uses the same");
        }
    }
    match args.value("--recognizer") {
        Some("sface") => fm::RECOGNIZER_OVERRIDE.store(1, Ordering::Release),
        Some("arcface") => fm::RECOGNIZER_OVERRIDE.store(2, Ordering::Release),
        Some(_) => return Err("the recognizer is sface or arcface".into()),
        None => {}
    }
    Ok(())
}

fn detect(runtime: &Path, args: &Args, o: &mut Out, image: &str) -> i32 {
    let path = PathBuf::from(image);
    if !path.is_absolute() {
        o.note("give the image as an absolute path");
        return EXIT_USAGE;
    }
    if !args.flag("--allow-network") && crate::volumes::MountSnapshot::take().is_network(&path) {
        o.note("that image is on a network share: add --allow-network to read it");
        return EXIT_USAGE;
    }
    if let Err(e) = apply_overrides(args, o, runtime) {
        o.note(&e);
        return EXIT_USAGE;
    }
    let models = Models::new(runtime.join("faces-models"), runtime.join("faces-eyes"));
    let root = match models.package() {
        Ok(r) => r,
        Err(e) => {
            o.note(&format!("the Faces model is not available: {e}"));
            return EXIT_UNAVAILABLE;
        }
    };
    let size = fm::wanted_detect_size(runtime);
    let Some(pic) = crate::analysis_image::picture_at(&path, size) else {
        o.note("cannot read a picture from that file (a JPEG, or a RAW with a preview)");
        return EXIT_FAIL;
    };
    let engine = match start_engine(runtime, args) {
        Ok(e) => e,
        Err(e) => {
            o.note(&e);
            return EXIT_UNAVAILABLE;
        }
    };
    let work = runtime.join("faces-cli-io");
    let result = crate::faces_service::detect_faces(&engine.tagged(), &work, &root, &pic.rgb, size > 1024);
    drop(engine);
    let _ = std::fs::remove_dir_all(&work);
    match result {
        Ok((faces, ms)) => {
            let (w, h) = (pic.rgb.width(), pic.rgb.height());
            let list: Vec<Value> = faces
                .iter()
                .map(|f| {
                    json!({"x": f.x, "y": f.y, "w": f.w, "h": f.h, "width_px": (f.w * w as f32).round(), "score": f.score,
                           "frontal": crate::faces_detect::frontal_score(f), "roll_degrees": crate::faces_detect::roll_degrees(f, w, h)})
                })
                .collect();
            o.result(json!({"image": image, "picture": [w, h], "faces": list, "ms": ms}), || {
                let mut s = format!("{} faces in a {w} x {h} picture ({ms:.0} ms)", faces.len());
                for f in &faces {
                    s += &format!("\n  box {:.3},{:.3} {:.3}x{:.3} ({} px wide)  score {:.2}  frontal {:.2}  roll {:.0} deg", f.x, f.y, f.w, f.h, (f.w * w as f32).round(), f.score, crate::faces_detect::frontal_score(f), crate::faces_detect::roll_degrees(f, w, h));
                }
                s
            });
            EXIT_OK
        }
        Err(e) => {
            o.note(&e);
            EXIT_FAIL
        }
    }
}

fn scan(runtime: &Path, args: &Args, o: &mut Out, store: &FaceStore, models: &Models, folder: &str) -> i32 {
    use std::sync::atomic::Ordering;
    let folder = PathBuf::from(folder);
    if !folder.is_absolute() {
        o.note("give the folder as an absolute path");
        return EXIT_USAGE;
    }
    let allow_network = args.flag("--allow-network");
    if !allow_network && crate::volumes::MountSnapshot::take().is_network(&folder) {
        o.note("that folder is on a network share: add --allow-network to scan it");
        return EXIT_USAGE;
    }
    if !folder.is_dir() {
        o.note("that is not a folder");
        return EXIT_USAGE;
    }
    match args.value("--recognizer") {
        None | Some("sface") => {}
        Some("arcface") if models.arcface().is_none() => {
            o.note("ArcFace is not available: its file is missing or is not the expected one");
            return EXIT_UNAVAILABLE;
        }
        Some("arcface") => {}
        Some(_) => {
            o.note("the recognizer is sface or arcface");
            return EXIT_USAGE;
        }
    }
    if let Err(e) = apply_overrides(args, o, runtime) {
        o.note(&e);
        return EXIT_USAGE;
    }
    if let Err(e) = models.package() {
        o.note(&format!("the Faces model is not available: {e}"));
        return EXIT_UNAVAILABLE;
    }
    // one command line scan at a time; the app leaves its own passes for later while this one holds the lock
    let lock = match std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(crate::faces_service::scan_lock_path(runtime)) {
        Ok(f) => f,
        Err(e) => {
            o.note(&format!("cannot open the scan lock: {e}"));
            return EXIT_STORE;
        }
    };
    if lock.try_lock().is_err() {
        o.note("another faces scan is running");
        return EXIT_STORE;
    }
    let engine = match start_engine(runtime, args) {
        Ok(e) => e,
        Err(e) => {
            o.note(&e);
            return EXIT_UNAVAILABLE;
        }
    };
    let queue = std::sync::Arc::new(crate::analysis_queue::AnalysisQueue::new(2, 256));
    let output = std::sync::Arc::new(crate::frame_output::FrameOutput::new(runtime));
    let shared_store = std::sync::Arc::new(match FaceStore::open(&runtime.join("faces.sqlite")) {
        Ok(s) => s,
        Err(e) => {
            o.note(&format!("cannot open faces.sqlite: {e}"));
            return EXIT_STORE;
        }
    });
    let models = std::sync::Arc::new(Models::new(runtime.join("faces-models"), runtime.join("faces-eyes")));
    models.enabled_flag().store(true, Ordering::Release);
    let failed_before = store.stats().map_or(0, |s| s.failed);
    let control = crate::faces_service::start(None, models.clone(), shared_store.clone(), queue.clone(), engine.tagged(), output, runtime.to_path_buf(), std::sync::Arc::new(|_| {}));
    control.set_stale_pass(false);
    control.set_defer_to_cli(false);
    control.scan_folder(folder.clone(), !args.flag("--no-subfolders"), allow_network);
    let started = std::time::Instant::now();
    let mut last = (0u64, 0u64);
    let mut idle_rounds = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let p = control.progress();
        if (p.scanned, p.faces_found) != last {
            last = (p.scanned, p.faces_found);
            o.note(&format!("scanned {} photos, {} faces found ({:.0} s)", p.scanned, p.faces_found, started.elapsed().as_secs_f64()));
        }
        // finished: the folder left the queue (everything in it is stored) and nothing is running
        if p.folder_pending == 0 && p.running == 0 {
            idle_rounds += 1;
            if idle_rounds >= 3 {
                break;
            }
        } else {
            idle_rounds = 0;
        }
        if control.health().state == "blocked" {
            o.note("the scan stopped: the model or the engine failed repeatedly");
            control.stop();
            return EXIT_UNAVAILABLE;
        }
    }
    let p = control.progress();
    control.stop();
    drop(engine);
    if let Err(e) = regroup(runtime, store, &models) {
        o.note(&format!("could not regroup: {e}"));
        return EXIT_FAIL;
    }
    let failed = store.stats().map_or(0, |s| s.failed).saturating_sub(failed_before);
    let t = store.totals().ok();
    o.result(
        json!({"scanned": p.scanned, "faces_found": p.faces_found, "unreadable": failed, "groups": t.as_ref().map(|t| t.groups), "seconds": started.elapsed().as_secs_f64()}),
        || format!("scanned {} photos, {} faces found, {} unreadable", p.scanned, p.faces_found, failed),
    );
    let _ = lock.unlock();
    if failed > 0 {
        EXIT_PARTIAL
    } else {
        EXIT_OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faces_store::NewFace;

    fn face(x: f32, e: [f32; 2]) -> NewFace {
        let mut emb = vec![0.0; 128];
        emb[0] = e[0];
        emb[1] = e[1];
        NewFace { x, y: 0.2, w: 0.2, h: 0.25, score: 0.9, lm: [0.0; 10], frontal: 0.9, roll: 0.0, aligned: true, emb }
    }
    fn sh(runtime: &Path, args: &[&str]) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(runtime, args.iter().map(|s| s.to_string()).collect(), &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }
    fn json(runtime: &Path, args: &[&str]) -> Value {
        let mut a = args.to_vec();
        a.push("--json");
        let (code, out, err) = sh(runtime, &a);
        assert_eq!(code, 0, "{args:?}: {err}");
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?}: {e}: {out}"))
    }
    /// A temp runtime with a store: two photos, each with two unnamed faces of the same two (synthetic) people.
    fn runtime() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let store = FaceStore::open(&dir.path().join("faces.sqlite")).unwrap();
        let model = fm::version_for(dir.path(), Recognizer::Sface);
        for (i, path) in ["/pics/a.jpg", "/pics/b.jpg", "/pics/c.jpg"].iter().enumerate() {
            let t = i as f32 * 0.01;
            store.put_scan(path, "fp", &model, 1000, 800, &[face(0.1, [1.0 - t, t]), face(0.6, [t, 1.0 - t])], None).unwrap();
        }
        dir
    }

    #[test]
    fn usage_errors_exit_two_and_help_exits_zero() {
        let dir = runtime();
        assert_eq!(sh(dir.path(), &[]).0, 2);
        assert_eq!(sh(dir.path(), &["--help"]).0, 0);
        assert_eq!(sh(dir.path(), &["nonsense"]).0, 2);
        assert_eq!(sh(dir.path(), &["--bogus", "status"]).0, 2);
        assert_eq!(sh(dir.path(), &["status", "extra"]).0, 2);
        assert_eq!(sh(dir.path(), &["person"]).0, 2);
        assert_eq!(sh(dir.path(), &["person", "nobody"]).0, 2);
        assert_eq!(sh(dir.path(), &["delete-all"]).0, 2, "needs --yes");
        assert_eq!(sh(dir.path(), &["strictness", "medium"]).0, 2);
        assert_eq!(sh(dir.path(), &["scan", "relative/folder"]).0, 2);
        assert_eq!(sh(dir.path(), &["detect", "photo.jpg"]).0, 2);
    }

    #[test]
    fn a_review_session_from_the_command_line() {
        let dir = runtime();
        let rt = dir.path();
        let s = json(rt, &["status"]);
        assert_eq!((s["faces"].as_u64(), s["photos"].as_u64(), s["enabled"].as_bool(), s["recognizer"].as_str()), (Some(6), Some(3), Some(false), Some("sface")));
        // nothing is grouped until the first analysis: regroup by naming
        let store = FaceStore::open(&rt.join("faces.sqlite")).unwrap();
        let models = Models::new(rt.join("faces-models"), rt.join("faces-eyes"));
        regroup(rt, &store, &models).unwrap();
        let groups = json(rt, &["groups"]);
        assert_eq!(groups.as_array().unwrap().len(), 2, "{groups}");
        assert_eq!(groups[0]["faces"], 3);
        let g0 = groups[0]["id"].as_i64().unwrap().to_string();
        let named = json(rt, &["name", &g0, "Ann Lee"]);
        assert_eq!(named["faces"], 3);
        let people = json(rt, &["people"]);
        assert_eq!((people[0]["name"].as_str(), people[0]["photos"].as_u64(), people[0]["faces"].as_u64()), (Some("Ann Lee"), Some(3), Some(3)));
        // the other group is a second person, renamed by name, then merged back by id
        // group numbers are made again after every change: read them again
        let g1 = json(rt, &["groups"])[0]["id"].as_i64().unwrap().to_string();
        let b = json(rt, &["name", &g1, "Bob"])["person"].as_i64().unwrap();
        assert_eq!(json(rt, &["rename", "bob", "Robert"])["name"], "Robert");
        assert_eq!(json(rt, &["person", "Robert"])["photos"], 3);
        let paths = json(rt, &["person", "Ann Lee", "--images"]);
        assert_eq!(paths["paths"].as_array().unwrap().len(), 3);
        let ann = json(rt, &["person", "Ann Lee"])["id"].as_i64().unwrap();
        json(rt, &["merge", &b.to_string(), &ann.to_string()]);
        assert_eq!(json(rt, &["people"]).as_array().unwrap().len(), 1);
        // ignore and restore
        json(rt, &["ignore", "Ann Lee"]);
        assert!(json(rt, &["people"]).as_array().unwrap().is_empty());
        assert_eq!(json(rt, &["people", "--all"]).as_array().unwrap().len(), 1);
        json(rt, &["ignore", "Ann Lee", "--restore"]);
        // export: counts and names, never embeddings
        let e = json(rt, &["export"]);
        assert_eq!(e["people"][0]["name"], "Ann Lee");
        assert!(!e.to_string().contains("emb"), "{e}");
        // strictness is stored
        json(rt, &["strictness", "strict"]);
        assert_eq!(json(rt, &["status"])["strictness"], "strict");
        assert!(json(rt, &["suggestions"]).is_array());
        // delete everything
        assert_eq!(sh(rt, &["delete-all", "--yes"]).0, 0);
        assert_eq!(json(rt, &["status"])["faces"], 0);
        assert_eq!(crate::settings::read_key(rt, "ai.faces.enabled").as_deref(), Some("0"));
    }

    #[test]
    fn confirm_and_reject_work_on_face_ids() {
        let dir = runtime();
        let rt = dir.path();
        let store = FaceStore::open(&rt.join("faces.sqlite")).unwrap();
        let ids: Vec<i64> = store.list_faces(&FaceFilter::Photo { path: "/pics/a.jpg".into() }, 0, 10).unwrap().iter().map(|f| f.id).collect();
        let ann = store.create_person("Ann", &[ids[0]]).unwrap();
        drop(store);
        let (code, _, err) = sh(rt, &["confirm", "Ann"]);
        assert_eq!(code, 2, "{err}");
        assert_eq!(json(rt, &["confirm", &ann.to_string(), &ids[1].to_string()])["confirmed"], 1);
        assert_eq!(json(rt, &["person", "Ann"])["faces"], 2);
        assert_eq!(sh(rt, &["confirm", "Ann", "notanumber"]).0, 2);
        // "not a face" hides it; "not this person" with --person
        assert_eq!(json(rt, &["reject", &ids[1].to_string()])["rejected"], 1);
        assert_eq!(json(rt, &["person", "Ann"])["faces"], 1);
        assert_eq!(sh(rt, &["reject"]).0, 2);
        assert_eq!(json(rt, &["status"])["rejected"], 1);
    }

    #[test]
    fn a_network_folder_and_a_missing_model_are_refused_before_anything_is_touched() {
        let dir = runtime();
        // the model is not installed in a fresh runtime: scan and detect say so (exit 4), not a crash
        let folder = tempfile::tempdir().unwrap();
        let (code, _, err) = sh(dir.path(), &["scan", folder.path().to_str().unwrap()]);
        assert_eq!(code, 4, "{err}");
        assert!(err.contains("not available"), "{err}");
        let img = folder.path().join("x.jpg");
        std::fs::write(&img, b"not a jpeg").unwrap();
        assert_eq!(sh(dir.path(), &["detect", img.to_str().unwrap()]).0, 4);
        assert_eq!(sh(dir.path(), &["scan", folder.path().join("missing").to_str().unwrap()]).0, 2);
        assert_eq!(sh(dir.path(), &["scan", folder.path().to_str().unwrap(), "--recognizer", "other"]).0, 2);
        assert_eq!(sh(dir.path(), &["scan", folder.path().to_str().unwrap(), "--recognizer", "arcface"]).0, 4, "ArcFace needs its file");
        // the folder is untouched
        assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1);
    }

    #[test]
    fn the_scan_lock_tells_the_app_to_wait() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!crate::faces_service::cli_scanning(dir.path()));
        let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(crate::faces_service::scan_lock_path(dir.path())).unwrap();
        f.try_lock().unwrap();
        assert!(crate::faces_service::cli_scanning(dir.path()));
        drop(f);
        assert!(!crate::faces_service::cli_scanning(dir.path()));
    }

    #[test]
    fn json_output_is_parseable_and_human_output_is_plain() {
        let dir = runtime();
        let (code, out, _) = sh(dir.path(), &["status"]);
        assert_eq!(code, 0);
        assert!(out.contains("3 photos") && out.contains("6 faces"), "{out}");
        assert!(serde_json::from_str::<Value>(&out).is_err());
        let (_, out, _) = sh(dir.path(), &["people"]);
        assert_eq!(out.trim(), "no people");
    }

    /// The whole command line against the real engine and models on synthetic pictures (no faces in them) and a temp store:
    /// IMMAGE_FACES_ROOT=<folder with models/> IMMAGE_TEST_ENGINE=<darktable-engine> IMMAGE_TEST_ORT=<libonnxruntime.so>
    ///   cargo test opt_in_real_cli -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs the Faces models, the built engine and ONNX Runtime"]
    fn opt_in_real_cli() {
        let var = |k: &str| PathBuf::from(std::env::var_os(k).unwrap_or_else(|| panic!("set {k}")));
        let (root, engine, ort) = (var("IMMAGE_FACES_ROOT"), var("IMMAGE_TEST_ENGINE"), var("IMMAGE_TEST_ORT"));
        let dir = tempfile::tempdir().unwrap();
        let rt = dir.path().join("rt");
        let pack = rt.join("faces-models").join(fm::REVISION);
        for f in [fm::DETECTOR_FILE, fm::RECOGNIZER_FILE] {
            let to = pack.join("models").join(f);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(root.join("models").join(f), to).unwrap();
        }
        let pics = dir.path().join("pics");
        std::fs::create_dir_all(pics.join("sub")).unwrap();
        for (i, name) in ["a.jpg", "b.jpg", "sub/c.jpg"].iter().enumerate() {
            let img = image::RgbImage::from_fn(900, 600, |x, y| image::Rgb([(x / 4 + i as u32 * 40) as u8, (y / 3) as u8, ((x ^ y) / 8) as u8]));
            img.save(pics.join(name)).unwrap();
        }
        let before: Vec<_> = std::fs::read_dir(&pics).unwrap().flatten().map(|e| e.file_name()).collect();
        let (e, o) = (engine.to_str().unwrap(), ort.to_str().unwrap());
        let (code, out, err) = sh(&rt, &["scan", pics.to_str().unwrap(), "--engine", e, "--ort", o, "--json"]);
        assert_eq!(code, 0, "{err}");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["scanned"], 3, "{out}");
        // a second scan finds nothing new and does not read the photos again
        let (code, out, _) = sh(&rt, &["scan", pics.to_str().unwrap(), "--engine", e, "--ort", o, "--json", "--no-subfolders"]);
        assert_eq!(code, 0);
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["scanned"], 0);
        let (code, out, err) = sh(&rt, &["detect", pics.join("a.jpg").to_str().unwrap(), "--engine", e, "--ort", o, "--json"]);
        assert_eq!(code, 0, "{err}");
        let d: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(d["faces"].as_array().unwrap().len(), 0);
        assert_eq!(json(&rt, &["status"])["photos"], 3);
        // nothing was written into the picture folder
        let after: Vec<_> = std::fs::read_dir(&pics).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(before.len(), after.len());
        assert!(!crate::faces_service::cli_scanning(&rt), "the scan lock is released");
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// People, in the grid's place (notes/faces-ux-spec.md, phases P1 and P2): the consent card while it is off,
// then a section rail (People, To review, Unassigned, Ignored), the virtualised grid, an inspector, and four
// fixed rows (the scan strip, the toolbar, the selection bar, the Undo line) that are always laid out. The
// state is peopleStore.ts's and the rules are peopleModel.ts's; this file is the layout and the wiring.
import "./people.css";
import { X } from "lucide-react";
import { createPortal } from "react-dom";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FaceRow, PackInfo } from "./bindings";
import type { ActionId } from "./aiPacks";
import { createUndoTimer } from "./undoLine";
import { direction, formatNumber, t } from "./i18n";
import { PeopleConsent } from "./PeopleConsent";
import { Crop, PeopleGrid, tileLabel, type GridOverlay } from "./PeopleGrid";
import { NamePopover } from "./PeopleName";
import {
  NO_SEL, SECTIONS, TILE_DEFAULT, TILE_MAX, TILE_MIN, keepSelected, scanNotes, undoMs, sectionCounts, sectionTiles, select, stableOrder, viewKind,
  type Intent, type Section, type Sel, type SelAction, type Tile, type UndoEntry,
} from "./peopleModel";
import {
  clearUndo, confirmTiles, deleteAllFaceData, groupFaces, ignoreTiles, failedPhotos, loadMoreIgnored, loadMoreUnknown, maybeFaces, mergePeople, nameTiles, notPerson, personFaces,
  refreshAll, renamePerson, restorePeople, retryFailed, unignoreFaces, setCover, togglePause, turnOff, turnOn, undoLast, usePeople, type NameTarget, type Scope,
} from "./peopleStore";
import { settings } from "./settings";

const SIZE_KEY = "people.tile";
const readSize = () => { const n = Number(settings.get(SIZE_KEY)); return n >= TILE_MIN && n <= TILE_MAX ? n : TILE_DEFAULT; };

export type Dialog =
  | { kind: "merge"; ids: number[]; into: number }
  | { kind: "cover"; id: number; faces: FaceRow[] }
  | { kind: "delete" }
  | { kind: "failed"; paths: string[] }
  | { kind: "off" };

function Modal({ title, onClose, children, foot }: { title: string; onClose: () => void; children: React.ReactNode; foot: React.ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.querySelector<HTMLButtonElement>("button[data-first]")?.focus();
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); e.preventDefault(); onClose(); } };
    document.addEventListener("keydown", key, true);
    return () => document.removeEventListener("keydown", key, true);
  }, [onClose]);
  return createPortal(
    <div className="modal-backdrop pack-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal pack-dialog" role="dialog" aria-modal="true" aria-label={title} ref={box}>
        <div className="modal-head"><h2>{title}</h2></div>
        <div className="modal-body">{children}</div>
        <div className="modal-foot">{foot}</div>
      </div>
    </div>, document.body);
}

export const undoText = (u: UndoEntry) => {
  const v = { n: u.n, count: formatNumber(u.n), name: u.name, other: u.other };
  return t(`people.undo.${u.kind}`, v);
};

export type PeopleViewProps = {
  folder: string | null;                 // the open folder: the scan's other scope
  hasCatalog: boolean;
  pack: PackInfo | null;
  packBusy?: boolean;
  onPack: (a: ActionId) => void;
};

export function PeopleView({ folder, hasCatalog, pack, packBusy = false, onPack }: PeopleViewProps) {
  const s = usePeople();
  const dir = direction === "rtl" ? "rtl" : "ltr";
  const kind = viewKind(s.status, s.failed, s.scan, s.loaded);
  const [section, setSection] = useState<Section>("people");
  const [picked, setPicked] = useState(false);
  const [sheet, setSheet] = useState<{ key: string; title: string; faces: FaceRow[] } | null>(null);
  const [sel, setSel] = useState<Sel>(NO_SEL);
  const [size, setSize] = useState(readSize);
  const [query, setQuery] = useState("");
  const [naming, setNaming] = useState<{ keys: string[]; seed: string; index: number; faces: number; groups: number } | null>(null);
  const [renameKey, setRenameKey] = useState<string | null>(null);
  const [dupe, setDupe] = useState<{ name: string; id: number; target: number } | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);

  const data = s.data;
  const counts = useMemo(() => sectionCounts(data, s.status?.totals ?? null), [data, s.status?.totals]);
  const names = useMemo(() => new Map(data.people.filter((p) => p.state === "active").map((p) => [p.id, p.name])), [data.people]);

  // until the user picks a section the view lands on To review when there is something there: when the lists first
  // arrive, and later (while no one is named yet) when the first groups come in during a scan
  const landed = useRef(false);
  useEffect(() => {
    if (!s.loaded || picked || section !== "people") return;
    if (!landed.current || counts.people === 0) {
      landed.current = true;
      if (counts.review > 0) setSection("review");
    }
  }, [s.loaded, picked, section, counts.people, counts.review]);

  // tiles keep the order the user is looking at; only a section change or a sort starts it afresh
  const order = useRef<{ id: string; keys: string[] }>({ id: "", keys: [] });
  const tiles = useMemo<Tile[]>(() => {
    const id = sheet ? `sheet:${sheet.key}` : section;
    const base: Tile[] = sheet ? sheet.faces.map((f) => ({ key: `f:${f.id}`, kind: "face" as const, id: f.id, path: f.path, maybe: f.suggested_person }))
      : sectionTiles(section, data);
    const out = stableOrder(order.current.id === id ? order.current.keys : null, base);
    order.current = { id, keys: out.map((x) => x.key) };
    const q = query.trim().toLowerCase();
    return q ? out.filter((x) => (x.kind === "person" || x.kind === "maybe") && x.name.toLowerCase().includes(q)) : out;
  }, [section, sheet, data, query]);
  const keys = useMemo(() => tiles.map((x) => x.key), [tiles]);
  const byKey = useMemo(() => new Map(tiles.map((x) => [x.key, x])), [tiles]);
  useEffect(() => { setSel((p) => keepSelected(p, keys)); }, [keys]);

  const apply = useCallback((a: SelAction) => setSel((p) => select(p, a, keys)), [keys]);
  const goSection = (x: Section) => { setPicked(true); setSection(x); setSheet(null); setSel(NO_SEL); setNaming(null); setQuery(""); };
  const selected = [...sel.keys].map((k) => byKey.get(k)).filter((x): x is Tile => !!x);
  const target = selected.length ? selected : tiles[sel.active] ? [tiles[sel.active]] : [];
  const nameable = target.filter((x) => x.kind !== "person");
  const people = target.filter((x) => x.kind === "person");

  // ---- actions ----
  const openSheet = async (tile: Tile) => {
    if (tile.kind !== "group" && tile.kind !== "maybe") return;
    const faces = tile.kind === "group" ? await groupFaces(tile.id) : await maybeFaces(tile.person);
    setSheet({ key: tile.key, title: tile.kind === "maybe" ? t("people.suggest.maybe", { name: tile.name }) : t("people.group.hint"), faces });
    setSel(NO_SEL);
  };
  const startNaming = (seed: string) => {
    if (!nameable.length) return;
    // what is named is faces: a group stands for all of its faces
    const faces = nameable.reduce((n, x) => n + (x.kind === "group" || x.kind === "maybe" ? x.size : 1), 0);
    setNaming({ keys: nameable.map((x) => x.key), seed, index: sel.active, faces, groups: nameable.filter((x) => x.kind === "group").length });
  };
  const finishNaming = (to: NameTarget, next: boolean) => {
    if (!naming) return;
    const gone = new Set(naming.keys);
    const left = keys.filter((k) => !gone.has(k));
    const at = Math.min(naming.index, Math.max(0, left.length - 1));
    setNaming(null);
    if (sheet) {
      const faceIds = new Set(naming.keys.map((k) => Number(k.slice(2))));
      const rest = sheet.faces.filter((f) => !faceIds.has(f.id));
      if (rest.length) setSheet({ ...sheet, faces: rest }); else setSheet(null);
    }
    setSel(left.length ? select(NO_SEL, { type: "click", index: at }, left) : NO_SEL);
    void nameTiles(naming.keys, to).then(() => {
      if (next && left.length) {
        const nt = byKey.get(left[at]);
        setNaming({ keys: [left[at]], seed: "", index: at, faces: nt && (nt.kind === "group" || nt.kind === "maybe") ? nt.size : 1, groups: nt?.kind === "group" ? 1 : 0 });
      }
    });
  };
  const rename = async (key: string, name: string | null) => {
    setRenameKey(null);
    const tile = byKey.get(key);
    if (!name || !tile || tile.kind !== "person" || name.trim() === tile.name) return;
    const other = data.people.find((p) => p.state === "active" && p.id !== tile.id && p.name.toLowerCase() === name.trim().toLowerCase());
    if (other) { setDupe({ name: other.name, id: other.id, target: tile.id }); return; }
    setDupe(null);
    await renamePerson(tile.id, name.trim());
  };
  const askMerge = () => {
    if (people.length < 2) return;
    const ids = people.map((x) => (x.kind === "person" ? x.id : 0));
    const most = [...people].sort((a, b) => (b.kind === "person" ? b.photos : 0) - (a.kind === "person" ? a.photos : 0))[0];
    setDialog({ kind: "merge", ids, into: most.kind === "person" ? most.id : ids[0] });
  };
  const askCover = async () => {
    const p = people[0];
    if (!p || p.kind !== "person") return;
    setDialog({ kind: "cover", id: p.id, faces: await personFaces(p.id) });
  };
  const ignore = () => {
    if (section === "ignored") {
      void restorePeople(people.map((x) => (x.kind === "person" ? x.id : 0)));
      void unignoreFaces(target.filter((x) => x.kind === "face").map((x) => x.key));
      setSel(NO_SEL);
      return;
    }
    const k = target.filter((x) => x.kind !== "maybe").map((x) => x.key);
    if (k.length) void ignoreTiles(k);
    setSel(NO_SEL);
  };
  const reject = () => {
    const maybes = new Map<string, number | null>(target.map((x) => [x.key, x.kind === "face" ? x.maybe : x.kind === "maybe" ? x.person : null]));
    const withPerson = target.filter((x) => maybes.get(x.key) != null).map((x) => x.key);
    if (withPerson.length) void notPerson(withPerson, maybes, names);
    const rest = target.filter((x) => maybes.get(x.key) == null && x.kind !== "person").map((x) => x.key);
    if (rest.length) void ignoreTiles(rest);
    setSel(NO_SEL);
  };
  const act = (i: Intent) => {
    switch (i.kind) {
      case "name": startNaming(i.seed); break;
      case "open": if (tiles[sel.active]) void openSheet(tiles[sel.active]); break;
      case "rename": if (tiles[sel.active]?.kind === "person") setRenameKey(tiles[sel.active].key); break;
      case "merge": askMerge(); break;
      case "ignore": ignore(); break;
      case "reject": reject(); break;
      case "confirm": void confirmTiles(target.filter((x) => x.kind === "maybe").map((x) => x.key), names); setSel(NO_SEL); break;
      case "cover": void askCover(); break;
      case "undo": if (s.undo) void undoLast(s.undo.steps); break;
      case "clear": if (!sel.keys.size && sheet) setSheet(null); setNaming(null); break;
      default: break;
    }
  };

  // ---- the Undo line ----
  const timer = useRef<ReturnType<typeof createUndoTimer> | null>(null);
  useEffect(() => {
    timer.current?.cancel();
    // a change to many faces (a whole group) stays up longer
    timer.current = createUndoTimer(() => clearUndo(), s.undo ? undoMs(s.undo.n) : undefined);
    if (s.undo) timer.current.start();
  }, [s.undo?.id]);
  // Ctrl or Cmd+Z takes back the latest change while People is up and no text field has the keyboard: the app's own
  // Ctrl+Z is the edit history, which is not what the view is showing
  const undoRef = useRef(s);
  undoRef.current = s;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey || (e.key !== "z" && e.key !== "Z")) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName))) return;
      if (document.querySelector(".modal-backdrop")) return;
      const cur = undoRef.current;
      if (!cur.undo && !cur.status?.undo) return;
      e.preventDefault(); e.stopPropagation();
      void undoLast(cur.undo?.steps ?? 1);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  useEffect(() => () => timer.current?.cancel(), []);

  // ---- rows ----
  const scan = s.scan;
  // the count as far as it is known, and the time left only once photos have been timed: never a placeholder
  const stripText = scan.kind === "scanning"
    ? scan.total === null ? t("people.scan.lineCount", { done: formatNumber(scan.done), n: scan.done })
      : scan.left === null ? t("people.scan.lineNoEta", { done: formatNumber(scan.done), total: formatNumber(scan.total) })
      : t("people.scan.line", { done: formatNumber(scan.done), total: formatNumber(scan.total), left: etaText(scan.left) })
    : scan.kind === "paused" ? t("people.scan.paused", { n: scan.total == null ? 0 : Math.max(0, scan.total - scan.done), count: formatNumber(scan.total == null ? 0 : Math.max(0, scan.total - scan.done)) })
    : scan.kind === "waiting" ? t("people.scan.counting")
    : scan.kind === "retrying" ? t("people.scan.retrying", { reason: scan.reason ?? "" })
    : scan.kind === "blocked" ? t("people.scan.blocked", { reason: scan.reason ?? "" }) : "";
  const notes = scanNotes(s.status);
  const stripTone = scan.kind === "blocked" ? "bad" : scan.kind === "retrying" ? "warn" : undefined;
  const stripAction = scan.kind === "scanning" ? { label: t("aimodels.pause"), run: togglePause }
    : scan.kind === "paused" ? { label: t("aimodels.resume"), run: togglePause }
    : scan.kind === "blocked" || scan.kind === "retrying" ? { label: t("people.scan.tryAgain"), run: retryFailed } : null;

  // what the scan covers, in words: the folder that was chosen, or the Catalog
  const scopeFolder = settings.get("faces.scope") === "folder" ? settings.get("faces.scope_folder") : null;
  const scopeText = scopeFolder ? t("people.scope.folder", { name: scopeFolder.split(/[\\/]/).filter(Boolean).pop() ?? scopeFolder }) : t("people.scope.catalog");
  const scopeTitle = scopeFolder ?? t("people.scope.title");
  const sectionLabel = (x: Section) => t(`people.section.${x}`);
  const one = target.length === 1 ? target[0] : null;
  const maybeOnly = nameable.length > 0 && nameable.every((x) => x.kind === "maybe");
  const faceTotal = selected.reduce((n, x) => n + (x.kind === "person" ? x.faces : x.kind === "face" ? 1 : x.kind === "group" || x.kind === "maybe" ? x.size : 0), 0);
  const title = one ? tileLabel(one, names).name : target.length > 1 ? t("people.selected", { n: target.length, count: formatNumber(target.length) }) : sectionLabel(section);
  const facts = one ? tileLabel(one, names).sub : target.length > 1 ? t("people.tile.faces", { n: faceTotal, count: formatNumber(faceTotal) })
    : s.status ? t("people.facts.totals", { people: formatNumber(s.status.totals.people), faces: formatNumber(s.status.totals.faces) }) : "";

  const gridLabel = sheet ? sheet.title : sectionLabel(section);
  const state = kind === "loading" ? "loading" : kind === "scanning" ? "scanning" : kind === "error" ? "error" : kind === "none" ? "none" : null;
  const emptyText = !state && tiles.length === 0
    ? query ? t("people.empty.search") : section === "review" ? t("people.empty.reviewed") : section === "people" ? t("people.empty.people") : section === "unassigned" ? t("people.empty.unassigned") : t("people.empty.ignored")
    : "";

  return (
    <div className="people-view" role="region" aria-label={t("people.label")} data-kind={kind}>
      <div className="people-strip" role="status" aria-live="polite" data-tone={stripTone}>
        <span className="people-strip-text" title={stripText}>{stripText}</span>
        {notes.network > 0 && <span className="people-strip-note" title={t("people.scan.network", { n: notes.network, count: formatNumber(notes.network) })}>{t("people.scan.network", { n: notes.network, count: formatNumber(notes.network) })}</span>}
        {notes.offline > 0 && <span className="people-strip-note" data-tone="warn" title={t("people.scan.offline")}>{t("people.scan.offlineCount", { n: notes.offline, count: formatNumber(notes.offline) })}</span>}
        {notes.failed > 0 && <button type="button" className="people-link" onClick={() => void failedPhotos().then((paths) => setDialog({ kind: "failed", paths }), () => setDialog({ kind: "failed", paths: [] }))}>{t("people.scan.failed", { n: notes.failed, count: formatNumber(notes.failed) })}</button>}
        {stripAction && <button type="button" className="people-btn" onClick={() => void stripAction.run()}>{stripAction.label}</button>}
      </div>

      <div className="people-bar">
        {kind === "ready" || kind === "scanning" || kind === "none" ? <>
          <span className="people-scope" title={scopeTitle}>{scopeText}</span>
          <input className="people-find" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("people.find")} aria-label={t("people.find")} />
          <label className="people-size">{t("people.size")}
            <input type="range" min={TILE_MIN} max={TILE_MAX} step={4} value={size}
                   onChange={(e) => { const v = Number(e.target.value); setSize(v); settings.set(SIZE_KEY, String(v)); }} aria-label={t("people.size")} /></label>
        </> : null}
      </div>

      <div className="people-selbar" role="toolbar" aria-label={t("people.selection")}>
        {target.length > 0 && sel.keys.size > 0 && <>
          <span className="people-sel-count">{t("people.selected", { n: sel.keys.size, count: formatNumber(sel.keys.size) })}</span>
          {nameable.length > 0 && !maybeOnly && <button type="button" className="people-btn" onClick={() => startNaming("")}>{t("people.act.name")}</button>}
          {maybeOnly && <button type="button" className="people-btn" onClick={() => act({ kind: "confirm" })}>{t("people.act.confirm")}</button>}
          {nameable.length > 0 && <button type="button" className="people-btn" onClick={reject}>{maybeOnly ? t("people.act.notThem") : t("people.act.notFace")}</button>}
          {people.length >= 2 && section === "people" && <button type="button" className="people-btn" onClick={askMerge}>{t("people.act.merge")}</button>}
          {people.length === 1 && section === "people" && <button type="button" className="people-btn" onClick={() => setRenameKey(people[0].key)}>{t("people.act.rename")}</button>}
          {people.length > 0 && <button type="button" className="people-btn" onClick={ignore}>{section === "ignored" ? t("people.act.restore") : t("people.act.ignore")}</button>}
          {nameable.length > 0 && !maybeOnly && <button type="button" className="people-btn" onClick={ignore}>{t("people.act.ignore")}</button>}
          <button type="button" className="people-btn" aria-label={t("people.act.clear")} title={t("people.act.clear")} onClick={() => setSel(NO_SEL)}><X size={12} aria-hidden="true" /></button>
        </>}
      </div>

      {kind === "consent" ? (
        <PeopleConsent pack={pack} folder={folder} hasCatalog={hasCatalog} busy={s.busy || packBusy} error={s.error} onPack={onPack}
                       onTurnOn={(scope: Scope) => void turnOn(scope, folder)} />
      ) : (
        <div className="people-body">
          <div className="people-rail" role="tablist" aria-orientation="vertical" aria-label={t("people.sections")}>
            {SECTIONS.map((x) => {
              const c = counts[x];
              return (
                <button key={x} type="button" role="tab" className="people-rail-item" aria-selected={section === x && !sheet} tabIndex={section === x ? 0 : -1}
                        onClick={() => goSection(x)}
                        onKeyDown={(e) => {
                          const i = SECTIONS.indexOf(x), d = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
                          if (!d) return;
                          e.preventDefault();
                          const to = SECTIONS[(i + d + SECTIONS.length) % SECTIONS.length];
                          goSection(to);
                          (e.currentTarget.parentElement?.querySelectorAll<HTMLElement>("[role=tab]")[SECTIONS.indexOf(to)])?.focus();
                        }}>
                  <span className="people-rail-label">{sectionLabel(x)}</span>
                  <span className="people-rail-dot" data-on={x === "review" && c > 0 ? "" : undefined} aria-hidden="true" />
                  <span className="people-rail-count">{formatNumber(c)}</span>
                </button>
              );
            })}
          </div>

          <div className="people-main">
            <div className="people-crumb" hidden={!sheet}>
              {sheet && <>
                <button type="button" className="people-btn" onClick={() => setSheet(null)}>{t("people.back")}</button>
                <span>{sheet.title}</span>
                <span>{t("people.tile.faces", { n: sheet.faces.length, count: formatNumber(sheet.faces.length) })}</span>
              </>}
            </div>
            <PeopleGrid tiles={tiles} section={section} open={!!sheet} size={size} sel={sel} byId={names} renameKey={renameKey} label={gridLabel} dir={dir}
                        onSel={apply} onIntent={act} onRename={(k, n) => void rename(k, n)}
                        onNearEnd={sheet ? undefined : section === "unassigned" ? () => void loadMoreUnknown() : section === "ignored" ? () => void loadMoreIgnored() : undefined}
                        empty={<>
                          {(state === "loading" || state === "scanning") && <div className="people-skel" aria-hidden="true">{Array.from({ length: 12 }, (_, i) => <i key={i} />)}</div>}
                          {(state === "scanning" || state === "none" || state === "error" || state === "loading" || emptyText) && (
                            <div className="people-state">
                              {state === "scanning" ? <span>{t("people.state.scanning")}</span>
                                : state === "none" ? <><b>{t("people.state.none")}</b><span>{t("people.state.scanned", { n: s.scan.done, count: formatNumber(s.scan.done) })}</span></>
                                : state === "error" ? <><b>{t("people.state.error")}</b><button type="button" className="people-btn" onClick={() => void refreshAll(true)}>{t("people.scan.tryAgain")}</button></>
                                : state === "loading" ? <span>{t("people.state.loading")}</span>
                                : <span>{emptyText}</span>}
                            </div>
                          )}
                        </>}
                        overlay={(o: GridOverlay) => naming ? (
                          <NamePopover key={naming.keys.join()} o={o} index={naming.index} count={naming.faces} group={naming.groups === 1 && naming.keys.length === 1} seed={naming.seed} people={data.people}
                                       onPick={finishNaming} onClose={() => { setNaming(null); }} />
                        ) : null} />
          </div>

          <aside className="people-inspector" aria-label={t("people.inspector")}>
            <h3 title={title}>{title}</h3>
            <p className="people-facts">{facts}</p>
            <div className="people-actions">
              <button type="button" className="people-btn" disabled={!nameable.length || maybeOnly} onClick={() => startNaming("")}>{t("people.act.name")}</button>
              <button type="button" className="people-btn" disabled={!maybeOnly} onClick={() => act({ kind: "confirm" })}>{t("people.act.confirm")}</button>
              <button type="button" className="people-btn" disabled={!nameable.length} onClick={reject}>{maybeOnly ? t("people.act.notThem") : t("people.act.notFace")}</button>
              <button type="button" className="people-btn" disabled={people.length !== 1 || section !== "people"} onClick={() => people[0] && setRenameKey(people[0].key)}>{t("people.act.rename")}</button>
              <button type="button" className="people-btn" disabled={people.length !== 1 || section !== "people"} onClick={() => void askCover()}>{t("people.act.cover")}</button>
              <button type="button" className="people-btn" disabled={people.length < 2 || section !== "people"} onClick={askMerge}>{t("people.act.merge")}</button>
              <button type="button" className="people-btn" disabled={!target.length || (target.some((x) => x.kind === "maybe") && section === "review" && target.every((x) => x.kind === "maybe"))} onClick={ignore}>
                {section === "ignored" ? t("people.act.restore") : t("people.act.ignore")}</button>
            </div>
            <hr />
            <p className="note">{dupe ? <>{t("people.rename.exists", { name: dupe.name })}{" "}
              <button type="button" className="people-btn" onClick={() => { setDialog({ kind: "merge", ids: [dupe.target], into: dupe.id }); setDupe(null); }}>{t("people.act.merge")}</button></>
              : t("people.inspector.note")}</p>
            <div className="people-actions">
              <button type="button" className="people-btn" disabled={scan.kind !== "scanning" && scan.kind !== "paused"} onClick={() => void togglePause()}>{scan.kind === "paused" ? t("aimodels.resume") : t("aimodels.pause")}</button>
              <button type="button" className="people-btn" onClick={() => setDialog({ kind: "off" })}>{t("people.off.act")}</button>
              <button type="button" className="people-btn" onClick={() => setDialog({ kind: "delete" })}>{t("people.delete.act")}</button>
            </div>
          </aside>
        </div>
      )}

      <div className="people-undo" role="status" aria-live="polite"
           onPointerEnter={() => timer.current?.hold("hover")} onPointerLeave={() => timer.current?.release("hover")}
           onFocus={() => timer.current?.hold("focus")} onBlur={() => timer.current?.release("focus")}>
        {s.error ? <span className="people-undo-text people-error" title={s.error}>{s.error}</span>
          : s.undo ? <>
            <span className="people-undo-text">{undoText(s.undo)}</span>
            <button type="button" className="people-btn" onClick={() => void undoLast(s.undo!.steps)}>{t("people.undo.button")}</button>
          </> : null}
      </div>

      {dialog?.kind === "merge" && <MergeDialog d={dialog} people={data.people} onChange={setDialog} onClose={() => setDialog(null)}
                                                onMerge={(from, into) => { setDialog(null); setSel(NO_SEL); void mergePeople(from, into); }} />}
      {dialog?.kind === "cover" && (
        <Modal title={t("people.cover.title")} onClose={() => setDialog(null)} foot={<button type="button" className="btn" data-first onClick={() => setDialog(null)}>{t("aimodels.act.cancel")}</button>}>
          <div className="people-cover-grid">
            {dialog.faces.map((f) => (
              <button key={f.id} type="button" aria-pressed={data.people.find((p) => p.id === dialog.id)?.cover_face_id === f.id}
                      aria-label={t("people.cover.use")} onClick={() => { setDialog(null); void setCover(dialog.id, f.id); }}>
                <Crop face={f.id} tile={56} />
              </button>
            ))}
          </div>
        </Modal>
      )}
      {dialog?.kind === "failed" && (
        <Modal title={t("people.failed.title")} onClose={() => setDialog(null)}
               foot={<><button type="button" className="btn" data-first onClick={() => setDialog(null)}>{t("people.failed.close")}</button>
                 <button type="button" className="btn primary" onClick={() => { setDialog(null); void retryFailed(); }}>{t("people.scan.tryAgain")}</button></>}>
          <p className="pack-lead">{t("people.failed.body")}</p>
          <ul className="people-failed">{dialog.paths.map((x) => <li key={x} title={x}>{x}</li>)}</ul>
        </Modal>
      )}
      {dialog?.kind === "off" && (
        <Modal title={t("people.off.title")} onClose={() => setDialog(null)}
               foot={<><button type="button" className="btn" data-first onClick={() => setDialog(null)}>{t("aimodels.act.cancel")}</button>
                 <button type="button" className="btn primary" onClick={() => { setDialog(null); void turnOff(); }}>{t("people.off.confirm")}</button></>}>
          <p className="pack-lead">{t("people.off.body")}</p>
        </Modal>
      )}
      {dialog?.kind === "delete" && (
        <Modal title={t("people.delete.title")} onClose={() => setDialog(null)}
               foot={<><button type="button" className="btn" data-first onClick={() => setDialog(null)}>{t("aimodels.act.cancel")}</button>
                 <button type="button" className="btn primary" onClick={() => { setDialog(null); void deleteAllFaceData(); }}>{t("people.delete.confirm")}</button></>}>
          <p className="pack-lead">{t("people.delete.body", { people: formatNumber(s.status?.totals.people ?? 0), faces: formatNumber(s.status?.totals.faces ?? 0) })}</p>
        </Modal>
      )}
    </div>
  );
}

function etaText(seconds: number): string {
  if (seconds < 90) return t("aimodels.eta.seconds", { n: Math.max(5, Math.round(seconds / 5) * 5) });
  if (seconds < 90 * 60) return t("aimodels.eta.minutes", { n: Math.round(seconds / 60) });
  return t("aimodels.eta.hours", { n: Math.round(seconds / 3600) });
}

function MergeDialog({ d, people, onChange, onClose, onMerge }: {
  d: Extract<Dialog, { kind: "merge" }>; people: { id: number; name: string; cover_face_id: number | null }[];
  onChange: (d: Dialog) => void; onClose: () => void; onMerge: (from: number[], into: number) => void;
}) {
  const by = new Map(people.map((p) => [p.id, p]));
  const into = by.get(d.into);
  const from = d.ids.filter((id) => id !== d.into);
  const all = [...new Set([...d.ids, d.into])];
  const first = by.get(from[0] ?? -1);
  return (
    <Modal title={t("people.merge.title", { a: first?.name ?? "", b: into?.name ?? "" })} onClose={onClose}
           foot={<><button type="button" className="btn" data-first onClick={onClose}>{t("aimodels.act.cancel")}</button>
             <button type="button" className="btn primary" disabled={!from.length} onClick={() => onMerge(from, d.into)}>{t("people.act.merge")}</button></>}>
      <div className="people-merge" aria-hidden="true">
        {from.slice(0, 3).map((id) => <Crop key={id} face={by.get(id)?.cover_face_id ?? null} tile={72} />)}
        <span className="arrow">-&gt;</span>
        <Crop face={into?.cover_face_id ?? null} tile={72} />
      </div>
      <p className="pack-lead">{t("people.merge.body", { b: into?.name ?? "" })}</p>
      {all.length > 1 && (
        <label className="people-merge-pick">{t("people.merge.keep")}
          <select value={d.into} onChange={(e) => onChange({ ...d, into: Number(e.target.value) })}>
            {all.map((id) => <option key={id} value={id}>{by.get(id)?.name}</option>)}
          </select>
        </label>
      )}
    </Modal>
  );
}

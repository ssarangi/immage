// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Lightroom's Keywording panel: the photo's keywords, a box to add new or
// known ones, and the recently used ones a click away. keywords are
// darktable's tags, "|" between the levels of a hierarchy; the engine keeps
// them in the sidecar and remembers every one ever used
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { commands } from "./commands";
import { settings } from "./settings";
import { t } from "./i18n";
import { menuKeys, useCardMenu } from "./cardMenu";
import { createUndoTimer, mergeUndo, removalItems, undoSummary, type UndoLine } from "./undoLine";
import { autoGroupState, groupRows, type AutoInput, type KeywordItem } from "./keywordGroup";

const RECENT_KEY = "immage.recentKeywords";
const RECENT_MAX = 9;

function loadRecent(): string[] {
  try {
    const v = JSON.parse(settings.get(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((k) => typeof k === "string") : [];
  } catch { return []; }
}
function storeRecent(v: string[]) {
  try { settings.set(RECENT_KEY, JSON.stringify(v)); } catch { /* not kept */ }
}

// what was typed as darktable's tag: Lightroom's "Places > Paris" or
// darktable's "Places|Paris", each level trimmed
export function parseKeyword(typed: string): string {
  return typed.split(/[>|]/).map((p) => p.trim()).filter(Boolean).join("|");
}

const leaf = (k: string) => k.slice(k.lastIndexOf("|") + 1);
const path = (k: string) => k.split("|").join(" > ");

// the box to type keywords in, with the known ones that match below it:
// Enter takes the highlighted suggestion for the last entry, or the text as
// typed; commas separate several
export function KeywordInput({ attached, known, onAdd, addOnBlur = false, autoFocus = false, disabled = false }: {
  attached: Set<string>;         // keywords already there, not suggested
  known: string[];               // every keyword ever used
  onAdd: (names: string[]) => void;
  // what is typed is added when the box loses focus, as a setting nobody
  // presses Enter for would otherwise be lost
  addOnBlur?: boolean;
  autoFocus?: boolean;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState("");
  const [hi, setHi] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const discarding = useRef(false);   // Escape blurs the box without adding

  // known keywords whose name, or a level of it, starts with what is typed;
  // those whose own name does come first
  const query = draft.split(",").pop()!.trim().toLowerCase();
  const suggestions = useMemo(() => {
    if (!query) return [];
    const q = parseKeyword(query).toLowerCase();
    const hits = known.filter((k) => !attached.has(k)
      && k.toLowerCase().split("|").some((p) => p.startsWith(q) || k.toLowerCase().startsWith(q)));
    return hits.sort((a, b) => Number(!leaf(a).toLowerCase().startsWith(q)) - Number(!leaf(b).toLowerCase().startsWith(q))
                               || a.localeCompare(b)).slice(0, 8);
  }, [query, known, attached]);
  useEffect(() => setHi(0), [query]);

  const commit = () => {
    const parts = draft.split(",");
    const last = parts.pop()!;
    onAdd([...parts, suggestions[hi] ?? last]);
    setDraft("");
  };

  return (
    <div className="kw-add">
      <input ref={input} autoFocus={autoFocus} disabled={disabled} className="field" value={draft} placeholder={t("library.keywording.add")}
             onChange={(e) => setDraft(e.target.value)}
             onBlur={() => {
               if (addOnBlur && !discarding.current && draft.trim()) { onAdd(draft.split(",")); setDraft(""); }
               discarding.current = false;
             }}
             onKeyDown={(e) => {
               if (e.key === "Enter") { e.preventDefault(); commit(); }
               else if (e.key === "ArrowDown" && suggestions.length) { e.preventDefault(); setHi((h) => (h + 1) % suggestions.length); }
               else if (e.key === "ArrowUp" && suggestions.length) {
                 e.preventDefault();
                 setHi((h) => (h + suggestions.length - 1) % suggestions.length);
               }
               else if (e.key === "Escape") { discarding.current = true; setDraft(""); input.current?.blur(); }
             }} />
      {suggestions.length > 0 && (
        <ul className="kw-suggest">
          {suggestions.map((k, i) => (
            <li key={k} className={i === hi ? "hi" : ""}
                onPointerDown={(e) => { e.preventDefault(); onAdd([...draft.split(",").slice(0, -1), k]); setDraft(""); }}>
              <span>{leaf(k)}</span>
              {k.includes("|") && <span className="kw-path">{path(k.slice(0, k.lastIndexOf("|")))}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// the chips of a list of keywords, each taken off with its x. One roving tab stop: Tab enters the list,
// the arrow keys (swapped in right-to-left text) and Home/End move along it, Backspace or Delete takes
// the focused one off. `ai` names the keywords Auto-keywords added: a sparkle, a tint and an accessible
// name say so (never color alone). `collapse` keeps a long list to two rows with a "+N" button.
const ROWS = 2;
const MORE_W = 34;     // room kept for the "+N" button at the end of the last row, px
export function KeywordChips({ keywords, onRemove, ai, disabled = false, label, collapse = false, counts, conf, onKeep, onAddMixed }: {
  keywords: string[]; onRemove: (name: string) => void; ai?: ReadonlySet<string>; disabled?: boolean;
  label?: string; collapse?: boolean;
  // a selection: the photos each keyword is on (a keyword on only some is dimmed with "n/m")
  counts?: ReadonlyMap<string, KeywordItem>;
  conf?: ReadonlyMap<string, number>;      // the confidence of Auto-keywords' keywords, percent
  onKeep?: (name: string) => void;         // click or Enter on an Auto-keywords chip keeps it
  onAddMixed?: (name: string) => void;     // click or Enter on a chip on only some of the photos adds it to all
}) {
  const wrap = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);     // kept while the card stays, not per photo
  const [limit, setLimit] = useState<number | null>(null);   // null: everything is drawn, to measure it
  const [width, setWidth] = useState(0);
  const next = useRef<{ key: string | null; at: number } | null>(null);
  const names = keywords.join("\0");
  const collapsed = collapse && !expanded;

  useLayoutEffect(() => {
    const el = wrap.current;
    if (!collapse || !el) return;
    const ro = new ResizeObserver(() => setWidth(Math.round(el.clientWidth)));
    ro.observe(el);
    return () => ro.disconnect();
  }, [collapse]);
  // a new list, width or fold: draw everything, then (before paint) count what fits in two rows
  useLayoutEffect(() => { setLimit(null); }, [names, width, collapsed]);
  useLayoutEffect(() => {
    const el = wrap.current;
    if (limit !== null || !collapsed || !el) return;
    const chips = Array.from(el.querySelectorAll<HTMLElement>(".kw-chip"));
    const tops = [...new Set(chips.map((c) => Math.round(c.getBoundingClientRect().top)))];
    if (tops.length <= ROWS) { setLimit(chips.length); return; }
    const box = el.getBoundingClientRect(), rtl = getComputedStyle(el).direction === "rtl";
    const used = (c: HTMLElement) => { const r = c.getBoundingClientRect(); return rtl ? box.right - r.left : r.right - box.left; };
    let n = chips.filter((c) => Math.round(c.getBoundingClientRect().top) <= tops[ROWS - 1]).length;
    while (n > 1 && used(chips[n - 1]) + 4 + MORE_W > box.width) n--;
    setLimit(n);
  }, [limit, collapsed, names]);

  const shown = collapsed && limit !== null ? keywords.slice(0, limit) : keywords;
  const hidden = keywords.length - shown.length;
  const tabKey = active !== null && shown.includes(active) ? active : shown[0];

  const focusKey = (k: string | null) => {
    const el = wrap.current;
    const chip = k === null ? null : Array.from(el?.querySelectorAll<HTMLElement>(".kw-chip") ?? []).find((c) => c.dataset.kw === k);
    if (chip) chip.focus();
    else el?.closest(".kw-body, .keyword-dialog-content")?.querySelector<HTMLInputElement>(".kw-add input")?.focus();
  };
  // after a removal (which the engine answers a moment later) focus goes to the neighbour, or the add box
  useEffect(() => {
    const n = next.current;
    if (!n) return;
    next.current = null;
    if (Date.now() - n.at < 3000) focusKey(n.key);
  }, [names]);

  // what a click or Enter does to a chip: keep an Auto-keywords one, add a mixed one to every photo
  const act = (k: string): boolean => {
    const c = counts?.get(k);
    if (c && c.n < c.total && onAddMixed) { onAddMixed(k); return true; }
    if (ai?.has(k) && onKeep) { onKeep(k); return true; }
    return false;
  };
  const onKey = (e: React.KeyboardEvent<HTMLElement>, k: string) => {
    if (e.target !== e.currentTarget) return;
    const i = shown.indexOf(k);
    const rtl = getComputedStyle(e.currentTarget).direction === "rtl";
    let to: number | null = null;
    if (e.key === (rtl ? "ArrowLeft" : "ArrowRight")) to = Math.min(i + 1, shown.length - 1);
    else if (e.key === (rtl ? "ArrowRight" : "ArrowLeft")) to = Math.max(i - 1, 0);
    else if (e.key === "Home") to = 0;
    else if (e.key === "End") to = shown.length - 1;
    else if (e.key === "Enter" && !disabled && (act(k))) {
      e.preventDefault();
      next.current = { key: shown[i + 1] ?? shown[i - 1] ?? null, at: Date.now() };
      return;
    }
    else if ((e.key === "Backspace" || e.key === "Delete") && !disabled) {
      e.preventDefault();
      next.current = { key: shown[i + 1] ?? shown[i - 1] ?? null, at: Date.now() };
      onRemove(k);
      return;
    } else return;
    e.preventDefault();
    focusKey(shown[to]);
  };

  return (
    <div className="kw-chips" ref={wrap} role="list" aria-label={label}>
      {keywords.length === 0 && <span className="kw-none">{t("library.keywording.none")}</span>}
      {shown.map((k) => {
        const auto = !!ai?.has(k);
        const c = counts?.get(k);
        const mixed = !!c && c.n < c.total;
        const pct = auto ? conf?.get(k) : undefined;
        const actable = !disabled && ((mixed && !!onAddMixed) || (auto && !!onKeep));
        const many = c && c.total > 1 ? c.n : 0;
        const rm = many ? t("keywording.removeFrom", { keyword: path(k), n: many }) : t("library.keywording.remove", { keyword: path(k) });
        const tip = [path(k), auto ? t("keywording.aiTip") : "", pct !== undefined ? t("keywording.sure", { pct }) : "",
                     mixed ? t("keywording.onSome", { n: c!.n, total: c!.total }) : "",
                     actable ? t(mixed ? "keywording.addAllTip" : "keywording.keepTip") : ""].filter(Boolean).join(" - ");
        const name = auto ? t(pct !== undefined ? "keywording.autoChipSure" : "keywording.autoChip", { keyword: path(k), pct: pct ?? 0 })
          : mixed ? t("keywording.onSomeName", { keyword: path(k), n: c!.n, total: c!.total }) : path(k);
        return (
          <span key={k} role="listitem" className={`kw-chip${auto ? " ai" : ""}${mixed ? " mixed" : ""}${actable ? " act" : ""}`} data-kw={k} data-conf={pct === undefined ? undefined : Math.max(0, Math.min(5, Math.floor((pct - 40) / 10)))}
                style={pct !== undefined ? { "--c": `${pct}%` } as React.CSSProperties : undefined}
                tabIndex={k === tabKey ? 0 : -1} onFocus={() => setActive(k)} onKeyDown={(e) => onKey(e, k)}
                onClick={(e) => { if (!disabled && !(e.target as HTMLElement).closest("button")) act(k); }}
                aria-label={name} title={tip}>
            {auto && <Sparkle className="kw-spark" />}
            <span className="kw-leaf">{leaf(k)}</span>
            {mixed && <bdi dir="ltr" className="kw-n">{`${c!.n}/${c!.total}`}</bdi>}
            <button type="button" tabIndex={-1} disabled={disabled} title={rm} aria-label={rm} onClick={() => onRemove(k)}>×</button>
          </span>
        );
      })}
      {collapse && (hidden > 0 || expanded) && (
        <button type="button" className="kw-more" aria-expanded={expanded}
                aria-label={expanded ? t("keywording.less") : t("keywording.more", { n: hidden })}
                title={expanded ? t("keywording.less") : t("keywording.more", { n: hidden })}
                onClick={() => setExpanded((v) => !v)}>
          {expanded
            ? <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="m6 15 6-6 6 6" /></svg>
            : <bdi dir="ltr">{`+${hidden}`}</bdi>}
        </button>
      )}
    </div>
  );
}

// the four-point sparkle that marks what Auto-keywords added (the same glyph leads Find's AI keywords)
export function Sparkle({ className }: { className?: string }) {
  return <svg className={className} viewBox="0 0 24 24" width="10" height="10" fill="currentColor" aria-hidden="true"><path d="M12 2C12.8 8 16 11.2 22 12 16 12.8 12.8 16 12 22 11.2 16 8 12.8 2 12 8 11.2 11.2 8 12 2Z" /></svg>;
}

let lastAuto: AutoInput = null;     // the last read, so a card that mounts again starts from it
// Auto-keywords' library-wide state while the card is open: read now and every 5 s while the window
// is visible. `keywords_summary` is only asked when photos wait and nothing runs, to tell stalled from idle
function useAutoStatus(on: boolean): AutoInput {
  const [s, setS] = useState(lastAuto);
  useEffect(() => {
    if (!on) return;
    let alive = true, reading = false;
    const read = async () => {
      if (reading || document.visibilityState !== "visible") return;
      reading = true;
      try {
        const a = await commands.analysisStatus();
        const row = a.rows.find((r) => r.id === "keywords");
        if (!row) return;
        const pending = row.pending ?? 0;
        const enabled = row.enabled && row.installed;
        let blocked = false;
        if (enabled && !a.paused && row.running === 0 && !row.active && pending > 0) {
          const k = await commands.keywordsSummary();
          blocked = !!k.blocked || k.health !== "ok";
        }
        const nextState: AutoInput = { enabled, paused: a.paused, busy: row.running > 0 || row.active || pending > 0, blocked };
        lastAuto = nextState;
        if (alive) setS((p) => (p && JSON.stringify(p) === JSON.stringify(nextState) ? p : nextState));
      } catch { /* keep the last reading; the next read tries again */ }
      finally { reading = false; }
    };
    void read();
    const id = setInterval(read, 5000);
    return () => { alive = false; clearInterval(id); };
  }, [on]);
  return s;
}

type Row = { path: string; keywords: string[] };
type Snap = { key: string; rows: Row[]; ai: Map<string, ReadonlyMap<string, number>>; analysed: boolean };
const NO_CONF: ReadonlyMap<string, number> = new Map();
const NO_STATES = new Map<string, ReadonlyMap<string, number>>();

export function KeywordingPanel({ keywords, known, onAdd, onRemove, open = true, onToggle, photoPath, loading = false, selected, onChanged, onSetUp }: {
  keywords: string[] | null;     // the open photo's; null while there is none
  // the open photo's file, to mark the keywords Auto-keywords added (none for a plain list)
  photoPath?: string;
  // several photos selected: their paths and keywords. The card then shows the keywords on all of them
  // as chips, and the ones on only some dimmed with "n/m"; onAdd and onRemove act on every one of them
  selected?: Row[];
  known: string[];               // every keyword ever used
  onAdd: (names: string[]) => void;
  onRemove: (name: string) => void;
  // Keep, Remove all and removing an Auto-keywords chip change photos' keywords outside onAdd and
  // onRemove: the page reads them again
  onChanged?: () => void;
  onSetUp?: () => void;          // the group menu's Auto-keywords settings
  open?: boolean;
  onToggle?: (alt: boolean) => void;
  // the next photo's keywords are not in yet: the card keeps its place with the
  // last ones, read-only, instead of vanishing
  loading?: boolean;
}) {
  const [recent, setRecent] = useState(loadRecent);
  const rows: Row[] = selected && selected.length > 1 ? selected : photoPath && keywords ? [{ path: photoPath, keywords }] : [];
  const key = rows.map((r) => r.path).join("\0"), kwKey = rows.map((r) => r.keywords.join("\u0001")).join("\0");
  const [rev, setRev] = useState(0);     // bumped by Keep and removals, which change what Auto-keywords added
  // The chips shown are the last whole read: the keywords and which of them Auto-keywords added (and how
  // sure it was), swapped in together, so a photo change moves nothing until both are in. Edits to the same
  // photos show at once; the card is read-only while it waits for the other.
  const [snap, setSnap] = useState<Snap | null>(null);
  useEffect(() => {
    if (!rows.length || loading) return;
    let alive = true;
    const done = (ai: Map<string, ReadonlyMap<string, number>>, analysed: boolean) => { if (alive) setSnap({ key, rows, ai, analysed }); };
    commands.keywordsPhotoStates(rows.map((r) => r.path)).then((l) => {
      done(new Map(l.map((s) => [s.path, new Map(s.keywords.map((k) => [k.name, Math.round((k.confidence ?? 0) * 100)]))])), l.some((s) => s.analysed));
    }, () => done(new Map(), false));
    return () => { alive = false; };
  }, [key, kwKey, loading, rev]);
  const fresh = rows.length > 0 && !loading && snap?.key === key;
  const shownRows: Row[] = rows.length ? (fresh ? rows : snap?.rows ?? rows) : [];
  const ai = snap?.ai ?? NO_STATES;
  const busy = loading || (rows.length > 0 && !fresh);
  const model = useMemo(() => {
    if (!rows.length) return { own: (keywords ?? []).map((name) => ({ name, n: 1, total: 1 })), auto: [] as KeywordItem[], conf: NO_CONF };
    const conf = new Map<string, number>();
    const sums = new Map<string, { sum: number; n: number }>();
    for (const r of shownRows) for (const [k, c] of ai.get(r.path) ?? NO_CONF) {
      if (!r.keywords.includes(k)) continue;
      const e = sums.get(k) ?? { sum: 0, n: 0 }; e.sum += c; e.n++; sums.set(k, e);
    }
    for (const [k, e] of sums) conf.set(k, Math.round(e.sum / e.n));
    const g = groupRows(shownRows.map((r) => ({ keywords: r.keywords, ai: new Set([...(ai.get(r.path) ?? NO_CONF).keys()].filter((k) => r.keywords.includes(k))) })), (k) => conf.get(k) ?? 0);
    return { ...g, conf };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snap, fresh, key, kwKey, keywords]);
  const { own, auto } = model;
  const counts = useMemo(() => new Map([...own, ...auto].map((k) => [k.name, k])), [own, auto]);
  const autoSet = useMemo(() => new Set(auto.map((k) => k.name)), [auto]);
  const multi = shownRows.length > 1;
  // names on every photo: not suggested again, and what the Recent buttons toggle
  const attached = useMemo(() => new Set(own.filter((k) => k.n === k.total).concat(auto).map((k) => k.name)), [own, auto]);
  const status = useAutoStatus(rows.length > 0 && !!keywords && (onToggle ? open : true));
  const state = rows.length ? autoGroupState(status, auto.length, snap?.analysed) : null;
  const headId = useId();
  // once a photo's group has had a body, it keeps it (taking off the last chip must not pull Recent up)
  const bodyKept = useRef<{ key?: string; on: boolean }>({ on: false });
  const showBody = auto.length > 0 || state === "analysing" || (bodyKept.current.key === key && bodyKept.current.on);
  bodyKept.current = { key, on: showBody && !!state };

  // the Auto-keywords actions run on the photos the card shows; none of them touches the user's own keywords
  const paths = shownRows.map((r) => r.path);
  const changed = () => { setRev((v) => v + 1); onChanged?.(); };
  const act = (run: Promise<unknown>) => { void run.then(changed, changed); };
  const keep = (names: string[]) => act(commands.keywordsAiKeep(paths, names));
  // the 5 s Undo line: what a removal took off goes back through keywords_ai_restore. Keep is not undoable
  // (nothing leaves the photo), and a removal while the line is up joins it
  const [undo, setUndo] = useState<UndoLine | null>(null);
  const timer = useRef<ReturnType<typeof createUndoTimer> | null>(null);
  timer.current ??= createUndoTimer(() => setUndo(null));
  useEffect(() => { timer.current?.cancel(); setUndo(null); }, [key]);
  useEffect(() => () => timer.current?.cancel(), []);
  const removeAuto = (names: string[]) => {
    const items = removalItems(shownRows, new Map([...ai].map(([p, m]) => [p, new Set(m.keys())])), names);
    act(commands.keywordsAiRemovePhotos(paths, names));
    if (items.length) { setUndo((u) => mergeUndo(u, key, items)); timer.current?.start(); }
  };
  const undoNow = () => {
    if (!undo) return;
    const items = undo.items;
    timer.current?.cancel(); setUndo(null);
    act(commands.keywordsAiRestore(items));
  };
  const line = undo && undo.key === key ? undo : null;
  const sum = line ? undoSummary(line) : null;
  const menu = useCardMenu(auto.length);

  const add = (names: string[]) => {
    const typed = [...new Set(names.map(parseKeyword).filter(Boolean))];
    if (!typed.length) return;
    onAdd(typed.filter((k) => !attached.has(k)));
    const next = [...typed, ...recent.filter((k) => !typed.includes(k))].slice(0, RECENT_MAX);
    setRecent(next);
    storeRecent(next);
  };

  if (!keywords) return null;
  const content = (
    <div className={onToggle ? "kw-body" : "keyword-dialog-content"}>
      {(own.length > 0 || auto.length === 0) && (
        <KeywordChips keywords={own.map((k) => k.name)} onRemove={onRemove} disabled={busy} label={t("library.keywording.title")}
                      counts={multi ? counts : undefined} onAddMixed={multi ? (k) => add([k]) : undefined} />
      )}
      <KeywordInput attached={attached} known={known} onAdd={add} disabled={busy} />
      {(state || line) && (
        <div className="kw-auto" role="group" aria-labelledby={headId}>
          <div className="kw-auto-head" title={t("keywording.groupTip")}>
            <Sparkle className="kw-spark" />
            <span id={headId} className="kw-auto-name">{t("keywording.auto")}</span>
            <span className="kw-auto-state" role="status" aria-live="polite" aria-atomic="true"
                  onPointerEnter={() => timer.current?.hold("hover")} onPointerLeave={() => timer.current?.release("hover")}
                  onFocus={() => timer.current?.hold("focus")} onBlur={() => timer.current?.release("focus")}>
              {line && sum ? (
                <>
                  <span className="kw-undo-text">{sum.n === 1 ? t("keywording.undoRemoved", { keyword: path(sum.first) }) : t("keywording.undoRemovedMany", { n: sum.n })}</span>
                  <button type="button" className="kw-undo" disabled={busy} title={t("keywording.undoTip")} onClick={undoNow}>{t("keywording.undo")}</button>
                </>
              ) : state && (
                <>
                  {state === "analysing" && <span className="kw-spin" aria-hidden="true" />}
                  {state === "done" ? auto.length : t(`keywording.state.${state}`)}
                </>
              )}
            </span>
            <span className="kw-auto-menu" ref={menu.box} style={{ visibility: auto.length ? "visible" : "hidden" }}>
              <button type="button" className={`kw-auto-more${menu.at ? " on" : ""}`} aria-haspopup="menu" aria-expanded={!!menu.at} disabled={busy || !auto.length}
                      tabIndex={auto.length ? 0 : -1} title={t("keywording.menu")} aria-label={t("keywording.menu")} onClick={(e) => { menu.toggle(e); }}>
                <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8" /><circle cx="12" cy="12" r="1.8" /><circle cx="19" cy="12" r="1.8" /></svg>
              </button>
              {menu.at && createPortal(
                <div ref={menu.menu} className="context-menu module-menu" role="menu" onKeyDown={menuKeys} style={{ position: "fixed", left: menu.at.x, top: menu.at.y }}>
                  <button type="button" role="menuitem" title={t("keywording.keepAllTip")} onClick={(e) => { e.stopPropagation(); menu.close(); keep([]); }}>
                    <span className="name">{t("keywording.keepAll")}</span></button>
                  <button type="button" role="menuitem" title={t("keywording.removeAllTip")} onClick={(e) => { e.stopPropagation(); menu.close(); removeAuto([]); }}>
                    <span className="name">{multi ? t("keywording.removeAllMany", { n: paths.length }) : t("keywording.removeAll")}</span></button>
                  {onSetUp && (
                    <button type="button" role="menuitem" onClick={(e) => { e.stopPropagation(); menu.close(); onSetUp(); }}>
                      <span className="name">{t("keywording.settings")}</span></button>)}
                </div>, document.body)}
            </span>
          </div>
          {showBody && (
            <div className="kw-auto-body">
              {auto.length > 0 && (
                <KeywordChips keywords={auto.map((k) => k.name)} ai={autoSet} conf={model.conf} counts={multi ? counts : undefined}
                              onKeep={(k) => keep([k])} onRemove={(k) => removeAuto([k])} disabled={busy} collapse />
              )}
            </div>
          )}
        </div>
      )}
      {recent.length > 0 && (
        <>
          <div className="kw-label">{t("library.keywording.recent")}</div>
          <div className="kw-recent">
            {recent.map((k) => (
              <button key={k} disabled={busy} className={attached.has(k) ? "on" : ""} title={path(k)}
                      onClick={() => (attached.has(k) ? onRemove(k) : add([k]))}>{leaf(k)}</button>
            ))}
          </div>
        </>
      )}
    </div>
  );
  if (!onToggle) return content;
  return (
    <div className="panel keywording">
      <div className={`panel-title${open ? " open" : ""}`} onClick={(e) => onToggle(e.altKey)}>
        <span>{t("library.keywording.title")}</span>
        <svg className={`chevron${open ? " open" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor"
             strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="m6 15 6-6 6 6" /></svg>
      </div>
      {open && content}
    </div>
  );
}

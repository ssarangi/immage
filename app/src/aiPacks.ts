// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The model packs of Preferences > General > AI models, as the page shows them:
// the plain-language state of a pack (its pill, the one line in its state slot,
// its buttons and whether its analysis switch can be turned on), worked out from
// the backend's PackInfo with no side effects, so a Node test and the harness
// check every state. The hook at the end keeps the list current from
// `pack_list` and the `pack-state` / `pack-progress` events.
import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "./commands";
import type { KeywordsSummary, PackInfo, PackProgress } from "./bindings";
import { formatDate, formatNumber } from "./i18n";

export type Tr = (key: string, vars?: Record<string, string | number>) => string;
export type Fmt = { num: (n: number, o?: Intl.NumberFormatOptions) => string; date: (ms: number) => string };
export const plainFmt: Fmt = {
  num: (n, o) => new Intl.NumberFormat("en", o).format(n),
  date: (ms) => new Intl.DateTimeFormat("en", { day: "numeric", month: "short" }).format(ms),
};

/** the user's number and date format; the tests use plainFmt */
export const uiFmt: Fmt = { num: formatNumber, date: (ms) => formatDate(ms, { day: "numeric", month: "short" }) };

export type ActionId = "download" | "cancel" | "check" | "remove" | "repair" | "resume" | "removePartial";
export type PillKind = "off" | "prog" | "ok" | "warn";
export type PillIcon = "dashed" | "spinner" | "check" | "alert";
export type PackView = {
  pill: { kind: PillKind; icon: PillIcon; text: string };
  message: string;
  /** the right end of the slot line (what comes after the download) */
  right: string;
  tone: "plain" | "warn" | "bad";
  /** the slot's bar, 0 to 1; null keeps its space and hides it */
  bar: number | null;
  actions: { id: ActionId; label: string; primary?: boolean }[];
  /** the pack can run its analyses: installed, whole, not being changed */
  usable: boolean;
  /** why the analysis switch is disabled, shown as its hint */
  reason: string | null;
};

/** 608 MB below a gigabyte, 1.7 GB above, in the user's number format */
export function sizeLabel(bytes: number, fmt: Fmt = plainFmt, unit?: "MB" | "GB"): string {
  const gb = unit ? unit === "GB" : bytes >= 1e9;
  return gb ? `${fmt.num(bytes / 1e9, { maximumFractionDigits: 1, minimumFractionDigits: 1 })} GB`
            : `${fmt.num(Math.ceil(bytes / 1e6))} MB`;
}
export const speedLabel = (bytesPerSec: number, fmt: Fmt = plainFmt) =>
  `${fmt.num(bytesPerSec / 1e6, { maximumFractionDigits: 1, minimumFractionDigits: 1 })} MB`;
export function etaLabel(seconds: number, tr: Tr): string {
  if (seconds < 90) return tr("aimodels.eta.seconds", { n: Math.max(5, Math.round(seconds / 5) * 5) });
  if (seconds < 90 * 60) return tr("aimodels.eta.minutes", { n: Math.round(seconds / 60) });
  return tr("aimodels.eta.hours", { n: Math.round(seconds / 3600) });
}
export const percent = (p: Pick<PackProgress, "done" | "total">) =>
  p.total > 0 ? Math.min(100, Math.floor((p.done / p.total) * 100)) : 0;

export function packView(info: PackInfo, tr: Tr, fmt: Fmt = plainFmt, justChecked = false): PackView {
  const size = sizeLabel(info.bytes, fmt);
  const none: PackView = {
    pill: { kind: "off", icon: "dashed", text: tr("aimodels.st.notInstalled") },
    message: "", right: "", tone: "plain", bar: null, actions: [], usable: false,
    reason: tr("aimodels.switch.needModel"),
  };
  switch (info.state) {
    case "notInstalled": {
      const full = info.error?.kind === "diskFull";
      return { ...none,
        message: full ? tr("aimodels.msg.diskFull", { size, free: sizeLabel(info.free_bytes, fmt) })
                      : tr("aimodels.msg.notInstalled", { size }),
        tone: full ? "bad" : "plain",
        actions: [{ id: "download", label: tr("aimodels.act.download", { size }), primary: true }] };
    }
    case "downloading": {
      const p = info.progress;
      const verifying = p?.phase === "verify";
      const pct = p ? percent(p) : 0;
      const rate = p?.bytes_per_sec ?? 0;
      const parts = p && !verifying
        ? [tr("aimodels.msg.progress", { done: sizeLabel(p.done, fmt, info.bytes >= 1e9 ? "GB" : "MB"), total: size }),
           rate > 0 ? tr("aimodels.msg.speed", { speed: speedLabel(rate, fmt) }) : tr("aimodels.msg.starting"),
           p.eta_s != null ? tr("aimodels.msg.eta", { eta: etaLabel(p.eta_s, tr) }) : ""].filter(Boolean)
        : [];
      return { ...none,
        pill: { kind: "prog", icon: "spinner", text: verifying ? tr("aimodels.st.checkingFiles") : tr("aimodels.st.downloading", { pct }) },
        message: verifying ? tr("aimodels.msg.verifying") : parts.join(" · "),
        right: verifying ? "" : tr("aimodels.msg.after"),
        bar: p ? p.done / Math.max(1, p.total) : 0,
        actions: [{ id: "cancel", label: tr("aimodels.act.cancel") }],
        reason: tr("aimodels.switch.needModel") };
    }
    case "installed": {
      const when = info.verified_at != null ? fmt.date(info.verified_at * 1000) : null;
      const message = justChecked ? tr("aimodels.check.ok", { n: info.files })
        : info.checking || when == null ? tr("aimodels.msg.checkingFirst")
        : tr("aimodels.msg.installed", { size: sizeLabel(info.bytes_on_disk, fmt), when });
      return { ...none,
        pill: { kind: "ok", icon: "check", text: info.checking || when == null ? tr("aimodels.st.installedChecking") : tr("aimodels.st.installed") },
        message, tone: "plain", usable: true, reason: null,
        actions: [{ id: "check", label: tr("aimodels.act.check") }, { id: "remove", label: tr("aimodels.act.remove") }] };
    }
    case "needsRepair":
      return { ...none,
        pill: { kind: "warn", icon: "alert", text: tr("aimodels.st.repair") },
        message: tr("aimodels.msg.repair"), tone: "warn", reason: tr("aimodels.switch.repair"),
        actions: [{ id: "remove", label: tr("aimodels.act.remove") }, { id: "repair", label: tr("aimodels.act.repair", { size }), primary: true }] };
    case "updateAvailable":
      return { ...none,
        pill: { kind: "warn", icon: "alert", text: tr("aimodels.st.update") },
        message: tr("aimodels.msg.update"), tone: "warn", usable: true, reason: null,
        actions: [{ id: "remove", label: tr("aimodels.act.remove") },
                  { id: "download", label: tr("aimodels.act.update", { size: sizeLabel(info.update_bytes ?? info.bytes, fmt) }), primary: true }] };
    case "unsupported":
      return { ...none,
        pill: { kind: "off", icon: "alert", text: tr("aimodels.st.unsupported") },
        message: info.unsupported_reason ?? tr("aimodels.msg.unsupported"), tone: "warn",
        reason: tr("aimodels.switch.unsupported"),
        actions: [{ id: "remove", label: tr("aimodels.act.remove") }] };
    case "stopped": {
      const have = sizeLabel(info.bytes - info.bytes_left, fmt, info.bytes >= 1e9 ? "GB" : "MB");
      const offline = info.error?.kind === "offline";
      return { ...none,
        pill: { kind: "warn", icon: "alert", text: tr("aimodels.st.stopped") },
        message: tr(offline ? "aimodels.msg.offline" : "aimodels.msg.stopped", { have, total: size }),
        tone: offline ? "warn" : "plain",
        actions: [{ id: "removePartial", label: tr("aimodels.act.removePartial") },
                  { id: "resume", label: tr("aimodels.act.resume", { size: sizeLabel(info.bytes_left, fmt) }), primary: true }] };
    }
  }
}

/** Preferences > General's Background work line, from the analysis status and the cache counts */
export type WorkInput = { paused: boolean; enabled: boolean; installed: boolean; pending: number | null; running: number; active: boolean; analysed: number | null };
export function workView(w: WorkInput | null, tr: Tr, fmt: Fmt = plainFmt): { line: string; bar: number | null; paused: boolean } {
  if (!w) return { line: tr("aimodels.work.reading"), bar: null, paused: false };
  const pending = w.pending;
  const total = pending != null && w.analysed != null ? w.analysed + pending : null;
  const bar = total && total > 0 && w.analysed != null ? w.analysed / total : null;
  if (w.paused && w.enabled && w.installed)
    return { line: pending ? tr("aimodels.work.paused", { n: pending, count: fmt.num(pending) }) : tr("aimodels.work.pausedEmpty"), bar, paused: true };
  if (!w.enabled || !w.installed || pending === 0) return { line: tr("aimodels.work.nothing"), bar: pending === 0 ? 1 : null, paused: w.paused };
  if (pending == null) return { line: tr("aimodels.work.counting"), bar: null, paused: w.paused };
  const working = w.running > 0 || w.active;
  return {
    line: working && total != null && w.analysed != null
      ? tr("aimodels.work.analysing", { done: fmt.num(w.analysed), total: fmt.num(total) })
      : tr("aimodels.work.waiting", { n: pending, count: fmt.num(pending) }),
    bar, paused: w.paused,
  };
}

/** the standing line under the Smart search switch */
export function analysedLine(analysed: number | null, pending: number | null, enabled: boolean, tr: Tr, fmt: Fmt = plainFmt): string {
  if (!enabled) return tr("aimodels.clip.off");
  if (analysed == null) return tr("aimodels.work.reading");
  if (pending == null || pending === 0) return tr("aimodels.clip.doneNothing", { n: analysed, count: fmt.num(analysed) });
  return tr("aimodels.clip.doneWaiting", { n: analysed, count: fmt.num(analysed), left: fmt.num(pending) });
}

/** the standing line under the Auto-keywords switch */
export function keywordsLine(analysed: number | null, pending: number | null, enabled: boolean, tr: Tr, fmt: Fmt = plainFmt, blocked: string | null = null): string {
  if (!enabled) return tr("aimodels.kw.off");
  // photos wait, untouched, until the model fits in memory
  if (blocked === "level") return tr("aimodels.kw.blockedLevel");
  if (blocked === "free") return tr("aimodels.kw.blockedFree");
  if (analysed == null) return tr("aimodels.work.reading");
  if (pending == null || pending === 0) return tr("aimodels.clip.doneNothing", { n: analysed, count: fmt.num(analysed) });
  return tr("aimodels.clip.doneWaiting", { n: analysed, count: fmt.num(analysed), left: fmt.num(pending) });
}

/** "About 1.4 s per photo ... 2.9 GB", or the line shown before the first photo is measured */
export function speedLine(s: KeywordsSummary | null, tr: Tr, fmt: Fmt = plainFmt): string {
  if (!s || s.infer_ms == null) return tr("aimodels.kw.speedNone");
  const secs = fmt.num(s.infer_ms / 1000, { maximumFractionDigits: 1, minimumFractionDigits: 1 });
  return s.memory_mb != null
    ? tr("aimodels.kw.speed", { s: secs, mem: `${fmt.num(Math.round(s.memory_mb / 100) / 10, { maximumFractionDigits: 1, minimumFractionDigits: 1 })} GB` })
    : tr("aimodels.kw.speedOnly", { s: secs });
}
/** the Results row's hint */
export function resultsLine(s: KeywordsSummary | null, tr: Tr, fmt: Fmt = plainFmt): string {
  if (!s) return tr("aimodels.work.reading");
  if (s.keywords === 0) return tr("aimodels.kw.resultsNone");
  return tr("aimodels.kw.results", { n: s.keyworded, count: fmt.num(s.keyworded), keywords: fmt.num(s.keywords) });
}

/** the Burst sharing row's hint: what it does, and how many photos it has already saved an analysis for */
export function burstLine(s: KeywordsSummary | null, tr: Tr, fmt: Fmt = plainFmt): string {
  if (!s) return tr("aimodels.work.reading");
  if (!s.burst) return tr("aimodels.kw.burstOff");
  return s.copied > 0 ? tr("aimodels.kw.burstCopied", { n: s.copied, count: fmt.num(s.copied) }) : tr("aimodels.kw.burstHint");
}

export function summaryOf(packs: PackInfo[]) {
  return {
    installed: packs.filter((p) => p.state === "installed" || p.state === "updateAvailable").length,
    total: packs.length,
    bytes: packs.reduce((n, p) => n + p.bytes_on_disk, 0),
  };
}

const IDLE_MS = 4000;
const BUSY_MS = 1000;

/** The pack list, kept current: an immediate read, events from the backend, and a
 *  poll (1 s while something downloads or is checked, 4 s otherwise) that also
 *  catches a change made elsewhere. Reads one at a time. */
export function usePacks() {
  const [packs, setPacks] = useState<PackInfo[] | null>(null);
  const [error, setError] = useState("");
  const busy = useRef(false);
  const alive = useRef(true);
  const reading = useRef(false);
  const refresh = useCallback(async () => {
    if (reading.current) return;
    reading.current = true;
    try { const next = await commands.packList(); if (alive.current) { setPacks(next); setError(""); } }
    catch (e) { if (alive.current) setError(String(e)); }
    finally { reading.current = false; }
  }, []);
  useEffect(() => {
    alive.current = true;
    void refresh();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = () => {
      if (document.visibilityState === "visible") void refresh();
      timer = setTimeout(tick, busy.current ? BUSY_MS : IDLE_MS);
    };
    timer = setTimeout(tick, IDLE_MS);
    const unlisten: (() => void)[] = [];
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten.push(await listen<PackInfo>("pack-state", (e) => {
          if (alive.current) setPacks((list) => list && list.map((p) => (p.id === e.payload.id ? { ...e.payload, download_consent: p.download_consent, analysis_consent: p.analysis_consent } : p)));
        }));
        unlisten.push(await listen<PackProgress>("pack-progress", (e) => {
          if (alive.current) setPacks((list) => list && list.map((p) => (p.id === e.payload.id && p.state === "downloading" ? { ...p, progress: e.payload } : p)));
        }));
      } catch { /* no event bridge (a plain browser): the poll keeps it current */ }
    })();
    return () => { alive.current = false; clearTimeout(timer); unlisten.forEach((u) => u()); };
  }, [refresh]);
  busy.current = !!packs?.some((p) => p.state === "downloading" || p.checking);
  return { packs, error, refresh, setPacks };
}

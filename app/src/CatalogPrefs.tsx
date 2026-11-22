// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Preferences > General > Catalog: what the Catalog holds, what it takes on disk, and the one
// dangerous action (Rebuild), which asks twice. The Local photo cache under Performance is a
// separate store that Find and Statistics still read; it joins this card when they move onto the
// Catalog (notes/nas-catalog-plan.md C7).
import { useState } from "react";
import { commands, type CatalogStatus, type SourceStatus } from "./bindings";
import { sizeOf } from "./catalogModel";
import { rebuildCatalog, refresh, useCatalog } from "./catalogStore";
import { PrefCard, PrefRow } from "./PrefsUI";
import { formatNumber, t } from "./i18n";

const size = (bytes: number) => { const s = sizeOf(bytes); return `${formatNumber(s.n, { maximumFractionDigits: 1 })} ${s.unit}`; };

export function CatalogPrefsView({ status, sources, confirming, busy, message, error, onAskRebuild, onCancel, onRebuild, onTidy }: {
  status: CatalogStatus | null; sources: SourceStatus[];
  confirming: boolean; busy: boolean; message: string; error: string;
  onAskRebuild(): void; onCancel(): void; onRebuild(): void; onTidy(): void;
}) {
  const photos = status?.photos ?? 0;
  const online = sources.filter((s) => s.availability.state === "Online").length;
  return (
    <PrefCard title={t("catalog.prefs.title")} card="catalog">
      <PrefRow id="Catalog sources" label={t("catalog.prefs.sources")}
               summary={sources.length ? t("catalog.prefs.sourcesSummary", { n: sources.length, photos: formatNumber(photos), online }) : t("catalog.prefs.sourcesNone")}
               hint={t("catalog.prefs.sourcesHint")} children={null} />
      <PrefRow id="Catalog disk" label={t("catalog.prefs.disk")}
               summary={status ? t("catalog.prefs.diskSummary", { catalog: size(status.bytes), thumbs: size(status.thumbnails.bytes) }) : " "}
               hint={t("catalog.prefs.diskHint")}>
        <button className="btn small" disabled={busy || !status} onClick={onTidy}>{t("catalog.prefs.tidy")}</button>
      </PrefRow>
      <PrefRow id="Rebuild catalog" label={t("catalog.prefs.rebuild")} summary={t("catalog.prefs.rebuildSummary")} hint={t("catalog.prefs.rebuildHint")}>
        {confirming ? (
          <span className="cat-confirm" role="group" aria-label={t("catalog.prefs.rebuildSure", { n: formatNumber(photos) })}>
            <span>{t("catalog.prefs.rebuildSure", { n: formatNumber(photos) })}</span>
            <button className="btn small" autoFocus onClick={onCancel}>{t("catalog.prefs.rebuildNo")}</button>
            <button className="btn small danger" disabled={busy} onClick={onRebuild}>{t("catalog.prefs.rebuildConfirm")}</button>
          </span>
        ) : (
          <button className="btn small danger" disabled={busy || sources.length === 0} onClick={onAskRebuild}>{t("catalog.prefs.rebuildGo")}</button>
        )}
      </PrefRow>
      <p className="cat-reassure" style={{ margin: "4px 0 8px", textAlign: "left" }}>{t("catalog.prefs.cache")}</p>
      {message && <p role="status" className="pref-hint" style={{ margin: "0 16px 8px" }}>{message}</p>}
      {error && <p role="alert" className="prefs-error">{error}</p>}
    </PrefCard>
  );
}

export function CatalogPrefs() {
  const cat = useCatalog();
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const run = async (f: () => Promise<void>) => {
    setBusy(true); setError(""); setMessage("");
    try { await f(); } catch (e) { setError(String(e instanceof Error ? e.message : e)); } finally { setBusy(false); }
  };
  return <CatalogPrefsView status={cat.status} sources={cat.sources} confirming={confirming} busy={busy} message={message} error={error}
    onAskRebuild={() => setConfirming(true)} onCancel={() => setConfirming(false)}
    onRebuild={() => void run(async () => { const n = await rebuildCatalog(); setConfirming(false); setMessage(t("catalog.prefs.rebuildStarted", { n: formatNumber(n) })); })}
    onTidy={() => void run(async () => { await commands.thumbnailsGc(); await refresh(); setMessage(t("catalog.prefs.tidyDone")); })} />;
}

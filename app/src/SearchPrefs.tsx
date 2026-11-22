// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Preferences > Performance > Library cache. The CLIP model card and the Background work
// pause moved to Preferences > General > AI models (AiModelsPrefs.tsx).
import { useEffect, useState } from "react";
import { commands, type SearchStatus } from "./commands";
import { PrefCard, PrefRow } from "./PrefsUI";
import { t } from "./i18n";

function useSearchStatus() {
  const [status, setStatus] = useState<SearchStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    let polling = false;
    const poll = async () => {
      if (polling) return;
      polling = true;
      try { const next = await commands.searchStatus(); if (alive) setStatus(next); }
      catch (e) { if (alive) setError(String(e)); }
      finally { polling = false; }
    };
    void poll();
    const timer = setInterval(() => void poll(), 2000);
    return () => { alive = false; clearInterval(timer); };
  }, []);
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true); setError("");
    try { await action(); setStatus(await commands.searchStatus()); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };
  return { status, busy, error, setError, run };
}

export function SearchPrefs() {
  const { status, busy, error, run } = useSearchStatus();
  const count = status?.counts;
  return <PrefCard title={t("library.cache.title")}>
    <PrefRow label={t("library.cache.metadata")} summary={count ? t("library.cache.count", { photos: count.assets.toLocaleString(), versions: count.variants.toLocaleString(), records: count.metadata.toLocaleString(), analyses: count.analyses.toLocaleString() }) : status ? t("library.cache.unavailable") : t("library.cache.reading")}
      hint={t("library.cache.hint")}>
      <button className="btn small" disabled={busy || !count} onClick={() => void run(() => commands.searchCacheClear(false))}>{t("library.cache.clear")}</button>
      <button className="btn small" disabled={busy || !count} onClick={() => void run(() => commands.searchCacheClear(true))}>{t("library.cache.rebuild")}</button>
    </PrefRow>
    <PrefRow label={t("library.cache.analysis")} summary={count ? t("library.cache.analysedPhotos", { photos: count.analysed_photos.toLocaleString(), analyses: count.analyses.toLocaleString() }) : status ? t("library.cache.unavailable") : t("library.cache.reading")} children={null} />
    {!!status?.roots.length && <div className="cache-directories" role="region" aria-label={t("library.cache.directories")} tabIndex={0}>
      {status.roots.map(root => {
        const connected = root.state === "online";
        const label = root.state === "online" || root.state === "offline"
          ? t(connected ? "library.folders.connected" : "library.folders.offline")
          : t("library.cache.lastCheck", { state: root.state, error: "" });
        return <div className="cache-directory" key={root.path}>
          <span className="cache-directory-path" title={root.path}>{root.path}</span>
          <span className="cache-directory-status" role="img" aria-label={label}
            title={`${label}${root.error ? ` — ${root.error}` : ""}`}>
            <span className={`drive-dot${connected ? "" : " off"}`} />
          </span>
        </div>;
      })}
    </div>}
    {error && <p role="alert" className="prefs-error">{error}</p>}
  </PrefCard>;
}

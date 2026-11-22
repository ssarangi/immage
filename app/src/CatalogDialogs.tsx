// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Catalog's two small dialogs: adding a folder (what it is, and what Immage will and will not do
// to it, said before anything is read) and removing a source (what leaves the Catalog; never a file).
import { useEffect, useState } from "react";
import { commands, type RemovePreview, type SourceDetect, type SourceStatus } from "./bindings";
import { folderCatalog, sourceRoot } from "./catalogModel";
import { baseName } from "./paths";
import { formatNumber, t } from "./i18n";

type Detect = (path: string) => Promise<SourceDetect>;

export function AddToCatalogDialog({ path, sources, onAdd, onClose, detect = commands.sourceDetect }: {
  path: string; sources: SourceStatus[]; onAdd(path: string): Promise<unknown>; onClose(): void; detect?: Detect;
}) {
  const [found, setFound] = useState<SourceDetect | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const here = folderCatalog(path, sources);
  useEffect(() => {
    let alive = true;
    detect(path).then((d) => { if (alive) setFound(d); }, (e) => { if (alive) setProblem(String(e instanceof Error ? e.message : e)); });
    return () => { alive = false; };
  }, [path, detect]);
  const inside = here.kind === "none" ? null : here.source.source.name;
  const unreadable = found !== null && !found.readable && !found.asleep;
  const block = inside ? t("catalog.add.already", { name: inside }) : unreadable ? t("catalog.add.unreadable")
    : found?.overlaps != null ? t("catalog.add.overlap") : problem;
  const go = async () => {
    setBusy(true);
    try { await onAdd(path); onClose(); } catch (e) { setProblem(String(e instanceof Error ? e.message : e)); setBusy(false); }
  };
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose(); }}
         onKeyDown={(e) => { if (e.key === "Escape" && !busy) { e.stopPropagation(); onClose(); } }}>
      <div className="modal cat-dialog" role="dialog" aria-modal="true" aria-label={t("catalog.add.title")}>
        <div className="modal-head"><h2>{t("catalog.add.title")}</h2></div>
        <div className="modal-body">
          <div className="cat-kv">
            <span className="k">{t("catalog.add.folder")}</span><span className="v mono" title={path}>{path}</span>
          </div>
          <h3>{t("catalog.add.found")}</h3>
          <div className="cat-found" aria-busy={found === null && !problem}>
            {(["storage", "host", "share"] as const).map((row) => {
              const v = found === null ? null : row === "storage" ? found.storage : row === "host" ? found.host : found.share ? t("catalog.add.mounted", { share: found.share, at: found.mount_path }) : found.mount_path;
              return (
                <div className="cat-kv" key={row}>
                  <span className="k">{t(`catalog.add.${row}`)}</span>
                  <span className={`v${found === null ? " skeleton" : ""}`}>{found === null ? " " : v ?? "-"}</span>
                </div>
              );
            })}
          </div>
          <p className="cat-note">{found?.asleep ? t("catalog.add.asleep") : " "}</p>
          <p className="cat-warn" role={block ? "alert" : undefined}>{block ?? " "}</p>
          <p className="cat-reassure"><span aria-hidden="true">{"\u{1F512}"} </span>{t("catalog.readOnly")}</p>
        </div>
        <div className="modal-foot">
          <button className="btn" disabled={busy} onClick={onClose}>{t("app.action.cancel")}</button>
          <button className="btn primary" disabled={busy || found === null || !!block} onClick={() => void go()}>{t("catalog.add.go")}</button>
        </div>
      </div>
    </div>
  );
}

export function RemoveSourceDialog({ source, onRemove, onClose, preview = (id) => commands.sourceRemove(id, true) }: {
  source: SourceStatus; onRemove(id: number): Promise<unknown>; onClose(): void; preview?: (id: number) => Promise<RemovePreview>;
}) {
  const [counts, setCounts] = useState<RemovePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = source.source.id;
  useEffect(() => {
    let alive = true;
    preview(id).then((c) => { if (alive) setCounts(c); }, () => {});
    return () => { alive = false; };
  }, [id, preview]);
  const go = async () => {
    setBusy(true);
    try { await onRemove(id); onClose(); } catch (e) { setError(String(e instanceof Error ? e.message : e)); setBusy(false); }
  };
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose(); }}
         onKeyDown={(e) => { if (e.key === "Escape" && !busy) { e.stopPropagation(); onClose(); } }}>
      <div className="modal cat-dialog" role="dialog" aria-modal="true" aria-label={t("catalog.remove.title", { name: source.source.name })}>
        <div className="modal-head"><h2>{t("catalog.remove.title", { name: source.source.name })}</h2></div>
        <div className="modal-body">
          <p>{counts === null ? " " : t("catalog.remove.body", { n: counts.photos, count: formatNumber(counts.photos) })}</p>
          <p className="cat-note">{t("catalog.remove.safe", { folder: baseName(sourceRoot(source)) })}</p>
          <p className="cat-warn" role={error ? "alert" : undefined}>{error ?? " "}</p>
        </div>
        <div className="modal-foot">
          <button className="btn" autoFocus disabled={busy} onClick={onClose}>{t("app.action.cancel")}</button>
          <button className="btn danger" disabled={busy || counts === null} onClick={() => void go()}>{t("catalog.remove.go")}</button>
        </div>
      </div>
    </div>
  );
}

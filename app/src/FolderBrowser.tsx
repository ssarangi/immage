// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Immage's own folder chooser, in the app's theme: Places on the left (home, drives and shares, the folders
// already added), a breadcrumb bar that turns into a path field, and the subfolders of the folder shown.
// It only reads directory names, through commands that time out, so a dead share shows an error row with
// Retry and never freezes the window. browseFolder() is the one call for callers: it resolves with the
// chosen folder, or null when cancelled.
import { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { commands, type BrowserDir, type BrowserListing, type BrowserPlace } from "./bindings";
import { crumbs, moveIndex, parentOf, visibleDirs } from "./folderBrowserModel";
import { t } from "./i18n";

export type FolderBrowserProps = {
  title: string;
  /** where to open; home when empty */
  start?: string;
  /** the folders already added, listed under Places */
  added?: string[];
  onChoose(path: string): void;
  onCancel(): void;
  /** the commands, replaceable for the harness */
  list?: (path: string) => Promise<BrowserListing>;
  places?: (added: string[]) => Promise<BrowserPlace[]>;
};

const Folder = () => (
  <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" className="fb-ico">
    <path d="M1.5 4.5a1 1 0 0 1 1-1h3.2l1.3 1.5h6.5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" fill="none" stroke="currentColor" strokeWidth="1.2" />
  </svg>
);
const Drive = ({ network }: { network: boolean }) => (
  <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" className="fb-ico">
    {network
      ? <path d="M2 3.5h12v6H2zM5 12.5h6M8 9.5v3" fill="none" stroke="currentColor" strokeWidth="1.2" />
      : <path d="M2 9.5l1.6-5h8.8l1.6 5v3H2zM2 9.5h12M11.5 11.5h.01" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />}
  </svg>
);

const placeIcon = (p: BrowserPlace) => p.kind === "drive" || p.kind === "root" ? <Drive network={p.network} /> : <Folder />;
const placeLabel = (p: BrowserPlace) => {
  switch (p.kind) {
    case "home": return t("folderBrowser.place.home");
    case "pictures": return t("folderBrowser.place.pictures");
    case "desktop": return t("folderBrowser.place.desktop");
    case "documents": return t("folderBrowser.place.documents");
    case "root": return t("folderBrowser.place.root");
    default: return p.label;
  }
};

export function FolderBrowser({ title, start, added = [], onChoose, onCancel, list = commands.listSubdirectories, places = commands.folderPlaces }: FolderBrowserProps) {
  const [places_, setPlaces] = useState<BrowserPlace[] | null>(null);
  const [path, setPath] = useState<string>(start || "~");
  const [shown, setShown] = useState<BrowserListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [showHidden, setShowHidden] = useState(false);
  const [active, setActive] = useState(-1);
  const [editing, setEditing] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const dialog = useRef<HTMLDivElement>(null);
  const rows = useRef<HTMLDivElement>(null);
  const token = useRef(0);

  useEffect(() => {
    let alive = true;
    places(added).then((p) => { if (alive) setPlaces(p); }, () => { if (alive) setPlaces([]); });
    return () => { alive = false; };
    // the list is read once per opening
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const mine = ++token.current;
    setLoading(true);
    setActive(-1);
    list(path).then((r) => r, () => ({ path, parent: null, dirs: [], error: "other" } as BrowserListing)).then((r) => {
      if (mine !== token.current) return;   // a newer folder was asked for meanwhile
      setShown(r);
      setLoading(false);
    });
  }, [path, attempt, list]);

  // the folder the reply names, with "~" resolved, once there is one
  const here = shown && !loading ? shown.path : null;
  const dirs = shown && !loading && !shown.error ? visibleDirs(shown.dirs, showHidden) : [];
  const up = here ? parentOf(here) : null;
  const go = useCallback((p: string) => { setEditing(null); setPath(p); }, []);

  useEffect(() => {
    rows.current?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);
  useEffect(() => { dialog.current?.focus(); }, []);
  // closing the path field removes the focused element: take the focus back so the keys keep working
  useEffect(() => { if (editing === null && !dialog.current?.contains(document.activeElement)) dialog.current?.focus(); }, [editing]);

  const inField = (e: React.KeyboardEvent) => (e.target as HTMLElement).tagName === "INPUT";
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") { e.stopPropagation(); if (editing !== null) setEditing(null); else onCancel(); return; }
    if (e.key === "Tab") {   // keep focus inside the dialog
      const items = [...dialog.current!.querySelectorAll<HTMLElement>("button:not(:disabled), input, [tabindex='0']")];
      if (items.length === 0) return;
      const i = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : (i === items.length - 1 ? 0 : i + 1);
      e.preventDefault();
      items[next].focus();
      return;
    }
    if (inField(e)) return;
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && here) { e.preventDefault(); onChoose(here); return; }
    if ((e.target as HTMLElement).tagName === "BUTTON" && (e.key === "Enter" || e.key === " ")) return;   // the button's own click
    if (e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      setActive((a) => moveIndex(a, e.key as "ArrowUp", dirs.length));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (active >= 0 && dirs[active]) go(dirs[active].path);
    } else if (e.key === "Backspace" || (e.key === "ArrowLeft" && e.altKey)) {
      e.preventDefault();
      if (up) go(up);
    } else if ((e.key === "l" && (e.ctrlKey || e.metaKey)) || e.key === "/") {
      e.preventDefault();
      setEditing(here ?? path);
    }
  };

  const bar = here ?? path;
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onCancel(); }}>
      <div className="modal fb-dialog" role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} ref={dialog} onKeyDown={onKeyDown}>
        <div className="modal-head"><h2>{title}</h2></div>
        <div className="fb-main">
          <nav className="fb-places" aria-label={t("folderBrowser.places")}>
            <div className="fb-places-head">{t("folderBrowser.places")}</div>
            {places_ === null && <div className="fb-note">{t("folderBrowser.loading")}</div>}
            {(places_ ?? []).map((p, i) => {
              const added = p.kind === "added";
              const first = added && places_![i - 1]?.kind !== "added";
              return (
                <div key={`${p.kind}:${p.path}`}>
                  {first && <div className="fb-places-head">{t("folderBrowser.added")}</div>}
                  <button className={`fb-place${p.path === here ? " on" : ""}${p.online ? "" : " off"}`} title={p.path} onClick={() => go(p.path)}>
                    {placeIcon(p)}
                    <span className="fb-place-name">{placeLabel(p)}</span>
                    {!p.online && <span className="fb-badge">{t("folderBrowser.offline")}</span>}
                  </button>
                </div>
              );
            })}
          </nav>
          <div className="fb-pane">
            <div className="fb-bar">
              <button className="fb-up" disabled={!up} onClick={() => up && go(up)} aria-label={t("folderBrowser.up")} title={t("folderBrowser.up")}>
                <span aria-hidden="true">{"↑"}</span>
              </button>
              {editing === null ? (
                <div className="fb-crumbs" onClick={(e) => { if (e.target === e.currentTarget) setEditing(bar); }}>
                  {crumbs(bar).map((c, i, all) => (
                    <span key={c.path} className="fb-crumb-wrap">
                      {i > 0 && <span className="fb-sep" aria-hidden="true">{"›"}</span>}
                      <button className={`fb-crumb${i === all.length - 1 ? " last" : ""}`} onClick={() => go(c.path)}>{c.label}</button>
                    </span>
                  ))}
                  <button className="fb-edit" onClick={() => setEditing(bar)} aria-label={t("folderBrowser.editPath")} title={t("folderBrowser.editPath")}>
                    <span aria-hidden="true">{"✎"}</span>
                  </button>
                </div>
              ) : (
                <input className="fb-path" dir="ltr" autoFocus spellCheck={false} aria-label={t("folderBrowser.pathLabel")}
                       value={editing} onChange={(e) => setEditing(e.target.value)}
                       onFocus={(e) => e.currentTarget.select()}
                       onKeyDown={(e) => { if (e.key === "Enter" && editing.trim()) { e.preventDefault(); go(editing.trim()); } }} />
              )}
            </div>
            <div className="fb-list" ref={rows} role="listbox" aria-label={t("folderBrowser.folders")} aria-busy={loading}>
              {loading && <div className="fb-state" role="status"><span className="fb-spin" aria-hidden="true" />{t("folderBrowser.reading")}</div>}
              {!loading && shown?.error && (
                <div className="fb-state err" role="alert">
                  <span>{t(`folderBrowser.error.${shown.error}`)}</span>
                  <button className="btn" onClick={() => setAttempt((a) => a + 1)}>{t("folderBrowser.retry")}</button>
                </div>
              )}
              {!loading && !shown?.error && dirs.length === 0 && <div className="fb-state">{t("folderBrowser.empty")}</div>}
              {dirs.map((d: BrowserDir, i) => (
                <div key={d.path} data-i={i} role="option" aria-selected={i === active} className={`fb-row${i === active ? " on" : ""}${d.hidden ? " dim" : ""}`}
                     onClick={() => setActive(i)} onDoubleClick={() => go(d.path)}>
                  <Folder /><span className="fb-row-name">{d.name}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
        <div className="modal-foot fb-foot">
          <label className="fb-hidden"><input type="checkbox" checked={showHidden} onChange={(e) => setShowHidden(e.target.checked)} />{t("folderBrowser.showHidden")}</label>
          <span className="fb-spacer" />
          <button className="btn" onClick={onCancel}>{t("app.action.cancel")}</button>
          <button className="btn primary" disabled={!here || !!shown?.error} onClick={() => here && onChoose(here)}>{t("folderBrowser.select")}</button>
        </div>
      </div>
    </div>
  );
}

let lastFolder = "";

/** Opens the browser over the app; resolves with the chosen folder, or null when cancelled. */
export function browseFolder(opts: { title?: string; start?: string; added?: string[] } = {}): Promise<string | null> {
  return new Promise((resolve) => {
    const host = document.createElement("div");
    document.body.append(host);
    const opener = document.activeElement as HTMLElement | null;
    const root = createRoot(host);
    const done = (p: string | null) => {
      if (p) lastFolder = p;
      root.unmount();
      host.remove();
      opener?.focus?.();
      resolve(p);
    };
    root.render(<FolderBrowser title={opts.title ?? t("folderBrowser.title")} start={opts.start || lastFolder}
                               added={opts.added} onChoose={done} onCancel={() => done(null)} />);
  });
}

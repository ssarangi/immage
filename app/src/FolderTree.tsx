// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// the library panel's folders as a tree, as Lightroom's: each folder added
// opens into its subfolders, read from disk when first opened and again
// whenever it is reopened, so folders made since (by an import) show.
// with folders on more than one drive, each drive is a heading over its
// folders, so a 2026 on the Mac and a 2026 on a NAS are told apart.
// this is the Folders adapter: it owns the data (listSubfolders), the drag and
// drop, New Folder and the host rename, and hands FolderTreeView a flat list of
// rows; the Catalog tree (CatalogCard.tsx) draws through the same view
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands, type SubFolder, type Volume } from "./commands";
import { NameEdit } from "./MasksPanel";
import { baseName, within } from "./paths";
import { pressPhoto, usePhotoDrag } from "./photoDrag";
import { t } from "./i18n";
import { FolderTreeView, chevron } from "./FolderTreeView";
import { FolderMenu, type FolderTools } from "./FolderMenu";
import type { TreeRow } from "./folderTreeModel";

export type { FolderTools };

// the drive a folder is on (folder_volumes, volumes.rs)
export type { Volume };

// what a Folders row needs to be drawn
type Data = {
  path: string;
  offline: boolean;
  /** a drive heading */
  drive?: { mount: string; volume: Volume | undefined; open: boolean };
  /** the row where a new subfolder is being named */
  creating?: boolean;
};

export function FolderTree({ roots, volumes, hostNames, folded, current, origin = "folders", photo, reveal, expanded, hidden, refresh, icon, driveIcon, networkIcon,
                             onToggle, onFold, onOpen, onRemove, onDropFolder, transfer, onRenameHost, onError, tools }: {
  roots: string[];
  volumes: Map<string, Volume>;        // by root
  hostNames: Record<string, string>;   // the user's names for servers, by host
  folded: Set<string>;                 // drive headings closed, by mount
  current: string | null;
  origin?: "folders" | "catalog" | null; // which tree the open folder was opened from; the other one marks it quietly
  photo: string | null;                // the open photo's folder: scrolled to, and marked when not `current`
  reveal?: { path: string; n: number } | null; // "Show in Folders": scrolled to when `n` changes (App opens what is above it)
  expanded: Set<string>;
  hidden: Set<string>;                 // subfolders taken off the tree
  refresh: number;                     // bumped to read every open folder again
  icon: React.ReactNode;
  driveIcon: React.ReactNode;
  networkIcon: React.ReactNode;
  onToggle: (path: string) => void;
  onFold: (mount: string) => void;
  onOpen: (path: string) => void;
  onRemove: (path: string) => void;
  onDropFolder: (source: string, dest: string, copy: boolean) => void;
  transfer: { source: string; target: string } | null;
  onRenameHost: (host: string, name: string) => void;  // "" goes back to the host
  onError: (message: string) => void;
  tools?: FolderTools;
}) {
  const [editingHost, setEditingHost] = useState<string | null>(null);
  const [children, setChildren] = useState<Map<string, SubFolder[]>>(new Map());
  // a folder's right-click menu, and the folder a new subfolder is being
  // named in (its row, first among the subfolders, until Enter or Esc)
  const [menu, setMenu] = useState<{ x: number; y: number; path: string; offline: boolean } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  const [creating, setCreating] = useState<string | null>(null);
  const creatingRef = useRef<string | null>(null);
  const [made, setMade] = useState(0);   // bumped to read the open folders again after one is made
  const newFolder = (parent: string) => {
    if (!expanded.has(parent)) onToggle(parent);
    creatingRef.current = parent;
    setCreating(parent);
  };
  // Enter and the blur that follows it both end the edit: the first one counts
  const named = (name: string | null) => {
    const parent = creatingRef.current;
    creatingRef.current = null;
    setCreating(null);
    if (!parent || !name) return;
    commands.createFolder(parent, name).then(() => setMade((n) => n + 1), (e) => onError(`New Folder: ${e}`));
  };
  // a photo dragged over a folder (photoDrag.ts): the folder takes the drop,
  // and one that is shut opens after a moment's rest on it, so its
  // subfolders can take it instead. the photos' own folder opens too: it
  // refuses the drop, but its subfolders take it
  const drag = usePhotoDrag();
  const opens = useRef(new Map<string, boolean>());
  const toggle = useRef(onToggle);
  toggle.current = onToggle;
  const rest = drag?.rest ?? null;
  useEffect(() => {
    if (!rest || expanded.has(rest) || !opens.current.get(rest)) return;
    const t = window.setTimeout(() => toggle.current(rest), 500);
    return () => window.clearTimeout(t);
  }, [rest, expanded]);
  // the open folders' subfolders, read when a folder opens or on refresh
  useEffect(() => {
    let stale = false;
    // Publish each folder as it arrives: a slow or unavailable drive must
    // not hold back the children of another folder the user just expanded.
    for (const p of expanded) {
      void commands.listSubfolders(p).then((list) => {
        if (stale) return;
        setChildren((old) => {
          const next = new Map(old);
          next.set(p, list);
          return next;
        });
      }).catch(() => { /* keep prior children; reopening or refresh retries */ });
    }
    return () => { stale = true; };
  }, [expanded, refresh, made]);

  // the rows: drive headings (when there is more than one drive), then each
  // root and, under every open folder, its subfolders
  const rows = useMemo(() => {
    const out: TreeRow<Data>[] = [];
    const byDrive = new Map<string, string[]>();
    for (const r of roots) {
      const mount = volumes.get(r)?.mount ?? "";
      byDrive.set(mount, [...(byDrive.get(mount) ?? []), r]);
    }
    const headed = byDrive.size >= 2;
    const add = (path: string, name: string, depth: number, hasChildren: boolean, offline: boolean, parentKey: string | null) => {
      const open = expanded.has(path);
      out.push({ key: path, parentKey, depth, kind: "folder", path, name, hasChildren, open, data: { path, offline } });
      if (creating === path) {
        out.push({ key: `new:${path}`, parentKey: path, depth: depth + 1, kind: "message", path: "", name: "", hasChildren: false, open: false,
                   data: { path, offline, creating: true } });
      }
      if (!open) return;
      for (const k of children.get(path)?.filter((c) => !hidden.has(c.path)) ?? []) add(k.path, k.name, depth + 1, k.has_children, offline, path);
    };
    const root = (r: string, depth: number, parentKey: string | null) =>
      add(r, baseName(r), depth, children.get(r)?.length !== 0, volumes.get(r)?.online === false, parentKey);
    if (!headed) {
      for (const r of roots) root(r, 0, null);
    } else {
      for (const [mount, list] of byDrive) {
        const v = volumes.get(list[0]);
        const open = !folded.has(mount);
        const key = `drive:${mount}`;
        out.push({ key, parentKey: null, depth: 0, kind: "heading", path: "", name: v?.name || mount, hasChildren: false, open,
                   data: { path: "", offline: false, drive: { mount, volume: v, open } } });
        if (open) for (const r of list) root(r, 1, key);
      }
    }
    return out;
  }, [roots, volumes, folded, expanded, children, hidden, creating]);

  // the photo's folder is scrolled to once per photo folder, when its row
  // first shows: its parents' subfolders are read after it opens. "Show in
  // Folders" is scrolled to once per request
  const target = useRef({ key: "", n: 0, photo: null as string | null, reveal: 0 });
  const has = (key: string) => rows.some((r) => r.key === key);
  if (reveal && reveal.n !== target.current.reveal && has(reveal.path)) {
    target.current = { ...target.current, key: reveal.path, n: target.current.n + 1, reveal: reveal.n };
  } else if (photo && photo !== target.current.photo && has(photo)) {
    target.current = { ...target.current, key: photo, n: target.current.n + 1, photo };
  }
  const scrollTo = target.current.n ? { key: target.current.key, n: target.current.n } : null;

  const folderMenu = menu && (
    <FolderMenu {...menu} variant="folders" onClose={closeMenu} tools={tools}
                onNew={() => newFolder(menu.path)} onRemove={() => onRemove(menu.path)} />
  );
  return (<>
    <FolderTreeView<Data> rows={rows} label={t("library.folders.label")} className="folders" selectedKey={origin === "catalog" ? null : current}
      scrollTo={scrollTo}
      onOpen={(row) => {
        const busy = transfer && [transfer.source, transfer.target].some((root) => within(row.path, root));
        if (!busy) onOpen(row.path);
      }}
      onToggle={(row) => onToggle(row.path)}
      onMenu={(row, x, y) => setMenu({ x, y, path: row.path, offline: row.data.offline })}
      rowAttrs={(r) => {
        const { path, offline } = r.data;
        const holdsPhoto = path === photo && path !== current;
        const alsoCatalog = path === current && origin === "catalog";
        const busy = transfer && [transfer.source, transfer.target].some((root) => within(path, root));
        opens.current.set(path, r.hasChildren && !offline && !busy);
        // the photos' own folder, or a drive not answering, takes no drop
        const refuse = offline || busy || (drag !== null && drag.from.length === 1 && drag.from[0] === path);
        const into = drag !== null && drag.over === path;
        return {
          className: `${holdsPhoto ? "has-photo" : ""}${offline ? " offline" : ""}${into ? ` drop-into${drag.copy ? " copy" : ""}` : ""}${alsoCatalog ? " also" : ""}`,
          "data-folder-path": path, "data-folder-refuse": refuse ? "1" : undefined,
          title: busy ? t("dialogs.folderMove.busy") : offline ? t("library.folders.pathOffline", { path }) : path,
          onPointerDown: (e) => {
            if (offline || transfer || (e.target as Element).closest("button, input")) return;
            pressPhoto(e, { folder: { path, name: r.name }, count: () => 1, from: () => [path],
              drop: (dest, copy) => onDropFolder(path, dest, copy) });
          },
          onContextMenu: (e) => {
            e.preventDefault();
            e.stopPropagation();
            setMenu({ x: e.clientX, y: e.clientY, path, offline });
          },
        };
      }}
      renderItem={(r) => {
        const { path } = r.data;
        const holdsPhoto = path === photo && path !== current;
        const alsoCatalog = path === current && origin === "catalog";
        return <>
          <span className="ftree-icon">{icon}</span><span className="name">{r.name}</span>
          {holdsPhoto && <span className="photo-dot" title={t("library.folders.openPhoto")} />}
          {alsoCatalog && <span className="photo-dot" role="img" aria-label={t("catalog.tree.alsoCatalog")} title={t("catalog.tree.alsoCatalog")} />}
          <button className="remove" title={t("library.folders.removeFromList")} tabIndex={-1}
                  onClick={(e) => { e.stopPropagation(); onRemove(path); }}>×</button>
        </>;
      }}
      renderOther={(r) => {
        if (r.data.creating) {
          return <><span className="twisty none">{chevron}</span><span className="ftree-icon">{icon}</span><NameEdit shown={t("library.folders.newFolder")} onDone={named} /></>;
        }
        const { mount, volume: v, open } = r.data.drive!;
        // a share mounted by its address shows it, unless the user named it
        const host = v?.host ? hostNames[v.host] || v.host : null;
        return (
          <div className="folder-volume-row" role="button" tabIndex={-1} aria-expanded={open}
               title={v?.host ? `${mount}\n${t("fields.drive.onHost", { host })}${host !== v.host ? ` (${v.host})` : ""}` : mount}
               onClick={() => onFold(mount)}>
            <span className={`twisty${open ? " open" : ""}`}>{chevron}</span>
            {v?.network ? networkIcon : driveIcon}
            <span className="name">{v?.name || mount}</span>
            {v?.host && (editingHost === mount
              ? <NameEdit shown={host!} onDone={(to) => {
                  setEditingHost(null);
                  if (to !== null && to !== host) onRenameHost(v.host!, to === v.host ? "" : to);
                }} />
              : <span className="host" title={t("library.folders.renameHost")}
                      onClick={(e) => e.stopPropagation()}
                      onDoubleClick={(e) => { e.stopPropagation(); setEditingHost(mount); }}>{host}</span>)}
            <span className={`drive-dot${v?.online === false ? " off" : ""}`} role="img"
                  aria-label={v?.online === false ? t("library.folders.offline") : t("library.folders.connected")}
                  title={v?.online === false ? t("library.folders.offline") : t("library.folders.connected")} />
          </div>
        );
      }}
    />
    {folderMenu}
  </>);
}

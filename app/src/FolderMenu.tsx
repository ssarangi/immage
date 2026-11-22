// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// A folder's right-click menu, for a row of the Folders tree and for a folder row of the Catalog tree
// (spec section 4.6). The Folders one makes a subfolder, shows the folder in the file manager and takes
// it off the list; the Catalog one cannot (the Catalog follows the disk) and says whose part the folder
// is. Cull and Statistics read the original files, so an offline catalog folder shows them disabled.
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FILE_MANAGER } from "./ExportDialog";
import { useMenuClose } from "./FileMenu";
import { t } from "./i18n";
import { folderCatalog } from "./catalogModel";
import type { SourceStatus } from "./bindings";

// what a folder's menu can do with the Catalog and the other tools (App supplies them)
export type FolderTools = {
  sources: SourceStatus[];
  onCull(path: string): void;
  onStats(path: string): void;
  onAddToCatalog(path: string): void;
  /** brings the folder's row into view in the Catalog tree, opening what is above it (no folder is opened) */
  onShowInCatalog(source: SourceStatus, path: string): void;
  onRemoveFromCatalog(source: SourceStatus): void;
  /** whether the Folders list has a root above `path`, and brings its row into view */
  inFolders(path: string): boolean;
  onShowInFolders(path: string): void;
};

export type FolderMenuProps = {
  x: number; y: number; path: string; offline: boolean; tools?: FolderTools; onClose: () => void;
} & (
  | { variant: "folders"; onNew: () => void; onRemove: () => void }
  | { variant: "catalog"; source: SourceStatus; onOpen: () => void }
);

export function FolderMenu(props: FolderMenuProps) {
  const { x, y, path, offline, tools, onClose } = props;
  const ref = useMenuClose(onClose);
  const left = Math.min(x, window.innerWidth - 230), top = Math.min(y, window.innerHeight - (tools ? 300 : 130));
  const why = offline ? t("library.folders.notConnected") : undefined;
  const go = (f: () => void) => () => { onClose(); f(); };
  const copy = () => { void navigator.clipboard?.writeText(path).catch(() => {}); };
  if (props.variant === "catalog") {
    const name = props.source.source.name;
    return (
      <div ref={ref} className="context-menu cat-menu" style={{ left, top }} role="menu">
        <button role="menuitem" onClick={go(props.onOpen)}>{t("catalog.menu.open")}</button>
        {tools && <>
          <button role="menuitem" disabled={offline} title={why} onClick={go(() => tools.onCull(path))}>{t("catalog.folder.cull")}</button>
          <button role="menuitem" disabled={offline} title={why} onClick={go(() => tools.onStats(path))}>{t("catalog.folder.stats")}</button>
          <div className="menu-sep" />
          {tools.inFolders(path) && <button role="menuitem" onClick={go(() => tools.onShowInFolders(path))}>{t("catalog.tree.showInFolders")}</button>}
        </>}
        <button role="menuitem" disabled={offline} title={why} onClick={go(() => void revealItemInDir(path).catch(() => {}))}>{t("library.menu.showIn", { manager: FILE_MANAGER })}</button>
        <button role="menuitem" onClick={go(copy)}>{t("catalog.tree.copyPath")}</button>
        <div className="menu-sep" />
        <button role="menuitem" disabled>{t("catalog.tree.partOf", { source: name })}</button>
      </div>
    );
  }
  const here = tools ? folderCatalog(path, tools.sources) : null;
  return (
    <div ref={ref} className="context-menu" style={{ left, top }}>
      {tools && here && <>
        <button disabled={offline} title={why} onClick={go(() => tools.onCull(path))}>{t("catalog.folder.cull")}</button>
        <button onClick={go(() => tools.onStats(path))}>{t("catalog.folder.stats")}</button>
        <div className="menu-sep" />
        {here.kind === "none" && <button disabled={offline} title={why} onClick={go(() => tools.onAddToCatalog(path))}>{t("catalog.folder.add")}</button>}
        {here.kind !== "none" && <button onClick={go(() => tools.onShowInCatalog(here.source, path))}>{t("catalog.tree.showInCatalog")}</button>}
        {here.kind === "root" && <button onClick={go(() => tools.onRemoveFromCatalog(here.source))}>{t("catalog.folder.remove")}</button>}
        <div className="menu-sep" />
      </>}
      <button disabled={offline} title={why} onClick={go(props.onNew)}>{t("library.folders.newFolderMenu")}</button>
      <div className="menu-sep" />
      <button disabled={offline} title={why} onClick={go(() => void revealItemInDir(path).catch(() => {}))}>{t("library.menu.showIn", { manager: FILE_MANAGER })}</button>
      <div className="menu-sep" />
      <button onClick={go(props.onRemove)}>{t("library.folders.removeFromList")}</button>
    </div>
  );
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// the view tabs in the window's top strip: the ways to look at the same photos. Adding a
// view is one entry in VIEW_TABS and its three strings.
import { useRef } from "react";
import { t } from "./i18n";
import { keys } from "./keys";
import { MenuBar } from "./MenuBar";
import type { ActionId, Menu } from "./menu";
import { EditViewIcon, GridViewIcon, PeopleViewIcon, ScenesViewIcon } from "./icons";

export type ViewMode = "edit" | "grid" | "scenes" | "people";

export type ViewTab = {
  id: ViewMode;
  label: string;                       // locale key: the tab's text
  title: string;                       // locale key: what it shows, before the shortcut
  icon: () => React.ReactNode;
  shortcut: string;                    // shown in the tooltip, as the keyboard help writes it
  badge?: boolean;                     // a needs-review dot, when the `badges` prop does not say: takes no width
  needs?: "scenes";                    // what must be true for the tab to be enabled
};

export const VIEW_TABS: ViewTab[] = [
  { id: "edit", label: "viewTabs.edit", title: "viewTabs.editTitle", icon: EditViewIcon, shortcut: "G / Esc" },
  { id: "grid", label: "viewTabs.grid", title: "viewTabs.gridTitle", icon: GridViewIcon, shortcut: "G" },
  { id: "scenes", label: "viewTabs.scenes", title: "viewTabs.scenesTitle", icon: ScenesViewIcon, shortcut: "⇧S", needs: "scenes" },
  { id: "people", label: "viewTabs.people", title: "viewTabs.peopleTitle", icon: PeopleViewIcon, shortcut: "⇧P" },
];

export function ViewTabs({ view, onView, canScenes, tabs = VIEW_TABS, badges }: {
  view: ViewMode; onView: (v: ViewMode) => void; canScenes: boolean; tabs?: ViewTab[]; badges?: Partial<Record<ViewMode, boolean>>;
}) {
  const list = useRef<HTMLDivElement>(null);
  const off = (tab: ViewTab) => tab.needs === "scenes" && !canScenes;
  const onKeyDown = (e: React.KeyboardEvent) => {
    const els = [...(list.current?.querySelectorAll<HTMLButtonElement>("[role=tab]") ?? [])];
    const i = els.indexOf(e.target as HTMLButtonElement);
    if (i < 0) return;
    const rtl = getComputedStyle(list.current!).direction === "rtl";
    const usable = (j: number) => els[j].getAttribute("aria-disabled") !== "true";
    let step = 0, to = -1;
    if (e.key === "ArrowRight") step = rtl ? -1 : 1;
    else if (e.key === "ArrowLeft") step = rtl ? 1 : -1;
    else if (e.key === "Home") { to = 0; step = 1; }
    else if (e.key === "End") { to = els.length - 1; step = -1; }
    else return;
    e.preventDefault();
    if (to < 0) to = i + step;
    while (to >= 0 && to < els.length && !usable(to)) to += step;
    if (to < 0 || to >= els.length) return;
    els[to].focus();
    els[to].click();   // automatic activation: the arrow selects the view
  };
  // one tab stop: the selected tab, or the first usable one when the view has no tab here
  const stop = tabs.some((x) => x.id === view) ? view : tabs.find((x) => !off(x))?.id;
  return (
    <div className="view-tabs" role="tablist" aria-label={t("viewTabs.label")} ref={list} onKeyDown={onKeyDown}>
      {tabs.map((tab) => {
        const on = view === tab.id, label = t(tab.label);
        return (
          <button key={tab.id} type="button" role="tab" className="view-tab" id={`view-tab-${tab.id}`} data-view={tab.id}
                  data-badge={(badges?.[tab.id] ?? tab.badge) || undefined} aria-selected={on} aria-disabled={off(tab) || undefined} aria-label={label}
                  tabIndex={stop === tab.id ? 0 : -1}
                  title={`${t(tab.title)} (${keys(tab.shortcut)})`}
                  onClick={() => { if (!off(tab)) onView(tab.id); }}>
            <tab.icon /><span className="view-tab-label">{label}</span><span className="tab-badge" aria-hidden="true" />
          </button>
        );
      })}
    </div>
  );
}

// the window's top strip: the title, the menus (Windows and Linux; macOS has the system
// menu bar), then the view tabs centred in the space that is left, in a box that is the container. Only the strip, the
// title and the box around the tabs drag the window: a drag region over the tabs would eat clicks.
export function TitleStrip({ mac, menus, disabled, onAction, ...tabs }: {
  mac: boolean; menus: Menu[]; disabled: ReadonlySet<ActionId>; onAction: (id: ActionId) => void;
  view: ViewMode; onView: (v: ViewMode) => void; canScenes: boolean; tabs?: ViewTab[]; badges?: Partial<Record<ViewMode, boolean>>;
}) {
  return (
    <div className={`titlebar${mac ? "" : " with-menus"}`} data-tauri-drag-region>
      <span data-tauri-drag-region>Immage</span>
      {!mac && <MenuBar menus={menus} disabled={disabled} onAction={onAction} />}
      <div className="titlebar-tabs" data-tauri-drag-region><ViewTabs {...tabs} /></div>
    </div>
  );
}

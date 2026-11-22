// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The folder browser's pure parts: breadcrumbs, the parent of a path, which
// folders show and how the highlight moves. No React, no commands.
import type { BrowserDir } from "./bindings";

export type Crumb = { label: string; path: string };

const windows = (p: string) => /^[A-Za-z]:[\\/]?/.test(p) || p.startsWith("\\\\");

/** the path's segments from its root, each with the path it opens */
export function crumbs(path: string): Crumb[] {
  if (windows(path)) {
    const unc = path.startsWith("\\\\");
    const parts = path.split(/[\\/]+/).filter(Boolean);
    if (unc) {
      const root = `\\\\${parts[0]}\\${parts[1] ?? ""}`;
      return parts.slice(1).map((label, i) => ({ label: i === 0 ? `${parts[0]}\\${label}` : label,
        path: i === 0 ? root : `${root}\\${parts.slice(2, i + 2).join("\\")}` }));
    }
    return parts.map((label, i) => ({ label, path: i === 0 ? `${label}\\` : `${parts[0]}\\${parts.slice(1, i + 1).join("\\")}` }));
  }
  if (!path.startsWith("/")) return [{ label: path, path }];   // "~" before it is resolved
  const parts = path.split("/").filter(Boolean);
  return [{ label: "/", path: "/" }, ...parts.map((label, i) => ({ label, path: `/${parts.slice(0, i + 1).join("/")}` }))];
}

/** the folder above, or null at a root */
export function parentOf(path: string): string | null {
  const c = crumbs(path);
  return c.length > 1 ? c[c.length - 2].path : null;
}

export function visibleDirs(dirs: BrowserDir[], showHidden: boolean): BrowserDir[] {
  return showHidden ? dirs : dirs.filter((d) => !d.hidden);
}

/** the highlighted row after a key; -1 means none. Stays within the list */
export function moveIndex(current: number, key: "ArrowUp" | "ArrowDown" | "Home" | "End", count: number): number {
  if (count === 0) return -1;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  if (key === "ArrowDown") return Math.min(count - 1, current + 1);
  return current < 0 ? count - 1 : Math.max(0, current - 1);
}

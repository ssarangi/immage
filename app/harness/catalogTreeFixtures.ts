// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Fixture folder trees for the Catalog harness (view=card): made-up names and counts, in the shape the
// backend's catalog_tree_children answers (SourceTree, catalogTree.ts). "small" is a handful of
// folders per source with an error folder and a long name; "big" is one source of 110,101 folders
// (100 x 100 x 10), to prove the tree draws a window of rows whatever its size.
import type { TreeNode } from "../src/bindings";
import { emptyTree, treeKey, type SourceTree } from "../src/catalogTree";

const node = (parent: string, name: string, direct: number, subtree: number, hasChildren: boolean, status = "ok"): TreeNode => ({
  parent, rel: parent ? `${parent}/${name}` : name, name, has_children: hasChildren, direct, subtree, status,
});

const make = (rootDirect: number, kids: Record<string, TreeNode[]>): SourceTree => {
  const top = kids[""] ?? [];
  return {
    generation: 1,
    root: { parent: "", rel: "", name: "", has_children: top.length > 0, direct: rootDirect, subtree: rootDirect + top.reduce((a, k) => a + k.subtree, 0), status: "ok" },
    kids: new Map(Object.entries(kids)),
  };
};

export function smallTrees(): Map<number, SourceTree> {
  const trees = new Map<number, SourceTree>();
  trees.set(1, make(0, {
    "": [node("", "2017", 12400, 12400, false), node("", "2018", 0, 11980, true), node("", "2019", 20, 14310, true), node("", "Scans (access denied)", 0, 0, false, "denied"),
         node("", "Weddings and other long events with a very long folder name that has to be cut", 52, 52, false)],
    "2019": [node("2019", "Iceland", 3204, 3204, true), node("2019", "Lisbon", 1022, 1022, false), node("2019", "Misc", 87, 87, false), node("2019", "Zürich", 0, 0, false)],
    "2019/Iceland": [node("2019/Iceland", "Day 1", 999, 999, false), node("2019/Iceland", "Day 2", 2205, 2205, false)],
    "2018": [node("2018", "Spring", 5000, 5000, false), node("2018", "Autumn", 6980, 6980, false)],
  }));
  trees.set(2, make(0, { "": [node("", "Kyoto", 2100, 2100, false), node("", "Tokyo", 2110, 2110, false)] }));
  trees.set(3, make(0, { "": [node("", "Birds", 30000, 30000, false), node("", "Mammals", 8771, 8771, false)] }));
  trees.set(4, make(12, { "": [node("", "DCIM", 0, 3436, true)], "DCIM": [node("DCIM", "100EOS5D", 3436, 3436, false)] }));
  trees.set(5, emptyTree());
  return trees;
}

/** One source (id 1) of 110,101 folders: 100 years, each with 100 events of 10 shoots. */
export function bigTrees(): Map<number, SourceTree> {
  const kids: Record<string, TreeNode[]> = { "": [] };
  for (let a = 0; a < 100; a++) {
    const year = `year${String(a).padStart(3, "0")}`;
    kids[""].push(node("", year, 0, 3000, true));
    kids[year] = [];
    for (let b = 0; b < 100; b++) {
      const event = `event${String(b).padStart(3, "0")}`;
      kids[year].push(node(year, event, 0, 30, true));
      const path = `${year}/${event}`;
      kids[path] = [];
      for (let c = 0; c < 10; c++) kids[path].push(node(path, `shoot${c}`, 3, 3, false));
    }
  }
  const trees = new Map<number, SourceTree>();
  trees.set(1, make(0, kids));
  return trees;
}

/** Every folder of a source open (for the big tree: 110,101 rows). */
export function allOpen(tree: SourceTree, id: number): Set<string> {
  const out = new Set<string>([treeKey(id, "")]);
  for (const [parent, list] of tree.kids) if (list.length && parent !== "") out.add(treeKey(id, parent));
  for (const k of tree.kids.get("") ?? []) if (k.has_children) out.add(treeKey(id, k.rel));
  return out;
}

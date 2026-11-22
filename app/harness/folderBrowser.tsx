// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the folder browser: the real component over a fake file system, no Tauri.
//
//   /harness/folderBrowser.html?state=slow&theme=light&dir=rtl
//
//   state   normal (default) | slow (a folder never answers) | error (it fails, Retry works the second time)
//           | offline | timeout | empty | loading (the first listing never returns)
//   theme   a built-in theme id (dark, light, ...)
//   dir     rtl mirrors the page
//   start   the folder to open (default /home/me/Pictures)
//   result  #result shows what the last choice or cancel returned
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { FolderBrowser } from "../src/FolderBrowser";
import type { BrowserListing, BrowserPlace } from "../src/bindings";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((x) => x.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
if (q.get("dir") === "rtl") document.documentElement.dir = "rtl";
document.body.style.background = "var(--window)";
const state = q.get("state") ?? "normal";

const TREE: Record<string, string[]> = {
  "/": ["home", "mnt", "usr", "var"],
  "/home": ["me"],
  "/home/me": ["Pictures", "Documents", "Desktop", "Downloads", ".cache", ".config"],
  "/home/me/Pictures": ["2024", "2025", "Edits", "Scans", "Travel to Lisbon with a very long folder name that has to be clipped", ".thumbnails"],
  "/home/me/Pictures/2025": ["01 January", "02 February", "03 March"],
  "/mnt": ["synology"],
  "/mnt/synology": ["Photos_Backup"],
  "/mnt/synology/Photos_Backup": ["Family", "Trips", "Raw"],
};
const place = (kind: string, label: string, path: string, o: Partial<BrowserPlace> = {}): BrowserPlace =>
  ({ kind, label, path, host: null, network: false, online: true, ...o });
const PLACES: BrowserPlace[] = [
  place("home", "me", "/home/me"), place("pictures", "Pictures", "/home/me/Pictures"),
  place("desktop", "Desktop", "/home/me/Desktop"), place("documents", "Documents", "/home/me/Documents"),
  place("root", "/", "/"),
  place("drive", "USB Stick", "/media/me/USB Stick"),
  place("drive", "Photos_Backup", "/mnt/synology/Photos_Backup", { network: true, host: "SynologyDS220.local", online: state !== "offline" }),
  place("added", "Pictures", "/home/me/Pictures"), place("added", "Raw", "/mnt/synology/Photos_Backup/Raw"),
];
let failed = false;
const never = <T,>() => new Promise<T>(() => {});
const listing = (path: string, error: string | null = null): BrowserListing => {
  const names = TREE[path] ?? [];
  return { path, parent: path === "/" ? null : path.replace(/\/[^/]*$/, "") || "/", error,
    dirs: names.map((name) => ({ name, path: `${path === "/" ? "" : path}/${name}`, hidden: name.startsWith(".") })) };
};
const list = async (p: string): Promise<BrowserListing> => {
  const path = p === "~" ? "/home/me" : p;
  const at = new Promise((r) => setTimeout(r, 120));
  await at;
  if (state === "loading") return never();
  if (path.startsWith("/mnt/synology")) {
    if (state === "slow") return never();
    if (state === "offline") return listing(path, "offline");
    if (state === "timeout") return listing(path, "timeout");
    if (state === "error" && !failed) { failed = true; return listing(path, "denied"); }
  }
  if (state === "empty" && path.endsWith("/Edits")) return listing(path);
  if (!(path in TREE) && !path.endsWith("/Edits")) return listing(path, "missing");
  return listing(path);
};

function Harness() {
  const [result, setResult] = useState("");
  return (
    <>
      <FolderBrowser title="Choose a folder" start={q.get("start") ?? "/home/me/Pictures"} added={["/home/me/Pictures"]}
                     list={list} places={async () => PLACES}
                     onChoose={(p) => setResult(`chose ${p}`)} onCancel={() => setResult("cancelled")} />
      <pre id="result" hidden>{result}</pre>
    </>
  );
}
createRoot(document.getElementById("root")!).render(<Harness />);
document.body.dataset.ready = "1";

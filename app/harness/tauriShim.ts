// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Stands in for the Tauri runtime so the app's modules load in a plain browser.
// Any command the page sends fails, which is what the harness wants: nothing in
// it may reach the engine.
const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
w.__TAURI_INTERNALS__ ??= {
  convertFileSrc: (path: string) => path,
  invoke: () => Promise.reject(new Error("harness: no Tauri")),
  transformCallback: () => 0,
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
};

// SPDX-License-Identifier: GPL-3.0-or-later
// @tauri-apps/api/core for benchmarking/open-photo.mjs: verify/shim.mjs (the app's Session
// against a real darktable-engine) plus the commands opening a photo also sends, which the
// verify shim leaves out (a real cancel_base, prefetch, path_exists), and a record of every
// command's start, end and engine-side milliseconds.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import * as shim from "../../verify/shim.mjs";
export * from "../../verify/shim.mjs";

export const log = [];
// commands the engine refused (a late render of a photo the engine has since left, a canceled base)
export const failures = [];
export const trace = { on: false };

// the base render as the Rust layer sends it (session_cmds.rs, render): on the engine's base pipe, with
// a numbered id that cancel_base names, and the frame in the 64-byte header format of main.rs
let baseId = 0, home = null;
export const setHome = (h) => { home = h; };
async function render({ request: { width, height, scale, x, y, skip, draft, base, have, edits } }) {
  for (const e of edits ?? []) {
    const set = { cmd: "set" };
    for (const k of ["op", "params", "enabled", "after", "before", "instance"]) if (e[k] != null) set[k] = e[k];
    await shim.request(set);
  }
  const frame = join(home, "frame.rgba");
  const req = { cmd: "render", width, height, rgba: frame };
  if (skip) req.skip = skip;
  if (draft) req.draft = true;
  if (have) req.have = have;
  if (base) { req.base = true; req.id = ++baseId; }
  if (scale != null) Object.assign(req, { scale, x: x ?? 0, y: y ?? 0 });
  const r = await shim.request(req);
  const pixels = r.kept ? new Uint8Array(0) : readFileSync(frame);
  const out = new ArrayBuffer(shim.FRAME_HEAD + pixels.length);
  const v = new DataView(out);
  ["width", "height", "full_width", "full_height", "x", "y"].forEach((k, i) => v.setUint32(4 * i, r[k] ?? 0, true));
  v.setFloat32(32, r.process_ms ?? 0, true);
  v.setFloat32(36, r.write_ms ?? 0, true);
  v.setBigUint64(48, BigInt("0x" + (r.hash || "0")), true);
  new Uint8Array(out, shim.FRAME_HEAD).set(pixels);
  return out;
}

const OVERRIDES = {
  render,
  path_exists: ({ path }) => Promise.resolve(existsSync(path)),
  // reader-only in the engine: answered while the main loop renders, as the Rust layer's cancel does
  // answered by nothing (engine/main.c, _read_requests): written and not waited for
  cancel_base: () => { if (baseId) void shim.requestTagged({ cmd: "cancel_base", id: baseId }).catch(() => {}); return Promise.resolve(); },
  prefetch: ({ imgids }) => shim.request({ cmd: "prefetch", imgids }).then(() => null),
};

export async function invoke(cmd, args = {}) {
  const t0 = performance.now();
  const entry = { cmd, t0 };
  if (trace.on) log.push(entry);
  if (process.env.OPEN_DEBUG) console.error(Math.round(t0), "->", cmd);
  try {
    const r = await (OVERRIDES[cmd] ? OVERRIDES[cmd](args) : shim.invoke(cmd, args));
    // the engine's own time for the request (replies carry `ms`; frames carry process_ms in a header)
    if (r && typeof r === "object" && typeof r.ms === "number") entry.engineMs = r.ms;
    return r;
  } catch (e) {
    entry.error = String(e.message ?? e);
    failures.push({ cmd, error: entry.error });
    throw e;
  } finally {
    entry.t1 = performance.now();
    if (process.env.OPEN_DEBUG) console.error(Math.round(entry.t1), "<-", cmd);
  }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Scroll smoothness of the large grid in headless Chrome (app/harness/grid.tsx: the real Filmstrip and
// ThumbLoader, synthetic thumbnails, 2-5 ms mocked IPC). Needs `vite` serving app/ and playwright:
//   PORT=14777 PLAYWRIGHT=/path/to/playwright/index.mjs node benchmarking/grid-scroll.mjs [n ...]
// Reports per scenario: frame times (rAF deltas), long tasks, React commits, cells added/removed, scroll
// events, loader calls and IPC calls, and the browser's layout / style / script time.
const { chromium } = await import(process.env.PLAYWRIGHT ?? "playwright");
const PORT = process.env.PORT ?? 14777;
const sizes = process.argv.slice(2).map(Number);
if (!sizes.length) sizes.push(10000, 25000);
const LAT = process.env.LATENCY ?? 3;
const pct = (a, p) => { const s = [...a].sort((x, y) => x - y); return s[Math.min(s.length - 1, Math.floor(s.length * p))] ?? 0; };

for (const n of sizes) {
  for (const scenario of ["wheel", "drag", "pagedown", "reverse"]) {
    const b = await chromium.launch();
    const p = await b.newPage({ viewport: { width: 1300, height: 900 } });
    const cdp = await p.context().newCDPSession(p);
    await cdp.send("Performance.enable");
    await p.goto(`http://127.0.0.1:${PORT}/harness/grid.html?n=${n}&mode=grid&latency=${LAT}`);
    await p.waitForFunction(() => window.__h && document.querySelector(".filmstrip .thumb"));
    await p.waitForTimeout(1500);
    // OPEN=1: a photo is open, as it is while the grid is in use (the grid keeps its anchor and scroll to it)
    if (process.env.OPEN) { await p.evaluate(() => window.__h.open(3)); await p.waitForTimeout(300); }
    await p.evaluate(() => {
      const root = document.querySelector(".filmstrip");
      const w = window; w.__m = { frames: [], scrolls: 0, added: 0, removed: 0, blank: [], nodes: 0 };
      root.addEventListener("scroll", () => w.__m.scrolls++, { passive: true });
      new MutationObserver((rs) => { for (const r of rs) { for (const a of r.addedNodes) if (a.classList?.contains("thumb")) w.__m.added++;
        for (const a of r.removedNodes) if (a.classList?.contains("thumb")) w.__m.removed++; } }).observe(root, { childList: true });
      let last = performance.now();
      // cells on screen still showing the placeholder, sampled every 4th frame: the pop-in a person sees
      let k = 0;
      const tick = (t) => {
        w.__m.frames.push(t - last); last = t;
        if (++k % 4 === 0) {
          const vr = root.getBoundingClientRect(); let blank = 0;
          for (const c of root.querySelectorAll(".thumb")) { const r = c.getBoundingClientRect();
            if (r.bottom > vr.top && r.top < vr.bottom && c.querySelector(".placeholder")) blank++; }
          w.__m.blank.push(blank);
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
      w.__h.longTasks.length = 0;
      Object.keys(w.__h.counts).forEach((k) => (w.__h.counts[k] = 0));
    });
    const m0 = Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((x) => [x.name, x.value]));
    await p.mouse.move(600, 400);
    const start = Date.now();
    if (scenario === "wheel") for (let i = 0; i < 180; i++) { await p.mouse.wheel(0, 100); await p.waitForTimeout(16); }
    else if (scenario === "drag") for (let i = 0; i < 90; i++) { await p.evaluate((i) => { const r = document.querySelector(".filmstrip"); r.scrollTop = i * 1500; }, i); await p.waitForTimeout(16); }
    else if (scenario === "pagedown") for (let i = 0; i < 40; i++) { await p.evaluate(() => { const r = document.querySelector(".filmstrip"); r.scrollTop += r.clientHeight * 0.9; }); await p.waitForTimeout(120); }
    else { // reverse: down then back up, scrolling back over what was loaded
      for (let i = 0; i < 90; i++) { await p.mouse.wheel(0, 100); await p.waitForTimeout(16); }
      await p.evaluate(() => { const w = window; w.__m.frames.length = 0; w.__m.added = w.__m.removed = 0; w.__m.scrolls = 0; w.__h.longTasks.length = 0; Object.keys(w.__h.counts).forEach((k) => (w.__h.counts[k] = 0)); });
      for (let i = 0; i < 90; i++) { await p.mouse.wheel(0, -100); await p.waitForTimeout(16); }
    }
    const wall = Date.now() - start;
    await p.waitForTimeout(500);
    const r = await p.evaluate(() => ({ ...window.__m, counts: { ...window.__h.counts }, long: [...window.__h.longTasks], cells: document.querySelectorAll(".thumb").length, painted: document.querySelectorAll(".thumb-image canvas").length, nodes: document.getElementsByTagName("*").length }));
    const m1 = Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((x) => [x.name, x.value]));
    const d = (k) => Math.round((m1[k] - m0[k]) * 1000);
    const f = r.frames.slice(1);
    console.log(JSON.stringify({ n, scenario, wallMs: wall, frames: f.length, p50: +pct(f, .5).toFixed(1), p95: +pct(f, .95).toFixed(1), max: Math.round(Math.max(...f)),
      over20: f.filter((x) => x > 20).length, over34: f.filter((x) => x > 34).length, blankAvg: +(r.blank.reduce((a, c) => a + c, 0) / Math.max(1, r.blank.length)).toFixed(1), blankMax: Math.max(0, ...r.blank), nodesPerCell: Math.round(r.nodes / Math.max(1, r.cells)), long: r.long.length, longMax: Math.max(0, ...r.long),
      scrolls: r.scrolls, commits: r.counts.commits, commitMs: Math.round(r.counts.commitMs), added: r.added, removed: r.removed,
      request: r.counts.request, show: r.counts.show, retain: r.counts.retain, ipc: r.counts.invokes,
      layoutMs: d("LayoutDuration"), styleMs: d("RecalcStyleDuration"), scriptMs: d("ScriptDuration"), taskMs: d("TaskDuration"), cells: r.cells, painted: r.painted }));
    await b.close();
  }
}

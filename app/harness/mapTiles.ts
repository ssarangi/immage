// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Statistics harness's stand-in for the `map_tile` command: OpenStreetMap-looking light tiles (water, land,
// a faint graticule) drawn on a canvas from a coarse hand-made world outline, so a headless screenshot shows a
// realistic map without a network or Tauri. They are light on purpose: the page's own dark treatment (a CSS
// filter over the tile pane) turns them dark exactly as it does real tiles. ?tiles=offline makes every tile fail.
const LAND: [number, number][][] = [
  // North America
  [[-168, 66], [-156, 71], [-125, 70], [-95, 72], [-80, 68], [-62, 60], [-56, 52], [-66, 44], [-76, 35], [-81, 25], [-97, 26], [-97, 19], [-88, 16], [-83, 9], [-78, 8], [-92, 14], [-105, 20], [-117, 32], [-125, 42], [-124, 49], [-135, 58], [-150, 60], [-165, 55]],
  // South America
  [[-78, 8], [-60, 10], [-50, 0], [-35, -6], [-40, -22], [-48, -28], [-58, -38], [-66, -46], [-72, -53], [-75, -45], [-71, -30], [-70, -18], [-81, -6]],
  // Europe and Asia
  [[-10, 36], [-9, 43], [-2, 48], [3, 52], [8, 57], [12, 66], [28, 71], [60, 69], [100, 77], [140, 73], [170, 69], [180, 66], [160, 58], [142, 52], [135, 43], [122, 40], [122, 30], [110, 20], [106, 10], [100, 3], [104, 1], [98, 8], [93, 20], [80, 10], [73, 18], [67, 25], [57, 25], [52, 16], [43, 13], [35, 28], [32, 36], [26, 38], [22, 37], [12, 43], [8, 44], [-2, 37]],
  // Africa
  [[-17, 21], [-10, 35], [10, 37], [32, 31], [43, 12], [51, 11], [40, -3], [40, -15], [33, -26], [20, -35], [12, -17], [9, -1], [5, 5], [-8, 4], [-17, 14]],
  // Australia, Greenland, Britain, Japan, New Zealand
  [[114, -22], [130, -12], [142, -11], [153, -26], [150, -37], [138, -35], [115, -34]],
  [[-55, 60], [-45, 60], [-20, 70], [-20, 82], [-60, 82], [-70, 76]],
  [[-5, 50], [1, 51], [-3, 56], [-6, 58]],
  [[130, 31], [140, 35], [142, 43], [140, 41], [135, 34]],
  [[172, -34], [178, -38], [174, -42], [168, -46], [171, -41]],
];

const world = (lng: number, lat: number, z: number) => {
  const size = 256 * 2 ** z;
  const s = Math.sin((Math.max(-85.05, Math.min(85.05, lat)) * Math.PI) / 180);
  return [((lng + 180) / 360) * size, (0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI)) * size] as const;
};

export async function harnessTile(z: number, x: number, y: number, mode: string | null): Promise<ArrayBuffer> {
  if (mode === "offline") throw new Error("harness: offline");
  const c = document.createElement("canvas");
  c.width = c.height = 256;
  const g = c.getContext("2d")!;
  g.fillStyle = "#aad3df";
  g.fillRect(0, 0, 256, 256);
  const [ox, oy] = [x * 256, y * 256];
  g.fillStyle = "#f2efe9";
  g.strokeStyle = "#c6bfae";
  g.lineWidth = 1;
  for (const poly of LAND) {
    g.beginPath();
    poly.forEach(([lng, lat], i) => { const [px, py] = world(lng, lat, z); if (i) g.lineTo(px - ox, py - oy); else g.moveTo(px - ox, py - oy); });
    g.closePath();
    g.fill();
    g.stroke();
  }
  // a faint graticule like the road grid of a map, every 10 degrees at low zoom and finer as it zooms
  const step = z < 4 ? 10 : z < 7 ? 2 : z < 9 ? 0.5 : 0.1;
  g.strokeStyle = "rgba(120, 120, 120, 0.22)";
  for (let lng = -180; lng <= 180; lng += step) { const [px] = world(lng, 0, z); if (px >= ox - 1 && px <= ox + 257) { g.beginPath(); g.moveTo(px - ox, 0); g.lineTo(px - ox, 256); g.stroke(); } }
  for (let lat = -80; lat <= 80; lat += step) { const [, py] = world(0, lat, z); if (py >= oy - 1 && py <= oy + 257) { g.beginPath(); g.moveTo(0, py - oy); g.lineTo(256, py - oy); g.stroke(); } }
  const blob = await new Promise<Blob | null>((done) => c.toBlob(done, "image/png"));
  if (!blob) throw new Error("harness: no tile");
  return blob.arrayBuffer();
}

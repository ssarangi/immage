// SPDX-License-Identifier: GPL-3.0-or-later
// The Statistics view's no-layout-shift rule: hover, keyboard focus and a selected state on every
// interactive element, and a real click on a chart mark, move nothing (app/harness/statsShift.ts). Needs the
// Vite dev server on port 1430 (cd app && npm run dev) and Google Chrome; skipped without either.
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { mkdtempSync } from 'node:fs';
import { join } from 'node:path';

const CHROME = '/usr/bin/google-chrome';
const BASE = 'http://localhost:1430/harness/statistics.html';
const up = async () => { try { return (await fetch(BASE, { signal: AbortSignal.timeout(2000) })).ok; } catch { return false; } };
const skip = !existsSync(CHROME) ? 'Google Chrome is not installed' : !(await up()) ? 'the dev server is not running on port 1430' : false;

function run(query, w = 1440, h = 900) {
  const dir = mkdtempSync(join(tmpdir(), 'stats-shift-'));
  const dom = execFileSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', '--hide-scrollbars', `--user-data-dir=${dir}`,
    '--virtual-time-budget=30000', `--window-size=${w},${h}`, '--dump-dom', `${BASE}?${query}&shift=1`], { encoding: 'utf8', timeout: 90000, maxBuffer: 64 << 20 });
  const m = dom.match(/<pre id="report"[^>]*>([\s\S]*?)<\/pre>/);
  assert.ok(m && m[1].trim(), `no report for ${query}`);
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
}

for (const [name, query, w, h] of [
  ['ready, dark', 'state=ready', 1440, 900],
  ['ready, light', 'state=ready&theme=light', 1440, 900],
  ['ready, filtered', 'state=ready&filter=focal:normal,month:10,maker:Canon', 1440, 900],
  ['ready, wide', 'state=ready', 2560, 1528],
  ['loading', 'state=loading', 1440, 900],
  ['empty result', 'state=empty', 1440, 900],
  ['error', 'state=error', 1440, 900],
  ['first run', 'state=firstrun', 1440, 900],
]) {
  test(`no layout shift: ${name}`, { skip }, () => {
    const r = run(query, w, h);
    assert.ok(r.interactions > (query.includes('firstrun') ? 1 : 5), `${r.interactions} interactions measured`);
    assert.equal(r.maxDelta, 0, `shift on ${JSON.stringify(r.failures?.[0])}`);
    assert.equal(r.click.max, 0, `a click moved ${r.click.worst}`);
  });
}

// the test has teeth: a hover style that adds padding must be reported as a shift
test('negative control: a style that moves layout on hover fails the measurement', { skip }, () => {
  const r = run('state=ready&bad=1');
  assert.ok(r.maxDelta > 0, 'the measurement did not notice a hover that moves the card');
});

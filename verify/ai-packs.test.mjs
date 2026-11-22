// SPDX-License-Identifier: GPL-3.0-or-later
// The pure logic of Preferences > General > AI models (app/src/aiPacks.ts): what each pack state
// shows (pill, one slot line, buttons, whether the analysis switch may be on), the background
// work line, and the rule that a download starts only from the consent dialog, Resume or Repair.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import './load-app-ts.mjs';
import './shim.mjs';

const en = JSON.parse(readFileSync(new URL('../app/src/locales/en.json', import.meta.url), 'utf8'));
const tr = (key, vars = {}) => {
  const n = vars.n;
  const pick = (k) => en[k];
  const text = (typeof n === 'number' ? pick(`${key}_${n === 1 ? 'one' : 'other'}`) : undefined) ?? pick(key) ?? pick(`${key}_other`);
  assert.ok(text !== undefined, `missing string ${key}`);
  return text.replace(/\{(\w+)\}/g, (m, name) => (name in vars ? String(vars[name]) : m));
};
const { packView, workView, analysedLine, sizeLabel, etaLabel, summaryOf, percent, plainFmt } = await import('../app/src/aiPacks.ts');

const base = {
  id: 'clip', name: 'Smart search (CLIP)', license: 'MIT', license_url: '', source_host: 'huggingface.co',
  bytes: 607_974_672, bytes_on_disk: 0, bytes_left: 607_974_672, files: 7, files_ok: 0, state: 'notInstalled', checking: false,
  error: null, version: 'rev', update_version: null, update_bytes: null, verified_at: null, free_bytes: 212e9,
  unsupported_reason: null, analyses: ['smartSearch'], progress: null, download_consent: false, analysis_consent: false, folder: '/x',
};
const view = (over, flash = false) => packView({ ...base, ...over }, tr, plainFmt, flash);
const ids = (v) => v.actions.map((a) => a.id);

test('sizes: MB below a gigabyte, one-decimal GB above', () => {
  assert.equal(sizeLabel(607_974_672), '608 MB');
  assert.equal(sizeLabel(1_700_000_000), '1.7 GB');
  assert.equal(sizeLabel(1_234_567, plainFmt, 'MB'), '2 MB');
  assert.equal(etaLabel(40, tr), '40 s');
  assert.equal(etaLabel(600, tr), '10 min');
  assert.equal(etaLabel(3 * 3600, tr), '3 h');
});

test('not installed: Download is the only action, nothing is started, the switch says why it is off', () => {
  const v = view({});
  assert.equal(v.pill.text, 'Not installed');
  assert.deepEqual(ids(v), ['download']);
  assert.equal(v.actions[0].label, 'Download (608 MB)...');
  assert.equal(v.actions[0].primary, true);
  assert.match(v.message, /Needs 608 MB of free space\. Nothing is downloaded until you press Download\./);
  assert.equal(v.usable, false);
  assert.equal(v.reason, 'Turn this on after the model is installed.');
  assert.equal(v.bar, null);
});

test('disk full: the message is red and Download stays so the user can retry', () => {
  const v = view({ error: { kind: 'diskFull', detail: 'x' }, free_bytes: 0.9e9, bytes: 1.7e9 });
  assert.equal(v.tone, 'bad');
  assert.match(v.message, /Needs 1\.7 GB free\. This disk has 900 MB\./);
  assert.deepEqual(ids(v), ['download']);
});

test('downloading: percent in the pill, bytes, speed and time in the slot, only Cancel', () => {
  const v = view({ state: 'downloading', progress: { id: 'clip', done: 207_000_000, total: 607_974_672, bytes_per_sec: 6_200_000, eta_s: 65, phase: 'download' } });
  assert.equal(v.pill.text, 'Downloading 34%');
  assert.equal(v.pill.icon, 'spinner');
  assert.equal(v.message, '207 MB of 608 MB · 6.2 MB/s · about 70 s left'.replace('70 s', etaLabel(65, tr)));
  assert.equal(v.right, 'Checking files after the download');
  assert.ok(Math.abs(v.bar - 0.34) < 0.01);
  assert.deepEqual(ids(v), ['cancel']);
  const checking = view({ state: 'downloading', progress: { id: 'clip', done: 1, total: 1, bytes_per_sec: 0, eta_s: null, phase: 'verify' } });
  assert.equal(checking.pill.text, 'Checking files');
  assert.equal(checking.message, 'Checking the downloaded files…');
  // before the first sample: "Starting", never "NaN" or an empty line
  assert.match(view({ state: 'downloading', progress: { id: 'clip', done: 0, total: 100, bytes_per_sec: 0, eta_s: null, phase: 'download' } }).message, /Starting/);
  assert.equal(percent({ done: 5, total: 0 }), 0);
});

test('installed: "verified" only with a recorded check; otherwise "checking"', () => {
  const when = Date.UTC(2026, 10, 14, 12) / 1000;
  const v = view({ state: 'installed', bytes_on_disk: 607_974_672, verified_at: when });
  assert.equal(v.pill.text, 'Installed · verified');
  assert.equal(v.pill.kind, 'ok');
  assert.match(v.message, /^608 MB on disk · checked Nov 14, every file matches its known checksum$/);
  assert.deepEqual(ids(v), ['check', 'remove']);
  assert.equal(v.usable, true);
  assert.equal(v.reason, null);
  assert.equal(v.actions.some((a) => a.primary), false, 'an installed pack has no filled button');
  const unchecked = view({ state: 'installed', verified_at: null, checking: true });
  assert.equal(unchecked.pill.text, 'Installed · checking');
  assert.ok(!/verified/.test(unchecked.pill.text));
  assert.equal(view({ state: 'installed', verified_at: when }, true).message, 'All 7 files match. Checked just now.');
});

test('needs repair: plain reason, Repair is primary, the analysis is unavailable', () => {
  const v = view({ state: 'needsRepair', error: { kind: 'hashMismatch', detail: 'onnx/a.bin failed pinned integrity check' } });
  assert.equal(v.pill.text, 'Needs repair');
  assert.equal(v.pill.kind, 'warn');
  assert.match(v.message, /does not match its checksum.*Analysis is off until it is repaired/);
  assert.ok(!v.message.includes('onnx/'), 'technical detail stays in the tooltip');
  assert.deepEqual(ids(v), ['remove', 'repair']);
  assert.equal(v.actions[1].primary, true);
  assert.equal(v.usable, false);
  assert.equal(v.reason, 'Not available until the model is repaired.');
});

test('download stopped: offline keeps what was fetched, Resume shows what is left', () => {
  const v = view({ state: 'stopped', bytes: 1_700_000_000, bytes_left: 1_100_000_000, error: { kind: 'offline', detail: 'x' } });
  assert.equal(v.pill.text, 'Download stopped');
  assert.match(v.message, /No connection\. 0\.6 GB of 1\.7 GB kept\. Immage will not retry on its own\./);
  assert.deepEqual(ids(v), ['removePartial', 'resume']);
  assert.equal(v.actions[1].label, 'Resume (1.1 GB left)');
  const cancelled = view({ state: 'stopped', bytes_left: 300_000_000 });
  assert.match(cancelled.message, /Download stopped\. 308 MB of 608 MB kept/);
});

test('update and unsupported packs draw from the same card', () => {
  assert.equal(view({ state: 'updateAvailable', update_bytes: 5e8 }).actions[1].label, 'Update (500 MB)');
  const u = view({ state: 'unsupported', unsupported_reason: 'This processor lacks AVX2.' });
  assert.equal(u.pill.text, 'Cannot run on this computer');
  assert.deepEqual(ids(u), ['remove'], 'no Repair button: it would loop');
  assert.equal(u.usable, false);
});

test('background work line: nothing, counting, analysing, waiting, paused', () => {
  const on = { paused: false, enabled: true, installed: true, pending: 616, running: 1, active: true, analysed: 1204 };
  assert.equal(workView(on, tr).line, 'Analysing 1,204 of 1,820 photos');
  assert.ok(Math.abs(workView(on, tr).bar - 1204 / 1820) < 1e-9);
  assert.equal(workView({ ...on, running: 0, active: false }, tr).line, '616 photos waiting');
  assert.equal(workView({ ...on, pending: 1, running: 0, active: false }, tr).line, '1 photo waiting');
  assert.equal(workView({ ...on, paused: true }, tr).line, 'Paused · 616 photos waiting');
  assert.equal(workView({ ...on, paused: true }, tr).paused, true);
  assert.equal(workView({ ...on, pending: 0 }, tr).line, 'Nothing running');
  assert.equal(workView({ ...on, pending: null }, tr).line, 'Counting photos…');
  assert.equal(workView({ ...on, enabled: false }, tr).line, 'Nothing running');
  assert.equal(workView(null, tr).line, 'Reading status…');
});

test('the Smart search status line and the summary', () => {
  assert.equal(analysedLine(3402, 0, true, tr), '3,402 photos analysed. Nothing waiting.');
  assert.equal(analysedLine(1, 0, true, tr), '1 photo analysed. Nothing waiting.');
  assert.match(analysedLine(10, 5, true, tr), /10 photos analysed, 5 waiting/);
  assert.match(analysedLine(10, 5, false, tr), /Analysis is off/);
  const s = summaryOf([{ ...base, state: 'installed', bytes_on_disk: 5 }, { ...base, state: 'notInstalled', bytes_on_disk: 2 }]);
  assert.deepEqual(s, { installed: 1, total: 2, bytes: 7 });
});

test('a download starts only from the consent dialog; Resume and Repair pass no consent', () => {
  const src = readFileSync(new URL('../app/src/AiModelsPrefs.tsx', import.meta.url), 'utf8');
  const calls = [...src.matchAll(/commands\.packDownload\(([^)]*)\)/g)].map((m) => m[1].replace(/\s/g, ''));
  assert.deepEqual(calls.sort(), ['id,false', 'id,true']);
  // the true call is inside onConsent, which only the dialog's confirm button reaches
  assert.match(src, /onConsent=\{\(id\) => \{ setDialog\(null\); void guard\(async \(\) => \{ await commands\.packDownload\(id, true\)/);
  assert.match(src, /if \(action === "download"\) setDialog\(\{ kind: "consent", id \}\)/);
});

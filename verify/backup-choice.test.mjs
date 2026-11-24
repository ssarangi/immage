// SPDX-License-Identifier: GPL-3.0-or-later
// backupChoice.ts: what is ticked when the Back Up and Restore dialogs open. People & face data (names and biometric face
// data) is never ticked for you; the rest is. Every component has its text.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { COMPONENT_KEY, OPT_IN, initialChoice } from '../app/src/backupChoice.ts';

const en = JSON.parse(readFileSync(new URL('../app/src/locales/en.json', import.meta.url), 'utf8'));
const fr = JSON.parse(readFileSync(new URL('../app/src/locales/fr.json', import.meta.url), 'utf8'));

test('everything is ticked except People & face data, in Back Up and in Restore', () => {
  const ids = ['settings', 'presets', 'export_presets', 'looks', 'plugins', 'darktable', 'faces'];
  assert.deepEqual([...initialChoice(ids)], ids.slice(0, 6));
  assert.deepEqual([...initialChoice(['faces'])], []);
  assert.deepEqual([...initialChoice([])], []);
  assert.deepEqual([...OPT_IN], ['faces']);
});

test('every backup component has its label, and People & face data its warning, in both languages', () => {
  for (const key of Object.values(COMPONENT_KEY)) for (const dict of [en, fr]) assert.ok(dict[`dialogs.backup.component.${key}`], `missing ${key}`);
  assert.match(en['dialogs.backup.facesNote'], /names/);
  assert.match(en['dialogs.backup.facesNote'], /biometric/);
  assert.match(en['dialogs.backup.facesNote'], /not included unless you tick it/);
  assert.match(en['dialogs.backup.facesNote'], /same care as your photos/);
  assert.ok(fr['dialogs.backup.facesNote'] && fr['dialogs.backup.facesRestoreNote']);
});

test('the component ids the page knows match the ones backup.rs lists', () => {
  const rs = readFileSync(new URL('../app/src-tauri/src/backup.rs', import.meta.url), 'utf8');
  const ids = [...rs.slice(rs.indexOf('const COMPONENTS'), rs.indexOf('];', rs.indexOf('const COMPONENTS'))).matchAll(/\("(\w+)",/g)].map((m) => m[1]);
  assert.deepEqual(Object.keys(COMPONENT_KEY).sort(), ids.sort());
});

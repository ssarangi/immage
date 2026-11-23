// SPDX-License-Identifier: GPL-3.0-or-later
// Engine-free: the Library cards always toggle independently; the right panel's cards follow Focus mode.
import test from 'node:test';
import assert from 'node:assert/strict';
import './load-app-ts.mjs';

const { toggleLeftSet, togglePanelSet } = await import('../app/src/panelToggle.ts');
const s = (...a) => new Set(a);

test('left cards toggle only the clicked card, whatever Focus mode and Alt say', () => {
  assert.deepEqual([...toggleLeftSet(s('folders', 'catalog'), 'metadata')], ['folders', 'catalog', 'metadata']);
  assert.deepEqual([...toggleLeftSet(s('folders', 'catalog'), 'catalog')], ['folders']);
  assert.deepEqual([...toggleLeftSet(s(), 'history')], ['history']);
});

test('right panel with Focus mode on: a click opens alone, a click on the only open card shuts it, Alt adds', () => {
  assert.deepEqual([...togglePanelSet(s('a', 'b'), 'c', false, true)], ['c']);
  assert.deepEqual([...togglePanelSet(s('a'), 'a', false, true)], []);
  assert.deepEqual([...togglePanelSet(s('a', 'b'), 'a', false, true)], ['b']);
  assert.deepEqual([...togglePanelSet(s('a'), 'c', true, true)], ['a', 'c']);
});

test('right panel with Focus mode off: a click toggles one card, Alt makes it solo', () => {
  assert.deepEqual([...togglePanelSet(s('a'), 'c', false, false)], ['a', 'c']);
  assert.deepEqual([...togglePanelSet(s('a', 'c'), 'c', false, false)], ['a']);
  assert.deepEqual([...togglePanelSet(s('a', 'b'), 'c', true, false)], ['c']);
});

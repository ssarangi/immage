// SPDX-License-Identifier: GPL-3.0-or-later
// The folder browser's pure model: breadcrumbs, parent, hidden folders and the highlight.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
const path = new URL('../app/src/folderBrowserModel.ts', import.meta.url);
const code = transformSync(path.pathname, readFileSync(path, 'utf8')).code;
const { crumbs, parentOf, visibleDirs, moveIndex } =
  await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);

test('breadcrumbs of a posix path start at the root', () => {
  assert.deepEqual(crumbs('/mnt/nas/Photos'), [
    { label: '/', path: '/' }, { label: 'mnt', path: '/mnt' },
    { label: 'nas', path: '/mnt/nas' }, { label: 'Photos', path: '/mnt/nas/Photos' }]);
  assert.deepEqual(crumbs('/'), [{ label: '/', path: '/' }]);
});

test('breadcrumbs of drive letter and UNC paths', () => {
  assert.deepEqual(crumbs('C:\\Users\\me'), [
    { label: 'C:', path: 'C:\\' }, { label: 'Users', path: 'C:\\Users' }, { label: 'me', path: 'C:\\Users\\me' }]);
  assert.equal(parentOf('\\\\nas\\photos\\2024'), '\\\\nas\\photos');
  assert.equal(parentOf('\\\\nas\\photos'), null);
});

test('parentOf stops at the root', () => {
  assert.equal(parentOf('/a/b'), '/a');
  assert.equal(parentOf('/a'), '/');
  assert.equal(parentOf('/'), null);
  assert.equal(parentOf('C:\\'), null);
});

test('hidden folders show only when asked', () => {
  const d = [{ name: '.x', hidden: true }, { name: 'y', hidden: false }];
  assert.deepEqual(visibleDirs(d, false).map((x) => x.name), ['y']);
  assert.equal(visibleDirs(d, true).length, 2);
});

test('the highlight stays inside the list', () => {
  assert.equal(moveIndex(-1, 'ArrowDown', 3), 0);
  assert.equal(moveIndex(2, 'ArrowDown', 3), 2);
  assert.equal(moveIndex(-1, 'ArrowUp', 3), 2);
  assert.equal(moveIndex(0, 'ArrowUp', 3), 0);
  assert.equal(moveIndex(1, 'End', 3), 2);
  assert.equal(moveIndex(1, 'Home', 3), 0);
  assert.equal(moveIndex(0, 'ArrowDown', 0), -1);
});

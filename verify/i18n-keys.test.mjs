// SPDX-License-Identifier: GPL-3.0-or-later
// Every text key the page asks for exists in en.json: a missing one shows up on screen as the raw key. Scans app/src for
// t("literal.key") calls and for t(`prefix.${x}.suffix`) calls (checked by their fixed ends: some key must start with the
// prefix and end with the suffix). A plural key counts when its _one, _other (or other plural category) exists.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('../app/src/', import.meta.url));
const en = JSON.parse(readFileSync(join(SRC, 'locales/en.json'), 'utf8'));
const keys = Object.keys(en);
const have = new Set(keys);
const plural = /_(zero|one|two|few|many|other)$/;
const bases = new Set(keys.map((k) => k.replace(plural, '')));

function walk(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) return f === 'locales' ? [] : walk(p);
    return /\.tsx?$/.test(f) && !/\.d\.ts$/.test(f) ? [p] : [];
  });
}

const exists = (k) => have.has(k) || bases.has(k);
const dynamic = (prefix, suffix) => keys.some((k) => k.startsWith(prefix) && k.replace(plural, '').endsWith(suffix));

function scan() {
  const missing = [];
  for (const file of walk(SRC)) {
    const src = readFileSync(file, 'utf8');
    const rel = file.slice(SRC.length);
    // t("a.b"), t('a.b'): the call is a bare `t(`, not `.t(` or `at(`, with a key-shaped first argument
    for (const m of src.matchAll(/(?<![\w.$])t\(\s*(["'])([A-Za-z][\w-]*(?:\.[\w-]+)+)\1/g)) {
      if (!exists(m[2])) missing.push(`${rel}: ${m[2]}`);
    }
    for (const m of src.matchAll(/(?<![\w.$])t\(\s*`([^`]*)`/g)) {
      const body = m[1];
      if (!body.includes('${')) { if (!exists(body)) missing.push(`${rel}: ${body}`); continue; }
      const prefix = body.slice(0, body.indexOf('${'));
      const suffix = body.slice(body.lastIndexOf('}') + 1);
      if (prefix && !dynamic(prefix, suffix)) missing.push(`${rel}: \`${body}\` (nothing starts with ${prefix} and ends with ${suffix || 'anything'})`);
    }
  }
  return missing;
}

test('every literal text key in the page exists in en.json', () => {
  const missing = scan();
  assert.deepEqual(missing, [], `${missing.length} missing keys:\n${missing.join('\n')}`);
});

test('the scan sees what it should (it finds the keys the page uses)', () => {
  let n = 0;
  for (const file of walk(SRC)) n += [...readFileSync(file, 'utf8').matchAll(/(?<![\w.$])t\(\s*["`']/g)].length;
  assert.ok(n > 1500, `only ${n} t() calls found`);
});

// keys built from a list the code loops over: every member of the list needs its text
const read = (f) => readFileSync(join(SRC, f), 'utf8');
const union = (src, name) => [...(new RegExp(`type ${name} =([^;]*);`).exec(src)?.[1] ?? '').matchAll(/"([\w-]+)"/g)].map((m) => m[1]);

test('every Preferences tab has its name and its subtitle', () => {
  const tabs = union(read('Preferences.tsx'), 'PrefsTab');
  assert.ok(tabs.length >= 10, `only ${tabs.length} tabs found`);
  const missing = tabs.flatMap((id) => [`prefs.tab.${id}`, `prefs.section.${id}.desc`]).filter((k) => !exists(k));
  assert.deepEqual(missing, []);
});

test('every People section and every kind of Undo line has its text', () => {
  const sections = [...(/SECTIONS: Section\[\] = \[([^\]]*)\]/.exec(read('peopleModel.ts'))?.[1] ?? '').matchAll(/"(\w+)"/g)].map((m) => m[1]);
  const undo = union(read('peopleModel.ts'), 'UndoKind');
  assert.ok(sections.length === 4 && undo.length >= 10, `${sections.length} sections, ${undo.length} undo kinds`);
  const missing = [...sections.map((s) => `people.section.${s}`), ...undo.map((u) => `people.undo.${u}`)].filter((k) => !exists(k));
  assert.deepEqual(missing, []);
});

test('every menu action has its menu text', () => {
  const actions = [...(/export const ACTION_IDS: readonly ActionId\[\] = \[([^\]]*)\]/.exec(read('menu.ts'))?.[1] ?? '').matchAll(/"([\w-]+)"/g)].map((m) => m[1]);
  assert.ok(actions.length > 40, `${actions.length} actions`);
  const items = new Set([...read('menu.ts').matchAll(/item\("([\w-]+)"/g)].map((m) => m[1]));
  const missing = actions.filter((a) => items.has(a)).map((a) => `menu.${a}`).filter((k) => !exists(k));
  assert.deepEqual(missing, []);
});

// ---- keys held as data: label: "viewTabs.people", read later by t(tab.label) ----
// Any string literal whose shape is a key and whose first segment is a top-level prefix of en.json must be a key (or a
// namespace of keys, or a plural base). Literals that only look like keys are listed here with the reason.
const prefixes = new Set(keys.map((k) => k.split('.')[0]));
const ALLOW = new Map([
  // (file-relative path: literal) -> why it is not a key
]);
const SHAPE = /^[a-z][A-Za-z0-9]*(?:\.[A-Za-z0-9_-]+)+$/;
const isTest = (f) => /\.test\.|\/tests?\//.test(f);

function literals() {
  const out = [];
  for (const file of walk(SRC)) {
    const rel = file.slice(SRC.length);
    if (isTest(rel)) continue;
    const src = readFileSync(file, 'utf8');
    for (const m of src.matchAll(/(["'])([A-Za-z][A-Za-z0-9]*(?:\.[A-Za-z0-9_-]+)+)\1/g)) {
      const lit = m[2];
      if (!SHAPE.test(lit) || !prefixes.has(lit.split('.')[0])) continue;
      out.push({ rel, lit });
    }
  }
  return out;
}

// a namespace: some key continues it ("library.find" before "library.find.rating")
const namespace = (lit) => keys.some((k) => k.startsWith(`${lit}.`) || k.startsWith(`${lit}_`));

test('every string in the code that names a text key is one (keys held as data, read later by t(x))', () => {
  const found = literals();
  const bad = found.filter(({ rel, lit }) => !exists(lit) && !namespace(lit) && !ALLOW.has(`${rel}: ${lit}`))
    .map(({ rel, lit }) => `${rel}: ${lit}`);
  assert.ok(found.length > 1500, `only ${found.length} key-shaped literals found`);
  assert.deepEqual([...new Set(bad)], [], `${new Set(bad).size} literals look like keys but are not:\n${[...new Set(bad)].join('\n')}`);
});

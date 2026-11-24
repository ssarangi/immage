// SPDX-License-Identifier: GPL-3.0-or-later
// faceOverlayModel.ts: where a face box goes on the displayed picture, the order and names of the boxes.
import test from 'node:test';
import assert from 'node:assert/strict';
import { boxStyle, faceAria, faceAt, faceTitle, labelSide, orderFaces } from '../app/src/faceOverlayModel.ts';

const f = (face, x, y, w, h, o = {}) => ({ face, x, y, w, h, person: null, name: null, state: 'unassigned', confidence: 0.9, ...o });
const tr = (k, v = {}) => ({ 'people.face.unnamed': 'Unnamed face', 'people.suggest.maybe': `Maybe ${v.name}`, 'people.face.aria': `Face ${v.i} of ${v.n}, ${v.name}` }[k] ?? k);

test('a box is a percentage of the displayed picture, kept inside it', () => {
  assert.deepEqual(boxStyle(f(1, 0.25, 0.1, 0.5, 0.2)), { left: '25%', top: '10%', width: '50%', height: '20%' });
  assert.deepEqual(boxStyle(f(1, 0.9, 0.9, 0.5, 0.5)), { left: '90%', top: '90%', width: '10%', height: '10%' });   // cut at the edge
  assert.deepEqual(boxStyle(f(1, -0.2, 2, 0, NaN)), { left: '0%', top: '100%', width: '0.6%', height: '0%' });
  assert.equal(boxStyle(f(1, 0.1, 0.1, 0, 0)).width, '0.6%');                                                     // never vanishes
  // the same fractions give the same percentages whatever the zoom: the layer scales, not the boxes
  assert.deepEqual(boxStyle(f(1, 1 / 3, 0.5, 0.25, 0.25)), { left: '33.33%', top: '50%', width: '25%', height: '25%' });
});

test('faces are ordered left to right, rejected ones left out', () => {
  const list = [f(3, 0.6, 0.1, 0.1, 0.1), f(1, 0.1, 0.5, 0.1, 0.1), f(2, 0.1, 0.2, 0.1, 0.1), f(4, 0.3, 0.3, 0.1, 0.1, { state: 'rejected' })];
  assert.deepEqual(orderFaces(list).map((x) => x.face), [2, 1, 3]);
});

test('names: the person, Maybe for a suggestion, nobody yet; the aria label says where the face is', () => {
  assert.equal(faceTitle(f(1, 0, 0, 1, 1, { state: 'confirmed', name: 'Maya' }), tr), 'Maya');
  assert.equal(faceTitle(f(1, 0, 0, 1, 1, { state: 'suggested', name: 'Maya' }), tr), 'Maybe Maya');
  assert.equal(faceTitle(f(1, 0, 0, 1, 1), tr), 'Unnamed face');
  assert.equal(faceTitle(f(1, 0, 0, 1, 1, { state: 'confirmed', name: null }), tr), 'Unnamed face');
  assert.equal(faceAria(f(1, 0, 0, 1, 1, { state: 'confirmed', name: 'Dad' }), 1, 4, tr), 'Face 2 of 4, Dad');
});

test('the label goes below a box at the top edge; the smallest box under a point wins', () => {
  assert.equal(labelSide(f(1, 0.1, 0.5, 0.1, 0.1)), 'above');
  assert.equal(labelSide(f(1, 0.1, 0.02, 0.1, 0.1)), 'below');
  const big = f(1, 0.1, 0.1, 0.6, 0.6), small = f(2, 0.3, 0.3, 0.1, 0.1);
  assert.equal(faceAt([big, small], 0.35, 0.35).face, 2);
  assert.equal(faceAt([big, small], 0.6, 0.6).face, 1);
  assert.equal(faceAt([big, small], 0.9, 0.9), null);
});

const { fromBackend } = await import('../app/src/faceOverlayModel.ts');

test('a reply shaped as faces_photo_faces answers: boxes kept, a face without a box left out, odd states made unassigned', () => {
  // one real row of a RAW photo: fractions of the displayed picture, whatever the scan's 1024 x 684 size was
  const reply = [
    { face: 36, x: 0.3125, y: 0.2412, w: 0.1328, h: 0.2, person: null, name: null, state: 'unassigned', detector_confidence: 0.93, eyes: null },
    { face: 37, x: 0.6, y: 0.3, w: 0.1, h: 0.15, person: 2, name: 'Maya', state: 'confirmed', detector_confidence: null, eyes: null },
    { face: 38, x: null, y: null, w: null, h: null, person: null, name: null, state: 'unassigned', detector_confidence: 0.5, eyes: null },
    { face: 39, x: 0.1, y: 0.1, w: 0, h: 0.1, person: null, name: null, state: 'unassigned', detector_confidence: 0.5, eyes: null },
    { face: 40, x: 0.8, y: 0.1, w: 0.1, h: 0.1, person: null, name: null, state: 'weird', detector_confidence: 0.5, eyes: null },
  ];
  const got = fromBackend(reply);
  assert.deepEqual(got.map((g) => g.face), [36, 37, 40]);
  assert.deepEqual(got[0], { face: 36, x: 0.3125, y: 0.2412, w: 0.1328, h: 0.2, person: null, name: null, state: 'unassigned', confidence: 0.93 });
  assert.equal(got[1].confidence, 0);
  assert.equal(got[2].state, 'unassigned');
  assert.deepEqual(boxStyle(got[0]), { left: '31.25%', top: '24.12%', width: '13.28%', height: '20%' });
});

test('the Shift+F action reads a ref, not the value of the render that built the key handler', async () => {
  const { readFileSync } = await import('node:fs');
  const app = readFileSync(new URL('../app/src/App.tsx', import.meta.url), 'utf8');
  const line = app.split('\n').find((l) => l.includes('"faces": () =>'));
  assert.ok(line, 'no faces action');
  assert.match(line, /facesAvailableRef\.current/);
  assert.doesNotMatch(line, /\bfacesAvailable\b(?!Ref)/);
});

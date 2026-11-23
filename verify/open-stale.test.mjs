// SPDX-License-Identifier: GPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
const require=createRequire(new URL('../app/package.json',import.meta.url));
const {transformSync}=await import(require.resolve('rolldown/experimental'));
// Opening photo B while photo A's late work (a copy render that finishes after
// close began, seconds on a slow machine) is still arriving: A must never be
// drawn, and A's geometry must not outlive its pixels.
const source=readFileSync(new URL('../app/src/App.tsx',import.meta.url),'utf8');
const ref=current=>({current});
const compile=(name,text)=>transformSync(name,text).code;
function slice(from,to){
  const a=source.indexOf(from),b=source.indexOf(to,a);
  assert.ok(a>=0&&b>a,from);
  return source.slice(a,b);
}
const copyText=slice('      s.onCopy = (f) => {','      s.thumbSize =');
function copyFixture(text=copyText){
  const calls=[],pending=[],s={},token={};
  const env={s,token,frameOwner:ref(token),session:ref(s),copyCanvas:ref({getContext(){return {drawImage(){calls.push('pixels');}};}}),
    copyGen:ref(0),copyMeta:ref(null),placed:f=>f,placeLayers(){calls.push('place');},setCopyOn(on){calls.push(on);},
    ImageData:class {},createImageBitmap(){return new Promise(r=>pending.push(r));},
    perf:{begin(){return 1;},finish(){},drawStart(){},drawEnd(){},discard(_t,reason){calls.push(reason);}}};
  new Function('env',`with(env) { ${compile('copy.ts',text)}; }`)(env);
  return {env,s,calls,pending,f:{width:1,height:1,pixels:new Uint8ClampedArray(4),trace:{}}};
}
const tick=()=>new Promise(setImmediate);
test('a copy of the previous photo arriving during the next open is not drawn',async()=>{
  const {env,s,calls,pending,f}=copyFixture();
  env.frameOwner.current={};            // photo B's open began; session.current is still A
  s.onCopy(f);await tick();
  assert.equal(pending.length,0);assert.deepEqual(calls,[]);
  assert.equal(env.copyMeta.current,null);
});
test('a copy bitmap still pending when the next open begins is dropped',async()=>{
  const {env,s,calls,pending,f}=copyFixture();
  s.onCopy(f);                           // A's own open, still the owner
  env.frameOwner.current={};             // then B's open begins
  pending[0]({close(){calls.push('close');}});await tick();
  assert.deepEqual(calls,['close','copy-bitmap-obsolete']);
});
test('negative control: without the open-owner guard the late copy draws over the new photo',async()=>{
  const unguarded=copyText.replace('if (frameOwner.current !== token) return;','')
    .replace('frameOwner.current !== token || ','');
  assert.notEqual(unguarded,copyText);
  const {env,s,calls,pending,f}=copyFixture(unguarded);
  env.frameOwner.current={};
  s.onCopy(f);pending[0]({close(){}});await tick();
  assert.ok(calls.includes('pixels'));assert.ok(calls.includes(true));
});
test('showOpening drops the old photo geometry along with its pixels',()=>{
  const text=slice('  const showOpening = (id: number | null) => {','  // `imgid` picks one of the file');
  const calls=[],mk=()=>({width:5,getContext(){return {drawImage(){}};}});
  const env={resetCanvases(){calls.push('reset');},
    baseCanvas:ref(mk()),clipBase:ref(mk()),copyCanvas:ref(mk()),clipSharp:ref(mk()),navThumb:ref(mk()),canvas:ref(mk()),
    full:ref({w:1,h:1}),viewZ:ref(2),baseMeta:ref({}),sharpMeta:ref({}),copyMeta:ref({}),copyGen:ref(0),
    setCopyOn(v){calls.push(['copyOn',v]);},overviewHash:ref('x'),baseStale:ref(false),
    histograms:{set(v){calls.push(['hist',v]);}},loader:ref({get(){return undefined;}}),
    window:{devicePixelRatio:1},viewport:()=>({w:100,h:100}),MARGIN:1,placeCanvas(){}};
  const fn=new Function('env',`with(env) { ${compile('open.ts',text)}; return showOpening; }`)(env);
  fn(7);
  assert.equal(env.full.current,null);assert.equal(env.viewZ.current,0);
  assert.equal(env.sharpMeta.current,null);assert.equal(env.baseMeta.current,null);assert.equal(env.copyMeta.current,null);
  assert.equal(env.copyGen.current,1);assert.equal(env.navThumb.current.width,0);
  assert.deepEqual(calls.filter(c=>c[0]==='copyOn'||c[0]==='hist'),[['copyOn',false],['hist',null]]);
});

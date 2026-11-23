// SPDX-License-Identifier: GPL-3.0-or-later
// Closing the photo being left (the first step of opening the next) waits only for work that changes
// the photo. Its idle base, copy, fitted frame and a plain re-render are not asked for or waited on,
// and a base already rendering is stopped rather than waited out.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire, registerHooks } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const require=createRequire(new URL('../app/package.json',import.meta.url));
const {transformSync}=await import(require.resolve('rolldown/experimental'));
registerHooks({
  resolve(spec,ctx,next){
    if(spec==='@tauri-apps/api/core')return {url:'data:text/javascript,export async function invoke(c){(globalThis.__calls??=[]).push(c);return {};}',shortCircuit:true};
    if(spec==='react')return next(require.resolve('react'),ctx);
    if(spec.startsWith('./')||spec.startsWith('../'))for(const ext of ['.ts','.tsx']){
      const url=new URL(spec+ext,ctx.parentURL);if(existsSync(url))return next(url.href,ctx);
    }
    return next(spec,ctx);
  },
  load(url,ctx,next){
    if(!url.startsWith('file:')||!/\.tsx?$/.test(url))return next(url,ctx);
    const path=fileURLToPath(url),out=transformSync(path,readFileSync(path,'utf8'),{jsx:{runtime:'automatic'}});
    return {format:'module',source:out.code,shortCircuit:true};
  },
});
globalThis.localStorage={getItem:()=>null,setItem:()=>{}};
globalThis.window=globalThis;
globalThis.requestAnimationFrame=cb=>setTimeout(()=>cb(0),0);
globalThis.cancelAnimationFrame=clearTimeout;
const {Session}=await import('../app/src/session.ts');
const frame=(view)=>({width:1,height:1,fullWidth:1,fullHeight:1,x:0,y:0,view,pixels:new Uint8ClampedArray(4),ms:1,hash:1n});
const wait=async(cond)=>{for(let i=0;i<500&&!cond();i++)await new Promise(r=>setTimeout(r,2));assert.ok(cond());};
function session(){
  const shown=[],s=new Session(()=>({width:1,height:1}),f=>shown.push(f),e=>{throw Error(e);});
  const renders=[];
  s.renderView=async(view,kind)=>{renders.push(kind);return frame(view);};
  globalThis.__calls=[];
  return {s,renders,shown};
}
test('closing an idle photo renders nothing',async()=>{
  const {s,renders}=session();
  await s.close();
  assert.deepEqual(renders,[]);
});
test('queued base, copy and fitted-frame work is forgotten by close',async()=>{
  const {s,renders}=session();
  s.onThumb=()=>assert.fail('a base painted after close');
  s.needThumb=true;s.needCopy=true;s.fitView={width:1,height:1};s.needWarm=true;
  await s.close();
  assert.deepEqual(renders,[]);
});
test('a base already rendering is stopped, and its frame is not painted',async()=>{
  const {s}=session();let release;
  s.onThumb=()=>assert.fail('a base painted after close');
  s.renderView=()=>new Promise(r=>{release=()=>r(frame({width:1,height:1}));});
  s.requestThumb();await wait(()=>s.baseRendering);
  const closing=s.close();
  await wait(()=>globalThis.__calls.includes('cancel_base'));
  release();await closing;
  assert.equal(s.idle,true);
});
test('an edit not yet applied still renders before close finishes',async()=>{
  const {s,renders}=session();
  s.needRender=true;
  await s.close();
  assert.ok(renders.length>=1);
});
test('negative control: the old close rendered the photo again even when nothing had changed',async()=>{
  const {s,renders}=session();
  s.requestRender();                       // what close() used to do first
  await s.settled();
  assert.equal(renders.length,1);
});

// portrait photo A is being left for landscape photo B: nothing of A may reach the viewer once B's
// open has begun (the recorded overlap: A's fitted frame, 2000x3000, under B's 3000x2000)
const sized=(w,h)=>(view)=>({width:w,height:h,fullWidth:w,fullHeight:h,x:0,y:0,view,pixels:new Uint8ClampedArray(4),ms:1,hash:BigInt(w)});
function pair(retire){
  const painted=[];
  const mk=(w,h,name)=>{
    const s=new Session(()=>({width:1,height:1}),f=>painted.push([name,'frame',f.fullWidth,f.fullHeight]),e=>{throw Error(e);});
    s.onThumb=f=>painted.push([name,'base',f.fullWidth,f.fullHeight]);
    s.onCopy=f=>f&&painted.push([name,'copy',f.fullWidth,f.fullHeight]);
    s.releases=[];
    s.renderView=(view)=>new Promise(r=>s.releases.push(()=>r(sized(w,h)(view))));
    return s;
  };
  return {painted,A:mk(2000,3000,'A'),B:mk(3000,2000,'B')};
}
test('late base, copy and fitted frames of the portrait photo are dropped once the landscape photo opens',async()=>{
  const {painted,A,B}=pair();
  A.requestRender();await wait(()=>A.releases.length===1);
  A.retire();                              // B's open begins
  A.releases[0]();                         // A's fitted frame lands late
  await new Promise(r=>setTimeout(r,30));
  A.requestThumb();await wait(()=>A.releases.length>=1);
  B.requestRender();await wait(()=>B.releases.length===1);B.releases[0]();
  await wait(()=>painted.some(p=>p[0]==='B'));
  assert.deepEqual(painted.filter(p=>p[0]==='A'),[]);
  assert.deepEqual(painted.filter(p=>p[0]==='B').map(p=>p.slice(2)),[[3000,2000]]);
});
test('negative control: without retire the portrait photo\'s late frame is painted',async()=>{
  const {painted,A}=pair();
  A.requestRender();await wait(()=>A.releases.length===1);
  A.releases[0]();
  await wait(()=>painted.length>0);
  assert.deepEqual(painted[0],['A','frame',2000,3000]);
});

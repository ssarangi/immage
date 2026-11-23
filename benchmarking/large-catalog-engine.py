#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Engine side of a large folder, headless, with temporary state and
generated photos: import throughput (the batches open_folder sends), thumbnail
throughput on the pool, and the latency of opening one photo (open, render)
while thumbnails and imports run. Never reads real photos.

usage: large-catalog-engine.py WORKDIR [--count N] [--threads T] [--size WxH]
"""
import argparse, json, os, random, selectors, subprocess, sys, threading, time
from pathlib import Path
from PIL import Image

ap = argparse.ArgumentParser()
ap.add_argument('work'); ap.add_argument('--count', type=int, default=10000)
ap.add_argument('--threads', type=int, default=2); ap.add_argument('--size', default='1600x1066')
ap.add_argument('--thumbs', type=int, default=600); ap.add_argument('--engine', default='build/bin/darktable-engine')
a = ap.parse_args()
work = Path(a.work); photos = work / 'photos'
W, H = map(int, a.size.split('x'))
if not (photos / 'done').exists():
    photos.mkdir(parents=True, exist_ok=True)
    rnd = random.Random(1)
    bases = []
    for k in range(40):
        im = Image.effect_noise((W, H), 40).convert('RGB')
        p = work / f'base{k}.jpg'; im.save(p, quality=88); bases.append(p.read_bytes()); p.unlink()
    for i in range(a.count):
        d = photos / f'f{i % 32:02d}'; d.mkdir(exist_ok=True)
        f = d / f'IMG_{i:06d}.jpg'; f.write_bytes(bases[i % 40] + bytes([i & 255, (i >> 8) & 255]))
        os.utime(f, (1.7e9 + i * 60, 1.7e9 + i * 60))
    (photos / 'done').write_text('1')
files = sorted(str(p) for p in photos.rglob('*.jpg'))[:a.count]
print(f'{len(files)} files, {W}x{H}', flush=True)

args = [a.engine, '--core', '--configdir', str(work / 'config'), '--cachedir', str(work / 'cache'),
        '--conf', 'opencl=false', '--conf', 'resourcelevel=0']
env = os.environ | {'XDG_DATA_HOME': str(work / 'data')}
log = open(work / 'engine.log', 'w')
ch = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, env=env)
lock = threading.Lock(); waiters = {}
def reader():
    for line in ch.stdout:
        try: r = json.loads(line)
        except Exception: continue
        key = r.get('tag', ('rid', r.get('rid')))
        with lock: w = waiters.get(key)
        if w: w[1].append((time.monotonic(), r)); w[0].set() if r.get('ok') is not None or 'rid' in r else None
threading.Thread(target=reader, daemon=True).start()
seq = [100]
def call(req, tagged=False, timeout=600):
    with lock:
        seq[0] += 1; n = seq[0]
        key = n if tagged else ('rid', n)
        req = dict(req, **({'tag': n} if tagged else {'rid': n}))
        ev = threading.Event(); waiters[key] = (ev, [])
    t0 = time.monotonic()
    ch.stdin.write(json.dumps(req) + '\n'); ch.stdin.flush()
    if not ev.wait(timeout): raise SystemExit(f'timeout {req["cmd"]}')
    t1, r = waiters[key][1][-1]
    return t1 - t0, r
call({'cmd': 'version'})

# 1. import, as open_folder does: adaptive batches 8..64
t0 = time.monotonic(); done = 0; size = 8; ids = {}
while done < len(files):
    batch = files[done:done + size]; t = time.monotonic()
    dt, r = call({'cmd': 'import', 'files': batch, 'job': 1, 'budget': 400}, tagged=True)
    for im in r.get('images', []): ids[im['path']] = im['imgid']
    n = r.get('done', len(batch)); done += max(n, 1)
    took = time.monotonic() - t
    size = min(size * 2, 64) if took < 0.1 else max(size // 2, 1) if took > 0.4 else size
    if done % 2000 < size: print(f'  import {done}/{len(files)} {done / (time.monotonic() - t0):.0f}/s', flush=True)
imp = time.monotonic() - t0
print(f'IMPORT {len(ids)} photos in {imp:.1f} s = {len(ids) / imp:.0f}/s', flush=True)
order = [ids[f] for f in files if f in ids]

def one_photo(path, label):
    t = time.monotonic()
    dt, r = call({'cmd': 'open', 'path': path})
    t_open = time.monotonic() - t
    dt2, r2 = call({'cmd': 'render', 'width': 1000, 'height': 700, 'rgba': str(work / 'frame.rgba')})
    return t_open, dt2

# 2. open one photo, idle
opens = [one_photo(files[i * 7], 'idle') for i in range(5)]
print('OPEN idle   open %.0f ms render %.0f ms (median of 5)' % tuple(1000 * sorted(x[k] for x in opens)[2] for k in (0, 1)), flush=True)

# 3. thumbnails: a stream with `threads` in flight as ThumbLoader does
stop = threading.Event(); count = [0]
def stream():
    q = iter(order[:a.thumbs] if a.thumbs else order)
    inflight = threading.Semaphore(a.threads)
    def go(imgid):
        try: call({'cmd': 'thumb', 'imgid': imgid, 'size': 240, 'out': str(work / f't{imgid % 64}.rgba'), 'foreground': False}, tagged=True); count[0] += 1
        finally: inflight.release()
    for imgid in q:
        if stop.is_set(): break
        inflight.acquire(); threading.Thread(target=go, args=(imgid,), daemon=True).start()
    for _ in range(a.threads): inflight.acquire()
th = threading.Thread(target=stream); t0 = time.monotonic(); th.start()
time.sleep(3)
busy = [one_photo(files[100 + i * 11], 'busy') for i in range(5)]
print('OPEN during thumbnails   open %.0f ms render %.0f ms (median of 5), all: %s' % (
    *(1000 * sorted(x[k] for x in busy)[2] for k in (0, 1)), [(round(1000 * x[0]), round(1000 * x[1])) for x in busy]), flush=True)
th.join(); el = time.monotonic() - t0
print(f'THUMBS {count[0]} at {a.threads} threads in {el:.1f} s = {count[0] / el:.1f}/s', flush=True)
ch.stdin.close(); ch.kill()

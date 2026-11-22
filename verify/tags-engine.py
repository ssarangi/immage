#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""RAM++ through the real engine (engine/tags.c): inference parity with onnxruntime in Python,
speed and memory per photo, and the keyword write (tags_apply). Needs the installed pack
(IMMAGE_RAM_ROOT=<runtime>/ram-models/<revision>) and photos (IMMAGE_TEST_PHOTOS=<folder of jpegs>);
prints SKIP without them. Disposable profile, never the user's library.

  IMMAGE_RAM_ROOT=... IMMAGE_TEST_PHOTOS=... python3 verify/tags-engine.py [--no-parity]
"""
import json, os, selectors, struct, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAM = os.environ.get("IMMAGE_RAM_ROOT")
PHOTOS = os.environ.get("IMMAGE_TEST_PHOTOS")
if not RAM or not PHOTOS or not (Path(RAM) / "model.onnx").is_file():
    print("SKIP set IMMAGE_RAM_ROOT (an installed RAM++ pack) and IMMAGE_TEST_PHOTOS")
    sys.exit(0)
from PIL import Image  # noqa: E402
import numpy as np  # noqa: E402

EDGE = 384
MEAN = np.array([0.485, 0.456, 0.406], dtype=np.float32)
STD = np.array([0.229, 0.224, 0.225], dtype=np.float32)
tags = (Path(RAM) / "ram_tag_list.txt").read_text().splitlines()
thresholds = [float(x) for x in (Path(RAM) / "ram_tag_list_threshold.txt").read_text().split()]
assert len(tags) == len(thresholds) == 4585


def rss_kb(pid):
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    return 0


def tensor_of(jpeg):
    # the same steps as app/src-tauri/src/ram_models.rs preprocess: bilinear to 384, ImageNet mean/std, CHW
    image = Image.open(jpeg).convert("RGB").resize((EDGE, EDGE), Image.BILINEAR)
    a = (np.asarray(image, dtype=np.float32) / 255.0 - MEAN) / STD
    return np.ascontiguousarray(a.transpose(2, 0, 1))


def select(logits):
    p = 1.0 / (1.0 + np.exp(-np.clip(logits, -30, 30)))
    found = [(tags[i], float(p[i])) for i in range(4585) if p[i] >= thresholds[i]]
    return sorted(found, key=lambda t: -t[1])


# the engine finds ONNX Runtime beside darktable's plugins in a package; a build tree names it
ORT = os.environ.get("IMMAGE_TEST_ORT", str(ROOT / "build/_deps/onnxruntime/lib/libonnxruntime.so"))

with tempfile.TemporaryDirectory(prefix="immage-tags-engine-") as temp:
    work = Path(temp)
    log = (work / "engine.log").open("w")
    child = subprocess.Popen(
        [str(ROOT / "build/bin/darktable-engine"), "--core", "--configdir", str(work / "config"), "--cachedir", str(work / "cache"),
         "--conf", "opencl=false", "--conf", "resourcelevel=0", "--conf", "plugins/ai/enabled=true", "--conf", f"plugins/ai/ort_library_path={ORT}", "-d", "ai"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True,
        env=os.environ | {"XDG_DATA_HOME": str(work / "data"), "OMP_NUM_THREADS": "4"})
    sel = selectors.DefaultSelector()
    sel.register(child.stdout, selectors.EVENT_READ)
    pending = bytearray()
    counter = 0

    def receive():
        while b"\n" not in pending:
            assert sel.select(300), "engine response timed out"
            block = os.read(child.stdout.fileno(), 1 << 20)
            assert block, "engine exited"
            pending.extend(block)
        line, _, rest = pending.partition(b"\n")
        pending[:] = rest
        return json.loads(line)

    def ask(tagged=False, **req):
        global counter
        counter += 1
        if tagged:
            req["tag"] = 10_000 + counter
        else:
            req["rid"] = counter
        child.stdin.write(json.dumps(req) + "\n")
        child.stdin.flush()
        reply = receive()
        assert reply.get("tag" if tagged else "rid") == req["tag" if tagged else "rid"], reply
        return reply

    try:
        sess = None
        if "--no-parity" not in sys.argv:
            import onnxruntime as ort
            sess = ort.InferenceSession(str(Path(RAM) / "model.onnx"), providers=["CPUExecutionProvider"])
        rows = []
        for photo in sorted(Path(PHOTOS).glob("*.jpg")):
            copy = work / photo.name
            copy.write_bytes(photo.read_bytes())
            opened = ask(cmd="open", path=str(copy))
            assert opened.get("ok"), opened
            imgid = opened["imgid"]
            ask(cmd="baseline_end")
            out = work / "preview.jpg"
            assert ask(True, cmd="analysis_preview", imgid=imgid, subject="original", out=str(out)).get("ok")
            t = tensor_of(out)
            input_file = work / "tensor.bin"
            input_file.write_bytes(t.astype("<f4").tobytes())
            started = time.time()
            reply = ask(True, cmd="tags_analyze", model_root=RAM, input=str(input_file))
            wall = time.time() - started
            assert reply.get("ok"), reply
            logits = np.array(reply["logits"], dtype=np.float32)
            assert logits.shape == (4585,) and np.isfinite(logits).all()
            found = select(logits)
            parity = ""
            if sess is not None:
                ref = sess.run(None, {"image": t[None]})[0][0]
                diff = float(np.max(np.abs(ref - logits)))
                assert diff < 1e-2, f"{photo.name}: engine and onnxruntime logits differ by {diff}"
                same = [n for n, _ in select(ref)] == [n for n, _ in found]
                parity = f" max|dlogit|={diff:.2e} same_tags={same}"
            rows.append((photo.name, reply["ms"], wall, rss_kb(child.pid), [f"{n} {c:.2f}" for n, c in found[:8]]))
            print(f"{photo.name}: infer {reply['ms']:.0f} ms (wall {wall * 1000:.0f} ms), engine RSS {rss_kb(child.pid) / 1024:.0f} MB, "
                  f"{len(found)} tags: {', '.join(rows[-1][4])}{parity}", flush=True)
            # the keyword write: added once, never twice, removed again; sidecar written
            names = [n for n, c in found if c >= 0.7][:3] or ["verify-keyword"]
            first = ask(cmd="tags_apply", imgid=imgid, add=names)
            assert first.get("ok") and sorted(first["added"]) == sorted(names), first
            again = ask(cmd="tags_apply", imgid=imgid, add=names)
            assert again.get("ok") and again["added"] == [], again
            meta = ask(cmd="meta", imgid=imgid)
            assert all(n in meta["image"]["keywords"] for n in names), meta["image"].get("keywords")
            xmp = Path(str(copy) + ".xmp")
            assert xmp.is_file() and all(n in xmp.read_text() for n in names), "keywords missing from the XMP sidecar"
            gone = ask(cmd="tags_apply", imgid=imgid, remove=names + ["never-attached"])
            assert sorted(gone["removed"]) == sorted(names), gone
            ask(cmd="close")
        # unloading frees the model; the next request loads it again
        assert ask(True, cmd="tags_analyze", unload=True).get("ok")
        after_unload = rss_kb(child.pid)
        again = ask(True, cmd="tags_analyze", model_root=RAM, input=str(work / "tensor.bin"))
        assert again.get("ok")
        times = sorted(r[1] for r in rows[1:]) or [rows[0][1]]
        print(f"PASS {len(rows)} photos; first (model load) {rows[0][1]:.0f} ms, then median {times[len(times) // 2]:.0f} ms per photo; "
              f"engine RSS peak {max(r[3] for r in rows) / 1024:.0f} MB, after unload {after_unload / 1024:.0f} MB")
    except BaseException:
        sys.stderr.write("".join((work / "engine.log").read_text().splitlines(True)[-25:]))
        raise
    finally:
        try:
            child.stdin.write(json.dumps({"cmd": "quit"}) + "\n")
            child.stdin.flush()
            child.wait(10)
        except Exception:
            child.kill()

#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""YuNet, SFace and (optionally) OCEC through the real engine (engine/faces.c): the engine's
outputs against onnxruntime in Python on the same tensors, and speed per call. Needs an installed
Faces pack folder (IMMAGE_FACES_ROOT=<folder with models/ and engine/>) and photos
(IMMAGE_TEST_PHOTOS=<folder of jpegs>); prints SKIP without them. Reads photos only, writes into a
temporary folder, prints counts and timings, never pictures or embeddings.
IMMAGE_EYES_ROOT=<folder with models/ocec_s.onnx and engine/eyes/config.json> adds the eye model.

  IMMAGE_FACES_ROOT=... IMMAGE_TEST_PHOTOS=... python3 verify/faces-engine.py
"""
import json, os, selectors, struct, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FACES = os.environ.get("IMMAGE_FACES_ROOT")
PHOTOS = os.environ.get("IMMAGE_TEST_PHOTOS")
DET = "face_detection_yunet/face_detection_yunet_2023mar.onnx"
REC = "face_recognition_sface/face_recognition_sface_2021dec.onnx"
if not FACES or not PHOTOS or not (Path(FACES) / "models" / DET).is_file():
    print("SKIP set IMMAGE_FACES_ROOT (a Faces pack folder) and IMMAGE_TEST_PHOTOS")
    sys.exit(0)
from PIL import Image  # noqa: E402
import numpy as np  # noqa: E402
import onnxruntime as ort  # noqa: E402

ORT = os.environ.get("IMMAGE_TEST_ORT", str(ROOT / "build/_deps/onnxruntime/lib/libonnxruntime.so"))
for folder, ident in (("detector", "immage-yunet"), ("recognizer", "immage-sface")):
    d = Path(FACES) / "engine" / folder
    d.mkdir(parents=True, exist_ok=True)
    (d / "config.json").write_text(json.dumps({"id": ident, "name": ident, "backend": "onnx", "num_inputs": 1}))


def detector_tensor(path):
    img = Image.open(path).convert("RGB")
    img.thumbnail((1024, 1024))
    s = min(640 / img.width, 640 / img.height)
    w, h = max(1, round(img.width * s)), max(1, round(img.height * s))
    small = img.resize((w, h), Image.BILINEAR)
    canvas = np.zeros((640, 640, 3), np.float32)
    ox, oy = (640 - w) // 2, (640 - h) // 2
    canvas[oy:oy + h, ox:ox + w] = np.asarray(small, np.float32)[:, :, ::-1]
    return np.ascontiguousarray(canvas.transpose(2, 0, 1))[None]


with tempfile.TemporaryDirectory(prefix="immage-faces-engine-") as temp:
    work = Path(temp)
    child = subprocess.Popen(
        [str(ROOT / "build/bin/darktable-engine"), "--core", "--configdir", str(work / "config"), "--cachedir", str(work / "cache"),
         "--conf", "opencl=false", "--conf", "plugins/ai/enabled=true", "--conf", f"plugins/ai/ort_library_path={ORT}"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=(work / "log").open("w"), text=True,
        env=os.environ | {"XDG_DATA_HOME": str(work / "data")})
    sel = selectors.DefaultSelector()
    sel.register(child.stdout, selectors.EVENT_READ)
    counter = 0

    def infer(root, ident, model, tensor):
        global counter
        counter += 1
        inp, out = work / "in.bin", work / "out.bin"
        inp.write_bytes(tensor.astype("<f4").tobytes())
        child.stdin.write(json.dumps({"cmd": "faces_infer", "tag": counter, "model_root": root, "model_id": ident, "model_file": model,
                                       "input": str(inp), "shape": list(tensor.shape), "output": str(out)}) + "\n")
        child.stdin.flush()
        assert sel.select(120), "engine response timed out"
        reply = json.loads(child.stdout.readline())
        assert reply["tag"] == counter, reply
        return reply, out

    try:
        # refusals first: a bad request never reaches a model
        for bad in ({"model_file": "../x.onnx"}, {"model_id": "a b"}, {"model_file": "a/b/c.onnx"}):
            req = {"cmd": "faces_infer", "tag": 99, "model_root": FACES, "model_id": "immage-yunet", "model_file": DET,
                   "input": str(work / "none"), "shape": [1], "output": str(work / "o")} | bad
            child.stdin.write(json.dumps(req) + "\n")
            child.stdin.flush()
            assert sel.select(30)
            assert json.loads(child.stdout.readline())["ok"] is False, bad
        det = ort.InferenceSession(str(Path(FACES) / "models" / DET), providers=["CPUExecutionProvider"])
        rec = ort.InferenceSession(str(Path(FACES) / "models" / REC), providers=["CPUExecutionProvider"])
        photos = sorted(p for p in Path(PHOTOS).iterdir() if p.suffix.lower() in (".jpg", ".jpeg"))[:12]
        assert photos, "no jpeg photos"
        times, worst = [], 0.0
        for p in photos:
            t = detector_tensor(p)
            started = time.time()
            reply, out = infer(FACES, "immage-yunet", DET, t)
            wall = time.time() - started
            assert reply["ok"], reply
            sizes = [int(np.prod(o["shape"])) for o in reply["outputs"]]
            flat = np.frombuffer(out.read_bytes(), "<f4")
            assert flat.size == sum(sizes)
            ref = det.run(None, {det.get_inputs()[0].name: t})
            at = 0
            for o, r in zip(reply["outputs"], ref):
                got = flat[at:at + r.size].reshape(r.shape)
                at += r.size
                assert o["name"] == det.get_outputs()[reply["outputs"].index(o)].name
                worst = max(worst, float(np.max(np.abs(got - r))))
            times.append((reply["ms"], wall * 1000))
        assert worst < 1e-3, f"engine and onnxruntime differ by {worst}"
        # SFace on a synthetic 112 x 112 face-like tensor: same answer as onnxruntime
        rng = np.random.default_rng(1)
        t = rng.uniform(0, 255, (1, 3, 112, 112)).astype(np.float32)
        reply, out = infer(FACES, "immage-sface", REC, t)
        assert reply["ok"], reply
        got = np.frombuffer(out.read_bytes(), "<f4")
        r = rec.run(None, {rec.get_inputs()[0].name: t})[0].ravel()
        assert got.shape == r.shape == (128,) and float(np.max(np.abs(got - r))) < 1e-3
        eyes = os.environ.get("IMMAGE_EYES_ROOT")
        eye_line = "eyes: skipped (no IMMAGE_EYES_ROOT)"
        if eyes and (Path(eyes) / "models" / "ocec_s.onnx").is_file():
            e = Path(eyes) / "engine" / "eyes"
            e.mkdir(parents=True, exist_ok=True)
            (e / "config.json").write_text(json.dumps({"id": "immage-ocec", "name": "ocec", "backend": "onnx", "num_inputs": 1}))
            t = rng.uniform(0, 1, (1, 3, 24, 40)).astype(np.float32)
            reply, out = infer(eyes, "immage-ocec", "ocec_s.onnx", t)
            assert reply["ok"], reply
            sess = ort.InferenceSession(str(Path(eyes) / "models" / "ocec_s.onnx"), providers=["CPUExecutionProvider"])
            ref = sess.run(None, {sess.get_inputs()[0].name: t})[0].ravel()
            got = np.frombuffer(out.read_bytes(), "<f4")
            assert float(np.max(np.abs(got - ref))) < 1e-4
            eye_line = f"eyes: parity ok, {reply['ms']:.1f} ms"
        # unload frees the models; the next call loads again
        counter += 1
        child.stdin.write(json.dumps({"cmd": "faces_infer", "tag": counter, "unload": True}) + "\n")
        child.stdin.flush()
        assert sel.select(30) and json.loads(child.stdout.readline())["ok"]
        reply, _ = infer(FACES, "immage-sface", REC, t if t.shape[1:] == (3, 112, 112) else rng.uniform(0, 255, (1, 3, 112, 112)).astype(np.float32))
        assert reply["ok"], reply
        ms = [m for m, _ in times]
        print(f"faces-engine: {len(photos)} photos, YuNet parity max|d|={worst:.1e}, "
              f"inference median {sorted(ms)[len(ms) // 2]:.0f} ms (cold first {ms[0]:.0f} ms), SFace parity ok, {eye_line}")
    finally:
        child.stdin.close()
        child.wait(30)

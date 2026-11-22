#!/usr/bin/python3
"""Recompute the aggregates from the actual files (exiftool + sidecars) and compare with expected.json and manifest.csv."""
import csv, json, re, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate as G

HERE = G.HERE
files = sorted(p for p in G.PHOTOS.rglob("*.jpg"))
out = subprocess.run(["exiftool", "-json", "-n", "-Make", "-Model", "-LensModel", "-FocalLengthIn35mmFormat", "-FNumber", "-ExposureTime",
                      "-ISO", "-DateTimeOriginal", "-Composite:GPSLatitude", "-Composite:GPSLongitude", "-ImageWidth", "-ImageHeight",
                      "-ExifImageWidth", "-ExifImageHeight", *map(str, files)], check=True, capture_output=True, text=True).stdout
rows = []
for e in json.loads(out):
    p = Path(e["SourceFile"]); rel = p.relative_to(G.PHOTOS).as_posix()
    x = (p.parent / (p.name + ".xmp")).read_text()
    rating = int(re.search(r'xmp:Rating="(-?\d)"', x).group(1))
    m = re.search(r"<darktable:colorlabels>.*?<rdf:li>(\d)</rdf:li>", x, re.S)
    d = e.get("DateTimeOriginal", "")
    assert e["ImageWidth"] == e.get("ExifImageWidth", e["ImageWidth"]) and e["ImageHeight"] == e.get("ExifImageHeight", e["ImageHeight"]), rel
    rows.append(dict(path=rel, make=e.get("Make", ""), model=e.get("Model", ""), lens=e.get("LensModel", ""),
                     focal35=e["FocalLengthIn35mmFormat"], aperture=e["FNumber"], iso=e["ISO"], shutter_s=e["ExposureTime"],
                     dt=(d[:4] + "-" + d[5:7] + "-" + d[8:10] + " " + d[11:19]) if d else "",
                     lat=e.get("GPSLatitude", ""), lon=e.get("GPSLongitude", ""), rating=rating,
                     label=G.LABELS[int(m.group(1))] if m else "", w=e["ImageWidth"], h=e["ImageHeight"]))
exp = json.loads((G.OUT / "expected.json").read_text())["aggregates"]
got = json.loads(json.dumps(G.aggregate(rows)))
bad = 0
for k in exp:
    if exp[k] != got[k]:
        bad += 1; print("MISMATCH", k); print("  expected", exp[k]); print("  actual  ", got[k])
# per-row check against manifest
man = {r["path"]: r for r in csv.DictReader(open(G.OUT / "manifest.csv"))}
assert set(man) == {r["path"] for r in rows}, "file set differs from manifest"
for r in rows:
    m = man[r["path"]]
    chk = [(m["make"], r["make"]), (m["model"], r["model"]), (m["lens"], r["lens"]), (m["dt"], r["dt"]),
           (int(m["iso"]), int(r["iso"])), (int(m["rating"]), r["rating"]), (m["label"], r["label"]),
           (int(m["w"]), r["w"]), (int(m["h"]), r["h"]), (int(m["focal35"]), int(r["focal35"]))]
    chk += [(abs(float(m["aperture"]) - r["aperture"]) < .006, True), (abs(float(m["shutter_s"]) - r["shutter_s"]) < 1e-6 + 1e-3 * r["shutter_s"], True),
            ((m["lat"] == "") == (r["lat"] == ""), True)]
    if m["lat"]: chk += [(abs(float(m["lat"]) - r["lat"]) < 1e-4, True), (abs(float(m["lon"]) - r["lon"]) < 1e-4, True)]
    for a, b in chk:
        if a != b: bad += 1; print("ROW MISMATCH", r["path"], a, b); break
print("files:", len(rows), "mismatches:", bad)
sys.exit(1 if bad else 0)

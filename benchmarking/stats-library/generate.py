#!/usr/bin/python3
"""Synthetic photo library for testing Immage's Statistics feature.

Deterministic (SEED) and idempotent: every run deletes photos/ and work/ and
rebuilds them, plus manifest.csv and expected.json. See README.md.
Needs: python3 with Pillow, exiftool, ImageMagick `convert`.
"""
import csv, datetime as dt, json, math, os, random, shutil, subprocess, sys
from collections import Counter, defaultdict
from pathlib import Path
from zoneinfo import ZoneInfo
from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFilter

SEED = 20260501
N = 400
HERE = Path(__file__).resolve().parent
# the generated library: IMMAGE_STATS_LIBRARY, else out/ beside this script (git-ignored)
OUT = Path(os.environ.get("IMMAGE_STATS_LIBRARY") or HERE / "out").resolve()
PHOTOS, WORK, SOURCES = OUT / "photos", OUT / "work", HERE / "sources"
LABELS = ["red", "yellow", "green", "blue", "purple"]  # darktable colour label 0..4

# ---------------------------------------------------------------- design data
D = dt.date
CAMERAS = [  # model, make, count share, active range, filename prefix/start, lens mix
    dict(model="Canon EOS R5", make="Canon", share=.35, start=D(2022,1,1), end=D(2025,6,30), prefix="IMG_", n0=1,
         iso_base=100, iso_max=12800, crop=1.0, soft="Version 1.8.1",
         lenses=[("RF24-105", .55), ("RF70-200", .30), ("RF50", .15)], rated=1.0),
    dict(model="ILCE-7M4", make="SONY", share=.25, start=D(2022,7,1), end=D(2026,11,20), prefix="DSC0", n0=1,
         iso_base=100, iso_max=12800, crop=1.0, soft="ILCE-7M4 v5.00",
         lenses=[("FE24-70", .60), ("FE85", .40)], rated=.95),
    dict(model="NIKON Z 6_2", make="NIKON CORPORATION", share=.15, start=D(2022,1,1), end=D(2024,2,29), prefix="DSC_", n0=1,
         iso_base=100, iso_max=12800, crop=1.0, soft="Ver.05.10",
         lenses=[("Z24-70", .65), ("Z50", .35)], rated=.8),
    dict(model="X-T5", make="FUJIFILM", share=.10, start=D(2023,5,1), end=D(2026,11,20), prefix="DSCF", n0=1,
         iso_base=125, iso_max=12800, crop=1.5, soft="Digital Camera X-T5 Ver1.10",
         lenses=[("XF16-55", .65), ("XF56", .35)], rated=1.0),
    dict(model="iPhone 15 Pro", make="Apple", share=.10, start=D(2023,10,1), end=D(2026,11,20), prefix="IMG_", n0=6000,
         iso_base=50, iso_max=3200, crop=None, soft="17.5.1",
         lenses=[("IPH-MAIN", .70), ("IPH-TELE", .30)], rated=.35),
    dict(model="Pixel 10 Pro XL", make="Google", share=.05, start=D(2025,9,1), end=D(2026,11,20), prefix="PXL_", n0=1,
         iso_base=50, iso_max=3200, crop=None, soft="HDR+ 1.0.7",
         lenses=[("PXL-MAIN", 1.0)], rated=.35),
]
CAM = {c["model"]: c for c in CAMERAS}

def L(key, name, make, fmin, fmax, amax, feq=None, fixed=False, day_mu=1.8, day_sd=1.0):
    return key, dict(key=key, name=name, make=make, fmin=fmin, fmax=fmax, amax=amax, feq=feq, fixed=fixed,
                     day_mu=day_mu, day_sd=day_sd)
LENSES = dict([
    L("RF24-105", "RF24-105mm F4 L IS USM", "Canon", 24, 105, 4.0, day_mu=1.6),
    L("RF70-200", "RF70-200mm F2.8 L IS USM", "Canon", 70, 200, 2.8, day_mu=0.7, day_sd=0.8),
    L("RF50", "RF50mm F1.8 STM", "Canon", 50, 50, 1.8, day_mu=1.4, day_sd=1.3),
    L("FE24-70", "FE 24-70mm F2.8 GM II", "Sony", 24, 70, 2.8, day_mu=2.0),
    L("FE85", "FE 85mm F1.8", "Sony", 85, 85, 1.8, day_mu=0.8, day_sd=0.9),
    L("Z24-70", "NIKKOR Z 24-70mm f/4 S", "Nikon", 24, 70, 4.0, day_mu=1.6),
    L("Z50", "NIKKOR Z 50mm f/1.8 S", "Nikon", 50, 50, 1.8, day_mu=1.3, day_sd=1.3),
    L("XF16-55", "XF16-55mmF2.8 R LM WR", "Fujifilm", 16, 55, 2.8, day_mu=2.0),
    L("XF56", "XF56mmF1.2 R", "Fujifilm", 56, 56, 1.2, day_mu=1.5, day_sd=1.5),
    L("IPH-MAIN", "iPhone 15 Pro back triple camera 6.765mm f/1.78", "Apple", 6.765, 6.765, 1.78, feq=24, fixed=True),
    L("IPH-TELE", "iPhone 15 Pro back triple camera 9mm f/2.8", "Apple", 9, 9, 2.8, feq=77, fixed=True),
    L("PXL-MAIN", "Pixel 10 Pro XL back camera 6.9mm f/1.68", "Google", 6.9, 6.9, 1.68, feq=25, fixed=True),
])
APS = [1.2,1.4,1.6,1.8,2,2.2,2.5,2.8,3.2,3.5,4,4.5,5,5.6,6.3,7.1,8,9,10,11,13,14,16]
ISOS = [50,64,80,100,125,160,200,250,320,400,500,640,800,1000,1250,1600,2000,2500,3200,4000,5000,6400,8000,10000,12800]
SHUTTERS = [30,25,20,15,13,10,8,6,5,4,3.2,2.5,2,1.6,1.3,1,.8,.6,.5,.4,.3] + [1/x for x in
    (4,5,6,8,10,13,15,20,25,30,40,50,60,80,100,125,160,200,250,320,400,500,640,800,1000,1250,1600,2000,2500,3200,4000,5000,6400,8000)]

def S(slug, tz, a, b, w, spots, folder=None):
    return dict(slug=slug, tz=tz, start=a, end=b, w=w, spots=spots, year=slug[:4])
SEA = [("Space Needle", 47.6205, -122.3493, .35, "America/Los_Angeles"),
       ("Gas Works Park", 47.6456, -122.3344, .30, "America/Los_Angeles"),
       ("Discovery Park", 47.6573, -122.4054, .25, "America/Los_Angeles"),
       ("Mount Rainier Paradise", 46.7860, -121.7350, .10, "America/Los_Angeles")]
def P(n, la, lo, w, tz): return (n, la, lo, w, tz)
FOLDERS = [
    S("2022-01-seattle-winter", None, D(2022,1,1), D(2022,1,31), 1.3, SEA),
    S("2022-07-dolomites", None, D(2022,7,9), D(2022,7,20), 1.0, [P("Tre Cime di Lavaredo",46.6186,12.3028,.6,"Europe/Rome"), P("Lago di Braies",46.6942,12.0856,.4,"Europe/Rome")]),
    S("2022-10-seattle-fall", None, D(2022,10,1), D(2022,10,31), 1.2, SEA),
    S("2022-11-kyoto", None, D(2022,11,12), D(2022,11,22), 1.1, [P("Fushimi Inari Taisha",34.9671,135.7727,.55,"Asia/Tokyo"), P("Kinkaku-ji",35.0394,135.7292,.45,"Asia/Tokyo")]),
    S("2023-03-marrakech", None, D(2023,3,10), D(2023,3,17), .9, [P("Jemaa el-Fnaa",31.6258,-7.9891,.6,"Africa/Casablanca"), P("Majorelle Garden",31.6417,-8.0033,.4,"Africa/Casablanca")]),
    S("2023-06-iceland", None, D(2023,6,18), D(2023,6,28), 1.1, [P("Hallgrimskirkja",64.1417,-21.9266,.4,"Atlantic/Reykjavik"), P("Skogafoss",63.5321,-19.5114,.6,"Atlantic/Reykjavik")]),
    S("2023-09-seattle", None, D(2023,9,1), D(2023,9,30), 1.3, SEA),
    S("2023-12-new-york", None, D(2023,12,20), D(2023,12,29), .9, [P("Brooklyn Bridge",40.7061,-73.9969,.55,"America/New_York"), P("Times Square",40.7580,-73.9855,.45,"America/New_York")]),
    S("2024-02-cape-town", None, D(2024,2,3), D(2024,2,12), .9, [P("Table Mountain",-33.9628,18.4098,.6,"Africa/Johannesburg"), P("Boulders Beach",-34.1976,18.4510,.4,"Africa/Johannesburg")]),
    S("2024-08-seattle-summer", None, D(2024,7,25), D(2024,8,31), 1.3, SEA),
    S("2024-09-remote", None, D(2024,9,1), D(2024,9,30), 0.0, []),  # only the 3 remote points
    S("2025-02-patagonia", None, D(2025,2,5), D(2025,2,16), .8, [P("Torres del Paine",-50.9423,-73.4068,1.0,"America/Punta_Arenas")]),
    S("2025-05-sydney", None, D(2025,5,10), D(2025,5,20), .8, [P("Sydney Opera House",-33.8568,151.2153,.6,"Australia/Sydney"), P("Bondi Beach",-33.8915,151.2767,.4,"Australia/Sydney")]),
    S("2025-08-lofoten", None, D(2025,8,2), D(2025,8,12), .8, [P("Reine, Lofoten",67.9323,13.0887,1.0,"Europe/Oslo")]),
    S("2025-10-seattle-fall", None, D(2025,10,1), D(2025,10,31), 1.3, SEA),
    S("2026-06-tokyo", None, D(2026,6,5), D(2026,6,15), 1.0, [P("Shibuya Crossing",35.6595,139.7005,.55,"Asia/Tokyo"), P("Senso-ji",35.7148,139.7967,.45,"Asia/Tokyo")]),
    S("2026-10-paris", None, D(2026,10,8), D(2026,10,16), 1.0, [P("Eiffel Tower",48.8584,2.2945,.55,"Europe/Paris"), P("Louvre",48.8606,2.3376,.45,"Europe/Paris")]),
    S("2026-11-seattle", None, D(2026,11,1), D(2026,11,20), 1.0, SEA),
]
REMOTE_FOLDER = "2024-09-remote"
REMOTE_SPOTS = [P("Salar de Uyuni",-20.1338,-67.4891,1,"America/La_Paz"), P("Svalbard (Longyearbyen)",78.2232,15.6267,1,"Arctic/Longyearbyen"),
                P("Wadi Rum",29.5764,35.4200,1,"Asia/Amman")]
REMOTE_CAMS = ["X-T5", "iPhone 15 Pro", "ILCE-7M4"]

HOUR_W = [.15,.15,.15,.15,.15,.4,2.0,3.0,2.2,1.2,1.2,1.3,1.4,1.3,1.2,1.3,1.8,3.0,4.0,3.5,1.5,.6,.4,.25]

# ---------------------------------------------------------------- bucket helpers (shared with verify.py)
def bucket_aperture(f):
    for hi, lab in [(1.8,"f/1.0-1.8"),(2.8,"f/2.0-2.8"),(4.0,"f/3.2-4"),(5.6,"f/4.5-5.6"),(8.0,"f/6.3-8"),(11.0,"f/9-11")]:
        if f <= hi + 0.005: return lab
    return "f/13+"
def bucket_iso(i):
    for hi, lab in [(100,"<=100"),(400,"101-400"),(1600,"401-1600"),(6400,"1601-6400")]:
        if i <= hi: return lab
    return ">6400"
def bucket_shutter(t):
    for hi, lab in [(1/1000,"<=1/1000"),(1/250,"1/1000-1/250"),(1/60,"1/250-1/60"),(1/8,"1/60-1/8"),(1.0,"1/8-1s")]:
        if t <= hi * (1 + 1e-4): return lab
    return ">1s"
def bucket_focal(f):
    for hi, lab in [(24,"<=24"),(35,"25-35"),(70,"36-70"),(135,"71-135"),(200,"136-200")]:
        if f <= hi: return lab
    return ">200"
def aspect_name(w, h):
    r = w / h
    for v, lab in [(1.5,"3:2"),(4/3,"4:3"),(16/9,"16:9"),(1.0,"1:1"),(2/3,"2:3")]:
        if abs(r - v) < 0.01: return lab
    return "other"

def aggregate(rows):
    """rows: dicts with path make model lens focal35 aperture iso shutter_s dt lat lon rating label w h.
    Empty string / None means missing."""
    c = lambda: Counter()
    A = {k: c() for k in ["per_camera","per_make","per_lens","per_camera_lens","per_year","per_year_month","per_hour",
                          "per_weekday","per_rating","per_label","aperture_buckets","iso_buckets","shutter_buckets",
                          "focal35_buckets","aspect_ratios"]}
    lens_rows = defaultdict(list)
    for r in rows:
        mk, md, ln = r["make"] or "Unknown", r["model"] or "Unknown", r["lens"] or "Unknown"
        A["per_camera"][md] += 1; A["per_make"][mk] += 1; A["per_lens"][ln] += 1
        A["per_camera_lens"][f"{md} | {ln}"] += 1
        A["per_rating"][str(int(r["rating"]))] += 1
        A["per_label"][r["label"] or "none"] += 1
        A["aperture_buckets"][bucket_aperture(float(r["aperture"]))] += 1
        A["iso_buckets"][bucket_iso(int(r["iso"]))] += 1
        A["shutter_buckets"][bucket_shutter(float(r["shutter_s"]))] += 1
        A["focal35_buckets"][bucket_focal(float(r["focal35"]))] += 1
        A["aspect_ratios"][aspect_name(int(r["w"]), int(r["h"]))] += 1
        if r["dt"]:
            d = r["dt"]
            A["per_year"][d[:4]] += 1; A["per_year_month"][d[:7]] += 1; A["per_hour"][d[11:13]] += 1
            A["per_weekday"][dt.date(int(d[:4]), int(d[5:7]), int(d[8:10])).strftime("%a")] += 1
        else:
            A["per_year"]["no-datetime"] += 1
        lens_rows[ln].append(r)
    out = {k: dict(sorted(v.items())) for k, v in A.items()}
    out["total_photos"] = len(rows)
    out["no_datetime"] = sum(1 for r in rows if not r["dt"])
    g = sum(1 for r in rows if r["lat"] not in ("", None) and r["lon"] not in ("", None))
    out["gps"] = dict(with_gps=g, without_gps=len(rows) - g, coverage_pct=round(100 * g / len(rows), 2))
    per = {}
    for ln, rs in sorted(lens_rows.items()):
        rated = [int(x["rating"]) for x in rs if int(x["rating"]) >= 1]
        five = sorted([x for x in rs if int(x["rating"]) == 5], key=lambda x: (x["dt"] or "", x["path"]), reverse=True)
        per[ln] = dict(photos=len(rs), rated=len(rated), avg_rating_of_rated=round(sum(rated) / len(rated), 3) if rated else None,
                       five_star=len(five), top10_five_star=[x["path"] for x in five[:10]])
    out["per_lens_rating"] = per
    return out

# ---------------------------------------------------------------- photo design
def weighted(rng, items):
    tot = sum(w for _, w in items); x = rng.random() * tot
    for it, w in items:
        x -= w
        if x <= 0: return it
    return items[-1][0]

def holidays(y):
    nov1 = D(y, 11, 1); thu = nov1 + dt.timedelta(days=(3 - nov1.weekday()) % 7 + 21)
    return {D(y,1,1), D(y,7,4), D(y,12,24), D(y,12,25), D(y,12,31), thu}

def day_weights(folder, days):
    home = "seattle" in folder["slug"]
    ws = []
    for d in days:
        w = 1.0
        if home:
            if d.weekday() >= 5: w *= 2.5
            if d in holidays(d.year): w *= 3
        ws.append(w)
    return ws

def snap(v, lst):
    return min(lst, key=lambda x: abs(math.log(x) - math.log(v)))

def design_exposure(rng, lens, cam, light, focal35, astro):
    phone = cam["crop"] is None
    if astro:
        N_ = lens["amax"]; iso = rng.choice([1600, 2000, 2500, 3200, 4000, 5000, 6400]); t = rng.choice([10, 13, 15, 20, 25, 30])
        return N_, iso, float(t)
    ev = dict(day=rng.uniform(12, 15), golden=rng.uniform(9, 12.5), blue=rng.uniform(6, 8.5),
              night=rng.uniform(0, 4.5), indoor=rng.uniform(5, 8))[light]
    if lens["fixed"]: N_ = lens["amax"]
    else:
        if light in ("night", "indoor", "blue"): s = abs(rng.gauss(0, .5))
        elif light == "golden": s = abs(rng.gauss(lens["day_mu"] - .7, lens["day_sd"]))
        else: s = max(0, rng.gauss(lens["day_mu"], lens["day_sd"]))
        N_ = snap(lens["amax"] * 2 ** (s / 2), [a for a in APS if a >= lens["amax"] - .01])
    b = cam["iso_base"]
    iso0 = {"day": b * rng.choice([1, 1, 1, 2]), "golden": b * rng.choice([1, 2, 4]),
            "blue": rng.choice([400, 800, 1600]), "night": rng.choice([1600, 3200, 3200, 6400]),
            "indoor": rng.choice([800, 1250, 1600])}[light]
    if phone: iso0 = {"day": 50, "golden": rng.choice([64, 100, 200]), "blue": rng.choice([200, 400, 800]),
                      "night": rng.choice([800, 1600]), "indoor": rng.choice([400, 640, 800])}[light]
    iso = snap(iso0, ISOS)
    tt = lambda n, i: n * n / (2 ** ev * i / 100)
    t = tt(N_, iso)
    tripod = (not phone) and light in ("night", "blue") and rng.random() < .12
    tmax = 1 / max(focal35 / 2, 8)
    while t > tmax and iso < cam["iso_max"] and not tripod:
        iso = ISOS[ISOS.index(iso) + 1]; t = tt(N_, iso)
    while t < 1 / 8000 and not lens["fixed"] and N_ < 16:
        N_ = APS[APS.index(N_) + 1]; t = tt(N_, iso)
    t = max(t, 1 / 8000)
    return N_, iso, snap(t, SHUTTERS)

def fmt_shutter(t):
    if t >= 0.29:
        return ("%g" % t)
    return "1/%d" % round(1 / t)

def build_design():
    rng = random.Random(SEED)
    counts = {c["model"]: round(N * c["share"]) for c in CAMERAS}
    assert sum(counts.values()) == N
    cams = [m for m, n in counts.items() for _ in range(n)]
    rng.shuffle(cams)
    folders = {f["slug"]: f for f in FOLDERS}
    photos = []
    remote_idx = {}
    for i, m in enumerate(cams):
        if m in REMOTE_CAMS and m not in remote_idx: remote_idx[m] = i
    remote_set = set(remote_idx.values())
    for i, m in enumerate(cams):
        cam = CAM[m]
        p = dict(i=i, model=m, remote=i in remote_set)
        p["lens"] = weighted(rng, cam["lenses"])
        if p["remote"]:
            p["folder"] = REMOTE_FOLDER
        else:
            feas = [(f, f["w"]) for f in FOLDERS if f["w"] > 0 and f["start"] <= cam["end"] and f["end"] >= cam["start"]]
            p["folder"] = weighted(rng, feas)["slug"]
        photos.append(p)
    # per-folder outing days (weekend/holiday bursts)
    outing = {}
    for f in FOLDERS:
        days = [f["start"] + dt.timedelta(n) for n in range((f["end"] - f["start"]).days + 1)]
        ws = day_weights(f, days)
        k = 4 if "seattle" in f["slug"] else 3
        outing[f["slug"]] = [days[j] for j in sorted(rng.choices(range(len(days)), weights=ws, k=k))] if days else []
    # flags
    non_remote = [p for p in photos if not p["remote"]]
    ilc = [p for p in non_remote if CAM[p["model"]]["crop"] is not None and LENSES[p["lens"]]["fmin"] <= 24]
    for p in rng.sample(ilc, 12): p["astro"] = True
    pool = [p for p in non_remote if not p.get("astro")]
    for p in rng.sample(pool, 24): p["soft"] = True
    pool2 = [p for p in pool if not p.get("soft")]
    for p in rng.sample(pool2, 20): p["dark"] = True
    for p in rng.sample([p for p in non_remote if not p.get("astro")], 12): p["nodate"] = True
    # missing make / lens
    for p in rng.sample(photos, 16): p["nomake"] = True
    for p in rng.sample(photos, 40): p["nolens"] = True
    # GPS: 70% exactly
    others = [p for p in photos if not p["remote"]]
    gw = {"Apple": 4.0, "Google": 4.0, "Canon": 1.3, "SONY": 1.0, "FUJIFILM": 1.0, "NIKON CORPORATION": .5}
    keyed = sorted(others, key=lambda p: -(rng.random() ** (1 / gw[CAM[p["model"]]["make"]])))
    want = round(.7 * N) - len(remote_set)
    gset = {p["i"] for p in keyed[:want]}
    for p in photos: p["gps"] = p["remote"] or p["i"] in gset
    # place, datetime, geometry
    for p in photos:
        cam, lens = CAM[p["model"]], LENSES[p["lens"]]
        f = folders[p["folder"]]
        spots = REMOTE_SPOTS[REMOTE_CAMS.index(p["model"])] if p["remote"] else weighted(rng, [(s, s[3]) for s in f["spots"]])
        if p["remote"]: spots = REMOTE_SPOTS[REMOTE_CAMS.index(p["model"])]
        p["spot"] = spots
        lo, hi = max(f["start"], cam["start"]), min(f["end"], cam["end"])
        days = [lo + dt.timedelta(n) for n in range((hi - lo).days + 1)]
        od = [d for d in outing[f["slug"]] if lo <= d <= hi]
        day = rng.choice(od) if od and rng.random() < .55 else rng.choices(days, weights=day_weights(f, days))[0]
        if p.get("astro"): hour = rng.choice([22, 23, 0, 1, 2, 3])
        else:
            hw = list(HOUR_W)
            if day.weekday() >= 5 or day in holidays(day.year):
                hw = [w * (1.4 if 9 <= h <= 15 else 1.0) for h, w in enumerate(hw)]
            hour = rng.choices(range(24), weights=hw)[0]
        p["local"] = dt.datetime(day.year, day.month, day.day, hour, rng.randrange(60), rng.randrange(60))
        tz = ZoneInfo(spots[4]); aware = p["local"].replace(tzinfo=tz)
        off = aware.utcoffset(); tot = int(off.total_seconds() // 60)
        p["offset"] = "%s%02d:%02d" % ("+" if tot >= 0 else "-", abs(tot) // 60, abs(tot) % 60)
        p["epoch"] = aware.timestamp()
        # light + exposure
        if p.get("astro"): light = "astro"
        elif 6 <= hour <= 8 or 17 <= hour <= 19: light = "golden" if rng.random() < .85 else "day"
        elif 9 <= hour <= 16: light = "indoor" if rng.random() < .12 else "day"
        elif hour in (5, 20): light = "blue"
        else: light = "night"
        p["light"] = light
        if lens["fmin"] == lens["fmax"]: focal = lens["fmin"]
        else:
            r = rng.random()
            if r < .38: focal = lens["fmin"] + (lens["fmax"] - lens["fmin"]) * rng.random() ** 2 * .12
            elif r < .68: focal = lens["fmax"] - (lens["fmax"] - lens["fmin"]) * rng.random() ** 2 * .15
            else: focal = rng.uniform(lens["fmin"], lens["fmax"])
            focal = round(focal)
        if p.get("astro"): focal = lens["fmin"]
        p["focal"] = focal
        p["focal35"] = lens["feq"] if lens["feq"] else round(focal * (cam["crop"] or 1))
        p["aperture"], p["iso"], p["shutter"] = design_exposure(rng, lens, cam, light, p["focal35"], p.get("astro"))
        # aspect / size
        ar = {"phone": [("4:3", .50), ("3:2", .12), ("16:9", .10), ("1:1", .08), ("2:3", .20)],
              "fuji": [("3:2", .60), ("2:3", .20), ("1:1", .10), ("16:9", .05), ("4:3", .05)],
              "ilc": [("3:2", .70), ("2:3", .18), ("16:9", .05), ("1:1", .03), ("4:3", .04)]}[
            "phone" if cam["crop"] is None else "fuji" if cam["make"] == "FUJIFILM" else "ilc"]
        a = weighted(rng, ar)
        p["aspect"] = a
        p["w"], p["h"] = {"3:2": (800, 533), "2:3": (533, 800), "4:3": (800, 600), "16:9": (800, 450), "1:1": (800, 800)}[a]
        # jitter 50-500 m
        r = rng.uniform(50, 500); th = rng.uniform(0, 2 * math.pi)
        p["lat"] = round(spots[1] + r * math.cos(th) / 111320, 6)
        p["lon"] = round(spots[2] + r * math.sin(th) / (111320 * math.cos(math.radians(spots[1]))), 6)
        p["vis_make"] = "" if p.get("nomake") else cam["make"]
        p["vis_lens"] = "" if p.get("nolens") else lens["name"]
    # heroes: 3 per each of the 4 most used (visible) lenses
    lc = Counter(p["vis_lens"] for p in photos if p["vis_lens"])
    top = [l for l, _ in sorted(lc.items(), key=lambda kv: (-kv[1], kv[0]))[:4]]
    for ln in top:
        el = [p for p in photos if p["vis_lens"] == ln and p["vis_make"] and not p.get("nodate") and not p.get("soft")
              and not p.get("dark") and not p.get("astro") and not p["remote"] and p["light"] in ("golden", "day")]
        el.sort(key=lambda p: (p["light"] != "golden", p["i"]))
        for p in rng.sample(el[:max(8, len(el) // 2)], 3): p["hero"] = True
    # ratings
    for p in photos:
        cam = CAM[p["model"]]
        s = rng.gauss(0, .8) + (.4 if cam["model"] == "Canon EOS R5" else 0) + (1.6 if cam["model"] == "Canon EOS R5" and p["lens"] == "RF70-200" else 0)
        s += {"RF24-105": .2, "FE85": .5, "FE24-70": .2, "XF56": .3, "RF50": .1}.get(p["lens"], 0) - (.5 if cam["crop"] is None else 0)
        s += (-1.5 if p.get("soft") else 0) + (-1.0 if p.get("dark") else 0) + (.3 if p.get("astro") else 0) + (.3 if p["light"] == "golden" else 0)
        p["score"] = s + (100 if p.get("hero") else 0)
    heroes = [p for p in photos if p.get("hero")]
    n_rated = 226
    rest = [p for p in photos if not p.get("hero")]
    keyed = sorted(rest, key=lambda p: -(rng.random() ** (1 / CAM[p["model"]]["rated"])))
    rated = heroes + keyed[:n_rated - len(heroes)]
    rated_set = {p["i"] for p in rated}
    nonhero = sorted([p for p in rated if not p.get("hero")], key=lambda p: p["score"])
    for p in photos: p["rating"] = 0
    for p in nonhero[:6]: p["rating"] = -1
    body = nonhero[6:]
    start = 0
    for stars, n in [(1, 20), (2, 32), (3, 68), (4, 68), (5, 32 - len(heroes))]:
        for p in body[start:start + n]: p["rating"] = stars
        start += n
    assert start == len(body), (start, len(body))
    for p in heroes: p["rating"] = 5
    # labels ~10%
    lw = [(p, 3.0 if p["rating"] >= 4 else 1.0) for p in photos]
    keyed = sorted(photos, key=lambda p: -(rng.random() ** (1 / (3.0 if p["rating"] >= 4 else 1.0))))
    for p in keyed[:40]: p["label"] = rng.choice(LABELS)
    for p in photos: p.setdefault("label", "")
    # file names and paths (counter in time order per camera)
    for m, cam in CAM.items():
        ps = sorted([p for p in photos if p["model"] == m], key=lambda p: p["epoch"])
        n = cam["n0"] + rng.randrange(0, 300)
        for p in ps:
            n += rng.randrange(1, 40)
            if m == "Pixel 10 Pro XL":
                name = "PXL_%s_%03d.jpg" % (p["local"].strftime("%Y%m%d_%H%M%S"), n % 1000)
            else:
                name = "%s%04d.jpg" % (cam["prefix"], n)
            p["name"] = name
            p["path"] = "%s/%s/%s" % (folders[p["folder"]]["year"], p["folder"], name)
    assert len({p["path"] for p in photos}) == N
    return photos, top

# ---------------------------------------------------------------- pixels
COLORS = ["navy","orange","teal","gold","crimson","forestgreen","skyblue","tomato","purple","khaki","slategray","coral","darkcyan","sienna"]
def make_base(k):
    rng = random.Random(SEED * 31 + k)
    out = WORK / "base" / ("synth%02d.png" % k)
    c1, c2 = rng.sample(COLORS, 2)
    S_ = "1100x1100"; seed = str(rng.randrange(1, 10**6)); t = k % 6
    base = ["convert", "-seed", seed]
    if t == 0: cmd = base + ["-size", S_, "plasma:fractal", "-blur", "0x1", str(out)]
    elif t == 1: cmd = base + ["-size", S_, "plasma:%s-%s" % (c1, c2), str(out)]
    elif t == 2: cmd = ["convert", "-size", S_, "gradient:%s-%s" % (c1, c2), "-swirl", "220", "-attenuate", "0.3", "+noise", "Gaussian", "-blur", "0x1", str(out)]
    elif t == 3: cmd = ["convert", "-size", S_, "radial-gradient:%s-%s" % (c1, c2), "-swirl", "300", "-wave", "15x250", str(out)]
    elif t == 4: cmd = ["convert", "(", "-size", "1100x600", "gradient:%s-%s" % (c1, c2), ")", "(", "-seed", seed, "-size", "1100x500",
                        "plasma:fractal", "-colorspace", "Gray", "-level-colors", "black,%s" % rng.choice(["darkgreen", "tan", "sienna"]), ")",
                        "-append", "-blur", "0x2", str(out)]
    else: cmd = base + ["-size", S_, "plasma:fractal", "-paint", "5", "-modulate", "100,140", str(out)]
    subprocess.run(cmd, check=True, stderr=subprocess.DEVNULL)
    return out

def hue_shift(im, s):
    h, sa, v = im.convert("HSV").split()
    return Image.merge("HSV", (h.point(lambda x: (x + s) % 256), sa, v)).convert("RGB")

def fit(im, rng, w, h, lo=.65):
    tw = w / h; iw, ih = im.size
    cw, ch = (iw, iw / tw) if iw / ih < tw else (ih * tw, ih)
    s = rng.uniform(lo, 1.0); cw, ch = cw * s, ch * s
    x0 = rng.uniform(0, iw - cw); y0 = rng.uniform(0, ih - ch)
    im = im.crop((round(x0), round(y0), round(x0 + cw), round(y0 + ch)))
    if rng.random() < .5: im = im.transpose(Image.FLIP_LEFT_RIGHT)
    return im.resize((w, h), Image.LANCZOS)

def astro_frame(rng, w, h):
    im = Image.new("RGB", (w, h)); d = ImageDraw.Draw(im)
    for y in range(h):
        d.line([(0, y), (w, y)], fill=(4 + y * 14 // h, 6 + y * 18 // h, 16 + y * 30 // h))
    band = Image.new("RGB", (w, h)); bd = ImageDraw.Draw(band)
    for _ in range(40):
        x = rng.randrange(w); y = int(h * .1 + (x / w) * h * .5 + rng.gauss(0, h * .04)); r = rng.randrange(25, 70)
        bd.ellipse([x - r, y - r, x + r, y + r], fill=(30, 28, 45))
    im = ImageChops.add(im, band.filter(ImageFilter.GaussianBlur(26)))
    d = ImageDraw.Draw(im)
    for _ in range(600):
        x, y = rng.randrange(w), rng.randrange(int(h * .8)); b = rng.randrange(120, 255)
        d.point((x, y), fill=(b, b, min(255, b + 10)))
    for _ in range(25):
        x, y = rng.randrange(w), rng.randrange(int(h * .7)); b = 255; d.ellipse([x - 1, y - 1, x + 1, y + 1], fill=(b, b, b))
    pts = [(0, h)] + [(x, int(h * .88 - abs(math.sin(x / 70)) * h * .06 - rng.random() * 8)) for x in range(0, w + 40, 40)] + [(w, h)]
    d.polygon(pts, fill=(3, 3, 5))
    return im

def lights(im, rng):
    w, h = im.size; layer = Image.new("RGB", im.size); d = ImageDraw.Draw(layer)
    for _ in range(30):
        x, y, r = rng.randrange(w), rng.randrange(h), rng.randrange(3, 12)
        d.ellipse([x - r, y - r, x + r, y + r], fill=rng.choice([(255, 190, 90), (255, 230, 170), (140, 190, 255), (255, 110, 80)]))
    return ImageChops.add(im, layer.filter(ImageFilter.GaussianBlur(4)))

def render(p, bases, hero_bases):
    rng = random.Random("%d-%d" % (SEED, p["i"]))
    w, h = p["w"], p["h"]
    if p.get("astro"): im = astro_frame(rng, w, h)
    else:
        if p.get("hero"): b = rng.choice(hero_bases)
        else: b = rng.choice(bases)
        im = fit(b[1].copy(), rng, w, h, .6 if not p.get("hero") else .8)
        shift = rng.randrange(256) if b[0] == "synth" else rng.choice([0, 0, rng.randrange(-14, 14)])
        if p.get("hero"): shift = 0 if b[0] != "synth" else shift
        if shift: im = hue_shift(im, shift % 256)
        L_ = p["light"]
        br = {"day": rng.uniform(.95, 1.15), "golden": rng.uniform(.95, 1.1), "blue": rng.uniform(.55, .75),
              "night": rng.uniform(.25, .4), "indoor": rng.uniform(.7, .9)}[L_]
        if p.get("dark"): br = rng.uniform(.18, .3)
        if p.get("hero"): br, L_ = 1.05, "golden"
        im = ImageEnhance.Brightness(im).enhance(br)
        if L_ == "golden": im = Image.blend(im, Image.new("RGB", im.size, (255, 150, 60)), .12)
        if L_ in ("night", "blue"): im = lights(im, rng) if L_ == "night" else im
        if p.get("hero"): im = ImageEnhance.Contrast(im).enhance(1.15); im = ImageEnhance.Color(im).enhance(1.2)
        if p.get("soft"): im = im.filter(ImageFilter.GaussianBlur(rng.uniform(3, 7)))
        elif not p.get("hero") and rng.random() < .25: im = im.filter(ImageFilter.GaussianBlur(.6))
    dest = PHOTOS / p["path"]; dest.parent.mkdir(parents=True, exist_ok=True)
    im.save(dest, "JPEG", quality=80, optimize=True, subsampling=2)

def load_bases():
    (WORK / "base").mkdir(parents=True, exist_ok=True)
    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(8) as ex: synth = list(ex.map(make_base, range(36)))
    bases = []
    orig = sorted(SOURCES.glob("*.jpg"))
    for f in orig:
        im = Image.open(f).convert("RGB"); im.thumbnail((1400, 1400)); bases += [("orig", im)] * 2
    for f in synth: bases.append(("synth", Image.open(f).convert("RGB")))
    hero = [b for b in bases if b[0] == "orig" and False]
    hero = [("orig", Image.open(SOURCES / n).convert("RGB")) for n in ("unique_1.jpg", "unique_2.jpg")]
    for h in hero: h[1].thumbnail((1400, 1400))
    hero += [b for b in bases if b[0] == "synth"][4::6] + [b for b in bases if b[0] == "synth"][1::9]
    return bases, hero

# ---------------------------------------------------------------- metadata
SOFTWARE_IPHONE = lambda d: "17.5.1" if d < D(2024, 9, 16) else "18.1" if d < D(2025, 9, 15) else "26.0.1"
def exif_args(p):
    cam, lens = CAM[p["model"]], LENSES[p["lens"]]
    a = ["-Model=" + p["model"]]
    if p["vis_make"]: a.append("-Make=" + p["vis_make"])
    sw = cam["soft"]
    if p["model"] == "iPhone 15 Pro": sw = SOFTWARE_IPHONE(p["local"].date())
    if p["model"] == "Pixel 10 Pro XL": sw = "Android 16"
    a.append("-Software=" + sw)
    if p["vis_lens"]:
        a += ["-LensModel=" + lens["name"], "-LensMake=" + lens["make"]]
    a += ["-FocalLength=%s" % ("%g" % p["focal"]), "-FocalLengthIn35mmFormat=%d" % p["focal35"],
          "-FNumber=%s" % ("%g" % p["aperture"]), "-ExposureTime=" + fmt_shutter(p["shutter"]), "-ISO=%d" % p["iso"],
          "-ExifImageWidth=%d" % p["w"], "-ExifImageHeight=%d" % p["h"],
          "-ExposureProgram#=" + ("3" if p["model"] in ("iPhone 15 Pro", "Pixel 10 Pro XL") else "1" if p.get("astro") else "2")]
    if not p.get("nodate"):
        d = p["local"].strftime("%Y:%m:%d %H:%M:%S")
        a += ["-DateTimeOriginal=" + d, "-CreateDate=" + d, "-OffsetTimeOriginal=" + p["offset"], "-OffsetTimeDigitized=" + p["offset"]]
    if p["gps"]:
        a += ["-GPSLatitude=%f" % abs(p["lat"]), "-GPSLatitudeRef=" + ("N" if p["lat"] >= 0 else "S"),
              "-GPSLongitude=%f" % abs(p["lon"]), "-GPSLongitudeRef=" + ("E" if p["lon"] >= 0 else "W")]
    return a

def write_exif(photos):
    argf = WORK / "exif.args"
    with open(argf, "w") as f:
        for p in photos:
            for a in exif_args(p): f.write(a + "\n")
            f.write(str(PHOTOS / p["path"]) + "\n-execute\n")
    subprocess.run(["exiftool", "-q", "-q", "-P", "-overwrite_original", "-@", str(argf)], check=True)

XMP = """<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="XMP Core 4.4.0-Exiv2">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:exif="http://ns.adobe.com/exif/1.0/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:xmpMM="http://ns.adobe.com/xap/1.0/mm/"
    xmlns:darktable="http://darktable.sf.net/"
{dto}   xmp:Rating="{rating}"
   xmpMM:DerivedFrom="{name}"
   darktable:import_timestamp="-1"
   darktable:change_timestamp="-1"
   darktable:export_timestamp="-1"
   darktable:print_timestamp="-1"
   darktable:xmp_version="5"
   darktable:raw_params="0"
   darktable:auto_presets_applied="0"
   darktable:history_end="0"
   darktable:iop_order_version="5">
   <darktable:masks_history>
    <rdf:Seq/>
   </darktable:masks_history>
   <darktable:history>
    <rdf:Seq/>
   </darktable:history>
{labels}  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"""
def write_xmp(p):
    dto = "" if p.get("nodate") else '   exif:DateTimeOriginal="%s.000"\n' % p["local"].strftime("%Y:%m:%d %H:%M:%S")
    labels = ""
    if p["label"]:
        labels = "   <darktable:colorlabels>\n    <rdf:Seq>\n     <rdf:li>%d</rdf:li>\n    </rdf:Seq>\n   </darktable:colorlabels>\n" % LABELS.index(p["label"])
    (PHOTOS / (p["path"] + ".xmp")).write_text(XMP.format(dto=dto, rating=p["rating"], name=p["name"], labels=labels))

def row_of(p):
    return dict(path=p["path"], make=p["vis_make"], model=p["model"], lens=p["vis_lens"], focal=("%g" % p["focal"]),
                focal35=p["focal35"], aperture=("%g" % p["aperture"]), iso=p["iso"], shutter=fmt_shutter(p["shutter"]),
                shutter_s=round(p["shutter"], 7), dt="" if p.get("nodate") else p["local"].strftime("%Y-%m-%d %H:%M:%S"),
                offset="" if p.get("nodate") else p["offset"], lat=p["lat"] if p["gps"] else "", lon=p["lon"] if p["gps"] else "",
                rating=p["rating"], label=p["label"], w=p["w"], h=p["h"],
                mtime_utc=dt.datetime.fromtimestamp(p["epoch"], dt.timezone.utc).strftime("%Y-%m-%d %H:%M:%S"),
                place=p["spot"][0], hero=int(bool(p.get("hero"))), kind="astro" if p.get("astro") else "soft" if p.get("soft") else "dark" if p.get("dark") else "normal")
COLS = ["path","make","model","lens","focal","aperture","iso","shutter","dt","lat","lon","rating","label","w","h",
        "focal35","shutter_s","offset","mtime_utc","place","hero","kind"]

def main():
    for d in (PHOTOS, WORK): shutil.rmtree(d, ignore_errors=True)
    OUT.mkdir(parents=True, exist_ok=True); PHOTOS.mkdir(); WORK.mkdir()
    photos, top = build_design()
    bases, hero_bases = load_bases()
    for p in photos: render(p, bases, hero_bases)
    write_exif(photos)
    for p in photos:
        write_xmp(p)
        os.utime(PHOTOS / p["path"], (p["epoch"], p["epoch"]))
    photos.sort(key=lambda p: p["path"])
    rows = [row_of(p) for p in photos]
    with open(OUT / "manifest.csv", "w", newline="") as f:
        w = csv.DictWriter(f, COLS); w.writeheader(); w.writerows(rows)
    agg = aggregate([{**r, "lat": r["lat"], "lon": r["lon"]} for r in rows])
    exp = dict(seed=SEED, photos=N,
               notes=dict(unknown_bucket="'Unknown' = missing Make/LensModel; camera is keyed by Model (never missing)",
                          datetime="per_year_month/per_hour use DateTimeOriginal local time; photos without it count under per_year['no-datetime'] only",
                          top10_five_star="five-star photos of that lens ordered by datetime descending then path; undated sort last-ish ('' key)",
                          buckets=dict(aperture=["f/1.0-1.8","f/2.0-2.8","f/3.2-4","f/4.5-5.6","f/6.3-8","f/9-11","f/13+"],
                                       iso=["<=100","101-400","401-1600","1601-6400",">6400"],
                                       shutter=["<=1/1000","1/1000-1/250","1/250-1/60","1/60-1/8","1/8-1s",">1s"],
                                       focal35=["<=24","25-35","36-70","71-135","136-200",">200"])),
               heroes=dict(top_lenses=top, photos=[r["path"] for r in rows if r["hero"]]),
               per_folder=dict(sorted(Counter(r["path"].split("/")[1] for r in rows).items())),
               aggregates=agg)
    (OUT / "expected.json").write_text(json.dumps(exp, indent=1, ensure_ascii=False) + "\n")
    shutil.rmtree(WORK, ignore_errors=True)
    print("done:", N, "photos")

if __name__ == "__main__":
    main()

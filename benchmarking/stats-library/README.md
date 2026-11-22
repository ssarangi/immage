# Statistics test library

Synthetic 400-photo library (2022-01 to 2026-11) for testing Immage's Statistics feature.
Ground truth (in `out/`): `manifest.csv` (one row per photo) and `expected.json` (aggregates).

## Regenerate / verify

    python3 generate.py     # ~40 s; deletes and rebuilds out/photos, out/manifest.csv, out/expected.json
    python3 verify.py       # recomputes aggregates from the files (exiftool + .xmp) and compares

The output goes to `benchmarking/stats-library/out/` (git-ignored), or to the folder in
`IMMAGE_STATS_LIBRARY` when set; Rust's `cargo test` (`aggregates_equal_the_generated_libraries_ground_truth`,
`dump_harness_fixtures`) looks there too and skips when it is absent. Needs `python3` with Pillow, `exiftool`, ImageMagick `convert`. Seed: `SEED = 20260501` in
generate.py; output is byte-identical on re-run (manifest and expected.json checksum-verified).
`sources/` holds copies of the 6 sample photos used as pixel sources, plus 36 ImageMagick-synthesised bases
(plasma, gradients, swirl, wave) built in `out/work/` and deleted afterwards; per-photo crop, flip, hue, brightness,
blur and light tint make each thumbnail differ. 24 frames are soft, 20 deliberately dark, 12 synthetic astro starfields.

## Design

- Layout: `photos/<year>/<year>-<month>-<place>/<file>.jpg` plus `<file>.jpg.xmp` (darktable format with
  `xmp:Rating`, `darktable:colorlabels`: 0 red, 1 yellow, 2 green, 3 blue, 4 purple, empty history). 18 folders.
- Cameras (exact counts): Canon EOS R5 140, ILCE-7M4 100, NIKON Z 6_2 60, X-T5 40, iPhone 15 Pro 40, Pixel 10 Pro XL 20,
  each only within its active dates (Nikon 2022-01..2024-02, Canon ..2025-06, Sony from 2022-07, Fuji from 2023-05,
  iPhone from 2023-10, Pixel from 2025-09). 12 lenses with per-body mix; focal lengths cluster at the ends.
- Exposure: light class from hour (day/golden/blue/night/indoor) gives EV; aperture from lens range, ISO and shutter
  derived (handheld limit, 1/8000 cap, 1/3-stop snapping); 12 astro frames (10-30 s, wide, ISO 1600-6400, hours 22-03).
- Time: golden-hour peaks (6-8, 17-19), weekend and holiday bursts for home (Seattle) folders, outing days,
  summer/autumn trips; local `DateTimeOriginal` + correct `OffsetTimeOriginal`. 12 photos (3%) have no
  `DateTimeOriginal` (file mtime only; `mtime_utc` column). File mtime is set to the capture time.
- GPS: exactly 280 of 400 (70%); 24 public landmarks on 6 continents jittered 50-500 m, Seattle dominant; 3 remote
  single-photo points (Salar de Uyuni, Svalbard, Wadi Rum) in `2024-09-remote`.
- Unknown buckets: 16 photos without Make (Model kept), 40 without LensModel (focal data kept).
- Aspect ratios: 3:2, 2:3, 4:3, 16:9, 1:1 (long edge 800 px, JPEG q80).
- Ratings: 0:174, 1:20, 2:32, 3:68, 4:68, 5:32, -1 (rejected): 6. Canon R5 and especially R5 + RF70-200 score higher,
  phones and soft/dark frames lower, phones are mostly unrated. 40 colour labels (skewed to 4-5 stars).
  3 hero frames (5 stars, unprocessed good sources, golden/day) for each of the 4 most-used lenses
  (`expected.json` `heroes`).

## manifest.csv columns

path, make, model, lens, focal, aperture, iso, shutter (string), dt (local `YYYY-MM-DD HH:MM:SS`, blank if none),
lat, lon (signed, blank if none), rating, label, w, h, then extras: focal35, shutter_s, offset, mtime_utc, place,
hero (0/1), kind (normal/soft/dark/astro). Blank make/lens = unknown bucket.

## expected.json

`aggregates`: per_camera (Model), per_make, per_lens, per_camera_lens, per_year (+`no-datetime`), per_year_month,
per_hour, per_weekday, per_rating, per_label, aperture/iso/shutter/focal35 buckets (edges in `notes.buckets`,
upper bounds inclusive), aspect_ratios, gps, per_lens_rating (photos, rated count, mean rating of rated, five-star
count, `top10_five_star` paths ordered by datetime descending then path). Also `per_folder`, `heroes`.

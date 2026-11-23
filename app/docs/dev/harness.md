# Dev-only harness pages

[`app/harness/`](../../harness) holds four pages that mount a real piece of the
interface on fixture data in a plain browser, with no Tauri, no engine and no
library. They exist to look at states that are hard to reach in the app (a
critique that has been loading for 65 seconds, a download that stopped, a
library of 47,000 photos) and to measure them. They are for developers only:
nothing in the app or the user guide links to them.

| Page | Mounts | Fixture data | Checked by |
|---|---|---|---|
| [critique.html](../../harness/critique.html) | the culling preview's `CullLightbox`, `CullSide` and `CullOverlay` (Critique and Cull tabs) | in [critique.tsx](../../harness/critique.tsx) | [critique-shift.test.mjs](../../../verify/critique-shift.test.mjs) |
| [scenes.html](../../harness/scenes.html) | the Scenes timeline, scene cards and the scene view (`ScenesView`, `SceneLightbox`, `CullLightbox`, `CullSide`) | synthetic photos drawn in the page (`setThumbSource`), fake paths, a fixture store ([scenes.tsx](../../harness/scenes.tsx)) | [scenes-shift.test.mjs](../../../verify/scenes-shift.test.mjs) |
| [statistics.html](../../harness/statistics.html) | the whole `Statistics` view | recorded replies of the real backend, [statsMock.ts](../../harness/statsMock.ts) | [stats-shift.test.mjs](../../../verify/stats-shift.test.mjs) |
| [models.html](../../harness/models.html) | `AiModelsView`, the AI models group of Preferences > General | [modelsFixtures.ts](../../harness/modelsFixtures.ts) | [models-shift.test.mjs](../../../verify/models-shift.test.mjs) |
| [grid.html](../../harness/grid.html) | the real `Filmstrip` (grid or strip) and `ThumbLoader` over `n` synthetic photos (`mode=`, `preload=1`, `check=1`) | generated in [grid.tsx](../../harness/grid.tsx) | [large-grid.test.mjs](../../../verify/large-grid.test.mjs) |
| [catalog.html](../../harness/catalog.html) | `CatalogCard` with its index card, the add and remove dialogs and Preferences' `CatalogPrefsView` (`?state=`, `?view=`) | [catalogFixtures.ts](../../harness/catalogFixtures.ts) | [catalog-shift.test.mjs](../../../verify/catalog-shift.test.mjs) |

[tauriShim.ts](../../harness/tauriShim.ts) is imported first by every page. It
stands in for the Tauri runtime so the app's modules load, and every command the
page sends fails, so nothing a harness page does can reach the engine or the
disk. A page that needs data gets it through a seam its component already has
(`StatsApi` in [statsApi.ts](../../src/statsApi.ts); the props of `AiModelsView`;
`setCull` in [cullStore.ts](../../src/cullStore.ts)).

## Running one

```sh
cd app
npm run dev                 # Vite on port 1430
```

Then open `http://localhost:1430/harness/<page>.html?<parameters>` in a browser.
The harness uses the app's own `styles.css` and a built-in theme, so it looks as
the app does.

## URL parameters

Every page takes `theme=<id>` (a built-in theme: `dark` by default, `graphite`,
`slate`, `warm-ash`, `grey`, `classic-grey`, `soft-grey`, `light`) and `shift=1`
(run the no-layout-shift test and write its JSON into a hidden `<pre>`:
`#shift-report` on critique and models, `#report` on statistics). The comment at
the top of each page's `.tsx` is the complete list; the main ones:

- **critique.html**: `state=result|loading|error|errorkept|nokey|empty|cull`,
  `frame=mat|flush`, `hover=row<N>|badge<N>`, `pin=<N>`, `elapsed=<seconds>` (for
  `loading`), and `break=1`. `#motion-report` reports how many animations run
  1.5 seconds after load, to check reduced motion.
- **scenes.html**: `view=timeline|scene` (with `scene=<n>`), `state=ready|loading|updating|empty|error|notime|scoring|unscored`,
  `size=small|big` (400 or 20,000 synthetic photos), `dir=ltr|rtl`, `scroll=<px>`, `fold=<block>`
  and `break=1`. `--accept-lang=fr` (a Chrome flag) shows the French strings.
- **statistics.html**: `state=ready|loading|empty|error|firstrun`,
  `cov=indexing|ready|paused` (the coverage banner), `filter=key:value,...`
  (keys `maker`, `body`, `lens`, `year`, `month`, `hour`, `focal`, `aperture`,
  `shutter`, `iso`, `label`, `place`, `top`), `scope=library|range`, `min=<stars>`,
  `lib=big|gt`, `delay=<ms>`, `scrollTo=<section>`, `top=error|loading|pagefail|changed|slow`,
  `thumbs=none`, `gps=none`, `tiles=offline`, `photos=synthetic`, and `bad=1|2|3`.
- **models.html**: `state=` one of the names in `STATES` in
  [modelsFixtures.ts](../../harness/modelsFixtures.ts) (`notinstalled`,
  `downloading`, `installed`, `repair`, `offline`, `consent`, `remove`, and the
  Auto-keywords card's `kwoff`, `kwworking`, `kwpaused`, `kwreview`, ...),
  `w` and `h` (the page area in pixels), and `break=1`. `window.__h.set(name)`
  switches state from the console.

## Fixtures

- **Critique and models** keep their fixtures in code. A new state is a new
  entry in `STATES` (models) or a new branch in `critique.tsx`, and a new row in
  the matching test.
- **Statistics** serves the real aggregation, not a hand-written mock.
  [statsMock.ts](../../harness/statsMock.ts) looks a request up by its canonical
  JSON in [stats-fixtures/](../../harness/stats-fixtures) (`gt.json`: the 400
  photos of a generated ground-truth library; `big.json`: 117 copies of it, about
  47,000 photos) and passes the recorded reply through the page's own
  `normalizeReply`. A request with no recorded reply is shown as an error on the
  page and listed in `window.__missingFixtures`, so the recorded set is a visible
  limit: when a new section or request shape is added, record it again.
  Thumbnails are the pictures in [stats-real/](../../harness/stats-real) (or
  [stats-photos/](../../harness/stats-photos) with `photos=synthetic`), and map
  tiles are drawn on a canvas by [mapTiles.ts](../../harness/mapTiles.ts).

Regenerate the statistics fixtures with the ignored Rust test that runs the real
`stats.rs` over the ground-truth library:

```sh
cd app/src-tauri
cargo +1.98.0 test --locked --release dump_harness_fixtures -- --ignored --nocapture
```

The test reads the library's `manifest.csv` and rewrites both JSON files. The
library is made by [benchmarking/stats-library/](../../../benchmarking/stats-library)
(`python3 generate.py`, about 40 s, needs Pillow, `exiftool` and ImageMagick;
`python3 verify.py` checks it) into its git-ignored `out/` folder, which
[stats/tests.rs](../../src-tauri/src/stats/tests.rs) finds by itself. Set
`IMMAGE_STATS_LIBRARY` to use a library elsewhere (the generator honours it too).
Without a library the ground-truth integration test
(`aggregates_equal_the_generated_libraries_ground_truth`) prints a SKIPPED
message and passes, and `dump_harness_fixtures` fails with a message saying how
to generate it.

## The no-layout-shift tests

The rule: hovering, focusing, pinning or selecting something never moves another
thing on screen. Each page has a `*Shift.ts` module
([shiftTest.ts](../../harness/shiftTest.ts) for the critique,
[statsShift.ts](../../harness/statsShift.ts),
[modelsShift.ts](../../harness/modelsShift.ts)) that, in every state it lists:

1. records the rectangle of every element and every text run in the window
   (areas drawn out of flow by design, such as tooltips, are skipped);
2. applies each interaction in turn. A script cannot hover, so the stylesheets'
   `:hover` and `:focus-visible` rules are copied onto the classes `.__hover` and
   `.__fv`, and transitions and animations are switched off so the measurement
   is of layout and not of an animation;
3. records again, and reports the largest movement (`maxDelta`), the worst
   element, and how many interactions and measurements ran. Fixed regions
   (the stage, header, tabs, footer) are also compared across states
   (`fixedMaxDelta`); the Statistics test also really clicks a chart mark and a
   map bubble and checks that the frame around the content did not move.

The `.mjs` tests run each page in headless Chrome with `--dump-dom`, read the JSON
from the hidden `<pre>`, and assert that the movement is zero and that enough was
measured (a test that measures nothing fails). Each page also has a **negative
control**: `break=1` (critique, models, catalog) or `bad=1`, `bad=2`, `bad=3` (Statistics)
adds a style that does move things on hover, and the test asserts the measurement
catches it. Run them with:

```sh
node --test verify/critique-shift.test.mjs verify/models-shift.test.mjs verify/catalog-shift.test.mjs
node --test verify/stats-shift.test.mjs     # needs `npm run dev` already running on 1430
```

The critique and models tests start their own Vite server and skip when Chrome
(`/usr/bin/google-chrome`, or `$CHROME`) or `app/node_modules` is missing. The
Statistics test skips unless Chrome is installed and the dev server answers on
port 1430. None of the three is in the CI `checks` job, which lists only the
engine-free tests that need no browser (it does run the unit tests
`ai-packs`, `critique-pins` and the `stats-*` model tests). The Chrome tests are run
locally before a change to these views; a CI job would need a Chrome, a dev
server started in the background and a wait for its port, and was left out.

## Not part of the production build

[vite.config.ts](../../vite.config.ts) names only `index.html` and `splash.html`
as build inputs, so the harness pages are served by `npm run dev` and never
reach `vite build` or the installer; `models-shift.test.mjs` asserts that
`vite.config.ts` does not mention the harness. The pages share the app's source,
so an interface change shows up in them with no extra step, but `tsconfig.json`
includes only `src`: `npm run build` does not type-check `app/harness/`. Run
`npm run check:harness` in `app/` for that (it uses `tsconfig.harness.json`, which
adds `harness` to `src`; it is a development check, not part of the build or CI).

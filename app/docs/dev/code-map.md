# Code map

Immage is a raw photo editor with familiar panels, built on darktable. It runs as three
processes:

- **The WebView**: the React front end in [`app/src`](../../src).
- **The Tauri core**: the Rust layer in
  [`app/src-tauri/src`](../../src-tauri/src), the one gate to the engine.
- **`darktable-engine`**: a headless darktable session in
  [`engine/`](../../../engine) (`main.c` serves the requests), a child of the Rust layer,
  spoken to in JSON, one line per request.

The front end never touches files or darktable: it calls the typed
`commands.x(...)` (generated bindings, never raw `invoke`), and engine work goes through the one `Mutex<Engine>` in Rust,
one request at a time, except the engine's own threads for thumbnails,
folder reads, exports, AI and denoise. All image processing is
darktable's; its source changes only by
[patches/darktable-engine.patch](../../../patches/darktable-engine.patch).

This is the short version, loaded into every agent session.

Advanced mask cards use `AdvancedMaskCards` in `AdvancedPanel.tsx`, the
`MASK_MODULES` allowlist in `manifest.ts`, and Session's mask-module methods.
Mask instances carry their mask name in the engine's `instances` reply;
`Mask.extra` keeps operations with unmapped parameters out of `planMerge`.
[architecture.md](architecture.md) has the detail: each file's job in
full, the engine's pipes and commands, how each operation moves through
the app, where data lives, and the frame formats. Diagrams are in
[`diagrams/`](diagrams/). Paths below are relative to this file.

## Where to start a change

| Work | Start here | Related work |
|---|---|---|
| Remap a slider, or add one to an existing panel | `modules` and `panels` in [manifest.ts](../../src/manifest.ts) | the module's math file ([wb.ts](../../src/wb.ts), [tonecurve.ts](../../src/tonecurve.ts), ...), and [lib/edits.mjs](../../../benchmarking/lib/edits.mjs), which reads the manifest for the benchmark |
| A new panel or section | `panels` in [manifest.ts](../../src/manifest.ts) | `PanelView` in [PanelView.tsx](../../src/PanelView.tsx) for a custom graph; [CopySettings.tsx](../../src/CopySettings.tsx) groups |
| A per-mask (local) adjustment | [masks.md](masks.md) for the design; `LOCAL_CONTROLS` and `localMaps` in [manifest.ts](../../src/manifest.ts) (`merge` and `fixed` for a module masks share) | [MasksPanel.tsx](../../src/MasksPanel.tsx), [localMerge.ts](../../src/localMerge.ts); the engine's `masks_named_instance`, `masks_attach_mask` and `masks_combine` in [masks.c](../../../engine/masks.c) |
| A new mask shape or kind | [masks.ts](../../src/masks.ts) (`Shape`) | [MaskOverlay.tsx](../../src/MaskOverlay.tsx), `Session.addMask`, the engine's `_shape` / `masks_mask` in [masks.c](../../../engine/masks.c) |
| A typed Tauri command | [bindings.rs](../../src-tauri/src/bindings.rs) (`commands()`), then regenerate [bindings.ts](../../src/bindings.ts) (`IMMAGE_WRITE_BINDINGS=1 cargo test bindings`) | page calls go through [commands.ts](../../src/commands.ts); [architecture.md](architecture.md); a typed engine reply needs a fixture (`verify/engine-protocol.py --fixtures`), see there |
| A new engine command | the `_commands[]` table and a `_cmd_<name>` handler in [main.c](../../../engine/main.c), with the feature code in its section file (`session_`, `library_`, `export_`, `masks_`, ... below) | a `#[tauri::command]` with `#[specta::specta]` in the `*_cmds.rs` file for its group (below), listed in `collect_commands!` in [bindings.rs](../../src-tauri/src/bindings.rs), then `IMMAGE_WRITE_BINDINGS=1 cargo test --locked bindings` regenerates [bindings.ts](../../src/bindings.ts); the caller calls `commands.x(...)` from [commands.ts](../../src/commands.ts) in [session.ts](../../src/session.ts) or [App.tsx](../../src/App.tsx) (never `invoke`: [typed-commands.test.mjs](../../../verify/typed-commands.test.mjs) fails on it);  [engine-commands.test.mjs](../../../verify/engine-commands.test.mjs) and [engine-protocol.py](../../../verify/engine-protocol.py) check the table and the replies |
| Rendering, frames, zoom | `Session.pump` / `renderView` in [session.ts](../../src/session.ts) | `currentView`, `paintFrame`, `animateTo` in [App.tsx](../../src/App.tsx); `render` in [session_cmds.rs](../../src-tauri/src/session_cmds.rs); `session_render` in [session.c](../../../engine/session.c) |
| A Scenes feature (timeline, scene cards, scene view, rules) | [scenes.ts](../../src/scenes.ts) (pure model, keys, row heights) | [ScenesView.tsx](../../src/ScenesView.tsx), [SceneLightbox.tsx](../../src/SceneLightbox.tsx), [SceneRules.tsx](../../src/SceneRules.tsx), [ScenePiles.tsx](../../src/ScenePiles.tsx), [scenesStore.ts](../../src/scenesStore.ts); [scenes.rs](../../src-tauri/src/scenes.rs) (the sweep), [scenes_rules.rs](../../src-tauri/src/scenes_rules.rs) (keep rules), [scenes_cmds.rs](../../src-tauri/src/scenes_cmds.rs); the harness `scenes.html` ([scenes-shift.test.mjs](../../../verify/scenes-shift.test.mjs)) |
| An export option | `Settings` in [exportSettings.ts](../../src/exportSettings.ts) | [ExportDialog.tsx](../../src/ExportDialog.tsx); `export_images` in [export_cmds.rs](../../src-tauri/src/export_cmds.rs); `export_export` / `_frame_style` in [export.c](../../../engine/export.c); its progress, [background.ts](../../src/background.ts) |
| An AI feature | `runAi` / `runDenoise` in [App.tsx](../../src/App.tsx) | `withModel` in [aiModel.ts](../../src/aiModel.ts); `ai_model_ensure` in [ai_cmds.rs](../../src-tauri/src/ai_cmds.rs); [ai.c](../../../engine/ai.c) (`HAVE_AI`) and [device.c](../../../engine/device.c); darktable's `src/ai` and `src/common/ai` |
| A preference | [Preferences.tsx](../../src/Preferences.tsx) (a `load*`/`store*` pair and a `PrefRow`) | [PrefsUI.tsx](../../src/PrefsUI.tsx); the consumer calls `load*()` live |
| A backup component (Preferences > General > Backup) | `COMPONENTS` in [backup.rs](../../src-tauri/src/backup.rs) (its zip folder, file and label), then the matching arms of `components`, `file_source`, `create`, `save_current`, `validate` and `replace` | [BackupDialog.tsx](../../src/BackupDialog.tsx); the `backup_*` commands in backup.rs; for darktable's data, the engine's `data_backup` in [library.c](../../../engine/library.c) |
| An Immich call | [immich_search.rs](../../src-tauri/src/immich_search.rs) or [immich_upload.rs](../../src-tauri/src/immich_upload.rs) | `immich_*` commands in [immich_cmds.rs](../../src-tauri/src/immich_cmds.rs); [immich.ts](../../src/immich.ts); Immich search is in Find (below) |
| An Edit In plugin kind | `KINDS` in [plugins.ts](../../src/plugins.ts) | [plugins.rs](../../src-tauri/src/plugins.rs); `editIn` in [App.tsx](../../src/App.tsx) |
| Printing, soft proofing | [print.md](print.md); [PrintPanel.tsx](../../src/PrintPanel.tsx), [print.ts](../../src/print.ts) | [engine/print.c](../../../engine/print.c); `print_*` (`print_photo`, `print_sheet`), `printers`, `printer_info`, `softproof` in [print_cmds.rs](../../src-tauri/src/print_cmds.rs); `session_frame_hash` in [session.c](../../../engine/session.c); `Session.redraw` |
| Performance | [benchmarking/README.md](../../../benchmarking/README.md), [tracing flow](performance-tracing.md); opening a photo: [open-photo.mjs](../../../benchmarking/open-photo.mjs) (the app's `Session` against the engine), `openFileNow` in [App.tsx](../../src/App.tsx), `Session.close` / `retire` | [perf.ts](../../src/perf.ts), [perfDiagnostics.ts](../../src/perfDiagnostics.ts), [PerfHud.tsx](../../src/PerfHud.tsx); `perf_*` in [app_cmds.rs](../../src-tauri/src/app_cmds.rs), [perf_trace.rs](../../src-tauri/src/perf_trace.rs); [diagnose.py](../../../benchmarking/diagnose.py) |
| Local search, CLIP | [search_cache.rs](../../src-tauri/src/search_cache.rs), [local_search.rs](../../src-tauri/src/local_search.rs) | [combinedSearch.ts](../../src/combinedSearch.ts), [SearchPrefs.tsx](../../src/SearchPrefs.tsx); `clip_embed` in [main.c](../../../engine/main.c) and [clip.c](../../../engine/clip.c); [architecture.md](architecture.md) |
| Move to another darktable commit or release | [darktable-pin.md](darktable-pin.md) | [patches/darktable-engine.patch](../../../patches/darktable-engine.patch) |
| A release | the five version files ([package.json](../../package.json), `package-lock.json`, [Cargo.toml](../../src-tauri/Cargo.toml), `Cargo.lock`, [tauri.conf.json](../../src-tauri/tauri.conf.json)) | [releasing.md](releasing.md): the work on `release-X.Y.Z`, tagged `vX.Y.Z` there; [build_dmg.sh](../../scripts/build_dmg.sh) |

## Files

Front end ([`app/src`](../../src)):

- [settings.ts](../../src/settings.ts): the synchronous settings store every module uses instead of localStorage; [main.tsx](../../src/main.tsx) waits for it, then [boot.tsx](../../src/boot.tsx) starts the app
- [i18n.ts](../../src/i18n.ts): automatic catalog discovery, startup language and display formatting;
  [i18nCore.ts](../../src/i18nCore.ts): language matching, plural fallback and text direction.
  Language files are in [locales/](../../src/locales); see [i18n.md](i18n.md).
- [App.tsx](../../src/App.tsx): the editor's shell and state, with its components in [Toolbar.tsx](../../src/Toolbar.tsx), [Filmstrip.tsx](../../src/Filmstrip.tsx), [ActivityBar.tsx](../../src/ActivityBar.tsx), [FileMenu.tsx](../../src/FileMenu.tsx), [HistogramView.tsx](../../src/HistogramView.tsx), [Navigator.tsx](../../src/Navigator.tsx), [PanelView.tsx](../../src/PanelView.tsx), [Controls.tsx](../../src/Controls.tsx), [icons.tsx](../../src/icons.tsx), [layoutPrefs.ts](../../src/layoutPrefs.ts) and [zoom.ts](../../src/zoom.ts); [session.ts](../../src/session.ts): one open photo, edits to the engine, renders, saves; [engineState.ts](../../src/engineState.ts): engine restarts and the restore after one
- [manifest.ts](../../src/manifest.ts): panels, controls, and their mapping to darktable params; math in [wb.ts](../../src/wb.ts), [tonecurve.ts](../../src/tonecurve.ts), [transform.ts](../../src/transform.ts), [colorgrading.ts](../../src/colorgrading.ts), [colorequal.ts](../../src/colorequal.ts), [autoTone.ts](../../src/autoTone.ts), [bwauto.ts](../../src/bwauto.ts)
- masks: [masks.ts](../../src/masks.ts) (shapes, overlay pixels), [localMerge.ts](../../src/localMerge.ts) (shared instances), [MaskOverlay.tsx](../../src/MaskOverlay.tsx), [MasksPanel.tsx](../../src/MasksPanel.tsx), [ParametricEditor.tsx](../../src/ParametricEditor.tsx); design in [masks.md](masks.md)
- heal, negatives, zones: [spots.ts](../../src/spots.ts), [SpotOverlay.tsx](../../src/SpotOverlay.tsx), [HealPanel.tsx](../../src/HealPanel.tsx); [negadoctor.ts](../../src/negadoctor.ts), [NegativePanel.tsx](../../src/NegativePanel.tsx) (including input and working color profiles); [zones.ts](../../src/zones.ts), [ZoneSystem.tsx](../../src/ZoneSystem.tsx)
- film simulation: [spektrafilm.ts](../../src/spektrafilm.ts) (params, stock hash, presets), [FilmPanel.tsx](../../src/FilmPanel.tsx) and [NegativePanel.tsx](../../src/NegativePanel.tsx) as numbered steps in [GuidedSteps.tsx](../../src/GuidedSteps.tsx); [PositiveFooter.tsx](../../src/PositiveFooter.tsx) (Export to Library and Turn Off, shared with Negative); `CONVERSIONS` in manifest.ts; the engine's `film_spektra_stocks` and `film_spektra_fetch` in [film.c](../../../engine/film.c), and `_held_for_mode` in [session.c](../../../engine/session.c)
- canvas layers: [canvasLayers.ts](../../src/canvasLayers.ts) keeps canvas allocations and immutable frame uploads, reuses a sharp canvas for the base, and skips unchanged placement styles
- frames to the page: [sharedFrames.ts](../../src/sharedFrames.ts) (Windows and Linux shared memory), [pixelRatio.ts](../../src/pixelRatio.ts) (Wayland draft scale)
- folder trees: [FolderTreeView.tsx](../../src/FolderTreeView.tsx) and [folderTreeModel.ts](../../src/folderTreeModel.ts) (the one tree: flat rows, ARIA, keyboard, window), [FolderTree.tsx](../../src/FolderTree.tsx) (Folders adapter), the Catalog adapter in [CatalogCard.tsx](../../src/CatalogCard.tsx) with [catalogTree.ts](../../src/catalogTree.ts) (rows, merge, throttle) and [catalogTreeStore.ts](../../src/catalogTreeStore.ts) (open folders, the 2 s tick), [FolderMenu.tsx](../../src/FolderMenu.tsx); the backend's [catalog_tree.rs](../../src-tauri/src/catalog_tree.rs) (`catalog_tree_children`)
- Scenes: [scenes.ts](../../src/scenes.ts) (pure: model, state, row heights, keys), [scenesStore.ts](../../src/scenesStore.ts) (build, scoring, decisions), [ScenesView.tsx](../../src/ScenesView.tsx) (the virtualised timeline, in the grid's place), [SceneLightbox.tsx](../../src/SceneLightbox.tsx) (the scene view over `GridCulling`: burst filmstrip, Scene tab), [SceneThumb.tsx](../../src/SceneThumb.tsx) (thumbnails and the sharpest-region crop), [SceneRules.tsx](../../src/SceneRules.tsx) (the rules panel), [ScenePiles.tsx](../../src/ScenePiles.tsx) (the piles review), [scenesFormat.ts](../../src/scenesFormat.ts), [scenes.css](../../src/scenes.css)
- library: [library.ts](../../src/library.ts), [GridCulling.tsx](../../src/GridCulling.tsx) and [culling.ts](../../src/culling.ts) (grid culling), [folderEdits.ts](../../src/folderEdits.ts) (edits made during a folder read), [FolderTree.tsx](../../src/FolderTree.tsx), [filter.ts](../../src/filter.ts); Find: [findModel.ts](../../src/findModel.ts) (state, `needsSearch`, `effectiveFilter`, `chips` for the active count, `clearAll`), [FindPanel.tsx](../../src/FindPanel.tsx) (the panel, the accordion's scrolling and Escape), [FindFilters.tsx](../../src/FindFilters.tsx) (rows and summaries), [FindEditors.tsx](../../src/FindEditors.tsx) (the editors a row opens), [savedFilters.ts](../../src/savedFilters.ts), [SavedFilterMenus.tsx](../../src/SavedFilterMenus.tsx); merges in [HdrDialog.tsx](../../src/HdrDialog.tsx), [PanoDialog.tsx](../../src/PanoDialog.tsx)
- export: [exportSettings.ts](../../src/exportSettings.ts), [exportTokens.ts](../../src/exportTokens.ts), [ExportDialog.tsx](../../src/ExportDialog.tsx), [ExportFrame.tsx](../../src/ExportFrame.tsx); background work in [background.ts](../../src/background.ts), [activity.ts](../../src/activity.ts)
- print: [print.ts](../../src/print.ts) (settings, papers, single and Cells page layouts, soft proof), [PrintPanel.tsx](../../src/PrintPanel.tsx); the engine's [print.c](../../../engine/print.c)
- [AdvancedPanel.tsx](../../src/AdvancedPanel.tsx): Advanced editing mode's Adjustments: the module search ([advancedSearch.ts](../../src/advancedSearch.ts): the matching and ranking; App's `searchPick` opens the group and passes a pick to it) and a group body (`AdvancedGroup`, also the Technical group), hooked in from `PanelView`; `loadEditMode` in Preferences.tsx; groups render `ModuleCard` ([ModuleCard.tsx](../../src/ModuleCard.tsx), with its instance menu in [InstanceMenu.tsx](../../src/InstanceMenu.tsx), menus placed by [cardMenu.ts](../../src/cardMenu.ts), the group's [AddModuleMenu.tsx](../../src/AddModuleMenu.tsx), the tone mapper switch [FamilySwitch.tsx](../../src/FamilySwitch.tsx) and its module presets menu in [PresetMenu.tsx](../../src/PresetMenu.tsx), names shown by `presetLabel` in [presetName.ts](../../src/presetName.ts)) from `Session.instances` / `Session.advanced`, with per-module layouts in [advancedLayout.ts](../../src/advancedLayout.ts); Simple's "N advanced edits" badge counts from the engine's `instances` (`changed`) in [advancedEdits.ts](../../src/advancedEdits.ts), shown by `PanelWidget`; the same file picks Copy Settings' Advanced modules (`advancedCopies`, `pasteTarget`, used by `Session.copyAdvanced` / `pasteAdvanced`)
- local search: [localSearch.ts](../../src/localSearch.ts), [AnalysisStatus.tsx](../../src/AnalysisStatus.tsx) (Find's analysis icon: polls the sync `search_analysis_status` every 2 s, backlog counted by `search_analysis.rs`, click toggles `search_pause`), [clipStatus.ts](../../src/clipStatus.ts) (whether the CLIP model is installed: Find's Smart switch and tip), [combinedSearch.ts](../../src/combinedSearch.ts) (independent local and Immich sources), [searchIdentity.ts](../../src/searchIdentity.ts) (merging by verified local path), [SearchPrefs.tsx](../../src/SearchPrefs.tsx) (cache and CLIP model controls)
- folder chooser for Add folder and Add to Catalog: [FolderBrowser.tsx](../../src/FolderBrowser.tsx) (`browseFolder`; model in [folderBrowserModel.ts](../../src/folderBrowserModel.ts)), over `folder_places` and `list_subdirectories` in [folder_browser.rs](../../src-tauri/src/folder_browser.rs) (read-only, timed); harness in `app/harness/folderBrowser.html`
- People (user guide [people.md](../people.md)): [peopleModel.ts](../../src/peopleModel.ts) (pure: sections and tiles (group collage, ignored faces), the rail counts from the backend totals, scan notes, stable order, selection reducer, grid geometry and marquee, keyboard map, name ranking, Undo merging, scan state; tested by verify/people-model.test.mjs), [peopleStore.ts](../../src/peopleStore.ts) (state, events, the 2 s batched reads held while the pointer is on a tile, optimistic changes with rollback and `faces_undo`; `PeopleApi` is the one door to the `faces_*` commands, so the harness runs on fixtures), [peopleCrops.ts](../../src/peopleCrops.ts) (lazy crops, 6 in flight), [PeopleView.tsx](../../src/PeopleView.tsx) (the shell: scan strip, toolbar, selection bar, rail, inspector, Undo line, dialogs), [PeopleGrid.tsx](../../src/PeopleGrid.tsx) (the virtualised grid and marquee), [PeopleName.tsx](../../src/PeopleName.tsx) (the name combobox), [FaceOverlay.tsx](../../src/FaceOverlay.tsx) with [faceOverlayModel.ts](../../src/faceOverlayModel.ts) (the boxes over the photo: a layer App places with the mask editor's boxes, geometry and names pure; tested by verify/face-overlay.test.mjs and the `view=overlay` harness), [PeopleCard.tsx](../../src/PeopleCard.tsx) with [usePhotoFaces.ts](../../src/usePhotoFaces.ts) (the Library column's People card; `people` in `LEFT_CARDS`), [PersonPage.tsx](../../src/PersonPage.tsx) (a person's page head and tabs) and [PersonPhotos.tsx](../../src/PersonPhotos.tsx) (its Photos tab: the app's Filmstrip over `faces_person_photos`), [FindFaces.tsx](../../src/FindFaces.tsx) with [facesFilter.ts](../../src/facesFilter.ts) (Find's People row and the path sets behind it; `findModel.ts` `faces` category), [PeopleSuggest.tsx](../../src/PeopleSuggest.tsx) (the Suggestions row; `suggestRows`, `capRows`, `lookAlikes` in peopleModel.ts), [PeopleConsent.tsx](../../src/PeopleConsent.tsx) (the first-run card), [PeoplePane.tsx](../../src/PeoplePane.tsx) (mounts it with the faces pack), [people.css](../../src/people.css); the tab is in `VIEW_TABS` (ViewTabs.tsx) and `people` in App.tsx; the scan line in the activity bar is `setScanJob` in [background.ts](../../src/background.ts); harness `app/harness/people.html` and verify/people-shift.test.mjs (no layout shift, keyboard-only naming, 5,000 faces, negative control)
- [SectionStatus.tsx](../../src/SectionStatus.tsx) (a folded Library card's header status; `catalogSummary` in catalogModel.ts feeds the Catalog's, the harness test is `fold=1` in catalogShift.ts), [LensPicker.tsx](../../src/LensPicker.tsx): Lens Corrections' lens drop-down with a search box (`lenses` in edit_cmds.rs and the engine's [main.c](../../../engine/main.c), [lenses.cc](../../../engine/lenses.cc); `lens_pick` in manifest.ts)
- moving and copying photos: [photoDrag.ts](../../src/photoDrag.ts) (thumbnail drag onto a Folders row), [MoveDialog.tsx](../../src/MoveDialog.tsx) (confirmation), `askMove` / `confirmMove` in App.tsx, `runMove` in background.ts; Rust [move_cmds.rs](../../src-tauri/src/move_cmds.rs)
- [Presets.tsx](../../src/Presets.tsx) (presets, looks), [HistoryPanel.tsx](../../src/HistoryPanel.tsx), [CopySettings.tsx](../../src/CopySettings.tsx), [ColorWheel.tsx](../../src/ColorWheel.tsx), [PointCurve.tsx](../../src/PointCurve.tsx), [CropTool.tsx](../../src/CropTool.tsx), [VignetteOverlay.tsx](../../src/VignetteOverlay.tsx), [GuideOverlay.tsx](../../src/GuideOverlay.tsx), [MetadataPanel.tsx](../../src/MetadataPanel.tsx), [Keywording.tsx](../../src/Keywording.tsx)
- Immich and plugins: [immich.ts](../../src/immich.ts), [ImmichPrefs.tsx](../../src/ImmichPrefs.tsx); [plugins.ts](../../src/plugins.ts), [PluginPrefs.tsx](../../src/PluginPrefs.tsx)
- Catalog UI: [catalogModel.ts](../../src/catalogModel.ts) (pure: the one state a source shows, phases, time left, which source a folder is in) and [catalogEvents.ts](../../src/catalogEvents.ts) (the reducer that chains find, read and thumbnails), tested by verify/catalog-ui.test.mjs; [catalogStore.ts](../../src/catalogStore.ts) (events, polling, `indexSource`, `addToCatalog`, `rebuildCatalog`); [catalogUi.tsx](../../src/catalogUi.tsx) (the one hook App uses: card, folder-menu tools, dialogs), [CatalogCard.tsx](../../src/CatalogCard.tsx) (Library card, rows, menu), [IndexCard.tsx](../../src/IndexCard.tsx) (fixed-height first-index card), [CatalogDialogs.tsx](../../src/CatalogDialogs.tsx) (add, remove), [CatalogPrefs.tsx](../../src/CatalogPrefs.tsx) (Preferences > General); the folder menu's tools are `FolderTools` in [FolderTree.tsx](../../src/FolderTree.tsx); design in notes/catalog-ux-spec.md; harness `app/harness/catalog.html` and verify/catalog-shift.test.mjs (no layout shift, with a negative control)
- [PerformanceTuner.tsx](../../src/PerformanceTuner.tsx), [previewTuning.ts](../../src/previewTuning.ts), [tunerOptions.ts](../../src/tunerOptions.ts), [tunerWorkload.ts](../../src/tunerWorkload.ts): quality-first preview search with trimmed-median metrics, performance option comparisons, relaunch verification and navigation workload recording
- [benchDrivers.ts](../../src/benchDrivers.ts): `runGuiBenchmark` and `runGpuCheck`, the Preferences GUI benchmark and GPU check drivers (App passes a `BenchContext`)
- [gpuCheck.ts](../../src/gpuCheck.ts): the GPU check's cases and verdicts (CPU vs OpenCL parity, `device_parity` in [device.c](../../../engine/device.c), `gpu_check_parity` in app_cmds.rs, [gpucheck.py](../../../benchmarking/gpucheck.py))
- [BackupDialog.tsx](../../src/BackupDialog.tsx): the Back Up and Restore dialog under Preferences > General, over backup.rs.
- [thumbSignals.ts](../../src/thumbSignals.ts): tells the Catalog's thumbnail job (`thumbnails_busy`, `thumbnails_hint`) that the page is in use and which photos are on screen; [Filmstrip.tsx](../../src/Filmstrip.tsx) draws only the cells near the screen and asks `ThumbLoader` only for those (benchmarks: [large-catalog-js.mjs](../../../benchmarking/large-catalog-js.mjs), [large-catalog-engine.py](../../../benchmarking/large-catalog-engine.py))
- [SelectMenu.tsx](../../src/SelectMenu.tsx): the app-drawn list every `<select>` opens (see architecture.md)
- [menu.ts](../../src/menu.ts): the File/Edit/Photo/View/Help menus, once for both platforms; [MenuBar.tsx](../../src/MenuBar.tsx) draws them in the title strip on Windows and Linux; `menu_set` in app_cmds.rs builds the macOS menu bar, its clicks come back as the `menu` event to `runMenuAction` in App.tsx; [actions.ts](../../src/actions.ts) is the table of key chords that run those same actions
- [Preferences.tsx](../../src/Preferences.tsx), [PrefsUI.tsx](../../src/PrefsUI.tsx); [histogram.ts](../../src/histogram.ts), [probe.ts](../../src/probe.ts); [perf.ts](../../src/perf.ts), [PerfHud.tsx](../../src/PerfHud.tsx); [aiModel.ts](../../src/aiModel.ts); [ImportView.tsx](../../src/ImportView.tsx) (the importer); Statistics: [Statistics.tsx](../../src/Statistics.tsx) (lazy overlay, its fonts and [statistics.css](../../src/statistics.css) load with it), [statsModel.ts](../../src/statsModel.ts) and [statsMath.ts](../../src/statsMath.ts) (pure state, request and chart arithmetic, tested by verify/stats-model.test.mjs), [statsStore.ts](../../src/statsStore.ts) (state and the data hook), [statsApi.ts](../../src/statsApi.ts) (what the view asks of Rust; `statsCommands` in commands.ts), sections and charts in [stats/](../../src/stats); Top images: [TopImagesSection.tsx](../../src/stats/TopImagesSection.tsx), [TopCard.tsx](../../src/stats/TopCard.tsx), [TopStrip.tsx](../../src/stats/TopStrip.tsx), [LabelStrip.tsx](../../src/stats/LabelStrip.tsx) and the React-free [statsTop.ts](../../src/statsTop.ts) (the paged `TopFeed` over `stats_top_page`, strips, keyboard map; verify/stats-top.test.mjs); dev harness `app/harness/statistics.html` on fixtures, and verify/stats-shift.test.mjs (no layout shift); [styles.css](../../src/styles.css)

Rust ([`app/src-tauri/src`](../../src-tauri/src)):

- [settings.rs](../../src-tauri/src/settings.rs): the settings store (`settings.json`; `settings_all`, `settings_set`)
- [main.rs](../../src-tauri/src/main.rs): `main()`, the setup, `AppState`; the commands live in one file per group: [session_cmds.rs](../../src-tauri/src/session_cmds.rs) (open, params, history, render and frames, save, preview), [library_cmds.rs](../../src-tauri/src/library_cmds.rs) (folders, `create_folder`, import, thumbnails, `thumb_threads`, metadata, trash, caches), [export_cmds.rs](../../src-tauri/src/export_cmds.rs) (export, merges, positives, presets files, file pickers), [move_cmds.rs](../../src-tauri/src/move_cmds.rs) (move and copy photos into a folder, checksummed), [immich_cmds.rs](../../src-tauri/src/immich_cmds.rs), [print_cmds.rs](../../src-tauri/src/print_cmds.rs), [edit_cmds.rs](../../src-tauri/src/edit_cmds.rs) (masks, spots, Upright, looks, AgX, film, lenses), [ai_cmds.rs](../../src-tauri/src/ai_cmds.rs) (AI, model downloads), [app_cmds.rs](../../src-tauri/src/app_cmds.rs) (Preferences, performance, GPU check, build, engine restart, native menu); [engine.rs](../../src-tauri/src/engine.rs): the engine child and its protocol
- [bindings.rs](../../src-tauri/src/bindings.rs): `collect_commands!`, the list of every command, and the exported [bindings.ts](../../src/bindings.ts); reply types in `typed_session.rs`, `typed_library.rs`, `typed_export.rs`, `typed_edit.rs`, `typed_app.rs`, `typed_stats.rs` and `typed_catalog.rs` beside it
- Scenes: [scenes.rs](../../src-tauri/src/scenes.rs) (the time-sorted grouping sweep and the thumbnail hash), [scenes_store.rs](../../src-tauri/src/scenes_store.rs) (`scenes.sqlite`: hashes, accepted frames, split and merge overrides), [scenes_rules.rs](../../src-tauri/src/scenes_rules.rs) (the keep rules: pure verdicts and the cull plan items), [scenes_cmds.rs](../../src-tauri/src/scenes_cmds.rs) (`scenes_build`, `scenes_score`, `scenes_accepted`, `scenes_set_accepted`, `scenes_override`, `scenes_rules_preview`, `scenes_rules_plan`, `scenes_cancel`; the `scenes-progress` event)
- local search: [local_search.rs](../../src-tauri/src/local_search.rs) and [local_search_cmds.rs](../../src-tauri/src/local_search_cmds.rs) (vector retrieval), [search_cache.rs](../../src-tauri/src/search_cache.rs), [search_filters.rs](../../src-tauri/src/search_filters.rs), [search_reconcile.rs](../../src-tauri/src/search_reconcile.rs), [search_analysis.rs](../../src-tauri/src/search_analysis.rs), [search_cmds.rs](../../src-tauri/src/search_cmds.rs), [analysis_queue.rs](../../src-tauri/src/analysis_queue.rs) (bounded background work), [library_ids.rs](../../src-tauri/src/library_ids.rs) (handles for cached rows), [cache.rs](../../src-tauri/src/cache.rs) (disk caches), CLIP in [clip_models.rs](../../src-tauri/src/clip_models.rs), [clip_text.rs](../../src-tauri/src/clip_text.rs), [clip_image.rs](../../src-tauri/src/clip_image.rs) and [clip_infer.rs](../../src-tauri/src/clip_infer.rs)
- [memory_budget.rs](../../src-tauri/src/memory_budget.rs) (the one memory budget from RAM x the Memory level; page side [memoryBudget.ts](../../src/memoryBudget.ts)); [immich_search.rs](../../src-tauri/src/immich_search.rs), [immich_upload.rs](../../src-tauri/src/immich_upload.rs); [plugins.rs](../../src-tauri/src/plugins.rs) (Edit In); [listing.rs](../../src-tauri/src/listing.rs) (folder listings with sizes and times); [volumes.rs](../../src-tauri/src/volumes.rs) (drives, and whether each answers); [scale.rs](../../src-tauri/src/scale.rs) (Wayland scale); [renderer.rs](../../src-tauri/src/renderer.rs) (Linux interface EGL selection before GTK starts); [pinch.rs](../../src-tauri/src/pinch.rs) (Linux: a touchpad pinch, taken from WebKitGTK, reaches the page as a ctrl wheel event); [webext/frames.c](../../src-tauri/webext/frames.c) (Linux shared frames); [backup.rs](../../src-tauri/src/backup.rs) (Backup: one .zip of the parts chosen, checked and staged before a restore).

Engine ([`engine/`](../../../engine)):

- [main.c](../../../engine/main.c): `main()`, the `_commands[]` table (`_cmd_<name>` handlers and flags), `_handle`, `_read_requests`, replies, yield, crash handler, memory pressure
- one file per area, with its prefix on what other files call: [session.c](../../../engine/session.c) (`session_`: open and close, four pipes (main, preview, drag, base) over the open photo, set and get, render, save), [library.c](../../../engine/library.c) (`library_`: folders, import, thumbnails, metadata, sidecars), [export.c](../../../engine/export.c) (`export_`: export, frames, positives, HDR and Panorama glue), [masks.c](../../../engine/masks.c) (`masks_`: masks, spots, ranges, zones, sampling, Upright), [ai.c](../../../engine/ai.c) (`ai_`, `HAVE_AI`), [device.c](../../../engine/device.c) (`device_`: OpenCL, GPU check), [looks.c](../../../engine/looks.c) (`looks_`), [film.c](../../../engine/film.c) (`film_`), [clip.c](../../../engine/clip.c) and [clip_contract.h](../../../engine/clip_contract.h) (the `clip_embed` command: CPU image and text embeddings), [tags.c](../../../engine/tags.c) (`tags_analyze`: RAM++ logits on its own serial CPU worker, and `tags_apply`: keyword attach and detach that reports what it changed), [files.c](../../../engine/files.c) (`files_`: the LUT 3D file and Watermark marker choosers, as hashes of the stored names; `lut_files`, `lut_folder`, `watermarks`), [cmapping.c](../../../engine/cmapping.c) (`cmap_`: Color mapping's acquire as source and target, `colormapping_acquire`, and the fields `cm_source` and `cm_target`); [engine.h](../../../engine/engine.h) holds the shared types and prototypes, documents the lock order and checks it in debug builds
- [print.c](../../../engine/print.c) (printer profiles, soft proof, printers, print jobs and PDFs, from darktable's print view), [upright.c](../../../engine/upright.c) (Upright, from ashift.c), [hdrmerge.c](../../../engine/hdrmerge.c) (HDR Merge), [pano.cc](../../../engine/pano.cc) and [dng.c](../../../engine/dng.c) (Panorama Merge), [strip.cc](../../../engine/strip.cc) (export metadata)

Checks and tools: [release asset tests](../../../scripts/test_release_assets.py) (`python3 -I scripts/test_release_assets.py`), [engine-state tests](../../../verify/engine-state.test.mjs) (`node --test verify/engine-state.test.mjs`), [engine-command check](../../../verify/engine-commands.test.mjs) (sent commands exist in the `_commands[]` table in main.c); [engine-protocol.py](../../../verify/engine-protocol.py) (headless golden replies, `--record` / `--compare`), [verify/](../../../verify/README.md) (the release checks: `Session` under Node against the engine), [benchmarking/](../../../benchmarking/README.md), `cargo test` in `app/src-tauri`, `npm run build` (type check), [darktable-pin.md](darktable-pin.md), and darktable's own [dev-doc/](../../../darktable/dev-doc/README.md). CI's checks job runs `cargo test` and the `verify/*.test.mjs` files that need no engine (listed in [build.yml](../../../.github/workflows/build.yml)).

[verify/thumbnail-io.cc](../../../verify/thumbnail-io.cc) is the optional
`immage-thumbnail-check` CMake target: metadata prefixes, late metadata
fallback, embedded JPEG equality, orientation and scaled decoding.


Auto cull (Rust): [cull.rs](../../src-tauri/src/cull.rs) (Laplacian sharpness, exposure, blurry region, perceptual hash and grouping over pixels),
[cull_plan.rs](../../src-tauri/src/cull_plan.rs) (`plan`: sets, keepers, rejects, highlights, decision factors),
[cull_store.rs](../../src-tauri/src/cull_store.rs) (`cull.sqlite`: sessions, decisions, events, analysis cache),
[cull_apply.rs](../../src-tauri/src/cull_apply.rs) (apply and undo through the `Library` trait),
[cull_cmds.rs](../../src-tauri/src/cull_cmds.rs) (`cull_*` commands, the `cull-progress` event, `EngineLibrary` over `meta` / `meta_set`; the analysis reads the engine's `analysis_preview`).

Dev-only harness: [harness.md](harness.md) covers `app/harness/` (the critique, Statistics and AI models pages on
fixture data, their URL parameters, the recorded Statistics fixtures and the no-layout-shift tests
`verify/critique-shift`, `stats-shift` and `models-shift`); it is not part of the production build. `npm run check:harness` type-checks it. The Statistics ground-truth library is generated by [benchmarking/stats-library/](../../../benchmarking/stats-library) into its ignored `out/` (`IMMAGE_STATS_LIBRARY` overrides) for the Rust ground-truth test and `dump_harness_fixtures`.
`benchmarking/nas/` ([README](../../../benchmarking/nas/README.md), `nas_bench.py`) measures a network share's
listing and read behaviour for the Catalog scanner's defaults.

## Keeping this map current

- Add a file to its line here and its entry in [architecture.md](architecture.md); keep this map short, as it is read at the start of every session.
- Refer to code by symbol, not line number.

Statistics (user guide [statistics.md](../statistics.md)): [stats.rs](../../src-tauri/src/stats.rs)
(pure aggregation: buckets, cross-filter masks, quadkey cells, unit tests in `src/stats/tests.rs`,
benchmark `cargo test --release benchmark_300k -- --ignored --nocapture`),
[stats_cmds.rs](../../src-tauri/src/stats_cmds.rs) (`stats_*` commands, the in-memory index over
`SearchCache::stats_page`, the `stats_dirty` event), [typed_stats.rs](../../src-tauri/src/typed_stats.rs)
(request and reply types). The page: [Statistics.tsx](../../src/Statistics.tsx) (the overlay, opened by `statsOpen` in
App.tsx) with one file per section and chart in [stats/](../../src/stats) (`*Section.tsx`, `PlacesMap.tsx`,
`placesGeometry.ts`), [statsModel.ts](../../src/statsModel.ts) (pure state and filters),
[statsStore.ts](../../src/statsStore.ts), [statsApi.ts](../../src/statsApi.ts) (the one interface to the `stats_*`
commands), [statsNormalize.ts](../../src/statsNormalize.ts), [statsTypes.ts](../../src/statsTypes.ts),
[statsMath.ts](../../src/statsMath.ts), [statsExposure.ts](../../src/statsExposure.ts),
[statsTop.ts](../../src/statsTop.ts), [statsFormat.ts](../../src/statsFormat.ts),
[statsGrid.ts](../../src/statsGrid.ts), [statsThumbs.tsx](../../src/statsThumbs.tsx),
[statistics.css](../../src/statistics.css); tests `verify/stats-*.test.mjs`.

Catalog (C1/C2, Rust only so far; plan in notes/nas-catalog-plan.md, notes in notes/catalog-backend-notes.md):
[catalog_db.rs](../../src-tauri/src/catalog_db.rs) (`catalog.sqlite`, migration `LADDER`, one writer
thread, `DirBatch`, `finish_run` sweep, jobs and runs),
[catalog_sources.rs](../../src-tauri/src/catalog_sources.rs) (`MountTable` from mountinfo, `VolumeId`,
sources CRUD, `availability`, `reconcile`, `relink`),
[catalog_scan.rs](../../src-tauri/src/catalog_scan.rs) (`run_scan`, `ScanFs`/`RealFs`, `ScanControl`, checkpoints;
tests in `catalog_scan_tests.rs` with the fault-injecting `catalog_fakefs.rs`; benchmark `bench_500k`),
[catalog_cli.rs](../../src-tauri/src/catalog_cli.rs) (`immage catalog ...`, run from `main()` before Tauri; user guide
[catalog-cli.md](../catalog-cli.md)), [catalog_cmds.rs](../../src-tauri/src/catalog_cmds.rs) and
[typed_catalog.rs](../../src-tauri/src/typed_catalog.rs) (typed commands and the `catalog://scan-progress` /
`catalog://scan-finished` events). volumes.rs: `answering()` runs on a bounded pool, `is_network_fs` knows
autofs-behind, gvfs, mergerfs, ceph, gluster, lustre and rclone.

Catalog metadata (C3; notes in notes/catalog-meta-notes.md): the extractor is a pure function over a head
window, [catalog_meta.rs](../../src-tauri/src/catalog_meta.rs) (`extract` -> `Extract::{Done, NeedMore,
Unsupported, Corrupt}`, the 64 KB / 256 KB / 1 MB `HEAD_LADDER`, JPEG and RAF) with
[catalog_meta_tiff.rs](../../src-tauri/src/catalog_meta_tiff.rs) (IFD0, Exif, GPS, SubIFDs of every TIFF-based RAW),
[catalog_meta_iso.rs](../../src-tauri/src/catalog_meta_iso.rs) (HEIC/HEIF/AVIF, CR3, MP4/MOV) and
[catalog_meta_xmp.rs](../../src-tauri/src/catalog_meta_xmp.rs) (sidecars). The job is
[catalog_meta_job.rs](../../src-tauri/src/catalog_meta_job.rs) (`run_metadata`, the `MetaFs` read trait, `Adaptive`
back-off, `content_key`, `coverage`); renames are [catalog_relink.rs](../../src-tauri/src/catalog_relink.rs). Schema v2
(`v2_metadata` in catalog_db.rs) added the typed columns and `keywords`; `meta_state` is 0 pending, 1 done, 2 failed,
3 unparsed. Tests: `catalog_meta_tests.rs` and `catalog_meta_job_tests.rs` over the byte-exact builders in
`catalog_meta_fixtures.rs` and the fault-injecting `catalog_fakefs.rs` (now also serves file contents).
Commands `metadata_start`, `metadata_jobs_active`; events `catalog://metadata-progress` / `catalog://metadata-finished`;
CLI `immage catalog metadata` and `scan --metadata`. Benchmarks (ignored): `bench_50k`, `raw_samples_header_sizes`
(`IMMAGE_RAW_SAMPLES`).

Catalog thumbnails (C4; notes in notes/catalog-thumbs-notes.md): the job is
[catalog_thumb_job.rs](../../src-tauri/src/catalog_thumb_job.rs) (`run_thumbnails`: reader and decoder pools, the source
ladder in `Ctx::fetch`, `write_batch`, checkpoint and resume); the pure part is
[catalog_thumb_src.rs](../../src-tauri/src/catalog_thumb_src.rs) (`exif_thumbnail`, `render`, `orient`);
[catalog_thumbs.rs](../../src-tauri/src/catalog_thumbs.rs) holds `ThumbConfig` (including `WholeJpegs`, the policy for
reading whole JPEGs, and the engine hook), `Signals` (the app's `busy` flag and visible-photo hints), `Store` (one JPEG per
`photos.id`, atomic writes, free-space floor), `gc`, the link into the grid's cache (`cache_name`, `publish_source`,
`lookup_by_path`: the name is `cache::named` for version 0 and size 240) and `coverage`. Schema v3 (`v3_thumbs`) adds the
`thumbs` table. `cache::DiskCache::adopt_file` links a store file into the library cache; `thumb_cached` in library_cmds.rs
falls back to `catalog_cmds::thumb_fallback`. Commands `thumbnails_start`, `thumbnails_jobs_active`, `thumbnails_busy`,
`thumbnails_hint`, `thumbnails_gc`; events `catalog://thumbnails-progress` / `catalog://thumbnails-finished`; CLI
`immage catalog thumbnails` and `scan --thumbnails`. Tests in `catalog_thumb_tests.rs`; ignored: `bench_20k`,
`raw_samples_preview` (`IMMAGE_RAW_SAMPLES`).

Support bundle: [scrub.rs](../../src-tauri/src/scrub.rs) (the scrubber),
[support_bundle.rs](../../src-tauri/src/support_bundle.rs) (`support_bundle_*` commands, zip),
[SupportBundleDialog.tsx](../../src/SupportBundleDialog.tsx) (review and remove lines); opened from Help
(`support-bundle` in menu.ts) and from the engine-exit status in App.tsx.

Auto cull (page): [cullModel.ts](../../src/cullModel.ts) (pure: scope, chips, `reviewOrder`, review keys, defaults, `applySearch` for Find), [cullStore.ts](../../src/cullStore.ts) (the plan under review; `useDecision` for a thumbnail's badge), [cullDefaults.ts](../../src/cullDefaults.ts), [CullDialog.tsx](../../src/CullDialog.tsx) (setup), `runCull` in [background.ts](../../src/background.ts) (the job and `cull-progress`), [CullPanel.tsx](../../src/CullPanel.tsx) (review, in Find's slot via `renderCull` in App.tsx), [CullInspector.tsx](../../src/CullInspector.tsx) (the lightbox's decision, overrides, critique and overlay, mounted by GridCulling.tsx), the `cull-badge` in Filmstrip.tsx's `FilmCell`, and Find's "Describe the photos in words" row in FindPanel.tsx (`geminiSearch`). Menu `auto-cull` and the toolbar's Auto Cull button send the `auto-cull-open` window event; once a plan exists the button is Review cull and opens the panel (`onReview` in App.tsx).

Gemini (AI grading): [gemini.rs](../../src-tauri/src/gemini.rs) (HTTP with model fallback, critique
schema and sanitizing, critique cache `gemini-critiques.sqlite`, natural-language search mapped onto
`GridFilter`; the key is in the keychain, `app.immage` / `gemini-api-key`) and
[gemini_cmds.rs](../../src-tauri/src/gemini_cmds.rs) (`gemini_key_*`, `gemini_critique`, `gemini_search`);
page side [CullingPrefs.tsx](../../src/CullingPrefs.tsx) and [aiGrading.ts](../../src/aiGrading.ts)
(Preferences > Culling). `scrub.rs` redacts keys.

GUI benchmark resource sampling: [resources.rs](../../src-tauri/src/resources.rs)
records CPU, process memory, Linux threads and available GPU counters on a
background worker; `App.tsx` attaches phase timestamps to report version 5.
The GUI dashboard plots these alongside timing summaries.

AI models (Preferences > General): [packs.rs](../../src-tauri/src/packs.rs) is the generic model pack
pipeline (`PackSpec` registry entry, `PackStore`: download with Range resume, verify, repair, remove,
cancel, `pack-state` / `pack-progress` events, disk accounting; layout `<root>/<revision>/` plus
`.partial-<revision>/`), [clip_models.rs](../../src-tauri/src/clip_models.rs) is its CLIP entry and the CLIP
preparation, [analysis_settings.rs](../../src-tauri/src/analysis_settings.rs) the `ai.*` switches and consents,
[pack_cmds.rs](../../src-tauri/src/pack_cmds.rs) the `pack_*` / `analysis_*` commands. Page:
[AiModelsPrefs.tsx](../../src/AiModelsPrefs.tsx) (`AiModelsView`, connected `AiModelsPrefs`),
[PackCard.tsx](../../src/PackCard.tsx), [PackDialogs.tsx](../../src/PackDialogs.tsx),
[aiPacks.ts](../../src/aiPacks.ts) (state to view, `usePacks`), [aiModels.css](../../src/aiModels.css); a new pack
is a new `PackSpec` in the manager's list in main.rs and an entry in `NAMES`. The harness
[models.html](../../harness/models.html) (`?state=`) and [models-shift.test.mjs](../../../verify/models-shift.test.mjs)
check every state for layout shift.

Module switch: [ModuleSwitch.tsx](../../src/ModuleSwitch.tsx) is the panel icon and menu beside a
multi-module tool's name, with a green dot when `Session.adjustedModules` (masks: `maskAdjusted`) finds
edits in another module (hosts chosen in `PanelView` and `MasksPanel`); its
Set as default writes the Preferences choice.

Module options: [moduleChoice.ts](../../src/moduleChoice.ts) resolves saved
choices and preferences; [hiddenModules.ts](../../src/hiddenModules.ts) stores Advanced mode's hidden modules (`group:op` keys, `immage.hiddenModules`), read live by AdvancedPanel.tsx and edited in ModulePrefs' Advanced modules card; [ModulePrefs.tsx](../../src/ModulePrefs.tsx) displays
`MODULE_CHOICES` and `MASK_MODULE_CHOICES` from the manifest under Global and
Masks. It receives `focusMode` from PreferencesDialog and uses the panels'
normal/Alt-click folding behavior. Closing Preferences reapplies choices to
unused global and mask tools in the current session.
[ModuleFile.tsx](../../src/ModuleFile.tsx) draws those two choosers (`file` marker in advancedLayout.ts; the 3D LUT folder is [lutFolder.ts](../../src/lutFolder.ts), read by settings.rs `LUT_FOLDER` for the engine start).
[ModuleClusters.tsx](../../src/ModuleClusters.tsx) is Color mapping's cluster previews and acquire buttons (`clusters` marker; swatch math in [colormapping.ts](../../src/colormapping.ts); `clears` on the cluster count field).
[ModuleAlign.tsx](../../src/ModuleAlign.tsx) is the 3x3 alignment grid of Composite and Watermark (`align` marker, index mapping in [alignment.ts](../../src/alignment.ts)); [ModulePhoto.tsx](../../src/ModulePhoto.tsx) is Composite's photo choice (`photo` marker on `imgid`: a picker over the folder's photos, passed to the card as `photos`; the engine's `files_overlay_write` in [files.c](../../../engine/files.c) is the write, `Session.reloadAdvanced` reads the card back after a refusal).
[ContrastEqualizer.tsx](../../src/ContrastEqualizer.tsx) edits global or per-mask
`atrous` curves (`EQ_IDS`, `equalizerMap` in manifest.ts); mask choices live
in each `Mask.values`, separate from `Session.values`.
[SaturationCurve.tsx](../../src/SaturationCurve.tsx) edits global or per-mask
`satcurvergb` saturation and brilliance curves (`SATCURVE_IDS`, `satcurveMap`).
[DetailControls.tsx](../../src/DetailControls.tsx) lays out contrast & texture
controls, with Medium/Fine always visible and the rest only when the panel's
advanced toggle is on, and, under the same toggle, Whites' Highlight
Brilliance (`whites_brilliance`, AgX only).
[advanced.tsx](../../src/advanced.tsx) holds `useAdvanced` (per-panel state,
remembered) and `AdvancedToggle` (the title-row icon, with a dot for hidden
edits).

Faces (design in [faces.md](faces.md)): engine [faces.c](../../../engine/faces.c) (`faces_infer`); Rust [faces_models.rs](../../src-tauri/src/faces_models.rs) (pack, engine call), [faces_detect.rs](../../src-tauri/src/faces_detect.rs) (pure detection and alignment maths), [faces_cluster.rs](../../src-tauri/src/faces_cluster.rs), [faces_store.rs](../../src-tauri/src/faces_store.rs) (`faces.sqlite`), [faces_service.rs](../../src-tauri/src/faces_service.rs), [faces_cmds.rs](../../src-tauri/src/faces_cmds.rs); `python3 verify/faces-engine.py`.

Auto-keywords: [ram_models.rs](../../src-tauri/src/ram_models.rs) (the pack pin, vocabulary, preprocessing), [tags_store.rs](../../src-tauri/src/tags_store.rs) (`tags.sqlite`), [tags_service.rs](../../src-tauri/src/tags_service.rs) (the background service), [tags_health.rs](../../src-tauri/src/tags_health.rs) (failure classes, backoff, block, skipped photos), [tags_cmds.rs](../../src-tauri/src/tags_cmds.rs); the page side is [KeywordsCard.tsx](../../src/KeywordsCard.tsx) in AiModelsPrefs.tsx, `AiKeywords` in FindEditors.tsx and the Auto-keywords group in Keywording.tsx (state and split in [keywordGroup.ts](../../src/keywordGroup.ts)); `python3 verify/tags-engine.py` runs the real model through the engine.

Metadata: [CaptureTimeDialog.tsx](../../src/CaptureTimeDialog.tsx) previews and
confirms batch offsets and restoration; App.tsx owns the last batch and calls
`capture_times`. [LocationEditor.tsx](../../src/LocationEditor.tsx) edits GPS via
`meta_set`; [map_tiles.rs](../../src-tauri/src/map_tiles.rs) serves map tiles.
`positive_export` in Rust export_cmds.rs writes and imports a positive; engine export.c
uses `_positive_import` for its neutral starting history. App.tsx handles generated
results and the Open generated photos preference, including results outside
an active Immich search.

Native detail previews: [detailPreview.ts](../../src/detailPreview.ts) identifies controls eligible for the loupe. `Session.draftView` selects native whole-image drafts or the detail crop; release requests the exact full view. Adaptive cached preview interpolation was removed after latency and visual quality regressed.

### Transform sampling regression

`masks_upright` in `engine/masks.c` reads `_STAGE_ASHIFT` through `_working_at`,
before geometry. Rendering the current crop can request an input region with
nonzero x/y, including on EXIF-oriented portraits. Upright forces that region's
producer and later cache entries to rerender, accepts its offset, and passes
x/y/scale to `upright_detect` so detected lines remain in sensor coordinates.
Other working-space consumers retain their origin-zero check. Do not relax
that check for mask sampling: a stale zoom region would clamp mask samples.

Run `python3 verify/transform-sampling.py` after building the engine. This
headless CPU check generates landscape, native portrait, both EXIF quarter
turns and mirrored portrait TIFFs above the 1600-pixel detection limit. It
renders at a different scale, checks all four automatic modes before/after a
crop and after restoring the frame, and applies each result before the next
fit. `--write-corpus benchmarking/corpus` retains the synthetic photos in the
ignored local corpus. This verifies engine sampling, not visible controls,
OpenCL or camera RAW decoding; retain real portrait RAWs for those checks.

Culling lightbox: [CullLightbox.tsx](../../src/CullLightbox.tsx) is its structure (head, stage, report panel, foot; the stage is a fixed mat for the photo), [CullInspector.tsx](../../src/CullInspector.tsx) the report panel (`CullSide`), the photo overlay (`CullOverlay`) and `useCritique`, [critiquePins.ts](../../src/critiquePins.ts) the badge placement. No hover, focus or pin state may change a size or position there. [app/harness/critique.html](../../harness/critique.html) renders those real components with fixture data in a plain browser (`npm run dev`, then `/harness/critique.html?state=result&hover=row2`; parameters in `critique.tsx`), outside the production build; `node --test verify/critique-shift.test.mjs` measures every element before and after each hover, focus and pin in headless Chrome and fails on any movement.

Catalog benchmark: [catalog_bench.rs](../../src-tauri/src/catalog_bench.rs) is `immage catalog bench` (read-only run over `ScanFs`/`MetaFs`, scratch SQLite, aggregated JSON and text report; how to run and read it: [benchmarking/nas/README.md](../../../benchmarking/nas/README.md)).

Bottom toolbar: [Toolbar.tsx](../../src/Toolbar.tsx) is the second row of the `.center` grid in
styles.css (the first is `.center-stage`: viewer, filmstrip, grid, status pills), so no height is
measured in script; `.center` is a size container and the bar adapts with `@container center` rules.
The view tabs (Edit | Grid | Scenes) are no longer in this bar: `VIEW_TABS`, `ViewTabs` and `TitleStrip` (the top strip: title, [MenuBar.tsx](../../src/MenuBar.tsx) menus, tabs; on macOS no menus) are in [ViewTabs.tsx](../../src/ViewTabs.tsx), checked by `app/harness/viewtabs.html` and [view-tabs-shift.test.mjs](../../../verify/view-tabs-shift.test.mjs). The page
[harness/toolbar.html](../../harness/toolbar.html) shows the real bar in the real column, and
[toolbar-shift.test.mjs](../../../verify/toolbar-shift.test.mjs) checks it: geometry against the grid surface,
no sideways scroll from 680px, no movement when the view or state changes, one tab stop, names. The
narrow-column menus (labels, more filters, rating, overflow) are in [ToolbarMenus.tsx](../../src/ToolbarMenus.tsx);
`Toolbar` keeps one tab stop and moves it with the arrow keys.

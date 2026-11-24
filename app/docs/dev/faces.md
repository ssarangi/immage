# Faces (backend)

Finds faces in the user's photos, suggests who they are and groups the unnamed ones. All local; off until
the user turns on **Faces** under Preferences > AI models (`analysis_set` with id `faces`, key
`ai.faces.enabled`). The data (`faces.sqlite`, face crops in `faces-crops/`) are biometric: they live only in
the app's data folder, and `faces_delete_all` removes them and turns the switch off.

## Pieces
- `engine/faces.c`: command `faces_infer`, one float tensor file in, all outputs in a file out, on its own serial
  CPU worker through darktable's AI layer (a model folder needs `config.json` with an `id`). Knows nothing of faces.
- `faces_models.rs`: the pack (YuNet 2023mar + SFace 2021dec, pinned sizes and SHA-256, opencv_zoo LFS media
  host; the URL layout and commit are unverified online) and `run`, the call into the engine. The OCEC eye model
  is not in any pack list (its weights have no stated licence): it is used only if the user puts
  `ocec_s.onnx` (SHA-256 pinned) in `<data>/faces-eyes/models/`.
- `faces_detect.rs`: pure maths: 640 letterbox, YuNet decode, NMS (IoU < 0.30), validity filters, pose, ArcFace
  template alignment, SFace and OCEC inputs.
- `faces_cluster.rs`: `suggest` (best exemplar cosine >= 0.45) and `cluster` (average link, threshold 0.48, size cap 200;
  hashed candidate pairs and per-component agglomeration above 2500 faces, so no all-pairs work on a big library).
- `faces_store.rs`: the store, review states, undo (20 deep, in memory), RAW+JPG companions.
- `faces_service.rs`: the background scan (Catalog or one folder) on the shared analysis queue, then names and groups.
- `faces_cmds.rs`: the typed `faces_*` commands; events `faces-progress`, `faces-changed`.

## Scope and folders
`faces.scope` (settings, written by the page) is "catalog" or "folder". With "folder" only folders given to
`faces_scan_folder` / `faces_scan_folder_with` are scanned (one-off, with or without subfolders, kept across a
restart in faces.sqlite); with "catalog" the Catalog is scanned too. Crops reach the page as bytes
(`faces_crop_bytes`), since the page cannot read file paths.

## Grouping
Unnamed faces are suggested for a person first (best confirmed face, >= suggest). The rest are grouped by
`faces_cluster::group_faces`: quality faces seed groups (average link), the others attach to the best group or stay
loose, then groups whose centroids agree are merged. A group has no size cap below 2000. `faces_merge_suggestions`
offers the pairs below the merge threshold. A review decision triggers the analysis at once. `faces.recognizer`
("sface" default, "arcface" only with the local non-commercial file in `<data>/faces-arcface/models/`) picks the embedding;
thresholds per recognizer are in `faces_models::thresholds`. `cargo test opt_in_regroup_store -- --ignored` regroups a copy of a faces.sqlite and prints counts.

Joining two groups (`faces_merge_groups`) and "Not the same" (`faces_not_same`) are remembered in `faces.sqlite`
(`face_links`, `groups_not_same`, `people_not_same`, schema v2) by faces, not by group id, because groups are recomputed.

## Person page, overlay, Find
Counts of photos are of shots (`shot_key`: folder without a trailing Processed, plus lower-case stem), see the design note. `faces_stats` feeds Preferences > Faces.

`faces_person_photos` and `faces_photo_faces` read faces.sqlite only. Find's `GridFilter.people` is resolved by
`local_search_cmds` into allowed and denied paths (`faces_store::filter_paths`) and checked per photo in
`local_search::retrieve_with_provider`; it does nothing while Faces is off.

## Reprocess, strictness, detection size
`faces_reprocess` marks every photo stale; a re-scan keeps who each face was named as by matching boxes (see the design note).
`faces.strictness` shifts the grouping thresholds by 0.04 either way, `faces.detect_size` (1024, 1600, 2048; larger also
looks at 640 px tiles) is part of the stored model version. `cargo test opt_in_detect_sizes -- --ignored` measures faces found per size.

`faces_set_recognizer` switches the embedding network (ArcFace only with its local file); see the design note.

## Eyes and culling
`faces_culling.rs` (pure) turns a photo's stored faces into `FaceQuality`: primary face (`0.8 * frontal + 0.2 * w * h`), pose
`clamp((frontal - 0.5) * 0.08, +-0.04)`, eye penalty (main subject: -0.15 closed, -0.05 semi; any face: -0.08, -0.03; the
worse eye decides), total clamped to -0.2..+0.04, and the `face_pose` / `eye_state` factors. Only faces of at least 48 px
(width in the scan picture) with frontal >= 0.5 can have an eye verdict. `faces_eyes.rs` stores the verdict per face
(schema v4: `faces.eye_open`, `eye_state`, `eye_model`; `unknown` marks a face that was looked at and cannot be judged),
computes the missing ones for the photos a cull, a Scenes view or `faces_eyes_for` asks about (the user's OCEC file,
2048 px picture; network photos and unscanned photos are skipped) and answers Find's `closedEyes` flag. `cull_plan::plan_with_faces`
adds the adjustment to the frame score (ranking, set winner, highlights) and never rejects on it; `scenes_rules` ranks by
`quality + faceAdjust` while `Rules.preferOpenEyes` is on but judges the minimum quality on the raw score.
`cargo test opt_in_real_eyes -- --ignored --nocapture` (needs the same variables as `opt_in_real_faces`, plus `IMMAGE_FACES_EYES`)
prints the eye step's counts and timings only. Notes: notes/faces-p4-backend-notes.md.

## Network shares
Faces never reads, stats or lists a path on a network share (judged from the mount table, `volumes::MountSnapshot`)
unless the user allowed that source (`faces.network_sources`, `faces_network_source`) or asked for that folder with
`allow_network`. The status poll does not ask network folders whether they answer and caches its answer for 30 s.

## Checks
`cargo test faces_` (pure parts, store, clustering with synthetic embeddings). `python3 verify/faces-engine.py`
(`IMMAGE_FACES_ROOT`, `IMMAGE_TEST_PHOTOS`, optional `IMMAGE_EYES_ROOT`): engine outputs against onnxruntime.
`cargo test opt_in_real_faces -- --ignored --nocapture`: the real pipeline over a photo folder, printing counts and timings only.
Design and decisions: notes/faces-backend-design.md.

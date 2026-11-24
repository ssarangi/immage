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

## Checks
`cargo test faces_` (pure parts, store, clustering with synthetic embeddings). `python3 verify/faces-engine.py`
(`IMMAGE_FACES_ROOT`, `IMMAGE_TEST_PHOTOS`, optional `IMMAGE_EYES_ROOT`): engine outputs against onnxruntime.
`cargo test opt_in_real_faces -- --ignored --nocapture`: the real pipeline over a photo folder, printing counts and timings only.
Design and decisions: notes/faces-backend-design.md.

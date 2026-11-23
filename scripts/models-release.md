# Re-hosting the model packs

`scripts/models-release.sh` stages the RAM++ (Auto-keywords) pack from files already on
disk as the assets of a GitHub release. It checks each file's size and SHA-256 against the
pins in `app/src-tauri/src/ram_models.rs`, writes `SHA256SUMS`, `NOTICE` and the Apache-2.0
text, and prints the `gh release create` command. It never runs `gh`, never touches the
network and never changes the pack folder.

    scripts/models-release.sh                 # uses build/app-runtime/ram-models/*/
    scripts/models-release.sh --pack DIR --out DIR --tag TAG --repo OWNER/NAME

Default tag `models-ram-plus-v1`, repo `ssarangi/immage`. The asset names are the pack's
file names (`model.onnx`, `ram_tag_list.txt`, `ram_tag_list_threshold.txt`), because the app
fetches `{RELEASE_BASE}/{name}`.

Once the release exists, set `RELEASE_BASE` in `ram_models.rs`. The app then tries the
release first and the original host second, with the same size and hash. Only RAM++ is
covered: BioCLIP is not approved to download and OCEC's weights licence is unstated.

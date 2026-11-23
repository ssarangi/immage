#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Stage the RAM++ model pack as the assets of a GitHub release and print the command
# that publishes it. Preparation only: this script never runs gh, never uses the network
# and never uploads anything. See scripts/models-release.md.
#
#   scripts/models-release.sh [--pack DIR] [--out DIR] [--tag TAG] [--repo OWNER/NAME]
#
# --pack  the installed pack folder holding model.onnx, ram_tag_list.txt and
#         ram_tag_list_threshold.txt (default: the first build/app-runtime/ram-models/*/)
# --out   the staging folder, created empty (default: build/models-release/TAG)
# Environment: APACHE_LICENSE = path of the Apache-2.0 text
#              (default /usr/share/common-licenses/Apache-2.0)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/app/src-tauri/src/ram_models.rs"
tag="models-ram-plus-v1"
repo="ssarangi/immage"
pack=""
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --pack) pack="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --tag) tag="$2"; shift 2 ;;
    --repo) repo="$2"; shift 2 ;;
    -h|--help) sed -n 2,13p "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$pack" ] || pack="$(ls -d "$root"/build/app-runtime/ram-models/*/ 2>/dev/null | head -n1 || true)"
[ -d "$pack" ] || { echo "no pack folder: pass --pack DIR" >&2; exit 2; }
[ -n "$out" ] || out="$root/build/models-release/$tag"
licence_src="${APACHE_LICENSE:-/usr/share/common-licenses/Apache-2.0}"
[ -f "$licence_src" ] || { echo "no Apache-2.0 text at $licence_src (set APACHE_LICENSE)" >&2; exit 2; }

# the pins: path, size and sha256 of each PackFile in ram_models.rs ASSETS
pins="$(python3 -I - "$src" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
block = text[text.index("const ASSETS"):text.index("const ORIGINAL_BASE") if "const ORIGINAL_BASE" in text else None]
consts = dict(re.findall(r'pub const (\w+): &str = "([^"]*)";', text))
for m in re.finditer(r'path: ([^,]+),\s*size: (\d+),\s*digest: "([0-9a-f]{64})"', block):
    path = m.group(1).strip()
    path = consts[path] if not path.startswith('"') else path.strip('"')
    print(path, m.group(2), m.group(3))
PY
)"
[ "$(printf '%s\n' "$pins" | wc -l)" -eq 3 ] || { echo "expected 3 pins in $src" >&2; exit 1; }

if [ -e "$out" ] && [ -n "$(ls -A "$out")" ]; then echo "$out exists and is not empty" >&2; exit 1; fi
mkdir -p "$out"

while read -r name size digest; do
  f="$pack/$name"
  [ -f "$f" ] || { echo "missing $f" >&2; exit 1; }
  actual_size="$(stat -c %s "$f")"
  [ "$actual_size" = "$size" ] || { echo "$name: size $actual_size, pinned $size" >&2; exit 1; }
  actual="$(sha256sum "$f" | cut -d' ' -f1)"
  [ "$actual" = "$digest" ] || { echo "$name: sha256 $actual, pinned $digest" >&2; exit 1; }
  echo "ok  $name  $size bytes  $digest"
  # a copy, never a move: the pack folder stays as it is (a hard link where the disk allows)
  ln "$f" "$out/$name" 2>/dev/null || cp "$f" "$out/$name"
done <<< "$pins"

cp "$licence_src" "$out/LICENSE-Apache-2.0.txt"
cat > "$out/NOTICE" <<EON
Immage model pack: RAM++ (Recognize Anything Plus), ONNX export

RAM++ was created by the OPPO research team and is published at
https://huggingface.co/xinyu1205/recognize-anything-plus-model and
https://github.com/xinyu1205/recognize-anything under the Apache License,
Version 2.0 (LICENSE-Apache-2.0.txt in this release).

model.onnx is an ONNX export of those official weights, made by WC-ECL
(https://huggingface.co/WC-ECL/ram-plus-onnx, commit
$(sed -n 's/^pub const REVISION: &str = "\(.*\)";/\1/p' "$src")). It is redistributed here unchanged so that Immage does not
depend on a third-party host. ram_tag_list.txt and ram_tag_list_threshold.txt are the
tag vocabulary and per-tag thresholds shipped with the same model.

Licensed under the Apache License, Version 2.0; the software is distributed on an
"AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND. This release is not
affiliated with or endorsed by OPPO or WC-ECL.
EON

( cd "$out" && sha256sum model.onnx ram_tag_list.txt ram_tag_list_threshold.txt LICENSE-Apache-2.0.txt NOTICE > SHA256SUMS && sha256sum -c SHA256SUMS >/dev/null )

echo
echo "staged in $out"
ls -l "$out"
cat <<EOC

Nothing has been published. To publish, review the folder, then run this yourself:

gh release create $tag \\
  --repo $repo \\
  --title "Immage models: RAM++ (Auto-keywords)" \\
  --notes "ONNX export of RAM++ (Apache-2.0). Verified by SHA256SUMS; see NOTICE." \\
  "$out/model.onnx" \\
  "$out/ram_tag_list.txt" \\
  "$out/ram_tag_list_threshold.txt" \\
  "$out/SHA256SUMS" \\
  "$out/LICENSE-Apache-2.0.txt" \\
  "$out/NOTICE"

Then set RELEASE_BASE in app/src-tauri/src/ram_models.rs (see scripts/models-release.md).
EOC

#!/usr/bin/env bash
# Self-tests for the content and artifact guards. These run on every CI guard
# job so a regex/path change cannot silently weaken the boundary (#33/#35).
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

expect_rc() {
  local want=$1
  shift
  set +e
  "$@" >/dev/null 2>&1
  local got=$?
  set -e
  if ((got != want)); then
    echo "guard-self-test: expected rc=$want, got rc=$got: $*" >&2
    exit 1
  fi
}

# --- scan_personal.py -------------------------------------------------------
printf 'clean text\n' >"$TMP/clean.txt"
expect_rc 0 python3 ci/guard/scan_personal.py "$TMP/clean.txt"
printf '/mnt/x/private\n' >"$TMP/leak.txt"
expect_rc 1 python3 ci/guard/scan_personal.py "$TMP/leak.txt"

# --- scan_magic.py ----------------------------------------------------------
printf 'clean text\n' >"$TMP/clean2.txt"
expect_rc 0 python3 ci/guard/scan_magic.py "$TMP/clean2.txt"
hex=$(printf 'a%.0s' {1..64})
printf 'master_key_00 = %s\n' "$hex" >"$TMP/key.txt"
expect_rc 1 python3 ci/guard/scan_magic.py "$TMP/key.txt"
printf '\x89PNG\r\n\x1a\n' >"$TMP/media.png"
expect_rc 1 python3 ci/guard/scan_magic.py "$TMP/media.png"
# Accepted history must not trip the media check.
expect_rc 0 python3 ci/guard/scan_magic.py --history

# --- check_artifact.sh (evidence) ------------------------------------------
mkdir -p "$TMP/evidence"
printf '\x7fELF' >"$TMP/evidence/abi_guest"
printf 'relative only\n' >"$TMP/evidence/readelf.txt"
expect_rc 0 bash ci/guard/check_artifact.sh "$TMP/evidence" self-test evidence

printf '\x89PNG\r\n\x1a\n' >"$TMP/evidence/shot.png"
expect_rc 1 bash ci/guard/check_artifact.sh "$TMP/evidence" self-test evidence
rm -f "$TMP/evidence/shot.png"

printf '/mnt/x/private\n' >"$TMP/evidence/leak.txt"
expect_rc 1 bash ci/guard/check_artifact.sh "$TMP/evidence" self-test evidence
rm -f "$TMP/evidence/leak.txt"

# A clean archive may carry a bare ELF; an archive with media inside must fail.
# ELF files legitimately contain source paths; only text members are path-checked.
mkdir -p "$TMP/src" "$TMP/arch"
printf '\x7fELF' >"$TMP/src/abi_guest"
printf '\x7fELF/home/runner/.cargo/registry\n' >"$TMP/src/libnvn_host.so"
printf 'ok\n' >"$TMP/src/readelf.txt"
( cd "$TMP/src" && tar czf "$TMP/arch/clean.tar.gz" . )
expect_rc 0 bash ci/guard/check_artifact.sh "$TMP/arch" self-test evidence

printf '\x89PNG\r\n\x1a\n' >"$TMP/src/evil.png"
( cd "$TMP/src" && tar czf "$TMP/arch/bad.tar.gz" . )
expect_rc 1 bash ci/guard/check_artifact.sh "$TMP/arch" self-test evidence

# --- check_header_collision.sh (#38) ---------------------------------------
# A spike file named like a product include/ file lets -I order pick the wrong
# header silently; the spike tree must keep zero collisions.
HDR_REPO=$TMP/hdr-repo
mkdir -p "$HDR_REPO/include/internal" "$HDR_REPO/spike/abi-slice/include" \
  "$HDR_REPO/spike/abi-slice/android" "$HDR_REPO/spike/abi-slice/target"
git -C "$HDR_REPO" init -q
printf '**/target/\n' >"$HDR_REPO/.gitignore"
printf '#pragma once\n' >"$HDR_REPO/include/nva_abi.h"
printf '#pragma once\n' >"$HDR_REPO/include/abi_layout_check.c"
printf '#pragma once\n' >"$HDR_REPO/include/internal/nva_nvdrv.h"
printf '#pragma once\n' >"$HDR_REPO/spike/abi-slice/include/nvn_guest.h"
# Track the product files: in CI they are committed, so the clean case and the
# collision cases must both work through `git ls-files`, not only untracked.
git -C "$HDR_REPO" add -A
expect_rc 0 bash ci/guard/check_header_collision.sh "$HDR_REPO"

# A stray untracked copy is still a collision.
cp "$HDR_REPO/include/nva_abi.h" "$HDR_REPO/spike/abi-slice/include/"
expect_rc 1 bash ci/guard/check_header_collision.sh "$HDR_REPO"
rm "$HDR_REPO/spike/abi-slice/include/nva_abi.h"

# The same collision committed is caught too; one per product file, including
# one outside include/.
collide() {
  local src=$1 dst=$2
  cp "$HDR_REPO/include/$src" "$HDR_REPO/$dst/"
  git -C "$HDR_REPO" add -A
  expect_rc 1 bash ci/guard/check_header_collision.sh "$HDR_REPO"
  rm "$HDR_REPO/$dst/${src##*/}"
  git -C "$HDR_REPO" add -A
}
collide nva_abi.h spike/abi-slice/include
collide abi_layout_check.c spike/abi-slice/include
collide internal/nva_nvdrv.h spike/abi-slice/android

# Gitignored build outputs are not part of the tree.
cp "$HDR_REPO/include/nva_abi.h" "$HDR_REPO/spike/abi-slice/target/"
expect_rc 0 bash ci/guard/check_header_collision.sh "$HDR_REPO"

# An empty product include/ is a broken check, not a pass.
mkdir -p "$TMP/hdr-empty"
git -C "$TMP/hdr-empty" init -q
expect_rc 2 bash ci/guard/check_header_collision.sh "$TMP/hdr-empty"

# A bad root is a broken check, not a pass.
mkdir -p "$TMP/hdr-not-git"
expect_rc 2 bash ci/guard/check_header_collision.sh "$TMP/hdr-not-git"
expect_rc 2 bash ci/guard/check_header_collision.sh "$TMP/does-not-exist"

# --- check_workspace.sh (#39) ---------------------------------------------
# The gate normally asks cargo for metadata; inject root + metadata so the
# guard job can exercise its negative branches without a Rust toolchain.
WS=$TMP/workspace
mkdir -p "$WS"/crates/nva-{abi,pack,rewrite,verify,runtime}
write_workspace_metadata() {
  NAMES="$1" BUILD_SCRIPT="${2:-}" python3 - "$WS" >"$TMP/workspace-metadata.json" <<'PY'
import json, os, sys

root = sys.argv[1]
build_script = os.environ.get("BUILD_SCRIPT", "")
packages = []
for name in os.environ["NAMES"].split():
    targets = []
    if build_script and name == "nva-abi":
        targets.append({"kind": ["custom-build"], "src_path": build_script})
    packages.append({
        "name": name,
        "manifest_path": os.path.join(root, "crates", name, "Cargo.toml"),
        "targets": targets,
    })
print(json.dumps({"packages": packages}))
PY
}
run_workspace_gate() {
  NVA_WORKSPACE_ROOT="$WS" \
    NVA_WORKSPACE_METADATA="$(<"$TMP/workspace-metadata.json")" \
    bash ci/guard/check_workspace.sh
}
write_workspace_metadata "nva-abi nva-pack nva-rewrite nva-verify nva-runtime"
expect_rc 0 run_workspace_gate

# Extra / missing / renamed members all fail (count or name match).
write_workspace_metadata "nva-abi nva-pack nva-rewrite nva-verify nva-runtime nva-shell"
expect_rc 1 run_workspace_gate
write_workspace_metadata "nva-abi nva-pack nva-rewrite nva-verify"
expect_rc 1 run_workspace_gate
write_workspace_metadata "nva-abi nva-pack nva-rewrite nva-verify nva-runtime-renamed"
expect_rc 1 run_workspace_gate

# APK shell inputs anywhere under a member directory are rejected.
write_workspace_metadata "nva-abi nva-pack nva-rewrite nva-verify nva-runtime"
mkdir -p "$WS/crates/nva-runtime/android"
printf '<manifest/>\n' >"$WS/crates/nva-runtime/android/AndroidManifest.xml"
expect_rc 1 run_workspace_gate
rm -f "$WS/crates/nva-runtime/android/AndroidManifest.xml"
rmdir "$WS/crates/nva-runtime/android"

# Android SDK/NDK tools in a member build script are rejected; a plain one is fine.
printf 'fn main() {}\n' >"$WS/crates/nva-abi/build.rs"
write_workspace_metadata "nva-abi nva-pack nva-rewrite nva-verify nva-runtime" \
  "$WS/crates/nva-abi/build.rs"
expect_rc 0 run_workspace_gate
printf 'fn main() { std::process::Command::new("aapt2").status().unwrap(); }\n' \
  >"$WS/crates/nva-abi/build.rs"
expect_rc 1 run_workspace_gate
rm -f "$WS/crates/nva-abi/build.rs"

# A workspace-level cargo config that targets Android is rejected.
write_workspace_metadata "nva-abi nva-pack nva-rewrite nva-verify nva-runtime"
mkdir -p "$WS/.cargo"
printf '[build]\ntarget = "aarch64-linux-android"\n' >"$WS/.cargo/config.toml"
expect_rc 1 run_workspace_gate
rm -f "$WS/.cargo/config.toml"
rmdir "$WS/.cargo"
expect_rc 0 run_workspace_gate

# --- check_doc_refs.py -----------------------------------------------------
# A stale `GATES §x.y` must fail; a valid alias and full-path ref must pass.
DOCS_REPO=$TMP/docrefs
mkdir -p "$DOCS_REPO/docs"
git -C "$DOCS_REPO" init -q
printf '# G\n\n## 1. One\n\n### 1.1 Sub\n' >"$DOCS_REPO/docs/GATES.md"
printf 'ok: GATES §1 and docs/GATES.md §1.1\n' >"$DOCS_REPO/note.md"
git -C "$DOCS_REPO" add -A
expect_rc 0 python3 ci/guard/check_doc_refs.py --root "$DOCS_REPO"
printf 'bad: GATES §1.2\n' >>"$DOCS_REPO/note.md"
git -C "$DOCS_REPO" add -A
expect_rc 1 python3 ci/guard/check_doc_refs.py --root "$DOCS_REPO"
printf 'bad target: docs/MISSING.md §1\n' >"$DOCS_REPO/nope.md"
git -C "$DOCS_REPO" add -A
expect_rc 1 python3 ci/guard/check_doc_refs.py --root "$DOCS_REPO"

echo "guard self-tests OK"

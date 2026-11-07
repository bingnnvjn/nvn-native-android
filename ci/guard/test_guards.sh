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

# --- fixtures (#42) --------------------------------------------------------
# The committed bytes must match a fresh generation, and a hand edit must fail.
expect_rc 0 python3 ci/fixtures/gen_fixtures.py --check

# Two independent processes must write identical bytes (the spec asks for a
# real second generation, not only two in-process builds).
GEN1=$TMP/gen1
GEN2=$TMP/gen2
python3 ci/fixtures/gen_fixtures.py --write --root "$GEN1" >/dev/null
python3 ci/fixtures/gen_fixtures.py --write --root "$GEN2" >/dev/null
diff -r "$GEN1/ci/fixtures" "$GEN2/ci/fixtures"

FX=$TMP/fixtures
mkdir -p "$FX/ci"
cp -a ci/fixtures "$FX/ci/fixtures"
printf ' ' >>"$FX/ci/fixtures/pack-min/load-plan.json"
expect_rc 1 python3 ci/fixtures/gen_fixtures.py --check --root "$FX"

# The fixture names are registered one path at a time. The same blacklisted
# name outside the registration must stay red; a directory-wide exception
# would make this case pass and must fail the self-test. The files are
# committed, so the tree scan and the history scan both see them.
SCAN_REPO=$TMP/scan-repo
mkdir -p "$SCAN_REPO/ci/guard" "$SCAN_REPO/ci/fixtures/pack-min"
cp ci/guard/scan_tree.sh ci/guard/scan_magic.py ci/guard/scan_personal.py \
  ci/guard/allowlist.txt "$SCAN_REPO/ci/guard/"
printf '{"version":1}\n' >"$SCAN_REPO/ci/fixtures/pack-min/load-plan.json"
git -C "$SCAN_REPO" init -q
git -C "$SCAN_REPO" add -A
git -C "$SCAN_REPO" -c user.name=guard -c user.email=guard@example.invalid \
  commit -qm self-test
# scan_tree.sh resolves its root from git, so run it from inside the temp repo.
run_scan() { ( cd "$SCAN_REPO" && bash ci/guard/scan_tree.sh ); }
expect_rc 0 run_scan
mkdir -p "$SCAN_REPO/ci/fixtures/unregistered"
cp "$SCAN_REPO/ci/fixtures/pack-min/load-plan.json" \
  "$SCAN_REPO/ci/fixtures/unregistered/load-plan.json"
git -C "$SCAN_REPO" add -A
git -C "$SCAN_REPO" -c user.name=guard -c user.email=guard@example.invalid \
  commit -qm unregistered
expect_rc 1 run_scan

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

# --- check_layout_assertions.sh (#49) -------------------------------------
# include/abi_layout_check.c holds the only compile-time pin on C ABI v1;
# deleting one assertion used to leave CI green (#47). Every case runs in a
# temp tree on purpose: during the counter-proof run the real tree is one
# assertion short, and a "real tree passes" case here would move that red onto
# this step instead of the inventory step.
LAY=$TMP/layout
mkdir -p "$LAY/include"
LAY_SRC=$LAY/include/abi_layout_check.c
LAY_EXPECT=$TMP/layout-expect.txt
run_layout() { bash ci/guard/check_layout_assertions.sh "$LAY" "$LAY_EXPECT"; }

# Three assertions in two groups. The fence header carries a note after ':'
# like the real file, and the label must still be read as `nva_fence`.
cat >"$LAY_SRC" <<'EOF'
/* ---- nva_fence: 16 B ---- */
NVA_SA(sizeof(nva_fence) == 16, "nva_fence size");
NVA_SA(offsetof(nva_fence, gen) == 12, "fence gen");
/* ---- SVC ---- */
NVA_SA(sizeof(nva_svc_frame) == 96, "svc_frame size");
EOF
printf 'total 3\nnva_fence 2\nSVC 1\n' >"$LAY_EXPECT"
expect_rc 0 run_layout

# One assertion gone: the total and its group drop together.
cat >"$LAY_SRC" <<'EOF'
/* ---- nva_fence: 16 B ---- */
NVA_SA(sizeof(nva_fence) == 16, "nva_fence size");
/* ---- SVC ---- */
NVA_SA(sizeof(nva_svc_frame) == 96, "svc_frame size");
EOF
expect_rc 1 run_layout

# Total unchanged, one group emptied into another: only the per-group split
# catches this, which is why equality is exact rather than "at least one".
cat >"$LAY_SRC" <<'EOF'
/* ---- nva_fence: 16 B ---- */
NVA_SA(sizeof(nva_fence) == 16, "nva_fence size");
NVA_SA(offsetof(nva_fence, gen) == 12, "fence gen");
NVA_SA(sizeof(nva_svc_frame) == 96, "svc_frame size");
/* ---- SVC ---- */
EOF
expect_rc 1 run_layout

# An assertion above the first header belongs to no group.
cat >"$LAY_SRC" <<'EOF'
NVA_SA(offsetof(nva_fence, gen) == 12, "fence gen");
/* ---- nva_fence: 16 B ---- */
NVA_SA(sizeof(nva_fence) == 16, "nva_fence size");
NVA_SA(offsetof(nva_fence, gen) == 12, "fence gen");
/* ---- SVC ---- */
NVA_SA(sizeof(nva_svc_frame) == 96, "svc_frame size");
EOF
expect_rc 1 run_layout

# Same source, an inventory whose total and groups all match: the orphan rule
# alone must still fail, or "every assertion belongs to a group" is untested.
printf 'total 4\nnva_fence 2\nSVC 1\n' >"$TMP/layout-expect-orphan.txt"
expect_rc 1 bash ci/guard/check_layout_assertions.sh "$LAY" "$TMP/layout-expect-orphan.txt"

# A group the inventory never heard of fails on its own, with the total and
# every listed group matching.
cat >"$LAY_SRC" <<'EOF'
/* ---- nva_fence: 16 B ---- */
NVA_SA(sizeof(nva_fence) == 16, "nva_fence size");
NVA_SA(offsetof(nva_fence, gen) == 12, "fence gen");
/* ---- SVC ---- */
NVA_SA(sizeof(nva_svc_frame) == 96, "svc_frame size");
EOF
printf 'total 3\nnva_fence 2\n' >"$TMP/layout-expect-partial.txt"
expect_rc 1 bash ci/guard/check_layout_assertions.sh "$LAY" "$TMP/layout-expect-partial.txt"

# Bad input is exit 2, not a silent pass.
rm "$LAY_SRC"
expect_rc 2 run_layout
expect_rc 2 bash ci/guard/check_layout_assertions.sh "$LAY" "$TMP/no-such-expect.txt"
expect_rc 2 bash ci/guard/check_layout_assertions.sh "$TMP/no-such-root" "$LAY_EXPECT"

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

# --- check_shell_apk.sh ----------------------------------------------------
# It must fail closed: a missing APK, or a missing Android toolchain, has to
# exit 2 instead of silently passing. The positive path needs a real APK, so it
# is exercised by the shell-apk job (and the #46 counter-proof run), not here.
expect_rc 2 bash ci/guard/check_shell_apk.sh "$TMP/no-such.apk"
printf 'not an apk\n' >"$TMP/fake.apk"
expect_rc 2 env ANDROID_HOME=/nonexistent SHELL_NDK_BIN=/nonexistent \
  bash ci/guard/check_shell_apk.sh "$TMP/fake.apk"

echo "guard self-tests OK"

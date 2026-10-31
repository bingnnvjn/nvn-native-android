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
expect_rc 0 bash ci/guard/check_header_collision.sh "$HDR_REPO"

collide() {
  local src=$1 dst=$2
  cp "$HDR_REPO/include/$src" "$HDR_REPO/$dst/"
  expect_rc 1 bash ci/guard/check_header_collision.sh "$HDR_REPO"
  rm "$HDR_REPO/$dst/${src##*/}"
}

# One collision per product file, including one outside include/.
for pair in \
  "nva_abi.h spike/abi-slice/include" \
  "abi_layout_check.c spike/abi-slice/include" \
  "internal/nva_nvdrv.h spike/abi-slice/android"; do
  collide "${pair%% *}" "${pair#* }"
done

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

echo "guard self-tests OK"

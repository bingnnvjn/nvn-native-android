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
mkdir -p "$TMP/src" "$TMP/arch"
printf '\x7fELF' >"$TMP/src/abi_guest"
printf 'ok\n' >"$TMP/src/readelf.txt"
( cd "$TMP/src" && tar czf "$TMP/arch/clean.tar.gz" . )
expect_rc 0 bash ci/guard/check_artifact.sh "$TMP/arch" self-test evidence

printf '\x89PNG\r\n\x1a\n' >"$TMP/src/evil.png"
( cd "$TMP/src" && tar czf "$TMP/arch/bad.tar.gz" . )
expect_rc 1 bash ci/guard/check_artifact.sh "$TMP/arch" self-test evidence

echo "guard self-tests OK"

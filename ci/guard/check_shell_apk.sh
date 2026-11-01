#!/usr/bin/env bash
# Product shell APK assertions (#46, docs/GATES.md §3, docs/ARCHITECTURE.md §3.7).
#
# Usage: check_shell_apk.sh <apk>
#
# Pins what GATES §3 asks of the shell: product package, minSdk 26, no
# `debuggable` attribute, and a `JNI_OnLoad` export. Package/minSdk/debuggable
# are read from the *binary* manifest (`aapt2 dump xmltree`), not from
# `dump badging`: build-tools 36 already dropped badging fields once
# (docs/PIPELINE.md §9) and a missing field would turn an assertion into a
# no-op that always passes.
#
# Certificate, permissions, ABI and the v2/v3 schemes stay in
# ci/guard/check_manifest.sh (GUARD_MODE=product); this script adds the two
# assertions that one cannot make (xmltree debuggable, JNI_OnLoad).
#
# Environment:
#   ANDROID_HOME / ANDROID_SDK_ROOT  aapt2 (same discovery as check_manifest.sh)
#   SHELL_NDK_BIN                    NDK llvm-nm directory
#   GUARD_PACKAGE / GUARD_MIN_SDK    overrides, same names as check_manifest.sh
set -euo pipefail

APK=${1:?usage: check_shell_apk.sh <apk>}
[[ -f $APK ]] || { echo "guard: no such APK: $APK" >&2; exit 2; }

EXPECT_PACKAGE=${GUARD_PACKAGE:-com.bingnnvjn.nva}
EXPECT_MIN=${GUARD_MIN_SDK:-26}

SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
BT=$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1 || true)
[[ -x ${BT:-}/aapt2 ]] || { echo "guard: aapt2 not found under $SDK" >&2; exit 2; }

NDK_BIN=${SHELL_NDK_BIN:-}
[[ -x ${NDK_BIN:-}/llvm-nm ]] || {
  echo "guard: llvm-nm not found (set SHELL_NDK_BIN to the NDK toolchain bin)" >&2
  exit 2
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail=0
report() { printf 'SHELL-RED-LINE: %s\n' "$*" >&2; fail=1; }

"$BT/aapt2" dump xmltree --file AndroidManifest.xml "$APK" > "$TMP/xmltree.txt" 2>&1 || {
  echo "guard: aapt2 dump xmltree failed" >&2
  cat "$TMP/xmltree.txt" >&2
  exit 2
}
"$BT/aapt2" dump badging "$APK" > "$TMP/badging.txt" 2>&1 || {
  echo "guard: aapt2 dump badging failed" >&2
  exit 2
}

# `aapt2` prints an int attribute either as `=26` or as `=(type 0x10)0x1a`
# depending on build-tools. Decode both, and fail loudly on anything else so a
# format change can never be mistaken for "value matched".
read_int() { # read_int <attribute-suffix> -> decimal on stdout, 1 on failure
  local raw dec hex
  raw=$(sed -n "s/.*:$1([^)]*)=\(.*\)/\1/p" "$TMP/xmltree.txt" | head -1)
  [[ -n $raw ]] || return 1
  case $raw in
    '('*')'0x*)
      hex=${raw##*')0x'}
      dec=$(( 16#$hex ))
      ;;
    [0-9]*) dec=$raw ;;
    *) return 1 ;;
  esac
  [[ $dec =~ ^[0-9]+$ ]] || return 1
  printf '%s' "$dec"
}

pkg=$(sed -n 's/.*A: package="\([^"]*\)".*/\1/p' "$TMP/xmltree.txt" | head -1)
[[ -n $pkg ]] || report "no package attribute in AndroidManifest.xml"
[[ $pkg == "$EXPECT_PACKAGE" ]] || report "package is '$pkg', expected '$EXPECT_PACKAGE'"

if min=$(read_int minSdkVersion); then
  [[ $min == "$EXPECT_MIN" ]] || report "minSdk is '$min', expected '$EXPECT_MIN'"
else
  report "cannot read minSdkVersion from the binary manifest"
fi

# GATES §3: "no debuggable". An absent attribute is the platform default
# `false`; an explicit `false` (TYPE_INT_BOOLEAN 0) is accepted too, so this
# assertion cannot be satisfied by a badging field going missing.
if grep -q ':debuggable(' "$TMP/xmltree.txt"; then
  deb=$(sed -n 's/.*:debuggable([^)]*)=\(.*\)/\1/p' "$TMP/xmltree.txt" | head -1)
  # Current build-tools print a boolean as `=true` / `=false`; older ones as
  # `(type 0x12)0x0`. Accept both spellings of false — an explicit
  # `debuggable="false"` is as good as an absent attribute (GATES §3 wants
  # "no debuggable", and the platform default is false).
  case $deb in
    *0x0 | false) : ;;
    *) report "android:debuggable is '$deb', expected absent or false" ;;
  esac
fi

python3 - "$APK" > "$TMP/entries.txt" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    for name in z.namelist():
        print(name)
PY

bad_abi=$(grep '^lib/' "$TMP/entries.txt" | grep -v '^lib/arm64-v8a/' || true)
[[ -z $bad_abi ]] || report "native libraries outside lib/arm64-v8a: $bad_abi"

found_jni_onload=0
while IFS= read -r entry; do
  [[ -z $entry ]] && continue
  unzip -p "$APK" "$entry" > "$TMP/lib.so"
  if "$NDK_BIN/llvm-nm" -D --defined-only "$TMP/lib.so" 2>/dev/null \
       | grep -q ' T JNI_OnLoad$'; then
    echo "guard: JNI_OnLoad exported by $entry"
    found_jni_onload=1
    break
  fi
done < <(grep '^lib/arm64-v8a/.*\.so$' "$TMP/entries.txt" || true)
((found_jni_onload)) \
  || report "no JNI_OnLoad export in lib/arm64-v8a/*.so (llvm-nm -D)"

if ((fail)); then
  echo "guard: shell APK FAILED the #46 assertions (docs/GATES.md §3)." >&2
  echo "--- xmltree ---" >&2
  cat "$TMP/xmltree.txt" >&2
  echo "--- entries ---" >&2
  cat "$TMP/entries.txt" >&2
  exit 1
fi
echo "guard: shell APK OK (package=$pkg minSdk=$min debuggable=absent JNI_OnLoad=yes)"

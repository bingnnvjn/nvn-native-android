#!/usr/bin/env bash
# APK manifest + signing certificate guard.
#
# Usage: GUARD_MODE=artifact|release check_manifest.sh <apk>
#
# Pins the things that must not drift between runs (#21 §4.2). For artifact
# mode the certificate must EQUAL ci/signing/fingerprint.txt; for release mode
# it must DIFFER (the artifact key is public by design and can never be a
# release key).
set -euo pipefail

APK=${1:?usage: check_manifest.sh <apk>}
MODE=${GUARD_MODE:-artifact}
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)

SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
BT=$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1 || true)
[[ -x $BT/aapt2 ]] || { echo "guard: aapt2 not found under $SDK" >&2; exit 2; }

fail=0
report() { printf 'MANIFEST-RED-LINE: %s\n' "$*" >&2; fail=1; }

EXPECT_PACKAGE=${GUARD_PACKAGE:-com.bingnnvjn.abislice}
EXPECT_MIN=${GUARD_MIN_SDK:-26}
EXPECT_TARGET=${GUARD_TARGET_SDK:-34}

"$BT/aapt2" dump badging "$APK" > /tmp/badging.txt 2>&1 || {
  echo "guard: aapt2 dump badging failed" >&2; exit 2; }

pkg=$(sed -n "s/^package: name='\([^']*\)'.*/\1/p" /tmp/badging.txt)
min=$(sed -n "s/^sdkVersion:'\([0-9]*\)'.*/\1/p" /tmp/badging.txt)
tgt=$(sed -n "s/^targetSdkVersion:'\([0-9]*\)'.*/\1/p" /tmp/badging.txt)
nat=$(sed -n "s/^native-code: \(.*\)/\1/p" /tmp/badging.txt)

[[ $pkg == "$EXPECT_PACKAGE" ]] || report "package is '$pkg', expected '$EXPECT_PACKAGE'"
[[ $min == "$EXPECT_MIN" ]] || report "minSdk is '$min', expected '$EXPECT_MIN'"
[[ $tgt == "$EXPECT_TARGET" ]] || report "targetSdk is '$tgt', expected '$EXPECT_TARGET'"
[[ $nat == "'arm64-v8a'" ]] || report "native-code is '$nat', expected only 'arm64-v8a'"

# No dangerous permissions in the spike. The product shell will need a narrow,
# reviewed set (see #20 L6) — widen this list deliberately, never silently.
# Permission policy differs by mode: the spike needs none; the product shell
# needs exactly the L6 set (SAF import + FGS for import/prewarm), nothing else.
perm_ok() {
  case $1 in
    "android.permission.FOREGROUND_SERVICE"|"android.permission.FOREGROUND_SERVICE_SPECIAL_USE") return 0 ;;
    *) return 1 ;;
  esac
}
if grep -q '^uses-permission:' /tmp/badging.txt; then
  if [[ $MODE != product ]]; then
    report "unexpected permissions in $MODE mode:"
    grep '^uses-permission:' /tmp/badging.txt >&2
  else
    while IFS= read -r line; do
      perm=${line##*name=\'}
      perm=${perm%%\'*}
      perm_ok "$perm" || report "permission outside the L6 whitelist: $perm"
    done < <(grep '^uses-permission:' /tmp/badging.txt)
  fi
fi

debuggable=$(grep -c "application-debuggable" /tmp/badging.txt || true)
case $MODE in
  artifact) : ;; # spike artifact may be debuggable
  release|product)
    ((debuggable == 0)) || report "$MODE APK must not be debuggable" ;;
  *) echo "guard: unknown GUARD_MODE=$MODE" >&2; exit 2 ;;
esac

# --- signing certificate ----------------------------------------------------
"$BT/apksigner" verify -v "$APK" > /tmp/verify.txt 2>&1 || {
  echo "guard: apksigner verify failed" >&2; cat /tmp/verify.txt >&2; exit 1; }
"$BT/apksigner" verify --print-certs "$APK" > /tmp/certs.txt 2>&1 || {
  echo "guard: apksigner --print-certs failed" >&2; cat /tmp/certs.txt >&2; exit 1; }

# minSdk 26 produces v2+v3; v1 cannot exist and must not be asserted.
for scheme in v2 v3; do
  grep -q "Verified using $scheme scheme" /tmp/verify.txt \
    || report "no $scheme signature"
  grep -q "Verified using $scheme scheme.*: true" /tmp/verify.txt \
    || report "$scheme signature did not verify"
done

# The digest line is emitted per signer version ("V3.0 Signer: certificate
# SHA-256 digest: ..." on newer build-tools), so match the suffix only.
digest=$(sed -n 's/.*certificate SHA-256 digest: //p' /tmp/certs.txt | head -1 | tr 'A-F' 'a-f')
[[ -n $digest ]] || { echo "guard: no signer digest" >&2; exit 1; }

pinned_file="$ROOT/ci/signing/fingerprint.txt"
pinned=$(head -1 "$pinned_file" 2>/dev/null | tr 'A-F' 'a-f' | tr -d ':[:space:]')

case $MODE in
  artifact|product)
    [[ $digest == "$pinned" ]] || report "certificate mismatch: $digest != $pinned"
    ;;
  release)
    [[ $digest != "$pinned" ]] || report "release APK is signed with the public artifact key"
    ;;
esac

if ((fail)); then
  echo "guard: manifest/certificate FAILED (docs/PIPELINE.md §4)." >&2
  exit 1
fi
echo "guard: manifest and certificate OK ($pkg, $MODE, ${digest:0:16}…)"

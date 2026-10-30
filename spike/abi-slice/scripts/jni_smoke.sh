#!/usr/bin/env bash
# Linux JNI smoke: same jni.c + guest.c as the APK, no Android device.
# Requires: cc, javac/java, libnvn_host.so, a working Vulkan ICD (lavapipe in CI).
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if [ -n "${JAVA_HOME:-}" ] && [ -x "$JAVA_HOME/bin/javac" ]; then
    JAVAC="$JAVA_HOME/bin/javac"
else
    JAVAC=$(command -v javac || true)
fi
if [ -z "$JAVAC" ]; then
    echo "error: javac not found" >&2
    exit 127
fi
JAVA_HOME=${JAVA_HOME:-$(dirname "$(dirname "$(readlink -f "$JAVAC")")")}
JAVA="$JAVA_HOME/bin/java"

LIB=${NVA_HOST_LIB:-target/release/libnvn_host.so}
if [ ! -f "$LIB" ]; then
    echo "error: $LIB not found; run cargo build --release first" >&2
    exit 1
fi

OUT=${OUT:-dist/jni}
rm -rf "$OUT"
mkdir -p "$OUT/classes"
cp "$LIB" "$OUT/libnvn_host.so"

echo "==> compile libabi_slice_jni.so (Linux)"
cc -shared -fPIC -O2 -fno-stack-protector \
    -Dmain=abi_slice_guest_main \
    -Iinclude -I"$JAVA_HOME/include" -I"$JAVA_HOME/include/linux" \
    android/jni/abi_slice_jni.c guest/guest.c \
    -o "$OUT/libabi_slice_jni.so" \
    -L"$(dirname "$LIB")" -lnvn_host -ldl -lm -lpthread \
    -Wl,-rpath,'$ORIGIN'

echo "==> compile Java harness"
"$JAVAC" -source 8 -target 8 -Xlint:-options \
    -d "$OUT/classes" \
    android/jni_smoke/com/bingnnvjn/abislice/MainActivity.java

CACHE=$(mktemp -d)
trap 'rm -rf "$CACHE"' EXIT

echo "==> run"
set +e
LD_LIBRARY_PATH="$OUT:${LD_LIBRARY_PATH:-}" \
    "$JAVA" -Djava.library.path="$OUT" -cp "$OUT/classes" \
    com.bingnnvjn.abislice.MainActivity "$CACHE" \
    > "$OUT/jni.log" 2>&1
rc=$?
set -e
cat "$OUT/jni.log"

grep -q '"mode":"all".*"ok":true' "$OUT/jni.log"
grep -q '"mode":"gate".*"ok":true' "$OUT/jni.log"
grep -q 'signal=6' "$OUT/jni.log"
if [ "$rc" -ne 0 ]; then
    echo "FAIL: java exit=$rc" >&2
    exit 1
fi
echo "JNI SMOKE PASS"

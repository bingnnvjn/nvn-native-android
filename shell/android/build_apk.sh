#!/usr/bin/env bash
# Build the product shell APK without Gradle (#46).
#
# Same no-Gradle path as spike/abi-slice/android/build_apk.sh
# (docs/PIPELINE.md §9: aapt2 link -> javac -> d8 -> zipalign -> apksigner).
# The spike script stays untouched on purpose: it is the #30 evidence tree, and
# #37 D15 says the product shell reuses the *path*, not the code. Anything the
# two builds share is documented in PIPELINE §9 instead.
#
# Inputs:
#   ANDROID_HOME / ANDROID_SDK_ROOT  (the GitHub runner image has this)
#   ANDROID_NDK_HOME                 (nttld/setup-ndk)
#   CC_aarch64_linux_android         (NDK clang, API 26)
# Output:
#   shell/dist/android-apk/nva-shell.apk plus the text evidence next to it
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd) # shell/
cd "$ROOT"

SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
if [ -z "$SDK" ] || [ ! -d "$SDK" ]; then
    echo "error: ANDROID_HOME/ANDROID_SDK_ROOT is not set" >&2
    exit 1
fi

BT=$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1 || true)
PLATFORM=$(ls -d "$SDK"/platforms/android-* 2>/dev/null | sort -V | tail -1 || true)
if [ -z "$BT" ] || [ -z "$PLATFORM" ]; then
    echo "error: no build-tools/platform under $SDK" >&2
    exit 1
fi

NDK=${ANDROID_NDK_HOME:-}
CC=${CC_aarch64_linux_android:-}
if [ -z "$CC" ] && [ -n "$NDK" ]; then
    CC="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"
fi
if [ -z "$CC" ] || [ ! -x "$CC" ]; then
    echo "error: aarch64-linux-android26-clang not found" >&2
    exit 1
fi

APK_NAME=${APK_NAME:-nva-shell.apk}
OUT=${OUT:-dist/android-apk}
OUT=$(realpath -m "$OUT")
BUILD="$OUT/build"
APKROOT="$BUILD/apkroot"

rm -rf "$OUT"
mkdir -p "$OUT/lib/arm64-v8a" "$BUILD/classes" "$BUILD/dex" "$APKROOT/lib/arm64-v8a"

echo "==> native library"
# -ffixed-x18 is the project-wide arm64 rule (docs/GATES.md §3). Nothing here
# touches the C ABI; the only export that matters is JNI_OnLoad.
"$CC" -std=c11 -shared -fPIC -O2 -fno-stack-protector -ffixed-x18 \
    -Wl,-soname,libnva_shell_jni.so \
    android/jni/nva_shell_jni.c \
    -o "$OUT/lib/arm64-v8a/libnva_shell_jni.so" \
    -llog

echo "==> Java"
javac -source 8 -target 8 -Xlint:-options \
    -bootclasspath "$PLATFORM/android.jar" \
    -d "$BUILD/classes" \
    android/java/com/bingnnvjn/nva/MainActivity.java

echo "==> dex"
mapfile -t CLASS_FILES < <(find "$BUILD/classes" -name '*.class' | sort)
if [ "${#CLASS_FILES[@]}" -eq 0 ]; then
    echo "error: javac produced no classes" >&2
    exit 1
fi
"$BT/d8" --lib "$PLATFORM/android.jar" --min-api 26 \
    --output "$BUILD/dex" \
    "${CLASS_FILES[@]}"

echo "==> resources / manifest"
# targetSdk 34 is the spike value and stays until L6 decides 36
# (docs/PIPELINE.md §4.2, #37 D15). minSdk 26 is fixed by docs/GATES.md §3.
"$BT/aapt2" link -o "$BUILD/base.apk" \
    -I "$PLATFORM/android.jar" \
    --manifest android/AndroidManifest.xml \
    --min-sdk-version 26 --target-sdk-version 34 \
    --version-code 1 --version-name 1.0

cp "$BUILD/base.apk" "$BUILD/unsigned.apk"
cp "$BUILD/dex/classes.dex" "$APKROOT/classes.dex"
cp "$OUT/lib/arm64-v8a/libnva_shell_jni.so" "$APKROOT/lib/arm64-v8a/"
( cd "$APKROOT" && zip -q -r "$BUILD/unsigned.apk" . )

echo "==> align + sign"
"$BT/zipalign" -p -f 4 "$BUILD/unsigned.apk" "$BUILD/aligned.apk"

# The fixed, PUBLIC artifact key (docs/PIPELINE.md §4.1). It is not a release
# key. Unlike the spike build there is no debug-key fallback: a shell signed
# with a throwaway key would fail the certificate assertion downstream, and a
# red guard two steps later is worse than failing here.
REPO=$(cd "$ROOT/.." && pwd)
KS=${SIGNING_KEYSTORE:-$REPO/ci/signing/artifact.p12}
KS_PASS=${SIGNING_KEYSTORE_PASS:-nva-artifact}
KS_ALIAS=${SIGNING_KEYSTORE_ALIAS:-nva-artifact}
if [ ! -f "$KS" ]; then
    echo "error: artifact keystore missing: $KS (docs/PIPELINE.md §4.1)" >&2
    exit 1
fi
# v2/v3 only. v1 (JAR signing) is a no-op at minSdk 26 — see PIPELINE §9.
"$BT/apksigner" sign \
    --ks "$KS" --ks-type PKCS12 \
    --ks-pass "pass:$KS_PASS" --key-pass "pass:$KS_PASS" \
    --ks-key-alias "$KS_ALIAS" \
    --v2-signing-enabled true --v3-signing-enabled true --v4-signing-enabled false \
    --min-sdk-version 26 \
    --out "$OUT/$APK_NAME" "$BUILD/aligned.apk"
"$BT/apksigner" verify --verbose --min-sdk-version 26 "$OUT/$APK_NAME" \
    > "$OUT/apksigner-verify.txt" 2>&1
"$BT/apksigner" verify --print-certs "$OUT/$APK_NAME" \
    >> "$OUT/apksigner-verify.txt" 2>&1
cat "$OUT/apksigner-verify.txt"
for scheme in v2 v3; do
    grep -q "Verified using $scheme scheme.*: true" "$OUT/apksigner-verify.txt" \
        || { echo "error: $scheme signature not verified" >&2; exit 1; }
done

echo "==> inspect"
"$BT/aapt2" dump badging "$OUT/$APK_NAME" > "$OUT/badging.txt" 2>&1 || true
# The binary manifest is the assertion source for ci/guard/check_shell_apk.sh:
# `dump badging` drops sdkVersion/targetSdkVersion on build-tools 36
# (docs/PIPELINE.md §9), so the xmltree dump is kept as evidence too.
"$BT/aapt2" dump xmltree --file AndroidManifest.xml "$OUT/$APK_NAME" \
    > "$OUT/xmltree.txt" 2>&1
# Relative paths on purpose: `unzip -l` echoes its argument, and an absolute
# path would leak the build host into the artifact (PIPELINE §9).
( cd "$OUT" && unzip -l "$APK_NAME" > unzip-l.txt 2>&1 ) || true
grep -E 'package:|launchable-activity:|native-code:' "$OUT/badging.txt" || true
grep -E 'classes.dex|lib/arm64-v8a' "$OUT/unzip-l.txt" || true
( cd "$OUT" && sha256sum "$APK_NAME" > sha256sum.txt )
ls -lh "$OUT/$APK_NAME"

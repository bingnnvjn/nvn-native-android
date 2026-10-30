#!/usr/bin/env bash
# Build the #30 ABI-slice APK without Gradle.
#
# Inputs:
#   ANDROID_HOME / ANDROID_SDK_ROOT  (GitHub runner has this)
#   ANDROID_NDK_HOME                 (nttld/setup-ndk)
#   CC_aarch64_linux_android         (NDK clang)
#   TARGET_DIR                       (Cargo target dir, default target/aarch64-linux-android/release)
# Output:
#   dist/android-apk/abi-slice.apk
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
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

TARGET_DIR=${TARGET_DIR:-target/aarch64-linux-android/release}
OUT=${OUT:-dist/android-apk}
BUILD="$OUT/build"
APKROOT="$BUILD/apkroot"

rm -rf "$OUT"
mkdir -p "$OUT/lib/arm64-v8a" "$BUILD/classes" "$BUILD/dex" "$APKROOT/lib/arm64-v8a"

echo "==> native libraries"
cp "$TARGET_DIR/libnvn_host.so" "$OUT/lib/arm64-v8a/"
# guest.c defines main; rename it so JNI can call it as a normal function.
"$CC" -shared -fPIC -O2 -fno-stack-protector -ffixed-x18 -DNVN_X18_RESERVED=1 \
    -Dmain=abi_slice_guest_main \
    -Iinclude \
    android/jni/abi_slice_jni.c guest/guest.c \
    -o "$OUT/lib/arm64-v8a/libabi_slice_jni.so" \
    -L"$TARGET_DIR" -lnvn_host -llog -ldl -lm \
    -Wl,-soname,libabi_slice_jni.so

readelf -d "$OUT/lib/arm64-v8a/libabi_slice_jni.so" | \
    grep -E 'NEEDED|SONAME' > "$OUT/readelf-jni.txt" || true
cat "$OUT/readelf-jni.txt"
if ! grep -q 'libnvn_host.so' "$OUT/readelf-jni.txt"; then
    echo "error: libabi_slice_jni.so has no NEEDED libnvn_host.so" >&2
    exit 1
fi

echo "==> Java"
javac -source 8 -target 8 -Xlint:-options \
    -bootclasspath "$PLATFORM/android.jar" \
    -d "$BUILD/classes" \
    android/java/com/bingnnvjn/abislice/MainActivity.java

echo "==> dex"
"$BT/d8" --lib "$PLATFORM/android.jar" --min-api 26 \
    --output "$BUILD/dex" \
    $(find "$BUILD/classes" -name '*.class' | sort)

echo "==> resources / manifest"
"$BT/aapt2" link -o "$BUILD/base.apk" \
    -I "$PLATFORM/android.jar" \
    --manifest android/AndroidManifest.xml \
    --min-sdk-version 26 --target-sdk-version 34 \
    --version-code 1 --version-name 1.0

cp "$BUILD/base.apk" "$BUILD/unsigned.apk"
cp "$BUILD/dex/classes.dex" "$APKROOT/classes.dex"
cp "$OUT/lib/arm64-v8a/libnvn_host.so" "$APKROOT/lib/arm64-v8a/"
cp "$OUT/lib/arm64-v8a/libabi_slice_jni.so" "$APKROOT/lib/arm64-v8a/"
( cd "$APKROOT" && zip -q -r "$BUILD/unsigned.apk" . )

echo "==> align + sign"
"$BT/zipalign" -p -f 4 "$BUILD/unsigned.apk" "$BUILD/aligned.apk"

KS="$BUILD/debug.keystore"
if [ ! -f "$KS" ]; then
    keytool -genkeypair -keystore "$KS" -storepass android -keypass android \
        -alias androiddebugkey -dname "CN=Android Debug,O=Android,C=US" \
        -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1
fi
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android \
    --min-sdk-version 26 --out "$OUT/abi-slice.apk" "$BUILD/aligned.apk"
"$BT/apksigner" verify --verbose --min-sdk-version 26 "$OUT/abi-slice.apk" \
    > "$OUT/apksigner-verify.txt" 2>&1
cat "$OUT/apksigner-verify.txt"

echo "==> inspect"
"$BT/aapt2" dump badging "$OUT/abi-slice.apk" > "$OUT/badging.txt" 2>&1 || true
unzip -l "$OUT/abi-slice.apk" > "$OUT/unzip-l.txt" 2>&1 || true
grep -E 'package:|launchable-activity:|native-code:' "$OUT/badging.txt" || true
grep -E 'classes.dex|lib/arm64-v8a' "$OUT/unzip-l.txt" || true
sha256sum "$OUT/abi-slice.apk" > "$OUT/sha256sum.txt"
ls -lh "$OUT/abi-slice.apk"

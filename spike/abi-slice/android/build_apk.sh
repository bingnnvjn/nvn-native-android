#!/usr/bin/env bash
# Build the #30 ABI-slice APK without Gradle.
#
# Inputs:
#   ANDROID_HOME / ANDROID_SDK_ROOT  (GitHub runner has this)
#   ANDROID_NDK_HOME                 (nttld/setup-ndk)
#   CC_aarch64_linux_android         (NDK clang)
#   TARGET_DIR                       (Cargo target dir, default target/aarch64-linux-android/release)
#   VVL_VERSION                      (optional Khronos validation layer version, e.g. 1.4.350.0)
#   VVL_ARCHIVE                      (optional local tarball; skips the download)
#   VVL_SHA256                       (optional archive hash override; pinned per version)
#   VVL_BASE_URL                     (optional single download base; default GitHub + mirrors)
#   VVL_CACHE                        (optional download cache, default dist/vvl)
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
OUT=$(realpath -m "$OUT")
BUILD="$OUT/build"
APKROOT="$BUILD/apkroot"

# Optional Khronos validation layer (#31). Empty VVL_VERSION keeps the old APK
# shape; set it to bundle libVkLayer_khronos_validation.so for the device.
#
# The archive is pinned by SHA-256, so a truncated or wrong download never
# reaches the APK. Inputs, in order: VVL_ARCHIVE (local file), the cache, then
# VVL_BASE_URL (single override) or GitHub plus mirrors.
VVL_VERSION=${VVL_VERSION:-}
VVL_CACHE=${VVL_CACHE:-dist/vvl}
VVL_ARCHIVE=${VVL_ARCHIVE:-}
VVL_BASE_URL=${VVL_BASE_URL:-}
VVL_SHA256=${VVL_SHA256:-}
VVL_SO=

# Known releases. Add a line per version, or pass VVL_SHA256 explicitly.
case "$VVL_VERSION" in
    1.4.350.0) VVL_SHA256=${VVL_SHA256:-ea64e96a7c0c4a3711a506b9bc769a0eadc9924f389504051ac1f392213d1b31} ;;
esac

vvl_check() {  # vvl_check <file>
    printf '%s  %s\n' "$VVL_SHA256" "$1" | sha256sum -c --status -
}

vvl_sources() {
    local gh="https://github.com/KhronosGroup/Vulkan-ValidationLayers/releases/download/vulkan-sdk-$VVL_VERSION"
    if [ -n "$VVL_BASE_URL" ]; then
        printf '%s\n' "$VVL_BASE_URL"
    else
        printf '%s\n' \
            "$gh" \
            "https://ghfast.top/$gh" \
            "https://ghproxy.net/$gh"
    fi
}

bundle_validation_layer() {
    [ -n "$VVL_VERSION" ] || return 0
    if [ -z "$VVL_SHA256" ]; then
        echo "error: no SHA-256 pinned for VVL $VVL_VERSION (set VVL_SHA256)" >&2
        exit 1
    fi

    local cache="$VVL_CACHE/$VVL_VERSION"
    local archive="$cache/android-binaries-$VVL_VERSION.tar.gz"
    local member="android-binaries-$VVL_VERSION/arm64-v8a/libVkLayer_khronos_validation.so"
    mkdir -p "$cache"

    if [ -n "$VVL_ARCHIVE" ]; then
        if ! vvl_check "$VVL_ARCHIVE"; then
            echo "error: VVL_ARCHIVE hash mismatch: $VVL_ARCHIVE" >&2
            exit 1
        fi
        archive="$VVL_ARCHIVE"
    else
        if [ -f "$archive" ] && ! vvl_check "$archive"; then
            echo "warn: cached VVL hash mismatch, re-downloading" >&2
            rm -f "$archive"
        fi
        if [ ! -f "$archive" ]; then
            local url
            for url in $(vvl_sources); do
                echo "==> download VVL $VVL_VERSION: $url"
                # --speed-* aborts stalled links instead of hanging the build.
                if curl -fSL --retry 2 --retry-all-errors \
                        --connect-timeout 15 --max-time 300 \
                        --speed-time 20 --speed-limit 2048 \
                        -o "$archive.part" "$url/android-binaries-$VVL_VERSION.tar.gz" \
                   && vvl_check "$archive.part"; then
                    mv "$archive.part" "$archive"
                    break
                fi
                echo "warn: download failed or hash mismatch from $url" >&2
                rm -f "$archive.part"
            done
        fi
        if [ ! -f "$archive" ]; then
            echo "error: could not get a verified VVL $VVL_VERSION archive" >&2
            echo "       seed $archive, or set VVL_ARCHIVE / VVL_BASE_URL" >&2
            exit 1
        fi
    fi

    echo "==> VVL $VVL_VERSION archive: $archive"
    tar -xzf "$archive" -C "$cache" "$member"
    VVL_SO="$OUT/lib/arm64-v8a/libVkLayer_khronos_validation.so"
    cp "$cache/$member" "$VVL_SO"
    echo "==> VVL $VVL_VERSION: $(sha256sum "$VVL_SO" | cut -d' ' -f1)"
}

rm -rf "$OUT"
mkdir -p "$OUT/lib/arm64-v8a" "$BUILD/classes" "$BUILD/dex" "$APKROOT/lib/arm64-v8a"
bundle_validation_layer

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
CLASS_FILES=()
while IFS= read -r f; do
    CLASS_FILES+=("$f")
done < <(find "$BUILD/classes" -name '*.class' | sort)
"$BT/d8" --lib "$PLATFORM/android.jar" --min-api 26 \
    --output "$BUILD/dex" \
    "${CLASS_FILES[@]}"

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
if [ -n "$VVL_SO" ]; then
    cp "$VVL_SO" "$APKROOT/lib/arm64-v8a/"
fi
( cd "$APKROOT" && zip -q -r "$BUILD/unsigned.apk" . )
if [ -n "$VVL_SO" ] && [ ! -f "$APKROOT/lib/arm64-v8a/libVkLayer_khronos_validation.so" ]; then
    echo "error: validation layer missing from the APK root" >&2
    exit 1
fi

echo "==> align + sign"
"$BT/zipalign" -p -f 4 "$BUILD/unsigned.apk" "$BUILD/aligned.apk"

# Fixed, PUBLIC artifact key, so consecutive CI runs can update-install over
# each other. It is not a release key and never will be: docs/PIPELINE.md §4.1.
REPO=$(cd "$ROOT/../.." && pwd)
KS=${SIGNING_KEYSTORE:-$REPO/ci/signing/artifact.p12}
KS_PASS=${SIGNING_KEYSTORE_PASS:-nva-artifact}
KS_ALIAS=${SIGNING_KEYSTORE_ALIAS:-nva-artifact}

if [ -f "$KS" ]; then
    SIGN_ARGS=(--ks "$KS" --ks-type PKCS12
               --ks-pass "pass:$KS_PASS" --key-pass "pass:$KS_PASS"
               --ks-key-alias "$KS_ALIAS")
    echo "    signing with $KS (artifact key)"
else
    echo "    warning: $KS missing; falling back to a throwaway debug key." >&2
    echo "    warning: update-install across runs will fail." >&2
    KS="$BUILD/debug.keystore"
    if [ ! -f "$KS" ]; then
        keytool -genkeypair -keystore "$KS" -storepass android -keypass android \
            -alias androiddebugkey -dname "CN=Android Debug,O=Android,C=US" \
            -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1
    fi
    SIGN_ARGS=(--ks "$KS" --ks-pass pass:android --key-pass pass:android)
fi

# v2/v3 only. v1 (JAR signing) is a no-op at minSdk 26: even with
# --v1-signing-enabled true the tool emits no v1 signature and verify reports
# "v1 scheme: false" (measured in run 41089559337). Pinning a scheme that
# cannot exist would make the gate lie.
"$BT/apksigner" sign "${SIGN_ARGS[@]}" \
    --v2-signing-enabled true --v3-signing-enabled true --v4-signing-enabled false \
    --min-sdk-version 26 --out "$OUT/abi-slice.apk" "$BUILD/aligned.apk"
"$BT/apksigner" verify --verbose --min-sdk-version 26 "$OUT/abi-slice.apk" \
    > "$OUT/apksigner-verify.txt" 2>&1
"$BT/apksigner" verify --print-certs "$OUT/abi-slice.apk" \
    >> "$OUT/apksigner-verify.txt" 2>&1
cat "$OUT/apksigner-verify.txt"
for scheme in v2 v3; do
    grep -q "Verified using $scheme scheme.*: true" "$OUT/apksigner-verify.txt" \
        || { echo "error: $scheme signature not verified" >&2; exit 1; }
done

echo "==> inspect"
"$BT/aapt2" dump badging "$OUT/abi-slice.apk" > "$OUT/badging.txt" 2>&1 || true
# Relative paths on purpose: `unzip -l` echoes the argument it was given, and
# an absolute one would leak the build host into the artifact.
( cd "$OUT" && unzip -l abi-slice.apk > unzip-l.txt 2>&1 ) || true
grep -E 'package:|launchable-activity:|native-code:' "$OUT/badging.txt" || true
grep -E 'classes.dex|lib/arm64-v8a' "$OUT/unzip-l.txt" || true
# Relative path on purpose: an absolute path would leak the build host into
# the artifact (ci/guard/check_artifact.sh rejects that).
( cd "$OUT" && sha256sum abi-slice.apk > sha256sum.txt )
ls -lh "$OUT/abi-slice.apk"

if [ -n "$VVL_SO" ]; then
    cat <<EOF

Validation layer bundled. Enable it on the device:
  adb shell settings put global enable_gpu_debug_layers 1
  adb shell settings put global gpu_debug_app com.bingnnvjn.abislice
  adb shell settings put global gpu_debug_layer_app com.bingnnvjn.abislice
  adb shell settings put global gpu_debug_layers VK_LAYER_KHRONOS_validation
EOF
fi

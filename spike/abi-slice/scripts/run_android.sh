#!/usr/bin/env bash
# run_android.sh — push and run the #30 ABI-slice guest on a real Android device.
#
# CI only cross-builds the artifact; this script is the local device leg.
# It does not need root and touches only /data/local/tmp/abi-slice on the
# device. Tombstones live in /data/tombstones (root-only) and are skipped.
#
# Usage (from spike/abi-slice):
#   scripts/run_android.sh
#   ABI_SLICE_ANDROID_DIR=dist/android scripts/run_android.sh
#
# Environment:
#   ANDROID_SERIAL          device serial (default: let adb choose the only device)
#   ADB                     adb binary (default: adb)
#   ABI_SLICE_ANDROID_DIR   local artifact dir (default: <spike>/dist/android, or
#                           the directory containing this script if it was
#                           unpacked next to abi_guest)
#   ABI_SLICE_LOG_DIR       where to write host-side logs (default: artifact dir)
#
# Checks:
#   all, gate, stackgate    required: exit 0
#   unimplemented           required: exit 134 (SIGABRT)
set -euo pipefail

ADB=${ADB:-adb}
SERIAL=${ANDROID_SERIAL:-}
DIR=/data/local/tmp/abi-slice

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [ -n "${ABI_SLICE_ANDROID_DIR:-}" ]; then
    LOCAL_DIR=$ABI_SLICE_ANDROID_DIR
elif [ -f "$SCRIPT_DIR/abi_guest" ]; then
    # Packaged copy: run_android.sh sits next to abi_guest and libnvn_host.so.
    LOCAL_DIR=$SCRIPT_DIR
else
    LOCAL_DIR=$SCRIPT_DIR/../dist/android
fi
LOG_DIR=${ABI_SLICE_LOG_DIR:-$LOCAL_DIR}
FAILED=0

say()  { printf '%s\n' "$*"; }
pass() { say "PASS  $*"; }
fail() { say "FAIL  $*"; FAILED=1; }

# ---- adb / device checks ---------------------------------------------------
if ! command -v "$ADB" >/dev/null 2>&1; then
    cat >&2 <<EOF
error: adb ('$ADB') not found.
       Install Android platform-tools and put adb on PATH, or set ADB=/path/to/adb.
EOF
    exit 127
fi

ADB_CMD=("$ADB")
if [ -n "$SERIAL" ]; then
    ADB_CMD+=(-s "$SERIAL")
fi

if ! get_state=$("${ADB_CMD[@]}" get-state 2>&1); then
    {
        echo "error: no usable Android device (adb get-state failed)."
        if [ -n "$SERIAL" ]; then
            echo "       ANDROID_SERIAL='$SERIAL' was requested."
        fi
        echo "       adb devices -l:"
        "${ADB_CMD[@]}" devices -l || true
    } >&2
    exit 3
fi
say "adb device state: $get_state"

for f in abi_guest libnvn_host.so; do
    if [ ! -f "$LOCAL_DIR/$f" ]; then
        echo "error: missing $LOCAL_DIR/$f; build the android artifact first" >&2
        exit 1
    fi
done
mkdir -p "$LOG_DIR"

# getprop with CR stripped (adb shell emits \r\n).
getprop() {
    "${ADB_CMD[@]}" shell getprop "$1" 2>/dev/null | tr -d '\r' || true
}

MODEL=$(getprop ro.product.model)
RELEASE=$(getprop ro.build.version.release)
HARDWARE=$(getprop ro.hardware)
PLATFORM=$(getprop ro.board.platform)
ABI=$(getprop ro.product.cpu.abi)
SERIAL_ACTUAL=$("${ADB_CMD[@]}" get-serialno 2>/dev/null | tr -d '\r' || true)

{
    echo "# abi-slice device info"
    echo "serial:        $SERIAL_ACTUAL"
    echo "ro.product.model:            $MODEL"
    echo "ro.build.version.release:    $RELEASE"
    echo "ro.hardware:                 $HARDWARE"
    echo "ro.board.platform:           $PLATFORM"
    echo "ro.product.cpu.abi:          $ABI"
    echo "host_arch:     $(uname -m)"
    echo "date_utc:      $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
} | tee "$LOG_DIR/android-device.txt"

case "$ABI" in
    arm64-v8a) ;;
    *) say "warning: ro.product.cpu.abi=$ABI, but abi_guest is aarch64; it will not run" ;;
esac

# ---- push ------------------------------------------------------------------
say ""
say "==> pushing artifacts to $DIR"
"${ADB_CMD[@]}" shell rm -rf "$DIR"
"${ADB_CMD[@]}" shell mkdir -p "$DIR"
"${ADB_CMD[@]}" push "$LOCAL_DIR/abi_guest" "$DIR/abi_guest" >/dev/null
"${ADB_CMD[@]}" push "$LOCAL_DIR/libnvn_host.so" "$DIR/libnvn_host.so" >/dev/null
if [ -f "$LOCAL_DIR/run_android.sh" ]; then
    "${ADB_CMD[@]}" push "$LOCAL_DIR/run_android.sh" "$DIR/run_android.sh" >/dev/null
fi
"${ADB_CMD[@]}" shell chmod 755 "$DIR/abi_guest"

say "==> clearing logcat"
if ! "${ADB_CMD[@]}" logcat -c; then
    say "warning: adb logcat -c failed; logcat may contain stale lines"
fi

# ---- run -------------------------------------------------------------------
# Runs one guest subcommand remotely, tees stdout to a host-side log, and sets
# LAST_RC. $ORIGIN rpath is unreliable on some Android linkers, so the library
# dir is also put on LD_LIBRARY_PATH.
LAST_RC=0
run_guest() {
    local name=$1
    local log="$LOG_DIR/android-$name.log"

    say ""
    say "==> abi_guest $name"
    set +e
    "${ADB_CMD[@]}" shell "cd $DIR && LD_LIBRARY_PATH=$DIR ./abi_guest $name" >"$log" 2>&1
    LAST_RC=$?
    set -e
    # Show the captured output without \r noise.
    tr -d '\r' <"$log"
    say "    exit $LAST_RC (log: $log)"
}

run_guest all
if [ "$LAST_RC" -eq 0 ]; then pass "all (exit 0)"; else fail "all (exit $LAST_RC, expected 0)"; fi

run_guest gate
if [ "$LAST_RC" -eq 0 ]; then pass "gate (exit 0)"; else fail "gate (exit $LAST_RC, expected 0)"; fi

run_guest stackgate
if [ "$LAST_RC" -eq 0 ]; then pass "stackgate (exit 0)"; else fail "stackgate (exit $LAST_RC, expected 0)"; fi

run_guest unimplemented
if [ "$LAST_RC" -eq 134 ]; then
    pass "unimplemented aborted with SIGABRT (exit 134)"
else
    fail "unimplemented exit $LAST_RC, expected 134 (SIGABRT); 139=SIGSEGV is a failure"
fi

# ---- logcat ----------------------------------------------------------------
LOGCAT_LOG=$LOG_DIR/android-logcat.txt
say ""
say "==> logcat -d (tags: abi-slice DEBUG libc) -> $LOGCAT_LOG"
if ! "${ADB_CMD[@]}" logcat -d -v threadtime -s abi-slice:V DEBUG:V libc:V >"$LOGCAT_LOG" 2>&1; then
    say "warning: adb logcat -d failed"
fi
# /data/tombstones needs root; deliberately not pulled.
say "note: /data/tombstones is root-only and was not pulled (no root assumed)"

# ---- summary ---------------------------------------------------------------
say ""
say "== summary =="
say "device:   $MODEL ($RELEASE), $HARDWARE / $PLATFORM, abi=$ABI"
say "remote:   $DIR"
say "logs:"
say "  $LOG_DIR/android-device.txt"
say "  $LOG_DIR/android-all.log"
say "  $LOG_DIR/android-gate.log"
say "  $LOG_DIR/android-stackgate.log"
say "  $LOG_DIR/android-unimplemented.log"
say "  $LOGCAT_LOG"

if [ "$FAILED" -eq 0 ]; then
    say "result: PASS"
    exit 0
fi
say "result: FAIL"
exit 1

#!/usr/bin/env bash
# run_pc.sh — build (optionally) and run the #30 ABI-slice guest on a Linux host.
#
# This is the local/desktop harness. CI inlines the same commands in
# .github/workflows/abi-slice.yml so a red step maps to exactly one check.
#
# Usage:
#   scripts/run_pc.sh [--no-build] [--dist DIR]
#
# Environment:
#   BUILD=0                 skip cargo/guest build (default: build)
#   ABI_SLICE_PC_DIST       artifact dir (default: dist/pc)
#
# Checks:
#   all           required: exit 0
#   unimplemented required: exit 134 (SIGABRT); 139 (SIGSEGV) is a FAILURE
#   gate          required on aarch64; on other arches the TLS gate does not
#                 exist, so a non-zero exit is reported as SKIP, not FAIL
#   stackgate     same policy as gate (gate stack-arg probe is AArch64-only)
#
# Runs on both x86_64 and aarch64; the lavapipe ICD is chosen by `uname -m`.
# On aarch64 it defaults RUSTFLAGS to +reserve-x18 and compiles the guest with
# -ffixed-x18, matching the arm64 CI leg (an explicit RUSTFLAGS wins).
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

DIST=${ABI_SLICE_PC_DIST:-dist/pc}
BUILD=${BUILD:-1}
ARCH=$(uname -m)
FAILURES=0

usage() {
    cat <<'EOF'
usage: scripts/run_pc.sh [--no-build] [--dist DIR]

  --no-build   use an existing DIR/abi_guest instead of building
  --dist DIR   output directory (default: dist/pc)
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-build)
            BUILD=0
            shift
            ;;
        --dist)
            if [ "$#" -lt 2 ]; then
                echo "error: --dist needs a directory" >&2
                exit 2
            fi
            DIST=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

say()  { printf '%s\n' "$*"; }
pass() { say "PASS  $*"; }
fail() { say "FAIL  $*"; FAILURES=$((FAILURES + 1)); }
skip() { say "SKIP  $*"; }

is_aarch64() {
    case "$ARCH" in
        aarch64|arm64) return 0 ;;
        *) return 1 ;;
    esac
}

# Match the arm64 CI leg by default so a local aarch64 run exercises the same
# x18 contract. An explicit RUSTFLAGS from the caller always wins.
if is_aarch64 && [ -z "${RUSTFLAGS:-}" ]; then
    export RUSTFLAGS="-C target-feature=+reserve-x18"
fi

# ---- lavapipe ICD ----------------------------------------------------------
# Mesa ships one JSON per arch; pick by uname -m, fall back to a glob.
case "$ARCH" in
    x86_64|amd64) ICD_EXPECTED=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json ;;
    aarch64|arm64) ICD_EXPECTED=/usr/share/vulkan/icd.d/lvp_icd.aarch64.json ;;
    *) ICD_EXPECTED= ;;
esac

ICD=
if [ -n "$ICD_EXPECTED" ] && [ -f "$ICD_EXPECTED" ]; then
    ICD=$ICD_EXPECTED
else
    for f in /usr/share/vulkan/icd.d/lvp_icd.*.json /usr/share/vulkan/icd.d/lvp_icd.json; do
        if [ -f "$f" ]; then
            ICD=$f
            break
        fi
    done
fi

if [ -n "$ICD" ]; then
    # VK_ICD_FILENAMES is honoured by older loaders, VK_DRIVER_FILES by 1.3.207+.
    export VK_ICD_FILENAMES=$ICD
    export VK_DRIVER_FILES=$ICD
    say "lavapipe ICD: $ICD"
else
    say "warning: no lavapipe ICD JSON under /usr/share/vulkan/icd.d; using the loader default"
fi

# ---- build -----------------------------------------------------------------
if [ "$BUILD" = 1 ]; then
    command -v cargo >/dev/null 2>&1 || { echo "error: cargo not found" >&2; exit 127; }
    command -v cc >/dev/null 2>&1 || { echo "error: cc not found" >&2; exit 127; }
    command -v glslangValidator >/dev/null 2>&1 || {
        echo "error: glslangValidator not found; install glslang-tools" >&2
        exit 127
    }

    say "==> cargo build --release (host: $ARCH)"
    cargo build --release

    mkdir -p "$DIST"
    cp -f target/release/libnvn_host.so "$DIST/"

    say "==> compiling guest into $DIST/abi_guest"
    cc_flags=(-std=c11 -O2 -fno-stack-protector)
    if is_aarch64; then
        # Keep x18 free in the guest too, like the arm64 CI leg.
        cc_flags+=(-ffixed-x18 -DNVN_X18_RESERVED=1)
    fi
    cc "${cc_flags[@]}" -Iinclude guest/guest.c \
        -o "$DIST/abi_guest" \
        -Ltarget/release -lnvn_host \
        -Wl,-rpath,'$ORIGIN' \
        -ldl -lpthread -lm
else
    if [ ! -x "$DIST/abi_guest" ]; then
        echo "error: $DIST/abi_guest missing or not executable (drop --no-build)" >&2
        exit 1
    fi
    if [ ! -f "$DIST/libnvn_host.so" ]; then
        echo "error: $DIST/libnvn_host.so missing (rpath is \$ORIGIN)" >&2
        exit 1
    fi
fi

mkdir -p "$DIST"

if command -v vulkaninfo >/dev/null 2>&1; then
    if vulkaninfo --summary >"$DIST/vulkaninfo.txt" 2>&1; then
        say "vulkaninfo summary: $DIST/vulkaninfo.txt"
    else
        say "warning: vulkaninfo failed; see $DIST/vulkaninfo.txt"
    fi
fi

# ---- runners ---------------------------------------------------------------
# Each runner writes $DIST/pc-<name>.log even when the guest aborts.
run_expect_zero() {
    local name=$1
    local log="$DIST/pc-$name.log"
    local rc

    say ""
    say "==> abi_guest $name"
    set +e
    "$DIST/abi_guest" "$name" >"$log" 2>&1
    rc=$?
    set -e
    cat "$log"
    if [ "$rc" -eq 0 ]; then
        pass "$name (exit 0)"
    else
        fail "$name (exit $rc, expected 0); log: $log"
    fi
}

run_gate() {
    local name=$1
    local log="$DIST/pc-$name.log"
    local rc

    say ""
    say "==> abi_guest $name"
    set +e
    "$DIST/abi_guest" "$name" >"$log" 2>&1
    rc=$?
    set -e
    cat "$log"
    if [ "$rc" -eq 0 ]; then
        pass "$name (exit 0)"
    elif is_aarch64; then
        fail "$name (exit $rc, expected 0 on aarch64); log: $log"
    else
        skip "$name (exit $rc; the TLS gate is AArch64-only, not a failure on $ARCH); log: $log"
    fi
}

run_unimplemented() {
    local log="$DIST/pc-unimplemented.log"
    local rc

    say ""
    say "==> abi_guest unimplemented (expect SIGABRT = exit 134)"
    set +e
    "$DIST/abi_guest" unimplemented >"$log" 2>&1
    rc=$?
    set -e
    cat "$log"
    if [ "$rc" -eq 134 ]; then
        pass "unimplemented aborted with SIGABRT (exit 134)"
    else
        fail "unimplemented exit $rc, expected 134 (SIGABRT); 139=SIGSEGV is a failure; log: $log"
    fi
}

run_expect_zero all
run_gate gate
run_gate stackgate
run_unimplemented

# ---- summary ---------------------------------------------------------------
say ""
say "== summary =="
say "host:   $ARCH"
say "dist:   $DIST"
if [ "$FAILURES" -eq 0 ]; then
    say "result: PASS"
    exit 0
fi
say "result: FAIL ($FAILURES check(s))"
exit 1

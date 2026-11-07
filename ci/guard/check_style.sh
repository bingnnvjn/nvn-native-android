#!/usr/bin/env bash
# One mechanical style gate. CI and a local shell run this same file.
#
#   bash ci/guard/check_style.sh          # fmt, then clippy
#   bash ci/guard/check_style.sh fmt      # format only
#   bash ci/guard/check_style.sh clippy   # lint only
#
# A machine without glslang makes the runtime build script panic; this script
# does not set NVA_SKIP_SHADERS, because CI has glslang and must not skip.
# Locally: NVA_SKIP_SHADERS=1 bash ci/guard/check_style.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)" || {
  echo "check_style: not inside a git work tree" >&2
  exit 2
}

cargo_bin=${NVA_CARGO:-cargo}
mode=${1:-all}
case "$mode" in
  all | fmt | clippy) ;;
  *)
    echo "check_style: unknown mode '$mode' (want all|fmt|clippy)" >&2
    exit 2
    ;;
esac

if [ "$mode" = all ] || [ "$mode" = fmt ]; then
  echo "check_style: cargo fmt --all --check"
  "$cargo_bin" fmt --all --check
fi

if [ "$mode" = all ] || [ "$mode" = clippy ]; then
  echo "check_style: cargo clippy --workspace --all-targets --locked -- -D warnings"
  "$cargo_bin" clippy --workspace --all-targets --locked -- -D warnings
fi

echo "check_style: $mode ok"

#!/usr/bin/env bash
# Cheap, deterministic checks for the #30 spike. No Vulkan/Android needed.
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

echo "== JSON syntax =="
python3 -m json.tool nvn-table.json >/dev/null
echo "OK: nvn-table.json"

echo "== slot table =="
python3 scripts/check_table.py

echo "== C ABI layout =="
cc -std=c11 -Wall -Wextra -Werror -Iinclude \
    -c include/abi_layout_check_spike.c \
    -o "${TMPDIR:-/tmp}/abi_layout_check_spike.o"
echo "OK: include/abi_layout_check_spike.c"

echo "== shell syntax =="
for f in scripts/*.sh android/build_apk.sh; do
    bash -n "$f"
done
echo "OK: bash -n"

if command -v shellcheck >/dev/null 2>&1; then
    echo "== shellcheck =="
    shellcheck -S warning scripts/*.sh android/build_apk.sh
fi

echo "ALL CHECKS PASS"

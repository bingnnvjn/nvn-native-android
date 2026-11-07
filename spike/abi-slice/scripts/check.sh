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

echo "== no stale product header paths =="
# #38: spike files must name their own headers, never the product ones. The
# filename check in ci/guard/ only catches a reintroduced file, not a stale
# reference in a comment or script.
if grep -rn -I --exclude=check.sh --exclude-dir=target --exclude-dir=dist \
    -e 'include/nva_abi.h' -e 'include/abi_layout_check.c' .; then
    echo "FAIL: stale product header path; use include/nva_abi_spike.h / include/abi_layout_check_spike.c" >&2
    exit 1
fi
echo "OK: no stale product header paths"

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

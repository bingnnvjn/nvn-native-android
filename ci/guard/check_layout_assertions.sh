#!/usr/bin/env bash
# The layout assertions in include/abi_layout_check.c are the only thing that
# pins C ABI v1 at compile time. Nothing counted them, so deleting one left a
# valid translation unit and CI stayed green (#47, measured). The four compiles
# in docs/GATES.md §3 can prove "the remaining assertions hold"; they cannot
# prove "the assertions that should exist are still there".
#
# This guard pins the total and the per-group split. The expected numbers live
# in layout-assertions.txt next to this script: an ABI append then changes data
# rather than shell code, and the diff reads "76 -> 77, nva_stats 2 -> 11"
# (CODING_STANDARDS.md, "ABI change").
#
# Usage: check_layout_assertions.sh [REPO_ROOT] [EXPECT_FILE]
# Exit codes: 0 clean, 1 inventory drift, 2 bad input (root/file/expect).
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

if (($# > 2)); then
  echo "usage: $0 [REPO_ROOT] [EXPECT_FILE]" >&2
  exit 2
fi

root=${1:-}
if [[ -z $root ]]; then
  if ! root=$(git rev-parse --show-toplevel 2>/dev/null); then
    echo "check_layout_assertions: not inside a git repository" >&2
    exit 2
  fi
fi
if [[ ! -d $root ]]; then
  echo "check_layout_assertions: not a directory: $root" >&2
  exit 2
fi

expect=${2:-$here/layout-assertions.txt}
if [[ ! -f $expect ]]; then
  echo "check_layout_assertions: no expectation file: $expect" >&2
  exit 2
fi

cd "$root"
src=include/abi_layout_check.c
if [[ ! -f $src ]]; then
  echo "check_layout_assertions: no $src under $root" >&2
  exit 2
fi

# Counted per line, which is how the file is written today: one assertion per
# line. The `#define NVA_SA` block at the top starts with `#define`, so this
# pattern does not pick it up; no exclusion rule is needed. A future assertion
# wrapped over two lines would still count once, so the total would drift and
# this guard would say so instead of passing.
#
# Group labels come from the `/* ---- <label> ---- */` headers. A label may
# carry a trailing note after ':' ("nva_fence: 16 B"), cut off below. The five
# version-macro assertions at the end of the file have no header of their own
# and therefore count under the last one (SVC, 10 rather than 5). That is the
# file as it stands; this ticket adds no assertion and reorders none.
actual=$(awk '
  /^\/\* ---- / {
    label = $0
    sub(/^\/\* ---- /, "", label); sub(/ ---- \*\/$/, "", label)
    cur = label; sub(/:.*$/, "", cur); next
  }
  /^[[:space:]]*NVA_SA\(/ {
    total++
    if (cur == "") { orphan++; next }
    if (!(cur in per)) groups++
    per[cur]++
  }
  END {
    for (k in per) printf "group %s %d\n", k, per[k]
    printf "total %d\n", total
    printf "groups %d\n", groups
    if (orphan) printf "orphan %d\n", orphan
  }' "$src" | sort -k2)

actual_total=$(awk '$1 == "total" { print $2 }' <<<"$actual")
actual_groups=$(awk '$1 == "groups" { print $2 }' <<<"$actual")

drift=0
declare -A pinned=()
while read -r key n; do
  case $key in
    '' | '#'*) continue ;;
  esac
  if [[ $key == total ]]; then
    if [[ $actual_total != "$n" ]]; then
      printf 'LAYOUT-INVENTORY: %s has %s assertions, expected %s\n' "$src" "$actual_total" "$n" >&2
      drift=1
    fi
    continue
  fi
  pinned[$key]=$n
  got=$(awk -v k="$key" '$1 == "group" && $2 == k { print $3 }' <<<"$actual")
  if [[ -z $got ]]; then
    printf 'LAYOUT-INVENTORY: group %s is gone (expected %s assertions)\n' "$key" "$n" >&2
    drift=1
  elif [[ $got != "$n" ]]; then
    printf 'LAYOUT-INVENTORY: group %s has %s assertions, expected %s\n' "$key" "$got" "$n" >&2
    drift=1
  fi
done <"$expect"

# Equality is exact per group on purpose: a group emptied into another group
# leaves the total intact, and only the split catches that (#49 §3.1).
while read -r _ name count; do
  if [[ -z ${pinned[$name]:-} ]]; then
    printf 'LAYOUT-INVENTORY: group %s has %s assertions but is not in the inventory\n' \
      "$name" "$count" >&2
    drift=1
  fi
done < <(awk '$1 == "group" { print $1, $2, $3 }' <<<"$actual")

# An assertion above the first header belongs to no group; per-group counts
# would then still sum to the pinned total while the split lies.
orphan=$(awk '$1 == "orphan" { print $2 }' <<<"$actual")
if [[ -n $orphan ]]; then
  printf 'LAYOUT-INVENTORY: %s assertions above the first group header\n' "$orphan" >&2
  drift=1
fi

if ((drift)); then
  echo "check_layout_assertions: FAILED — this is an inventory change, not a compile error." >&2
  echo "If the ABI grew on purpose, update ci/guard/layout-assertions.txt in the same" >&2
  echo "commit and say so in the message (#49, CODING_STANDARDS.md \"ABI change\")." >&2
  exit 1
fi

printf 'check_layout_assertions: %s assertions in %s groups, as pinned\n' \
  "$actual_total" "$actual_groups"

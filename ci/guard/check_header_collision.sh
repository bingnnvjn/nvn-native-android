#!/usr/bin/env bash
# The spike tree may not contain a file named like any product include/ file
# (#38). The two headers are not the same header: a build that picks the wrong
# one through `-I` order fails assertions or passes the wrong layout checks,
# and nothing says which one it read.
#
# Usage: check_header_collision.sh [REPO_ROOT]
# Exit codes: 0 clean, 1 collision, 2 bad root / no product headers.
set -euo pipefail

if (($# > 1)); then
  echo "usage: $0 [REPO_ROOT]" >&2
  exit 2
fi

root=${1:-}
if [[ -z $root ]]; then
  if ! root=$(git rev-parse --show-toplevel 2>/dev/null); then
    echo "check_header_collision: not inside a git repository" >&2
    exit 2
  fi
fi
if [[ ! -d $root ]]; then
  echo "check_header_collision: not a directory: $root" >&2
  exit 2
fi
cd "$root"
if ! git rev-parse --git-dir >/dev/null 2>&1; then
  echo "check_header_collision: not a git repository: $root" >&2
  exit 2
fi

# Tracked plus untracked, excluding gitignored build outputs (`target/`,
# `dist/`) — those are not part of the tree and must not turn CI red.
tree_files() {
  git ls-files -z -- "$1"
  git ls-files -z -o --exclude-standard -- "$1"
}

declare -A product=()
product_count=0
while IFS= read -r -d '' p; do
  [[ -n $p ]] || continue
  product[${p##*/}]=1
  product_count=$((product_count + 1))
done < <(tree_files include)

if ((product_count == 0)); then
  echo "check_header_collision: no files under include/ — refusing to pass" >&2
  exit 2
fi

fail=0
spike_count=0
while IFS= read -r -d '' p; do
  [[ -n $p ]] || continue
  spike_count=$((spike_count + 1))
  name=${p##*/}
  if [[ -n ${product[$name]:-} ]]; then
    printf 'HEADER-COLLISION: spike tree has a product header name: %s\n' "$p" >&2
    fail=1
  fi
done < <(tree_files spike)

if ((fail)); then
  echo "check_header_collision: FAILED — rename the spike file (#38, GATES §2.2)" >&2
  exit 1
fi

printf 'check_header_collision: %d spike paths vs %d product include names, clean\n' \
  "$spike_count" "$product_count"

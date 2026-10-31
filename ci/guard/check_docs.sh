#!/usr/bin/env bash
# Documentation guard: the red-line sentences must exist, verbatim, in README
# (both languages) and in the release-notes template. If someone rewrites the
# disclaimer away, CI should say so (#16 §4, #21 §6).
#
# It also rejects personal-data shapes in public Markdown (#33/#35). The policy
# document docs/PRIVACY.md is exempt because it quotes those shapes by design.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

fail=0
need() {
  local file=$1 needle=$2
  [[ -f $file ]] || { echo "DOCS-RED-LINE: missing $file" >&2; fail=1; return; }
  grep -qF -- "$needle" "$file" || {
    echo "DOCS-RED-LINE: $file is missing: $needle" >&2
    fail=1
  }
}

need README.md "This repository contains and distributes no game content"
need README.md "sponsored by Nintendo"
need README.md "we do not explain how to obtain either"
need README.md "本仓库不含任何游戏数据"
need README.md "未获其授权、背书或赞助"
need docs/RELEASE-NOTES-TEMPLATE.md "Needs your own game files and keys"
need docs/RELEASE-NOTES-TEMPLATE.md "需要自备游戏文件与密钥"
need docs/RELEASE-NOTES-TEMPLATE.md "This release contains no import pack."
# The ledger's *columns* are parsed by check_derived_ledger.py; docs guard the
# rule sentence instead, so adding a column does not trip a pixel test.
need docs/DERIVED.md "disclose each table, or do not ship it"

# Personal-data shapes are checked by scan_personal.py, the single source for
# those rules. The policy document docs/PRIVACY.md is exempt there because it
# quotes the shapes by design.
mapfile -d '' docs < <(git ls-files -z -- '*.md')
if ((${#docs[@]})); then
  if ! printf '%s\0' "${docs[@]}" \
      | python3 ci/guard/scan_personal.py --stdin-null; then
    echo "DOCS-RED-LINE: public Markdown contains a personal-data shape" >&2
    fail=1
  fi
fi

if ((fail)); then
  echo "guard: docs FAILED (docs/PIPELINE.md §6)." >&2
  exit 1
fi
echo "guard: docs OK"

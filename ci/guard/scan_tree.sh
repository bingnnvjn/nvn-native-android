#!/usr/bin/env bash
# Red-line guard over the working tree AND the full git history.
#
# Rationale (#16): a file deleted in a later commit is still in the pack and
# still served by the repo. Scanning only `git ls-files` gives false comfort.
# Rationale (review 2026-10-31): a suffix list alone is bypassed by renaming,
# so content magic and key-shaped text are checked too (scan_magic.py).
#
# Judged here: game content, keys, saves, shader originals/binaries, carry-over
# pack layouts, private key material, and oversized unregistered blobs.
# Derived tables (identifiers only) are allowed but must appear in
# docs/DERIVED.md — that is check_derived_ledger.py's job.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

ALLOWLIST=ci/guard/allowlist.txt
[[ -f $ALLOWLIST ]] || { echo "guard: missing $ALLOWLIST" >&2; exit 2; }

# Paths that may exceed the size gate, one glob per line (# comments ignored).
allowed() {
  local p=$1 line
  while IFS= read -r line; do
    [[ -z $line || $line == \#* ]] && continue
    # shellcheck disable=SC2053
    [[ $p == $line ]] && return 0
  done <"$ALLOWLIST"
  return 1
}

fail=0
report() {
  printf 'RED-LINE: %s\n' "$*" >&2
  fail=1
}

# --- 1. suffixes: game content, archives, shader binaries -------------------
BANNED_EXT_RE='\.(xci|xcz|nsp|nsz|nca|ncz|nso|npdm|kip|kipx|nss|app|cci|cnmt|pfs0|hfs|hfs0|istorage|kek|tik|sav|bfstm|bars|bntx|bfttf|bflyt|sarc|bfarc|bfres|bnk|bfsar|xgs|spvbin|spv|spirv)$'

# --- 2. paths: keys, extracted game trees, carry-over pack layouts ----------
# File names that end the path:
BANNED_FILE_RE='(^|/)(prod\.keys|title\.keys|console\.keys|[a-z0-9.]+\.keys|main\.npdm|main|sdk|rtld|subsdk[0-9]*|nnSdk|derived\.tsv|shader-cache\.bin|romfs\.bin|romfs\.idx|load-plan\.json|pack_info\.txt)$'
# Directory segments that may appear anywhere:
BANNED_DIR_RE='(^|/)(exefs|romfs|nca|nsp|xci|secure|save|saves|savedata)/'

# --- 3. private key material: everything except the published artifact key ---
PRIVATE_KEY_RE='\.(p12|pfx|jks|keystore|pem|key)$'
IS_PUBLISHED_ARTIFACT_KEY='^ci/signing/artifact\.p12$'

scan_path() {
  local p=$1 where=$2
  [[ $p =~ $BANNED_EXT_RE ]] && report "$where: game/shader file: $p"
  # ci/fixtures/ is synthetic by construction; its members carry real magic on
  # purpose (to exercise the tools) and never game bytes.
  [[ $p =~ $BANNED_FILE_RE && ! $p =~ ^ci/fixtures/ ]] && report "$where: game data path: $p"
  [[ $p =~ $BANNED_DIR_RE ]] && report "$where: extracted game tree: $p"
  if [[ $p =~ $PRIVATE_KEY_RE && ! $p =~ $IS_PUBLISHED_ARTIFACT_KEY ]] && ! allowed "$p"; then
    report "$where: private key material: $p ($(printf '%s' "$p" | wc -c) chars)"
  fi
  return 0
}

# Bare NSO names (`main`, `sdk`, `rtld`, `subsdk*`) are only suspicious when
# they are big or start with a program magic; scan_magic.py judges that part.
BARE_NSO_RE='(^|/)(main|sdk|rtld|subsdk[0-9]*|nnSdk)$'

# --- 4. live tree: path rules + size ---------------------------------------
scan_stream() {
  local where=$1
  while IFS= read -r -d '' p; do
    scan_path "$p" "$where"
    [[ -f $p ]] || continue
    local sz
    sz=$(stat -c %s -- "$p")
    if ((sz > 4 * 1024 * 1024)) && ! allowed "$p"; then
      report "$where: >4 MiB and not in allowlist: $p ($sz bytes)"
    fi
    if ((sz > 256 * 1024)) && ! allowed "$p" && file -b -- "$p" | grep -q '^data$'; then
      report "$where: unregistered binary blob: $p"
    fi
    if [[ $p =~ $BARE_NSO_RE ]] && ((sz > 64 * 1024)); then
      report "$where: bare NSO-sized '$p' ($sz bytes)"
    fi
  done
}

scan_stream "tree" < <(git ls-files -z)
# Untracked files are scanned too: `git add -A` after a build is the common
# accident, and a clean `git status` is not the same as a clean tree.
scan_stream "untracked" < <(git ls-files -z -o --exclude-standard)

# --- 5. history: every path ever committed ---------------------------------
while IFS= read -r line; do
  p=${line#* }
  [[ $p == "$line" ]] && continue
  scan_path "$p" "history"
done < <(git rev-list --objects --all | sed 's/^[0-9a-f]* //')

# --- 6. content: magic bytes and key-shaped text ---------------------------
{ git ls-files -z; git ls-files -z -o --exclude-standard; } \
  | python3 ci/guard/scan_magic.py --stdin-null || fail=1

if ((fail)); then
  echo
  echo "guard: FAILED. See docs/PIPELINE.md §6 (red lines) and docs/DERIVED.md." >&2
  exit 1
fi
echo "guard: tree, untracked files, history and content clean"

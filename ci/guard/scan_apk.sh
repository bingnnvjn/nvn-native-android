#!/usr/bin/env bash
# APK content guard. The Android counterpart of LAS's check_public_apk.sh
# (see #16 §1): "an APK to share carries no code or data made from the game".
#
# Usage: scan_apk.sh <apk> [--allow-debuggable]
set -euo pipefail

APK=${1:?usage: scan_apk.sh <apk>}
[[ -f $APK ]] || { echo "guard: no such APK: $APK" >&2; exit 2; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

fail=0
report() { printf 'APK-RED-LINE: %s\n' "$*" >&2; fail=1; }

echo "== entries =="
python3 - "$APK" "$WORK" <<'PY'
import sys, zipfile
apk, work = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(apk) as z:
    names = z.namelist()
    with open(f"{work}/entries.txt", "w", encoding="utf-8") as fh:
        fh.write("\n".join(names) + "\n")
        fh.flush()
    z.extractall(f"{work}/x")
PY
cat "$WORK/entries.txt"

# --- 1. nothing that is game content or a shader binary ---------------------
BANNED_RE='\.(xci|xcz|nsp|nsz|nca|ncz|nso|npdm|kip|kipx|nss|app|cci|cnmt|pfs0|hfs|hfs0|istorage|kek|tik|sav|bfstm|bars|bntx|bfttf|bflyt|sarc|bfarc|bfres|bnk|bfsar|xgs|spvbin|spv|spirv)$'
while IFS= read -r e; do
  [[ -z $e ]] && continue
  [[ $e =~ $BANNED_RE ]] && report "banned entry: $e"
  case $e in
    # v0 APKs have no assets at all. When the product shell needs assets, widen
    # this by TYPE (licenses/fonts/locale/img), never by suffix alone — a renamed
    # shader blob must still fail the magic check below.
    assets/*) report "assets/ is not allowed in v0 APKs (#16: shader/pipeline data never ships): $e" ;;
    */main.npdm|*/exefs/*|*/romfs/*|*/derived.tsv|*/shader-cache.bin)
      report "game data path: $e" ;;
  esac
done <"$WORK/entries.txt"

# --- 2. required entries ----------------------------------------------------
grep -qx 'AndroidManifest.xml' "$WORK/entries.txt" || report "missing AndroidManifest.xml"
grep -qx 'classes.dex' "$WORK/entries.txt" || report "missing classes.dex"
grep -q '^lib/arm64-v8a/.*\.so$' "$WORK/entries.txt" || report "missing lib/arm64-v8a/*.so"

# --- 3. size ----------------------------------------------------------------
sz=$(stat -c %s "$APK")
if ((sz > 32 * 1024 * 1024)); then
  report "APK is $sz bytes, over the 32 MiB budget (#21 §4.2)"
fi

# --- 4. unpacked content: no runner paths, no generated game functions ------


LEAK_RE='/(home|Users)/[A-Za-z0-9._-]+/(work|DeepSeekharness|dm-work)'
if grep -r -a -l -E "$LEAK_RE" "$WORK/x" >/dev/null 2>&1; then
  report "build-host path leaked into the APK (strip or -fdebug-prefix-map):"
  grep -r -a -l -E "$LEAK_RE" "$WORK/x" >&2
fi

# Magic bytes: renaming a file does not change NSO0/PFS0/SPIR-V at offset 0.
python3 ci/guard/scan_magic.py --stdin-null < <(find "$WORK/x" -type f -print0) || fail=1

# LAS's criterion, translated: no recompiled game function bodies.
if grep -r -a -o -E '\bf_[0-9a-f]{8}\b' "$WORK/x" 2>/dev/null | head -5 | grep -q .; then
  report "found f_<hex> symbols — recompiled game functions must never ship"
fi

if ((fail)); then
  echo "guard: APK FAILED the public boundary (docs/PIPELINE.md §6)." >&2
  exit 1
fi
echo "guard: APK clean ($sz bytes)"

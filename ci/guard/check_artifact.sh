#!/usr/bin/env bash
# Pre-upload guard: an artifact directory must contain exactly what the
# contract says (#21 §3) — no build intermediates, no keys, no game content,
# no absolute paths.
#
# Usage: check_artifact.sh <dir> <artifact-name> [apk|evidence]
set -euo pipefail

DIR=${1:?usage: check_artifact.sh <dir> <name> [apk|evidence]}
NAME=${2:?usage: check_artifact.sh <dir> <name> [apk|evidence]}
KIND=${3:-apk}
[[ -d $DIR ]] || { echo "guard: no such directory: $DIR" >&2; exit 2; }

fail=0
report() { printf 'ARTIFACT-RED-LINE: %s\n' "$*" >&2; fail=1; }

# Intermediates from build_apk.sh (apkroot/, classes, dex, aligned/unsigned)
# must not ride along.
for d in build apkroot classes dex; do
  [[ -e $DIR/$d ]] && report "build intermediate present: $d/ (prune it before upload)"
done

# Key material must never leave the runner. The published artifact key is
# intentionally in the repo, but it is not an artifact member either.
while IFS= read -r -d '' p; do
  case $p in
    *.keystore|*.jks|*.p12|*.pem|*.key) report "key material in artifact: $p" ;;
  esac
done < <(find "$DIR" -type f -print0)

# Game content by suffix, in every member and inside every tar member.
BANNED_RE='\.(xci|xcz|nsp|nsz|nca|ncz|nso|npdm|kip|kipx|sav|bfstm|bars|bntx|bfttf|bflyt|sarc|bfarc|bfres|bnk|bfsar|xgs|spvbin|spv|spirv|romfs|istorage)$'
while IFS= read -r -d '' p; do
  [[ $p =~ $BANNED_RE ]] && report "game content in artifact: $p"
done < <(find "$DIR" -type f -print0)

while IFS= read -r -d '' t; do
  while IFS= read -r member; do
    [[ $member =~ $BANNED_RE ]] && report "game content inside $(basename "$t"): $member"
  done < <(tar tzf "$t" 2>/dev/null || true)
done < <(find "$DIR" -type f -name '*.tar.gz' -print0)

case $KIND in
  apk)
    # Everything at top level must be a known document or the APK itself.
    ALLOWED_RE='^(abi-slice\.apk|.+\.txt|.+\.json|.+\.log)$'
    while IFS= read -r -d '' p; do
      rel=${p#"$DIR"/}
      [[ $rel == */* ]] && continue
      [[ $rel =~ $ALLOWED_RE ]] || report "unexpected artifact member: $rel"
    done < <(find "$DIR" -maxdepth 1 -type f -print0)

    APK=$DIR/abi-slice.apk
    if [[ -f $APK ]]; then
      [[ -f $DIR/sha256sum.txt ]] || report "missing sha256sum.txt next to the APK"
      if [[ -f $DIR/sha256sum.txt ]]; then
        ( cd "$DIR" && sha256sum -c --ignore-missing sha256sum.txt >/dev/null 2>&1 ) \
          || report "sha256sum.txt does not match the APK"
      fi
      # The checksum file must not embed an absolute path (it leaks the runner).
      grep -qE '(^| )/' "$DIR/sha256sum.txt" && report "sha256sum.txt contains an absolute path"
    else
      report "artifact '$NAME' does not contain abi-slice.apk"
    fi
    ;;
  evidence)
    # Evidence is logs + ELF metadata; it may hold bare binaries (abi_guest),
    # but no game content (checked above) and no host paths (checked below).
    ;;
  *)
    echo "guard: unknown artifact kind: $KIND" >&2
    exit 2
    ;;
esac

# No build-host paths in any text member.
while IFS= read -r f; do
  report "absolute build-host path in artifact text file: $f"
done < <(grep -r -l -E '/(home|Users)/[A-Za-z0-9._-]+/(work|DeepSeekharness|dm-work)' "$DIR" \
           --include='*.txt' --include='*.json' --include='*.log' 2>/dev/null || true)

if ((fail)); then
  echo "guard: artifact FAILED its contract (docs/PIPELINE.md §3)." >&2
  exit 1
fi
echo "guard: artifact contract OK ($NAME, $KIND)"

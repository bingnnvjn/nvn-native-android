#!/usr/bin/env bash
# Pre-upload guard: an artifact directory must contain exactly what the
# contract says (#21 §3) — no build intermediates, no keys, no game content,
# no media, no absolute paths.
#
# Usage: check_artifact.sh <dir> <artifact-name> [apk|evidence]
set -euo pipefail

DIR=${1:?usage: check_artifact.sh <dir> <name> [apk|evidence]}
NAME=${2:?usage: check_artifact.sh <dir> <name> [apk|evidence]}
KIND=${3:-apk}
[[ -d $DIR ]] || { echo "guard: no such directory: $DIR" >&2; exit 2; }
DIR=$(realpath -m "$DIR")

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail=0
report() { printf 'ARTIFACT-RED-LINE: %s\n' "$*" >&2; fail=1; }

# Intermediates from build_apk.sh (apkroot/, classes, dex, aligned/unsigned)
# must not ride along.
for d in build apkroot classes dex lib; do
  [[ -e $DIR/$d ]] && report "build intermediate present: $d/ (prune it, or upload explicit files)"
done

# Key material must never leave the runner. The published artifact key is
# intentionally in the repo, but it is not an artifact member either.
PRIVATE_KEY_RE='\.(p12|pfx|jks|keystore|pem|key)$'

# Media and archives are not evidence members (#33).
MEDIA_RE='\.(png|jpe?g|gif|webp|bmp|tiff?|heic|heif|mp4|m4v|mov|avi|mkv|webm|wav|mp3|flac|ogg)$'

# Game content by suffix, in every member and inside every archive member.
BANNED_RE='\.(xci|xcz|nsp|nsz|nca|ncz|nso|npdm|kip|kipx|sav|bfstm|bars|bntx|bfttf|bflyt|sarc|bfarc|bfres|bnk|bfsar|xgs|spvbin|spv|spirv|romfs|istorage)$'

check_name() {
  local where=$1 rel=$2
  [[ $rel =~ $BANNED_RE ]] && report "game content $where: $rel"
  [[ $rel =~ $PRIVATE_KEY_RE ]] && report "key material $where: $rel"
  [[ $rel =~ $MEDIA_RE ]] && report "media $where: $rel"
  return 0
}

while IFS= read -r -d '' p; do
  check_name "in artifact" "${p#"$DIR"/}"
done < <(find "$DIR" -type f -print0)

is_elf() {
  local magic
  magic=$(head -c 4 -- "$1" 2>/dev/null || true)
  [[ $magic == $'\x7fELF' ]]
}

# Evidence is code + logs, never media or game data. A bare executable has no
# suffix, so its ELF magic is the allowlist entry.
evidence_member_ok() {
  local rel=$1 file=$2
  case $rel in
    *.txt|*.json|*.log|*.sh|*.so|*.tar|*.tar.gz|*.tgz|*.zip) return 0 ;;
  esac
  [[ $rel == *.* ]] && return 1
  is_elf "$file"
}

# Scan archive members after unpacking, not just their names: a renamed blob
# inside a .tar.gz must still fail the magic/path/key checks.
scan_archive_members() {
  local archive=$1
  local n=$2
  local adir="$TMP/extract-$n"
  local list="$TMP/members-$n.txt"
  mkdir -p "$adir"

  case $archive in
    *.tar.gz|*.tgz) tar tzf "$archive" >"$list" 2>/dev/null || {
      report "cannot list archive: $archive"; return; } ;;
    *.tar) tar tf "$archive" >"$list" 2>/dev/null || {
      report "cannot list archive: $archive"; return; } ;;
    *.zip) unzip -Z1 "$archive" >"$list" 2>/dev/null || {
      report "cannot list archive: $archive"; return; } ;;
    *) return ;;
  esac

  while IFS= read -r member; do
    member=${member#./}
    [[ -z $member || $member == */ ]] && continue
    [[ $member == /* || $member == ../* || $member == *"/../"* ]] \
      && report "unsafe archive member in $(basename "$archive"): $member"
    check_name "inside $(basename "$archive")" "$member"
  done <"$list"

  case $archive in
    *.tar.gz|*.tgz) tar xzf "$archive" -C "$adir" 2>/dev/null || {
      report "cannot unpack archive: $archive"; return; } ;;
    *.tar) tar xf "$archive" -C "$adir" 2>/dev/null || {
      report "cannot unpack archive: $archive"; return; } ;;
    *.zip) unzip -q "$archive" -d "$adir" 2>/dev/null || {
      report "cannot unpack archive: $archive"; return; } ;;
  esac

  while IFS= read -r -d '' p; do
    local rel=${p#"$adir"/}
    evidence_member_ok "$rel" "$p" \
      || report "unexpected evidence member in $(basename "$archive"): $rel"
    check_name "inside $(basename "$archive")" "$rel"
  done < <(find "$adir" -type f -print0)

  if [[ -n $(find "$adir" -type f -print0 | xargs -0 -r grep -l -E \
        '(/home/|/Users/|/mnt/|[A-Za-z]:\\)' 2>/dev/null || true) ]]; then
    report "absolute build-host path inside $(basename "$archive")"
  fi

  find "$adir" -type f -print0 \
    | python3 "$ROOT/ci/guard/scan_magic.py" --stdin-null || fail=1
}

# Check every archive (tar/tgz/zip) and its members.
n=0
while IFS= read -r -d '' archive; do
  n=$((n + 1))
  scan_archive_members "$archive" "$n"
done < <(find "$DIR" -type f \( -name '*.tar.gz' -o -name '*.tgz' -o -name '*.tar' -o -name '*.zip' \) -print0)

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
    # Evidence is logs, scripts, ELF metadata, and the tarball. It may hold
    # bare binaries (abi_guest), but no media, keys, game content, or paths.
    while IFS= read -r -d '' p; do
      rel=${p#"$DIR"/}
      evidence_member_ok "$rel" "$p" || report "unexpected evidence member: $rel"
    done < <(find "$DIR" -type f -print0)

    # Direct files get content magic; archives are checked after unpacking
    # above, so exclude them here (their own magic is a container, not media).
    find "$DIR" -type f \
      ! -name '*.tar.gz' ! -name '*.tgz' ! -name '*.tar' ! -name '*.zip' -print0 \
      | python3 "$ROOT/ci/guard/scan_magic.py" --stdin-null || fail=1
    ;;
  *)
    echo "guard: unknown artifact kind: $KIND" >&2
    exit 2
    ;;
esac

# No build-host paths in any text member. /tmp/ is deliberately not a leak:
# the build uses temp dirs legitimately.
while IFS= read -r f; do
  report "absolute build-host path in artifact text file: $f"
done < <(grep -r -l -E '(/home/|/Users/|/mnt/|[A-Za-z]:\\)' "$DIR" \
           --include='*.txt' --include='*.json' --include='*.log' --include='*.sh' 2>/dev/null || true)

if ((fail)); then
  echo "guard: artifact FAILED its contract (docs/PIPELINE.md §3)." >&2
  exit 1
fi
echo "guard: artifact contract OK ($NAME, $KIND)"

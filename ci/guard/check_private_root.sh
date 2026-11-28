#!/usr/bin/env bash
# Private-root guard (docs/PRIVACY.md §2, #96).
#
#   check_private_root.sh <path>
#
# Exit 0: the path may hold private evidence.
# Exit 1: it is on a Windows/removable mount, under a cloud-sync name, on a
#         filesystem other than ext4|btrfs|xfs|zfs, or inside a git work tree.
# Exit 2: usage error, or the checks cannot run on this machine.
set -euo pipefail

if (($# != 1)); then
  echo "usage: check_private_root.sh <path>" >&2
  exit 2
fi

for tool in realpath findmnt; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "check_private_root: $tool is required" >&2
    exit 2
  fi
done

path=$(realpath -m -- "$1")
lower=${path,,}

reject() {
  echo "check_private_root: $1: $path" >&2
  exit 1
}

# Windows / removable mounts are not the private root.
for prefix in /mnt /media /run/media; do
  if [[ $path == "$prefix" || $path == "$prefix"/* ]]; then
    reject "Windows or removable mount"
  fi
done

# Cloud-sync directories are excluded by name, whatever their filesystem.
for cloud in onedrive dropbox "google drive" icloud nextcloud syncthing; do
  if [[ $lower == *"$cloud"* ]]; then
    reject "cloud-sync directory"
  fi
done

# The path may not exist yet: use its nearest existing ancestor.
probe=$path
while [[ ! -e $probe ]]; do
  parent=$(dirname -- "$probe")
  [[ $parent == "$probe" ]] && break
  probe=$parent
done
[[ -d $probe ]] || probe=$(dirname -- "$probe")

filesystem=$(findmnt -no FSTYPE --target "$probe" 2>/dev/null || true)
case "$filesystem" in
  ext4 | btrfs | xfs | zfs) ;;
  "") reject "cannot determine the filesystem type" ;;
  *) reject "filesystem $filesystem is not ext4|btrfs|xfs|zfs" ;;
esac

if command -v git >/dev/null 2>&1; then
  inside=$(git -C "$probe" rev-parse --is-inside-work-tree 2>/dev/null || true)
  if [[ $inside == true ]]; then
    reject "inside a git work tree"
  fi
fi

echo "check_private_root: $path on $filesystem is acceptable"
exit 0

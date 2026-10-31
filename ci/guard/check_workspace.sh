#!/usr/bin/env bash
# Workspace membership gate (#39, docs/GATES.md §2.1 and §3).
#
# Pins the product workspace to exactly five crates, proves the APK shell is not
# a member, and keeps Android SDK/NDK usage out of the workspace build scripts.
# Needs cargo + jq, so it runs in pc-x86_64 (the guard job has no Rust toolchain).
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
shopt -s globstar nullglob

expected=(nva-abi nva-pack nva-rewrite nva-runtime nva-verify)
metadata="$(cargo metadata --no-deps --format-version 1)"

count="$(jq '.packages | length' <<<"$metadata")"
if [ "$count" -ne "${#expected[@]}" ]; then
  echo "workspace membership: expected ${#expected[@]} crates, got $count" >&2
  jq -r '.packages[].name' <<<"$metadata" | sort >&2
  exit 1
fi

actual="$(jq -r '.packages[].name' <<<"$metadata" | sort)"
want="$(printf '%s\n' "${expected[@]}" | sort)"
if [ "$actual" != "$want" ]; then
  echo "workspace membership: crate names do not match #39" >&2
  diff -u <(printf '%s\n' "$want") <(printf '%s\n' "$actual") >&2 || true
  exit 1
fi
echo "workspace membership: ${count}/${#expected[@]} (${expected[*]})"

# The APK shell is Kotlin/JNI built by aapt2/d8/apksigner, never a cargo member.
# No member directory may carry Android build inputs, at any depth.
while IFS= read -r member_dir; do
  for name in AndroidManifest.xml build.gradle build.gradle.kts; do
    for found in "$member_dir"/**/"$name"; do
      echo "APK shell input inside a workspace member: $found" >&2
      exit 1
    done
  done
done < <(jq -r '.packages[].manifest_path | sub("/Cargo.toml$"; "")' <<<"$metadata")

# A member build script or the workspace cargo config must not pull in Android.
while IFS= read -r build_script; do
  if grep -Eiq 'android|ndk|aapt2|apksigner|sdkmanager|\bd8\b' "$build_script"; then
    echo "Android-dependent build script in the workspace: $build_script" >&2
    exit 1
  fi
done < <(jq -r '.packages[].targets[] | select(.kind | index("custom-build")) | .src_path' <<<"$metadata")

if [ -f .cargo/config.toml ] && grep -Eiq 'android|ndk|aarch64-linux-android' .cargo/config.toml; then
  echo "Android-dependent workspace cargo config: .cargo/config.toml" >&2
  exit 1
fi

echo "APK shell: not a workspace member; no Android-dependent build scripts"

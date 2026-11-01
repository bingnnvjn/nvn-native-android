#!/usr/bin/env python3
"""Generate the synthetic import-pack fixtures (#42).

The corpus is a minimum import pack and the rewrite product it is packed from.
It contains no game bytes: every title identifier is zero and the only code is
a single AArch64 `ret` instruction.

Two generations must be byte-identical (docs/PIPELINE.md §5.1). The rules:

  * canonical JSON: sorted keys, `,` and `:` separators, UTF-8, trailing LF;
  * every list is sorted by a stated key;
  * no time fields, no absolute paths, no user or host names;
  * binary fields are little-endian and every padding byte is zero.

Usage:
  gen_fixtures.py --write        write ci/fixtures/ in place
  gen_fixtures.py --check        compare ci/fixtures/ with a fresh build
  --root DIR                     repository root (default: this checkout)

Exit 0 on success, 1 when --check finds a difference, 2 on a usage error.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

PACK_MANIFEST = "ci/fixtures/pack-min/manifest.json"
LOAD_PLAN = "ci/fixtures/pack-min/load-plan.json"
REWRITE_MANIFEST = "ci/fixtures/rewrite-min/rewrite-manifest.json"
REWRITE_CODE = "ci/fixtures/rewrite-min/code/main.rewritten"
SHADER_CACHE = "ci/fixtures/rewrite-min/shader-cache.bin"

TEXT_VADDR = 0x1000
TEXT_SIZE = 4
ZERO_TITLE_ID = "0" * 16
ZERO_BUILD_ID = "0" * 64

# Not generated, but allowed next to the generated files.
EXTRA_ALLOWED = {
    "ci/fixtures/README.md",
    "ci/fixtures/gen_fixtures.py",
}


def canonical_json(value: object) -> bytes:
    """Canonical JSON for this corpus.

    Keys are ASCII and values are integers, strings, arrays and objects, so
    sorted keys plus `,`/`:` separators are enough to match JCS here. Floats
    are never generated, so there is no number-formatting divergence.
    """
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def synthetic_nso() -> bytes:
    """An NSO0 module: 0x100-byte header plus one AArch64 `ret`."""
    header = bytearray(0x100)
    header[0:4] = b"NSO0"
    # text: file offset 0x100, memory offset 0, size 4. rodata, data, bss and
    # the 32-byte module ID stay zero.
    struct.pack_into("<III", header, 0x10, 0x100, 0, TEXT_SIZE)
    return bytes(header) + struct.pack("<I", 0xD65F03C0)


def shader_cache() -> bytes:
    """A synthetic shader cache: header, one SPIR-V blob, one index entry.

    docs/ARCHITECTURE.md §3.3 defines the 50-byte index entry but not the file
    header. The fixture adds the header documented in ci/fixtures/README.md.
    The SPIR-V blob is the 5-word module header only; it is not a valid module
    and spirv-val belongs to the L2 ticket.
    """
    spirv = struct.pack("<IIIII", 0x07230203, 0x00010000, 0, 1, 0)
    index_offset = 0x20 + len(spirv)
    header = struct.pack(
        "<4sIIIIIII",
        b"NVSC",
        1,  # format_version
        1,  # key_algo_id: SHA-256(control ‖ code)
        0,  # translator_build_id
        0,  # target_vk_api
        1,  # entry_count
        index_offset,
        0,  # reserved
    )
    # One entry: zero key, stage 0 (vertex), no flags, one SPIR-V blob, no deps.
    entry = bytes(32) + struct.pack(
        "<BBIIII", 0, 0, 0x20, len(spirv), 0, 0
    )
    return header + spirv + entry


def rewrite_manifest(module_sha256: str) -> dict:
    return {
        "input_sha256": module_sha256,
        "output_sha256": module_sha256,
        "build_id": ZERO_BUILD_ID,
        "coverage": {
            "total": 1,
            "classified": 1,
            "by_rule": {"keep": 1},
            "unknown": [],
        },
        "fixups": [],
        "gates": [],
        "ruleset": "synthetic/v0",
        "rewriter_version": "0.1.0",
        "degraded": [],
    }


def load_plan() -> dict:
    return {
        "version": 1,
        "entry": TEXT_VADDR,
        "segments": [
            {
                "module": "main",
                "name": "text",
                "vaddr": TEXT_VADDR,
                "size": TEXT_SIZE,
                "perm": "rx",
            }
        ],
    }


def pack_manifest() -> dict:
    manifest = {
        "version": 1,
        "title_id": ZERO_TITLE_ID,
        "title_version": 0,
        "build_id": ZERO_BUILD_ID,
        "generation": 1,
        "keys": {"generation": 1, "key_count": 0},
        "degraded": [],
    }
    manifest["pack_id"] = sha256_hex(canonical_json(manifest))
    return manifest


def build() -> dict[str, bytes]:
    module = synthetic_nso()
    files = {
        REWRITE_CODE: module,
        SHADER_CACHE: shader_cache(),
        REWRITE_MANIFEST: canonical_json(rewrite_manifest(sha256_hex(module))) + b"\n",
        LOAD_PLAN: canonical_json(load_plan()) + b"\n",
        PACK_MANIFEST: canonical_json(pack_manifest()) + b"\n",
    }
    return {path: files[path] for path in sorted(files)}


def write(root: Path) -> int:
    files = build()
    for rel in sorted(files):
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(files[rel])
    print(f"gen_fixtures: wrote {len(files)} files")
    return 0


def check_pack_id(root: Path) -> bool:
    path = root / PACK_MANIFEST
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"gen_fixtures: cannot read {PACK_MANIFEST}: {exc}", file=sys.stderr)
        return False
    pack_id = manifest.pop("pack_id", None)
    if pack_id != sha256_hex(canonical_json(manifest)):
        print("gen_fixtures: pack_id does not match the manifest", file=sys.stderr)
        return False
    return True


def check_no_extra(root: Path, expected: dict[str, bytes]) -> bool:
    """Reject files that are neither generated nor part of the corpus docs."""
    directory = root / "ci/fixtures"
    if not directory.is_dir():
        return False
    fail = False
    for path in sorted(directory.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(root).as_posix()
        if rel in expected or rel in EXTRA_ALLOWED:
            continue
        print(f"gen_fixtures: unexpected file {rel}", file=sys.stderr)
        fail = True
    return fail


def check(root: Path) -> int:
    first = build()
    second = build()
    if first != second:
        print("gen_fixtures: two in-memory generations differ", file=sys.stderr)
        return 1

    fail = 0
    for rel in sorted(first):
        path = root / rel
        if not path.is_file():
            print(f"gen_fixtures: missing {rel}", file=sys.stderr)
            fail = 1
            continue
        if path.read_bytes() != first[rel]:
            print(f"gen_fixtures: {rel} differs from a fresh generation", file=sys.stderr)
            fail = 1
    if not check_pack_id(root):
        fail = 1
    if check_no_extra(root, first):
        fail = 1
    if fail:
        return 1
    print(f"gen_fixtures: {len(first)} files match a fresh generation")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true", help="write the fixtures")
    mode.add_argument(
        "--check",
        action="store_true",
        help="compare the committed fixtures with a fresh generation",
    )
    parser.add_argument(
        "--root",
        default=str(Path(__file__).resolve().parents[2]),
        help="repository root (default: this checkout)",
    )
    args = parser.parse_args(argv[1:])
    root = Path(args.root).resolve()
    if args.write:
        return write(root)
    return check(root)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

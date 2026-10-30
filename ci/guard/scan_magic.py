#!/usr/bin/env python3
"""Content-level red-line checks that a name/suffix list cannot catch.

Two problems this closes (#16, review 2026-10-31):

  * renaming. `main` / `sdk` / `foo.bin` copied out of an XCI still starts with
    the same magic at offset 0. A suffix blacklist misses it entirely.
  * keys pasted as text. `prod.keys`, `keys.txt`, `notes.dat` all carry the
    same `name = <32 hex>` shape.

Magic is matched at its real offset (0, or 0x100 for KIP1), not by grepping
the first 4 KiB — a doc that merely *mentions* "PFS0" must not trip this.

Usage: scan_magic.py --stdin-null   (null-separated paths on stdin)
       scan_magic.py <path>...
Exit 0 when clean, 1 when something matched, 2 on usage error.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

MAGIC_AT_0 = {
    b"NSO0": "NSO program (exefs)",
    b"PFS0": "PFS0 partition",
    b"NCA\x00": "NCA content archive",
    b"NCA0": "NCA content archive",
    b"NPDM": "NPDM metadata",
    b"HEAD": "XCI card header",
    b"NRO0": "NRO",
    b"NSO\x00": "NSO program",
    b"\x03\x02\x23\x07": "SPIR-V module (little endian)",
}
MAGIC_AT_0X100 = {b"KIP1": "KIP kernel module"}

TEXT_SUFFIXES = {".txt", ".dat", ".bin", ".cfg", ".ini", ".conf", ".keys", ".json"}
KEY_LINE = re.compile(r"^[0-9a-fA-F]{32}\s*=\s*[0-9a-fA-F]{32}\s*$")
LONG_HEX = re.compile(r"^[0-9a-fA-F]{64}\s*$")

# Docs may quote key material as an example; source files may not.
DOC_PREFIXES = ("docs/", "research/", "README.md", "GLOSSARY.md", "CODING_STANDARDS.md", "AGENTS.md")
# Synthetic fixtures must carry real magic (NSO0, PFS0) to exercise the tools;
# they contain no game bytes by construction (title IDs are all zero).
FIXTURE_PREFIXES = ("ci/fixtures/",)
# ci/signing/fingerprint.txt is a published 64-hex certificate digest, not key
# material; the key-material check must not flag its own pin file.
FINGERPRINT_PREFIXES = ("ci/signing/",)


def read_head(path: Path, n: int = 0x110) -> bytes:
    try:
        with path.open("rb") as fh:
            return fh.read(n)
    except OSError:
        return b""


def check_one(path: Path) -> list[str]:
    hits: list[str] = []
    head = read_head(path)
    if not head:
        return hits

    rel = path.as_posix()
    if not rel.startswith(FIXTURE_PREFIXES):
        if head[:4] in MAGIC_AT_0:
            hits.append(f"magic: {MAGIC_AT_0[head[:4]]}")
        if len(head) >= 0x104 and head[0x100:0x104] in MAGIC_AT_0X100:
            hits.append(f"magic: {MAGIC_AT_0X100[head[0x100:0x104]]}")

    if (
        path.suffix.lower() in TEXT_SUFFIXES
        and not rel.startswith(DOC_PREFIXES)
        and not rel.startswith(FINGERPRINT_PREFIXES)
    ):
        try:
            if path.stat().st_size <= 2 * 1024 * 1024:
                for line in path.read_text(encoding="utf-8", errors="ignore").splitlines():
                    if KEY_LINE.match(line.strip()) or LONG_HEX.match(line.strip()):
                        hits.append("keys: name = <32 hex> material")
                        break
        except OSError:
            pass
    return hits


def main(argv: list[str]) -> int:
    if "--stdin-null" in argv:
        raw = sys.stdin.buffer.read().split(b"\0")
        paths = [Path(p.decode()) for p in raw if p]
    else:
        paths = [Path(p) for p in argv[1:]]
    if not paths:
        print("usage: scan_magic.py --stdin-null | <path>...", file=sys.stderr)
        return 2

    fail = 0
    for p in paths:
        if not p.is_file():
            continue
        for why in check_one(p):
            print(f"CONTENT-RED-LINE: {p}: {why}", file=sys.stderr)
            fail = 1
    return fail


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

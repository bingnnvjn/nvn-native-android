#!/usr/bin/env python3
"""Content-level red-line checks that a name/suffix list cannot catch.

Two problems this closes (#16, review 2026-10-31):

  * renaming. `main` / `sdk` / `foo.bin` copied out of an XCI still starts with
    the same magic at offset 0. A suffix blacklist misses it entirely.
  * keys pasted as text. `prod.keys`, `keys.txt`, `notes.dat` all carry the
    same `name = <32 hex>` shape. The name may contain `key` or `kek` anywhere,
    so `master_key_00 = ...` and `kek_01 = ...` are both caught.

Magic is matched at its real offset (0, or 0x100 for KIP1), not by grepping
the first 4 KiB — a doc that merely *mentions* "PFS0" must not trip this.
Media and archive magic is checked only in the working tree: three device
screenshots are accepted history (#33).

Usage: scan_magic.py --stdin-null   (null-separated paths on stdin)
       scan_magic.py <path>...
       scan_magic.py --history     (every blob reachable from any ref)
Exit 0 when clean, 1 when something matched, 2 on usage error.
"""
from __future__ import annotations

import re
import subprocess
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

# Working-tree only (#33): these paths already exist in history and may not be
# rewritten. Renaming still fails the tree check.
MEDIA_AT_0 = {
    b"\x89PNG\r\n\x1a\n": "PNG image",
    b"\xff\xd8\xff": "JPEG image",
    b"GIF87a": "GIF image",
    b"GIF89a": "GIF image",
    b"II*\x00": "TIFF image",
    b"MM\x00*": "TIFF image",
    b"PK\x03\x04": "ZIP archive",
    b"PK\x05\x06": "ZIP archive (empty)",
    b"\x1f\x8b": "gzip archive",
    b"7z\xbc\xaf\x27\x1c": "7z archive",
    b"Rar!\x1a\x07": "RAR archive",
    b"OggS": "Ogg media",
    b"ID3": "MP3 audio",
    b"\x1aE\xdf\xa3": "Matroska/WebM",
    b"RIFF": "RIFF media",
}

TEXT_SUFFIXES = {
    ".txt", ".dat", ".bin", ".cfg", ".ini", ".conf", ".keys", ".json",
    ".md", ".rst", ".yaml", ".yml", ".toml", ".csv", ".tsv", ".log",
    ".sh", ".py", ".rs", ".c", ".h", ".xml",
}
KEY_LINE = re.compile(
    r"^[A-Za-z0-9_.-]*(?:key|kek)[A-Za-z0-9_.-]*\s*=\s*[0-9a-fA-F]{32,}\s*$",
    re.IGNORECASE,
)
LONG_HEX = re.compile(r"^[0-9a-fA-F]{64}\s*$")

# Synthetic fixtures must carry real magic (NSO0, PFS0) to exercise the tools;
# they contain no game bytes by construction (title IDs are all zero).
FIXTURE_PREFIXES = ("ci/fixtures/",)
# ci/signing/fingerprint.txt is a published 64-hex certificate digest, not key
# material; the key-material check must not flag its own pin file.
FINGERPRINT_PREFIXES = ("ci/signing/",)

MAX_TEXT = 2 * 1024 * 1024


def is_fixture(rel: str) -> bool:
    return rel.startswith(FIXTURE_PREFIXES)


def check_bytes(data: bytes, rel: str, history: bool = False) -> list[str]:
    """Return red-line hits for one blob's bytes under a repo-relative name."""
    hits: list[str] = []
    head = data[:0x110]

    if not is_fixture(rel):
        if head[:4] in MAGIC_AT_0:
            hits.append(f"magic: {MAGIC_AT_0[head[:4]]}")
        if len(head) >= 0x104 and head[0x100:0x104] in MAGIC_AT_0X100:
            hits.append(f"magic: {MAGIC_AT_0X100[head[0x100:0x104]]}")

        if not history:
            for magic, label in MEDIA_AT_0.items():
                if head.startswith(magic):
                    hits.append(f"media: {label}")
                    break
            if len(head) >= 8 and head[4:8] == b"ftyp":
                hits.append("media: MP4/QuickTime")
            if len(head) >= 262 and head[257:262] == b"ustar":
                hits.append("media: tar archive")

    if (
        Path(rel).suffix.lower() in TEXT_SUFFIXES
        and not is_fixture(rel)
        and not rel.startswith(FINGERPRINT_PREFIXES)
        and len(data) <= MAX_TEXT
    ):
        text = data.decode("utf-8", errors="ignore")
        for line in text.splitlines():
            stripped = line.strip()
            if KEY_LINE.match(stripped) or LONG_HEX.match(stripped):
                hits.append("keys: name = <32 hex> material")
                break
    return hits


def check_one(path: Path) -> list[str]:
    try:
        with path.open("rb") as fh:
            data = fh.read(MAX_TEXT)
    except OSError:
        return []
    return check_bytes(data, path.as_posix())


def git(*args: str) -> bytes:
    return subprocess.run(
        ["git", *args], check=True, capture_output=True
    ).stdout


def history_hits() -> int:
    """Scan every blob reachable from any ref, once per object id."""
    try:
        raw = git("rev-list", "--objects", "--all").decode("utf-8", "surrogateescape")
    except (subprocess.CalledProcessError, FileNotFoundError) as exc:
        print(f"scan_magic: git history unavailable: {exc}", file=sys.stderr)
        return 2

    by_oid: dict[str, list[str]] = {}
    for line in raw.splitlines():
        oid, sep, path = line.partition(" ")
        if not sep:
            continue
        by_oid.setdefault(oid, []).append(path)

    if not by_oid:
        return 0

    # One batch call tells us which objects are blobs; trees and commits are
    # not content and must not be read as files.
    proc = subprocess.run(
        ["git", "cat-file", "--batch-check"],
        input="\n".join(by_oid) + "\n",
        text=True,
        capture_output=True,
        check=True,
    )
    kinds: dict[str, str] = {}
    for line in proc.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 2:
            kinds[parts[0]] = parts[1]

    fail = 0
    seen: set[str] = set()
    for oid, paths in by_oid.items():
        if oid in seen or kinds.get(oid) != "blob":
            continue
        seen.add(oid)
        live = [p for p in paths if not is_fixture(p)]
        if not live:
            continue
        rel = live[0]
        try:
            data = git("cat-file", "blob", oid)
        except subprocess.CalledProcessError:
            continue
        for why in check_bytes(data[:MAX_TEXT], rel, history=True):
            print(f"CONTENT-RED-LINE: history:{rel}: {why}", file=sys.stderr)
            fail = 1
    return fail


def main(argv: list[str]) -> int:
    if "--history" in argv:
        return history_hits()

    if "--stdin-null" in argv:
        raw = sys.stdin.buffer.read().split(b"\0")
        paths = [Path(p.decode()) for p in raw if p]
        if not paths:
            return 0
    else:
        paths = [Path(p) for p in argv[1:]]
    if not paths:
        print("usage: scan_magic.py --stdin-null | --history | <path>...", file=sys.stderr)
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

#!/usr/bin/env python3
"""Personal-data shape guard for the public tree (#33, #35).

This checks *shapes*, not a denylist of real device names, users, or report
IDs. A shape rule keeps working for the next machine and does not publish the
value it is trying to keep private.

Judged here: Windows drive paths, WSL mount paths, home/user directory paths,
and gpuinfo report IDs with a numeric `id`.

Usage: scan_personal.py --stdin-null   (null-separated paths on stdin)
       scan_personal.py <path>...
Exit 0 when clean, 1 when something matched, 2 on usage error.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

RULES = (
    ("windows drive path", re.compile(r"(?<![A-Za-z0-9])[A-Za-z]:[\\/]")),
    ("WSL mount path", re.compile(r"/mnt/")),
    ("home directory path", re.compile(r"/home/[A-Za-z0-9._-]+/")),
    ("macOS user path", re.compile(r"/Users/[A-Za-z0-9._-]+/")),
    ("gpuinfo report id", re.compile(r"displayreport\.php\?id=[0-9]+")),
)

# The guards and the policy document contain these patterns by construction.
# Everything else is checked, including untracked files.
EXEMPT_PREFIXES = ("ci/guard/",)
EXEMPT_FILES = ("docs/PRIVACY.md",)


def exempt(rel: str) -> bool:
    return rel.startswith(EXEMPT_PREFIXES) or rel in EXEMPT_FILES


def check_one(path: Path) -> list[str]:
    rel = path.as_posix()
    if exempt(rel):
        return []
    try:
        data = path.read_bytes()
    except OSError:
        return []
    if len(data) > 2 * 1024 * 1024 or b"\0" in data[:8192]:
        return []

    hits: list[str] = []
    text = data.decode("utf-8", errors="ignore")
    for line_no, line in enumerate(text.splitlines(), 1):
        for name, rule in RULES:
            if rule.search(line):
                hits.append(f"{rel}:{line_no}: {name}: {line.strip()[:160]}")
    return hits


def main(argv: list[str]) -> int:
    if "--stdin-null" in argv:
        raw = sys.stdin.buffer.read().split(b"\0")
        paths = [Path(p.decode()) for p in raw if p]
        if not paths:
            return 0
    else:
        paths = [Path(p) for p in argv[1:]]
    if not paths:
        print("usage: scan_personal.py --stdin-null | <path>...", file=sys.stderr)
        return 2

    fail = 0
    for p in paths:
        if not p.is_file():
            continue
        for why in check_one(p):
            print(f"PERSONAL-RED-LINE: {why}", file=sys.stderr)
            fail = 1
    return fail


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

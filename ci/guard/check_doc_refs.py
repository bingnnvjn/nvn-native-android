#!/usr/bin/env python3
"""Reject cross-references to documentation sections that do not exist.

The repo cites sections as `docs/GATES.md §2.1`, `GATES §2.1`, `ARCH §7.4`,
etc. A stale section number is invisible to every other gate, so it survives
review and sends the next reader to the wrong place. Three-level refs such as
`PIPELINE §4.3.4` point at list items rather than headings, so they are skipped.
"""
from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys

ALIASES = {
    "ARCH": "docs/ARCHITECTURE.md",
    "GATES": "docs/GATES.md",
    "PIPELINE": "docs/PIPELINE.md",
    "RISKS": "docs/RISKS.md",
    "PRIVACY": "docs/PRIVACY.md",
}

REF_RE = re.compile(
    r"(?:(?P<path>docs/[A-Za-z0-9_-]+\.md)"
    r"|(?P<alias>ARCH|GATES|PIPELINE|RISKS|PRIVACY))"
    r"\s*§\s*(?P<section>\d+(?:\.\d+)?)(?!\.\d)"
)
HEADING_RE = re.compile(r"^#{2,4}\s+(\d+(?:\.\d+)*)(?:[.、\s]|$)")

# The guard self-tests embed deliberately dangling refs as fixtures.
IGNORE = {"ci/guard/test_guards.sh"}


def repo_files(root: pathlib.Path) -> list[pathlib.Path]:
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"],
            check=True,
            capture_output=True,
        ).stdout
    except (FileNotFoundError, subprocess.CalledProcessError):
        return sorted(
            path for path in root.rglob("*")
            if path.is_file() and ".git" not in path.parts
        )
    return [root / name.decode() for name in out.split(b"\0") if name]


def headings(path: pathlib.Path) -> set[str]:
    sections: set[str] = set()
    for line in path.read_text(encoding="utf-8", errors="ignore").splitlines():
        match = HEADING_RE.match(line)
        if match:
            sections.add(match.group(1))
    return sections


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", help="repository root (default: this checkout)")
    args = parser.parse_args()

    root = (
        pathlib.Path(args.root).resolve()
        if args.root
        else pathlib.Path(__file__).resolve().parents[2]
    )
    cache: dict[pathlib.Path, set[str]] = {}
    checked = 0
    failures: list[tuple[pathlib.Path, int, str, str]] = []

    for path in repo_files(root):
        try:
            data = path.read_bytes()
        except OSError:
            continue
        if b"\x00" in data[:8192]:
            continue
        rel = path.relative_to(root)
        if rel.as_posix() in IGNORE:
            continue
        for lineno, line in enumerate(
            data.decode("utf-8", errors="ignore").splitlines(), 1
        ):
            for match in REF_RE.finditer(line):
                checked += 1
                target = root / (
                    match.group("path") or ALIASES[match.group("alias")]
                )
                section = match.group("section")
                if not target.is_file():
                    failures.append(
                        (rel, lineno, match.group(0), f"{target.relative_to(root)} does not exist")
                    )
                    continue
                if target not in cache:
                    cache[target] = headings(target)
                if section not in cache[target]:
                    failures.append(
                        (
                            rel,
                            lineno,
                            match.group(0),
                            f"{target.relative_to(root)} has no section §{section}",
                        )
                    )

    for rel, lineno, ref, why in failures:
        print(f"{rel}:{lineno}: {ref!r}: {why}", file=sys.stderr)
    if failures:
        print(f"doc refs: {len(failures)} dangling of {checked} checked", file=sys.stderr)
        return 1
    print(f"doc refs: {checked} checked, all resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())

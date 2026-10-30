#!/usr/bin/env python3
"""Reconcile docs/DERIVED.md against the tree.

#16 rule: identifier-only tables (NVN names, addresses, patch rules) may ship,
but every one of them is disclosed with size and reason. This script makes the
disclosure mechanical:

  * every row in the ledger must point at a real file, with a size that roughly
    matches (25% tolerance, sizes drift with edits);
  * every derived-looking table in the tree must be in the ledger — by name
    (*.tsv, *-table.json, policy.json, svc_gates.json, expected-*.json, *.csv),
    or by content (a non-doc file carrying five or more title IDs or `main+0x`
    addresses is an address/title table whether it says so or not).

Tests in ci/fixtures/ are synthetic (title IDs are all zero) and are exempt.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LEDGER = ROOT / "docs" / "DERIVED.md"

ROW_RE = re.compile(r"^\|\s*`([^`]+)`\s*\|\s*([^|]+?)\s*\|")
SIZE_RE = re.compile(r"([0-9]+(?:\.[0-9]+)?)\s*(B|KB|KiB|MB|MiB|k|K)\b")
DERIVED_GLOBS = (
    "*.tsv",
    "*-table.json",
    "*_map.tsv",
    "shader_*.json",
    "policy.json",
    "svc_gates.json",
    "svc-gates.json",
    "expected-*.json",
    "*.csv",
)
TITLE_ID = re.compile(r"0100[0-9a-fA-F]{12}")
ADDRESS_ROW = re.compile(r"main\+0x[0-9a-fA-F]+")
EXEMPT_PREFIXES = ("ci/guard/", "ci/signing/", "ci/fixtures/", "docs/")

UNITS = {"B": 1, "KB": 1000, "KiB": 1024, "MB": 1000**2, "MiB": 1024**2, "k": 1024, "K": 1024}


def parse_size(text: str) -> float | None:
    m = SIZE_RE.search(text)
    if not m:
        return None
    return float(m.group(1)) * UNITS[m.group(2)]


def tracked_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    ).stdout
    return [p for p in out.decode().split("\0") if p]


def derived_by_name(path: str) -> bool:
    if path.startswith(EXEMPT_PREFIXES):
        return False
    return any(Path(path).match(g) for g in DERIVED_GLOBS)


def derived_by_content(path: str) -> bool:
    if path.startswith(EXEMPT_PREFIXES) or Path(path).suffix in {".md", ".sh", ".py", ".yml", ".yaml"}:
        return False
    target = ROOT / path
    try:
        if target.stat().st_size > 2 * 1024 * 1024:
            return False
        text = target.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return False
    return len(TITLE_ID.findall(text)) + len(ADDRESS_ROW.findall(text)) >= 5


def main() -> int:
    if not LEDGER.is_file():
        print("ledger: missing docs/DERIVED.md", file=sys.stderr)
        return 1

    rows: dict[str, float | None] = {}
    for line in LEDGER.read_text(encoding="utf-8").splitlines():
        m = ROW_RE.match(line)
        if m:
            rows[m.group(1)] = parse_size(m.group(2))

    fail = 0

    for path, declared in rows.items():
        target = ROOT / path
        if not target.is_file():
            print(f"ledger: row points at a missing file: {path}", file=sys.stderr)
            fail = 1
            continue
        if declared is None:
            continue
        actual = target.stat().st_size
        if actual == 0 or abs(actual - declared) / max(declared, 1) > 0.25:
            print(
                f"ledger: size drift for {path}: declared {declared:.0f} B, actual {actual} B",
                file=sys.stderr,
            )
            fail = 1

    for p in tracked_files():
        if p in rows:
            continue
        if derived_by_name(p):
            print(f"ledger: derived table not disclosed (by name): {p}", file=sys.stderr)
            fail = 1
        elif derived_by_content(p):
            print(f"ledger: title/address table not disclosed (by content): {p}", file=sys.stderr)
            fail = 1

    if fail:
        print("ledger: FAILED — every derived table needs a row (path, size, what, why).", file=sys.stderr)
        return 1
    print(f"ledger: OK ({len(rows)} declared tables)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

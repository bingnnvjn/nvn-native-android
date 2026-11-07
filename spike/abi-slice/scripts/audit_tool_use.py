#!/usr/bin/env python3
"""Report read-vs-bash tool use for a DSH session log.

Usage:
  python3 scripts/audit_tool_use.py <session.v4.jsonl.zstd>
  python3 scripts/audit_tool_use.py --latest [--strict]

--latest  pick the newest ~/.dsh/sessions/*/session-*/session.v4.jsonl.zstd
--strict  exit 1 when inspection-shaped bash commands are present
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import re
import subprocess
import sys


def latest_session() -> pathlib.Path:
    root = pathlib.Path.home() / ".dsh" / "sessions"
    candidates = list(root.glob("*/session-*/session.v4.jsonl.zstd"))
    if not candidates:
        sys.exit(f"no session logs under {root}")
    return max(candidates, key=lambda path: path.stat().st_mtime)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("session", nargs="?", help="path to session.v4.jsonl.zstd")
    parser.add_argument(
        "--latest", action="store_true", help="use the newest session log"
    )
    parser.add_argument(
        "--strict", action="store_true", help="exit 1 on inspection-shaped bash"
    )
    args = parser.parse_args()
    if args.latest == bool(args.session):
        parser.error("pass either a session path or --latest")
    path = pathlib.Path(args.session) if args.session else latest_session()

    raw = subprocess.run(
        ["zstd", "-d", "-c", str(path)], check=True, capture_output=True
    ).stdout.decode("utf-8", "replace")

    calls = []
    bash_heads = collections.Counter()
    bash_rg = 0
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") != "tool/call":
            continue
        data = event.get("data") or {}
        name = data.get("name")
        calls.append(name)
        if name == "bash":
            try:
                arguments = json.loads(data.get("arguments") or "{}")
            except json.JSONDecodeError:
                arguments = {}
            command = arguments.get("command") or ""
            # rg is the allowed bash fallback for search; it is not inspection-shaped.
            for head in re.findall(
                r"(?:^|&&|;)\s*(cat|grep|find|sed|head|tail|wc)\b", command
            ):
                bash_heads[head] += 1
            bash_rg += len(re.findall(r"(?:^|&&|;)\s*rg\b", command))

    counts = collections.Counter(calls)
    inspection = sum(bash_heads.values())
    print(f"session: {path}")
    print("tool calls:", len(calls))
    for name in ("bash", "read", "glob", "grep", "write", "edit"):
        print(f"  {name}: {counts.get(name, 0)}")
    print("inspection-shaped bash (cat/grep/find/sed/head/tail/wc):", inspection)
    for head, count in bash_heads.most_common():
        print(f"  {head}: {count}")
    print("allowed bash rg fallback:", bash_rg)
    return 1 if args.strict and inspection else 0


if __name__ == "__main__":
    sys.exit(main())

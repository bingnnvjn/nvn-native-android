#!/usr/bin/env python3
"""Report read-vs-bash tool use for a DSH session log.

Usage: python3 scripts/audit_tool_use.py /path/to/session.v4.jsonl.zstd
"""
from __future__ import annotations

import collections
import json
import re
import subprocess
import sys

if len(sys.argv) != 2:
    sys.exit("usage: audit_tool_use.py <session.v4.jsonl.zstd>")

raw = subprocess.run(
    ["zstd", "-d", "-c", sys.argv[1]], check=True, capture_output=True
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
            args = json.loads(data.get("arguments") or "{}")
        except json.JSONDecodeError:
            args = {}
        cmd = args.get("command") or ""
        # rg is the allowed bash fallback for search; it is not inspection-shaped.
        for head in re.findall(r"(?:^|&&|;)\s*(cat|grep|find|sed|head|tail|wc)\b", cmd):
            bash_heads[head] += 1
        bash_rg += len(re.findall(r"(?:^|&&|;)\s*rg\b", cmd))

counts = collections.Counter(calls)
print("tool calls:", len(calls))
for name in ("bash", "read", "glob", "grep", "write", "edit"):
    print(f"  {name}: {counts.get(name, 0)}")
print("inspection-shaped bash (cat/grep/find/sed/head/tail/wc):",
      sum(bash_heads.values()))
for head, n in bash_heads.most_common():
    print(f"  {head}: {n}")
print("allowed bash rg fallback:", bash_rg)

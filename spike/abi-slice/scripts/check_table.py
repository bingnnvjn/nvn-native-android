#!/usr/bin/env python3
"""Check that the Rust slot table and nvn-table.json agree, in order."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
lib = (root / "src/lib.rs").read_text()
table = json.loads((root / "nvn-table.json").read_text())

m = re.search(r"const\s+SLOT_COUNT:\s*usize\s*=\s*(\d+)\s*;", lib)
if not m:
    sys.exit("FAIL: SLOT_COUNT not found in src/lib.rs")
slot_count = int(m.group(1))

m = re.search(
    r"const\s+SLOT_NAMES:\s*\[&str;\s*SLOT_COUNT\]\s*=\s*\[(.*?)\];",
    lib,
    re.S,
)
if not m:
    sys.exit("FAIL: SLOT_NAMES not found in src/lib.rs")
rust_names = re.findall(r'"([^"]+)"', m.group(1))
json_names = [s["name"] for s in table.get("slots", [])]

errors = []
if len(rust_names) != slot_count:
    errors.append(f"Rust SLOT_NAMES has {len(rust_names)} names, SLOT_COUNT={slot_count}")
if len(json_names) != slot_count:
    errors.append(f"nvn-table.json has {len(json_names)} slots, SLOT_COUNT={slot_count}")
if len(set(rust_names)) != len(rust_names):
    errors.append("Rust SLOT_NAMES has duplicate names")
if len(set(json_names)) != len(json_names):
    errors.append("nvn-table.json has duplicate names")
if rust_names != json_names:
    for i, (r, j) in enumerate(zip(rust_names, json_names)):
        if r != j:
            errors.append(f"slot {i}: Rust={r!r} JSON={j!r}")
            break
    if len(rust_names) != len(json_names):
        errors.append("name lists differ in length")
for i, s in enumerate(table.get("slots", [])):
    if s.get("slot") != i:
        errors.append(f"nvn-table.json slots[{i}].slot={s.get('slot')!r}, expected {i}")

if errors:
    print("FAIL: slot table mismatch")
    for e in errors:
        print(" -", e)
    sys.exit(1)

print(f"OK: {slot_count} slots, Rust SLOT_NAMES == nvn-table.json (order and names)")

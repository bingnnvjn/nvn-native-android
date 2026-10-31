# Derived work ledger

Every file in this repository that was derived from a Switch title — even when
it contains only identifiers — is listed here with its size and the reason it
must exist. This file is enforced by `ci/guard/check_derived_ledger.py`, and
the rule comes from #16: disclose each table, or do not ship it.

**Not derived, not listed:** original code, the C ABI, docs, tests, and build
scripts. **Never in this repo:** game programs, NSO/NCA/NSP/XCI, keys, romfs
assets or text, shader originals or translated shader source/binaries, pipeline
caches, saves.

| Path | Size | What it is | Why it must exist |
|---|---|---|---|
| `spike/abi-slice/nvn-table.json` | 14 KB | NVN slot table: names, argument classes, return class, stub class (513 name faces). No code, no addresses, no data. | The engine resolves NVN calls by name and the ABI slice asserts against the same table. |

## How to add a row

1. Keep the file identifier-only. If it contains bytes from a game file, stop.
2. Add a row: path, size, one sentence on what it is, one on why it must exist.
3. Run `python3 ci/guard/check_derived_ledger.py` — the size must be within 25%
   of the row and no derived-looking table may exist without a row.

## Document-internal derived tables

These documents contain derived identifiers but are not standalone table files.
They are disclosed here under #16; the ledger script intentionally exempts
`docs/` from its file/size rows.

- `docs/botw-assets.md` — BotW 1.6.0 title IDs and content metadata.
- `docs/shared-fonts.md` — six BFTTF sizes and SHA-256 digests.
- `docs/botw-nvn-usage.md` — ExeFS module sizes and symbol conclusions.

## Shader and patch tables (not yet present)

When the product tree lands (#20 L1/L2), these will appear and must be added
here in the same commit:

- `title/<build-id>/policy.json` — per-title HLE policy (identifiers only).
- `svc-gates.json` — generated gate numbers (numbers only).
- `shader-list.tsv` — SHA-256 keys + stages + dependencies (no shader code).

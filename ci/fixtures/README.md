# Synthetic fixtures (#42)

This directory holds the fixtures that let the artifact checks run without any
private material. Every byte is synthetic: all title identifiers are zero and
there is no game data. `gen_fixtures.py` regenerates the corpus byte for byte.

## Files

### `pack-min/`

- `manifest.json` — the pack manifest. It keeps the part of the contract in
  `docs/ARCHITECTURE.md` §3.1 and `docs/PIPELINE.md` §5.1 that the fixture
  needs: `version`, `pack_id`, `title_id`, `title_version`, `build_id`,
  `generation`, `keys`, `degraded`. `loadplan` reads `version` and `pack_id`;
  the other values are fixture content.
- `load-plan.json` — the load plan. The schema is below.

The pack has no payload on purpose: the only reader in this ticket is
`nva-verify loadplan`, and it reads JSON only. `files`, `romfs_hash`,
`selfcheck` and `dlc` stay out until a tool reads them. `pack-min` is the
metadata part of the fixture pair, not a complete loadable pack; the shader
cache lives in `rewrite-min/`.

### `rewrite-min/`

- `rewrite-manifest.json` — the L1 manifest (`docs/ARCHITECTURE.md` §3.2).
  `input_sha256` equals `output_sha256` because the synthetic module is not
  changed. `degraded` is empty.
- `code/main.rewritten` — a synthetic NSO0: a 0x100-byte header and one
  AArch64 `ret` (`C0 03 5F D6`). The name is neither `.nso` nor `main` because
  the red-line suffix and name lists reject those.
- `shader-cache.bin` — a synthetic shader cache. It is a registered exception
  in `ci/guard/allowlist.txt` (`docs/PIPELINE.md` §5.3).

## Load-plan schema (version 1)

This is the minimal schema the fixture needs. The full schema is owned by L3
and is still open (`docs/PIPELINE.md` §10).

```json
{
  "version": 1,
  "entry": 4096,
  "segments": [
    {"module": "main", "name": "text", "vaddr": 4096, "size": 4, "perm": "rx"}
  ]
}
```

- `version` — schema version. `loadplan` accepts only `1`.
- `entry` — guest VA of the first instruction. A later ticket checks that it
  falls inside an `rx` segment.
- `segments[]` — the generator writes them sorted by `(vaddr, name)`; the
  reader does not check the order. `perm` is one of `r`, `rw`, `rx`; `rwx` is
  never allowed (W^X).
- TLS, stack, file offsets and multiple modules are not in this version.

## Manifest subset

`pack-min/manifest.json` has no `files` list. The values that make the fixture
synthetic are:

- `title_id` is `0000000000000000`, `title_version` is 0, `build_id` is 64
  zeros.
- `keys` is `{"generation": 1, "key_count": 0}`: no key material, not even a
  key hash or a key path.
- `degraded` is `[]`: the shader cache exists, in `rewrite-min/`.
- `pack_id` is the SHA-256 of the canonical JSON of the manifest without
  `pack_id` (`docs/PIPELINE.md` §5.1).

## Shader-cache header

`docs/ARCHITECTURE.md` §3.3 defines the 50-byte index entry but not the file
header. The fixture adds this synthetic header:

```
offset  size  field
0x00    4     magic "NVSC"
0x04    4     format_version = 1
0x08    4     key_algo_id = 1 (SHA-256 of control ‖ code)
0x0C    4     translator_build_id = 0
0x10    4     target_vk_api = 0
0x14    4     entry_count = 1
0x18    4     index_offset = 0x34
0x1C    4     reserved = 0
0x20    20    SPIR-V blob (the 5-word module header only)
0x34    50    index entry
```

The index entry is the layout from `docs/ARCHITECTURE.md` §3.3: a zero key,
stage 0 (vertex), no flags, one 20-byte SPIR-V blob at 0x20 and no
dependencies. The blob is not a valid SPIR-V module; `spirv-val` belongs to the
L2 ticket. The cache holds no game bytes.

## Determinism

The generator enforces these rules:

- Canonical JSON: sorted keys, `,` and `:` separators, UTF-8, one trailing LF.
- Every list is sorted by a stated key.
- `pack_id` hashes the canonical JSON without the trailing LF.
- No time fields; the generator does not read a clock.
- Every path is a pack-relative POSIX literal; no absolute path, user name or
  host name.
- Binary fields are little-endian and every padding byte is zero.

## Regenerate and check

```sh
python3 ci/fixtures/gen_fixtures.py --write
python3 ci/fixtures/gen_fixtures.py --check
```

`--check` builds the corpus twice in memory, compares the two builds, then
compares them with the committed bytes and recomputes `pack_id`.
`ci/guard/test_guards.sh` runs it in CI, so a hand edit or a non-deterministic
generator fails the guard job.

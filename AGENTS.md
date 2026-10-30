# Repo map

Read only what the task needs.

- ABI spike design: `spike/abi-slice/DESIGN.md`
- ABI spike results: `spike/abi-slice/RESULTS.md`
- C ABI v1 context: `../work/research/issue19-resolution-v2.md`,
  `../work/research/review-abi.md`
- Terms: `GLOSSARY.md`
- Review rules: `CODING_STANDARDS.md`
- Cheap checks: `spike/abi-slice/scripts/check.sh`
- CI: `.github/workflows/abi-slice.yml`
- Device: `spike/abi-slice/scripts/run_android.sh`
- Phone APK: `spike/abi-slice/android/build_apk.sh`

Tool use:

- Read a file with `read` (use `offset`/`limit`).
- Find paths with `glob`.
- Search contents with the `grep` tool.
- Use `bash` for build, run, and network tools.
- If a bash search is unavoidable, use `rg` only:
  `rg pattern`, `rg --files -g '*.rs'`. Do not use `cat`, `find`, `grep`,
  `sed`, `head`, or `tail` for inspection.

Long sessions can be audited with
`spike/abi-slice/scripts/audit_tool_use.py <session.v4.jsonl.zstd>`.

## ABI change

Read `CODING_STANDARDS.md` before you change `include/nva_abi.h`,
`src/lib.rs` slot names, or `nvn-table.json`.

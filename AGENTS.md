# Repo map

Read only what the task needs.

`work/` is a local checkout scratch area. Pointers that start with `../work/`
are **本机路径，仅本机有效**; they are not part of the public repository. Do not
copy `work/` content into tracked files.

- ABI spike design: `spike/abi-slice/DESIGN.md`
- ABI spike results: `spike/abi-slice/RESULTS.md`
- Public boundary: `docs/PRIVACY.md`
- C ABI v1 context (本机路径，仅本机有效): `../work/research/issue19-resolution-v2.md`,
  `../work/research/review-abi.md`
- Terms: `GLOSSARY.md`
- Review rules: `CODING_STANDARDS.md`
- Cheap checks: `spike/abi-slice/scripts/check.sh`
- CI: `.github/workflows/ci.yml`
- Device: `spike/abi-slice/scripts/run_android.sh`
- Phone APK: `spike/abi-slice/android/build_apk.sh`

Tool use: inspect files with `read`, `glob`, `grep`; use `bash` for build,
run, and network tools. Long sessions can be audited with
`spike/abi-slice/scripts/audit_tool_use.py <session.v4.jsonl.zstd>`.

## ABI change

Read `CODING_STANDARDS.md` before you change `include/nva_abi.h`,
`src/lib.rs` slot names, or `nvn-table.json`.

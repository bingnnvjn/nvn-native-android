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
- Local build recipe (本机路径，仅本机有效): `../work/build-local.md`
- CI: `.github/workflows/ci.yml`
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

Before closing a session, audit it with
`spike/abi-slice/scripts/audit_tool_use.py --latest` (add `--strict` to fail on
inspection-shaped bash).

## ABI change

Read `CODING_STANDARDS.md` before you change the product `include/nva_abi.h`,
the spike `spike/abi-slice/include/nva_abi_spike.h`, `src/lib.rs` slot names,
or `nvn-table.json`.

## Agent skills

### Issue tracker

issue 和 spec 都走 GitHub Issues（`gh` CLI）。见 `docs/agents/issue-tracker.md`。

### Triage labels

五个 triage 角色用中文标签。见 `docs/agents/triage-labels.md`。

### Domain docs

单 context 布局：根目录 `GLOSSARY.md` + `docs/adr/`。见 `docs/agents/domain.md`。

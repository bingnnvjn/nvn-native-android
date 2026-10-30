# nvn-native-android
Native execution of Switch games on Android — NSO rewriting, Horizon HLE, NVN→Vulkan. Planning &amp; architecture.

## 不含游戏数据 / No game content

本仓库不含任何游戏数据：没有游戏程序、没有 NSO/NCA/NSP/XCI、没有 `prod.keys`/`title.keys`/`console.keys` 等任何密钥、没有 romfs 资源或文本、没有着色器原件或翻译后的着色器源码与二进制、没有 pipeline/着色器缓存、没有存档，也不分发它们。你需要自备合法获得的游戏本体与自己提取的 `prod.keys`；本项目不提供获取方式，也不回答「在哪里下载」。游戏代码与着色器在你自己的设备上、从你自己的文件中生成，生成的 import pack 仅供你本人使用，请勿上传、转存或分享；自建的 APK 不含游戏数据，不在此列。CI 产出的 APK 是空壳，不含任何游戏数据。本项目与任天堂无关联，未获其授权、背书或赞助。

This repository contains and distributes no game content — no game program, no NSO/NCA/NSP/XCI, no `prod.keys`/`title.keys`/`console.keys` or any other keys, no romfs assets or text, no shader originals or translated shader source/binaries, no pipeline or shader caches, no saves. You must own a legally obtained copy of the game and supply your own `prod.keys`, extracted by you; we do not explain how to obtain either, and we will not answer where to download them. Game code and shaders are generated on your own device from your own files, and the resulting import pack is for your own use only — do not upload, mirror, or share it. (An APK you build contains no game data, so it is not covered by that restriction.) APKs built by CI are empty shells with no game data. Not affiliated with, endorsed by, or sponsored by Nintendo.

红线清单与门禁见 [`docs/PIPELINE.md`](docs/PIPELINE.md) §6；仓内每份派生表登记在 [`docs/DERIVED.md`](docs/DERIVED.md)。

## Artifacts, not releases

CI 不建 GitHub Release。APK 只从 Actions 的 artifact 拿，用一把**公开的** artifact 密钥签名（`ci/signing/artifact.p12`），目的是让不同 CI run 的包能互相覆盖安装——**它不是发布密钥**，任何人拿到都能重签同样的包。开 Release 的前置条件写在 `docs/PIPELINE.md` §4.3。

## Build

```bash
# 廉价静态检查（C 布局、槽表、JSON、shell）
bash spike/abi-slice/scripts/check.sh

# PC 回归（需要 Vulkan / lavapipe）
bash spike/abi-slice/scripts/run_pc.sh

# Android 交叉 + APK（需要 NDK r27d 与 SDK build-tools）
bash spike/abi-slice/android/build_apk.sh

# 红线门禁（任何提交前都该跑）
bash ci/guard/scan_tree.sh && python3 ci/guard/check_derived_ledger.py
```

## Repo map

Read only what the task needs.

- Pipeline decisions (CI, artifacts, signing, import pack, red lines): `docs/PIPELINE.md`
- Derived-work ledger: `docs/DERIVED.md`
- Release-notes template (when a release is eventually cut): `docs/RELEASE-NOTES-TEMPLATE.md`
- ABI spike design: `spike/abi-slice/DESIGN.md`
- ABI spike results: `spike/abi-slice/RESULTS.md`
- C ABI v1 context: `../work/research/issue19-resolution-v2.md`,
  `../work/research/review-abi.md`
- Terms: `GLOSSARY.md`
- Review rules: `CODING_STANDARDS.md`
- Cheap checks: `spike/abi-slice/scripts/check.sh`
- CI: `.github/workflows/ci.yml` (replaces `abi-slice.yml`)
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

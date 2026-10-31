# Release notes template

Copy this file, fill the `<>` slots, keep every fixed paragraph verbatim.
`ci/guard/check_docs.sh` asserts the two bolded sentences still exist.

**This template is not used yet.** v0 ships artifacts only; see
`docs/PIPELINE.md` §4.3 for the conditions that must hold before the first
GitHub Release.

---

## nva <version> — <BotW 1.6.0 | TotK 1.x> on Android

<one paragraph: what works, what does not, on which device>

### 需要自备游戏文件与密钥 / Needs your own game files and keys

本仓库不含任何游戏数据，也不分发它们。你需要自备合法获得的游戏本体与自己提取的 `prod.keys`，本项目不提供获取方式，也不回答“在哪里下载”。游戏数据在你的电脑上生成 import pack，只供你本人使用；请勿上传、转存或分享。本项目与任天堂无关联，未获其授权或背书。

This repository contains and distributes no game content. You must own a legally obtained copy of the game and supply your own `prod.keys`; we do not explain how to obtain either. Game data is packed on your own machine, for your own use only — do not upload, mirror, or share it. Not affiliated with, endorsed by, or sponsored by Nintendo.

### 本页包含 / In this release

- `nva-<version>-arm64.apk` — 壳，零游戏数据
- `nva-pack-<os>-<arch>.zip` — 本机导入包生成工具，零游戏数据
- 源码：本 tag

**本页不含 import pack。** 它只能由你在本机生成，永不进入 release。

**This release contains no import pack.** It can only be generated on your own machine and never enters a release.

### 安装 / Install

1. 下载 `nva-<version>-arm64.apk`，核对下面的 SHA-256。
2. 用 `nva-pack` 在你自己的机器上生成 import pack。
3. 侧载 APK，首次启动时导入 pack。

签名为 release 密钥，证书 SHA-256：`<release fingerprint>`。

### 校验 / Verify

```
sha256sum nva-<version>-arm64.apk
# <sha256>
```

### 已知限制 / Known limitations

- <device matrix, framerate, missing services>

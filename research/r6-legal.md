# r6 · 公开边界：源码、APK、import pack 的法律与社区惯例

> 取证日 2026-10-24。一手来源：GitHub REST API 状态码、`github/dmca` 通知原文、各仓 README/docs 原文。新闻只作佐证并标注。

## 1. 同类项目的实际做法

| 项目 | 发二进制？ | 发 shader 清单？ | keys |
|---|---|---|---|
| [MacNativeGames](https://github.com/ly05010419/MacNativeGames)（MIT，5,091★） | **不发**（0 个 release） | 发 `games/*/assets/shader_list.tsv`，仅标识符 | 用户自备，`-k ~/keys/prod.keys` |
| [nxnative](https://github.com/seedds/nxnative)（MIT，2,779★） | **不发**（0 个 release） | 发 `dm/games/<titleid>/shader_map.tsv`、`shader_facts.tsv`、`pso_manifest.tsv` | 同上 |
| [d2r-mobile](https://github.com/d2r-mobile/d2r-mobile)（MIT，4,522★） | 发 **4.4 MB 的空壳 IPA**（`D2RMobile-shell.ipa`）；Android 不发 APK | 发 `shader_map/facts/pso_manifest.tsv` | 同上 |
| [LASRecomp-Android-Plus](https://github.com/Nycolazs/LASRecomp-Android-Plus)（MIT，133★） | **发公开 APK**（`LASRecomp-Android-1.2.0.apk`，80 MB） | 不发；构建期生成 | 应用内选 `prod.keys` |

原文摘录：

- MacNativeGames：「The repository contains no game content and distributes none: no game program, art, text, audio, shaders or keys.」并交代仓里**有**什么：「the patch code, the addresses and hashes used to check your files, and the shared shader list, which names shaders and bind states but contains no shader code.」还明确禁止转发自建产物：「The app you build contains the game, so it is for your own use only; do not share it.」([README](https://github.com/ly05010419/MacNativeGames/blob/HEAD/README.md))
- nxnative：「Nothing from any game is in this repository.」([README](https://github.com/seedds/nxnative/blob/HEAD/README.md))
- d2r-mobile 有一份可抄的清单 [docs/DERIVED.md](https://github.com/d2r-mobile/d2r-mobile/blob/HEAD/docs/DERIVED.md)：逐项列出仓内每份游戏相关记录的体积与用途，并把 **shader（原件、翻译后的 Metal 源码、编译产物）、存档、启动预取表** 明确列入「不在仓内」；导出时"用工具拿真实游戏文件逐字节扫全树，命中即阻止导出"。
- LAS-Android：「**No game files are included.** … together with the `prod.keys` from your own Switch」「Instructions on how to obtain them will **not** be provided.」([README](https://github.com/Nycolazs/LASRecomp-Android-Plus/blob/HEAD/README.md))

第五个样本是 [ScarletNX](https://github.com/scarletnx/ScarletNX)（GPL-3.0，221★）：它自称有 nightly APK，但本次查 `releases?per_page=2` 只看到 `nightly` 这个 tag、附件列表为空（APK 是否另有分发渠道 `UNVERIFIED`）。可见"发 APK"在 GPL 项目里也在做，但**没有任何一家把游戏数据塞进公开包**。

**「公开 APK」不是口头承诺，是脚本断言。** `tools/android/ondevice/check_public_apk.sh` 令 APK 出现以下任一项即失败：`liblasgame.so`；定义了 `f_70xxxxxxxx` 重编译游戏函数的 `.so`；生成器产出的 C（`lasgen: split-dispatch|cold`）；`*.nso/*.nca/*.nsp/*.xci`、`main.npdm`、`exefs/`、`romfs/`。注释里写明「an APK to share carries no code or data made from the game」。([脚本](https://github.com/Nycolazs/LASRecomp-Android-Plus/blob/HEAD/tools/android/ondevice/check_public_apk.sh)、[文档](https://github.com/Nycolazs/LASRecomp-Android-Plus/blob/HEAD/docs/android-ondevice-build.md))

## 2. 2026-08 那轮 DMCA 的范围与触发点

**触发的是「分发这个工具」，不是「仓里放了游戏文件」。** 通知原文（[2026-08-17-nintendo](https://github.com/github/dmca/blob/master/2026/08/2026-08-17-nintendo.md)）：「these Nintendo Switch emulators illegally circumvent Nintendo's TPMs … the emulators at the reported repositories necessarily use unauthorized copies of these cryptographic keys to decrypt unauthorized copies of Nintendo Switch games」→「distribution of these emulators constitutes unlawful trafficking in technology that is 'primarily designed or produced for the purpose of circumventing a technological measure' … 17 U.S.C. § 1201(a)(1) and (2)」。全篇没有一句指控仓内含游戏文件。

**范围**：2026-08-17 一天内 7 份 Nintendo 通知；`github/dmca/2026/08` 下其中 5 份带名单，去重后 **349 个仓库**（新闻常说约 401，`UNVERIFIED`，与本人数出的 349 不符）。其中最大一份是 fork 网络整体下架：「the reported network … was larger than one hundred (100) repositories … GitHub processed the takedown notice against the entire network of **311** repositories, inclusive of the parent repository」。名单构成：suyu 系 289、MonoNX 18、yuzu/yuzu-android 系 22。

**状态码证据**（`gh api`，2026-10-24）：`amaron2/MonoNX`、`1024210879/suyu`、`absol255/yuzu` 均 **451**，payload 为 `"reason":"dmca"`，`created_at 2026-08-19`，并回指上列通知；`Pinkuburu/MonoNX`、`eden-emu/eden`、`citron-emu/citron`、`sudachi-emu/sudachi`、`Kenji-NX/Kenji-NX`、`yuzu-emu/yuzu` 均 **404**（404 与 451 的差别在于删除/自行下架还是 GitHub 拦截，`UNVERIFIED`）。旧案同构：[2025-03-06](https://github.com/github/dmca/blob/master/2025/03/2025-03-06-nintendo.md) 仅点 7 个 Ryujinx 镜像/fork，`ryujinx-mirror/ryujinx` 至今 451。

**一次都没点到我们这一类**：349 个名字里没有任何 recomp / 原生执行项目；MacNativeGames 200、nxnative 200、d2r-mobile 200、LASRecomp-Android-Plus 200、ScarletNX 200 全部存活（`archived=false`，10-24 仍在 push）。

**要记住的是"整网"规则**：GitHub 的处置单位是 fork 网络，不是单仓。311 个 fork 里绝大多数是常年不动的镜像，照样一起 451。我们不控 fork，一旦上游被点名，别人 fork 的副本会被同批带走；这属于平台风控，与我们做没做错无直接关系。

**但没停手**：2026-09-30、10-01、10-23 仍有 Nintendo 通知。10-23 那批 21 个是 *Rhythm Tengoku* 反编译项目，用的是另一条法理——§512 衍生作品：「unauthorized derivative, or decompiled, copies of the video game (i) code, and (ii) audiovisual assets」（[通知](https://github.com/github/dmca/blob/master/2026/10/2026-10-23-nintendo.md)）。**这条对我们更要紧：它打的正是"代码"，且不需要仓里有游戏资源。**

## 3. 红线清单

**不带（任何公开产物）**：游戏程序与 NSO/NCA/NSP/XCI、`prod.keys`/`title.keys`、romfs 资源与文本表、shader 原件、翻译后的 shader 源码与编译产物、pipeline/着色器二进制缓存、存档。

**可带（四家全都有先例）**：仅标识符的 shader/pipeline 清单（SHA-256 + deps + 偏移）、NVN 函数名表、地址表、按 build ID 的补丁/字段表、生成器源码（含解密流程，LAS 还把 on-device 生成器打进公共 APK 的 `assets/ondevice.zip`）。

- **build-ID 补丁表算不算衍生数据？** 社区惯例是"事实记录"，可发——但它是 Nintendo 派生事实，不是游戏数据；d2r 在 DERIVED.md 里点明每条规则读作「原指令字 → 替换指令字」，并把约 800 个 `main+0x…` 地址列为仓内记录。建议照 d2r 逐项披露体积与用途。
- **APK 能否带 shader 缓存？** 无先例。MacNativeGames 把预热 shader 编译进 app，但那个 app 是用户本机自建、且写明"do not share it"；d2r 把编译产物列进"不在仓内"。结论：不进公共 APK，进本机 import pack。
- **import pack 生成工具能否公开？** 能，四家都公开，且 LAS 把生成器打进公共 APK。真正有风险的是**分发**产物，不是**发布**生成器——但不要把"生成器"和"预生成的 pack"混在一个 release 里。

**三条落地建议**：① 三件产物分三处——源码仓只放标识符表与生成器，公共 APK 零游戏数据，import pack 只在用户本机生成、永不进 release；② CI 加一个 `check_public_apk.sh` 等价脚本，判据直接照抄 LAS；③ 每增加一份游戏相关表，就在 `docs/DERIVED.md` 加一行，写清体积、用途、为什么必须在。

## 4. 建议措辞

先认清性质：免责声明不是法律屏障。四家都写了，但它们至今没被点名的原因是没有进入 Nintendo 的清单，而不是声明本身有效；`451`/`404` 是平台处置，不是司法判决。

**中文**：本仓库不含任何游戏程序、资源、文本、着色器、密钥或存档，也不分发它们。你需要自备合法获得的游戏本体与你自己的 `prod.keys`；本项目不提供获取方式。游戏代码与着色器在你自己的设备上、从你自己的文件中生成，生成的 import pack 仅供你本人使用，请勿分享。本项目与任天堂无关联，未获其授权或背书。

**English**: This repository contains and distributes no game program, assets, text, shaders, keys or saves. You must own a legally obtained copy of the game and supply your own `prod.keys`; we do not explain how to obtain either. Game code and shaders are generated on your own device from your own files, and the resulting import pack is for your own use only — do not share it. Not affiliated with, endorsed by, or sponsored by Nintendo.

建议再补一页 `docs/DERIVED.md`（照抄 d2r 的结构）：仓内每份游戏相关记录 = 文件、体积、是什么、为什么必须在；并加一个 CI 门禁脚本（照抄 `check_public_apk.sh` 的判据）。

# BotW 1.6.0 素材核验

> 核验日期：2026-10-24。文件在本机，未上传任何位置。

## 文件

| 项 | 值 |
|---|---|
| 路径 | `E:\1\romrom\1 塞尔达传说旷野之息整合版1.6.0.xci` |
| 大小 | 16,863,438,848 字节（15.7 GiB） |
| `prod.keys` | `E:\1\romrom\prod.keys`（14,996 字节） |
| 解包工具 | nstool v1.9.2（MIT），本地 `tools/nstool` |

## 卡片结构（nstool 实测）

XCI 头：`CardHeaderVersion 0`、`RomSize 16GB`、`SelT1Key 2`、`SelKey 0`。

分区：`update/`、`normal/`、`secure/`；**`update/` 与 `normal/` 为空**，全部 12 个 NCA 都在 `secure/`。

四个 CNMT 的 ProgID：

| ProgID | 内容 | 说明 |
|---|---|---|
| `01007EF00011E000` | Base | 本体 |
| `01007EF00011E800` | Patch | 更新，**Title Version `v786432` = 1.6.0** |
| `01007EF00011F001` | AddOnContent 1 | DLC 1 |
| `01007EF00011F002` | AddOnContent 2 | DLC 2（只有 `AocVersion.txt`，648 字节） |

更新包的 CNMT 还给出：`RequiredSystemVersion 8.0.0`、Program NCA `61c5e2bb…`（0x3b223000 ≈ 992 MB）、`SdkAddonVer 7.3.0`。

## 结论

1. **keys 够用。** 更新包的 `Key Generation` 是 3，`prod.keys` 能解开；缺 `title.keys` 只影响 NSP ticket，不影响 XCI。
2. **不需要单独的 1.6.0 更新 NSP。** MacNativeGames 的 `build_app.py` 明确支持「1.6.0 作为一张卡带镜像，含本体、更新与追加内容」。CNMT 的 `v786432` 与 `expected-v1.6.0.json` 的 `version: 786432` 一致。
3. **卡带自带两个 DLC。** `expected-v1.6.0.json` 也登记了 AOC 1 与 AOC 2 的尺寸与 hash；1.6.0 表单不需要 `--dlc`。
4. **磁盘够。** `E:` 剩余 57 GB；解包工作目录约 25 GB，加一份输出仍有余量。

## 缺口：系统共享字体不在卡里

`expected-v1.6.0.json` 的注释写明：

> the system shared fonts (the same six the Tears of the Kingdom card carries; **this card's system update partition is empty**, so they come from a cached or given font folder)

也就是说，游戏正文要用六份 `.bfttf` 系统字体，来源只有两种：

- 一份 firmware 解包目录（`--firmware`），或
- 某个 TotK 构建留下的 `extracted/firmware/shared-fonts`（`--shared-fonts`）。

我们两样都没有，手上也没有 Switch。**这是当前唯一挡在「能构建」前面的素材缺口**，且对我们自己的 host 同样成立：没有这六份字体，游戏文字会空白（#13 已把它列为 BotW 专属坑）。

已知可行的绕过：`build_app.py` 会用结构校验（`_bfttf_valid`）接受来自其他 firmware 版本的有效 BFTTF；所以只要有任一份 firmware 的共享字体即可，不要求与 1.6.0 同版本。

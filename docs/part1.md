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
2. **不需要单独的 1.6.0 更新 NSP。** MacNative
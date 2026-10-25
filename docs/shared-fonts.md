# 系统共享字体（本机，不入库）

> 用途：BotW 的文字渲染。六份 `.bfttf` **不是游戏文件**，是固件里的系统数据；它们不能进仓库。

## 来源

- 固件：Eden 模拟器已安装的 21.0 固件，`nand/system/Contents/registered/`（本机副本 `E:\1\romrom\firmware\registered`，234 个文件 / 343 MB）。
- 密钥：`E:\1\romrom\prod.keys`（Eden 的，代际覆盖到 `0x14`）。
- 提取器：MacNativeGames `games/totk/scripts/extract_shared_fonts.py`（连同 `extract_exefs.py`、`extract_romfs.py`，本地 `tools/mng/`）。
- 命令：

```
python3 extract_shared_fonts.py \
  --keys /mnt/e/1/romrom/prod.keys \
  --firmware /mnt/e/1/romrom/firmware/registered \
  --output <本机目录>/fonts
```

校验：NCA FS header + 六层 IVFC + 每个 BFTTF 头（解码为 sfnt）。

## 结果

| 类型 | 文件 | 字节 | SHA-256 |
|---|---|---|---|
| 0 标准 | `0-standard.bfttf` | 2,053,628 | `823913c9…7ae7d45d2ad` |
| 1 简体中文 | `1-chinese-simplified.bfttf` | 7,848,200 | `a04eb490…2003938` |
| 2 简中扩展 | `2-chinese-simplified-extension.bfttf` | 123,912 | `f5784e7a…fd9b9ff` |
| 3 繁体中文 | `3-chinese-traditional.bfttf` | 3,580,260 | `9d59b74e…8bd7457` |
| 4 韩文 | `4-korean.bfttf` | 3,672,868 | `5a4e6947…157c9e8` |
| 5 任天堂扩展 | `5-nintendo-extension.bfttf` | 180,236 | `f9f62e02…6174af68` |

`manifest.json` 记录完整 hash 与校验状态。

## 与上游钉的 hash

- **标准字体**与 `expected-v1.6.0.json` 的 `shared_fonts_sha256.standard` **完全一致**。
- 其余五份与之不同：来自 21.0 固件，不是上游参考 dump 的版本。
- 上游 `build_app.py` 的 `font_report()` 明确接受这种情况：只要 BFTTF 结构有效，就按实际字节绑定，并记录 hash。所以**可用**。

## 边界

- 字体**不进仓库**、不进 APK、不分发；只在本机 `fonts/`（及生成 import pack 时使用）。
- 若以后换固件版本，重跑一次提取，`manifest.json` 会更新 hash。

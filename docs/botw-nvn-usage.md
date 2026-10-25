# BotW 1.6.0 的 NVN 使用面（静态可得部分）

> 方法：ExeFS 提取 + NSO 符号表 + `shader_list.tsv` 依赖解析。
> 结论分「已确定」「高置信推断」「仍然未知」三档。

## 1. ExeFS 已提取并校验

`games/botw/scripts/extract_exefs.py --keys prod.keys` 从卡带的 secure 分区提取，哈希与 `expected-v1.6.0.json` 一致：

| 模块 | 大小 | 说明 |
|---|---|---|
| `main` | 23,223,060 | 游戏主体（NSO0，三段 LZ4） |
| `sdk` | 5,310,646 | Nintendo SDK 7.3.2，**NVN 在这里** |
| `subsdk0` | 3,557,941 | movie |
| `rtld` | 7,435 | |

## 2. NVN 不能按名字扫

- `main` 的 SDK API 列表里没有 NVN；它只用 NintendoWare（Atk/Font/G3d/Ui2d/Vfx）与 Havok。
- `sdk` 导入 `NintendoSDK_NVN-7_3_2-Release`，但整个模块只有 3 个 `nvn*` 符号：
  `nvnBootstrapLoader`、`nvnBootstrapLoaderInternal`、`nvnDevtoolsBootstrap`。
- 也就是说，NVN 的函数表是 bootstrap 返回的结构体，调用全走间接指针。**没有逐函数的符号名可查**，静态按名扫描到此为止。

## 3. 从 shader 表能确定的事

`games/botw/assets/shader_list.tsv`（3,728 条 V 变体，8.3 MB）的 `deps` 字段记录了绑定点类型。

**片元输出（fout）数量分布**（= 同时写几个 render target）：

| 输出数 | 变体数 |
|---|---|
| 0 | 1,514 |
| 1 | 1,812 |
| 2 | 23 |
| 3 | 160 |
| 4 | 195 |
| 5 | 14 |
| 6 | 4 |
| **8** | **6** |

- **结论：BotW 确实用 MRT，最多 8 个颜色附件**，正好等于 Adreno 725 的 `maxColorAttachments = 8`。引擎必须原生支持 8 个附件，没有余量。
- 输出槽位是稀疏的（如 `0,1,3,5`），说明部分附件不写——引擎不要假设连续绑定。

**其它**

- `img`（storage image）在所有 3,728 条变体里都是空 → 预热表里**没有 compute/image-store 变体**；`C`（compute pipeline）行也是 0 条。
- `tex` 最多 17 个采样器；`attr` 最多 12 个以上。
- `S`（shader library）行的 stage 只有 0 与 1（顶点/片元），**没有几何/细分阶段的库**。

## 4. 与 R2 缺口表的对照（重要修正）

- **MRT：必须做。** 这是硬需求，不是可选项；且上限就是 8。
- **条件渲染：很可能不需要。** NVN API 本身没有条件渲染；R2 把它列为 P0，是把 Vulkan 的扩展当成了游戏会用的特性。BotW 用的是遮挡查询，由 CPU 侧决定。
- **几何/细分着色器：很可能不需要。** 预热表里没有对应 stage；需要运行时 trace 才能 100% 确认。
- **去 swizzle：仍然必须做。** 这与 shader 表无关，是纹理上传路径。

## 5. 仍然未知（需要运行时 trace）

- draw 级用法：indirect draw、遮挡查询频率、render pass 切换粒度。
- 是否有运行时编译的 shader 变体（不在预热表里）。
- 纹理格式的真实分布（BC vs ASTC vs 未压缩）。

这些不影响当前架构结论：引擎按「8 附件 + 无 compute 前提」起步，把 trace 留到有可运行 host 之后。

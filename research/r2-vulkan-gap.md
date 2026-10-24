# R2：d2r Vulkan 引擎 → BotW 1.6.0 的 NVN 缺口

> 基座：`d2r-mobile/d2r-mobile@dev`（a8782caa）的 `dm/gfx/vulkan`（82 文件，Adreno 730 上 D2R 60fps）。
> per-title 资产：`ly05010419/MacNativeGames`（无 `dm/`，布局为 `games/botw` + `games/totk`）。
> 验收机：Adreno 725 / Android 15。事实底稿：`/home/myworkspace/DeepSeekharness/research/switch-native-android-research.md` §4.3。

## 1. d2r Vulkan 引擎入口清单

读了 4 个优先文件 + 2 个旁证：[engine/xbg_nvn.cpp](https://github.com/d2r-mobile/d2r-mobile/blob/a8782caa/dm/gfx/vulkan/engine/xbg_nvn.cpp)、[nvn_vk.c](https://github.com/d2r-mobile/d2r-mobile/blob/a8782caa/dm/gfx/vulkan/nvn_vk.c)、[engine/xbg_vulkan.cpp](https://github.com/d2r-mobile/d2r-mobile/blob/a8782caa/dm/gfx/vulkan/engine/xbg_vulkan.cpp)、[INTEGRATION.md](https://github.com/d2r-mobile/d2r-mobile/blob/a8782caa/dm/gfx/vulkan/INTEGRATION.md)，以及 `engine/xbg_texture.c`、`engine/xbg_draw_plan.c`。tree 实测 88 条路径（背景"82 文件"应为不含 `engine/tests` 的口径）。

- **分层**：`nvn_vk.c`（13,033 B）是宿主侧薄适配层，照上游 `nvn_mtl.h`/`nvn_obj.h` 的 `mtl_*` ABI 实现（该文件出现 69 个 `nvn_*`/`mtl_*` 标识符），自身不发射 Vulkan；真回放在 `engine/`（15 个源文件，哈希见 `engine/SOURCE.json`，复制自鸿蒙 gfx50-claude-perf）。
- **枚举翻译**集中在 `xbg_nvn.cpp`（71 行）：`xbg_nvn_viewport`（Y 翻转 + 有限性校验）、`xbg_2d_sampler_from_nvn`（address mode 1/5/6/7，min filter→mipmap mode）、`xbg_2d_blend_from_nvn`（blend factor 1–11 与 0x61–0x64，VkBlendOp 1–5）、`xbg_2d_vertices_from_nvn`（NVN format 0x0a/0x16/0x22/0x2e/0x25 顶点解包）。
- **设备/管线**在 `xbg_vulkan.cpp`：扫 SPIR-V opcode 333–366 判定 subgroup 需求，61–66 映射 `VK_SUBGROUP_FEATURE_*`，设备创建时强制 `shaderDrawParameters`。
- **命令面**（同文件 `vkCmd*` 统计）：Draw/DrawIndexed、BindVertex/IndexBuffer、BindDescriptorSets、BindPipeline、Begin+EndRenderPass、PipelineBarrier×2、CopyBuffer↔Image（各 1 处）。**未见** DrawIndirect / MultiDraw / Dispatch / ConditionalRendering；compute 只出现在内置 kernel `kernels/builtin_queries.comp` 与 `xbg_bright_lut.comp`（其他文件是否有 Dispatch：UNVERIFIED）。
- **呈现/同步**：`INTEGRATION.md` 载离屏→swapchain blit、FIFO、按真 `VkFence` 判完成、`OP_SIGNAL_EVENT` 先等 GPU 再发 CPU 事件、禁异步翻译。注意该文自述 2026-10-15 仅链接通过、**未真机出图**，与"Adreno 730 上 60fps"口径冲突，应以 `docs/ANDROID_COLLABORATION` 为准。

## 2. BotW 1.6.0 的 NVN 面

- 资产卡 [expected-v1.6.0.json](https://github.com/ly05010419/MacNativeGames/blob/HEAD/games/botw/assets/expected-v1.6.0.json) 只有 2,348 B：`title_id 01007EF00011E000`、`version 786432`、exefs 5 模块、rewritten 3、romfs 3、aoc 2、shared_fonts 6。**没有任何图形能力清单**，所以 NVN 面只能从 exe 静态扫描得到；`assets/shader_list.tsv` 经 raw API 取回 0 字节（可能是 LFS 或路径差异），行类型 **UNVERIFIED**。
- `games/totk/third_party/d2r_gfx` 的 [README](https://github.com/ly05010419/MacNativeGames/blob/HEAD/games/totk/third_party/d2r_gfx/README.md) 自述：从 D2R commit `9f0c2576`（≠ a8782caa）复制 `gfx/nvn`+`gfx/tex`+bcdec，排除 D2R 的 shader/PSO TSV，未知 NVN 调用直接失败，"no game frame is verified"。
- 实测该目录 `totk_*` 为 **18 个模块 / 30 个文件**（背景说 26 个，口径不符）。按文件名推断（UNVERIFIED）：`totk_nvn_ext.c` = NVN 扩展入口，`totk_vpool` = 虚拟资源池≈bindless 描述符，`totk_pipeline_archive` = PSO 归档，`totk_taa(.m)` = 需 MRT+历史 RT，`totk_mfx*` = 后处理，`totk_shader_rt(.m)` = 运行时着色器，`totk_serial_set.h` = serial 兼容集。这些是 **TOTK** 需求，不能直接外推 BotW。
- 逐项判定：**几何/细分 UNVERIFIED**（8 个 stage 里无 geometry/tessellation 证据）；**MRT UNVERIFIED**（BotW 有 G-buffer 类 pass 属推断）；**conditional render UNVERIFIED**（NVN 有 `SetRenderEnable`，BotW 是否用未证）；**block-linear 需要**（Tegra 纹理一律 swizzle，见底稿 §4.3）；**compute UNVERIFIED**；**MultiDrawIndirect UNVERIFIED**；**bindless 弱证据**（`totk_vpool`）；**fence/present 需要**（`INTEGRATION.md` 已给实现路径）。

## 3. 缺口表

共 **14 项**：P0 3 项（条件渲染/遮挡、block-linear、MRT），P1 5 项，P2 6 项。

| 特性 | BotW 是否需要（证据） | d2r 是否覆盖 | Vulkan 对应物 | 工作量 |
|---|---|---|---|---|
| 条件渲染 | 需（NVN 有 SetRenderEnable；实际调用 UNVERIFIED） | 未见 | `vkCmdBeginConditionalRenderingEXT` | 3–5 天 |
| 遮挡查询 | 需（与上成对） | 部分（`engine/tests/occlusion_*`） | `vkCmdBeginQuery` + OCCLUSION | 3–5 天 |
| 间接绘制 | 需（UNVERIFIED） | 未见 | `vkCmdDrawIndexedIndirect(Count)` | 2–3 天 |
| MultiDrawIndirect | 需（UNVERIFIED） | 未见 | `VK_EXT_multi_draw` / 循环 indirect | 1 周 |
| Compute | 可能（UNVERIFIED） | 仅两个内置 kernel | `vkCmdDispatch` + SSBO | 3–5 天 |
| MRT 多目标 | 需（推断：TOTK TAA 要历史 RT） | 未见多 target | `pColorAttachments[]` + subpass | 3–4 天 |
| Block-linear 去 swizzle | 需（Tegra 纹理必 swizzle，底稿 §4.3） | 部分（`xbg_texture.c` 349 行） | 自写 deswizzle + `VkImage` | 1–2 周 |
| BC 压缩纹理 | 需 | 部分（`xbg_bc_policy.h`+bcdec） | `VK_FORMAT_BC*`+textureCompressionBC | 2–3 天 |
| Bindless/描述符索引 | 需（`totk_vpool` 弱证据） | 未见 | `VK_EXT_descriptor_indexing` | 1–2 周 |
| 几何着色器 | 低（UNVERIFIED） | 未见 | `geometryShader` stage | 1 周 |
| 细分 | 极低（UNVERIFIED） | 未见 | `tessellationShader` | 1–2 周 |
| Fence/present/窗口 | 需 | 已覆盖 | `VkSwapchainKHR`+`VkFence`+blit | 0.5–1 天（移植） |
| Serial 状态机 | 需（NSO 启动期校验版本） | 部分（宿主 `nvn_new_serial`） | 无对应，纯 CPU | 3–5 天 |
| PSO 缓存/异步编译 | 中 | 部分（`pipeline_memo.h`） | `VkPipelineCache` | 3–5 天 |

注：Adreno 725 低于 730，geometry/tessellation 与 subgroup 能力须在真机枚举 `VkPhysicalDeviceFeatures2`，不可照抄 730 假设。

## 4. fallback 分档

**A 档：可先跳过/降级**（画面或性能受损，但能启动出图）

1. 细分：BotW 基本不用，`xbg_*` 直接返回不支持，按基础网格画。
2. 几何着色器：改多 pass 或 CPU 展开，草/粒子密度下降。
3. Bindless：退回固定 descriptor set、每帧重绑，预估 -10~20% 帧率（降级路径必须先写，不要赌 `totk_vpool`）。
4. MultiDrawIndirect：拆成 N 次 `vkCmdDrawIndexed`，drawcall 从 1 变 N，CPU 成瓶颈。
5. Compute：若只用于后处理/亮度，先 CPU 近似或跳过（内置 kernel 已证明链路可用）。
6. 异步 PSO 编译改同步：卡顿但语义正确。
7. BC 不支持时解成 RGBA8：显存 ×4，Adreno 725 上大概率爆。
8. TAA/`totk_mfx` 类后处理：先关。

**B 档：必须实现**（否则黑屏/花屏/无输出）

1. Serial 状态机：NSO 启动即校验版本，失败直接 abort，绕不过。
2. Fence/present/窗口生命周期：`INTEGRATION.md` 的离屏→swapchain blit 路径可整段移植。
3. 基础 block-linear 去 swizzle：不对则全场景纹理花，没有降级空间。
4. 条件渲染 + 遮挡查询：BotW 大量 occlusion，顺序错会穿帮/漏画。
5. MRT：推断风险最高，建议**先静态扫 1.6.0 exe** 确认多 target 调用再排期。
6. `.xgs`/shader 二进制识别 → SPIR-V：没有它什么也画不出。

**排期建议**：第 1 周 serial+fence+纹理基础；第 2–3 周 条件渲染+MRT；第 4–6 周 间接/MultiDraw 与 bindless 降级路径；第 7 周起 compute/GS。注意 `INTEGRATION.md` 自述"未真机出图"，第一个里程碑应是"出第一帧"，而不是 60fps。

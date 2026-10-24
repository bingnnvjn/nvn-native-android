# 设备能力：Redmi Note 12 Turbo（Adreno 725）

> 实测来源：Hardware CapsViewer for Vulkan，报告 https://vulkan.gpuinfo.org/displayreport.php?id=43856
> 采集时间：2026-10-24。驱动版本会随系统更新变化，换版本要重采。

## 身份

| 项 | 值 |
|---|---|
| 设备 | Xiaomi 23049RAD8C（Redmi Note 12 Turbo） |
| GPU | Adreno (TM) 725，`0x7030002`，vendor `0x5143`（Qualcomm） |
| Vulkan API | **1.1.128**（不是 1.2/1.3） |
| 驱动 | `512.615.95` |
| pipelineCacheUUID | `39E31F0C-4351-0000-0000-020003070000` |

**第一条结论：只能按 Vulkan 1.1 core 设计。** 任何 1.2/1.3 的 core 特性（dynamic rendering、synchronization2 的 core 部分、maintenance4+）都不可依赖，只能用扩展。

## 关键 Features

| 能力 | 值 | 影响 |
|---|---|---|
| `geometryShader` | ✅ | BotW 若有 GS 变体，可走硬件 |
| `tessellationShader` | ✅ | 同上 |
| `multiViewport` | ❌（`maxViewports`=1） | 不能用多视口；VR/分屏类不可行 |
| `independentBlend` | ✅ | MRT 混合可行 |
| `textureCompressionBC` | ✅ | BC 纹理可直接采样（Adreno 少见） |
| `textureCompressionASTC_LDR` | ✅ | 首选 ASTC |
| `shaderInt64` / `shaderFloat64` | ❌ | 生成的 SPIR-V 不能含 64 位整数/浮点 |
| `sparseBinding` | ✅ | |
| `shaderDrawParameters` | ✅ | |
| `bufferDeviceAddress` | ✅ | NVN 的 GPU 地址可直接映射 |
| `occlusionQueryPrecise` / `inheritedQueries` | ✅ | 条件渲染 fallback 的基础 |
| `variableMultisampleRate` | ❌ | |
| `shaderStorageImageMultisample` | ❌ | |

## 关键 Limits

| 限制 | 值 |
|---|---|
| `maxColorAttachments` | 8 |
| `maxFramebufferWidth/Height` | 16384 / 16384 |
| `maxImageDimension2D` | 16384 |
| `maxPushConstantsSize` | 256 |
| `maxVertexInputAttributes/Bindings` | 32 / 32 |
| `maxViewports` | 1 |
| `maxMemoryAllocationCount` | 4096 |
| `maxDescriptorSetSampledImages` | 524288 |
| `maxPerStageDescriptorSamplers` | 524288 |
| `maxComputeSharedMemorySize` | 32768 |
| `timestampPeriod` | 52 |

## 扩展与 R2 缺口表对照

**有**（可直接用）：

- `VK_EXT_descriptor_indexing`、`VK_KHR_buffer_device_address`
- `VK_KHR_synchronization2`、`VK_KHR_timeline_semaphore`
- `VK_EXT_pipeline_creation_cache_control`（首帧卡顿的关键）
- `VK_KHR_pipeline_executable_properties`（可查驱动编译耗时）
- `VK_KHR_imageless_framebuffer`、`VK_KHR_create_renderpass2`、`VK_KHR_depth_stencil_resolve`
- `VK_ANDROID_external_memory_android_hardware_buffer`
- `VK_EXT_transform_feedback`、`VK_KHR_draw_indirect_count`
- `VK_EXT_extended_dynamic_state`（仅 1，无 2/3）
- `VK_QCOM_render_pass_transform`（旋转/分辨率优化）
- `VK_EXT_host_query_reset`（fallback 用）

**没有**（决定 fallback）：

- ❌ `VK_EXT_conditional_rendering` —— R2 的 P0 缺口，无法用扩展直译；必须用 occlusion query + 主机端判定（`VK_EXT_host_query_reset` 可用）或跳过该 draw。
- ❌ `VK_KHR_dynamic_rendering` —— 用传统 render pass + framebuffer（d2r 本来就是这条路）。
- ❌ `VK_EXT_memory_budget` —— 内存预算要自己估，或用 `VK_EXT_device_memory_report`（有）。
- ❌ `VK_KHR_maintenance4/5/6`、`VK_KHR_copy_commands2` 之外的新维护扩展 —— 按 1.1 子集写。

## 对架构的硬约束

1. **目标 API = Vulkan 1.1 + 上述扩展**。不要按 1.3 设计再回退。
2. **条件渲染必须自研 fallback**：occlusion query 结果回读 + 主机端决定是否提交；这条直接改变 #19 的引擎结构。
3. **多视口不可用**：引擎不应假设 `maxViewports > 1`。
4. **没有 int64/float64**：着色器翻译产物要过滤/降级。
5. **`multiViewport=false`、`maxViewports=1`** 写进能力探测，启动时校验。

## 复现

1. 安装 Hardware CapsViewer for Vulkan（`de.saschawillems.vulkancapsviewer`）。
2. 打开，提交报告，得到 `vulkan.gpuinfo.org/displayreport.php?id=…`。
3. 系统或驱动更新后重采，并在本文件记录新 id 与日期。

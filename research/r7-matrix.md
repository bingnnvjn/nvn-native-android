# R7 按层能力矩阵：新做基座的零件选型

> 状态：v1（2026-10-24 完成）。规模数字 = 当日 `gh api repos/<r>/git/trees/HEAD?recursive=1` 聚合；行数 = raw 文件 `wc -l`；星级/许可 = `api.github.com/repos/<r>`。不 clone、不 push。
> 本地事实来源：[switch-native-android-research.md](../switch-native-android-research.md)（2026-10-24，20 项核实）。本文不重复其结论，只做按层选型。

## 0. 总表与范围

**头条结论**：d2r 最被高估的不是"Vulkan 没在真机出图"——[README 有 Redmi K50 Ultra（Adreno 730）中位 60 fps、GPU 14–15 ms/帧](https://github.com/jhaizhou-ops/d2r-mobile/blob/HEAD/README.md)，这条是**反证**。它被高估的是"可当通用基座"：它是 D2R 单游戏的移植工厂，Android 壳 63 个文件，Vulkan 后端只在 Adreno 7xx 上被真机验证（同页 OnePlus 13 / Adreno 830 仍有黑方块闪烁）。

覆盖面（● 完整 / ◐ 部分 / ○ 无；L1–L7 见下文各节）：

| 项目 | 许可 | L1 容器 | L2 重写 | L3 host/SVC | L4 图形 | L5 着色器 | L6 壳 | L7 验证 |
|---|---|---|---|---|---|---|---|---|
| d2r-mobile（4,515★） | MIT | ◐ | ● | ● | ● VK | ◐ 预烤 | ● | ○ |
| nxnative（2,742★） | MIT | ◐ | ● | ● | ◐ Metal | ◐ 快照 | ○ iOS | ○ |
| MacNativeGames（5,072★） | MIT | ◐ | ● | ● | ◐ Metal | ◐ 34k 变体 | ○ iOS | ◐ 哈希表 |
| LASRecomp（1,723★） | MIT | ◐ | ◐ 主机 x86 | ◐ | ◐ | ○ | ○ | ◐ tools/ |
| LASRecomp-Android-Plus（132★） | MIT | ◐ 机内 | ◐ 机内编译 | ◐ | ◐ | ○ | ● | ○ |
| SuperMarioWonderRecomp（2★） | MIT | ◐ | ◐ | ◐ | ● 55 文件 | ● 预译 | ● | ○ |
| SavageWindRecomp（1,130★） | MIT | ◐ | ◐ | ◐ | ● | ◐ | ○ | ○ |
| ScarletNX（220★） | GPL-3.0 | ◐ | ● LLVM | ◐ | ◐ 命令缓冲 | ◐ | ● APK | ○ |
| AC-Recomp（10★） | MIT | ◐ | ◐ Python | ○ | ◐ MoltenVK | ○ | ○ | ○ |
| Atmosphère / mesosphère（20,197★） | GPL-2.0 | — | — | ● 语义 oracle | — | — | — | ◐ |
| nstool（554★） | MIT | ● | — | — | — | — | — | ◐ |
| SPIRV-Cross（2,535★） | Apache-2.0 | — | — | — | — | ◐ | — | ◐ |

**边界（不进矩阵）**：`oboromi` 是 Switch 2（SM86 stub，自述"跑不了任何商业游戏"，160 个服务是 `define_service!` 空壳）；`Skyline` 源码已被 Nintendo 要求从所有分支移除，只剩历史记录。二者只作为"另一代问题"的注脚。

## 1. 容器 / 提取层

**结论**：组件复用 **nstool**（MIT，读取+校验），提取流程自研（update/DLC 覆盖、title 索引）。top-2：`nstool`、LASRecomp-Android-Plus 的机内提取路径。

| 项目 | 规模（文件/关键行） | 覆盖了什么 | 许可 | Android 真机证据 | 已知缺陷 |
|---|---|---|---|---|---|
| [nstool](https://github.com/jakcron/nstool) | 59 文件 / 0.34 MB；`src/KeyBag.cpp` 70 KB | NSP/XCI/NCA/NSO 读+解 | MIT | 无（宿主工具，非设备侧） | 最后 push 2024-10-14；zstd NSB 等新格式 UNVERIFIED |
| d2r-mobile | `dm/tools` 42 文件；`nso.py` 14 行；`patch_nso.py` 1024 行 | NSO0 解码 + LZ4/zstd 片段 + 补丁产出 | MIT | 有：Android 版自带机内提取（README 架构表） | 只覆盖 D2R 用到的 NCA 子集；`nx_extract` 等价物 UNVERIFIED |
| nxnative | `dm/tools` 含 `nx_extract`（C#/LibHac） | 同上 + IPA 打包 | MIT | 无（iOS 侧） | 依赖的 **LibHac 已 404**，此路径今天不可复现 |
| LAS-Recomp-Android-Plus | 738 文件 / 18 MB | 机内解包（33 s 提取，需 12 GB 空闲） | MIT | 有：Galaxy S20 FE | 全部写在单一安装流程里，不是库 |
| [nxo64](https://codeberg.org/masagrator/nxo64) | 极小，Python | NSO/NRO 布局 | **无许可** | 无 | 无授权 = 只能读，不能依赖 |

**d2r 在这一层的弱点**：`nso.py` 仅 14 行，说明它把容器解析压到最小；NCA/keys 侧完全依赖外部 `prod.keys` 与自研 `nx_extract`，公开树里没有可复用接口。→ 由 `nstool`（读取/校验）+ 自研 overlay 逻辑补。

**组合后仍缺**：NCA section 级增量更新（base+update+DLC 叠加）、`title.keys` 缺失时的降级策略、以及"提取完做一次自检"的钩子（见 §7）。

## 2. 重写器层

**结论**：**重构复用** d2r / nxnative 的 `patch_nso.py`（两者同源，1024 vs 1152 行），但 Android/Bionic 的 TLS、`x18`、`svc` 门必须重写。top-2：d2r `patch_nso.py`、nxnative `patch_nso.py`。静态重编译器（LASRecomp / SavageWind / ScarletNX）在 arm64 主机上退化为重写器，不作为基座。

| 项目 | 规模（文件/关键行） | 覆盖了什么 | 许可 | Android 真机证据 | 已知缺陷 |
|---|---|---|---|---|---|
| d2r-mobile | `patch_nso.py` 1024 行；`dm/tools` 42 文件 | x18→线程槽、`tpidr_el0`/`tpidrro_el0`、`svc` 门表、sp 对齐、自旋让步、per-build-ID 断言补丁 | MIT | 有（间接）：Android 版跑的就是重写后镜像 | 补丁针对 D2R 的 build ID；Apple ABI 假设（x18 归属）来自 iOS |
| nxnative | 1,348 文件 / 130 MB；`patch_nso.py` 1152 行；`dm/games/<id>/patches.py` 每游戏一份 | 同上 + 53 个 title 的 per-title 补丁库 | MIT | 无（iOS） | per-title 补丁是主要工作量；无通用补丁推导 |
| [LASRecomp](https://github.com/peiikai/LASRecomp) | 679 文件；`tools/` 167 文件；`runtime/dispatch.c` 336 行 | 静态重编译（LLVM→主机 ISA）+ 每 build 地址表 `runtime/game_addresses/` | MIT（API 报 NOASSERTION） | 无（Linux aarch64 自述"构建但未上真机"） | 面向 x86-64 主机；arm64 上等于重写器 + 地址表 |
| [ScarletNX](https://github.com/scarletnx/ScarletNX) | 1,483 文件 / 17 MB；`src/common/cpu/recompiler/llvm/`；`src/gui/main.c` 621 KB | LLVM IR codegen，链 ELF/COFF/Mach-O；`docs/ANDROID.md` 明说"为所在设备编译" | **GPL-3.0** | 有 APK，设备未注明 → **未知** | 单文件 `gui/main.c` 621 KB；GPL 传染；音频/更新/DLC 未完成 |
| [SavageWindRecomp](https://github.com/Revivedx/SavageWindRecomp) | 430 文件 / 12 MB | LASRecomp 派生，BotW 1.8.2 | MIT | 无（Win/Linux） | 只到"大高原"之前，未验证 |
| [AC-Recomp](https://github.com/swaymun/AC-Recomp) | 312 文件 / 6.5 MB；Python 工具链 | 静态重编译 + Vulkan-on-MoltenVK | MIT | 无（iOS 已 paused） | 预览版，未通关 |

**d2r 在这一层的弱点**：重写器的输出契约（`fixups.tsv` / `gates.tsv` + 补丁区重定位）只有 D2R 一个消费者，±128 MB 分支距离处理是否覆盖大 `.text`（如 MHR 级 >112 MB）**UNVERIFIED**。→ 由 nxnative 的多 title 补丁库补覆盖面，但"通用补丁推导"没人做。

**组合后仍缺**：重写正确性的可判定标准——没有项目给出"重写后指令覆盖率 / 未识别指令清单"的自动检查（见 §7）。

## 3. host / SVC 层

**结论**：**重构复用**（对照 nxnative 的 `hle_*.c` 清单逐服务实现，自研可移植 C 层）。top-2：d2r `dm/host`（174 文件，唯一同时在 Android 与 Apple 上出货的 host）、nxnative `dm/host`（服务覆盖更全的检查表）。mesosphère 只作语义 oracle，不链接。

| 项目 | 规模（文件/关键行） | 覆盖了什么 | 许可 | Android 真机证据 | 已知缺陷 |
|---|---|---|---|---|---|
| d2r-mobile | `dm/host` 174 文件 | svc/线程/内存/句柄表、fs、hid、audout/audren、vi、oe、ro、nvdrv、swkbd | MIT | 有：Android 与 iOS 共用同一 host，只换图形层（README 原话） | per-title hack 侵入渲染器：`hle_oe.c` 在"本游戏渲染器内部偏移"改写缩放值，README 自述"换游戏就是野指针" |
| nxnative | `dm/host` 38 个 `hle_*.c`；1,348 文件 | 同上 + 53 title 的 sdk 补丁 | MIT | 无（iOS/iPadOS 26+） | 服务数量多但同样 per-title；条件渲染恒绘制、`MultiDraw*IndirectCount` 走 CPU 等待 |
| LAS-Android-Plus | 738 文件 | 自研 HLE + 机内编译 | MIT | 有：S20 FE，60 fps（30 原生 + 1 生成帧） | 深度绑定单个游戏（Link's Awakening）的地址表 |
| ScarletNX | `src/nx/hle/`（1483 文件中） | 通用 HLE + LLVM 重编译器 | GPL-3.0 | APK 存在，设备未注明 → 未知 | 音频缺失（Catherine 记录："60 FPS, no audio"） |
| [Atmosphère / mesosphère](https://github.com/Atmosphere-NX/Atmosphere) | 3,984 文件；`libraries/libmesosphere/` 343 路径 | Horizon 微内核语义：地址仲裁、条件变量、svc、句柄表、loader 规则 | GPL-2.0 | 不适用（跑在真机固件上） | 只能读不能链；语义版本随固件漂移 |
| oboromi（边界） | Rust，`core/src/nn/mod.rs` 160 个 `define_service!` | 只有服务**名字**清单 | GPL-3.0 | 无（跑不了游戏） | 空壳 stub，`auto.defs` 为 0 字节 |

**d2r 在这一层的弱点**：`hle_oe.c` 这类"改游戏内部偏移"的补丁是硬编码的，host 接口没有为多 title 设计（无 per-title 策略注入点）。→ 由 nxnative 的 `dm/games/<id>/patches.py` 结构补"策略外置"，但语义仍要人写。

**组合后仍缺**：Android 侧的音频后端映射（audout/audren → AAudio/Oboe）没有可抄的实现；`hid` 的陀螺仪/震动映射同样缺（各项目只覆盖按键）。UNVERIFIED 是否需要 amiibo/NFC（nxnative 明确"No amiibo"）。

## 4. 图形引擎层

**结论**：**重构复用**（把 d2r 的 `dm/gfx/vulkan` 当参考实现读，接口自定）。top-2：d2r `dm/gfx/vulkan`（82 文件，唯一在 Android+Vulkan 上出货的 NVN 后端）、SuperMarioWonderRecomp `runtime/src/nvn/`（55 文件，第二份独立的 Android 实现）。

| 项目 | 规模（文件/关键行） | 覆盖了什么 | 许可 | Android 真机证据 | 已知缺陷 |
|---|---|---|---|---|---|
| d2r-mobile | `dm/gfx/vulkan` 82 文件；`dm/gfx/nvn` 36 文件 | NVN 调用 → Vulkan：内存池零拷贝、record-then-replay 命令缓冲、state→pipeline、block-linear 反 swizzle、present/crop、fence/event | MIT | **有**：K50 Ultra / Adreno 730 "median 60 fps, GPU 14–15 ms/frame"（README 第 203 行） | 同页第 204 行：OnePlus 13 / Adreno 830 "偶尔闪黑方块"（在查）；只覆盖 D2R 用到的 NVN 子集 |
| SuperMarioWonderRecomp | 318 文件；`runtime/src/nvn/` 55 文件（`nvn_cmd.cpp`/`nvn_astc.cpp`/`nvn_bc.cpp`…） | NVN → 原生 Vulkan 1.3 | MIT | **有**：Galaxy S23（README 第 40 行） | 单 title；资产主要由 GIF 演示（77 MB），测试面窄 |
| LAS-Android-Plus | `runtime/nvn/` + `third_party/` | NVN → Vulkan，机内编译 | MIT | **有**：S20 FE，~24.5 ms GPU/游戏帧 @1x | 需 12 GB 空闲；Turnip 驱动崩溃曾需修 |
| SavageWindRecomp | `runtime/nvn/` | NVN → Vulkan（Win/Linux） | MIT | 无（Windows/Linux） | 未上 Android；图形层无独立验证 |
| nxnative | `dm/gfx`（Metal）+ `nvn_metal_mapping.md` | 对象级语义表（最完整的书面规格） | MIT | 无（Metal） | 几何着色器不翻译；TBDR 优化默认关；bindless 未理解 |
| ScarletNX | `src/nx/gpu/`，Maxwell 命令缓冲解码 | 通用 GPU 层（不做 NVN→VK 翻译，直接解命令缓冲） | GPL-3.0 | APK 存在，设备未知 | 需 Vulkan 1.3；进度只到 Catherine 新游戏过场 |
| [deko3d](https://github.com/devkitPro/deko3d) | 74 文件 / 0.5 MB | 纹理布局尺寸算法（nxnative `nvn_tex.c` 移植自它） | zlib（UNVERIFIED，仓库未见 SPDX） | 无 | 是自研 Switch 端 API，不是翻译层 |

**d2r 在这一层的弱点**：Vulkan 后端只在一个 Adreno 世代上被真机证明；`vk` 层与 D2R 的 NVN 调用集绑定，缺少"能力探测 → 降级路径"（Vulkan 1.1 vs 1.3 的分叉在 §9）。→ 由 SMWR（Vulkan 1.3 / 3D 标题）与 LAS-Android（Adreno 650 + Turnip）补交叉机型。

**组合后仍缺**：Mali/Xclipse 上的真机证据为零（LAS-Android 原话是"Adreno 才是有构建和测试的"）；BCn/ASTC 在无 BC 采样设备上的解码回退；条件渲染 / `MultiDrawIndirectCount` 的正确实现（nxnative 自述恒绘制）。

## 5. 着色器翻译层

**结论**：**组件复用** Ryujinx.Graphics.Shader 的冻结快照（MIT，vendored），配自研的变体枚举与离线缓存。top-2：Ryujinx.Graphics.Shader 快照、yuzu Hades（仅在接受 GPL-3.0 时）。SPIRV-Cross 只做 reflection / GLSL 回退，**不是** Maxwell 解码器。

| 项目 | 规模（文件/关键行） | 覆盖了什么 | 许可 | Android 真机证据 | 已知缺陷 |
|---|---|---|---|---|---|
| Ryujinx.Graphics.Shader（快照） | nxnative 内 `dm/third_party/ryujinx/` 363 文件，保留 `LICENSE.txt` | Maxwell SPH+控制段 → SPIR-V，含变体特化 | MIT（原仓库 **404**，镜像 **451 DMCA**） | 间接：SMWR 用它预译全部 12,717 块，在 S23 跑通 | 无上游、无修复；公开 APK 的 takedown 风险未量化 |
| yuzu Hades `shader_recompiler` | suyu 归档内 `src/shader_recompiler/` | Maxwell → SPIR-V/GLSL | GPL-3.0 | 无独立证据 | 遗产线已被 DMCA 扫过（2026-08）；Skyline 当年靠特批豁免 |
| [shader-compiler-rs](https://github.com/DCNick3/shader-compiler-rs) | 2,966 文件 / 22 MB（多数 vendored）；CLI 薄封装 | NVN 0x30 字节前缀 + SPH → GLSL | MPL-2.0 | 无 | 最后 push 2025-06-22；只是 Hades 的壳 |
| [SPIRV-Cross](https://github.com/KhronosGroup/SPIRV-Cross) | 4,871 文件；`spirv_msl.cpp` 760 KB、`spirv_glsl.cpp` 689 KB | SPIR-V → MSL/GLSL/HLSL + reflection | Apache-2.0 | 无（不产出 SPIR-V） | MSL 半边对 Android 无用 |
| d2r-mobile | `dm/gfx/nvn/shader_map.tsv` 1.27 MB、`shader_facts.tsv` 1.1 MB | **预烤**的 NVN shader 哈希 → SPIR-V 索引 | MIT | 有：随 APK 出货 | 只有已收录变体；新变体 = 跳过 draw |
| MacNativeGames | `games/totk/assets/shader_list.tsv` 42.8 MB；34,179 变体 / 28,254 shader | 变体语料（最大公开样本） | MIT | 无（Metal） | 语料与 title 绑定；"51,075 变体"是错的 |
| SuperMarioWonderRecomp | 12,717 块全部 AOT | 离线全量预译 | MIT | 有：S23 | 只有单 title |

**d2r 在这一层的弱点**：没有运行时翻译器——`shader_map.tsv` 是静态清单（1.27 MB），未收录的变体直接丢 draw。→ 由 Ryujinx 快照（可在设备/构建期翻译）补，但手机不能承受运行时翻译（nxnative 原话："iPhone/iPad 不会翻译，缺变体就跳过 draw"）。

**组合后仍缺**：变体**枚举**（怎么在构建期知道该翻哪些）没有任何项目开源；几何着色器翻译（nxnative 明确失败，62/63 不构建）；以及"翻译失败 → 回退"的运行时策略。

## 6. 平台壳层

**结论**：**组件复用**（抄结构，不抄构建系统）。top-2：d2r `dm/android`（63 文件，与 Apple 壳并存的第二套平台层）、LASRecomp-Android-Plus `android/app`（"自带可执行 `.so` + GameActivity + 前台通知"的完整解法）。

| 项目 | 规模 | 覆盖了什么 | 许可 | Android 真机证据 | 已知缺陷 |
|---|---|---|---|---|---|
| d2r-mobile | `dm/android` 63 文件 | 窗口/VkSurface、触摸覆盖、生命周期、存储；最低 Android 12 / Vulkan 1.1 | MIT | 有：K50 Ultra、OnePlus 13 | 与自家 CMake/SDL 版本耦合；无通用外设抽象 |
| LAS-Android-Plus | `android/app` 18 个 Gradle 侧文件 | GameActivity、前台服务、机内编译、Turnip 加载 | MIT | 有：S20 FE（含触控截图） | 依赖它的机内 clang 流程，不是独立模块 |
| [ScarletNX](https://github.com/scarletnx/ScarletNX) | `docs/ANDROID.md` + nightly APK | 通用 APK 分发、arm64/x86_64 双 ABI | GPL-3.0 | APK 存在，**设备未注明** | "需要 Vulkan 1.3"把中端机排除 |
| SuperMarioWonderRecomp | 平台层最小集 | Android（arm64）分支，SDL 风格 | MIT | 有：Galaxy S23 | 只服务单 title |

**d2r 在这一层的弱点**：壳层薄且绑定自家构建；没有"设备能力探测 → 分辨率/后端降级"的产品逻辑。→ LAS-Android 补工程化，ScarletNX 补分发。

**组合后仍缺**：Mali/Xclipse 的驱动加载与特性探测；手柄/陀螺仪/震动统一映射；后台被杀的存档保护。

## 7. 验证工具层

**结论**：**自研**（这一层是全表最大空白）。top-2 只能算"半个"：nstool（容器/哈希校验）、MacNativeGames 的 `expected-v*.json`（版本级模块哈希基线）。没有任何项目给出可在真机上跑的自动回归。

| 项目 | 规模/形态 | 覆盖了什么 | 许可 | Android 真机证据 | 已知缺陷 |
|---|---|---|---|---|---|
| [nstool](https://github.com/jakcron/nstool) | 59 文件 | NCA/NSO 哈希与结构校验 | MIT | 无 | 只验容器，不验重写结果 |
| MacNativeGames | `games/totk/assets/expected-v*.json` 4 份 | 按游戏版本的模块哈希基线（"发布前比对"） | MIT | 无 | 与 title 版本绑定，非通用框架 |
| d2r-mobile | `shader_facts.tsv` 1.1 MB | shader 事实表（哈希→属性），可用于比对 | MIT | 间接 | 是数据不是测试 |
| LASRecomp | `tools/` 167 文件 | 分析/反汇编辅助 | MIT | 无 | 无断言、无 CI 证据 |
| shader-compiler-rs / SPIRV-Cross | CLI + reflection | 单条 shader 的翻译对拍 | MPL-2.0 / Apache-2.0 | 无 | 不覆盖 pipeline 组合 |

**d2r 在这一层的弱点**：树里未见测试入口/CI 配置（**UNVERIFIED**，未逐目录核对）；它的质量保证看起来是"真机试玩"。

**组合后仍缺**：三层验证缺两层——(a) 重写后指令覆盖率与未识别指令清单；(b) `svc`/HLE 语义一致性测试（mesosphère 可当 oracle，但没人写用例）；(c) 真机回归（帧时间分布、驱动错误码、画面哈希）。这三项没有现成零件。

## 8. d2r 弱点清单与补位

| # | 弱点 | 证据 | 由谁补 | 补完还剩 |
|---|---|---|---|---|
| 1 | 单游戏专用耦合 | `hle_oe.c` 在"本游戏渲染器内部偏移"改写缩放值，作者自述换游戏即野指针 | nxnative `dm/games/<id>/patches.py` 的"策略外置"结构 | 策略本身仍要人写，没有推导 |
| 2 | Vulkan 后端窄验证 | 同一 README：Adreno 730 60 fps，Adreno 830 闪黑方块 | SMWR（S23）、LAS-Android（Adreno 650 + Turnip）交叉机型 | Mali/Xclipse 仍无证据 |
| 3 | 无运行时着色器翻译 | 只有 `shader_map.tsv`（1.27 MB）静态清单，缺变体即跳过 draw | Ryujinx 快照 + SMWR 的全量预译流程 | 变体枚举仍无人开源 |
| 4 | "单文件 467 KB" | **未复现**：`gh trees` 里 d2r 最大的 5 个文件是 `shader_map.tsv` 1.33 MB、`shader_facts.tsv` 1.13 MB 和 3 个 PNG；源码无此量级 | —（该指控不成立；真·大单文件在 ScarletNX：`src/gui/main.c` 621 KB） | 规模问题是资产不是源码 |
| 5 | "14 缺口" | **UNVERIFIED**：本地核实报告没有这 14 项清单；可对照 nxnative 自述的 6 条 gaps | 需要重新核对来源 | 清单口径不明 |
| 6 | Android 壳薄（63 文件） | 见 §6 | LAS-Android 的 `android/app` 工程化 | 设备矩阵仍缺 |
| 7 | 无自动验证 | 见 §7 | 自研 | 全部 |

**回到"最被高估"**：不是"Vulkan 未真机出图"（有 K50 Ultra 数据反证），而是**可移植性**——host 与图形层都为 D2R 的 NVN 子集剪裁过，换 title 就要重写 `hle_*.c` 的补丁部分。

## 9. 组合之后仍缺什么

1. **通用（非 per-title）路径**：所有项目都是"移植工厂"；要么做 ScarletNX 那种 Maxwell 命令缓冲解码器，要么接受每 title 一个团队。没有第三条。
2. **shader 变体枚举与预算**：34k 变体（TotK）、12.7k 块（SMRPG 级）说明离线翻译可行，但"该翻哪些"没有工具。
3. **Vulkan 1.1 特性回退矩阵**：d2r 只要 1.1、ScarletNX 要 1.3；中间的 BCn/ASTC、geometry、bindless、conditional render 缺一张按 GPU 的可用性表。
4. **svc/HLE 一致性测试**：mesosphère 是 oracle，但没人把它变成用例集。
5. **音频与输入后端**：audout/audren → AAudio/Oboe、hid → Android 输入，无现成实现。
6. **合法分发**：全部依赖用户自备 dump + `prod.keys`；上游仓库会被下架（Ryujinx 451、LibHac 404、Skyline 清空），任何组件都必须 vendor + 锁哈希。
7. **Switch 2 / NVN2**：无公开规格，oboromi 只是 stub —— 明确不在本轮范围。

| 层 | 推荐（类型） | 基座 |
|---|---|---|
| L1 容器/提取 | 组件复用 | nstool + 自研 overlay |
| L2 重写器 | 重构复用 | d2r/nxnative `patch_nso.py` |
| L3 host/SVC | 重构复用 | d2r `dm/host`（对照 nxnative 清单） |
| L4 图形引擎 | 重构复用 | d2r `dm/gfx/vulkan` + SMWR `runtime/src/nvn` |
| L5 着色器 | 组件复用 | Ryujinx.Graphics.Shader 快照（+ SPIRV-Cross 回退） |
| L6 平台壳 | 组件复用 | LAS-Android `android/app` / d2r `dm/android` |
| L7 验证工具 | 自研 | 无基座 |

# R4：BotW 着色器变体的构建期翻译与预热（Android 原生）

> 范围：3,728 条 BotW 1.6.0 shader 变体如何在构建期翻译为 SPIR-V/Vulkan，运行时只做缓存查找。
> 状态：调研草稿，未验证项标 UNVERIFIED。

## 1. 翻译器选型：Ryujinx.Graphics.Shader（MIT）vs yuzu Hades（GPL）vs 自研

**结论：唯一选 Ryujinx.Graphics.Shader 快照（MIT）。** 不引入 yuzu Hades，不自研。

证据（均在 `d2r-mobile/d2r-mobile@dev`，仓库许可 MIT：https://github.com/d2r-mobile/d2r-mobile ）：

- 快照共 **363 个文件**，全部在 `dm/third_party/ryujinx/`，自带 `LICENSE.txt`（MIT）；与 GPL-3.0 的 yuzu Hades 无授权冲突，可直接并入我们的 MIT 仓。路径：`https://github.com/d2r-mobile/d2r-mobile/tree/dev/dm/third_party/ryujinx`。
- **指令覆盖**：`Decoders/InstName.cs` 有 **180 条 Maxwell mnemonic**（含控制码/属性/纹理/内存/`warp` 类），`Decoders/InstTable.cs` 是 14-bit 编码的查发表；`Instructions/` 下 **30 个 `InstEmit*.cs`** 发射器，按类分为 `InstEmitAluHelper`、`InstEmitFloatArithmetic`、`InstEmitIntegerArithmetic`、`InstEmitBitfield`、`InstEmitTexture`、`InstEmitSurface`、`InstEmitMemory`、`InstEmitAttribute`、`InstEmitWarp`、`InstEmitFlowControl`、`InstEmitPredicate` 等 → ALU/浮点/位域/纹理/SSBO/属性加载这些 BotW 主路径都有实现。
- **双后端**：`CodeGen/Glsl/GlslGenerator.cs`（GLSL）与 `CodeGen/Spirv/SpirvGenerator.cs`（SPIR-V），Android 走 Vulkan 只需 SPIR-V 分支，不用自己写后端。SPIR-V 生成器依赖 `src/Spv.Generator`（同仓，MIT）。
- **同血脉已做过离线批量翻译**：`games/totk/tools/totk_xlate/`（C#，vendored Ryujinx 1.3.2，MIT；`Program.cs` + `Service.cs` + `spirv_cross_inproc.cpp` + `patches/`），https://github.com/ly05010419/MacNativeGames/tree/main/games/totk/tools/totk_xlate 。我们可复用其 CLI/批处理外壳，换 BotW 的容器解析。

**能否覆盖 BotW 3,728 变体：主路径能，三点必须实测（UNVERIFIED）：**
1. 几何/细分：快照的发射器表里没有独立的 geometry emitter，NVN 是允许 GS 的；BotW 是否有 GS 变体需扫 `shader_list.tsv` 的 deps/flag 确认。
2. TotK 侧的 `VertexCounterLowering.cs` 修正是否已进 d2r 快照 —— 若没有，顶点计数类变体要单独打 patch。
3. 纹理格式与 border color 补丁（B 行语义）在 Vulkan 侧是否有等价实现（Metal 专有的 border 模拟不能照搬）。

排除 Hades：GPL-3.0 传染，只能靠独立进程 + CLI 边界隔离，工程量与法律审阅成本都高于收益。自研：180 条指令 + 结构化 IR + 双后端是从零重写，**推断**为 5–10 人年，不划算。

## 2. 变体枚举与构建期预热接入

**输入文件**：`games/botw/assets/shader_list.tsv`，8,338,230 字节，blob `14741d12e2e3…`（`gh api repos/ly05010419/MacNativeGames/contents/games/botw/assets/shader_list.tsv`）。

**行 → 编译任务**（行格式依据 `/home/myworkspace/DeepSeekharness/research/shader_list.md`）：

| 行 | 构建期动作 |
|---|---|
| `V` shader key + deps | 定变体。key = NVN control 段 + code 的 SHA-256，与运行时同法算出；deps 是 texture/image/attribute/fragment-output 类型集合，运行时按 deps 匹配而非名字。 |
| `L` key + 位置 + 偏移 | 非常规位置（RomFS 路径或 `module:<name>` + hex 偏移）。构建期从**玩家自己的游戏文件**取字节，再校验哈希等于 key，否则丢弃 —— 即 list 只存标识符，永不携带游戏数据。 |
| `S` `<variant>_b` | 载入 shader library（对应 `vkCreateShaderModule` 前的字节块）。 |
| `C` / `R` | compute / render pipeline 描述，构建期只记录 flag，不预编译 pipeline。 |
| `B` | R + 顶点/片元 border color 补丁（`-` 表示该 stage 无），预热时构建打过补丁的 shader 与 pipeline。 |

CI 流水线：① 解析 BNSH / `agl` 的 `.sharcb` / 游戏 module → 抽 control+code；② 哈希匹配 `V`/`L` 行；③ 调 `totk_xlate`（Ryujinx）产出 SPIR-V；④ 校验；⑤ 写 `build/pso_title.tsv`（`t` 行）与 `build/pso_manifest.tsv`（`m` 行），**不手改这两个文件**。

**无 GPU 的 ubuntu runner 够用吗：能校验，不能验收。**
- `spirv-val`（SPIRV-Tools，纯 CPU，https://github.com/KhronosGroup/SPIRV-Tools ）：模块结构/ID/类型规则校验，无需 Vulkan 驱动。
- `glslangValidator`（https://github.com/KhronosGroup/glslang ）：校验 GLSL 侧与内嵌 GLSL。
- `spirv-cross`：反向吐 GLSL 做二道对比，`totk_xlate` 已以 `spirv_cross_inproc.cpp` 内嵌（同目录）。
- 三条都跑在 CPU 上，**不需要 GPU/驱动**。但「校验通过 ≠ 能建 pipeline」：border color、format support、descriptor 上限只有真机 `vkCreateGraphicsPipelines` 才暴露 → runner 通过后仍需一台 Adreno 真机做冒烟。

**CI 缓存键**（建议）：
`sha256( shader_list.tsv blob sha ‖ 翻译器源码 tree sha（third_party/ryujinx + totk_xlate） ‖ spirv-tools 版本 ‖ glslang 版本 ‖ 目标 VK_API_VERSION ‖ B 行补丁表 hash )`
流水线产物（SPIR-V + `pso_*.tsv`）按该键整体缓存，键不变即跳过整段。翻译器源码用 tree sha 而非 commit sha，避免无关提交击穿缓存。

## 3. 运行时缓存：格式、mmap、查找与冷启动

**格式**：单文件 blob + 尾部索引（可整块 mmap 一次读入）。
- 条目：`{ key: 32B sha256, stage: u8, flags: u8, spirv_off: u32, spirv_len: u32, deps_off: u32, deps_len: u32 }`
- 索引：按 key 排序的固定宽度数组，`binary search` 定偏移；deps 用紧凑 JSON（与 list 的 deps 同格式）放在同一 blob 的 data 区，命中后再做一次 deps 匹配确认真是同一变体。
- 对齐：SPIR-V 要求 `codeSize` 为 4 的倍数、`code` 指针对齐 4 字节 → blob 内每个 spirv 段 padding 到 4。

**mmap**：`open` + `mmap(PROT_READ, MAP_PRIVATE)`，`vkCreateShaderModule` 直接用映射指针（codeSize/code），不拷贝、不解析；页面由内核按需换入，冷启动只触到被 draw 用到的页。

**按 key 查找**：运行时算出变体 key（NVN control+code 哈希，与构建期同法）→ 索引二分 → 命中则建/取 pipeline。**未命中按 nxnative 策略直接跳过该 draw**（手机端无翻译回退），因此命中率就是画面完整度，这是本方案的核心指标。

**冷启动**：首帧只等 `t` 行（title / 菜单 / 存档列表），`m` 行交给后台线程预热；`shader-keys.json`（游戏文件里实际存在的 key 全集）用来缩小预热范围，避免把别的游戏版本的条目一起热起来。

**能否进 APK：法律上先按「不进公共 APK」记。** 理由：SPIR-V 是从游戏资产派生的内容，公共 APK 分发等于分发派生资产。做法与现有 list 设计一致——APK 只带 `shader_list.tsv`（纯标识符），首次运行由用户在设备上用自己的游戏文件生成缓存；这与 `research/shader_list.md`「the list holds identifiers only, never game data」的约束同源。若日后要预置，走「独立可下载资产包 + 用户自备游戏文件校验」的路径，需法务单独确认（UNVERIFIED）。

## 4. 预热时长估算：M4 90s / 15,266 → Adreno 725 / 3,728

**基准**：M4 上 15,266 个变体约 90 s（15,266 prepared 来自 TotK 侧 README，见 `/home/myworkspace/DeepSeekharness/research/switch-native-android-research.md` §3.1）；折合 **5.9 ms/变体**，且这 90 s 是 Mac 上「解析 + 翻译 + 建 pipeline（+ 可能含 Metal PSO 序列化）」的混合口径，**统计边界 UNVERIFIED**。

**可检验的估算式**：

```
T_warm(N) ≈ N × ( c_parse + c_xlate + c_pipe ) + C_start
c_xlate ≈ 5.9 ms × r_cpu        r_cpu = 单变体 CPU 翻译成本比（M4 → 目标 SoC 单核）
c_pipe  ≈ Adreno 驱动编译每个新 SPIR-V 的固定开销（未命中 pipeline cache 时 1–10 ms）
C_start ≈ 缓存 mmap + 索引装载（一次性，< 100 ms）
```

取值代入（3,728 个，全部假定需建 pipeline）：
- **乐观**（驱动命中自身 pipeline cache，只算 CPU 段）：`3,728 × 5.9 × 1.5 ≈ 33 s`
- **中位**（每变体 +5 ms 驱动编译）：`3,728 × (8.9 + 5) ≈ 52 s`
- **悲观**（+10 ms，且无 pipeline cache）：`3,728 × 18.9 ≈ 70 s`

**结论**：Adreno 725 上 3,728 个预期落在 **35–70 s**，与 M4 的 90 s 同量级（数量少 4 倍但单件更贵）。真正要压的不是总时长，而是**首帧等待**：只让 `t` 行阻塞首帧，建议 `t` 子集控制在 ≤ 800 个（≈ 7–15 s），其余后台预热。

**风险**：
1. `r_cpu` 无法先验确定：M4 与 Adreno 725 的 GPU 驱动编译模型完全不同，5.9 ms 里 CPU/驱动占比未知 → 上式只能定界，不能定点（UNVERIFIED）。
2. Adreno 驱动更新会作废设备侧 pipeline cache，最坏情况每次升级都落到悲观界。
3. 若 BotW 变体含 GS/特殊纹理格式，走慢路径，单件成本上浮。
4. 后台预热与游戏抢 CPU/带宽，实测需同时看帧时间 P99，不能只看总时长。
5. M4 的 90 s 若含 Metal 专属的 PSO 二进制序列化，则 Vulkan 侧口径不同，公式需按实测重标定（**第一步应先测 200 个变体的单件成本**，再线性外推）。

# 架构

> 这是一个新项目，不是现有项目的拼装。
> 接口、骨架、模块边界由我们自己定；现有项目是**依赖、参考实现与零件库**，不是基座。
> 本版（2026-10-30）定稿 **L0–L6 七层**与 **C ABI v1**。层号只用这一套：
> 验证是横切列（不是层），驱动/硬件是第三方（不进表）。旧文里的 L1–L7 一律按层名读。

## 1. 完全体由两半拼成

```
APK（CI 编译，无游戏数据，可公开）
        +
import pack（本机用你的 ROM 生成，不可公开）
        =
        可在手机上玩
```

- **APK**：我们的程序。平台壳、host、图形引擎、加载器、着色器查表器都在里面。
- **import pack**：从你自己的 XCI/NSP 解出的游戏代码、RomFS、着色器表、着色器缓存。
- **keys**：只在电脑上生成 import pack 时用一次，**不上手机、不进仓库**。
- **游戏本体代码**：属于 import pack，不属于 APK。

## 2. 运行时：一帧是怎么出来的

```
┌──────────────────────────┐
│ BotW 原版代码（ARM64）    │
│ 经 L1 重写器改写后的那份  │
└───┬──────────────────┬───┘
    │ SVC（svc-gate）    │ NVN（thunk）
┌───▼─────────┐   ┌────▼──────────┐
│ L4 host     │   │ L5 图形引擎    │
│ 系统服务替身 │   │ NVN → Vulkan   │
└───┬─────────┘   └────┬──────────┘
    │ 音频/输入/日志     │ Vulkan
┌───▼──────────────────▼──────────┐
│ L6 平台壳 + Android              │
│ 窗口 / 音频 / 输入 / 驱动        │
└─────────────────────────────────┘
```

游戏代码在中间：向左要系统服务，向右要画面。左右两边都由我们实现。
L3 加载器把代码装进内存并跳 entry；L0/L2 是构建期，不在手机上跑。

## 3. L0–L6 可执行表

编号 = 构建与启动链上的顺序。每行都必须能被执行者直接照做；细节在 3.1–3.7。

| 层 | 职责 | 来源（组件/重构/自研） | 自研部分 | Android 接口 | 验证方式 | 层间契约 |
|---|---|---|---|---|---|---|
| **L0 容器/提取** | XCI/NSP + keys → import pack；不重写、不加载 | 组件 `nstool@cfb826ca`；overlay/清单自研 | `nva-pack` CLI、title index、NCA section 合并、自检 | 无（构建期 PC） | 合成 fixture；`nstool -y`；对拍 `expected-v1.6.0.json` | 出 `manifest.json`/`load-plan.json`/`raw/`/`romfs.bin`；`pack_id` 只追加 |
| **L1 重写器** | NSO0 → 同布局 Android/Bionic 镜像 + manifest | 重构 d2r/nxnative `patch_nso.py`；Rust 重写 | `svc-gate`、SP 16 对齐、veneer、覆盖率、BotW 补丁 | 无（构建期 PC） | `nva-verify rewrite`：100% 归类 + expected hash | 入 NSO0 + 规则 + `svc_gates.json` + `tls.json`；门号 host 发 |
| **L2 着色器** | Maxwell→SPIR-V 全在构建期；运行时只查表 | 组件 Ryujinx 快照@`a8782caa` | 变体枚举、cache、查找、deps 派生、预热、补丁 | 无（构建期；运行时 `mmap`） | `spirv-val`/`glslang`；确定性；命中率 | `shader-cache.bin` + index；key = SHA-256(control‖code) |
| **L3 加载器** | 读 pack、映射代码、设 TLS/栈、跳 entry | 参考 MacNativeGames/nxnative `loader.c`；自研 | image map、W^X/memfd/16KB、validator | `mmap`/`mprotect`/`memfd`/`pthread` | PC validator；arm64 CI entry；真机断言 | 顺序 host→gfx→…→entry；不导出 C header |
| **L4 host/SVC** | SVC、线程/内存/句柄、18 面服务 | 重构 d2r 通用 51 blobs；nxnative 补 14 | per-title policy、audio/fs/hid、事务存档、nvdrv | 经壳：AAudio/输入/日志 | `nva-verify hle`；mesosphère 对拍；设备 smoke | engine→host `nva_host_services`；host→壳 `nva_platform` |
| **L5 图形引擎** | 523 表、记录/重放、GPU 内存、surface/present | 参考 d2r `dm/gfx`/SMWR；自研 | dispatch、pipeline key、8 MRT、去 swizzle、fence | Vulkan、`ANativeWindow` | spike 9/9、10/10；8 MRT ≤28 ms | `nva_gfx` 控制面；NVN 表借用；`GATE_TLS` |
| **L6 平台壳** | 窗口/输入/音频/权限/SAF 导入/生命周期 | 参考 d2r `dm/android`/LAS；自研薄壳 | surface generation、AAudio、SAF、FGS、探测 | `Activity`+`SurfaceView`/`AAudio`/SAF | 真机 smoke；CI `p_align`/JNI/manifest | `nva_app` + `nva_platform`；不碰 Vulkan |

### 3.1 L0 容器/提取

- **落位**：组件 `jakcron/nstool@cfb826ca`（MIT，v1.9.2；本机 `tools/nstool`，sha256 `b77319d6…`）+ 自研 overlay/manifest/self-check。nstool 以**子进程**调用，钉 commit + sha256，不 vendor 源码。
- **输入**：XCI 或 NSP（可多个）+ `prod.keys`（+ 可选 `title.keys`）。keys 只传路径；manifest 只记 `generation/key_count`，不记字节、哈希或路径；用 0600 临时目录；禁用 `--showkeys`。
- **overlay**：title index 从 CNMT 取 title/version/content；base < update < DLC 只决定选择优先级；NCA **section 级**合并，ExeFS 按文件名、RomFS 按路径，update 胜、base 独有保留、同层重复即错。未知 FsType/删除标记硬报错，不猜。
- **别用错开关**：XCI 的 `--update` 是**系统更新分区**，不是游戏更新；游戏 update 来自 secure PFS0 的 NCA 或单独 update NSP。
- **产物**：`manifest.json`（`pack_id`、title/version/build_id、每文件 SHA-256、RomFS 树 hash、DLC、selfcheck）、`load-plan.json`（段表）、`raw/exefs/*`、`romfs.bin` + `romfs.index.json`、`aoc/N/`。RomFS 用单 blob + index（41,052 文件 / 15.1 GB，手机复制/校验/mmap 更稳）；`--tree` 只作调试。
- **验证**：CI 用合成 fixture 测 overlay/manifest/负例；本机 `nstool -k prod.keys -y` 逐 NCA，`nva-pack verify` 对拍 `expected-v1.6.0.json`（ExeFS 5 模块、rewritten 3、RomFS 41,052 文件 / 15,131,979,952 B、AOC 2、字体 6），同输入两跑 `pack_id` 相同。
- **硬边界**：不重建/重签/重新加密 NCA；RSA 信任链不做，只做哈希 + IVFC；`expected-*.json` 只在构建期用，不随 pack 分发；APK/仓库扫描 `*.xci|*.nsp|*.nca|*.keys|import-pack/`。
- **已知风险**：nstool 2024-10 后停更，zstd/NSB 等新格式 UNVERIFIED；BotW 1.6.0 的 update 已并入整合卡带（E800 仅 6,656 B CNMT）。

### 3.2 L1 重写器

- **落位**：重构 d2r `dm/tools/patch_nso.py@a8782caa`（约 1,024 行）与 nxnative `@0045c1df`（约 1,152 行）的补丁模型，Rust 重写为 `nva-rewrite`。丢 D2R 的 per-title 地址、7 处离线补丁、Apple x18 假设、设备内生成路径。
- **改什么**：`svc #n` → `svc-gate`（per-svc 号，site 只作诊断）、`sub sp,#N` 上取整 16、自旋让步、per-build-ID 字节断言、±128 MB 分支 veneer、跨段 fixup。**不改 TLS**：guest TLS 留在 `TPIDR_EL0`，host TLS 由 48 B gate/`svc-gate` 在进入 host 时切换（见 7.5）；也不碰 x18（host 用 `+reserve-x18`，thunk/gate 零接触）。
- **门号归 host**：`svc_gates.json` 是唯一源，host 侧 generator 发号并生成 `nva_svc.h`、host dispatch 表与门桩；重写器只读并在 manifest 记实际引用。`0x00–0xBF` = SVC 号，`0x100+` 留给 `cntpct`/`yield` 这类合成门。
- **BotW 1.6.0**：`main` ModuleId `8e9978d50bdd20b4c8395a106c27ffde`（镜像 `0x40`），`.text` 34,773,924 B（远小于 ±128 MB）；段 flags `0x3f`（三段 LZ4 + 三段 SHA-256）。
- **产物**：patched NSO（NSO0 布局不变）+ `rewrite-manifest.json`：`input_sha256/output_sha256/build_id/coverage{total,classified,by_rule,unknown[]}/fixups[]/gates[{id,svc,sites,count}]/ruleset/rewriter_version`。
- **验证**：`nva-verify rewrite` 断言 `.text` 每条 4 B 指令 100% 归类（原样/改写/数据/白名单），未识别清单为空；`gates[].id ∈ svc_gates.json` 且两侧 hash 相符；输出 hash == `expected-v1.6.0.json` 的 `rewritten_sha256`（3 个模块）；同输入两次输出字节相同。CI 用合成 NSO（覆盖 `svc`、`mrs/msr`、跨界 `b`、`adrp/add`、自旋）。
- **硬边界**：BotW 字节补丁**不入库**——只入规则 + 期望 hash，字节补丁在用户机器上生成。

### 3.3 L2 着色器

- **落位**：组件 `d2r@a8782caa` 的 Ryujinx 快照（`dm/third_party/ryujinx`，363 files，MIT）；`totk_xlate` 只作 CLI 参考。原 Ryujinx 仓 404、镜像 451，只能 vendor + 锁哈希。
- **语料**：`shader_list.tsv` 8.3 MB：`V` 3,728（唯一 key 3,630，**98 条同 key 不同 deps**）、`S` 26（stage 0/1）、`R` 6,801、`B` 245、`C` 0，fout 最多 8。GS/TS 无 emitter，BotW 语料也只到 VS/FS——先证伪，不写发射器。
- **key**：`SHA-256(control 的前 +0x20 长度字节 ‖ code)`；`stage`/`deps` 不进 key。key/deps 由 cache 库统一派生，构建期与运行期共用一份 Rust 实现；`#27` 漏了运行时 deps 派生，必须补。
- **cache**：`shader-cache.bin` + 尾部定宽 index，条目 `key[32] | stage u8 | flags u8 | spirv_off u32 | spirv_len u32 | deps_off u32 | deps_len u32`；SPIR-V 段 4 B 对齐；身份 `(key, stage, deps)`，索引按 `(key,stage)` 分桶 + 桶内 deps 全等。版本 = `format_version + key_algo_id + translator_build_id + target_vk_api`，任一变化整 cache 作废。
- **运行时**：**直连**——runtime `mmap`，`nva_gfx_desc.shader_cache` 交 `{ptr,len,version}`，engine 直接查；**不走** `nva_host_services.shader_lookup`。生命周期 = gfx，`destroy` 前 `waitIdle`，blob 借用期 = cache 打开期。
- **验证**：`spirv-val` + `glslangValidator` + `spirv-cross` 三道；重复翻译字节一致；index 偏移/对齐/越界自检；命中率 = 画面完整度；注入假 key 断言 not-found 且跳过 draw、不 abort。CI 无 GPU runner，只能验到 SPIR-V（`spirv-val 过 ≠ 能建 pipeline`）。
- **法律**：SPIR-V 是游戏资产派生内容，cache 只进本机 import pack，不进公开 APK。

### 3.4 L3 加载器

- **落位**：runtime 内部 Rust module，**不导出 C header**。参考 MacNativeGames `games/*/runtime/loader.c`（约 47 KB）与 nxnative `loader.c`/`gate.S` 的结构；两者都是 per-title 硬编码地址、iOS 假设，不搬。
- **做什么**：只读 `manifest.json` + `load-plan.json`，按 VA 布局落段（先 `PROT_NONE` 预留整段，再区内 `MAP_FIXED`），设 TLS/栈（`host_create_main_thread`），校验 entry 在 X 段，跳 `rtld→main`。不做文件系统、guest 堆、RomFS、Vulkan。
- **W^X/16 KB**：按 module 建 `memfd`，写段 RW，写完 code/ro 段 `mprotect` 成 R/RX；`memfd_create` 在 API 30 才有，minSdk 26 用 `syscall(SYS_memfd_create)` 兜底。16 KB 设备**不重排布局**：按页合并权限——RX 段共享页整页 RX（rodata 可执行），RW 段落进共享页则把那部分拷到匿名内存；validator 按目标页大小校验并列出被合并的段。先真机 `getconf PAGE_SIZE`。
- **启动顺序**：`host_create → gfx_create(host_services) → host_set_gate_table → host_create_main_thread(TLS/stack) → host_start → 装 svc-gate → get_nvn_table(校验 hash/槽数) → entry`。host 先行：engine 初始化要用 host 的 log/mem_query/on_device_lost。
- **per-thread host TLS**：GATE 模式需要每线程的 host TLS；放在 loader 预留的 guest TLS 头槽，偏移写进 `tls.json`，gate 用 `[TPIDR_EL0 - K]` 取。spike 的单一全局 `g_host_tls` 只够单线程。
- **验证**：PC validator + 合成镜像负例（段重叠/错 hash/entry 落 R 段/非目标页对齐）；arm64 CI entry 冒烟；真机先合成镜像后真镜像，断言 `TPIDR_EL0` = TLS base、`SP&15==0`、x18 sentinel、FPCR 不变、`/proc/self/maps` 无 `rwx`。
- **边界**：loader 不是 C ABI 的一部分；壳/app 不能调；跨语言内部实现放 `src/internal/`，不加 `extern "C"`。

### 3.5 L4 host/SVC

- **落位**：d2r `dm/host@a8782caa` 185 blobs 里约 **51 个通用**（21 `hle_*` + 13 core + 16 tests，约 0.70 MB / 18% 体积），40 个 `dm_*.c`（`dm_cursor`/`dm_rift`/`dm_hud`…）、`hle_granny`/`hle_tact`/`hle_net` 等 D2R 专用整块丢；nxnative `@0045c1df` 补 14 个缺项，但只有 `ro/sf/audren/nvdrv` 直接进产品（`mii` 不用、`movie_avc` 可软解、`haptics` 二期、`glslc/oodle` 是构建期、`sdk16/bigstack/uemalloc/fpslock` 是 per-title 数据、`offline` 是超时兜底）。
- **18 面**：`fs/hid/audout/audren/vi/oe/ro/sf/am/codec/swkbd/nvdrv/sdk/pctl/nifm` + 隐式 `sm/account/set`。MVP 实现 11 面（sm/fs/hid/audout/audren/vi/oe/account/set/sdk/nvdrv），其余先注册并返回 typed stub + 计数。
- **per-title 策略外置**：`title/<build-id>/policy.json`（服务开关、版本、补丁点、fpslock 地址），代码零 title 字符串；`hle_oe.c` 这类“改游戏渲染器内部偏移”的补丁必须删。
- **核心**：SVC dispatch（`nva_svc_frame` 96 B）、pthread + 固定栈/TLS 的线程表、guest VA 分配器 + `nva_mem_query` 唯一翻译入口、句柄 `index|gen<<32`（close 幂等，设备丢失 `gen++`）、nvdrv host 句柄表 + engine 后端、事务存档（临时目录 + commit-list + 整体 rename + fsync）。
- **mesosphère**：GPL-2.0，只作 oracle、独立进程 JSONL 对拍，不链接不复制；它覆盖内核 svc/地址仲裁/condvar/句柄/loader，**不覆盖 fs/hid/audio**。
- **验证**：`nva-verify hle` 场景 JSONL；内核面与 mesosphère 对拍（ret/errno/out/handle_delta/state_digest，虚拟时钟比唤醒顺序）；服务面 golden + 真机 trace；服务级单测（d2r 16 个 + nxnative 5 个）；真机 smoke 5 s entry / 10 s sm+fs / 20 s 首 guest 线程 / 120 s 首帧（阈值未校准）。
- **Android**：host 只用 bionic libc（pthread/mmap/memfd/openat/clock_gettime/futex，不链 `-lpthread`）；音频/震动/日志经 `nva_platform` 调壳；存档落私有目录，SAF 只做导入导出。

### 3.6 L5 图形引擎

- **落位**：自研 Rust + `ash`。d2r `dm/gfx/vulkan@a8782caa`（88 paths，MIT）作边界/实现基线；SMWR `runtime/src/nvn` 55 文件只借目录与行为（Vulkan 1.3/S23 不外推）；LAS 辅；ScarletNX（GPL-3.0）只读不 vendor。
- **口径**：**523** = engine 内部索引（512 core + 11 调试/查询）；**513** = BotW `.ro` 名字面（512 + `nvnBootstrapLoader`，无“名字→指针”表）；**477** = `nvn_protos.h` typedef；**46** = 无原型槽。
- **NVN 面**：engine 拥有 523×16 B `{count,target}` 表、thunk、impl、typed stub、sticky fatal；`nva_nvn_resolve` 按名字返回 thunk，未知名字返回非 NULL zero stub 并计数；46 个无原型槽 `stub_class=UNKNOWN`，只写 x0=0、**不碰 x8**。void 命令遇到未实现依赖必须 sticky-fatal，不静默成功。
- **thunk/TLS**：v1 默认 **48 B gate（`GATE_TLS`）**——保存 guest `TPIDR_EL0`、装 host TLS、`blr`、恢复；只碰 x16/x17/x30，不碰参数/返回寄存器。32 B direct 保留给“guest TLS 已移出 `TPIDR_EL0`”之后，本轮不冻结。**关键事实**：477 条原型整数参数最多 8、浮点最多 8，**NVN 没有栈参数**，gate 的 SP 偏移不破坏任何已知调用。
- **记录/重放**：记录期只解码不可变小状态；`nvnQueueSubmitCommands` 在调用线程同步重放，单把重放锁；两级 `PassCompatKey`/`PassKey` + `FramebufferKey` + `PipelineKey`（不含 viewport/scissor 值）；8 色槽固定，未用槽 `VK_ATTACHMENT_UNUSED`，framebuffer 仍需 8 个合法 view。
- **内存/同步**：GPU 内存 copy-first；guest VA 永不进 `vkCmd*`/descriptor/shader，只接受 `(object, offset, length)`；`nva_mem_query` 是唯一 region 解析入口；`nva_fence{serial,domain,gen}`，设备丢失唤醒 `DEVICE_LOST`/`STALE`。present = window texture mirror → blit → swapchain。
- **Vulkan 1.1**：725 实测 1.1.128、`maxViewports=1`、无 `conditional_rendering`/`dynamic_rendering`/`memory_budget`、无 int64/float64、BC/ASTC/BDA 可用；条件渲染用 occlusion query + 主机端判定；经典 render pass + framebuffer。
- **验证**：725 真机 `all` 9/9、`gate` 10/10、`abort` 子进程 SIGABRT=6；lavapipe validation 干净；真机无 validation layer（#31）；8 MRT ≤28 ms；100 次 attach/detach 无泄漏；device-lost 注入能唤醒等待；首里程碑 = **725 第一帧**（像素/fence/`vk_errors`/帧时间基线），不是 60 fps。

### 3.7 L6 平台壳

- **落位**：自研薄壳（Kotlin/Java + JNI）。参考 d2r `dm/android@a8782caa`（862 paths）与 LAS `@cca791a3` 的结构；LAS 自述是 SDL3 + Gradle，不是 GameActivity。起点是 `spike/abi-slice/android`（现有壳无 `JNI_OnLoad`、无窗口/输入/音频/权限）。
- **基座**：v1 用普通 `Activity` + `SurfaceView`（零依赖，现有 aapt2/d8 构建可用）；GameActivity（AAR/Gradle）作 follow-up；不用 SDL3（自带事件循环/线程模型，与我们的生命周期重复）。
- **surface**：`surfaceCreated/Changed/Destroyed` → `nva_app_attach_window/detach_window`，壳维护 `generation`；`detach` 允许 ≤50 ms 有界阻塞 UI（Android 的 “surfaceDestroyed 返回后不得再用” 是硬契约，强度高于 #27 的“UI 不阻塞”）。
- **输入**：触摸 → `nva_input_event` 无锁环（单生产者）；手柄 `KeyEvent`/`MotionEvent` 映射 Switch hid 位；陀螺仪 `SensorManager`；震动只有幅度包络（Android 无频段），HD 震动降级。
- **音频**：`AAudio`（API 26，Oboe 不用），48 kHz/2ch/i16；回调内只做环形缓冲 memcpy，不分配不加锁；焦点丢失走 duck。
- **导入/存档**：SAF `ACTION_OPEN_DOCUMENT_TREE` 选 pack → 流式拷贝 + 校验 → 私有目录，运行期只用 POSIX 路径；存档落私有目录，SAF 不做实时挂载。
- **FGS**：只承载导入/预热，类型 `specialUse`；游戏本体不挂前台服务（API 35 对 `dataSync` 有时长上限）。
- **验证**：真机 smoke（安装 → `surface_attached gen=1` → HOME → `surface_lost` → 回来 `gen=2` → 音频 active → BACK → 存档 flush → `kill -9` 后恢复一致）；CI 加 `llvm-readelf -l` 断言 `p_align==0x4000`、`llvm-nm -D` 断言 `JNI_OnLoad`、`aapt2 dump xmltree` 断言 `foregroundServiceType`/`minSdk`/无 `debuggable`。

### 3.8 验证（横切，不是层）

- `nva-verify rewrite`：L1（覆盖率、gates、expected hash）。
- `nva-verify shader`：L2（SPIR-V 校验、index、命中率）。
- `nva-verify loadplan`：L3（schema、重叠、对齐、entry）。
- `nva-verify hle`：L4（场景 JSONL、mesosphère 对拍）。
- `nva-verify device`：L5/L6（真机冒烟、帧时间、attach/detach、device-lost）。
- 退出码：0 全过；2 有 mismatch；3 oracle 不可用；4 fixture/hash 过期。CI 无 GPU runner，GPU 相关只能真机。

## 4. 构建时：从 ROM 到能玩

```
XCI / NSP + prod.keys
        │ L0 nstool 解包 + overlay（base→update→DLC，section 级）
        ▼
  raw/exefs（main/rtld/sdk/subsdk0）+ RomFS + shader 表
        │ L1 重写器（per-build-ID，svc-gate/TLS/对齐/veneer）
        │ L2 着色器（Ryujinx 快照，构建期翻译）
        ▼
   code/*.nso + rewrite-manifest.json + shader-cache.bin
        │ L0 nva-pack 汇总（manifest.json / load-plan.json / romfs.bin）
        ▼
   import pack ──┐
                 ├── 手机 ──► 可玩
   APK（CI）─────┘
```

APK 的编译不需要游戏数据：它只含代码与资源；游戏数据由 import pack 在设备上导入。
`pack_id = SHA-256(JCS(manifest 去掉 pack_id))`；manifest 不含绝对路径/时间戳，保证两次运行字节一致。

## 5. 语言与占比

**这是估算区间，不是承诺。** 占比指“我们自己写的源码行数”，不是 APK 体积。

| 语言 | 用在哪 | 源码占比（估算） | 说明 |
|---|---|---|---|
| **Rust** | 图形引擎、加载器、重写器、容器、验证工具、部分平台胶水 | **55–65%** | 新代码主力；随 host 迁移而上升 |
| **C / C++** | host 起步实现、第三方库（lz4、mbedtls、libdeflate） | **25–35%** | 起步更高，逐子系统迁移后下降 |
| **Kotlin/Java** | 安卓壳：Activity、权限、SAF、前台服务、UI | **5–10%** | 行数少，但没它就没有窗口 |
| **ARM64 汇编** | thunk、svc-gate、少量 ABI 垫片 | **< 2%** | 只在边界出现 |
| ~~GLSL~~ | 不写 | 0 | 着色器由游戏提供，我们只翻译 |

**不进 APK 的语言**：Python（构建/验证脚本）、C#（着色器翻译器，构建期）、ObjC/Swift（不在本项目）。

**为什么 Rust 占大头**：引擎的资源生命周期、命令编码、并发状态最吃内存安全；工具链也全在 Rust。
**为什么 C/C++ 不能立刻消失**：host 语义最密，现有 C 实现是唯一参照，先保正确性，再用等价性测试逐块换。

## 6. 一帧的生命周期

1. 游戏算完逻辑，调 `nvnCommandBuffer...` 记录“要画什么”。
2. 引擎把记录翻译成 `vkCmd*` 并提交。
3. GPU 执行，交换到屏幕。
4. 同时 host 在别的线程处理音频、输入与文件。

## 7. C ABI v1

**规则**：模块间只走 `extern "C"`（#29），包括 Rust↔Rust 的跨模块调用；模块内部的 loader/engine 私有函数不走 C ABI。全部固定宽度、结构体首两字段 `size/version`、Rust `panic=abort`、不允许 unwind 穿过 FFI。

### 7.1 三条接缝

| 调用方 → 被调方 | 头 | 内容 |
|---|---|---|
| 壳（JNI C glue）→ runtime | `nva_app.h` | create/start/pause/resume/stop/destroy、attach/detach window、push_input、set_rumble、get_stats、trim_memory |
| runtime → 壳 | `nva_platform.h` | 壳实现：audio、rumble、clock、log |
| runtime → host | `nva_host.h` / `nva_mem.h` / `nva_svc.h` | host_create/start/stop/destroy、svc_dispatch、thread/mem、gate 表 |
| host → engine | `nva_gfx.h` / `nva_nvdrv.h` | gfx 控制面 + typed nvdrv |
| engine → host | `nva_host_services`（`nva_abi.h`） | log/mem_query/on_device_lost/fatal |
| loader → engine | `nva_nvn.h` | 表、`resolve(_ex)`、`bootstrap`、`set_thread_tls`（Rust 调用，布局冻结） |
| guest → engine | thunk（汇编） | **AAPCS64 机器 ABI，不是 C ABI**；按名字 resolve |

### 7.2 头文件集合

v1 只有**一个**公开头 `include/nva_abi.h`：按 `/* ---- 段 ---- */` 分成 6 段（abi / host / gfx / nvn / app / platform），另有两个内部头。v1.1 把 6 段拆成 6 个文件，声明逐字不变。内部头不安装：`nva_svc.h`（生成：gate 号 + `nva_svc_frame`）、`nva_nvdrv.h`（host→engine typed）。

下表「头」一列是 v1.1 拆分后的文件名；v1 中对应 `nva_abi.h` 的同一段。

| 头 | 职责 | 状态 |
|---|---|---|
| `nva_abi.h` | 版本/status、`nva_mem_region`、`nva_fence`、`nva_host_services`、`nva_gfx_desc`、`nva_nvn_table/meta`、`nva_svc_frame` | 冻结 |
| `nva_app.h` | 壳→runtime 生命周期/窗口/输入/stats | 冻结 |
| `nva_platform.h` | runtime→壳 audio/rumble/clock/log | 冻结 |
| `nva_host.h` | runtime→host create/start/stop、dispatch、thread、mem_query | 冻结 |
| `nva_gfx.h` | 控制面 create/destroy/query_caps/wait_idle/wait_fence/register_host_memory/alloc_gpu_backing/notify_dirty/get_stats | 冻结 |
| `nva_nvn.h` | table/slot/meta/resolve(_ex)/bootstrap/set_thread_tls | 冻结 |
| `nva_svc.h` | gate 号 + dispatch（生成） | 内部 |
| `nva_nvdrv.h` | channel/ioctl/syncpoint/nvmap（host→engine） | 内部，v1.1 再定公开 |

**删掉的**：`nva_nvn_install`（loader 是 Rust 内部）、`nva_nvn_dispatch`/`nva_nvn_frame`（统一 frame 要溢出 x0–x7/v0–v7）、`nva_gfx_translate`（降 engine 内部）、`host_services.shader_lookup`（L2 改直连，`nva_gfx_desc.shader_cache` 交 `{ptr,len,version}`）。

### 7.3 归属

- **NVN 表**：engine 所有；借用至 `gfx_destroy`；loader 只校验 hash/槽数；guest 不可见（只拿 thunk）。
- **guest VA**：loader/runtime memmap 所有；host/engine 只能 `alloc_va`，不得各自 `mmap` 猜地址。
- **CPU 内存**：host 所有；engine 只借 `cpu_ptr` 一次调用/replay，不跨 submit 缓存。
- **GPU 内存/Vulkan 对象**：engine 所有；host 只持 opaque `gpu_handle`。
- **`VkSurfaceKHR`/swapchain**：engine 所有；`ANativeWindow` 壳所有，engine attach 时 acquire 自己的引用，detach 返回后壳才 release。
- **nvdrv 句柄**：host 句柄表（index+gen）+ engine 后端；设备丢失 `gen++` 一次性作废。
- **shader cache**：runtime `mmap` 所有；engine 借用至 gfx destroy。
- **SVC frame**：guest 线程栈；仅一次 dispatch 有效；host 要留就复制。
- **input**：壳分配；runtime 复制；不长期持有。
- **stats**：runtime/engine 填；调用方拥有结构体。

### 7.4 版本与不变量

- `NVA_ABI_VERSION = 1`；结构体版本独立：`mem_region 1`、`fence 1`、`host_services 2`、`gfx_desc 2`、`nvn_table 2`、`shader_cache 1`。
- 只尾部追加 + 升该结构 `version`；消费方按 `version` 分支，`size` 只做边界检查，禁止用 size 猜版本。
- `_Static_assert` 全量（`include/abi_layout_check.c`）：`mem_region` 104、`fence` 16、`host_services` 48、`gfx_desc` 32、`nvn_slot` 16、`slot_meta` 20、`nvn_table` 56、`resolve_result` 16、`svc_frame` 96、`shader_cache` 32、`app_callbacks` 48、`input_event` 32。
- `Rust panic=abort`；不允许 unwind 穿 FFI；边界函数不 `catch_unwind`。
- host AArch64 `+reserve-x18`；guest `-ffixed-x18`；thunk/impl/gate 不得读写 x18。
- FPCR/FPSR 跨 NVN 调用不变；`NVNboolean` 是 8-bit（typed stub 写 w0=0，调用方忽略高位）；x8 只由 `ret_class=STRUCT_GT16` 的 typed stub 写，`UNKNOWN` 槽不碰 x8。
- `stadd` 需 ARMv8.1-LSE；未置 `LSE` 用 `ldxr/stxr` 回退（UNVERIFIED）。

### 7.5 v1 必补（来自 #30 真机）

- `nva_nvn_table.flags`：`DIRECT=0` / `GATE_TLS=0x1` / `LSE=0x2`；**v1 默认 `GATE_TLS`**。
- `nva_nvn_slot_meta`（20 B，由 `table.version` 版本化）：`slot/ret_class/arg_gpr/arg_fpr/stack_bytes/stub_class/flags`；GATE 模式拒绝 `stack_bytes != 0`（477 条原型实测全为 0）。
- `nva_nvn_resolve_ex`：`{target, slot, flags}`，区分 KNOWN/UNKNOWN/GATE；`resolve` 保留兼容。
- `nva_nvn_bootstrap(gfx, name)`：`nvnBootstrapLoader`/`GetProcAddress` 的显式入口；未知名字返回非 NULL zero stub。
- `nva_nvn_set_thread_tls(gfx, host_tls)`：每 guest 线程、首次进入前调用；GATE 模式 0 非法。
- `nva_host_services.fatal(user, reason, slot, name)`：engine 只通知，不自行 `abort()`/`exit()`；host 决定墓碑/停 guest/UI 报错。
- `NVA_ERR_TIMEOUT = 8`：`wait_fence` 区分超时与设备丢失。

## 8. 还没定的

- 平台壳 follow-up：GameActivity 迁移、输入延迟实测、触屏 overlay 布局。
- 音频：48 kHz/2ch/i16 之外的格式（5.1 变体）、AAudio 低延迟模式实测。
- 加载器优化：import pack 是否再压、RomFS 是否 VFS、guest 线程/JNI attach 细节。
- 性能剖析与热管理：帧时间采集、sustained 策略、温控降级档。
- 设备兼容矩阵：Mali/PanVK、Adreno 6xx 底线、Vulkan 1.1 vs 1.3。
- 上游回流：给 d2r/MacNativeGames 提 PR 的边界。
- 发布：签名、更新、changelog。
- TotK：版本/DLC 组合、34,179 条变体表存放。
- 六份共享 `.bfttf` 的来源（卡带里没有）。
- `nva_nvdrv` 是否 v1.1 公开；shader 分桶/deps 的最终规则；direct 32 B 的 TLS 外移方案。

## 9. 变更记录

- 2026-10-24：初版（七层、构建链、语言分工）。
- 2026-10-30：定稿 L0–L6（构建链顺序、验证横切）、C ABI v1（`GATE_TLS` 默认、slot_meta、fatal、bootstrap、resolve_ex、shader 直连）、host 先行启动顺序、16 KB 合并权限策略。

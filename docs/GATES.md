# 里程碑门（M0–M3）

> 来源：#22。**本文件是权威正文**，#22 的评论只是指针。
> 证据全私有，见 #33：本文只有判据，没有截图、日志、序列号、本机路径。
> 日期：2026-10-31。决策依据：#20（L0–L6 + C ABI v1）、#21（流水线）、#30（真机竖切）、#31（真机 validation）。

## 0. 总则

- **门（gate）**：一组可执行判据。里程碑由若干**子门**组成，任一子门红 = 该里程碑未过。
- **判据只认命令退出码与 JSON 字段**。形容词不算判据（`start-22.md` §6）。
- 每个「计数 = 0」的判据必须配一次**注入对照**，证明计数器活着。
- **证据全私有**：真机截图、录像、日志、JSON、trace 一律不进仓库、不进公开 artifact。
- 任何门**不得**写 `validation clean`：真机 `validation_available:0`，#31 未关。
- **无时间盒**：只排顺序与前置，不写日期或周数。

## 1. 总表

| 里程碑 | 子门 | 一句话 | 真机 |
|---|---|---|---|
| **M0 骨架** | `M0a` PC 契约 | 五个编译单元 + 产品头布局断言 + lavapipe 一帧空命令 | 否 |
| | `M0b` 真机首帧 | 725 上窗口 + swapchain + clear + present | 是 |
| **M1 标题画面** | 单门三层判据 | 真 `main` 跑到标题 | 是 |
| **M2 进入世界** | `M2a` 世界帧 | 出神庙后高原室外、HUD 在、资源集在 | 是 |
| | `M2b` 控制回路 | 摇杆四向 + ABXY 生效 | 是 |
| **M3 可玩** | `M3a` 热稳态 | 30 min、720p、30 fps 稳 | 是 |
| | `M3b` 系统与内容 | 音频/存档/surface/输入延迟/内容完整性/SAF | 是 |

## 2. M0a — PC 契约门

**范围**：五个编译单元能编、产品头布局对、裸机一帧。不含 loader、不含 guest、不含游戏数据。

**编译单元**：`nva-pack`、`nva-rewrite`、`nva-verify`、`nva-runtime`（loader + host + gfx）、APK 壳（Kotlin + JNI glue）。L2 着色器翻译器（.NET）**不进 M0**。

**判据（全部退出码 0）**：

```sh
cc  -std=c11   -Iinclude -fsyntax-only include/abi_layout_check.c
c++ -std=c++17 -Iinclude -fsyntax-only include/abi_layout_check.c
"$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang" \
    -std=c11 -Iinclude -fsyntax-only include/abi_layout_check.c
cargo build --workspace --release
cargo test -p nva-abi layout          # size_of 断言与 C 侧一致（56/32/48 …）
cargo test -p nva-pack -p nva-rewrite # 合成 fixture
nva-verify loadplan --fixture ci/fixtures/pack-min
nva-pack verify ci/fixtures/pack-min
nva-smoke --headless --frames 1 --json out.json   # vk_errors=0
nm -D target/release/libnva.so | rg ' nva_(app|gfx|host)_'
```

- CI 新增 `abi-check` step，并加入 `gate.needs`（现在 `ci.yml` 里产品头零引用）。
- **变异测试**：临时删一条 `_Static_assert` 或改一个尺寸，CI 必须变红。

**前置**：裁出唯一的权威 ABI 头（产品头），spike 的同名头加 `#error` 互斥。

## 3. M0b — 真机首帧门

**前置**：`M0a` 全绿。

**判据**：

- 装 APK 到 725，启动后 `surface_attached gen=1`；swapchain 建立。
- 连续 `present` ≥1 帧；fence serial +1；画面为 clear 色。
- JSON 字段：`apk_sha256`、`device_model`、`driver`、`vulkan_api`、`surface_gen`、`present`、`vk_errors`、首帧 CPU/GPU 毫秒。
- **反证**：HOME 后不 attach，不得出图、不得崩。
- 不写 `validation clean`。

**红路径**：clear 色值接错（present 了但像素不对）必须红。

## 4. M1 — 标题画面（单门三层）

**前置**：`M0b` 全绿；`nva-pack verify` 退出码 0 且 `degraded == []`（degraded 包不得用于验收）。

**① 活体**：真 `main` 连续 ≥30 s；`present` 单调递增；`fatal = 0`；`sticky_error = 0`；`pack_id` 与 `build_id` 绑定进 JSON。

**② 非清屏**：画面来自 guest 提交的 draw（draw 计数 >0）；两帧差分非零；与 `M0b` 的 clear 参考帧不同。

**③ 语义锚点**：人签**四项排除**——启动 logo、淡入/黑场、错误框、文字空白；两张间隔 ≥5 s 的截图；截图与 JSON 同一 run；同时冻结一份 trace 摘要（RomFS 打开路径哈希、pipeline key 多重集）作交付物。

**文字空白不算过**。**红路径**：错 `build_id` 必须红；不给 pack 必须红。

## 5. M2 — 进入世界

**前置**：M1 全绿；输入有确认键；pack 经 `nva-pack install`（adb，dev-only）进机。

### M2a 世界帧

- 真实按键进 New Game → **出复活神庙后的大高原室外**（先做 movie/加载屏探针再定稿）。
- HUD 出现；连续 present ≥60 s 且 ≥120 帧；p95 ≤200 ms；RSS ≤6 GB。
- 锚定用状态与资源：RomFS 读到的 Map/Pack 集合、shader key 集合、每帧 draw 下限。
- `missing_variants`、`draw_skipped` 进 JSON；帧差非零且与 M1 标题帧不同。
- **红路径**：删世界 draw 必红；清空 cache 必红。

### M2b 控制回路

- 壳 → hid → guest 消费计数一致；摇杆四向各 ≥3 帧；ABXY 各一次；位移/朝向差达阈值。
- 每帧 HID 摘要进证据（输入源 + 注入脚本）；`adb shell input` 注入即可复跑。
- **红路径**：断开输入必红。

## 6. M3 — 可玩

### M3a 热稳态

- 前置：M2a/M2b 全绿；**M2a 冻结的 720p handheld 不得改**。
- 条件：60 Hz、飞行模式、不充电、亮度固定、`fpslock=off`、冷机起步。
- 30 min，前 2 min 作废，**只判最后 5 min**：p50 ≤34 ms、p99 ≤40 ms、1% low ≥27、连续 ≥3 帧 >50 ms 事件 ≤2、无 >200 ms 单帧、RSS 斜率 ≤50 MB/h。
- 采集：Perfetto（frametimeline + sched/freq + power）与 App 内帧时间环对齐；1 Hz 记温度与 GPU 频率（只记录，不作判据）。
- **红路径**：打开 fpslock 必红；降内部分辨率必红。

### M3b 系统与内容

- 五场景各 3 min（不承担热结论）。
- 音频 `xrun = 0`；存档 `kill -9` 后一致；`surface gen=2`；输入到光子 p95 ≤100 ms。
- 内容完整性：`shader miss = 0` 且 `draw_skipped = 0`（与帧率同权）。
- SAF 用 **1 GB 合成包**走通（时间/空间/校验入 JSON）；15.1 GB 全量 SAF 留发布前。
- `stub`/服务计数 = 0；`fatal`/`device_lost`/ANR = 0。
- **红路径**：注入 surface lost、切换音频路由、中断存档。

## 7. 交叉约束

- **输入分三段**：确认键（M2a 前）→ 摇杆/ABXY 的 overlay（M2b）→ 手柄（M3 之后）。
- **pack 上机**：M2/M3 用 adb `nva-pack install --reuse-romfs`（只换 code/shader）；SAF 在 M3b 用 1 GB 合成包验。
- **`nva_stats`**：在 M0b 前按 §7.4 尾部追加 `presents`/`draw_skipped`/`missing_variants`/`underruns`/`input_events`，升 version，不破坏 v1。
- **#31**：不前置任何门；每门只记 `validation_available` 与 `vk_errors`；时间盒到 M2 末，未关则按「环境限制」关。

## 8. 进门时仍存在的已知差距

- 真机 Vulkan validation 不可用（#31）。
- 523 槽只验过 14 槽；per-thread host TLS 未做（spike 是单全局）。
- movie/codec（`subsdk0`）是否能播未证；若开局必经，M2a 的探针先判。
- 六份 `.bfttf` 已提取并记 hash，来源不进仓库。
- 8 MRT ≤28 ms、present 延迟、16 KB page size 均未实测。
- `LSE` 回退（`ldxr/stxr`）未验。

## 9. 变更记录

- 2026-10-31：初版，来自 #22。

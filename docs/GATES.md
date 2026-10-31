# 里程碑门（M0–M3）

> 来源：#22（评论为指针，本文件为权威正文）。证据全私有，见 #33。
> 版本：v3（2026-10-31）。v1/v2 均经两遍审查判「需修/不可用」，改动见 §10。
> 依据：#20（L0–L6 + C ABI v1）、#21（流水线）、#30（真机竖切）、#31（真机 validation）、#33（公开边界）。

## 0. 总则

- **定位**：这是个人项目的验收纪律，不对外证明任何事。它要防的是**自己骗自己**：目标没达成就把门签掉。
- **门**：一组可执行判据。里程碑由子门组成，任一子门红 = 该里程碑未过。
- **载体**：真机门一律由 `nva-verify device --scenario <id> --json <out>` 执行；PC 门由 `nva-verify smoke` / `cargo test` 执行。退出码 `0` 通过 / `2` 不符 / `3` 见证不可用 / `4` 证据过期。
- **判据**：只认退出码与 JSON 字段。每个字段必须写**来源**（`stats` / `maps` / `dumpsys` / `getprop` / `perfetto` / `logcat` / 像素）。
- **身份字段由 verifier 现场取**，不接受调用者传值：

| 字段 | 取法 |
|---|---|
| `commit` | `git rev-parse HEAD` + `git status --porcelain` 为空 |
| `apk_sha256` | `sha256sum $(pm path <pkg>)` |
| `pack_id` / `build_id` | 读机上 pack 的 `manifest.json`，与 `nva-pack verify` 输出比对 |
| `device_model` / `driver_version` / `vulkan_api` / `page_size` | `getprop` / `vulkaninfo` / `getconf PAGE_SIZE` |
| `scenario_id` | 由 verifier 按命令行写入，JSON 里必须回写 |

- **注入对照**：每个 `= 0` 的计数，JSON 必须带 `injection{name,before,after}`，且 `after - before ≥ 1`；注入与正式测量分两个 run。
- **证据全私有**：截图、日志、JSON、trace、台账都不进仓库、不进公开 artifact。公开仓零验收数据。
- 任何门**不得**写 `validation clean`。
- **无时间盒**：只排顺序与前置。

## 1. 总表（含前置交付）

| 子门 | 目标 | 前置交付（当前不存在） | 真机 |
|---|---|---|---|
| `M0a` | 五个 Rust 单元 + 权威头 + 壳 + lavapipe 一帧 | 根 workspace、`ci/fixtures/*`、`nva-verify smoke`、5 个 crate | 否 |
| `M0b` | 窗口 + swapchain + clear + present | `nva-verify device`、`shell-apk` job 启用、产品 manifest 模式 | 是 |
| `M1` | 真 `main` 跑到标题 | pack 上机、guest 存活证据、四项排除签署 | 是 |
| `M2-0` | 确认键被 guest 消费 | overlay 的确认键 | 是 |
| `M2a` | 出神庙后高原室外 | movie/加载屏探针、`render_extent` 记录 | 是 |
| `M2b` | 摇杆四向 + ABXY | overlay 摇杆、`adb shell input motionevent` 注入脚本 | 是 |
| `M3a` | 30 min 热稳态 | 冻结路线、Perfetto config、fpslock 检测 | 是 |
| `M3b` | 系统与内容 | 五场景脚本、音频路由注入、SAF 1 GB 合成包生成器 | 是 |

## 2. 公共约定

### 2.1 Rust 单元与壳

- Rust workspace 恰好五个成员：`nva-abi`（头 + Rust 布局镜像）、`nva-pack`、`nva-rewrite`、`nva-verify`、`nva-runtime`。
- **APK 壳不是 cargo 成员**（Kotlin/JNI），用 `aapt2`/`d8`/`apksigner` 构建，单独断言。
- `shell-apk` job 由 `if: false` 改为启用，并进入 `gate.needs`。

### 2.2 JSON 公共字段

PC 门（`smoke`）：`schema_version`、`scenario_id`、`commit`、`tool_version`、`vk_errors`、`device_name`、`pixel_rgba`、`injection`。

真机门：上一组去掉 `pixel_rgba` 的必需性，加 `apk_sha256`、`pack_id`、`build_id`、`device_model`、`driver_version`、`vulkan_api`、`page_size`、`surface_generation`、`present_count`、`frame_ms[]`（原始样本，不是分位数）。

### 2.3 注入对照

| 计数 | 注入 | 期望 |
|---|---|---|
| `vk_errors` | 传一个非法 handle | +1 |
| `fatal` / `sticky_error` | 调 `ABORT` 槽 | +1 |
| `xrun` | 占满音频输出 | +1 |
| `missing_variants` / `draw_skipped` | 用一个不在 cache 的 key | 各 +1 |
| `stub_abort` | 调未实现且必须失败的命令 | +1 |
| `input_events_consumed` | 注入一次确认键 | +1 |
| `surface_generation` | detach / attach 两次 | +1 |

## 3. M0a — PC 契约门

**判据（全部退出码 0，且解析输出）**

```sh
cc  -std=c11   -Iinclude -fsyntax-only include/abi_layout_check.c
c++ -std=c++17 -Iinclude -fsyntax-only include/abi_layout_check.c
"$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang" \
    -std=c11 -Iinclude -fsyntax-only include/abi_layout_check.c
cargo metadata --no-deps --format-version 1 | jq '.packages | length'   # == 5
cargo build --workspace --release
cargo test -p nva-abi -- --exact layout_sizes      # 输出含 "1 passed"
cargo test -p nva-pack -p nva-rewrite              # 输出含 "2 passed" 及以上
nva-verify loadplan --fixture ci/fixtures/pack-min
nva-pack verify ci/fixtures/pack-min               # fixture 必须带 shader cache，degraded 为空
nva-verify smoke --headless --frames 1 --json smoke.json
rg -c ' nva_app_'  <(nm -D target/release/libnva_runtime.so)   # ≥1
rg -c ' nva_gfx_'  <(nm -D target/release/libnva_runtime.so)   # ≥1
rg -c ' nva_host_' <(nm -D target/release/libnva_runtime.so)   # ≥1
```

- `smoke.json`：`device_name` 含 `lavapipe`、`pixel_rgba == 0xff00ff00`、`vk_errors == 0`。
- 壳：`aapt2 dump xmltree` 断言包名/`minSdk 26`/`targetSdk`/无 `debuggable`，`llvm-nm -D` 断言 `JNI_OnLoad`。
- 两个同名 ABI 头互斥：spike 头必须含 `#error`，脚本 `rg -q '#error' spike/abi-slice/include/nva_abi.h`。
- 以上步骤并入现有 `pc-x86_64`，其失败必须让 `gate` 变红。
- **变异测试**：删一条 `_Static_assert`、或删一个 workspace 成员，CI 必红。

**ABI 边界**：`nva_stats` 的字段追加**不在本票**。追加由实现票完成（尾部追加 + 升该结构 version），完成后回填本门的布局断言；本票不改 ABI。

## 4. M0b — 真机首帧门

`--scenario m0b`。

| 字段 | 来源 | 期望 |
|---|---|---|
| `surface_generation` | stats | 1 |
| `present_count` | stats | ≥1 |
| `frame_ms[0]` | stats | 有值 |
| `pixel_rgba` | exec-out 截图取点 | 等于 clear 期望值 |
| `surface_destroyed_still_submitting` | stats | false |

- 反证：HOME 后 `present_count` 不增、不崩。
- 红路径：色值不符 → 2；destroy 后仍提交 → 2。

## 5. M1 — 标题画面

`--scenario m1`。前置：`M0b` 绿；pack 已 `nva-pack install --reuse-romfs` 进机；`nva-pack verify`=0 且 `degraded == []`。

### 5.1 活体（guest 真在跑）

| 字段 | 来源 | 期望 |
|---|---|---|
| `guest_image_sha256` | loader 对 guest 可执行段回读后哈希 | 等于 manifest 中改写后 `main` 的哈希 |
| `guest_entry_pc_seen` | loader 记录 entry 处取指 | true |
| `guest_svc_count` | host | ≥1 |
| `guest_threads` | host 线程表 | ≥1 |
| `present_count` 单调 | stats | 30 s 内递增 |
| `fatal` / `sticky_error` | stats | 0（带注入块） |

### 5.2 非清屏

`draw_calls > 0`；相邻帧差分非零（容差 1/255）；与 `M0b` 的 clear 参考帧不同。

### 5.3 语义锚点

- 硬锚：RomFS 打开路径集合的非空子集（如 `Layout/`、`Pack/` 前缀计数 > 0）与 pipeline key 数量 ≥ M1 首跑冻结值。
- 人签：四项排除（启动 logo、淡入/黑场、错误框、文字空白），两张间隔 ≥5 s 截图，与 JSON 同 `run_id`。

**红路径**：`guest_image_sha256` 不符或 `guest_svc_count = 0`（回放形态）→ 2；错 build-id → 2；无 pack → 2。

## 6. M2 — 进入世界

### 6.1 M2-0 确认键

`--scenario m2-0`。`input_events_consumed` ≥1（guest 侧计数），带注入块。

### 6.2 M2a 世界帧

`--scenario m2a`。

- 前置子门：`codec_calls` / `movie_open` 探针（记录即可）；若开局必经 movie，本门阻塞并升级为缺口。
- 真实按键进 New Game → 出神庙后高原室外。
- `render_extent` = 1280×720、`surface_size` 记录；`hud_present = true`。
- 60 s 内 `present_count ≥ 120`；p95 ≤200 ms；RSS ≤6 GB。
- 冻结：每帧 draw 下限、Map/Pack 集合、pipeline key 集合写入台账。
- 红路径：`draw_calls` 低于下限 → 2；清 cache 后 `missing_variants > 0` 且 `draw_skipped > 0` → 2。

### 6.3 M2b 控制回路

`--scenario m2b`。overlay + `adb shell input motionevent`。

- 摇杆四向各持续 ≥3 帧；ABXY 各一次；`input_events_consumed` 与壳侧发送数一致。
- 位移 > 0.5 世界单位，或朝向 > 3°（首跑写入台账后固定）。
- 红路径：断输入后计数不增 → 2。

## 7. M3 — 可玩

### 7.1 M3a 热稳态

`--scenario m3a`。前置：`M2a`/`M2b` 绿；`render_extent` 冻结 1280×720。

条件写入 JSON：`display_hz = 60`、`airplane = true`、`charging = false`、`brightness` 固定、`fpslock = off`、`cold_start = true`、`duration_s ≥ 1800`。

判据（只看最后 300 s）：

| 指标 | 期望 |
|---|---|
| 中位帧时间 | ≤33.4 ms（30 fps） |
| 1% low | ≥27 fps（沿用 #4） |
| 最长单帧 | ≤100 ms |
| 连续 ≥3 帧 >50 ms 的事件 | ≤2 次 |
| 每帧 draw | ≥ M2a 冻结的绝对下限 |
| `missing_variants` / `draw_skipped` | 0（带注入块） |
| RSS 斜率 | 全部 30 min 计算，≤50 MB/h |

- 采集：Perfetto（frametimeline + sched/freq）与 App 帧时间环对账；分位数从原始 `frame_ms[]` 现场重算，不采信 App 写的分位数。
- 红路径：`fpslock = on` → 2；`render_extent` 变低 → 2。

### 7.2 M3b 系统与内容

`--scenario m3b-<scene>`，五个场景各一份 JSON：高原、村庄、营地战斗、神庙、水边。

- 音频 `xrun = 0`（带注入块）；音画偏差记录。
- 存档：`kill -9` 前快照与重开后**逐字节相同**，且能读回同一存档位；中断写入后仍可恢复。
- `surface_generation ≥ 2`（一次 detach 后）。
- 输入到 present p95 ≤100 ms（同钟时间戳）。
- `missing_variants = 0`、`draw_skipped = 0`。
- `stub_abort = 0`；typed stub 只认封闭白名单，新增即红。
- SAF：1 GB 合成包导入成功，时间/空间/校验入 JSON。
- `fatal` / `device_lost` / ANR = 0。
- 红路径：注入 surface lost、切换音频路由、中断存档。

## 8. 交叉约束

- 输入三段：确认键（`M2-0`）→ overlay 摇杆/ABXY（`M2b`）→ 手柄（M3 之后）。
- pack 上机：M1–M3 用 `nva-pack install --reuse-romfs`（adb，dev-only）；SAF 在 `M3b`。
- `nva_stats` 追加由实现票完成，完成后回填 `M0a` 断言（本票不改 ABI）。
- **#31**：不前置任何门。每门记 `validation_available` 与 `vk_errors`；到 M2 末仍未关，则按 #31 自己的路径 3 收口，并附失败命令、loader 版本、logcat 哈希。
- 证据：`PIPELINE.md:49` 的公开 evidence artifact 对产品门退役，只在 spike 保留。

## 9. 进门时仍存在的已知差距

- 真机 Vulkan validation 不可用（#31）。
- 523 槽只验过 14 槽；per-thread host TLS 未做。
- movie/codec 能否播未证（`M2a` 前置子门）。
- 六份 `.bfttf` 已提取并记 hash，不进仓库。
- 8 MRT ≤28 ms、present 延迟、16 KB page size、`LSE` 回退未实测。
- `dm/` 与 `ci/dmgendir` 不在仓，`shell-apk` 启用需先补替身（`PIPELINE.md` §10）。

## 10. 变更记录

- 2026-10-31 v1：初版。
- v2：补载体与退出码、M1 加 guest 证据、`M2-0`、720p 记录。
- v3：修 v2 的硬伤——Rust 成员改为 5 个含 `nva-abi`、壳单列为 Kotlin 构建、`rg -c` 拆三条、`p50` 改 33.4 ms 对齐 #4、`nva_stats` 追加移出本票、注入带 before/after、身份字段现场取、M3b 五场景各一 JSON、存档一致性改为逐字节、stub 白名单封闭；删掉 `pc_samples_in_text`、`guest_draw_callsite`、draw 多重集比对（无采样器、场景未冻结）。

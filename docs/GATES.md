# 里程碑门（M0–M3）

> 来源：#22（评论为指针，本文件为权威正文）。证据全私有，见 #33。
> 版本：v2（2026-10-31）。v1 经两遍审查判为「需修」，改动见 §10。
> 依据：#20（L0–L6 + C ABI v1）、#21（流水线）、#30（真机竖切）、#31（真机 validation）、#33（公开边界）。

## 0. 总则

- **门**：一组可执行判据。里程碑由子门组成，任一子门红 = 该里程碑未过。
- **执行载体**：真机门一律通过 `nva-verify device --scenario <id> --json <out>` 执行，退出码 `0` 通过 / `2` 不符 / `3` 见证不可用 / `4` 证据过期（沿用 §3.8 的语义）。
- **判据只认命令退出码与 JSON 字段**。每个字段必须标来源：`stats`（App 内计数）、`dumpsys`（SurfaceFlinger/Activity）、`perfetto`、`logcat`、`exec-out`（像素）。没有来源的字段不算判据。
- **身份绑定**：每份 JSON 必须含 `commit`、`tool_version`、`apk_sha256`、`pack_id`、`build_id`、`device_model`、`driver_version`、`vulkan_api`、`page_size`。缺一项即不通过。
- **活体对照**：每个 `= 0` 的计数必须附一次注入，证明计数器活着（见 §2.3）。
- **证据全私有**，不进仓库、不进公开 artifact；公开仓零验收数据。台账（`gate-ledger.jsonl`：commit → 门 → 结果 → 证据 sha256）也放私有目录。
- 任何门**不得**写 `validation clean`：真机 `validation_available:0`，#31 未关。
- **无时间盒**：只排顺序与前置。

## 1. 总表

| 里程碑 | 子门 | 一句话 | 真机 |
|---|---|---|---|
| **M0 骨架** | `M0a` PC 契约 | 五个编译单元 + 权威头布局 + lavapipe 一帧 | 否 |
| | `M0b` 真机首帧 | 725 上窗口 + swapchain + clear + present | 是 |
| **M1 标题画面** | 单门三层 | 真 `main` 在跑，画面非清屏，人签四项排除 | 是 |
| **M2 进入世界** | `M2-0` 确认键 | 一次按键被 guest 消费 | 是 |
| | `M2a` 世界帧 | 出神庙后高原室外，HUD 与资源集在 | 是 |
| | `M2b` 控制回路 | 摇杆四向 + ABXY 生效 | 是 |
| **M3 可玩** | `M3a` 热稳态 | 30 min、720p、30 fps 稳 | 是 |
| | `M3b` 系统与内容 | 音频/存档/surface/延迟/内容/SAF | 是 |

## 2. 工具前置（不建则所有真机门不可执行）

### 2.1 必须存在的执行物

- 根 `Cargo.toml` workspace，成员恰好五个：`nva-pack`、`nva-rewrite`、`nva-verify`、`nva-runtime`、`shell`。
- `ci/fixtures/pack-min`、`ci/fixtures/rewrite-min`（合成，无游戏数据）。
- `nva-verify` 子命令：`rewrite`、`loadplan`、`shader`、`hle`、`device`、`smoke`。
- `shell-apk` job 启用，并写入 `gate.needs`；`check_manifest.sh` 增加产品模式。

### 2.2 设备场景 JSON 最小字段

`gate`/`commit`/`tool_version`/`run_id`/`ts_utc`/`apk_sha256`/`pack_id`/`build_id`/`device_model`/`driver_version`/`vulkan_api`/`page_size`/`surface_generation`/`present_count`/`frame_ms[]`/`vk_errors`/`validation_available`。

### 2.3 注入对照表

| 计数 | 注入方式 | 期望 |
|---|---|---|
| `vk_errors` | 故意传非法 handle 一次 | 计数 +1 |
| `fatal` / `sticky_error` | 调一个 `ABORT` 槽 | 计数 +1，进程按策略停 |
| `xrun` | 拔掉/占满音频输出 | 计数 +1 |
| `missing_variants` | 用一个不在 cache 的 key | 计数 +1 且该 draw 被跳过 |
| `draw_skipped` | 同上 | 计数 +1 |
| `stub_abort` | 调一个未实现且必须失败的命令 | 计数 +1 |

## 3. M0a — PC 契约门

**范围**：workspace 与合成 fixture 从零建立（§2.1），五个单元能编，权威头布局对，lavapipe 出一帧。

**前置**：裁出唯一权威头（产品 `include/nva_abi.h`），spike 的同名头加 `#error` 互斥（`PIPELINE.md:220` 的待办）。

**判据（全部退出码 0，且解析输出，不看「跑完就算」）**：

```sh
cc  -std=c11   -Iinclude -fsyntax-only include/abi_layout_check.c
c++ -std=c++17 -Iinclude -fsyntax-only include/abi_layout_check.c
"$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang" \
    -std=c11 -Iinclude -fsyntax-only include/abi_layout_check.c
cargo build --workspace --release
cargo test -p nva-abi -- --exact layout_sizes   # 输出必须含 "1 passed" 及以上
cargo test -p nva-pack -p nva-rewrite           # 输出必须含 "N passed"，N ≥ 1
nva-verify loadplan --fixture ci/fixtures/pack-min
nva-pack verify ci/fixtures/pack-min
nva-verify smoke --headless --frames 1 --json out.json  # 断言设备名含 lavapipe、pixel_rgba、vk_errors=0
nm -D target/release/libnva*.so | rg -c ' nva_(app|gfx|host)_'  # 三个前缀各至少一个
```

- workspace 成员数必须等于 5（脚本断言）。
- CI 新增 `abi-check` step，并加入 `gate.needs`。
- **变异测试**：删一条 `_Static_assert`、或删一个 workspace 成员，CI 必须变红。
- **`nva_stats` 尾部追加在本门完成**（`presents`、`draw_skipped`、`missing_variants`、`underruns`、`input_events`），升 version；追加后本门的布局断言按新版本重跑。

## 4. M0b — 真机首帧门

**前置**：`M0a` 全绿；`shell-apk` 已启用。

**场景**：`nva-verify device --scenario m0b --json m0b.json`。

**判据**：

- `surface_generation = 1`；swapchain 建立；`present_count ≥ 1`；`frame_ms[0]` 有值（首帧基线）。
- 像素：`exec-out screenshot` 取 clear 区域，`pixel_rgba` 等于期望值。
- 反证：HOME 后不 attach，必须 `present_count` 不增、且不崩。
- 不写 `validation clean`；`validation_available` 如实记 0。

**红路径**：clear 色值接错（present 了但像素不符）必须红；`surface_destroyed` 后仍提交必须红。

## 5. M1 — 标题画面

**前置**：`M0b` 全绿；`nva-pack verify` 退出 0 且 `degraded == []`；pack 已通过 `nva-pack install` 进机。

**① 活体（且必须是 guest 在跑）**：

- 真 `main` 连续 ≥30 s，`present_count` 单调递增。
- **`guest_image_sha256`** 等于 manifest 里改写后 `main` 的哈希（证明跑的是这份代码）。
- **`pc_samples_in_text ≥ 1000` 且 ≥95% 落在 guest X 段**（采样线程 PC，证明 guest 在取指）。
- **`guest_draw_callsite`**：抽一次 draw，返回地址落在 guest `.text` 内。
- `guest_threads ≥ 1`；`fatal = 0`；`sticky_error = 0`。

**② 非清屏**：`draw_calls > 0`；相邻两帧差分非零（容差写进 JSON）；与 `M0b` 的 clear 参考帧不同。

**③ 语义锚点**：人签四项排除（启动 logo、淡入/黑场、错误框、文字空白），两张间隔 ≥5 s 的截图，截图与 JSON 同一 `run_id`；冻结一份 trace 摘要（RomFS 打开路径哈希、pipeline key 多重集）作交付物。

**文字空白不算过。红路径**：<br>
- 回放模式（不加载 guest，仅重放录好的命令流）必须红——`guest_image_sha256` 为空或 `pc_samples_in_text = 0`；
- 错 `build_id` 必须红；不给 pack 必须红。

## 6. M2 — 进入世界

### 6.1 M2-0 确认键门

在 `M2a` 之前单独过：注入一次确认键，`input_events_consumed_by_guest ≥ 1`（guest 侧计数，非壳侧）。不要求摇杆。

### 6.2 M2a 世界帧

- 真实按键进 New Game → **出复活神庙后的大高原室外**。movie/加载屏探针作为本门的前置子门：`codec_calls` 与 `movie_open` 计数如实记录；若必经，则本门阻塞并升级为缺口。
- HUD 出现；`present_count` 60 s 内 ≥120；p95 ≤200 ms；RSS ≤6 GB。
- `render_extent` 与 `surface_size` 必须记录（M3a 冻结用）。
- 锚定：RomFS 读到的 Map/Pack 集合、shader key 集合、每帧 draw 下限（写入 JSON）。
- `missing_variants`、`draw_skipped` 进 JSON。
- **红路径**：删世界 draw 必红；清空 cache 必红（`missing_variants > 0` 且 `draw_skipped > 0`）。

### 6.3 M2b 控制回路

- 触屏 overlay，`adb shell input motionevent` 注入。
- 壳→hid→guest 消费计数一致；摇杆四向各持续 ≥3 帧；ABXY 各一次。
- 位移阈值：两次采样的玩家位置变化 > 0.5 世界单位，或朝向变化 > 3°（首跑写入台账，之后固定）。
- **红路径**：断开输入必红（`input_events_consumed = 0`）。

## 7. M3 — 可玩

### 7.1 M3a 热稳态

**前置**：`M2a`/`M2b` 全绿；`render_extent` 冻结为 720p handheld，本门断言其不变。

**条件（写进 JSON，不只写在文档里）**：`display_hz = 60`、`airplane = true`、`charging = false`、`brightness` 固定值、`fpslock = off`、`cold_start = true`。

**判据**：`duration_s ≥ 1800`；只判最后 300 s：`p50 ≤ 34 ms`、`p99 ≤ 40 ms`、`1% low ≥ 27 fps`、连续 ≥3 帧 >50 ms 的事件 ≤2 次、无 >200 ms 单帧；`RSS` 斜率按**全部 30 min** 计算 ≤50 MB/h。

**继承 M2a 的下限**：每帧 draw ≥ M2a 记录的 90%，且 `missing_variants = 0`、`draw_skipped = 0`；与 M2a 的 draw 多重集比对，缺一项即红。

**红路径**：`fpslock = on` 必红；`render_extent` 变低必红。

### 7.2 M3b 系统与内容

**前置**：`M3a` 全绿。

- 五场景各 3 min（不承担热结论）：高原、村庄、营地战斗、神庙、水边。
- 音频 `xrun = 0`；音画同步偏差有记录。
- 存档：`kill -9` 后重开一致；**中断写入**注入必须产生可恢复状态（不是红路径，是恢复判据）。
- `surface_generation ≥ 2`（一次 detach 之后）。
- 输入到 present p95 ≤100 ms（输入时间戳与 present 时间戳同钟）。
- 内容完整性：`missing_variants = 0` 且 `draw_skipped = 0`（与帧率同权）。
- `stub_abort = 0`；typed stub 的白名单与计数另记，不要求全 0（MVP 11 面本就返回 typed stub）。
- SAF：1 GB 合成包导入成功，记录时间/空间/校验；15.1 GB 全量留发布前。
- `fatal`/`device_lost`/ANR = 0。
- **红路径**：注入 surface lost、切换音频路由、中断存档。

## 8. 交叉约束

- **输入三段**：确认键（`M2-0`）→ overlay 摇杆/ABXY（`M2b`）→ 手柄（M3 之后）。
- **pack 上机**：M1–M3 用 `nva-pack install --reuse-romfs`（adb，dev-only）；SAF 在 `M3b`。
- **`nva_stats`**：在 `M0a` 内完成尾部追加并升 version（见 §3）。
- **#31**：不前置任何门；每门只记 `validation_available` 与 `vk_errors`；到 M2 末未关则按「环境限制」关（此条为实现侧判断，不进门的判据）。
- **证据**：`PIPELINE.md:49` 的公开 evidence artifact 对产品门退役，只在 spike 保留；产品门证据全部私有（#33）。

## 9. 进门时仍存在的已知差距

- 真机 Vulkan validation 不可用（#31）。
- 523 槽只验过 14 槽；per-thread host TLS 未做。
- movie/codec（`subsdk0`）能否播未证——已升为 `M2a` 前置子门。
- 六份 `.bfttf` 已提取并记 hash，来源不进仓库。
- 8 MRT ≤28 ms、present 延迟、16 KB page size、`LSE` 回退均未实测。

## 10. 变更记录

- 2026-10-31 v1：初版，来自 #22。
- 2026-10-31 v2：补执行载体与退出码（§0、§2）；M1 加 guest 执行证据；`nva_stats` 追加并入 M0a；`M2-0` 确认键门；720p 在 M2a 记录、M3a 断言；红路径补像素与计数；==0 计数补注入表；`stub` 判据改为只看 abort 类；每门绑 commit 与工具版本；补 workspace/fixtures 从零建立。

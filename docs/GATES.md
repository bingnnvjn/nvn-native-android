# 里程碑门（M0–M3）

> 来源：#22（评论为指针，本文件为权威正文）。证据全私有，见 #33。
> 版本：v4（2026-10-31）。v1–v3 均经两遍审查；本版修的是「规格 vs 可执行物」的先后。
> 依据：#20（L0–L6 + C ABI v1）、#21（流水线）、#30（真机竖切）、#31（validation）、#33（公开边界）。

## 0. 效度边界

**本文件是规格，不是今天就能跑的门。** 设备门的执行物与计数字段都还不存在，见 §2 的前置交付与 §2.4 的追加清单。在那些东西落地之前，M0b–M3b **不可签**，这是已知状态，不是缺陷。

它防的是自己骗自己：目标没达成就把门签掉。它不防刻意造假——门认的是本机 verifier 写出的数据。

## 1. 总则

- **门**：一组可执行判据；子门任一红 = 该里程碑未过。
- **载体**：设备门由 `nva-verify device --scenario <id> --json <out>` 执行；PC 门由 `cargo run -p nva-verify -- smoke` 与 `cargo test` 执行。退出码沿用 #20 §3.8：`0` 通过 / `2` 不符 / `3` oracle 不可用 / `4` fixture 或 hash 过期。
- **判据**：只认退出码与 JSON 字段；每个字段标来源（`stats` / `perfetto` / `dumpsys` / `getprop` / `logcat` / 像素）。
- **身份字段现场取**：

| 字段 | 取法 |
|---|---|
| `commit` | `git rev-parse HEAD`，且 `git status --porcelain` 为空 |
| `apk_sha256` | `adb shell pm path <pkg>` 去掉 `package:` 前缀后 `sha256sum`，取回本地比对 |
| `pack_id` / `build_id` | 读机上 pack 的 `manifest.json`，与 `nva-pack verify` 输出比对 |
| `device_model` / `driver_version` / `vulkan_api` / `page_size` | `getprop` / `dumpsys SurfaceFlinger` / `getconf PAGE_SIZE` |
| `run_id` / `ts_utc` | verifier 生成，写进 JSON，并作为证据文件名 |

- **注入对照**：每个 `= 0` 的计数，必须在**同一次 scenario run** 内带 `injection{name,before,after}` 且 `delta ≥ 1`；注入在测量之前完成，测量窗口内不带注入。
- **可停机的人手步骤**（进 New Game、SAF 选择）：scenario 允许 `operator_input` 暂停，JSON 记录 `operator_input{step,ts}`；不记为自动化。
- **证据全私有**，仓库零验收数据；台账 `gate-ledger.jsonl`（私有）记 `{gate, commit, apk_sha256, pack_id, run_id, ts_utc, result, evidence_sha256[]}`。
- 任何门**不得**写 `validation clean`。
- **无时间盒**。

## 2. 前置交付

### 2.1 仓库侧（M0a 负责建）

- 根 `Cargo.toml`，成员恰好五个：`nva-abi`、`nva-pack`、`nva-rewrite`、`nva-verify`、`nva-runtime`。
- **APK 壳不是 cargo 成员**（Kotlin/JNI，`aapt2`/`d8`/`apksigner` 构建），单独断言。
- `ci/fixtures/pack-min`、`ci/fixtures/rewrite-min`（含 shader cache，`degraded` 为空）。
- `nva-verify` 子命令：`rewrite`、`loadplan`、`shader`、`hle`、`device`、`smoke`。
- `shell-apk` job 启用并入 `gate.needs`；其 `dm/`、`ci/dmgendir` 替身先补（`PIPELINE.md` §10）。

### 2.2 头文件

- 产品头保持 `include/nva_abi.h`。
- **spike 的 `include/nva_abi.h` / `abi_layout_check.c` 改名为 `nva_abi_spike.h` / `abi_layout_check_spike.c`**，并更新 spike 的 include；不保留同名文件（同名加 `#error` 会打断 spike 构建）。CI 断言（`guard` job，`check_header_collision.sh`）spike 树里不存在与产品 `include/` 同名的文件。

### 2.3 设备侧

`nva-verify device` 的 scenario：`m0b`、`m1`、`m2-0`、`m2a`、`m2b`、`m3a`、`m3b-{plateau,village,camp,shrine,water}`。

### 2.4 计数字段追加（ABI 追加票的输入）

现有 `nva_stats`（`nva_abi.h:135`）只有 `unsupported_calls`、`unknown_resolves`、`stub_aborts`、`sticky_error`、`last_serial`、`flags`、`frames`、`draw_calls`、`submits`。设备门还需要下列字段，全部 `uint32_t`，**尾部追加、`NVA_STATS_VERSION` 升 2**，由实现票执行（本票不改 ABI）：

| 字段 | 用途 | 门 |
|---|---|---|
| `presents` | 上屏次数（与 `frames` 区分：跳帧时两者不等） | M0b–M3a |
| `surface_generation` | attach/detach 代数 | M0b、M3b |
| `missing_variants` | cache 未命中的着色器变体数 | M2a、M3a、M3b |
| `draw_skipped` | 因缺变体被跳过的 draw | M2a、M3a、M3b |
| `xruns` | 音频欠载次数 | M3b |
| `input_events` / `input_consumed` | 送出 / guest 消费 | M2-0、M2b |
| `guest_svc_count` | guest 陷入 host 的次数 | M1 |
| `draw_caller_in_guest` | 调用点落在 guest X 段的 draw 数（thunk 记 x30） | M1 |

帧时间**不进** `nva_stats`：用 Perfetto `FrameTimeline` 的原始帧样本，与 `presents` 交叉核对。

## 3. M0a — PC 契约门

全部命令在 `set -euo pipefail` 下执行，且**每条都断言输出**：

```sh
set -euo pipefail
cc  -std=c11   -Iinclude -fsyntax-only include/abi_layout_check.c
c++ -std=c++17 -Iinclude -fsyntax-only include/abi_layout_check.c
"$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang" \
    -std=c11 -Iinclude -fsyntax-only include/abi_layout_check.c
[ "$(cargo metadata --no-deps --format-version 1 | jq '.packages | length')" = "5" ]
cargo build --workspace --release
cargo test -p nva-abi -- --exact layout_sizes | tee t.log; grep -q '1 passed' t.log
cargo run -p nva-verify -- loadplan --fixture ci/fixtures/pack-min
cargo run -p nva-verify -- smoke --headless --frames 1 --json smoke.json
rg -q '"device_name" *: *"[^"]*llvmpipe' smoke.json      # #30：lavapipe 的 deviceName 是 llvmpipe
rg -q '"pixel_rgba" *: *"0xff00ff00"' smoke.json
rg -q '"vk_errors" *: *0' smoke.json
for p in nva_app_ nva_gfx_ nva_host_; do
  [ "$(nm -D target/release/libnva_runtime.so | rg -c " $p")" -ge 1 ]
done
```

- 壳：`aapt2 dump xmltree --file AndroidManifest.xml` 断言包名、`minSdk 26`、`debuggable=false`；`llvm-nm -D` 断言 `JNI_OnLoad`。
- spike 树不含与产品 `include/` 同名的文件（见 §2.2）；该断言在 `guard` job。
- 以上命令与壳断言并入 `pc-x86_64`，其失败必须让 `gate` 红。
- **变异测试**：删一条 `_Static_assert`、删一个 workspace 成员，各跑一次，CI 必须红；两次的 run id 记进台账。

## 4. M0b — 真机首帧门

前置：`M0a` 绿；`shell-apk` 启用；§2.4 的字段已落地。

| 判据 | 来源 | 期望 |
|---|---|---|
| `surface_generation` | stats | 1 |
| `presents` | stats | ≥1 |
| `pixel_rgba` | exec-out 截图，取窗口中心 | `0xff00ff00` |
| `surface_destroyed_still_submitting` | stats + logcat | 无 |

- 反证：HOME 后 `presents` 不增、不崩。
- 红路径：像素不符 → 2；destroy 后仍提交 → 2。

## 5. M1 — 标题画面

`--scenario m1`。前置：`M0b` 绿；pack 已装（`nva-pack install`，adb，dev-only）；`nva-pack verify` 退 0 且 `degraded == []`。

### 5.1 活体（guest 真在跑）

| 判据 | 来源 | 期望 |
|---|---|---|
| `guest_image_sha256` | loader 对 guest 可执行段回读后哈希 | 等于 manifest 中改写后 `main` 的哈希 |
| `guest_svc_count`（30 s 内增量） | stats | ≥50 |
| `draw_caller_in_guest` | stats（thunk 记 x30） | ≥1 |
| `presents` 单调 | stats | 递增 |
| `sticky_error` / `stub_aborts` | stats | 0（带注入块） |

### 5.2 非清屏

`draw_calls > 0`；相邻帧差分非零（容差 1/255）；与 `M0b` 的 clear 参考帧不同。

### 5.3 语义锚点

- 硬锚：RomFS 打开路径中 `Layout/` 与 `Pack/` 前缀计数 > 0；pipeline key 数 ≥ 首跑冻结值（写台账）。
- 人签：四项排除（启动 logo、淡入/黑场、错误框、文字空白），两张间隔 ≥5 s 截图，文件名带同一 `run_id`。

**红路径**：`guest_image_sha256` 不符、`guest_svc_count` 不足、`draw_caller_in_guest = 0`（回放形态）→ 2；错 build-id → 2；无 pack → 2。

## 6. M2 — 进入世界

### 6.1 M2-0 确认键

`--scenario m2-0`。`input_consumed ≥ 1`（guest 侧），带注入块。

### 6.2 M2a 世界帧

`--scenario m2a`。

- 前置子门：`codec_calls` / `movie_open` 探针（记录）；若开局必经 movie，本门阻塞并升级为缺口。
- `operator_input` 允许：按 A 进 New Game，记录时间戳。
- `render_extent` = 1280×720；`surface_size` 记录；`hud_present = true`。
- 60 s 内 `presents ≥ 120`；Perfetto 帧 p95 ≤200 ms；`dumpsys meminfo` 的 RSS ≤6 GB。
- 冻结进台账：每帧 draw 下限、Map/Pack 集合、pipeline key 集合。
- 红路径：`draw_calls` 低于下限 → 2；清 cache 后 `missing_variants > 0` 且 `draw_skipped > 0` → 2。

### 6.3 M2b 控制回路

`--scenario m2b`。overlay + `adb shell input motionevent`。

- 摇杆四向各持续 ≥3 帧；ABXY 各一次；`input_consumed` 与壳侧发送数一致。
- 位移 > 0.5 世界单位，或朝向 > 3°（首跑写台账后固定）。
- 红路径：断输入后计数不增 → 2。

## 7. M3 — 可玩

### 7.1 M3a 热稳态

`--scenario m3a`。前置：`M2a`/`M2b` 绿；`render_extent` 冻结 1280×720。

条件由 verifier 现场读，不采信 JSON 自述：

```sh
adb shell getprop persist.sys.sf.native_mode        # 分辨率/刷新
adb shell dumpsys battery | rg 'AC powered|USB powered'   # 未充电
adb shell settings get global airplane_mode_on      # 1
adb shell getprop ro.fpslock   # 无此属性；fpslock 由 App 自报 + 行为反证
```

- `duration_s ≥ 1800`；只判最后 300 s。
- 帧数覆盖：最后 300 s 内 `presents` 增量 ≥ 7500（25 fps 均值下限），与 Perfetto 帧样本数偏差 ≤1%。
- 判据：中位帧 ≤33.4 ms；1% low ≥27 fps；最长单帧 ≤100 ms；连续 ≥3 帧 >50 ms 事件 ≤2 次；每帧 draw ≥ 台账冻结的绝对下限；`missing_variants` / `draw_skipped` = 0；RSS 斜率按全部 30 min ≤50 MB/h。
- 分位数从 Perfetto 原始样本现场重算。
- 红路径：`render_extent` 变低 → 2；检测到固定节流（帧间隔方差低于阈值）→ 2。

### 7.2 M3b 系统与内容

每个场景一份 JSON：`m3b-plateau`、`-village`、`-camp`、`-shrine`、`-water`。每场景各 3 min，不承担热结论。

每场景独立判据（不共用）：

- 该场景特有的 `assets_opened` 前缀计数 > 0（证明进的是不同场景，不是同一处跑五遍）。
- 该场景 `draw_calls` ≥ 台账冻结下限。
- 像素：该场景指定采样点与期望值（首跑冻结）。
- 音频 `xruns = 0`（带注入块）；音画偏差记录。
- 存档：`kill -9` 前快照与重开后逐字节相同，且读回同一存档位；中断写入后仍可恢复。
- `surface_generation ≥ 2`；输入到 `presents` p95 ≤100 ms（同钟）。
- `stub_aborts = 0`；typed stub 只认封闭白名单，新增即红。
- SAF：1 GB 合成包导入成功（时间/空间/校验入 JSON，`operator_input` 记录）。
- 红路径：注入 surface lost、切换音频路由、中断存档。

## 8. 交叉约束

- 输入三段：确认键（`M2-0`）→ overlay 摇杆/ABXY（`M2b`）→ 手柄（M3 之后）。
- pack 上机：M1–M3 用 `nva-pack install`（adb，dev-only）；SAF 在 `M3b`。
- `nva_stats` 追加：§2.4，由实现票做；落地后回填本文件与 `abi_layout_check.c`。
- **#31**：不前置任何门；每门记 `validation_available` 与 `vk_errors`；到 M2 末未关则按 #31 路径 3 收口，附失败命令、loader 版本、logcat 哈希。
- 证据：`PIPELINE.md:49` 的公开 evidence artifact 对产品门退役，只在 spike 保留；#33 定私有根路径。

## 9. 进门时仍存在的已知差距

- 真机 Vulkan validation 不可用（#31）。
- 523 槽只验过 14 槽；per-thread host TLS 未做。
- movie/codec 能否播未证（`M2a` 前置子门）。
- 六份 `.bfttf` 已提取并记 hash，不进仓库。
- 8 MRT ≤28 ms、present 延迟、16 KB page size、`LSE` 回退未实测。
- `dm/` 与 `ci/dmgendir` 不在仓；`shell-apk` 启用需先补替身。

## 10. 变更记录

- v1：初版。
- v2：载体与退出码、M1 guest 证据、`M2-0`、720p 记录。
- v3：修成员数与 fps 口径、注入块、身份现场取。
- v4：承认「规格 ≠ 可执行物」；列出 §2.4 追加字段（ABI 追加票输入）；退出码改回 #20 语义；`pm path` 前缀、llvmpipe、spike 头改名；M1 用 `guest_svc_count ≥50` + `draw_caller_in_guest` 反回放；M3a 加帧数覆盖与现场读条件；M3b 五场景独立判据；恢复 `run_id`/`ts_utc` 与私有台账。

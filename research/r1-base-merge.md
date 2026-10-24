# #11 基座比对：三棵 dm/ 树的合并成本

**结论（one-liner）**：以 **d2r-mobile/d2r-mobile（`dev` 分支）为基座**成本最低——它是三棵树里唯一同时具备「Android 壳 + NVN→Vulkan 引擎」的；BotW 1.6.0 的 per-title 资产从 MacNativeGames 取，per-title profile 机制从 nxnative 取。成本排序：**(a) d2r 为基座 < (c) nxnative 为基座 < (b) MacNativeGames 为基座**。

## 口径与三处前提修正

分析基于三个快照（GitHub tree API，`?recursive=1`，均未截断）：

- A = `d2r-mobile/d2r-mobile` @ `dev` `a8782caa28d7b49d1b341e34de488d3168acb86c`（2026-10-24T01:39Z），2116 文件 / 28.5 MB
- B = `ly05010419/MacNativeGames` @ `main` `5281186af56bdcce6ae9b3bb9daa9791bc02994e`（2026-10-23T23:51Z），1161 文件 / 84 MB
- C = `seedds/nxnative` @ `main` `0045c1dfb639f9775891382646ad38c2a11c56bf`（2026-10-24T10:55Z），1339 文件 / 131 MB

**修正 1：B 的树里根本没有 `dm/`。** `gh api repos/ly05010419/MacNativeGames/git/trees/main?recursive=1` 的顶层是 `games/{totk,botw,odyssey,by3}`、`vendor/ryujinx`、`docs/`、`Makefile`、`build_app.py`；`dm/host|gfx|tools|android` 命中数均为 0。「三棵 `dm/` 树」这一说法只对 A、C 成立，B 是**同一血统的另一种布局**（每款游戏一棵自包含的 `runtime/`）。这直接决定了 (b) 的成本量级。

**修正 2：C 的 `dm/gfx/vk` 不是 Vulkan 后端。** 它只有一个文件 `dm/gfx/vk/hle_vk.m`（124 KB），文件头自述：「Vulkan → Metal layer for games that render with Vulkan (Hades: The Forge renderer)」；`dm/gfx/README.md:83` 同样写明它是给**用 Vulkan 渲染的游戏**翻译到 Metal，不是 NVN→Vulkan。C 的 NVN 路径只有 `nvn/nvn_mtl.m`。

**修正 3：A 的两个副本不是同一棵树。** `jhaizhou-ops/d2r-mobile@main` 只有 1318 文件、`dm/android` 63 文件；org 的 `dev` 是 2116 文件、`dm/android` 764 文件。同一棵树差 800 个文件。做快照必须钉死「org + dev + commit」，否则拿到的 Android 壳是残的。（`oneincase/MacNativeGames` 同理是 14★ fork，比上游少 17 个文件，不要用。）

## 1. 目录差异（文件数 / 体积）

| 区块 | A d2r@dev | B MacNativeGames | C nxnative |
|---|---|---|---|
| `dm/host` | 185 / 3810 KB | 0 | 81 / 1002 KB |
| `dm/gfx` | 214 / 5457 KB（`vulkan` 82、`nvn` 35、`metal` 9、`tests` 39） | 0 | 188 / 7357 KB（`tools` 91、`tests` 44、`nvn` 28，`vk` 1、`gl` 1） |
| `dm/tools` | 43 / 437 KB | 0 | 24 / 215 KB |
| `dm/android` | **764 / 5712 KB**（`tests` 331、`native` 114、`app` 74、`runtime` 52、`vulkan` 47、`java` 25、`compat` 13） | 0 | 0 |
| `games/` | 0 | **722 / 80.8 MB**（totk 317/54 MB、botw 17/8.2 MB、odyssey 217、by3 171） | 0 |
| `dm/games` | 0 | 0 | **410 / 107 MB**（54 个 profile 目录 + 5 个共享脚本/表） |
| `dm/ios` | 176 / 3888 KB | 15（在 `games/totk/ios`） | 15 / 122 KB |
| `third_party` | 423（ryujinx 363 + libdeflate 43 + flip 12） | 363（`vendor/ryujinx`）+ `games/totk/third_party/d2r_gfx` 97 | 553（ryujinx 363 + libvpx 156 + vulkan-headers 17 + flip 12） |

语义分块上，三棵树的差别不是「同一套模块各改了几行」，而是**模块归属不同**：

- A：`dm/host` 是「通用 HLE + D2R 游戏逻辑」混装——顶层 76 个 `.c` 里 40 个是 `dm_*.c`，再加 `hle_granny.c`/`hle_tact.c`/`hle_charname.c`/`hle_private_aim.c`，约 44 个是暗黑 2 专属（`dm_cursor.c` 521 KB、`dm_runeword.c`、`dm_bossrush.c`…）；此外还有 `dm/verify`(102)、`dm/harmony`(54)、`dm/tvos`(6) 三个平台侧。
- B：`games/totk/runtime` 61 文件就是它的「host」——单文件 `services.c`(34 KB) + `threads.c`(80 KB) + `loader.c`(47 KB) + `game_patches*.c`(41/72/36 KB) + `frontend.m`/`gamepad.m`(ObjC)。**没有 `hle_*` 拆分**，所以和 A/C 的文件级映射只有 `loader.c` 一个同名（还用不上）。
- C：`dm/host` 81 文件是三者中最干净的（无 `dm_*.c`），多出 A 没有的服务：`hle_ro.c`、`hle_sf.c`、`hle_audren.c`、`hle_mii.c`、`hle_offline.c`、`hle_oodle.c`、`hle_sdk16.c`、`hle_uemalloc.c`、`hle_fpslock.c`、`hle_glslc.c`。

**B 是 A 的硬分叉，且有据可查**：`games/totk/third_party/d2r_gfx` 是 A 的 `dm/gfx` 旧快照（97 文件，40 个可对上 A 的路径：15 个字节相同、25 个已改），另 57 个在 A 里没有对应物，其中 **26 个是 `totk_*` 扩展**（`totk_mfx.m`、`totk_taa.m`、`totk_vpool.c`、`totk_lifelog.c`、`totk_shader_rt.m`…）。同时它只 fork 了 `nvn`+`tex`，没有带走 `metal/` 与 `vulkan/`；连 A 的三个接口头都被削成垫片（`d2r_gfx/host/dm.h` 824 B vs A 的 14115 B、`front.h` 135 B vs 6021 B、`hle.h` 332 B vs 13662 B）——所以这份拷贝不能直接搬回 A，必须恢复接口。

## 2. 各自相对 d2r 的独有资产

**B（MacNativeGames）——BotW/TotK 的「资产」全在这里**

- BotW 1.6.0 profile：`games/botw/assets/expected-v1.6.0.json`（2.3 KB）。内容是校验用真值：`exefs_sha256`（main/main.npdm/rtld/sdk/subsdk0 五个模块）、**`rewritten_sha256`（三个改写后模块的哈希）**、`romfs.sha256`+`size` 15131979952+41052 文件、两个 AOC 包哈希、六个共享字体哈希。1.9.0 另有一份。
- BotW shader 清单：`games/botw/assets/shader_list.tsv` 8.3 MB；TotK 同名文件 **43.9 MB**。这是**原始 dump 清单**，不是翻译产物。
- BotW/TotK 的 NSO 改写补丁：`games/totk/runtime/game_patches_botw.c`(72 KB)、`game_patches_v143.c`(36 KB)、`game_patches.c`(41 KB)——BotW 的代码支持**寄生在 totk 树里**，`games/botw/` 只有资产 + 14 个脚本，没有 runtime。
- 工具链：`build_app.py`（入口，见 §4）、`games/totk/tools/totk_xlate/`（C# SPIR-V 工具 + 5 个补丁 + 7.8 MB 预编译 `prebuilt/macos-arm64/totk_xlate`）、`games/totk/scripts/` 74 个脚本、`games/totk/ios/tools/build_ipa.py`(16.7 KB)、`docs/` 下 15 篇笔记（含 `v1.2.1-port.md`/`v1.4.3-port.md`/`botw-v1.9.0-port.md` 三篇移植记录）。

**C（nxnative）——可复用的「profile 工厂」**

- `dm/games/<title id>/` 54 套：每套 `game.tsv`(3.2 KB，键值：title_id/program_id/name/modules/package/…)、`patches.py`(10 KB，**按 NSO build ID 索引的字节级补丁**：`CUTS = {'cd57b23fa4bbad65803d9788c01821ee': [(0x112670c,'6041201e','4041201e','...')]}`)、`shader_map.tsv`(3.3 MB)、`shader_pairs.tsv`(680 KB)、`pso_manifest.tsv`。另有 `dm/tools/sdk_patches.py`（SDK 模块补丁，与游戏补丁分离）与 `dm/games/profile_guess.py`。
- 注意：C 的 BotW profile 是 **1.9.0（v1114112）**，`game.tsv` 开头写死「base game v0 + update 1.9.0」，补丁 build ID 也是 1.9.0 的。**它没有 BotW 1.6.0。**
- 工具：`patch_nso.py`(82 KB)、`nso2elf.py`/`nso_fnmap.py`/`nso_syms.py`/`nso_code.py` 一套、`sdk_patches.py`、`nx_extract`(C#/LibHac)、`scripts/build_ipa_from_nsp.py`。

**A（d2r）——平台层独有**

- Android 壳 `dm/android` 764 文件：`native/compat/` 13 个 Apple→NDK 垫片（`mach/mach.h`、`CommonCrypto/CommonDigest.h`、`libkern/OSCacheControl.h`、`pthread/qos.h`…）、`native/frontend_activity.c`(120 KB)、`vulkan_present.cpp`、`tools/asm_elf.py`（Mach-O 汇编→ELF）、`tools/host_sources.py`（解析 `dm/host/Makefile` 的 `SRC` 生成源清单）、`tools/build.py`(27.6 KB)、`tools/shell_apk.sh`、`vulkan/CMakeLists.txt`(47.6 KB)、`docker/`。
- Vulkan 引擎 `dm/gfx/vulkan` 82 文件：`engine/xbg_game.cpp`(467 KB)、`xbg_vulkan.cpp`(38 KB)、`xbg_spirv_{quad,warp,ldc,temps,raster}.cpp`、`xbg_texture.c`、`nvn_vk.c`、`INTEGRATION.md`，`engine/SOURCE.json` 记录来源。
- 其它：`dm/verify`(102，含 `equiv.py`/`structck.py`/`nsoimg.py`，B 也 vendored 了其中 `equiv.py`/`structck.py`/`nsoimg.py` 三个)、`dm/harmony`、`docs/android`(53)+`docs/android-notes`(34) 真机验收记录、`dm/tools/d2rm_android.py` + `patch_nso_runtime_android.py`。

## 3. 三个方案的成本排序

**同名文件被两边改过的确切数字**（blob sha 比对，非行数）：

- A∩C 共有 528 条路径，**370 条字节相同**（其中 351 条是 `dm/third_party/ryujinx`——A↔B 的 363 个 ryujinx 文件字节全同，A↔C 有 12 个不同，三方共用同一份 vendored 快照，可整体固定），158 条双边都改过：`dm/gfx` 82、`dm/host` 50、`dm/ios` 7、`dm/tools` 2、`dm/third_party` 13。
- **`dm/host` 里 51 个同名文件只有 1 个还相同**——也就是 HLE 层几乎每个文件都是双向分叉。热文件：`nvn_mtl.m` A 669 KB / C 848 KB / B 1458 KB；`hle_nvn.c` 160 / 229 / 255 KB；`hle_fs.c` 77 / 70；`hle_thread.c` 59 / 75；`hle_movie.c` 39 / 65；`dmsh_build.py` 43 / 81；`patch_nso.py` 99 / 82 / 97。
- A∩B 只有 1 个同名（`loader.c`）；真正的冲突在 B 内嵌的 `d2r_gfx` 那 40 个。

**(a) A 为基座 + 移植 BotW/TotK 资产 —— 成本最低**

理由：两个最难、最平台特异、也最不可「复制粘贴」的层只在 A 存在——764 文件的 Android 壳（含 Apple→NDK 垫片、Mach-O→ELF 汇编转换、SurfaceView/Swapchain/AAudio/触摸叠层/生命周期）和 82 文件的 Vulkan 引擎。这两层「删」不掉也「写」不快。需要付的代价是：剔除 A 的 D2R 专属（40 个 `dm_*.c` + `hle_granny`/`hle_tact`/`hle_charname` + `dm/PRODUCT` 产品码），并补 BotW 需要的服务（`ro`/`sf`/`audren`/`sdk16`/`oodle` 在 C 里现成）。主要冲突点：`hle_fs.c`/`hle_thread.c`/`hle_movie.c`/`nvn_mtl.m` 与 C 的 158 条分叉，只能整文件取舍，不要试图三方合并。

**(c) C 为基座 —— 次之**

理由：HLE 最干净（无游戏逻辑）、profile 机制最成熟（54 套、按 build ID 打补丁、`sdk_patches` 分离），加一款新游戏≈加一个目录。但 Android 与 NVN→Vulkan **两层都要从零写**；`vk/hle_vk.m` 帮不上忙（Vulkan→Metal）；且 BotW 只有 1.9.0，要回到 1.6.0 得重做补丁与哈希（B 的 `expected-v1.6.0.json` 可当参照）。相比 (b)，它省下的是「把 monolith 拆回接口」这一大块。

**(b) B 为基座 —— 最贵**

理由：B 的强项只有「BotW 1.6.0 已经能在 Apple 上跑通」；但它的目录结构（每游戏一棵 `runtime/`、无 `hle_*` 拆分）、GFX 层（`nvn_mtl.m` 1.46 MB，A 的 2.2 倍，含 26 个 TotK 硬编码扩展）、和 100% 的 Objective-C 前端（`frontend.m`/`gamepad.m`）都需要先拆解；而 Android 壳 + Vulkan 引擎要从零重建。等于「先重组、再补两层」。

## 4. 每个方案「第一次编译出 APK」的最小步骤

**(a) A 为基座（唯一今天就能出 APK 的路径）**

1. 取快照 `dev@a8782caa`。
2. 满足 CMake 的硬前置：`dm/android/CMakeLists.txt` 里 `if(NOT DM_GEN_DIR OR NOT EXISTS ${DM_GEN_DIR}/dm_vault_tab.h) message(FATAL_ERROR ...)`——必须提供 `dm_vault_tab.h`/`dm_hud_exp.h`/`dm_bright_levels.inc`。这三个是 **D2R 专属**表，BotW 下必须先出空壳或等价物。
3. 注意 `dm/android/build.sh` **不是** APK 入口：它只以 `-DDM_ANDROID_CHECKS=ON` 交叉编译设备探针（`ANDROID_PLATFORM=android-29`、`ANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES=ON`），不读游戏文件。出 APK 用：`gradle -p dm/android :app:assembleDebug -PdmGenDir=<dir> -PdmPython=<python>`（README：JDK 17+、Gradle 9.8.1、SDK 36、NDK 29.0.14206865、CMake 3.31.6），产物 `dm/android/app/build/outputs/apk/debug/app-debug.apk`；Linux/WSL 可走 `docker build -t dm-android-build:1 dm/android/docker` + `DM_GEN_DIR=… dm/android/docker/build.sh`。
4. 依赖自动拉取并校验哈希：lz4 v1.10.0、mbedtls v3.6.4（`dependencies.cmake`，SHA256 固定）。
5. **macOS 专属**：`dm/host/Makefile` 的 `-mmacosx-version-min`/`app_mac.m`/`front_mac.m`/`*.m`（Android CMake 只把该 Makefile 当源清单解析，不执行）、`dm/tools/mac_app.sh`、`dm/ios/*`、`dm/gfx/metal/*`。Android 侧**不缺结构**，缺内容：BotW 1.6.0 profile、`shader_list`、以及 26 个 `totk_*` 扩展在 Vulkan 上的实现。

**(b) B 为基座**

1. B 没有 `make all`——根 `Makefile` 只有 `check`（单测门禁）。构建入口是 `python3 build_app.py <game.xci|nsp> -k <prod.keys> --platform macos|ios|visionos [--update …] [--shared-fonts …]`；BotW 1.6.0 走「一张卡带含本体+更新+DLC + `--shared-fonts`」。
2. iOS 阶段由 `games/totk/ios/tools/build_ipa.py` 驱动，需 Xcode 26+、xcodegen、CMake、联网拉设备库（脚本自述）。
3. 到 APK 的缺口：Gradle/NDK/JNI/SurfaceView/Swapchain/AAudio 全无（tree 里 `android|gradle|jni` 命中 0）、Vulkan 后端 0 文件、`entry.S`/`gate.S`/`hle_stack.S` 是 Mach-O 汇编（需 A 的 `asm_elf.py` 类工具）、前端是 `Foundation`/`MetalFX`/`AVFoundation`。等价于把 (a) 的平台层重写一遍再接上 monolith。

**(c) C 为基座**

1. 出 iOS 包：`GAME=<title id> W=<work 目录> dm/ios/tools/build_ipa.sh`（xcodegen + xcodebuild；`GFXAPI=nvn|vulkan`）；前置是 `dm/tools/patch_nso.py` 生成的 `$PATCHED/<mod>.patched.bin` 与 `$MSL_IOS` shader 库。
2. 到 APK 的缺口：与 (b) 同——Android 层 0 文件，`vk`/`gl` 是 Metal 方向。比 (b) 少的是「拆 monolith」：C 的 host 已经是接口化的，A 的 `native/compat` + `asm_elf.py` + `vulkan_present` 理论上可以成块搬过来，对接点就是 `dm/host/Makefile` 的 `SRC`（A 的 CMake 正是靠解析它拿源清单）。

## 最大不确定性

- **A 的 Vulkan 引擎对 BotW 所需 NVN 调用面的覆盖度未核实**。只读了 `xbg_game.cpp`/`xbg_nvn.cpp` 的头部：后者确实是通用 NVN→Vk 映射（viewport/sampler/blend 参数翻译），并与 `dm/gfx/nvn/*` 共享头文件、对 `dm/host/dm_bright.c` 用 weak 符号——耦合很轻，但 D2R 用到的 NVN 子集与 BotW/TotK 不是同一套（B 为此加了 26 个 `totk_*` 文件）。这项决定 (a) 的真实工期，`UNVERIFIED`。
- B 的 BotW runtime 与 A 的 `dm/host` 语义差异无法从文件层面判定（文件名不重合，`loader.c` 也不同源），只能靠读代码。

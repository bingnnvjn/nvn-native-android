# R5 · 构建链：在 GitHub Actions 上编译 d2r 血统 APK（issue #15）

**结论：有条件可行。** 原生交叉编译这一段，d2r 上游自己在 ubuntu-24.04 上跑通了；缺的是「Gradle 出 APK」这一环，而它被一个具体输入卡住：CMake 要求 `-DDM_GEN_DIR`，里面三份「从游戏文件生成」的表头加一份触屏素材索引（`shell_apk.sh` 语）。上游对这三份文件的定性是「替身里只有表的条数（数组大小），条数随游戏版本固定，不是游戏内容」，所以有条件把壳的编译搬进 CI。

一手来源：`d2r-mobile/d2r-mobile`（MIT，size 11343 KB，default_branch **`dev`**——不是 `main`；`gh api repos/d2r-mobile/d2r-mobile`），下文路径均指该仓 `dev`；CI 侧事实来自 `actions/runner-images` 的 ubuntu-24.04 镜像清单。

## 1. Android 编译的硬需求

**版本钉**（`docs/DEPENDENCIES.md` "Additionally for Android" 表 + `dm/android/README.md` "Build locally"，两处一致）：JDK 17 或更新、Gradle 9.8.1、Android SDK 36、NDK 29.0.14206865、CMake 3.31.6、Python 3。minSdk 31 / targetSdk 36 / compileSdk 36，只出 `arm64-v8a`。

**`dm/android/app/build.gradle`** 的实际内容（关键项）：`plugins { id 'com.android.application' }`；`ndkVersion '29.0.14206865'`；`externalNativeBuild { cmake { path '../CMakeLists.txt'; version '3.31.6' } }`；`externalNativeBuild.cmake.arguments` 里塞 `-DDM_GEN_DIR=<dmGenDir>`、`-DPython3_EXECUTABLE=<dmPython>`，`targets 'dm'`；`ndk { abiFilters 'arm64-v8a' }`；`packaging { jniLibs { useLegacyPackaging true; keepDebugSymbols += ['**/*.so'] } }`。签名从环境变量 `DM_ANDROID_KEYSTORE` / `_KEY_PASS` / `_KEY_ALIAS` 读，**没设就不签**——CI 出 debug 包不需要任何密钥。插件版本在 `dm/android/settings.gradle`：`com.android.application` **9.4.1**，仓库**没有** `gradle/wrapper`（tree 里无 `gradle-wrapper.properties`），所以必须靠宿主机的 `gradle` 9.8.1。

**`dm/android/CMakeLists.txt`**：`cmake_minimum_required(VERSION 3.22)`；第 40–41 行 `if(NOT DM_GEN_DIR OR NOT EXISTS ${DM_GEN_DIR}/dm_vault_tab.h) message(FATAL_ERROR ...)`——这就是全链唯一硬门（第 98 行还有第二道：缺 `dm_touch_native_art.c` 亦 FATAL_ERROR）；`DM_ANDROID_CHECKS=ON` 时第 15–19 行 `include(checks.cmake)` 后 **return**，整棵 App 树不建。产物两个：`dmhost`（命令行）与 `libdm.so`（JNI 宿主，`-Dmain=dm_main`）。源清单不手抄，从 `dm/host/Makefile` 的 `SRC/GFX/LIBDEFLATE` 解析。

**Vulkan headers**：仓库里**没有** vendored Vulkan-Headers，只有 `dm/android/test-volk/volk.{h,c}`（裁剪版 volk，`dm_test_volk` 目标），`dm/gfx/vulkan/CMakeLists.txt` 只 `target_link_libraries(... log)`（Android liblog）。因此 `vulkan/vulkan.h` 来自 **NDK sysroot**（推断，仓库内无其它候选；`UNVERIFIED` 到具体头文件路径）。

**其它外部代码**：`dm/android/dependencies.cmake` 用 FetchContent 钉两个归档，**带 SHA256**：lz4 `v1.10.0`（`5375129…`）与 mbedtls `v3.6.4`（`a1e01f6…`），只编 NSO 解压/校验需要的几个 .c。Python 在构建期不是可选：CMake `find_package(Python3 REQUIRED)`，跑 `dm/ios/tools/overlay_contract.py` 生成 `dm_overlay_contract.h`，Gradle 侧跑 `dm/tools/gen_strings.py`。

**CI 侧镜像事实**（[Ubuntu2404-Readme.md](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md)，`ubuntu-latest` 即 24.04，见 [runner-images README](https://github.com/actions/runner-images/blob/main/README.md) 第 25 行）：NDK 已装 `27.3.13750724`(默认) / **`29.0.14206865`** / `30.0.16248370`；SDK CMake 只有 `3.31.5` 和 `4.1.2`，但系统 CMake 是 **3.31.6**；Gradle **9.8.1** 预装；默认 JDK 17；Python 3.12.3；`ANDROID_HOME=/usr/local/lib/android/sdk`。→ NDK 那一项不用下载，但 `ANDROID_NDK_HOME` 默认指向 r27.3，必须显式改写。`cmake;3.31.6` **不在**镜像的 SDK 里（只有 3.31.5 / 4.1.2），而 AGP 的 `externalNativeBuild.cmake.version` 只从 SDK 解析，所以 CI 必须补装一次——这件事上游自己的容器已经替我们验证过：`dm/android/docker/Dockerfile` 的 `ARG SDK_PACKAGES="platforms;android-36 build-tools;36.0.0 ndk;29.0.14206865 cmake;3.31.6"`，两个精确版本包名都由 `sdkmanager` 按仓库清单 SHA-1 校验安装。该 Dockerfile 还把 Ubuntu 基础镜像钉到 digest（`ubuntu@sha256:534baea6…`）、Gradle 钉到 `GRADLE_SHA256`，是 §4 的现成范式。

## 2. 把 d2r 的构建路径拆成「CI 编 APK」+「本机出 import pack」

**上游路径原文**（`dm/android/README.md`）：`gradle -p dm/android :app:assembleDebug :app:lintDebug -PdmGenDir=<私有生成头目录> -PdmPython=<python>`，产物 `dm/android/app/build/outputs/apk/debug/app-debug.apk`；另给 `dm/android/docker/` 容器路径。**macOS 专属的只是默认值**：`shell_apk.sh` 里 `JAVA_HOME` 缺省 `$(brew --prefix openjdk@17)`、`ANDROID_HOME` 缺省 `~/Library/Android/sdk`、`PY` 缺省 `~/dm-work/build/cache/dm/venv/bin/python`、`ROMFS` 缺省 `~/dm-work/data/switch/3.3.94012/Update/RomFS`——全部可被环境变量覆盖，无一项是 OS 能力依赖。文件头也明说「移植作者在 Linux / WSL2 上构建和测试」。真正**依赖本地游戏数据**的是四件生成物：`dm_vault_tab.h`、`dm_hud_exp.h`、`dm_bright_levels.inc`、`dm_touch_native_art.c`，以及 6,927 个 `.xgs` 着色器包与 CASC 覆盖层。

**拆分点就在 `dm/android/tools/shell_apk.sh`**。它做的正是「无游戏数据 APK」：用本机 `gen_vault_table.py` / `gen_hud_exp.py` / `gen_bright_levels.py` / `native_art/generate.py` 出**真表**，然后 `dm/ios/tools/shell_stubs.py data-gen … stubs-gen` 把真表拆成 `derived.tsv`（游戏数据，随导入包走）+ 同名的**替身源文件**（只有数组大小和「从 derived.tsv 填表」的几行）。它给出的阻塞理由很具体：「出包这台机器要有游戏文件：要用真实的生成结果定表的大小」。而 `shell_stubs.py` 文件头把这条边界写死了：「表的条数会写进替身（数组大小）：条数随游戏版本固定，**不是游戏内容**」。

因此拆法：

- **本机（Windows + WSL，有游戏文件）**：跑一次 `shell_stubs.py` 产出替身三件套 + `dm_touch_native_art.c`，把替身目录当 `DM_GEN_DIR` 的固定输入沉淀下来（入库到我们仓，例如 `ci/dmgendir/`，因为按上游定义它只含条数）；同时用 `d2rm_android.py` 产出 import pack。`derived.tsv`、着色器包、覆盖层**永不入库**。
- **CI（无游戏文件）**：`gradle -p dm/android :app:assembleDebug -PdmGenDir=ci/dmgendir` → 壳 APK，然后用 `shell_apk.sh` 尾部那套自检做守门（APK 里不得出现真表字符串、不得有 `.xgs`/`assets/`/`overlay`/`derived.tsv`、不得含本机路径）。
- 风险与备选：替身仍是派生数据，且有「随游戏版本固定」的耦合——游戏版本一变就要重生成一次。备选是把 `DM_GEN_DIR` 改成可空 + CMake 自造零长表（要改上游代码，且运行时读表逻辑 `dm/host/dm_derived_rt.h` 是否有界要另行确认，`UNVERIFIED`）。

**`d2rm_android.py` 需要的输入**（文件头）：唯一位置参数是游戏目录 `<Update 目录>`，要求存在 `ExeFS/main`、`ExeFS/sdk`、`RomFS/data/build_manifest.psv`（Switch 版 3.3.94012）；第二参数是输出目录；缓存目录 `DM_ROOT`（缺省 `~/dm-work`）。工具链要求：Python 3.9+ 及 `pillow numpy lz4 capstone unicorn`、**.NET SDK**（编 `dmsh_xlate` 着色器翻译器）、`spirv-opt`（必须带 `--trim-capabilities`，NDK 自带那份太旧）、`spirv-val`、`glslc`。输出 `<输出目录>/android/` 下 `out/`、`shaders/`、`overlay/`、`derived.tsv`、`pack_info.txt`；`pack_info.txt` 里的产品代号必须等于 App 编进去的 `dm/PRODUCT` 首行（`build.gradle` 的 `DM_PRODUCT`）。

## 3. 最小可用 workflow

放在我们仓 `.github/workflows/android-apk.yml`（上游 `.github/workflows/ci.yml` 只有 `ci-metadata` / `java-boundary` / `host-x86_64` / `host-arm64`(ubuntu-24.04-arm) / `android-arm64-cross`，**没有** Gradle、没有 APK 产物）。注意上游 `ci.sh android` 走的是 `DM_ANDROID_CHECKS=ON`，因 CMakeLists 提前 return，**它只编 checks.cmake 里的测试目标，从不链接 `libdm.so`**——「上游 CI 证明原生交叉编译可跑」这句话必须打折。

```yaml
name: android-apk
on: { push: { branches: [main] }, pull_request: , workflow_dispatch: }
permissions: { contents: read }
jobs:
  apk:
    runs-on: ubuntu-24.04        # ubuntu-latest 当前即 24.04
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2，与 ci/actions.lock 一致
      - uses: actions/setup-java@v4
        with: { distribution: temurin, java-version: '17' }
      - name: Pin NDK + CMake (same exact packages as dm/android/docker/Dockerfile)
        run: |
          yes | sdkmanager --sdk_root=$ANDROID_HOME \
            "platforms;android-36" "build-tools;36.0.0" "ndk;29.0.14206865" "cmake;3.31.6"
          echo "ANDROID_NDK_HOME=$ANDROID_HOME/ndk/29.0.14206865" >> $GITHUB_ENV
          echo "ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/29.0.14206865" >> $GITHUB_ENV
      - name: Cache FetchContent + Gradle
        uses: actions/cache@0c907a75c2c80ebcb7f088228285e798b750cf8f     # v4.2.1
        with:
          path: |
            ~/.gradle/caches
            ~/.gradle/wrapper
            ${{ runner.temp }}/dm-ci/fetchcontent-x86_64
          key: apk-${{ hashFiles('dm/android/dependencies.cmake','dm/android/CMakeLists.txt','dm/android/*/CMakeLists.txt','dm/android/**/build.gradle') }}
      - name: Native unit tests (host CTest)
        env: { DM_CI_WORK: '${{ runner.temp }}/dm-ci' }
        run: bash ci/scripts/ci.sh host
      - name: Build APK (game-free shell)
        run: |
          gradle --no-daemon -p dm/android :app:assembleDebug :app:lintDebug \
            -PdmGenDir=ci/dmgendir -PdmPython=$(command -v python3)
      - name: Guard — no game content in the APK
        run: bash ci/tests/apk-no-game-content.sh dm/android/app/build/outputs/apk/debug/app-debug.apk
      - uses: actions/upload-artifact@b4b15b8c7c6ac21ea08fcf65892d2ee8f75cf882  # v4.4.3
        with: { name: dm-android-apk, path: dm/android/app/build/outputs/apk/debug/app-debug.apk, retention-days: 14 }
```

**预计耗时与体积**（`ESTIMATE`，上游未公布计时）：冷缓存 15–25 min（Gradle/AGP 首次下载、FetchContent 拉 lz4+mbedtls、checks 全量编译），热缓存 8–12 min；上游给 android 交叉任务的 `timeout-minutes` 是 45，留足余量。产物体积：`shell_apk.sh` 注释里出现「25 MB 的内容出了 46 MB」，说明壳 APK 内容量级 **~25 MB**，`libdm.so` 因 `keepDebugSymbols` 占大头（`UNVERIFIED` 精确数字，需实测）；仓库本身 11.1 MB，`.so` 远大于源码。

## 4. 需要 pin 的外部依赖与许可证归档

**上游已有的两套机制，直接沿用**：

1. **源码快照目录 + 各自 LICENSE**：`dm/third_party/` 下 `bcdec` / `flip` / `libdeflate` / `ryujinx`，README 明说「随仓发布，无需下载，每个组件保留原许可证，许可证文件就在它自己的目录里」。Ryujinx 是 **1.3.2 的三个工程**（`Ryujinx.Graphics.Shader`、`Ryujinx.Common`、`Spv.Generator`，401 个文件，MIT，上游记作 `https://github.com/Ryubing`），只被 `dm/gfx/tools/dmsh_xlate` 这个**本机工具**引用，许可证在 `dm/third_party/ryujinx/LICENSE.txt`（子项目 `Spv.Generator` 另有自己的 LICENSE）。注意顺序：**它不进 APK**，它进的是 import pack 里那 6,927 个着色器包的生成链路——所以「Ryujinx 快照」是**本机依赖**，不是 CI 依赖。
2. **构建期归档 + SHA256**：`dependencies.cmake` 的 lz4 / mbedtls 是唯一在 CI 真正下载的东西，`URL_HASH` 已经把版本钉死。GitHub 侧还有 `ci/actions.lock`（checkout v4.2.2 / cache v4.2.1 / upload-artifact v4.4.3 的完整 SHA，注明用 `git ls-remote` 在 2026-10-18 核过），这三个 action 在 workflow 里一律写 SHA 而不是 tag（上面 YAML 已照此写；`setup-java` 上游没钉，我们自己要用 SHA，别跟 v4 标签）。

**Turnip / SPIRV-Cross 的实际情况与票面假设不同**：

- **Turnip 是「预编译资产」，不是源码依赖，而且它已经在构建链里**。仓库里没有 Turnip 源，也没有 `dm/android/prebuilt/turnip/`（该目录未入库，需本地放 `vulkan.purple.so`）；`-PdmTurnipAssets=<dir>` 把它当 `assets` 打进 APK。但它旁边还有一条**钉死 commit 的加载器构建链**：`dm/android/tools/build_gpu_driver.py` 里 `PINNED = (("eden-emulator/libadrenotools","8ba23b42d7"), ("bylaws/liblinkernsbypass","aa3975893d"))`，产出 `libadrenotools.so` / `libhook_impl.so` / `libmain_hook.so` / `libfile_redirect_hook.so` / `libgsl_alloc_hook.so`（arm64-v8a、API29、`-Wl,-z,max-page-size=16384`、静态 libstdc++）。也就是说：**加载器可从源码重建并 pin，Turnip 本体不能**——它是一份来源不受控的二进制。结论：第一版 CI 不做 Turnip；要做的话 pin 规则是「加载器两个 commit + Turnip 的 .so 自带 SHA256 清单」，许可证得人工确认（参考源 `K11MCH1/AdrenoToolsDrivers` 的 API `license` 字段为 `null`，`gh api` 实测，即无可自动识别的许可证；Mesa Turnip 本体是 MIT，`UNVERIFIED` 到该仓的打包声明）。
- **SPIRV-Cross：全仓 grep `spirv-cross|spirv_cross` 命中 0 条**。它只在 `docs/DEPENDENCIES.md` 里作为 macOS 构建工具出现（第 23 行「Turning translated SPIR-V into Metal Shading Language」，装法 `brew install spirv-cross`，版本 1.4.357），用途是 iOS/macOS 的 Metal 后端。Android 侧走 vendored Ryujinx 的 C# SPIR-V 后端（`dm/third_party/ryujinx/src/…/CodeGen/Spirv/` + `Spv.Generator`）经 `dmsh_xlate` 出 SPIR-V——而 `dmsh_xlate` 只在**本机**跑（`d2rm_android.py` 里 `dotnet publish`），CI 编 APK 完全碰不到它。若将来引入，许可证是 **Apache-2.0**（`gh api repos/KhronosGroup/SPIRV-Cross` 实测 `license.spdx_id`）。

**子模块 vs 快照目录：选快照。** 硬理由有两条，都不是风格问题：其一，上游仓库 `gh api repos/d2r-mobile/d2r-mobile/git/trees/HEAD?recursive=1` 的 2,382 条路径里**没有任何 `.gitmodules`**，d2r 自己就是快照派；其二，Ryujinx 上游已 404（`gh api repos/Ryujinx/Ryujinx` → `Not Found`），子模块指针会指向一个不存在的地址，而快照目录 + LICENSE 在 DMCA/下架后仍可构建。这与既有调研 §6「不要依赖活的上游 URL，vendor + pin hash」一致。

**许可证归档方式（建议照抄并补齐）**：每个第三方一个目录，`LICENSE` 与代码同处（`dm/third_party/<pkg>/LICENSE`）；`docs/DEPENDENCIES.md` 一张总表（名称 / 版本 / 用途 / 上游 URL / 许可证 / 安装方式），我们是 MIT 公开仓，这张表就是 NOTICE 的替身；对「只留了哈希、没进仓」的依赖（lz4、mbedtls）在 CI 里把 `LICENSE` 从 FetchContent 缓存目录拷进 artifact，上游 `dependencies.cmake` 注释已写「许可文本随构建缓存保留」；对每个快照补一份 `provenance.json`（`dm/gfx/vulkan/engine/third_party/bcdec/provenance.json` 是现成范式：上游 URL + 版本/commit + 取用范围 + 改没改）。

**给 #15 的三条硬约束**：新仓需要自带 `gradle` 版本声明（无 wrapper，靠 runner 的 9.8.1，或用 `gradle/actions/setup-gradle` 固定）；`ci/dmgendir/` 的替身三件套必须配一个「只含条数」的守门测试，防止有人把真表提进来；`ubuntu-24.04-arm` 可用（上游 `host-arm64` 已在用），但 Android 交叉仍应固定在 x86_64 runner 上——`ci/scripts/ci.sh` 里有一句硬拒绝：`pinned NDK r29 Linux package targets x86_64; refusing $arch runner`（除非 `DM_CI_ALLOW_ARM_CROSS=1`）。

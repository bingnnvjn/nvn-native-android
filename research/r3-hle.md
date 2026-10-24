# BotW 1.6.0 Android 原生运行的 HLE 服务清单与差集

> 路线：NSO 重写 + Horizon 服务 HLE + NVN→Vulkan。基座 `d2r-mobile/d2r-mobile@dev`，资产参照 `ly05010419/MacNativeGames`、`seedds/nxnative`。

## 1. BotW 1.6.0 服务清单

**关键结构事实**：MacNativeGames 里**没有** `games/botw/runtime/`。BotW 复用 TotK 运行时，靠编译开关切版本（`-DGAME_BOTW160`；1.9.0 是 `make runtime-botw190` + `GAME_BOTW=1`，见 `docs/botw-v1.9.0-port.md` L39）。所以服务实现全在 `games/totk/runtime/`（61 文件），BotW 差异只在 `game_version.h`（1.6.0 块 L7–L22：`TOTK_GAME_DIR "botw160"`、`nn::swkbd::ShowKeyboard` 的 PLT/GOT 地址）和 `game_patches_botw.c`（994 行，`#ifdef GAME_BOTW160` / `#else` 1.9.0 分支）。

标题 ID：base `01007EF00011E000`、update `01007EF00011E800`、DLC `01007EF00011F001`/`…F002`（同上文档 L15–L16）。

15 项服务逐条（Mac = `github.com/ly05010419/MacNativeGames/blob/HEAD/games/totk/runtime/<file>`）：

- **fs** 已有 — `fs_host.c`（`mount_save`/`mount_save_ro`/`mount_cache`/`create_cache`/`save_touched`）。挂载点、errno 映射都写了。
- **hid** 已有 — `hid_feedback.c`、`input.c`、`gamepad.m`、`controller_map.h`。
- **audout** 已有（分界 UNVERIFIED）— `audio_mixer.c` 输出侧，host 设备走 `frontend.m` 的 CoreAudio。
- **audren** 已有 — `audio_renderer.c` + `audio_effects.c`/`audio_reverb.c`（自建混音与混响，非官方 lib）。
- **vi** 已有 — `gfx_bridge.c` + `frontend.m`（Metal）+ `vpool_probe.m`。
- **oe** 部分 — `applet_host.c`、`settings_service.c`；`nn::oe::SetFocusHandlingMode` 只靠 `game_version.h` 的返回地址点补（`GAME_ADDR_0xe995c4`），无完整 oe 会话。
- **ro** 仅 stub — 无 `ro` 文件；共享字体改走 `shared_font.c`，`ro` 的 NCA 读取未实现。
- **sf** 未覆盖 — 无独立实现，靠 `ipc.c` 的通用 named-port dispatch 兜底。
- **am** 部分 — `applet_host.c`（applet 生命周期）、`account.c`（单本地用户）。
- **codec** 仅 stub — `opus_codec.c` 只有 Opus 软解；硬件 codec（H.264/VP9）无，播片走 `movie_host.c` 自带解码。
- **swkbd** 已有 — `swkbd_host.c`（1.6.0 地址由 `game_version.h` 定位）。
- **nvdrv** 部分 — 无 ioctl 级实现，`gfx_bridge.c` 直桥 NVN 调用；`/dev/nvhost-*` 语义未建模。
- **sdk** 部分 — `loader.c` + `gate.S`/`hle_stack.S` 做 x18/svc 重写与 SDK init，无 `sdk16` 版本层。
- **pctl** 未覆盖 — 无文件（BotW 不查家长控制，风险低）。
- **nifm** 未覆盖 — 无文件（离线可接受）。

**必需服务数：15 项命名服务**；加上 sm（`ipc.c` 的 `sm` 会话 + `ConnectToNamedPort`）与 account/settings 三个隐式项，实际要落 18 个接口面。

## 2. Android 替换点

d2r 的 Android 端是现成样板（`github.com/d2r-mobile/d2r-mobile/blob/dev/dm/android/app/`），每行给"从哪切、换什么、风险"：

- **音频：CoreAudio → AAudio/Oboe**。切点 `games/totk/runtime/audio_renderer.c` 的设备回调；d2r 已有 `dm/android/app/audio_output.c`、`audio_session.c`、Java `java/.../AudioFocusJniBridge.java`。风险：audren 按帧（约 5ms/48kHz）出数据，AAudio 的 burst 不可控 → 欠载爆音；5.1 需自行下混成立体声。
- **输入：hid 状态 → AInputEvent/SDL3**。切点 `hid_feedback.c`/`input.c`；d2r 有 `input_source.c`、`gamepad_events.c`、`java/.../PadInput.java`。风险：`motion_fusion.c` 的陀螺仪需 SensorManager；触屏摇杆与 amiibo NFC、键盘 IME 抢焦点。
- **资产：romfs → AAssetManager**。切点 `romfs.c`/`fs_host.c` 的 `host_path`；风险：>2GB 资产不能塞进 APK 单文件（需 AAB asset pack）；`AAsset` 只给 fd 不给 mmap，`romfs_validate` 的哈希校验要么流式读要么初始化多花十几秒。
- **存档：posix 路径 → SAF 或私有目录**。切点 `fs_host.c` 的 `mount_save` 全用 `fopen` 路径。风险：SAF 无 POSIX 路径、无原子 rename，事务性提交（见 §3）直接崩；**建议走 `getFilesDir`，绕开 SAF**。
- **Vulkan WSI ← Metal**。切点 `gfx_bridge.c`；d2r 有 `wsi_bridge.c`、`wsi_frame_result.c`、`present_policy.c`、`lifecycle.c`。风险：surface 生命周期（`onPause` 后 swapchain 必须重建，`wsi_bridge.c` 已处理 D2R 场景，NVN 的 present 语义更严）；Maxwell→Vulkan 的 transform feedback、ASTC 边界行为。
- **`/dev/nvhost-*` 抽象**。切点 `gfx_bridge.c`/`vpool_probe.m`。Android 无该字符设备，必须把 nvdrv ioctl 命令号翻译成 Vulkan 对象表 + 自建 syncpoint/fence。风险：**这是最大缺口**——nvmap 的物理页映射无法映射到 Vulkan 内存类型，`vpool_probe.m` 那套 Metal 缓冲别名的绕法在 Vulkan 上要重做。

## 3. BotW 专属坑

- **事务性存档**。`fs_host.c` 用 `save_touched`/`save_change_path` 做写入跟踪，但 Mac 端是"直接写 + 事后 cache"。BotW 1.6.0 一次存档写上百个小文件（`GameData`、`Slot_0.sav`、`CaptionMsg`），写到一半被杀 = 坏档。nxnative 为此单列了 `dm/host/tests/t_fs_commit_list.c`。Android 低内存杀进程概率高，必须做 commit-list（写临时目录 + 整体 rename）再挂 `mount_save`。
- **系统字体 NCA**。`shared_font.c` + `games/totk/scripts/prepare_shared_fonts.py`；NSP 不含六款共享字体，构建要 `--shared-fonts` 或 `--firmware`。字体 pin 随固件变：1.9.0 记录的是 firmware 22.5.0，且四款与 TotK 1.0.0 不同（`docs/botw-v1.9.0-port.md` L53）。缺字体 → 标题菜单/对话文字空白，不是崩溃，最难诊断。
- **语言包**。RomFS 内 `de/fr/es/it/ja/ko/nl/ru/zh` 分目录（`services.c` 里成对出现的语言串）；1.6.0 与 1.9.0 的 RomFS 分布不同（1.9.0 要用 hactool BKTR 合并 base+update）。改文本或移植翻译mod要重签 RomFS，`docs/translation-mods.md`。
- **DLC 检测**。`aoc_romfs.c` + `games/botw/scripts/extract_dlc.py`/`extract_aoc_romfs.py`；1.6.0 卡带的 DLC 在 `aoc/<index>` 布局，`--dlc` 只对 1.9.0 构建生效（同文档 L28）。`01007EF00011F002` 只有 `System/AocVersion.txt`（648 字节），是版本/权益包——缺它 DLC 内容不出现，但**主线正常**。
- **amiibo**。服务清单里没有 NFC/amiibo 服务，判定在游戏内。推断不影响主线（UNVERIFIED，未在 `game_patches_botw.c` 中找到相关 patch）；1.6.0 的 amiibo 内容是可选掉落（Epona/Wolf Link）。Android 侧若要支持需 NFC + `nfc:` 服务，属二期。

## 4. 与 nxnative 的差集

nxnative `dm/host` 81 文件、`github.com/seedds/nxnative/blob/HEAD/dm/host/<file>`。

**nxnative 有、Mac/d2r 都没有**（13 个服务文件，正好是 d2r 基座的空白区）：

- `hle_ro.c`（`ro` 只读数据/NCA）、`hle_sf.c`（`sf` 服务框架）、`hle_audren.c`（audren 独立实现，Mac 是自建 `audio_renderer.c` 不叫这名）、`hle_nvdrv.c`（nvdrv ioctl + `/dev/nvhost-*`）、`hle_mii.c`、`hle_movie_avc.c`（硬解 H.264，Mac 是 `movie_host.c` 软解）、`hle_glslc.c`（shader 编译服务）、`hle_oodle.c`（Oodle 解压）、`hle_sdk16.c`（SDK 16 层）、`hle_bigstack.c`、`hle_uemalloc.c`、`hle_fpslock.c`、`hle_offline.c`、`hle_haptics.c` + `haptics_apple.m`、`hle_count.c`。

**Mac 有、nxnative 没有**：

- 图形：`gfx_bridge.c`、`gfx_probe.c`、`gpu_path_probe.m`、`vpool_probe.m`（NVN→Metal 直桥 + `totk_xlate` 译 shader，nxnative 只有 `hle_glslc.c`）。
- 资源：`shared_font.c`、`aoc_romfs.c`、`autoload.c`、`world_survey.c`、`settings_service.c`、`swkbd_host.c`。
- 重写层：`loader.c`、`gate.S`、`hle_stack.c`/`.S`、`thread_enter.S`、`game_patches*.c`、`game_version.h`（多版本地址表，nxnative 按 `dm/games/<id>/game.tsv` 数据驱动，机制不同）。

**d2r 独有**：`dm/` 的 mod 业务层（`dm_rift.c`、`dm_hud.c`、`dm_*` 约 60 文件，Diablo 专用，BotW 用不上）+ 完整 Android 端口 `dm/android/app/`（`wsi_bridge.c`、`audio_output.c`、`input_source.c`、`lifecycle.c`、`present_policy.c`）。**nxnative 无 Android 端口**（只有 `front_mac.m`）。

**结论**：Android 侧拿 d2r 的 `dm/android/app` 当移植层，服务侧拿 nxnative 的 `hle_ro/sf/audren/nvdrv` 补洞，图形侧只能留在 Mac 的 `gfx_bridge`+`totk_xlate` 路线上改 Vulkan——三者没有交集，这就是最大的集成成本。nxnative 的 BotW 是 1.9.0，`dm/games/` 下对应目录未在本次枚举中确认（列表被截断，UNVERIFIED）。

# 设备日志（#99）

> 来源：票 #99（Parent #89）。状态：v2（2026-11-15，吸收 review 修正）。
> 依据：#91（壳事件与词表）、docs/ARCHITECTURE.md §3.7（壳生命周期）。
> 代码：`shell/android/jni/nva_shell_jni.c`（写）、`shell/android/java/com/bingnnvjn/nva/LogProvider.java`（读）。

## 1. 它解决什么

真机取证原来只有一条路：`adb logcat`。要数据线、要驱动、要命令行，每张票都得重来一遍，
设备不在手边时票只能挂着。

现在壳自己把同一批事件写进 `<data_dir>/nva-shell.log`，取回方式是「用一下 → 长按画面 →
选一个 App 发出去」。**这不改变「谁操作」**：冷启动、HOME 回前台、旋转仍然要人动手，
Android 不允许应用重启自己，也不允许后台拉起界面。省掉的是「采集」，不是「操作」。

## 2. 怎么取

1. 装上 CI 产物（`shell-apk` 工件）。
2. 做要记录的操作。
3. 长按画面 → 系统分享面板 → 选任意目标（邮件、文件、聊天）。
4. 附件名是 `nva-shell.log`。

分享走 `LogProvider`，URI 是 `content://com.bingnnvjn.nva.logs/nva-shell.log`。
**provider 不是权限**：manifest 里没有声明任何权限；`<provider>` 声明
`exported="false"` + `grantUriPermissions="true"`，读权限只覆盖那一个 URI、
只在这一次分享的接收方生命周期内有效。

**为什么不是 `FileProvider`**：票面写的是 FileProvider，但那个前提在这个构建里不成立——
javac 的 bootclasspath 上只有 `android.jar`，没有 androidx；也没有 `res/` 可以挂
`@xml/file_paths`。所以 provider 是手写的，只用 framework 类，并且把「只服务这一个名字」
用代码钉死（`openFile` / `query` / `getType` 三个都校验路径）。

文件落在 `getFilesDir()`（应用私有目录），不是外部存储：外部目录在 Android 11+ 的文件管理器
里看不见，而既然本来就要走 provider，私有的更省事，也更不容易被别的应用读到。

## 3. 行格式

一行一条，字段顺序固定：

```
=== run 3 2026-11-15T13:40:12.345+08:00 ===
13:40:12.401 app created status=0
13:40:12.512 surface attached (generation=1) 1080x2160 status=0
13:40:18.003 detached (generation=1) status=0
```

- **run 头**：`=== run N <ISO8601> ===`，每次 create 尝试写一行（create 失败也写）。
  N 是「当前文件里已有的 run 头数 + 1」。ISO 时间戳带毫秒和时区偏移（如 `+08:00`），
  **两个文件之间靠它排序，不靠 N**（见 §5）。
- **事件行**：`HH:MM:SS.mmm <事件> [generation=N] [WxH] status=N`。时间是本机墙钟，
  `status` 的取值见 §7。
- **词表沿用 #91，不另造**：`app created`、`surface attached (generation=N)`、`detached`、
  `started`、`paused`、`resumed`、`stopped`、`destroyed`。
- **`nativeLog` 是第 10 个入口**，写自由文本（设备行、尺寸未定、分享失败），不参与上面
  9 条记录点；票面说的「9 个入口各一条」指的仍然是那 9 条。

少数行是**诊断**，不是入口的记录点，因此没有 `status=`：

| 行 | 侧 | 含义 |
|---|---|---|
| `destroy with a window still attached (generation=N)` | C | 正常拆除是先 `surfaceDestroyed` 再 `onDestroy`；这行说明走的不是那条路 |
| `destroy without an app` | C | `appCreate` 从没成功过 |
| `log open failed: …` / `log reopen failed: …` / `log rotate failed: …` | C | 文件那一侧出了问题，事件仍在 logcat |
| `no data dir: …` / `data dir is too long: …` | C | 文件不打开，只走 logcat |
| `NVA shell … on <型号> / Android <版本>` | Java | 设备行，每次 Activity 创建记一次，标明这份日志来自哪台机器哪个版本 |
| `surface has no size yet: WxH` / `surface changed with no size yet: WxH` | Java | holder 还没量出尺寸，这次根本不进 JNI |
| `share failed: …` | Java | 分享面板没起来（没装可接收的 App，或 Activity 已经没了） |

**拒绝只记一行，在 C 侧**（`<事件> failed: status=N`）。Java 不再抄一遍：同一次失败
出现两条、措辞还不一样，读的人只会当成两个事件。

## 4. 与 logcat 的关系

同一段格式化代码，两个出口。文件里的行与 logcat 里的行**逐字节相同**，包括
`HH:MM:SS.mmm` 前缀——所以 `adb logcat -s nva-shell` 和分享出来的文件可以直接对起来，
抽查任何一行都行。

唯一的差别是**开文件之前**的行：`JNI_OnLoad` 和 `shellInfo` 只在 logcat 里，因为那时
`appCreate` 还没拿到 `data_dir`。文件的第一行是 run 头；**第二行是这次 create 的结果**——
成功是 `app created`，失败是 `app create failed: status=N`。

Java 侧的行走 `nativeLog`，同样进文件、同样进 logcat。Java 自己不开文件。

## 5. 轮转

写之前先判断：这一行（换行算在内）会让文件超过 256 KB，就把当前文件改名成
`nva-shell.log.1` 再重开。

- 只保留一代 `.1`。第二次轮转会覆盖上一代的 `.1`。
- 轮转后的新文件从那一刻继续，**没有**新的 run 头——例外是触发轮转的恰好是 run 头自己
  （文件正好写到 256 KB 边上又来了一次 create），这时新文件的第一行就是那个头。
- 轮转后 run 号从 1 重新开始：`.1` 和 `.log` 之间会重号。对齐两个文件靠 ISO 时间戳。
- **分享永远只给 `nva-shell.log`**，所以轮转过的那个 run 会缺开头（`app created`、
  `generation=1` 留在 `.1` 里）。按一次完整 run（冷启动 + HOME 回前台 + 旋转，约 13 行、
  0.6 KB 量级）估算，约 400 次 run 才轮转一次，取证场景碰不到；真要看更早的，只能
  `adb pull <data_dir>/nva-shell.log.1`。这两个数字是估算，随生命周期回调次数与
  设备行的长度浮动。

## 6. 谁写、谁不写

- **C 侧是唯一写入方。** 准确说：`shell_emit` 是唯一**写** `g_log_file` 的函数；
  `shell_log_open`、`shell_rotate`、`shell_log_close` 只负责开、改名、关，
  `shell_count_runs` 用自己一个只读句柄数 run 头，不写。
- **一行 = 一次 `fwrite`**，连换行一起写；写完 `fflush`。一行不与另一行交错，
  靠的就是这个单点——写失败要么当场重测文件大小，不存在半行等下一行来补。
- **Java 侧不写文件。** 它要记一行就走 `nativeLog`，落到同一个 `shell_emit`、同一段
  格式化。`nativeLog` 还把 Java 文本里的换行、回车换成空格，保证一次调用仍是一行。
- **每行 `fflush`。** 进程被系统杀掉时缓冲区里的行会丢，而取证恰恰常发生在杀进程之后。
- **不额外加锁。** 所有入口都在主线程（`nva_shell_jni.c` 头注释的「Thread」一节），
  单次 `fwrite` 本身就保证整行落地。

## 7. 字段从哪来

| 字段 | 来源 |
|---|---|
| 日志路径 | Java `appCreate(String)` 的参数（`getFilesDir()` 的绝对路径）；同一个指针也原样填进 `nva_app_desc.data_dir` 交给运行时。**没有新增 ABI 字段** |
| `generation` | Java 侧 `MainActivity` 计数：`surfaceCreated` 加一，几何真的变了才加一，`surfaceDestroyed` 不加（§3.7） |
| `WxH` | `windowAttached` 的入参，不是从 holder 现读的 |
| `status` | 这次调用的返回码；非 0 的取值定义在 `include/nva_abi.h`，0 = 成功。三个入口没有可报告的返回值（`shellInfo` 返回字符串，`appDestroy` 底下的 `nva_app_destroy` 返回 void），它们的行恒记 0，含义是「入口正常返回」 |

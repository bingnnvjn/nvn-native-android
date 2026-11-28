# 设备日志（#99、#101、#94）

> 来源：票 #99（Parent #89）；§8 来自 #101（运行时事件）；§9 来自 #94（统计快照）。状态：v4（2026-11-28，新增 §9；v3 及更早见文件历史）。
> 依据：#91（壳事件与词表）、docs/ARCHITECTURE.md §3.7（壳生命周期）、#101（`nva_platform.log`）、#94（观测契约）。
> 代码：`shell/android/jni/nva_shell_jni.c`（写）、`shell/android/java/com/bingnnvjn/nva/LogProvider.java`（读）、`crates/nva-verify/src/statslog.rs`（解析）。

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
`exported="false"` + `grantUriPermissions="true"`，读权限只在这一次分享的接收方生命周期内
有效。manifest 层的 grant 是 authority 级的，「只服务这一个名字」是 provider 自己用代码钉死的
（`openFile` / `query` / `getType` 三个都校验路径）。

**为什么不是 `FileProvider`**：票面写的是 FileProvider，但那个前提在这个构建里不成立——
javac 的 bootclasspath 上只有 `android.jar`，没有 androidx；也没有 `res/` 可以挂
`@xml/file_paths`。所以 provider 是手写的，只用 framework 类。

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

- **run 头**：`=== run N <ISO8601> ===`，每次 create 尝试写一行（create 失败也写；唯一的例外是
  「app 已存在」的早退拒绝，那条路连文件都不开）。
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
| `native call failed: <异常>` | Java | `shellInfo` 抛了异常（典型是库没加载成功），设备行里会带着它 |

**拒绝在 C 侧记，Java 不再抄一遍。** #99 时这是「一次拒绝一行」（`<事件> failed: status=N`）；
#101 之后被拒的调用还会带上运行时那一行（`[rt] error … refused: …`，见 §8），所以现在是
**两行一对**：先说运行时为什么拒，再说壳这次调用的返回码。Java 仍然一句都不记——它再记一遍会
变成三行、三套措辞，读的人才真的会把它们当成三个事件。

## 4. 与 logcat 的关系

同一段格式化代码，两个出口。文件里的行与 logcat 里的行**逐字节相同**，包括
`HH:MM:SS.mmm` 前缀——所以把 logcat 各 tag 的行都抓下来，和分享出来的文件可以直接对起来，
抽查任何一行都行。

logcat 的 **tag 按来源分**（#94）：壳自己的事件是 `nva-shell`，运行时的普通行是
`nva-runtime`，统计快照是 `nva-stats`（§9）。文件里没有 tag 列，所以「逐字节相同」
不受影响；要一次抓全，用 `adb logcat -d`（不加 `-s`）或把三个 tag 都列上。

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
  `generation=1` 留在 `.1` 里）。按一次完整 run（冷启动 + HOME 回前台 + 旋转，约 20 行、
  0.9 KB 量级）估算，约 280 次 run 才轮转一次，取证场景碰不到；真要看更早的，只能
  `adb pull <data_dir>/nva-shell.log.1`。这两个数字是估算，随生命周期回调次数与
  设备行的长度浮动；#101 之后每个迁移多一行 `[rt]`，所以从 #99 的 13 行 / 400 次涨到这里。

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

## 8. 运行时自己的行（#101）

壳把 `nva_app_desc.platform` 指向 `nva_shell_jni.c` 的 `g_platform`，运行时于是能通过
`nva_platform.log` 把状态迁移和拒绝写进**同一个** `nva-shell.log`：

```
13:40:12.408 [rt] info nva-runtime: start
13:40:12.409 started status=0
13:40:12.412 [rt] info nva-runtime: resume
13:40:12.413 resumed status=0
13:40:12.418 [rt] info nva-runtime: attach generation=1 1080x2160
13:40:12.418 [rt] info nva-stats: {"presents":1,"surface_generation":1,"attached":true,"generation":1,"submits":1,"validation_available":0,"validation_errors":0}
13:40:12.419 surface attached (generation=1) 1080x2160 status=0
```

- **`[rt]` 是文件里的书写区别，logcat tag 是第二个区别。** 运行时行与壳事件都经同一个
  `shell_log_v` 格式化：同一个 `HH:MM:SS.mmm` 前缀、同一个文件、每行都 `fflush`。壳事件走
  `shell_log`（logcat tag `nva-shell`）；运行时行走 `shell_log_tagged`，tag 由运行时给——
  普通行是 `nva-runtime`，统计快照是 `nva-stats`（§4、§9）。**判断先后看文件偏移**：壳记返回码的那条总在 `[rt]` 之后
  （`nva_app_*` 返回了它才有那条），但诊断行可以在前面——带窗口进 `destroy` 时，
  `destroy with a window still attached` 就排在 `[rt] … destroy` 之前。
- **时间戳是本机墙钟**（§3）。两侧共用同一个取时函数、同一条线程顺序写，所以正常情况不倒退；
  墙钟被 NTP 或夏令时调整时可能倒退，代码没有为此做补偿。要断言「时间戳单调」，前提是这次 run
  里系统时钟没跳。
- **等级**取 `include/nva_abi.h` 的四个宏，壳映射成词：

  | ABI 宏 | 数值 | 文件里的词 | 什么时候出现 |
  |---|---|---|---|
  | `NVA_LOG_ERROR` | 0 | `error` | 拒绝：迁移不合法、attach 被拒、代差不匹配、状态锁中毒；引擎拒绝：渲染线程起不来、首帧 setup 失败 |
  | `NVA_LOG_WARN` | 1 | `warn` | 引擎备注：`suboptimal`、swapchain out-of-date、首帧 1 秒未就绪、渲染循环停止的原因 |
  | `NVA_LOG_INFO` | 2 | `info` | 迁移成功：`start` / `resume` / `pause` / `stop` / `attach` / `detach` / `destroy` |
  | `NVA_LOG_DEBUG` | 3 | `debug` | 本票不产生 |

  这四个数值只在本 ABI 内有意义，**不是** Android `Log.*` 的优先级（那是另一套 2..7）；
  映射在壳里做，运行时不知道 Android 存在。
- **等级只体现在行里的词上，不改 logcat 优先级。** `shell_emit` 两个出口都用
  `ANDROID_LOG_INFO`（§4 要求两边逐字节相同），所以 `adb logcat *:E` 抓不到 `[rt] error`；
  要按等级筛选，看文件，别用 logcat 的优先级开关。
- **一次迁移一行。** 冷启动的运行时迁移是 `start`、`resume`、`attach generation=N WxH`
  三条，各恰好一次；attach 成功、detach 成功后运行时各再说一条 `nva-stats` 快照（§9）。
  拒绝是一条 `error`，形如 `start refused: no transition from started`——带上被拒的操作和它当时
  所处的状态；`detach` 的代差不匹配会把两个代差都写出来。快照由运行时在 `nva_app_attach_window`
  / `nva_app_detach_window` 内说，壳不再在调用返回后另读一次 `nva_app_get_stats`。
- **运行时的词是 ABI 调用名，不是 §3 的壳事件词。** `[rt] ... destroy` 与壳的
  `destroyed status=0` 成对出现，是因为运行时说的是它自己被调用的那个函数，而壳说的是它对
  用户可见的事件（过去时）。两套词各自成立，不需要对齐。
- **两种行不记**：`nva_app_create` 自己的拒绝（那一刻还没有对象，没有可说话的出口，而壳那边
  已经有 `app create failed: status=N`），以及 `nva_app_get_stats` 的拒绝（查询不是事件，
  逐帧记录会把事件埋掉）。两处都是判断，写在 `crates/nva-runtime/src/app.rs` 的模块注释里。
- **统计快照是 JSON 行，不是壳事件。** #94 之后，attach 与 detach 各由运行时说一条
  `nva-stats` 快照（§9），壳不再在调用返回后另读一次 `nva_app_get_stats`。快照的
  `presents` 是运行时渲染线程的上屏次数，**进程内累计、不按 attach 轮次清零**；`generation`
  是**粘性**的，与 `attached` 分开表达「最后一代是多少」和「现在有没有窗口」。字段契约、
  三个时点与解析器见 §9。这两个时刻之外，运行时**不会**从渲染线程写任何行：日志文件仍然
  只有一个写者（本线程）。

## 9. 统计快照（#94）

设备侧观测契约。运行时在三个时点各说一条单行 JSON，tag 是 `nva-stats`；验收工具把它当
夹具，不写文件、不拉取、不要 root、不要 debuggable。

**行形状**（一行一条）：生产方按固定顺序写七个字段，这是生产方的契约；解析器按字段名取值，
不依赖顺序。

```
13:40:12.418 [rt] info nva-stats: {"presents":1,"surface_generation":1,"attached":true,"generation":1,"submits":1,"validation_available":0,"validation_errors":0}
```

| 字段 | 类型 | 含义 |
|---|---|---|
| `presents` | 整数 | 上屏次数；运行时渲染线程累计，不按 attach 轮次清零 |
| `surface_generation` | 整数 | 最后一次成功 attach 的代数；粘性，detach 不清零 |
| `attached` | 布尔 | 说这条快照时有没有窗口（attach 为 `true`，detach 为 `false`） |
| `generation` | 整数 | 与 `surface_generation` 相同的粘性代数，挨着 `attached` 读 |
| `submits` | 整数 | `vkQueueSubmit` 次数；进程内累计 |
| `validation_available` | 整数 | 产品包不带 VVL，恒 `0`；**不前置门** |
| `validation_errors` | 整数 | 同上，恒 `0`；不能当成「validation clean」的证据 |

**三个时点**，全部在主线程由运行时说：

1. **attach 成功后**：`attached:true`，`presents` 已含首帧（Android 上 attach 等首帧最多 1 秒）。
2. **detach 成功后**：`attached:false`，计数已冻结（detach 已 join 渲染线程）。
3. **detached 窗口内仍有提交**：同一个 detach 调用里追加一条独立信号行，与快照分开：
   `{"event":"detached_submit","count":N,"generation":G}`。`count` 是 stop 置位后仍进入
   present 的次数；`generation` 是刚 detach 的那一代。没发生就没有这条行，不靠推断。
   每次成功 attach 都会把计数清零，所以信号只反映最后一代，旋转时旧循环的 stop 不会串进来。

0x0 surface 的首次 attach（`surfaceCreated` 早于测量）不起渲染线程，快照可能是
`attached:true, presents:0`；这是记录，不是失败，下一次 `surfaceChanged` 的 attach 会带测量值。

**读取节奏（先清后取）**：

```sh
adb logcat -c                       # run 之前清缓冲区
adb shell am start ...              # 做一次冷启动 / HOME / 旋转
adb logcat -d -s nva-stats          # run 之后只取快照
```

`logcat -s nva-stats` 成立的前提是壳把运行时给的 tag 透传给 logcat（`shell_platform_log`
→ `shell_log_tagged`）；只改运行时那一侧、不改壳，抓不到任何一行。

**行长度上限**：`SHELL_LOG_LINE` 320 B，减 13 B 时钟前缀与 2 B 保留，消息体可用 305 B；
前缀 `[rt] info nva-stats: ` 占 21 B，所以 **JSON 必须 ≤ 284 B**。快照只写七个字段；把整个
20 字段的 `nva_stats` 序列化进去会超限，壳静默截断后解析器报 `NotJson`，看起来像解析器 bug。

**解析器严格性**（`crates/nva-verify/src/statslog.rs`）：

- 不含 `{` 的行（壳事件、运行时事件）→ 跳过；
- `detached_submit` 信号行（合法 JSON、带 `"event":"detached_submit"`）→ 跳过，不参与序列；
- 合法快照 → `Snapshot`；
- 含 `{` 但不是合法 JSON → `NotJson`；
- 缺字段 → `MissingField`；类型不符 → `WrongType`；
- 同一流里 `presents` 或 `generation` 变小 → `OutOfOrder`。

**缺字段、乱序、以及含 `{` 的坏 JSON 都必须可判错，不许静默吞**；无 `{` 的事件行与信号行是
两类明确的跳过，不是吞错。`parse_stream` 只对**同一个 run** 的流有意义：跨 run 的日志文件要
先按 `=== run` 切片再解析，因为新 run 的 `presents`/`generation` 归零会被判 `OutOfOrder`。

合法 / 缺字段 / 乱序 / 无 `{` 四种形状是 `src/statslog.rs` 的单测；含 `{` 的坏 JSON、类型不符
与信号跳过在 `tests/statslog.rs` 集成测试。都不依赖真机。

**`device --scenario m0b` 如何消费快照（#96）**

- **attach 快照**（`attached:true`）：`surface_generation`、`presents` 与
  `validation_available` / `validation_errors` 取自这里；就绪条件是 `attached:true`
  且 `presents >= 1`，0x0 surface 的 `presents:0` attach 不结束等待（§9 上文），
  继续等下一条带测量值的 attach。像素另从 `adb exec-out screencap` 的原始流取窗口
  中心（12 / 16 B 头都接受，不做 PNG 解码）；raw 头 `format` 只接受 `1`
  （`PixelFormat.RGBA_8888`），其他格式无法按本判据解包，按证据不符退 `2`。
- **detach 快照**（`attached:false`）：`presents` 是 stop 置位、渲染线程 join 之后的
  冻结值，**包含 attach 到 HOME 之间正常渲染的帧**，所以不能用它与 attach 快照的差值
  判失败。POST-HOME 是否还在提交由下一条信号判定；进程消失另记 `crashed`（→ `2`）。
- **`detached_submit` 信号**：`count` 是 stop 置位后仍进入 present 的次数。信号缺席即
  `count = 0`，也就是「HOME 后 `presents` 不增」；有信号则第 4 条判据与 HOME 反证
  同时失败（退出码 `2`），不从快照数推断。信号行仍由解析器跳过，不进入快照序列。
- **跨 run 切片**：`logcat -c` 之后取到的流通常只有一个 run；若混入 `=== run` 头
  （例如直接读日志文件），先按最后一个 run 头切片再交给 `parse_stream`，否则新 run
  的计数归零会被判 `OutOfOrder`。
- **不前置门**：`validation_available` / `validation_errors` 只写进证据 JSON，
  不参与 M0b 判定；产品包恒 `0` 既不是绿的理由，也不是红的理由。


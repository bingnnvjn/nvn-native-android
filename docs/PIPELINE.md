# 产物流水线：CI、artifact、签名、import pack（#21）

> 输入：#15（Actions 构建链事实）、#16（公开边界）、#20（L0–L6 与 C ABI v1）、#7（APK + 本地导入包）、
> `docs/ARCHITECTURE.md`（产品架构，§3.1–3.3 与 §4 是 import pack 的权威定义）。
> 状态：v0，覆盖 ABI spike 与产品壳 job（`shell-apk` 于 #46 启用）；`tools` 仍预留。决定日期 2026-10-31，产品壳更新 2026-11-01。

## 0. 一页结论

| 问题 | 决定 |
|---|---|
| CI 编什么 | `guard`（红线）、`pc-x86_64`（lavapipe 回归）、`arm64-linux`（AArch64 契约）、`android-aarch64`（交叉 + APK）、`shell-apk`（产品壳，无 Gradle：`aapt2`/`d8`/`apksigner`）。工具 `tools` 仍以骨架预留、默认不启用。 |
| 出什么 artifact | `abi-slice-apk`、`abi-slice-pc`、`abi-slice-arm64`、`abi-slice-evidence`、`shell-apk`（产品壳，`nva-shell.apk` + 证据，#46）。**不建 GitHub Release**（v0）。 |
| APK 怎么签名 | 固定 PKCS12 密钥 `ci/signing/artifact.p12`（公开、非发布信任），`apksigner` 显式 v1+v2+v3，证书 SHA-256 钉在 `ci/signing/fingerprint.txt`，每次断言。 |
| 发布 | v0 不发。开 Release 的前置与发布物见 §4.3。 |
| import pack 工具 | **三件，不是一件**：`nva-pack`（L0：extract / pack / verify / install）、`nva-rewrite`（L1）、`shader-build`（L2，.NET）。CI 只跑合成 fixture。 |
| 红线 | README 中英段 + `docs/DERIVED.md` 账本 + 门禁与自测脚本（含魔数/密钥/个人数据扫描），清单见 §7 + release notes 模板固定段。见 §6。 |

## 1. 为什么是这个形状

- **#15 的硬事实**：上游 `ci.sh android` 带 `DM_ANDROID_CHECKS=ON`，CMakeLists 第 15–19 行提前 return，**只编测试、从不链接 `libdm.so`、没有 Gradle、没有 APK**。「上游 CI 能出 APK」不成立。
- **#15 的第二、三个事实**：Ryujinx 着色器翻译器只在本机 `dmsh_xlate` 跑（`dotnet publish`），**不是 CI 依赖**；Turnip 是来源不受控的预编译 `.so`，v1 不进 CI。
- **#16 的硬事实**：DMCA 打的是「分发绕过 TPM 的工具」（§1201），不是仓内游戏文件；但 GitHub 的处置单位是 **fork 网络**（311 仓连坐）。所以独立仓 + 三件产物分三处 + 门禁是结构性对策。
- **#20 / ARCH 的硬事实**：C ABI v1 已冻结；L0/L1/L2 的产物边界固定（`manifest.json` / `load-plan.json` / `raw/` / `romfs.bin` / `shader-cache.bin` + index）；`svc_gates.json` 由 **host 侧 generator** 发号，重写器只读。流水线只负责**编译与校验**。
- **#7 的硬事实**：CI 只编译不含游戏数据的 APK；游戏数据本机生成 import pack。

## 2. Job 清单（`.github/workflows/ci.yml`）

| job | runner | 编什么 | 做什么校验 | 产物 |
|---|---|---|---|---|
| `guard` | ubuntu-24.04 | 不编 | 树 + 未跟踪 + 全历史 + 内容魔数/密钥扫描；`DERIVED.md` 对账；README 免责段；spike 与产品同名文件断言；文档 § 引用检查 | 无 |
| `pc-x86_64` | ubuntu-24.04 | Rust `nvn_host` + C guest | lavapipe 出图、validation clean、5 条断言、`unimplemented` 必须 SIGABRT；产品 workspace 成员恰好 5（#39）；C ABI 布局断言 宿主 C11 / C++17（`-fsyntax-only`，#40） | `abi-slice-pc` |
| `arm64-linux` | ubuntu-24.04-arm | 同上，`+reserve-x18` | gate/stackgate/TLS/x18/FPCR 契约，10 条断言 | `abi-slice-arm64` |
| `android-aarch64` | ubuntu-24.04 | NDK r27d 交叉 + aapt2/d8/apksigner | APK 内容扫描、manifest、签名证书、artifact 白名单；C ABI 布局断言 NDK C11 / C++17（#40） | `abi-slice-apk`、`abi-slice-evidence` |
| `gate` | ubuntu-24.04 | 不编 | 汇总：任一必需 job 非 success 即红 | 无 |
| `shell-apk` | ubuntu-24.04 | 产品壳 `shell/android/`，无 Gradle（`aapt2`/`d8`/`apksigner`，#46） | APK 内容扫描（`scan_apk.sh`）、`check_shell_apk.sh`（xmltree 断包名/`minSdk 26`/无 `debuggable` + `llvm-nm -D` 断 `JNI_OnLoad`）、`check_manifest.sh` 的 `product` 模式（证书/ABI/权限/v2+v3） | `shell-apk` |
| `tools`（预留 `if: false`） | ubuntu-24.04 | `nva-pack` / `nva-rewrite` | 合成 fixture：`nva-verify rewrite/loadplan` | `tools` |

**为什么 `gate` 单独存在**：分支保护只认一个必需检查时，多 job 的组合态不好表达；`gate` 折成一个状态位。**启用 `shell-apk` 或 `tools` 时，必须把它们加进 `gate.needs`**，否则启用后仍然绿——这条写在这里，因为它是开关，不是摆设。`shell-apk` 已于 #46 加入 `gate.needs`；`tools` 尚未启用。

**为什么 `guard` 不依赖任何构建**：红线检查必须最快、最便宜，且 PR 上先红。它 `fetch-depth: 0` 扫**全历史**——#16 的教训是「删掉了但历史里还在」同样致命。

**长支线覆盖与缺口（2026-10-31）**：`on.push.branches` 覆盖 `main`、`integrate/main`、`risks/v0`、`arch/l0-l6`、`ci/pipeline`、`spike/abi-slice`、`research/round-1`。`research/round-1` 与 `arch/l0-l6` 分支如果没有带本 workflow，push 仍不过门禁——**合并到 main 或 rebase 后才生效**。`spike/abi-slice` 上的旧 `abi-slice.yml` 仍用 tag 引用 action，合并前需单独钉 SHA。

**分支保护（人工设置，仓库脚本无法自检）**：GitHub 上 main 的 required check 必须设为 `gate`，并勾选禁止 force-push。改完在仓库设置页核对一次；这条不写进 CI。

### 2.1 job × 工具可用性

给 `ci.yml` 加 step 前先查这张表：step 能不能跑，取决于**那个 job 装没装工具**，不是
本地有什么。写错 job 的改动要到 CI 才红（2026-11-13 实录：曾把 `cargo test` 方案写进
`guard`，靠评审纠回 `pc-x86_64`——`guard` 没有 cargo）。

| job | cargo 1.97.1 | cc / c++ | NDK r27d | glslang | python3 |
|---|---|---|---|---|---|
| `guard` | — | — | — | — | ✅ |
| `pc-x86_64` | ✅ | ✅（build-essential） | — | ✅ | ✅（镜像自带） |
| `arm64-linux` | ✅（原生 AArch64） | ✅（原生 AArch64） | — | ✅ | ✅（镜像自带） |
| `android-aarch64` | ✅（+`aarch64-linux-android` target） | ✅（NDK clang） | ✅ | ✅ | ✅（镜像自带） |
| `shell-apk` | — | （NDK clang 已配置，无编译 step） | ✅ | — | ✅（镜像自带） |
| `tools`（预留） | ✅ | — | — | — | ✅（镜像自带） |

依据是 `ci.yml` 各 job 的 install step（`guard` 的「不编任何东西」见 §2 表与
`pc-x86_64` 内 #39 注释）；改了 install step 就同步改这张表。

口径脚注：`✅/—` 按「该 job 有没有使用它的 step」计；`python3` 是例外——各 job 的
runner 镜像都自带，`guard` 直接调用（无 install step），故按可用性全标 ✅。`cc/c++`
同为镜像自带，但表中按 install step 口径（`build-essential` / NDK clang）标。

## 3. Artifact 契约

| 名字 | 内容 | 保留 | 谁能拿 |
|---|---|---|---|
| `abi-slice-apk` | `abi-slice.apk`、`apksigner-verify.txt`、`sha256sum.txt`、`badging.txt`、`unzip-l.txt` | 30 天 | 任何人（公开仓 artifact） |
| `abi-slice-pc` | `abi_guest`、`libnvn_host.so`、`pc-all.log`、`vulkaninfo.txt`、`readelf-*` | 14 天 | 同上 |
| `abi-slice-arm64` | 同上（`+reserve-x18`），加 `objdump-thunks.txt`、`stadd.txt` | 14 天 | 同上 |
| `abi-slice-evidence` | `abi-slice-android.tar.gz`、`dist/android/`（ELF 证据、`readelf.txt`） | 14 天 | 同上 |
| `shell-apk` | `nva-shell.apk`、`apksigner-verify.txt`、`badging.txt`、`xmltree.txt`、`unzip-l.txt`、`sha256sum.txt` | 30 天 | 同上（#46） |

规则：

1. **artifact 里不出现构建机绝对路径**：spike 的 APK 与 evidence 都过 `check_artifact.sh`，它拒绝构建机 home 目录、macOS 用户目录、WSL 挂载路径与 Windows 盘符路径；`sha256sum.txt` 必须写相对文件名。`/tmp/` 不算外露路径。产品壳 `shell-apk` 只上传显式文件、不过该目录契约（原因见规则 5）。
2. **artifact 不带 `.git`、缓存、keystore**；`check_artifact.sh` 对目录做成员白名单，并展开 tar/tgz/zip 成员再查。evidence 只接受 `.txt`/`.json`/`.log`/`.sh`/`.so`/`.tar`/`.tar.gz`/`.tgz`/`.zip` 与无后缀 ELF。
3. **APK 永远带一份 `sha256sum.txt`**，与 `apksigner-verify.txt` 同目录。人拿到包先比对。
4. 保留期分开：APK 30 天（有人要装），证据 14 天（够复盘）。
5. 名字里的 `abi-slice-` 是 spike 前缀；产品壳已用 `nva-shell.apk` / artifact 名 `shell-apk`（#46）。`check_artifact.sh` 的 `apk` 模式仍写死 `abi-slice.apk`，所以 `shell-apk` job 只上传显式文件、不过该契约；要复用它得先把 APK 名参数化。

## 4. 签名与发布

### 4.1 v0：固定 artifact 密钥

- 文件：`ci/signing/artifact.p12`（PKCS12，RSA 2048，`CN=NVA Artifact, O=nvn-native-android`，10000 天）。
- 密码：默认值在 `spike/abi-slice/android/build_apk.sh` 与 `shell/android/build_apk.sh`，可用 `SIGNING_KEYSTORE_PASS` 覆盖；它不是秘密，见下。产品壳脚本没有 debug-key 回退：密钥缺失就直接失败。
- 算法：`apksigner` **显式** `--v2-signing-enabled true --v3-signing-enabled true`。**v1 是 no-op**：minSdk 26 下即使写 `--v1-signing-enabled true`，工具也不产出 v1 签名，`verify` 报 `v1 scheme: false`（run 41089559337 实测）。不开 v4：v4 要额外的 `.apk.idsig`，单文件分发会被这件事拖住。
- 指纹钉死：`ci/signing/fingerprint.txt` 存**证书** SHA-256（`openssl x509 -outform der | sha256sum`），与 `apksigner verify --print-certs` 的 `Signer #1 certificate SHA-256 digest` 同值；`check_manifest.sh` 每次比对。
- 校验：`apksigner verify -v` 输出里 `v2` / `v3` 两条必须是 `true`；v1 不检查（见上）。

**它是公开的，这是刻意的。** 它只解决一件事：不同 CI run 出来的 APK 能互相覆盖安装。它不是信任锚——任何人都能用同一把私钥签一个包。所以：

- 文档、README、artifact 描述里一律写 **artifact signing key, not a release key**；
- 一旦开 GitHub Release，必须换新密钥（`release.p12` 存 Actions Secret），并让用户卸载重装一次；
- 门禁 `check_manifest.sh` 区分 `mode=artifact` 与 `mode=release`：release 模式要求证书指纹 != artifact 指纹。

**为什么提交二进制而不是 CI 现场生成。** 现场 `keytool -genkeypair` 每次 run 都不同 → 包与包之间无法覆盖安装；`actions/cache` 缓存 keystore 只能在同一分支内复用。提交一个**公开的、无保密价值**的 PKCS12 是最小复杂度。已核：GitHub secret scanning 的支持模式表里只有 5 条 PEM 文本头私钥正则，没有 JKS/PKCS12/keystore 规则，二进制容器不命中（2026-10-31 核；「二进制一定跳过」这一点**没有官方明文**，是从 pattern 定义反推的）。代价要认：它永久留在 history，`git rm` 删不掉。

### 4.2 APK 内部断言

| 断言 | 值 | 谁检查 |
|---|---|---|
| `package` | `com.bingnnvjn.abislice`（spike）/ `com.bingnnvjn.nva`（产品，`GUARD_PACKAGE` 覆盖） | `check_manifest.sh`；产品另有 `check_shell_apk.sh`（xmltree） |
| `minSdk` | 26 | 同上 |
| `targetSdk` | 34（spike）；产品今天也是 34（#46 沿用 #37 D15）；36 是**待 L6 定**，不是已决 | 同上 |
| `native-code` | 只有 `arm64-v8a` | 同上 |
| 权限 | spike 0 个；产品按 L6 白名单（SAF/FGS），逐条审；今天产品 0 个 | 同上 |
| `debuggable` | spike `true`；产品**属性不存在**（= 默认 `false`） | `check_manifest.sh` 看 badging；`check_shell_apk.sh` 看 xmltree（#46） |
| 签名 scheme | v2+v3 两条 `true`（v1 在 minSdk 26 下不存在） | `check_manifest.sh` |
| 体积 | ≤ 32 MB | `scan_apk.sh` |

产品壳的 `product` 模式已实现并在 `shell-apk` job 里调用（#46）：包名、targetSdk、权限白名单、`debuggable` 四项按 `GUARD_MODE=product` 判。注意它的 `debuggable` 判据读 `dump badging` 的 `application-debuggable` 行——这个字段在 build-tools 36 上一次实测里掉过（§9），所以「无 debuggable」的权威断言放在 `check_shell_apk.sh` 的 xmltree 上；badging 那条是补充，不是唯一。

产品壳今天实际是：`com.bingnnvjn.nva`、minSdk 26、targetSdk 34、只有 `arm64-v8a`、0 个权限、无 `debuggable` 属性。

### 4.3 开 Release 的前置条件（写死，不到不做）

1. d2r 壳在 **Redmi Note 12 Turbo 真机**出过图（ARCH §3.6 的首里程碑「725 第一帧」）。**这是本机项，不进 CI**——CI 没有 Adreno 725，lavapipe 不代表它。
2. `release.p12` 已生成并存入 Actions Secret；`fingerprint.txt` 更新为 release 指纹。
3. `docs/DERIVED.md` 的每条都有对应门禁；`guard` 在 `main` 上连续绿 7 天。
4. README 红线段、`docs/RELEASE-NOTES-TEMPLATE.md` 已定稿。

发布物只有两样，**分开附件，绝不打包**：

- `nva-<version>-arm64.apk`（零游戏数据）
- 源码 tag（GitHub 自动）

**工具二进制不进 v0 release。** #16 §2 的指控对象正是「分发这个工具」；发 APK（空壳）有四家先例，发工具二进制没有。工具的**源码**随源码仓，二进制最多留在 artifact。真要发，另开票并先做法务判断。

**import pack 永远不进 release。** 这条没有例外，写进 release notes 模板。

若将来给 APK 做 attestation：**先签名、后 attest**（attestation 绑 digest，签名改字节），权限要 `id-token: write` + `attestations: write`。v0 不做。

## 5. import pack 工具：三件套，不是一个大 CLI

依据 `docs/ARCHITECTURE.md` §3.1–3.3 与 §4。产物链：

```
XCI/NSP + keys → nva-pack extract → nva-rewrite → shader-build → nva-pack pack
```

| 工具 | 层 | 语言 | 输入 | 输出 | CI |
|---|---|---|---|---|---|
| `nva-pack extract` | L0 | Rust | XCI/NSP + keys 路径 | `raw/exefs/*`、`raw/aoc/N/`、`romfs.bin`、`romfs.index.json` | 合成 fixture |
| `nva-rewrite` | L1 | Rust | NSO0 + 规则 + `svc_gates.json` + `tls.json` | patched NSO + `rewrite-manifest.json` | 合成 NSO |
| `shader-build` | L2 | .NET | `shader_list.tsv` + Ryujinx 快照 | `shader-cache.bin` + index | **不编**（本机依赖） |
| `nva-pack pack` | L0 | Rust | 上述产物 | `manifest.json`、`load-plan.json`、`romfs.bin` | fixture |
| `nva-pack verify` | — | Rust | pack 目录或 fixture | JSON 报告，退出码 0/2/3/4 | 两者 |
| `nva-pack install` | — | Rust | pack + 设备 | 推送到私有目录 | **dev-only**，不进 CI，也不作为产品导入路径（产品走 L6 SAF） |

### 5.1 包布局（照 ARCH §3.1/§3.3）

```
pack/
  manifest.json        # pack_id, title/version/build_id, 每文件 SHA-256, RomFS 树 hash,
                       # DLC, selfcheck, version；keys 只记 generation/key_count
  load-plan.json       # 段表；schema 归 L3，本文件不定义字段
  raw/exefs/{main,rtld,sdk,subsdk0}
  raw/aoc/N/
  romfs.bin            # 单 blob
  romfs.index.json     # 41,052 条路径索引
  shader-cache.bin     # + 尾部定宽 index（根级，不是 shaders/ 子目录）
  pack_info.txt        # 给人看的摘要，含 degraded 标记
```

硬约束：

- **keys 只传路径，manifest 只记 `generation/key_count`**——不记字节、不记哈希、**不记路径**。
- `pack_id = SHA-256(JCS(manifest 去掉 pack_id))`；manifest 不含绝对路径与时间戳，两次运行字节一致。「只追加」由单调 `generation` 落实，不是靠哈希排序。
- **门号唯一源是 `svc_gates.json`**（host 侧 generator 发）；`nva-rewrite` 只读，并在 `rewrite-manifest.json` 记 `gates[{id,svc,sites,count}]`，**不生成门表**。
- 字节补丁不入库（ARCH §3.2）；仓库只放规则 + 期望 hash。

### 5.2 降级语义

`shader-build` 缺席时 `nva-pack pack --no-shaders` 仍能出包，但必须：

- `manifest` 写 `degraded: ["shaders_missing"]`；
- `nva-pack verify` 对这种包返回**非 0**（默认拒绝），只有显式 `--allow-degraded` 才放行；
- 运行期行为「无 cache」在 L2 里尚未定义（ARCH 只定义「假 key → 跳过 draw」），所以这种包**不允许**用于验收，只用于开发。

### 5.3 CI 的交界

CI 只跑三件事，全部无游戏数据：

1. `cargo test -p nva-pack -p nva-rewrite`（合成 NSO / fixture）。
2. `nva-verify rewrite --fixture` 与 `nva-verify loadplan --fixture`，断言退出码 0/2/3/4 的语义。
3. `nva-pack verify ci/fixtures/pack-min/`，覆盖：schema 版本不符、哈希不匹配、`generation` 回退、缺 `load-plan.json`。

**CI 里没有任何一个 job 接受 `--xci` 或 `--keys`。** fixture 的 `title_id` 全 0。fixture 里被红线名字黑名单命中的文件**逐路径**登记在 `ci/guard/allowlist.txt`（`shader-cache.bin` 会被误伤，这是已知的例外，必须显式登记，不是放宽规则）。最小 load-plan schema、manifest 子集与确定性规则见 `ci/fixtures/README.md`（#42）；正式 L3 schema 仍归 L3（§10）。

## 6. 红线：落点与门禁

| 红线（#16） | 落点 | 门禁 |
|---|---|---|
| 游戏程序 / NSO / NCA / NSP / XCI（含改名） | README 段 | `scan_tree.sh` 后缀 + **`scan_magic.py` 魔数**（NSO0/PFS0/NCA/HEAD/KIP1/SPIR-V） |
| keys | README 段 + `DERIVED.md` 声明 | `scan_tree.sh` 文件名 + **`scan_magic.py` 的 `name = <32 hex>` 行** |
| romfs 资源与文本 | README 段 | `scan_tree.sh` 路径段（`exefs/`、`romfs/`、`save*/`） |
| shader 原件、翻译产物、pipeline cache | README 段 | `scan_apk.sh`（`.spv`/`.spvbin`/`.xgs`/`shader-cache.bin`）+ `scan_tree.sh` |
| 存档 | README 段 | `scan_tree.sh`（`*.sav`、`save*/`） |
| 私有密钥（release.p12 等） | README 段 | `scan_tree.sh` 后缀白名单，只放 `ci/signing/artifact.p12` |
| 个人数据（本机路径、报告 id） | `docs/PRIVACY.md` | `scan_personal.py` + `check_docs.sh`（tracked Markdown）+ `scan_tree.sh`（tree） |
| 派生表（可带，但要披露） | `docs/DERIVED.md` | `check_derived_ledger.py` + 内容判据（≥5 处 title ID / `main+0x` 必须有行） |

README 用 #16 §4 的中英措辞。发布说明用 `docs/RELEASE-NOTES-TEMPLATE.md` 的固定段，模板里「本页不含 import pack」中英各一句，`check_docs.sh` 断言它。

## 7. 门禁脚本清单

| 脚本 | 判据 | 失败信息 |
|---|---|---|
| `ci/guard/scan_tree.sh` | 后缀/路径段黑名单、>4 MiB 未登记二进制、裸 NSO 名、私有密钥、未跟踪文件、全历史；媒体/压缩包/evidence 只查工作树；调用 `scan_personal.py` | 打印命中路径 + 来源（tree/untracked/history） |
| `ci/guard/check_header_collision.sh` | 产品 `include/` 文件名集合与 spike 树文件名集合求交；只扫 tracked + 未忽略的未跟踪文件；`include/` 为空判错 | 打印碰撞的 spike 路径 |
| `ci/guard/check_workspace.sh` | 根 workspace 成员恰好 5 个且包名逐一匹配（`cargo metadata --no-deps`）、成员目录无 APK 壳输入、无依赖 Android 的 build script；需 cargo+jq，挂在 `pc-x86_64` | 打印期望 5 个 vs 实际包名或命中文件 |
| `ci/guard/check_layout_assertions.sh` | C ABI 布局断言的账本（#49）：`include/abi_layout_check.c` 的每条 `NVA_SA` 必须归入某个分组，`ci/guard/layout-assertions.txt` 的 `total` 与 11 个分组逐项比对；孤儿断言判错；不需编译器，挂在 `guard` | 打印 `LAYOUT-INVENTORY: <组> has N, expected M`，末行指出这是账本变更而非编译错误 |
| `ci/guard/check_doc_refs.py` | 文档交叉引用（`docs/X.md §N`、`ARCH/GATES/PIPELINE/RISKS/PRIVACY §N`）必须指向存在的标题；三级引用（如 `§4.3.4`）跳过；挂在 `guard` | 打印 file:line + 悬空引用 |
| `ci/guard/scan_magic.py` | 偏移 0 / 0x100 的魔数；`name(key\|kek) = <32 hex>` 行（含 docs）；`--history` 只扫 blob、按 oid 去重并保留 signing/fixtures 豁免；媒体魔数只查工作树 | 打印文件 + 命中类型 |
| `ci/guard/scan_personal.py` | 只查形状：盘符、WSL 挂载、home/Users 目录、数字 gpuinfo 报告 id；不建真实值 denylist | 打印文件:行 + 命中类型 |
| `ci/guard/scan_apk.sh` | 条目黑名单、`assets/`、必需条目、体积、绝对路径、`f_<hex>` 符号 | 打印命中条目 |
| `ci/guard/check_manifest.sh` | 包名/minSdk/targetSdk/ABI/权限/debuggable/签名 scheme/证书指纹 | 打印期望 vs 实际 |
| `ci/guard/check_shell_apk.sh` | 产品壳（#46）：xmltree 断包名/`minSdk 26`/无 `debuggable`，`llvm-nm -D` 断 `JNI_OnLoad`，断 `lib/` 只有 `arm64-v8a` | 打印 xmltree 与 APK 条目 |
| `ci/guard/check_artifact.sh` | 目录成员白名单、无中间产物、无密钥/媒体、tar/tgz/zip 成员解包、evidence 无后缀 ELF、sha256 自洽、无绝对路径 | 打印多余/缺失成员 |
| `ci/guard/check_derived_ledger.py` | `DERIVED.md` 每行对得上文件与体积；未登记派生表；内容判据 | 打印缺失/孤儿/漂移 |
| `ci/guard/check_docs.sh` | README 中英免责段、模板固定段（含「不含 import pack」）；复用 `scan_personal.py` 扫 public Markdown | 打印缺哪句或命中行 |
| `ci/guard/check_style.sh` | 5 个产品 crate 的格式与 lint：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`；spike 树被 workspace 排除，不在门下 | 打印失败的是 fmt 还是 clippy，并附工具输出 |
| `ci/guard/test_guards.sh` | 对 §7 的门做正反例自测；当前不覆盖 `scan_apk`、`check_manifest`、`check_derived_ledger`、`check_docs`（正例要真 APK 或真账本）；覆盖面见脚本内 `# --- <name> ---` 分节 | 打印失败用例（期望 rc vs 实际） |

机械门只管形状。规范文本本身在仓库之外，按公开边界 #33 不进 git；仓内留下面这张指针表，
新会话据此确认项目采用哪份规范，不必靠某台机器的本地文件：

| 文件（本地根，不进 git） | 是什么 | 内容 SHA-256 | 日期 |
|---|---|---|---|
| `RUST-STYLE.google.md` | Google Rust Style Guide 逐字全文 | 第 4 行起 `58500e5e95482cebdcfb79e8c4ac8298758107dc6693f7c65a3a1348f9f0aaec`，60,028 B | 取用 2026-11-08 |
| `RUST-STYLE.local.md` | 本项目与 Google 的偏差（`LOCAL §1`–`§3`） | 整文件 `eff9d92d310f024231cb3b6c87468d24aecff040d8dd7a60f020176fccef67a6` | 定稿 2026-11-08 |
| `docs/agents/standards.md` | 判据清单，每条挂 `Google > 章节` 或 `LOCAL §N` | 整文件 `cd9f15866769f20a923b714110b36b7a712a34dce0010cedcbcfdc3cc0fcc0eb` | 定稿 2026-11-08 |

哈希是校验手段，不是装饰：拿到同名文件先按标注的口径复算（原文去掉前 3 行元信息，另两份是整文件），
不一致就说明版本不对，别照着读。表里的路径相对本地工作副本的根——`docs/agents/` 是那里的目录，
不是本仓 `docs/` 的子目录，clone 出来后找不到它属正常。

### 7.1 按名字钉测试：两份内联，暂不抽（2026-11-14）

`pc-x86_64` 里有两段同形的「按名字钉测试」循环，各自内联在 step 中：`nva-runtime unit tests
(#61, #66)` 与 `nva-verify exit-code matrix (#43)`。

两份的循环骨架 6 行同形：`for name in \`、`grep -q "test ${name} \.\.\. ok" <日志> || {`、
`echo "::error::<前缀> …"`、`exit 1`、`}`、`done`（名单连同末行的 `; do` 算参数，故按 6 行计）。
其中 4 行逐字相同，另 2 行只有日志名与 `::error::` 前缀不同；理由注释的首行也逐字相同，第二行起
分叉。逐字重复合计 5 行。

**决定：暂不抽 `ci/guard/pin_tests.sh`。** 抽取能换来两样：`shellcheck` 覆盖，和只有一份实现。
后者的价钱是 §7 的脚本表加一行、`test_guards.sh` 加自测（`guard` 没有 cargo，脚本必须是纯 shell、
按 `check_workspace.sh` 的做法开 test seam）、以及 YAML 与脚本之间多一层间接。名单、日志、
前缀三项参数各自独立取值，抽成脚本也消不掉差异——5 行逐字重复换不动上面那堆。

**什么时候改这个决定**（任一条发生就抽）：

- 出现**第三处**这种 step。计法：某个 step 用 `grep` 逐名断言一份测试日志里出现
  `test <name> ... ok`，就计一处；范围是 `ci.yml` 全文件，不限 job。今天 `for name in`
  恰好两处；`Product ABI layout test` 只查 `1 passed` 这个计数，不计。
- 这两段之一的骨架需要变复杂（例如要认 `ignored`、要固定测试线程顺序、要按 `--test-threads`
  分组）。

内联 shell 没有静态检查：`ci.yml` 的 `shellcheck -S warning ci/guard/*.sh shell/android/*.sh`
覆盖不到 YAML 的 `run:` 块，全仓也没有 actionlint / yamllint。抽取的两样好处里，这一样是内联
永远拿不到的。

## 8. 不做什么

- 不在 CI 里跑真游戏数据（没有任何 job 接受 XCI/keys）。
- 不发 GitHub Release（v0）；不开 `workflow_run` 自动发布。
- 不把 Turnip 预编译 `.so` 打进 CI 产物（来源不受控，v1 不做）。
- 不在 CI 编 `dmsh_xlate` / `shader-build`（.NET 是本机依赖）。
- 不用 `ubuntu-latest` 漂移：钉 `ubuntu-24.04` / `ubuntu-24.04-arm`。
- 不做 SLSA attestation（v0 无发布信任需求；做法见 §4.3）。
- **真机项永不进 CI**：725 第一帧、帧时间 p95、8 MRT ≤28 ms、device validation——lavapipe 跑不了，写进 CI 就是自欺。

## 9. 实测记录（2026-10-31）

首个全绿 run：[41092359202](https://github.com/bingnnvjn/nvn-native-android/actions/runs/41092359202)——`guard` 5s、`pc-x86_64` 44s、`arm64-linux` 40s、`android-aarch64` 1m30s、`gate` 3s；四个 artifact 全部生成，APK 270,964 B，证书 SHA-256 与 `fingerprint.txt` 一致。

修 CI 时撞到的四个「纸面推断错了」：

1. **v1 在 minSdk 26 下不进验证**：显式 `--v1-signing-enabled true` 也只得到 `v1 scheme: false`，所以断言改成 v2/v3。**#46 修正**：`v1 scheme: false` 的意思是「没有用 v1 验证」，不是「APK 里没有 v1 文件」——产品壳 APK 实测仍带 `META-INF/*.SF` / `.RSA` / `MANIFEST.MF`，`.SF` 里有 `X-Android-APK-Signed: 2, 3`。门禁只断 v2/v3，不假设「没有 v1」。
2. **build-tools 36 默认产出 v4 的 `.apk.idsig`**，必须显式关掉，否则 artifact 多一个必须一起分发的文件。
3. **`aapt2 dump badging` 不再输出 `sdkVersion`/`targetSdkVersion`**（这个 manifest 形状下），minSdk/targetSdk 要读 `aapt2 dump xmltree --file AndroidManifest.xml`。
4. **`tar` 不能在自己的输出目录里打包自己**（`file changed as we read it`）；`sha256sum` 与 `unzip -l` 也不能带绝对路径，否则 artifact 带出构建机路径。
5. **`aapt2 dump xmltree` 把布尔属性打成 `=true` / `=false` 字面量**（build-tools 36 实测，2026-11-01），而 int 属性打成十进制 `=26`（同一次实测）。按「老格式 `(type 0x12)0x1`」写判据会误杀显式 `debuggable="false"`；`ci/guard/check_shell_apk.sh` 两种拼法都认（#46 的反证 run 抓到的）。

另：`shell-apk` 加入后首个五腿全绿 run 是 [41285991691](https://github.com/bingnnvjn/nvn-native-android/actions/runs/41285991691)（2026-11-01），`shell-apk` 52s。

另外：runner 已强制 node24，v4 系列 action 仍工作但报弃用警告；升级到 v7/v8 是独立动作，不在本票。

## 10. 仍未定

- `release.p12` 的生成与保管流程（开 Release 时定）。
- d2r 血统壳（若将来重提）的 `ci/dmgendir` 替身在游戏版本变化后怎么再生成（#15 的已知耦合）；`dm/` 树尚未入库（无 `.gitmodules`）。#46 的产品壳走无 Gradle 路径，不依赖 `dm/` 或该替身。
- `load-plan.json` 的 schema 归属（ARCH 说 L0 产、L3 消，#27 说另票）。
- `nva-pack` 的 Windows 构建是否要在 CI 里出（当前只需 Linux/本机）。
- artifact 保留期是否跟 `main` 的节奏调（30 天够不够装）。

# 公开边界与个人数据（#33）

> 来源：票 #33。状态：v1（2026-10-31）。
> 依据：#22（里程碑证据全私有）、#16（公开边界）、#21（流水线）、#23（法律边界）、#31/#34（私有证据）。
> 本文件只记录决定与清单。它不执行清理，不重写历史。清理见 #35。

## 0. 口径

- **公开**：可以留在仓库并推到 GitHub。
- **私有**：只在本机私有根，仓库与远程都不放。
- **灰色**：有争议；第 1.4 节逐项定。
- 分类只按「个人数据与公开边界」。法律评级归 #23；不重写历史；清理另开票。
- 记录语言中文。

## 1. 三类清单

### 1.1 公开（现状即可）

| 组 | 内容 |
|---|---|
| 源码与接口 | `spike/abi-slice/{src,guest,include,android,scripts}/`、根 `include/`、`Cargo.toml`、`nvn-table.json`（已在 `DERIVED.md` 登记） |
| CI 与门禁 | `.github/workflows/ci.yml`、`ci/guard/*`、`ci/signing/fingerprint.txt` |
| 文档 | `README.md`、`GLOSSARY.md`、`CODING_STANDARDS.md`、`docs/{ARCHITECTURE,PIPELINE,GATES,RISKS,DERIVED,RELEASE-NOTES-TEMPLATE}.md`、`spike/abi-slice/DESIGN.md` |
| 研究结论 | `research/r1–r7`（远程 `research/round-1`，追认公开，见 Q3） |

公开不等于已验证有效。`scan_magic.py` 与 `scan_tree.sh` 的缺陷在第 5 节修。

### 1.2 公开（改写后）

| 文件 | 改写 |
|---|---|
| `README.md` | 删 `../work/...` 死链 |
| `AGENTS.md` | 保留本地指针，标注「本机路径，仅本机有效」 |
| `docs/PIPELINE.md` | 「密码写在 workflow 里」→「默认值在 `build_apk.sh`，可用环境变量覆盖」 |
| `docs/device-adreno725.md` | 删机型代号、报告 id、`pipelineCacheUUID`；留机型名、驱动、API、能力表 |
| `docs/botw-assets.md` | 删本机 XCI/keys 路径、盘符、剩余空间、「keys 能解开」与「绕过」措辞；留 ProgID、版本、大小、结构结论 |
| `docs/shared-fonts.md` | 删本机固件/keys 路径与可复制命令；留字体类型、大小、SHA-256、与上游一致的结论 |
| `docs/botw-nvn-usage.md` | 删 `extract_exefs.py --keys prod.keys` 步骤；留模块大小与符号结论 |
| `spike/abi-slice/README.md`、`RESULTS.md` | 机型代号改机型名；删 png 链接 |

### 1.3 私有

| 类 | 内容 |
|---|---|
| 真机证据 | 截图、录像、logcat、JSON、trace、`work/evidence/*`；例外：3 张已公开的 png（Q5） |
| 游戏素材 | XCI/NSP/NCA/NSO/keys/import pack/shader cache/字体/RomFS 索引；外置素材盘 |
| 存档 | 游戏存档与导出 |
| 真标识 | 真正的序列号、IMEI、Android ID、账号、邮箱、主机名 |
| 含本机路径的笔记 | `work/` 草稿、`work/research`、`work/ctx`、`repo/research/r8` |
| 工作区根一次性材料 | `d2r-android-evidence.md`、`tmp-*.md`、`fonts/`、`research/` |
| 构建产物与私有工作树 | `dist/`、`target/`、`apk-artifact/`、`apk-final/`、`work/tree-main` |
| 未来的发布密钥 | `release.p12` 与发布签名 |

### 1.4 灰色决定

**Q1 `ci/signing/artifact.p12`：维持公开。**

- 它是真 PKCS12 私钥，可用；默认密码在 `build_apk.sh`；5 个远程 ref。
- 任何有读权限的人可签出同包名、同证书、可覆盖安装的 APK。
- 「非发布信任」是文档承诺，release 门禁未接线。
- 补文案：公开密钥、可签更新包、CI artifact 不是可信分发物。

**Q2 `docs/DERIVED.md`：公开。**

- 只登记 `nvn-table.json`；`check_derived_ledger.py` 对 `docs/` 与 `.md` 整段豁免。
- 公开是 #16 的披露要求；内容只有标识符。
- 文档内派生表（ProgID、字体哈希、模块大小）在第 5 节加披露。

**Q3 `research/`：r1–r7 公开，r8 私有。**

- 远程 `research/round-1` 有 r1–r7（r5 只在远程）；本机未跟踪目录有 r1–r4/r6–r7 同内容 + r8。
- 3 处本机路径已公开；只在新提交清洗，历史保留。
- 未来研究笔记先进私有根，结论再进 `docs/`。

**Q4 设备标识：三样都删。**

- 机型代号 4 处；报告 id；`pipelineCacheUUID`。
- 从公开文档删；只留「Redmi Note 12 Turbo / Adreno 725 / 驱动 / API / 能力」。
- 报告 id 与 UUID 记进私有证据；外部报告本身已公开，删链接不撤回。
- 历史同 Q5。

**Q5 3 张 png：删工作树，历史保留。**

- `device-{abort,all,gate}.png` 在 6 个远程 ref；内容是机型代号、断言、指针、时间；无序列号/账号/路径。
- `RESULTS.md` 链接它们；删除前先改链接。
- 删工作树对齐 #22「仓库零验收数据」。这不是隐私修复；历史仍在 6 个 ref；要消除只能重写历史，已排除。

**Q6 三份文档：公开，但只留结论。**

- `botw-assets`、`shared-fonts`、`botw-nvn-usage` 含 keys/提取命令与「绕过」措辞。
- 删本机路径、可复制命令、解包步骤；留标题 ID、大小、结构结论、哈希。
- 法律评级归 #23；这条只解决「公开内容里有什么」。

## 2. 私有根

- 绝对路径不入库。结构：

```
<private-root>/
  README.md
  evidence/<ticket>/
  assets/MANIFEST.md
  saves/
  notes/
```

- keys 不进 agent 工作区。它们留在外置素材盘；`assets/MANIFEST.md` 只记路径、大小、SHA-256。不复制。
- 证据、存档、笔记可留在工作区内；规则见第 3 节。
- `nvn-private/.gitignore` 内容 `*`，防止工作区根以后 `git init` 把私有根纳进工作树。
- `ci/guard/check_private_root.sh <path>`：
  - 拒绝 `/mnt/*`、`/media/*`、`/run/media/*`。
  - 拒绝路径含 `OneDrive`、`Dropbox`、`Google Drive`、`iCloud`、`Nextcloud`、`Syncthing`。
  - `findmnt -no FSTYPE --target` 必须是 `ext4|btrfs|xfs|zfs`；路径不存在时取最近存在父级。
  - 不能在 git 工作树里。
- 自检：`/mnt/x` → 1；私有根 → 0。
- 红了：私有根在云盘/Windows 盘/git 树里；移动后再建。
- 查不到：云客户端在本地 ext4 上同步；`findmnt` 看不出来。换机器或换同步工具后重跑。

## 3. 人工纪律

1. 截图/录像/logcat 先落 `evidence/<ticket>/`；仓库只放结论。
2. 不把盘符路径、`/mnt/`、`/home/` 粘进任何 tracked 文件。
3. 不 `git add -f` 被忽略的文件。
4. 设备型号、报告 id、UUID 只进私有证据。
5. 私有根不进云同步目录；换机器/换同步工具后重跑检查。
6. 不在 GitHub Issue/PR/Release 上传截图或日志。
7. 不重写历史。
8. agent 不主动读私有根；搜索只指向 `repo/`；证据内容不粘进会话。

## 4. 检查不了的后果

- 门禁只在配置的分支/PR 上跑；未列分支的 push 不检查 → 内容可能进历史。
- Issue/PR/Release 上传不经过仓库门禁 → 公开且永久。
- `git add -f` 绕过 `.gitignore`；内容扫描只认已知模式。
- 加密/压缩/隐写/EXIF 不查。
- 私有根被移进云同步目录 → 没有脚本能阻止。
- 历史已公开的内容撤回不了（不重写历史）。
- 二进制只查魔数/大小，不查内容。
- 清理 PR 可以同时改门禁脚本；单人仓无法用 CODEOWNERS 强制第二人。

## 5. 仓库防线改动

| 文件 | 改什么 | 自检 | 红了意味着 |
|---|---|---|---|
| `.gitignore`（根，新建） | `**/dist/`、`**/target/`、`**/evidence/`、`/research/`、`*.p12` + `!ci/signing/artifact.p12`、`*.keys`、游戏后缀、`*.log` | `git check-ignore -v foo.p12 foo.xci evidence/a.png research/r8.md` 全中；`git check-ignore -q ci/signing/artifact.p12` 退出 1 | 规则写错，该私有文件可被 `git add` |
| `scan_tree.sh` | 修历史循环：读原始行、`[[ $line == *" "* ]]`、`p=${line#* }`；媒体/压缩包/`evidence/` 只查工作树；调用 `scan_personal.py` | `guard: scanned <tree> tree paths, <history> history paths` 中 history 等于 `git rev-list --objects --all \| awk 'NF>1' \| wc -l`；清理后整树 0；临时 `foo.png` 1 | 树/历史里有被禁路径；先删或加 allowlist |
| `scan_magic.py` | key 正则 `^[A-Za-z0-9_.-]*(key\|kek)[A-Za-z0-9_.-]*\s*=\s*[0-9a-fA-F]{32,}\s*$`；去掉 `docs/` 豁免；扩文本后缀；`--history` 只扫 blob、按 object id 去重、保留 `ci/signing/` 与 `ci/fixtures/` 豁免；媒体魔数只工作树 | 合成 `master_key_00 = …` → 1；`--history` → 0；png → 1 | key/游戏魔数进树或历史；历史命中登记为事故 |
| `scan_personal.py`（新建） | 只形状规则：盘符、`/mnt/`、`/home/<user>/`、`/Users/`、`displayreport.php?id=[0-9]+`；不建公开 denylist | 临时文件写 `/mnt/x/x` → 1；清理后整树 0 | 本机路径或报告 id 进了公开文件 |
| `check_artifact.sh` | evidence 允许 `.txt/.json/.log/.sh/.so/.tar/.tar.gz/.tgz/.zip` 与无后缀 ELF（`\x7fELF`）；禁媒体/私钥；路径检查只查 `/home/`、`/Users/`、`/mnt/`、`C:\`（不含 `/tmp/`）；tar/tgz/zip 解包后检查 | 临时目录放 `a.png` → 1；`a.txt` 写 `/mnt/x/x` → 1；干净目录 0 | artifact 会公开媒体/路径/密钥 |
| `check_docs.sh` | 复用 `scan_personal.py` 扫 tracked Markdown（盘符、WSL 挂载、home/Users、数字报告 id）；保留免责句断言 | 临时插入 → 1；清理后 0 | 文档又写回本机值或报告 id |
| `test_guards.sh`（新建） | 对 `scan_magic.py`、`scan_personal.py`、`check_artifact.sh` 做正反例自测；CI `guard` job 先跑它 | 全部用例通过；故意放宽规则应红 | 门禁被改弱却没人发现 |
| `check_derived_ledger.py` | 不改脚本；在 `DERIVED.md` 加「文档内派生表」一节（ProgID、字体哈希、模块大小） | `python3 ci/guard/check_derived_ledger.py` → 0 | 账本行与文件对不上 |
| `ci.yml` | `guard` 已 `fetch-depth: 0`；`gate.needs` 已含 `guard`；`on.push.branches` 补 `integrate/main`、`risks/v0`、`research/round-1`；`arch/l0-l6` 的 workflow 缺口写进 `docs/PIPELINE.md` §2 | 核对 `on:`；向测试分支 push 一次 | 长期分支绕过门禁 |
| 分支保护/旧 workflow | 已补 `LICENSE`/`NOTICE`；main 的 required check/禁 force-push 与 `spike/abi-slice` 旧 workflow 钉 SHA 是仓库外/另一分支动作，缺口写进 `docs/PIPELINE.md` §2 | 人工核对 GitHub 设置 | 历史可被重写、红可合、许可缺失 |

## 6. 已公开外露的处置

| 外露 | 处置 | 残留 |
|---|---|---|
| 4 个文档里的本机路径/机型代号/报告 id/UUID | 在新提交改写；不进未来提交 | 历史里仍在（4 个远程 ref） |
| 3 张设备截图 | 删工作树；复制进私有根；先改 `RESULTS.md` 链接 | 历史里仍在（6 个远程 ref） |
| `docs/part1.md`（旧名）的本机路径 | 新提交已不带 | 历史里仍在（4 个远程 ref） |
| `research/round-1` 的 3 处本机路径 | 只在新提交清洗 | 历史里仍在 |
| `ci/signing/artifact.p12` | 维持公开；补警示 | 永久在历史（5 个 ref） |
| gpuinfo 报告与 UUID | 从文档删；外部报告已公开 | 外部页面撤不回 |

## 7. 后续

- 清理与防线落地：见 #35。
- 分支顺序：清理 PR 基于 `origin/main`，不基于 spike。本地 #31/#34 的 5 个提交先推/并，或清理后 rebase；`origin/spike` 的 AGENTS 提交会冲突。3 张 png 在清理后不会因合并 spike 回来。
- 已知缺口：清理 PR 可改门禁脚本；`research/round-1`、`arch/l0-l6` 没有 workflow；云客户端在 ext4 上 `findmnt` 抓不到；历史残留按第 6 节接受。
- 下一步：`/to-spec`。

## 8. 变更记录

- v1（2026-10-31）：首版。三类清单、私有根、防线清单、外露处置。

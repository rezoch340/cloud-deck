# Cloud Deck · 新 session 接手说明

更新时间：2026-11-18
正式项目：本仓库根目录（`.`）
原项目：`../../remote-ssh/ComfyUI-colab`（相对本仓库根目录）

## 2026-11-18：macOS ad-hoc 分发打包

- [#3 的 macOS 下载说明](https://github.com/rezoch340/cloud-deck/issues/3#issuecomment-6990318320)已附 [v0.1.2 Apple Silicon DMG](https://github.com/rezoch340/cloud-deck/releases/download/v0.1.2/CloudDeck-0.1.2-macos-arm64.dmg)。该映像从标签提交 `72e26d0` 独立构建，挂载后验签、内嵌界面检查和随包资源比对通过。GitHub 下载件与本地逐字节一致。
- 新入口 `node client/scripts/run.mjs macos-package`：核对四处版本号，构建当前 Mac 架构的默认 release，强制使用 `-` 签名并忽略个人签名及 Apple 公证环境变量。`desktop-build` / 原生测试仍保留原有个人签名选择。macOS 继续本机构建，没有扩展 Windows CI、升级版本或发布 Release。
- 新增 `client/scripts/package-macos.mjs`：Tauri 签内部代码，staging 副本清理扩展属性并完成最终签名；压缩前严格验签，通过 `ditto --norsrc --noextattr --noacl --keepParent` 生成 ZIP，解压后重新验签、核对版本和架构、执行 `--check-frontend-assets`。通过后才输出 ZIP、`SHA256SUMS.txt`、`INSTALL.md` 和 `verification.json`。安装说明在 `client/macos-install.md`，明确未公证、quarantine 处理及 ad-hoc 更新可能再次要求钥匙串授权。
- 本机 arm64 构建、TS/Vite、压缩前后签名/版本/架构/内嵌界面检查、脚本语法与 diff 检查通过。产物：`/Volumes/CODEX_SCRATCH/build/cloud-deck/client/artifacts/macos-arm64-L4tLNo/CloudDeck-0.1.2-macos-arm64.app.zip`，SHA-256 `873d18c83998eb10ef880360ef539c32059f7049d6620c5c50052bac18779e0d`。这不代表 Apple 公证、另一台 Mac 的 Gatekeeper 验收或 Intel/universal 验证；未操作 Google 授权、云端实例或个人凭据。

## 2026-11-16：Windows 发行包启动报 ERR_FILE_NOT_FOUND

- 基于 `main` 的 `9f3d835` / v0.1.1，新建 `codex/fix-windows-frontend-assets`。用户确认公开 Release 的 Windows x64 便携版启动即报错；下载该包后，在主程序里找到构建机的 `d:\a\_temp\cloud-deck-work\build\cloud-deck\client\dist`。
- 根因是 runner 将盘符绝对路径传入 `frontendDist`，Tauri 将其反序列化为外部 URL，跳过资源嵌入；运行时尝试加载用户机器上不存在的构建目录。参见 [Tauri #14005](https://github.com/tauri-apps/tauri/issues/14005)。缺少 `cloud/` 并不是本次启动错误的原因；该目录保存云端脚本与应用清单，安装版由安装器部署，便携版随 ZIP 解压。
- runner 新增 `client/dist` → 外部构建产物目录的链接（Windows junction），Tauri 始终使用 `../dist`。保留外盘构建要求，并支持源码和缓存位于不同磁盘。
- 新增 `--check-frontend-assets`，使用与窗口启动相同的 Tauri context，确认配置为本地目录且内嵌首页、首页引用的 JS/CSS 均可读取。该入口不打开窗口、不启动 MCP 或访问账号；CI 在构建 Windows 程序后执行，失败阻止发布。
- 定向 Rust 回归 2/2、TS/Vite 构建、macOS 原生离线 UI 10 项、内嵌资源自检、Rust 格式与 diff 检查通过。原生报告为 `<工作根>/tmp/cloud-deck/client/native-smoke-0urVRt/native-check.json`。Windows x64 NSIS 交叉编译通过，便携 ZIP 完整性及 7 份云端资源与源码逐字节核对通过；新二进制包含当前 JS/CSS 资源键，旧发布包没有，且新包不含错误的构建机加载地址。
- 本地修复测试包和 `verification.json` 位于 `<工作根>/tmp/cloud-deck-windows-startup-20261116/artifacts/`。便携包 `CloudDeck-0.1.1-windows-x64-portable-startup-fix.zip`，SHA-256 `05b32ab6ec341bfc54402f774103f1132df8d47bbd85cef3600d38729a2b4d62`；安装包 `CloudDeck-0.1.1-windows-x64-startup-fix-setup.exe`，SHA-256 `977d2976e1c2e37dac7233d8a2f8156e3ccc188acd7d2d9ed88085cb93279b81`。这些文件是正式发版前的 0.1.1 测试产物。
- 用户已授权提交、推送、合并并发布 v0.1.2。四处版本元数据已统一为 0.1.2；正式包由标签触发 Windows Runner 构建并执行内嵌界面检查，结果以 [v0.1.2 Release](https://github.com/rezoch340/cloud-deck/releases/tag/v0.1.2) 与对应 Actions 记录为准。Windows WebView2 实机启动仍待用户验收，本轮未操作云端。

## 2026-11-16：通用创作应用与保存记录

`codex/creator-workflows` 增加可选音频、视频、LLM、工作流分类和筛选，应用卡片可直接打开已就绪的本机入口。SSH 无活动超时从 90 秒延长至 420 秒，覆盖 Drive 交互授权最多 300 秒的等待。清单准备命令默认折叠，避免较大的内嵌安装脚本占满文件页。

文件页将未观测状态标为「本次未查询」。成功的 Drive 服务端校验记录单独落盘，按 Google 账号隔离显示，关机或重启后仍可查看。历史记录不更改实时同步状态，不参与关机放行或零计费判断；旧版本未留存的历史凭证不会补造。

在 Apple Silicon Mac 与一台真实 Colab G4 上完成本人参考音克隆、分段语气配音、Wan2.2 视频、音画合成和 Qwen3 接口生成。文件经 Drive 服务端大小与 MD5 校验，关机后云盘 WAV 与本机 MP4 播放通过，实例数和费率均归零。具体个人素材与账号账单保存在私有工作区。原创工作流组件独立于本仓库，未把无公开许可证的上游源码重新许可。

新增历史记录回归在本机通过，尚未再开 G4 验证该落盘字段。原有端到端链路与这项后补回归分开计。Windows 实机及全部第三方插件仍未验证。

## 当前分支：Git 标签自动发版、客户端自更新、Windows 滚动条与项目文档重做（2026-11-15，`codex/git-release-updater`）

- 自动发版与版本同步：新增 `.github/workflows/release.yml` 与 `client/scripts/sync-version.mjs`（含 `sync-version.d.mts`），并新增 `.gitattributes` 锁定 LF 行尾以确保 Windows Runner 检出时不会把内嵌至 Rust 二进制的 `start.sh` / Python 云端脚本转为 CRLF。按用户要求，GitHub Actions 发版工作流仅自动构建 Windows x64（`CloudDeck-<ver>-windows-x64-setup.exe` 与 `CloudDeck-<ver>-windows-x64-portable.zip`），同步 `client/package.json`、`client/src-tauri/tauri.conf.json`、`client/src-tauri/Cargo.toml`、`client/src-tauri/Cargo.lock` 四个文件，运行测试并生成 `SHA256SUMS.txt` 后在推送 `v*` 标签时发布 GitHub Release（macOS 应用由本机自行构建；`workflow_dispatch` 仅上传产物演练，不创建正式 Release）。
- 客户端更新检查与自更新：新增 `client/src-tauri/src/update.rs` 并在 `lib.rs`、`api.ts`、`domain.ts`、`App.tsx`、`WorkspaceShell.tsx`、`WorkspaceViews.tsx` 接入 `check_update` / `install_update` / `open_release_page`、底栏 `可更新 vX.Y.Z` 胶囊及设置页“版本与更新”卡片（支持 `CLOUD_DECK_AUTO_UPDATE_CHECK=0` 关闭启动 3 秒后自动检查）。仅信任 HTTPS GitHub 官方允许域名并跳过 draft/prerelease；Windows 下通过同目录 `uninstall.exe` 区分 `windows_nsis` 与 `windows_portable`，NSIS 版在本地校验 `SHA256SUMS.txt` 与安装包字节上限/SHA-256 通过后以 `/P /UPDATE /R` 静默升级并重启，Portable 与 macOS 引导至浏览器对应 Release 下载。
- Windows 滚动条与对齐修复：调整 `client/src/styles.css`、`WorkspaceShell.tsx` 与 `WorkspaceViews.tsx`，为 `[data-platform='win']` 启用透明轨道细胶囊圆角滚动条并隐藏箭头按钮，同时配置 `scrollbar-gutter: stable` 与右侧内边距补偿，消除 WebView2 经典灰轨滚动条与右侧卡片错位。
- 项目文档与配图重做：新增 `docs/images/` 7 份图集（`hero-banner.svg`、`architecture.svg` 及脱敏示例界面图 `workspace-overview.png`、`applications-hub.png`、`app-detail-logs.png`、`files-sync-verified.png`、`settings-updater.png`），重写根 `README.md` 并同步更新 `client/README.md`（脱敏示例记录占位符并补充发版与更新章节）。
- 本地交叉编译与离线验证：Windows 发行包已在本地交叉编译至 `/Volumes/CODEX_SCRATCH/build/cloud-deck/client/target/x86_64-pc-windows-msvc/release/`（主程序 `cloud-deck.exe` 17 MiB 及 `bundle/nsis/Cloud Deck_0.1.0_x64-setup.exe` 4.3 MiB）。当前离线验证通过：`sync-version.mjs 0.1.0 --check`、`format-check`、前端 `16/16`、`web-build`（`tsc --noEmit` + Vite）、`rust-test` `43/43`（含 `update` 7 项测试）、`agent-test` 41 通过 / 1 Linux 专属跳过（42 项）。后续已合并回 `main` 并推送 `v0.1.0` 标签触发 GitHub Release Windows 构建；Windows 实机安装升级仍待 Windows 环境真机验收。

## 当前分支：本机 MCP 第一版（2026-11-15，`codex/mcp-server`，本机冒烟通过）

- 客户端内置 MCP 服务（`client/src-tauri/src/mcp.rs`），监听 `127.0.0.1` 随机端口，令牌和地址写入数据目录 `mcp.json`（0600）。拒绝无令牌/错令牌（401）和带 `Origin` 的请求（403），请求体上限 1 MiB。工具直接调用现有 Tauri 命令，与窗口共用状态。
- `cloud-deck --mcp-stdio [--data-dir <目录>]`（`mcp_bridge.rs`）是给 Codex 等 MCP 客户端的 stdio 桥接，每次请求重新读取 `mcp.json`，客户端未运行时返回 -32000 中文错误。
- 工具：工作区/状态刷新、应用清单读取/说明/校验/保存/删除、启动（等待就绪/完成/失败并附日志）、停止、日志。按用户要求暂不提供申请/释放实例和登录工具，等界面确认可用后再加，届时需要窗口内确认。
- 同分支修复：部署后主卡片在应用失败或退出时一直停在“正在等待云端健康检查”，现在会显示启动失败、进程已退出或任务已结束（`cloud.rs::awaiting_outcome`）。
- 验证：Rust 36/36；隔离数据目录启动 debug 包，HTTP 与 stdio 桥接均完成 initialize、tools/list（10 个工具）、get_workspace、清单校验（正确/错误）、未连接实例时启动返回错误；401/403/202/-32601 均符合预期。未连接真实 Colab 实例做 MCP 部署，Windows 未验证。

## 当前修订：云端会话活动与所有应用的实时日志（2026-11-14，离线通过，真实验证暂缓）

用户纠正：实例保活必须在云端运行，不依赖客户端开着；日志属于所有应用的通用能力。用户随后表示去睡觉，要求需要其确认或操作时放缓；本轮只完成本地修改、离线检查及打包，未申请新实例、未变更 Google 授权，不发送手机通知。真实云端测试留待用户回来。用户再次明确要求：验证完成必须保存并校验、关闭测试实例，并查询确认实例数及每小时计费均为 0。

- 关键纠正：Google 官方 2026-09-25 [PR #144](https://github.com/googlecolab/google-colab-cli/pull/144) 已明确旧 `/keep-alive` ping 是 no-op，平台改按 kernel 活动和连接维持存活。下方历史轮次的 HTTP acknowledged 不能作为有效保活证据，之前结论过强。已删除 `auth.rs::keep_alive`、Rust 心跳定时器及旧 `heartbeat.rs`。
- 新 `cloud_heartbeat.py` 由云端 `cloud_agent.py --heartbeat` 启动独立线程，每轮间隔 60 秒，连接实例的 Jupyter 回环 API，复用/建立独立 kernel，发出轻量执行并核对本次 execute_reply。异常保留上次成功记录并下轮重试；不读取或上传桌面 OAuth token。状态/次数写入云端 `heartbeat.json` 并经 `/v1/state` 返回。客户端退出、休眠不停止云端线程；界面显示的是 kernel 活动确认，不承诺 Google 永不回收。
- 开机打包/上传三个通用文件：agent、lifecycle、heartbeat；版本摘要包括全部三者。所有服务/任务都复用 `ApplicationLogs.tsx`，部署/运行日志直接显示在详情页，每 2 秒读取，支持暂停跟随；取消原先一次性日志弹窗及部署期间禁用入口的行为。
- `cloud_lifecycle.py` 给所有清单记录资源检查/下载进度/校验/回退、恢复路径、准备命令序号/输出/退出码。`cloud_agent.py` 记录主命令、启动/就绪/停止事件，日志接口同时返回当前阶段与进度。命令 stdout/stderr 留在云端，重连可继续读取；默认不自动保存到 Drive，释放实例后不保留。读取失败保留最后内容并明确错误。
- 离线回归验证了无客户端轮询的独立心跳、失败重试/匹配响应/超时、普通非 Comfy 脚本的执行中输出、命令失败状态与重新读取。另捕获 macOS 上短进程退出瞬间 killpg 返回 EPERM 的竞态：只有确认进程组已无活动成员才忽略，活进程权限错误仍阻止保存。修正后相关保存回归连续 15 次通过。
- 验证：Python 41 通过/1 Linux 专属跳过（42 项）；Rust 30/30；前端 14/14；原生离线 UI 9 项通过（包含自定义任务的共用日志面板）；TS/Vite 构建通过。工作根仍为 `/Volumes/CODEX_SCRATCH/tmp/cloud-deck-drive-consent-20261114`，新日志名 `cloud-heartbeat-*.log`。原生报告 `tmp/cloud-deck/client/native-smoke-Fhz8Mt/native-check.json`。本轮没有 Linux 实机测试，不能沿用旧 36/36 作为当前源码结果。
- 待真实验证：Colab 实例自带 websocket-client/回环 Jupyter 兼容性，关闭所有桌面/MCP轮询后云端确认次数持续增加，应用真实下载/命令日志实时刷新、重新连接继续读取，最后保存/释放并确认计费。Google 平台最长寿命、余额与资源限制仍适用，几轮 kernel 回复不能证明无限保活。
- 新本地候选交付目录为 `artifacts/cloud-heartbeat-logs/`；个人签名、7 份云端资源逐字节校验、ZIP 解包签名均通过，尚未做云端验收。ZIP `CloudDeck-0.1.0-cloud-heartbeat-logs-macos-arm64.zip` 为 6,148,293 字节，SHA-256 `e0754c5fb39cad2bd7f8f56ce71a9d54109745b9d51a2b7af3dc9ca19863ff95`。原 `artifacts/readiness-heartbeat/` 为上一轮版本。当前旧 release 未主动替换或重新连接云端；用户回来后再继续真实检查。未 commit/push。

## 上轮记录：普通命令、就绪检查与客户端会话活动（2026-11-14，真实验收通过）

本节取代下面历史版本的 `installation` / `usePreparedLaunch` 设计及固定 HTTP 检查。用户要求清单通用、执行器只运行 Linux 命令或任意脚本；应用说明仅只读展示。用户所说心跳明确指发给 Google 的实例保活。

- `client/applications/comfyui/manifest.json` 声明运行命令、三个模型资源、恢复/准备命令、保存规则、只读 `details` Markdown 与 `runtime.readiness`。已删除 `documents`、`installation` 和旧客户端 `comfyui.json`；根清单和 Swift/Python 仅保留作迁移基线。
- ComfyUI 自己的 `setup.py` / `start.sh` 保存固定版本安装及启动逻辑，34 项插件来源保留，排除 DLSS；新客户端不打包旧 bootstrap。开机只部署通用守护与执行器，部署应用时才发送相邻脚本，不在开机阶段安装应用。
- `details` 是任意应用共用的只读 Markdown，支持表格、忽略 HTML，不生成可编辑字段。无下载/无 Drive 的完整例子在 `examples/command-app/manifest.json`。通用格式与命令说明在 `client/applications/README.md`。
- `runtime.readiness` 支持 HTTP 精确状态码、TCP 连接、普通命令或命令指向任意脚本（退出码 0 成功）。总时限/间隔/单次时限可配置，默认 120/2/5 秒；ComfyUI 总时限 180 秒。准备与下载不计入就绪时限，失败重试，超时保留主进程。慢探测在线程中执行，不锁住其他应用；停止/保存先终止探测进程组。旧 `healthPath` 兼容读取；无网页的服务无需端口，一次任务使用退出码判断完成。
- Google 保活由 Rust 后台在受管连接建立后自动发送：每轮等待 60 秒，再执行 Colab TFE keep-alive 和独立 Jupyter kernel 的轻量请求；忙碌的账号/实例操作会延后该轮。Google 确认、响应超时、kernel 确认、最近尝试和成功时间分别记录，失败不代表关机或零计费。切换页面不影响；释放/断开受管实例后停止。客户端退出、电脑休眠或断网时不能保证持续发送，不能保证绕过 Google 平台限制。自动重连、托盘尚未实现。
- 用户要求个人签名，runner 自动选择本机唯一有效开发/分发身份，或使用 `CLOUD_DECK_SIGNING_IDENTITY`；证书/私钥不复制进仓库。debug/release designated requirement 相同，真实新构建读取授权及刷新通过，没有再次要求钥匙串密码。启动只读公开 workspace，凭据按需访问，成功/缺失/拒绝在进程内缓存。
- 无保存规则时停止应用，用 `no_save_rules` 凭证放行，不访问 Drive；只要配置规则（包括空目录），仍须服务端校验。修复短任务退出时的 ProcessLookupError 竞态，并保留权限/进程归属检查。实测还发现停止成功响应残留旧 readiness 错误会让客户端中断后续启动，已清除当前错误、保留事件历史，并覆盖失败→停止→重启回归。

### 最新验证与实例清理

统一工作根 `/Volumes/CODEX_SCRATCH/tmp/cloud-deck-drive-consent-20261114`，所有构建/缓存/日志/临时产物在此。持久用户工作空间继续使用 `/Volumes/VoltDrive/AI/data/cloud-deck/client/`。

- 本地前端 14/14，Rust 32/32，Python 35 通过、1 Linux 专属跳过；同套最新源码在 T4 隔离目录 Linux 36/36 全通过。原生离线 UI 8 项、格式、TS/Vite、个人签名 debug/release 构建通过。离线报告 `tmp/cloud-deck/client/native-smoke-mElHWO/native-check.json`。
- 本轮 T4 `gpu-t4-s-usw4a2-23n7k39lhsoth`，实际地区 `us-west4`。最初开机只部署守护、Drive 浏览器授权返回同一请求续跑；三个模型均实际从外部下载并验大小/SHA-256。首次全新环境准备约 33 分钟，插件首次加载额外安装依赖导致旧 120 秒启动检查超时；后续复用同一实例，不重复分配或下载大模型。
- 最终报告 `tmp/cloud-deck/client/live-check-fJCFUv/native-check.json` 为 passed=true：Google 和 kernel 连续确认保活；重复连接复用同实例；再次独立下载 17,324,914 字节的既有 upscaler 并验 SHA-256；正式清单与云端清单一致；固定 ComfyUI 0.30.0 / Git 提交、Tesla T4 CUDA、三模型及远端/本机 HTTP 就绪通过。没有把本地资源复用冒充外部下载。本轮未重新执行图片生成工作流。
- 保存 138 个文件、233,125,188 字节，经 Drive 服务端大小/MD5 校验；再次核验后释放该 T4，最终本次查询确认 0 台实例、0 点/小时。未保留运行中的 GPU。`live-check-Pgljz4`、`qzkbRo`、`TjTEa9` 是中途调试/失败报告，不是最终验收结果。
- 部分第三方插件仍存在依赖/缺模型警告（例如 requirements-parserx、simple-lama-inpainting），核心服务验收不代表全部插件功能通过。Windows 尚未编译/实机验证。Vite 仍有大 chunk 提示。

### 最新交付包

`artifacts/readiness-heartbeat/Cloud Deck.app` 与 `CloudDeck-0.1.0-readiness-heartbeat-macos-arm64.zip`（均在上述工作根）。ZIP 6,166,073 字节，SHA-256 `a55eca3f4bf9da3f03773bb2d8fbbf29e691539f459f579afd5b8630bd5003b2`。6 份随包资源与源码逐字节相等，默认构建不含 QA 驱动/旧 bootstrap，ZIP 解压后签名通过；个人开发签名、未公证。同目录附完整清单、指南、真实/离线报告、Linux 测试、保存释放凭证与 verification.json。

旧 release 与 QA 进程已关闭，最终 release PID 86096 已使用原持久工作空间打开；实际点击只读刷新后确认账号已验证、未发现实例、0.00 点/小时，未再次要求钥匙串密码。未 commit/push，未发送手机通知。

## 最新检查点：实例/应用分离与单文件生命周期清单（2026-11-14）

- 用户要求开机只选择 GPU/CPU，自动连接并部署守护进程；后续独立部署应用。已删除部署模式选项及开机 autoStart 执行，新增 `start_instance` / `connect_instance`。健康的同端点连接复用，重连只选指定的现有端点，绝不隐式申请。前端根据本次观测显示刷新、开机、恢复连接或部署应用。
- 内置应用以 `client/applications/comfyui/manifest.json` 为唯一客户端清单，含 runtime、installation、resources.files、startup.restore/commands、shutdown.save。按用户后续指令已移除 `documents`；固定插件配置直接放 installation，模型只在 resources.files 定义一次。准备脚本直接读取 `$CLOUD_DECK_MANIFEST_FILE`。保留旧根清单和 Swift/Python 控制器作迁移基线。
- 通用 `cloud_lifecycle.py` 按 sources 顺序尝试 HTTPS 下载或 Drive；失败/大小或 SHA-256 不符自动回退。有效本地资源校验后复用，原子替换，全部失败不启动应用。恢复只补缺失文件并保留实例修改；保存明确映射来源/目标，继续使用 Drive API 大小/MD5 校验和释放保护。ComfyUI 当前三个模型均外部下载优先、Drive 兜底，固定版本/34 插件记录/禁用 DLSS 保持。
- 前端全部恢复可读格式，新增 Prettier 3.10.1 与 `format` / `format-check`；工作台、文件页、生命周期展示、清单编辑器拆开。原生清单编辑、无效 JSON 保留原配置和内置只读清单查看均已验收。说明：`client/applications/README.md`。
- runner 的 macOS 默认 scratch 根改为 `/Volumes/CODEX_SCRATCH`，要求挂载 marker；当前构建使用 `CLOUD_DECK_WORK_ROOT=/Volumes/CODEX_SCRATCH/tmp/cloud-deck-drive-consent-20261114`。已有构建链接可安全重定向，仅替换符号链接、不删除目标目录。当前 `node_modules` / gen / icons 指向此工作根。持久工作空间继续使用 `/Volumes/VoltDrive/AI/data/cloud-deck/client/`。

### 本轮真实 ComfyUI 验收

用户明确授权用复杂的现有 ComfyUI 实验，并选择外部下载优先。初次只读确认当前 0 台实例、0 点/小时，因此本轮新建 T4 `gpu-t4-s-use1c0-1uqqdy3eorcm7`，实际地区美国南卡 `us-east1`，运行时观察消耗 1.15 点/小时。开机只部署守护进程，之后单独提交应用。

- 本轮浏览器 Drive 授权后，同一次启动请求自动续跑；用户明确回复已完成授权。三个清单模型的云端日志均记录 `source: download` 并通过大小/SHA-256 校验（两个 6938040682 字节模型及一个 17324914 字节模型）。没有把权重下载到 Mac。
- 用户在测试中要求删除 documents，完成后复用同一 T4 和已校验模型继续验收；最终清单执行的资源凭证正确标记为 local。另用隔离副本真实下载 17324914 字节的既有 upscaler，SHA-256 `141bd9b90c323f84cbeb17b4238f3b27df29fb43aeb09916aa6791d00e9352e4`，确认为 download。没有把本地复用冒充外部下载。
- 最终清单与守护实际应用配方完全相等；ComfyUI 0.30.0、固定 Git 提交、Tesla T4 CUDA、三模型大小、远端与本机转发 `/system_stats` 全部通过。初次原型准备曾失败，清理后的正式清单重新准备及启动通过；以最终报告为准。本轮没有再执行生成图片工作流。
- 真实 Linux 守护/清单测试 27/27 通过，包括进程组、失败不启动、资源顺序与校验回退、保留本地修改、明确保存目标。Mac 本地同套 26 通过/1 项 Linux 专属跳过；Rust 27/27、前端 11/11、原生 UI 7 项、TypeScript/Vite、格式检查、默认 release 构建通过。
- 最终按新清单保存 136 个文件、233105930 字节，Drive 服务端大小/MD5 通过；再次核验后释放这台测试 T4。实时查询确认 assignments 为空、每小时消耗 0，最终余额 2459.4967718509656。未保留运行中的 GPU。
- 最终报告：`/Volumes/CODEX_SCRATCH/tmp/cloud-deck-drive-consent-20261114/tmp/cloud-deck/client/live-check-0z7nk4/native-check.json`（passed=true）；同目录有原生截图和保存释放凭证。离线原生报告 `native-smoke-pqSzrN/native-check.json`（passed=true）。Windows 未验证；第三方插件仍有已有依赖/缺模型提示，不代表全部插件功能通过。

### 最新交付包

`/Volumes/CODEX_SCRATCH/tmp/cloud-deck-drive-consent-20261114/artifacts/manifest-flow/Cloud Deck.app` 及 `CloudDeck-0.1.0-manifest-flow-macos-arm64.zip`，ZIP 6091320 字节，SHA-256 `c42ef2307c7a629bc8bf7814aabee2a8ff233eef03fa14f16b916f0aae4d5555`。同目录附完整 ComfyUI 清单、真实/离线报告、Drive 保存凭证和 verification.json。6 份随包资源与源码逐字节一致，默认构建不含 QA 驱动；完整签名、ZIP 解压后内容及签名通过。临时签名、未公证；Vite 仍有大 chunk 提示。已关闭本轮 QA 进程并打开默认 release（PID 45899），沿用原持久工作空间；开机/连接/部署均需用户后续操作。未 commit/push。

## 最新修复：Drive 浏览器授权后自动继续启动（2026-11-14）

- 用户反馈启动期间在浏览器完成 Drive 授权后，流程没有继续。根因是 `cloud.rs` 取得授权链接后主动中断挂载 kernel 并返回 `Ok(false)`，上层结束启动，没有等待授权或自动恢复。
- 新增 `client/src-tauri/src/drive.rs` 管理同一条 kernel WebSocket：等待期间每 3 秒检查授权传播并处理心跳；确认成功才回复原授权请求，挂载成功后才继续提交应用。支持 Colab 仅以 parent session 路由的授权消息，忽略其他 session。浏览器等待限时 5 分钟，随后挂载限时 4 分钟；传给 Colab 的 `timeout_ms=600000` 为浏览器步骤留出时间。
- 超时、授权查询失败、kernel 失败或断开都会使启动失败，清除待授权链接并尝试中断本次挂载请求，实例保留。前端提示改为授权后自动继续；显式真人 QA 脚本也改成等待同一个启动请求，不再要求用户点击第二次启动。
- 本轮通过 Rust 21 项（含 6 项新的模拟 WebSocket 授权回归）、前端状态 7 项、TypeScript/Vite 和默认特性 macOS release 构建；应用签名校验及 4 份随包云端资源逐字节校验通过。没有进行新的真实 Google/Drive 授权、实例申请/释放或云端启动验证；Windows 未验证。
- 本轮遵守最新全局 scratch 规则，所有新增缓存、构建、临时文件位于 `/Volumes/CODEX_SCRATCH/tmp/cloud-deck-drive-consent-20261114/`，通过 `CLOUD_DECK_WORK_ROOT` 覆盖 runner 工作根。构建时临时调整的 `node_modules` / `gen` / `icons` 链接收尾后恢复原目标；没有更改 runner 的默认路径。
- 修正版：`/Volumes/CODEX_SCRATCH/tmp/cloud-deck-drive-consent-20261114/artifacts/Cloud Deck.app`；ZIP 同目录 `CloudDeck-0.1.0-drive-consent-macos-arm64.zip`，SHA-256 `7549881666b95e83865719f2dd875c7a85720dff41b72f86f871a1992b21aacf`。验证摘要 `verification.json`。仍为临时签名、未公证。
- 已打开修正版 PID `22038`，沿用 `/Volumes/VoltDrive/AI/data/cloud-deck/client/` 的持久工作空间。原生窗口已确认加载，显示保存账号与历史地区、状态/计费待确认；需刷新状态后重新尝试启动。当前云端实例和计费没有重新查询，不能沿用旧的零计费结论。未 commit/push。

## 当前检查点：原生部署链路与真人联调（2026-11-14）

用户已授权逐步实现 Windows/macOS 客户端，并补充：**HeroUI、组件复用、内置 ComfyUI 使用现有配置、随包云端守护脚本、构建缓存全部外盘。** 最新明确指令是“这些功能写完了以后开始测试真实的部署+OAuth 登录”，已授权当前真实授权与部署联调，不需重复询问。

### 当前实现

- `client/`：Tauri 2 + React 19 / TypeScript + HeroUI 3 / Tailwind 4。五个页面、固定外框、内部滚动，共用控件/卡片/编辑器。
- `auth.rs`：系统浏览器 OAuth、PKCE/state/nonce、ID Token 校验、Keychain/Windows Credential Manager、刷新及 Colab 查询。令牌不交给前端，不读取/覆盖旧 `auth/`。
- `cloud.rs` / `transport.rs`：实例查询/申请（只用本次 eligible GPU 或明确 CPU）、Rust `russh` + `tokio-tungstenite` 内置 SSH over Colab WSS、Google TLS 代理来源检查及连接主机密钥固定、四份脚本资源上传、守护进程部署、应用启动/停止/日志、本机回环转发与健康检查、Drive 挂载与授权重试。
- 应用状态和入口已接到 UI；可只部署守护进程，或按 `autoStart` 配置启动。连接中断不意味着应用停止或零计费。
- 保存流程已接通并在 T4 真实验证：停止完整应用进程组，复制配置的保存目录，直接查询 Drive API 校验服务端大小与 MD5。变化/缺失/异常会阻止释放；不同内容的旧目标保留版本备份，备份不再次同步或恢复，不传播删除。“保存并关闭实例”在保存通过后再次核验文件与进程，再释放并确认停止计费。
- `client/applications/comfyui.json` 及 `manifest.json` 保留 ComfyUI 0.30.0、34 条插件记录和原模型来源，排除 DLSS。`cloud_bootstrap.py` 的受管模式要求真实 Drive 挂载、只准备、不覆盖未同步本地文件，不自行启动旧同步/SSH。`cloud_agent.py` 是回环鉴权、独立进程组、PID 身份验证的 Linux 守护进程。
- `client/README.md` 已补，根 README 与设计文档已更新；历史基线说明保留。Windows 未编译/实机验证，不可写成通过。没有 commit/push。

### 构建、包与验证

- 统一入口 `node client/scripts/run.mjs <mode>`。macOS 工作根 `/Volumes/VoltDrive/AI/`；缺盘报错，不回落。依赖、dist、Cargo target、图标、schema 都在 `build/cloud-deck/client`；Cargo/pnpm/npm/clang/Tauri/XDG 缓存在外盘 `cache`，临时和验证数据在 `tmp/cloud-deck/client`。
- 项目 `client/node_modules`、`src-tauri/icons`、`gen` 为外盘链接。复用既有 Rust 工具链；runner 为 macOS 的 `rust-objcopy` 加入现有 LLVM 动态库查找路径，没有改系统工具链。
- 源码目录中的旧 `__pycache__` 已迁到 `/Volumes/VoltDrive/AI/tmp/cloud-deck/client/legacy-source-cache-d96Gw6/__pycache__`。最新目录检查未发现源码中的依赖、target、dist 或 Python 缓存。
- macOS 配置 `bundle.macOS.signingIdentity: "-"`，完整应用和资源临时签名。首个本地增量已通过 `codesign --verify --deep --strict`，四份资源 SHA-256 与源码一致、无 auth/凭据/测试驱动，并验证 ZIP 解压后签名。
- 该增量 ZIP 已发给用户：`/Volumes/VoltDrive/AI/artifacts/cloud-deck/CloudDeck-0.1.0-macos-arm64-20261114-Mh9vHy/CloudDeck-0.1.0-macos-arm64.zip`，SHA-256 `f570861427e4eb4ad3e1dbc10eb9aad47035b56eb495ef193997801b08b3ad39`。**此包早于本轮新接入的云端链路，不是最终云端测试包。** 没有 Developer ID 签名或公证。
- 最新状态/配置测试 7 项、Rust 15 项、Linux 守护/准备 15 项通过，覆盖模型准备失败不启动应用、完整进程组停止、Drive 服务端校验与变更后拦截释放。
- 浏览器 23 项交互在桌面和 375×812 通过，包含未连接时禁止保存且不伪造校验结果；无 console/server 错误或失败请求，已恢复 desktop。
- 新统一 `native-check` 已多次真实运行。最新报告/截图 `/Volumes/VoltDrive/AI/tmp/cloud-deck/client/native-smoke-WmYBc7/{native-check.json,native-window.png}`：4 项通过，真实窗口截图已查看。该入口离线、隔离数据、不访问授权存储；不用 AX/屏幕录制权限。
- 默认发布构建不含 QA 驱动。`native-smoke` feature 下只有显式 `CLOUD_DECK_LIVE_CHECK=1` 才允许真人联调；普通离线模式仍禁止云端与授权。

### 真人联调结果与当前运行状态

- 真实 OAuth 已通过：系统浏览器授权、PKCE/state/nonce、ID Token 校验、Keychain 存储、跨进程恢复与刷新；旧 `auth/` 未读写。
- 真实通用部署已通过，报告目录 `/Volumes/VoltDrive/AI/tmp/cloud-deck/client/live-check-R461fR/`。CPU 实例依次完成分配、内置 SSH/WSS、守护部署、4 资源 SHA-256、任务/日志、HTTP 健康及本机转发；随后测试进程停止、实例回收并确认零计费。`native-check.json` 为 `passed:true`。
- 真实联调修复包括 `Accept: application/json`、空 POST 的 `Content-Length: 0`、`*.prod.r.colab.dev` 代理白名单、成功空响应以及 `error:null`；记录分配 receipt，连接失败时只清理本次新建且未写数据的测试机。
- Drive 授权曾因用户离开和外部 Jupyter WebSocket 超时失败；现已通过同一条已认证的内置 SSH 访问实例本机 Jupyter 8080。不要并发旧 SSH 控制器占用当前控制连接。
- 内置 ComfyUI 整轮验收通过：`live-check-M5NUPk/native-check.json`。实际 Drive 挂载、固定 Git 提交、ComfyUI 0.30.0、Tesla T4 CUDA、三项清单模型大小以及远端/本机 `/system_stats` 均通过。手动账号刷新已改为排队等待锁，避免与后台轮询竞争误报；恢复已有应用不会重复启动。
- 保留窗口验收 `live-check-ziOuzG/native-check.json` 为 `passed:true`、`nativeProcessRetained:true`；旧进程 PID `70453` 已在后续验证时结束。真实 ComfyUI 曾通过本机入口 `http://127.0.0.1:18188` 运行。
- ComfyUI 网页通过实际运行按钮执行 Wai 170 风景工作流成功：prompt ID `a4fe1ef0-2d41-42cd-b469-2cf1c04eb56c`，512×512、4 步、种子 24680。输出 PNG 为 402556 字节，SHA-256 `c98811c1b30f8e1ece157bacf9286d381c8986b7eef3c1ab2614d9448ef82bef`；图片与脱敏执行报告在 `/Volumes/VoltDrive/AI/artifacts/cloud-deck/ComfyUI-real-execution-j3HVeV/`，均已交付。
- 示例源文件 `examples/comfyui-wai170-deployment-test.json` 使用现有模型。默认 Z-Image 模板引用清单之外的模型，未为它额外下载。用户明确模型须在 ComfyUI 启动前准备好：当前流程确为模型下载/检查 → 准备脚本成功退出 → 守护启动应用 → 健康检查；出图时仍需将本地权重加载到显存。“启动应用”含准备过程，仅部署守护进程则不准备模型。
- T4 `gpu-t4-s-ass1c1-3m70ng7zo8lnz` 的真实地区为 `新加坡 · asia-southeast1`，已从之前该端点的 `list_runtimes` 返回提取脱敏证据 `/Volumes/VoltDrive/AI/artifacts/cloud-deck/instance-region-evidence.json`。用户指出新版漏了地区，现已沿用旧 Python/Swift 的代理主机解析，并接到 Rust 模型、只读查询、工作台和连接确认。释放后保留上次地区；未知不猜测。`client/regions.json` 保留旧版中文地区表。
- 真实保存/恢复已通过：`live-check-s4jE3L/native-check.json` 为 passed=true，86 个业务文件、24511766 字节通过 Drive 服务端大小/MD5 校验；修改已校验工作流会阻止释放，再启动应用后工作流、设置与 input 的 SHA-256 恢复一致，输出原图保留。修正备份目录被旧恢复逻辑重复恢复的问题；备份不参与正常恢复/再次同步。
- 完整保存关机已实际通过：`live-check-9g3WBI/native-check.json` 为 passed=true，真实原生按钮触发保存、Drive API 再校验、释放该 T4，随后确认 assignments 为空、每小时消耗 0。`live-deployment.jsonl` 的 `saved_instance_release_ready` 包含最终 86 文件的保存凭证。独立 `get_usage` 也确认 0 台、0 点/小时。
- Rust 15 项、前端状态 7 项、Linux 守护/准备 15 项均通过；Linux 含忽略 SIGTERM 子进程仍必须终止的检查。浏览器 23 项在桌面和 375×812 通过。离线地区原生检查 `native-smoke-eJJLR1/native-check.json` 5 项通过，并已查看截图；它使用脱敏真实历史地区，禁用 Google 和凭据存储，不重新申请实例。
- 核心出图成功不代表所有 34 个插件全部无警告，部分旧插件有弃用提示和旧接口 404。Windows 尚未编译/实机验证。
- 先前已交付测试包 `/Volumes/VoltDrive/AI/artifacts/cloud-deck/CloudDeck-0.1.0-native-deployment-QgkPFO/CloudDeck-0.1.0-macos-arm64.zip`（SHA-256 `01b99b1856c785ba592b4f8b24b769e3ed359e3e16b36d240728197f030706a3`）早于后续 Drive/刷新修复，不能当作最新源码构建。

### 收尾与未验证范围

最新默认特性 macOS 测试包已重建并交付：`/Volumes/VoltDrive/AI/artifacts/cloud-deck/CloudDeck-0.1.0-save-region-4hey0ak5/CloudDeck-0.1.0-macos-arm64.zip`，6039873 字节，SHA-256 `2365ed7efaa17a063c7fa355613af253a94ff838a6f26fc61bab67becc151b7e`。包含地区/历史地区显示、Drive 服务端保存校验和完整关机。TypeScript/Vite 与 release 构建通过；4 资源与源码逐字节一致、文件白名单、不含 auth/会话数据与 QA 驱动标识、地区标签已嵌入、完整签名、ZIP 解压后逐字节内容及签名均通过。附模型示例、出图/保存恢复/真实关机/地区显示报告和最终 Drive 凭证。当前仍为临时签名，未公证；Vite 有大于 500 kB 的 chunk 体积提示。先前 `CloudDeck-0.1.0-comfyui-8j95pykr` 包早于本轮保存/地区修正，已被此包替代。

保存、Drive 服务端校验、应用重启恢复和完整保存关机已验证；Windows 和第三方插件全部功能仍未验证。最后独立只读核验为 0 台实例、0 点/小时。用户要求启动最新版进行测试后，已打开默认特性 release 窗口 PID `10242`，应用持久数据目录 `/Volumes/VoltDrive/AI/data/cloud-deck/client/`；上次分配地区来自该 T4 的真实历史记录。窗口没有启用 QA 驱动，重新打开后计费显示历史/待确认，用户可刷新为本次查询。

旧客户端预览和 ComfyUI 预览已由用户停止；仅临时启动了客户端做最终界面检查，完成后停止。云端 ComfyUI 因实例已释放而不可用。不要重启旧 OAuth probe。保持中文；用户本轮未要求手机通知，勿发。

## 原会话标识

- Claude Code 原始会话 ID：`958c5223-a22b-4284-9b60-8fbb97e10036`
- Claude Desktop 会话 ID：`local_71a7dbb9-89ee-4a7c-88dd-f542dd876e14`
- 会话标题：Google Gemini 20x 会员附赠的 Colab
- 桌面会话链接：claude://claude.ai/epitaxy/local_71a7dbb9-89ee-4a7c-88dd-f542dd876e14
- 本机原始 transcript：`~/.claude/projects/<原项目会话目录>/958c5223-a22b-4284-9b60-8fbb97e10036.jsonl`

本文件是接手摘要，不是完整会话导出。原始 transcript 仍在原位置；读取时按需搜索和分段，不要把整份日志或其中的凭据直接输出。可通过会话工具读取桌面会话内容；`get_session` 只返回元数据，内容需要相应的事件/转录工具。

## 最新验证：Google OAuth 实测通过（2026-11-14）

用户要求先测试真实 OAuth。已完成新的一轮系统浏览器登录：账号选择 → PKCE S256 + 本机 loopback 回调 → 授权码兑换 → ID Token 签名/受众/签发者/有效期/nonce 验证 → Google UserInfo → Colab 用量与实例只读查询 → refresh token 刷新，全部成功，相关 HTTP 请求均为 200。

- 详细证据：[OAUTH-TEST.md](OAUTH-TEST.md)。测试脚本：`oauth_probe.py`，含 `--self-test` 回调安全检查；临时服务配置 `cloud-deck-oauth-probe`。
- 当前客户端与 Google 官方 Colab CLI 的 `installed` OAuth 配置一致。已证明这条浏览器自动回调路径在当前 Mac 上可行，不需要复制授权码或打开笔记本。
- 未覆盖现有 `auth/google-oauth-token.json`；新令牌只在测试进程内存中使用，外置盘仅保留脱敏结果。没有申请、释放实例或挂载 Drive。
- 隔离依赖环境在 `/Volumes/VoltDrive/AI/venvs/cloud-deck-oauth-probe-20261114`；未修改系统 Python。后续 Tauri 客户端的 Rust 真人授权、安全存储、刷新和 Colab 只读查询已通过；独立注册的产品 OAuth 客户端和 Windows 实机仍未验证。
- 再次运行浏览器测试会触发新的真实授权，只在用户明确要求时启动。不要因为读取交接就重跑。

## 浏览器登录设计 v03（2026-11-14）

用户在正式项目的新会话要求：Windows/macOS 共用、通过浏览器选择 Google 账号、让大家无需操作 Colab 笔记本即可使用，先设计。

- 新入口：`workstation-design/onboarding.html`；机制与发行边界：`workstation-design/LOGIN-DESIGN.md`。与旧工作台互相链接，原来的 Swift/Python 控制器未修改。
- 流程：系统浏览器选账号 → 分别检查身份、Colab 访问和计算权益 → 选应用/GPU/保存位置 → 确认计费 → 必要时单独授权 Drive → 应用就绪。
- Windows/macOS 共用标准桌面 OAuth 授权码 + PKCE + loopback 回调设计；用户不用创建 GCP 项目。产品方仍需自有 OAuth 客户端、生产同意屏幕与所需审核。
- 现有个人授权不等于公开发行可用。自有客户端对 consumer Colab 的 scope、计算和连接授权尚未验证；Google 登录成功不代表可以分配实例。Colab 免费档、付费权益和余额限制按官方政策与真实能力处理。
- 原型有 12 个场景，涵盖取消、超时、权益不足、查询失败、Drive 授权失败、运行中授权失效。新流程 34 项交互检查、旧工作台 19 项检查通过；已检查 1440×1000、1100×760、375×812 布局。所有账号与结果是演示，没有发起 Google 授权、查询/申请/释放 Colab 或修改 `auth/`。
- 后续仍按用户下一条指令推进，不因设计完成自动实施 OAuth 或跨平台重写。

## 项目复制阶段的要求与授权范围

用户先要求把现有 ComfyUI 专用控制器通用化，未来支持 Windows，并明确“先出设计稿”。随后补充：“其实不用外部的 ssh 了直接内部管理就好了没必要外部了”。

通用设计稿交付后，用户要求：“去我的 ~/Documents/project 开正式项目，给这的全复制过去”，并表示会自己在新项目开启 session，再读取当前会话。

本次完成的是正式项目复制与交接。不要因为读取本文件就自动申请/关闭实例、启动跨平台重写或新建其他 session；按用户在新 session 的下一条指令推进。

## 已确认的产品方向

暂名 Cloud Deck / 云端工作台。首版接入现有 Google Colab，一台云端 GPU 实例可以承载多个应用；ComfyUI 是首个内置应用，另提供 Python 任务与自定义 Web 服务的配置方式。

页面为：工作台、应用、文件与同步、活动记录、设置。实例、应用、连接、同步分别表示状态。断线或查询失败不能解释为关机；只有确认释放与停止计费后才能显示停止计费。错误保留为事件，不能被状态刷新覆盖。

用户喜欢固定的桌面窗口布局，不喜欢整窗滚动带来的网页感；长列表和日志可以在内部滚动。Windows/macOS 共用内容结构，标题栏和系统交互分别适配。

连接、鉴权、远程执行、端口转发与重连由客户端内部管理，不要求用户安装系统 SSH 程序、外部 SSH 客户端或外部终端。底层是否使用 SSH 协议是内部实现选择；把外部 ssh 命令藏起来不能算完成。

Tauri 2 + React / TypeScript + HeroUI 客户端已经在 `client/` 落地。OAuth、凭据存储和本地状态由 Rust 后端管理；Python 准备/守护脚本作为云端资源随包提供。内部连接与云端部署已实测，可靠同步尚未完成，Windows 未编译或实机验证。

## 当前目录内容

- `colab_mcp_server.py` / `colabctl`：现有 Python CLI/MCP 控制器，后者为相对符号链接。
- `cloud_bootstrap.py`：云端环境恢复、模型下载、ComfyUI 启动与文件同步。
- `manifest.json`：固定 ComfyUI/插件版本与已配置模型的来源、大小和校验信息。
- `auth/`：已完整保留的本地 OAuth 授权文件和 SSH 密钥。保留私有目录权限，已经 Git 忽略。不要打印、提交、上传。
- `ComfyCloudApp/`：原生 macOS SwiftUI/AppKit 源码。
- `Comfy Cloud.app`：已在正式目录重新构建的 macOS 应用。
- `ui-preview/`：旧 ComfyUI 专用设计原型。
- `workstation-design/index.html`：新的通用设计原型，单文件可离线打开。
- `workstation-design/DESIGN.md`：信息结构、通用化评估与实现建议。
- `workstation-design/smoke-check.js`：纯演示 UI 的交互检查脚本。
- `workstation-design/previews/`：4 张 2880×2000 高清设计图：首页、应用模板、添加应用、文件同步。
- `.claude/launch.json`：正式目录的两套设计预览配置，端口 18771、18772，避免与旧会话预览端口冲突。

原目录保持不变，未把其他 remote-ssh 项目复制进来。没有下载模型权重到 Mac，没有更改原 ComfyUI/NAS 或云端环境。原始源目录中的 23 个普通文件和 1 个符号链接已在复制后逐一做 SHA-256/链接目标验证；之后只对正式副本做路径适配及文档补充。

## 本次迁移的路径适配与验证

Swift 控制器优先保留 `COMFY_COLABCTL_PATH` 环境变量覆盖；默认从 `.app` 同级目录寻找 `colabctl`，删除了指向旧 remote-ssh 目录的硬编码路径。新目录的 `.app` 已重新编译并签名。

Swift 构建临时目录为 `/Volumes/VoltDrive/AI/tmp/cloud-deck/swift-build`，可通过 `COMFY_BUILD_DIR` 覆盖；脚本同时将临时文件和模块缓存设在该目录。

Python 控制器的运行状态、隧道 PID、清单缓存和日志仍指向 `/Volumes/VoltDrive/AI/tmp/comfyui-colab`；Swift 读取同一状态目录。保留现状以避免本次复制改变运行行为。跨平台改造时需要统一为可配置的系统目录/用户指定数据目录，不能直接照搬到 Windows。不要同时用新旧两个客户端控制同一账号和端口。

迁移期间没有启动 `.app` 或执行 Colab 的 status/probe/up/down，也没有新查余额或当前实例。历史“已关机、零计费”的结论不是当前状态证明。

本次通过：完整复制校验、Python 语法解析、JSON 配置解析、可执行符号链接检查、控制脚本路径检查、正式目录的 Swift 编译。通用 HTML 原型在前一阶段通过 19 项交互检查以及桌面/窄窗口检查；它的数据和云端操作均为演示。

本地 Git 已初始化为 `main`；没有 commit、push 或远端仓库。

## 现有云端实现与需要保留的约束

用户选择免 GCP 项目的 consumer Colab API。已有 OAuth 刷新、GPU 分配/释放、Drive 授权挂载、地区解析和受保护的 Colab WebSocket 通道。登录必须提供可打开的页面并允许选择/切换 Google 账号。

现有版本为 ComfyUI 0.30.0、34 个记录的插件，DLSS 明确排除。版本和来源以 `manifest.json` 为准，不自动升级或添加其他模型。已配置模型包括 170、140、AnimeSharp；模型从 Hugging Face 直接下载到实例本地盘，不能下载到 Mac。

云盘目录为 `/content/drive/MyDrive/ComfyUI_Cloud`；应用在 `/opt/ComfyUI`，虚拟环境 `/opt/comfy-venv`，模型 `/opt/comfy-models`，可变数据 `/opt/comfy-local/{user,input,output}`。

用户要求模型/文件列表按保存清单和真实目录获取，不在界面写死文件数、大小或“已保存”状态。配置下载项、缓存快照、实例本地实际文件和云盘已保存文件要分清。

工作流、JSON 配置、input/output 应保存回 Drive。目标关机流程是：停止新写入 → 同步 → 验证目标端持久化 → 释放实例 → 确认停止计费。原实现尚不满足完整保证。

## 后续改造必须处理的真实问题

1. `tool_up_comfy_linux` 将实例分配、Drive、ComfyUI 与端口全部串在一起；需提取应用配方，实例就绪不依赖 ComfyUI。
2. 现有连接仍调用系统 `ssh`、ControlMaster 和 `pkill`，并有 Unix 进程假设；新客户端需要真正的内置传输、命令执行与转发管理，先验证 Colab 通道和 Windows 兼容性。
3. IP、端口 18188/8188、`/system_stats`、本机 Python/临时目录等写死；需要按应用和平台配置。局域网访问应显式选择，默认仅本机。
4. 现有 shutdown 在同步返回错误时仍可能释放实例；network-TX quiet / os.sync 不是 Drive 远端持久化证明。后台同步与关机同步缺少互斥，源目录新写入也可能造成竞争。先确保失败阻止删除唯一数据副本。
5. 原 Swift 只在立即刷新时保留开机错误，后续刷新/重启仍可能清除；需独立事件记录和 unknown 状态。
6. Swift 进程执行先 wait 再读取 stdout/stderr，可能管道阻塞；任务取消不会可靠终止子进程。需要流式事件与可控取消。
7. 模型库存混合 manifest 和缓存，必须标明来源与观测时间，不能把计划下载项当成实际云盘扫描。
8. H100 曾手动复现 HTTP 503，但返回的是通用错误页，不能断言资源池满；此前“空闲回收导致机器消失”的结论及 keepalive 效果未被证实，不要当成已解决的根因。

不要在本次复制之后自动执行这些重构。先在新 session 读取本文件与设计稿，再根据用户要求开展。

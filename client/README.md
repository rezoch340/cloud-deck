# Cloud Deck 客户端

Tauri 2 + React 19 / TypeScript，界面使用 HeroUI v3 + Tailwind CSS v4。Windows 与 macOS 共用应用配置、状态模型和页面；目前只在 Apple Silicon Mac 验证过真实窗口，GitHub Actions 已配置 Windows x64（NSIS 安装包与便携版）自动构建（本地亦已通过 Windows x64 交叉编译打包），macOS 包继续在本机自行构建，Windows 实机运行仍待验收。

## 当前可用范围

- 五个页面：工作台、应用、文件与同步、活动记录、设置。固定窗口框架，内容区内部滚动；表单、卡片、编辑器和外框共用组件。Windows WebView2 视图启用自定义透明轨道胶囊细滚动条并设置 `scrollbar-gutter: stable` 补偿右侧内边距，避免默认灰底箭头滚动条与页头右侧按钮错位。
- 原生后端实现系统浏览器 Google OAuth：账号选择、PKCE、state、nonce、ID Token 校验、取消/超时、授权刷新。refresh token 留在 macOS Keychain / Windows Credential Manager，不交给前端。
- 已登录账号可手动刷新 Colab 用量与实例只读状态。读取失败保留错误和历史观测，不能解释为零余额、已关机或停止计费。查询确认无实例且计费为零后才允许退出或切换账号。
- 自定义脚本任务、Web 服务配置可添加、编辑、移除并原子保存。保存配置不执行命令，也不申请实例。
- 内置 ComfyUI 用 `applications/comfyui/manifest.json` 声明命令、资源和同步，`setup.py` / `start.sh` 保存应用自身的固定版本安装及启动逻辑；不安装 DLSS，不把下载计划展示成实际文件或已同步结果。
- 开机只部署通用 `cloud_agent.py`、`cloud_lifecycle.py`、`cloud_heartbeat.py`；部署应用时再上传其脚本。`runtime.readiness` 支持 HTTP、TCP、命令/任意脚本，具体格式见 [应用清单说明](applications/README.md)。
- 云端守护通过 `cloud_heartbeat.py` 独立维护 Jupyter kernel 活动，每轮间隔 60 秒；失败自动重试。客户端仅查询结果，退出客户端、电脑休眠或本地断网不停止云端线程。会话仅使用实例本机回环服务，不向实例上传桌面 OAuth 凭据。实例释放后活动自然终止；Google 平台的运行时长、资源和余额限制仍然适用。
- Google 于 2026-09-25 退役旧 `/keep-alive` ping（[官方 PR #144](https://github.com/googlecolab/google-colab-cli/pull/144)）。已移除旧客户端 TFE 请求及定时器；上轮 HTTP 成功只能证明接口响应，不能证明有效保活。工作台现在显示“云端 kernel 活动已确认”、最近时间和错误，连接断开后明确为历史状态。
- 所有应用详情页都有同一个“部署与运行日志”面板，每 2 秒读取云端输出。显示资源下载字节进度、恢复路径、准备命令步骤与退出码、主命令输出和就绪阶段；可暂停自动跟随，查看历史内容。读取失败保留上次日志；启动前等待 Drive 授权时展示当前操作状态，不锁住日志入口。
- 最新云端保活与日志改动仅完成离线测试和 macOS 原生 UI 验证。用户休息期间不进行新实例申请、Google 授权或真实云端测试；客户端离线后的 Colab 实际存活验证待后续完成。

- 原生后端已接通账号绑定的实例查询/申请、Rust 内置 SSH over Colab WSS、脚本上传与守护进程启动。连接默认仅本机回环转发，不调用系统 `ssh`、外部终端或本地 Python。
- 工作台及连接确认显示实际分配地区，沿用旧版从 Colab 返回的代理主机解析地区代码；中文地区名来自共享配置。释放后保留“上次分配地区”，查询失败明确标为上次观测；缺少地区来源时显示待确认，不按 GPU、出口 IP 或电脑位置猜测。
- 应用可独立启动、停止、查看日志；服务经远端健康检查及本机转发检查后才提供打开入口。需要保存文件的应用会先挂载 Drive，必要时单独打开 Google 授权页面；挂载不成功则不启动应用。
- 开机只选择计算类型，并自动部署、连接守护进程；就绪后单独部署应用。健康连接复用，断线仅重连指定的已有实例，不隐式分配。停止应用与释放实例分开；“释放空实例”保留严格限制，已有应用使用“保存并关闭实例”。
- “保存并校验”先停止受管应用及其进程组，再复制配置的保存目录；使用现有 Colab Drive 授权直接查询 Drive API，逐文件核对服务端大小与 MD5。文件清单、SHA-256 和校验时间显示在文件页；失败保留实例和本地文件。
- “保存并关闭实例”在保存和服务端校验通过后再次检查源文件、应用状态及 Drive 服务端记录，随后释放当前实例，并查询确认实例列表为空、每小时计费为零。

macOS 已通过真人 OAuth、通用实例部署/转发/回收、内置 ComfyUI 的真实 T4/Drive/固定版本/模型/健康检查、一次 512×512 出图，以及真实文件保存、变更拦截和应用重启恢复。完整保存关机也已实测通过，最后确认实例数与每小时计费均为零。Windows 尚未验收，第三方插件的全部功能和生成质量未逐项验证。旧 Swift/Python 控制器继续保留为迁移基线，不能与新客户端并发管理同一实例。

上轮清单验收（2026-11-14，旧 TFE 响应已不再作为保活证明）：同一 T4 上独立 kernel 活动已确认，普通命令清单的 ComfyUI / CUDA / 三模型 / 本机入口通过，Linux 36 项测试全部通过。保存并经 Drive 服务端核验 138 个文件后释放测试实例，查询确认 0 台、0 点/小时。最终报告见 `HANDOFF.md` 最新节；本轮未重新执行生成工作流，第三方插件功能未逐项验收。

浏览器预览只把配置保存在当前浏览器，不访问 Google 或 Colab。原生窗口通过 Tauri IPC 调用 Rust 后端，不要求最终用户安装 Node、Python 或外部 SSH 程序；随包 Python 脚本部署到 Linux 云端，不在用户电脑上充当控制服务。

## 登录验证边界

独立 Python 探针已完成真人浏览器登录、自动回调、ID Token 验证、Colab 只读查询和 refresh token 刷新。Rust 客户端已通过真实浏览器登录、安全存储、跨进程授权恢复/刷新及严格的 Colab 查询；真实 CPU 实例的申请、守护部署、资源校验、任务日志、HTTP 服务/转发和安全回收整条链路也已通过。内置 ComfyUI 的 GPU/Drive 验收及一次 512×512 出图也已通过。详见 [OAuth 实测记录](../OAUTH-TEST.md)。

本地验证仍读取 Google 官方 Colab CLI 的桌面 OAuth 客户端配置。公开发行前须配置产品自己的 OAuth 客户端，并验证独立测试账号的权限、计算接入和 Drive 流程；当前测试包不代表已通过公开发行审核。不要分发个人令牌，也不要把登录成功当作可分配 GPU 的证明。

## 部署与模型准备顺序

内置 ComfyUI 的顺序为：连接实例与守护进程 → 挂载需要的 Drive → 依清单准备并校验模型 → 恢复缺失的用户文件与额外模型 → 执行固定版本安装准备命令 → 启动 ComfyUI → 健康检查通过后开放入口。所有资源都核对大小和 SHA-256；有效本地文件直接复用。当前清单优先从 HF 下载，失败后退回 Drive。模型存放在实例 `/opt/comfy-models`，不会下载到 Mac。

“部署并启动应用”包含上述准备步骤；开机时不会下载应用模型。运行工作流时仍需要把已下载权重从磁盘读入内存/显存，这与联网下载不同。此保证针对内置清单与准备流程，不代表任意第三方插件引用的模型都已准备好。

## 保存与恢复

保存范围由清单的 `shutdown.save` 决定，每项明确映射来源与目标。启动恢复由 `startup.restore` 决定，只补缺失文件。ComfyUI 保存和恢复 `user`、`input`、`output`，模型和运行环境不重复上传。旧的简单表单配置保存时会转换为生命周期清单。

保存过程先停止整个受管应用进程组，再记录文件大小、SHA-256 与 MD5，以临时文件完成复制并原子替换，最后直接向 Google Drive API 读取服务端大小和 MD5。旧目标内容不同则保留版本备份；备份目录不参与正常恢复，也不再次同步。当前不传播删除。源文件改变、目录/挂载无效、重复目标、Drive 查询失败或校验超时都会阻止释放。

Drive 临时访问令牌仅在云端进程内使用，不传回桌面、不写入报告；复用已完成的挂载授权，无需用户额外登录。客户端断开不会取消云端保存任务。重新启动 ComfyUI 时恢复缺失的工作流与 input，并保留现有本地修改；输出按清单恢复到实例本地目录。

## 已配置模型的示例工作流

ComfyUI 新前端的默认 Z-Image 模板会引用本项目未配置的 Qwen / Z-Image 模型。可将 [Wai 170 部署检查工作流](../examples/comfyui-wai170-deployment-test.json) 拖入 ComfyUI，使用已有的 `waiIllustriousSDXL_v170.safetensors`。它采用 512×512、4 步、单张输出，用于快速检查执行链路，不代表生成质量；无需额外下载模型。

## 开发与外盘路径

先安装可用的 Node.js、pnpm 和 Rust 工具链。当前验证环境为 Node 24、pnpm 11、macOS arm64；macOS 需要 Xcode Command Line Tools。Windows 的 MSVC / WebView2 构建链仍待实机验证。

全部从项目根目录使用统一入口。macOS 默认工作根为 `/Volumes/CODEX_SCRATCH`，需要 `.codex-scratch-volume` 挂载标记；其他平台必须先将环境变量 `CLOUD_DECK_WORK_ROOT` 设为已存在的绝对路径。根目录不可用时直接失败，不回落内部盘。

- 依赖、前端产物、Cargo target、生成图标和 schema：`<工作根>/build/cloud-deck/client/`。
- Cargo、pnpm、npm、clang、Tauri、XDG 缓存：`<工作根>/cache/`；临时文件：`<工作根>/tmp/cloud-deck/client/`。
- 统一入口启动时，应用数据默认使用 `<工作根>/data/cloud-deck/client/`。可用 `CLOUD_DECK_DATA_DIR` 覆盖。
- `client/node_modules`、`client/src-tauri/icons` 和 `gen` 为指向外盘的链接；切换工作根时只重定向符号链接，不删除旧目标；遇到真实目录则拒绝覆盖。
- 复用既有 Rust 工具链，不自动安装。非默认位置可通过 `CLOUD_DECK_RUST_BIN` 指向其 `bin` 目录。
- 用户直接打开安装后的应用时，配置默认使用系统应用数据目录；开发及原生验证通过环境变量隔离到外盘。令牌始终由系统凭据存储管理。

检查路径：

```bash
node client/scripts/run.mjs paths
```

按锁文件安装到外盘：

```bash
node client/scripts/run.mjs install
```

启动浏览器预览（本机 `http://127.0.0.1:18774`）：

```bash
node client/scripts/run.mjs dev
```

执行 TypeScript 检查及前端生产构建：

```bash
node client/scripts/run.mjs web-build
```

生成 macOS 本地测试应用（默认特性，不包含 `native-smoke` 驱动）：

```bash
node client/scripts/run.mjs desktop-build --bundles app
```

输出位于 `<工作根>/build/cloud-deck/client/target/release/bundle/macos/Cloud Deck.app`。此命令的 `app` bundle 类型只适用于 macOS；Windows 安装包另行验证。runner 优先使用 `CLOUD_DECK_SIGNING_IDENTITY`，否则自动选择本机唯一有效的 Apple Development / Developer ID Application 身份；没有唯一身份时使用临时签名。本轮已使用用户现有个人开发签名，debug/release 的 designated requirement 一致，减少重建后的钥匙串授权提示；未公证，不属于正式发行包。

## 本地检查

状态与应用配置测试：

```bash
node client/scripts/run.mjs test
```

Rust 回调校验、计费状态、账号保护、固定清单和持久化测试：

```bash
node client/scripts/run.mjs rust-test
```

真实 macOS 窗口检查：

```bash
node client/scripts/run.mjs native-check
```

此入口构建带 `native-smoke` 的 debug 应用，使用新的外盘数据目录，禁用 Google 和系统授权存储访问；通过真实窗口完成 IPC、HeroUI 表单保存、移除配置和内置配方展示四项检查，并调用本应用的 WKWebView 截图。报告、截图与进程日志位于命令输出的 `native-smoke-*` 目录，不需要系统截图或辅助功能权限。该 debug 包只用于验证，不用于交付。

浏览器交互检查在 Vite 页面中运行：

```js
(await import('/ui-smoke-check.js')).checkClientUI()
```

它检查 23 项交互，包括通用配置增删改、端口冲突、控件对齐、清单来源、运行入口禁用、状态语义和溢出，只操作浏览器预览配置。

云端守护与清单执行使用 `node client/scripts/run.mjs agent-test` 检查；统一入口设置外置 TMPDIR 并禁用 Python 字节码。离线测试不连接 Colab、不下载模型、不挂载 Drive。

前述离线检查不申请/释放实例或发起新 Google 授权。

## 真实联调

只有在当前任务明确授权真实 OAuth 和部署后才执行：

```bash
node client/scripts/run.mjs live-check
```

此入口使用单独的外盘 `live-check-*` 目录及测试构建。尚无安全存储授权时，通过真实窗口的登录按钮打开系统浏览器，等待用户选择 Google 账号；已有授权时只刷新并核对身份，避免反复要求同意。验证 Rust OAuth、安全存储与 Colab 只读查询后，部署随包脚本，核对云端 SHA-256，执行不写用户文件的短任务与临时 HTTP 服务，并验证原生 SSH、任务日志、服务健康及本机回环转发。

没有现存实例时请求 CPU 测试机；有唯一实例则复用，不再申请新机器。验证目录的 `live-deployment.jsonl` 按时间记录实际实例标识、分配、守护进程连通、资源校验、测试进程和回收结果；因此最后的空实例列表表示清理后的状态，不代表没有申请过机器。只停止本次验证进程；仅当实例为本次新建、应用记录全部属于固定验证命令且已停止时才回收，并再次查询实例与计费。既有实例会保留。报告不含个人令牌；新授权保留在系统凭据存储，旧 `auth/` 不变。测试没有启动 ComfyUI、下载模型或写入 Drive，因此不能代替 ComfyUI / Drive 的真实验收。

普通构建不包含这段联调驱动。`native-check` 仍保持离线；`live-check` 不可作为读取交接或验证本地环境的默认步骤。

有当前任务的真实 GPU / Drive 验收授权后，内置 ComfyUI 采用同一个入口：

```bash
node client/scripts/run.mjs live-check --comfy
```

它先复用已登录账号，优先申请账号可用的 T4（按 T4、L4、G4、A100、H100 顺序选择），部署守护进程后通过产品的 `start_application` 命令启动原配方；已有受管应用准备中或运行中时直接恢复连接。Drive 的 Jupyter 会话通过同一条已认证的 SSH 访问实例本机，避免外部 kernel WebSocket 代理超时。需要单独 Drive 同意时打开系统浏览器；用户授权后在客户端点击“启动应用”继续。模型只下载到云端本地盘。通过后核对真实 Drive 挂载、固定 Git 提交、CUDA 设备、清单模型大小及本机转发健康响应；不声称已验证生成质量或远端文件持久化。应用已经运行或可能写入数据时保留实例，不自动释放；用户应关注工作台实际计费状态。

验收通过后保留真实窗口和本机入口，用于继续检查网页或试用：

```bash
node client/scripts/run.mjs live-check --comfy --keep-open
```

此模式写出报告后返回，输出保留的原生进程 ID；窗口保持连接，关闭窗口不会释放或停止云端实例。普通 `native-check` 和不带此参数的验收保持原来的退出行为。

真实文件保存、变更拦截及应用重启恢复验收（仅复用唯一现有实例，不分配新机器）：

```bash
node client/scripts/run.mjs live-check --persistence --keep-open
```

它通过实际原生按钮保存测试工作流、设置、input 和已有输出，直接核对 Drive API 元数据；修改已校验文件后确认释放被拦截，再移走仅本次创建的测试副本，重启 ComfyUI 检查恢复和 SHA-256。还在云端隔离目录运行 15 项 Linux 守护/准备检查。它保留实例。

当前任务明确授权真实保存关机时，验证实际原生关机按钮、Drive 服务端校验、释放及零计费确认：

```bash
node client/scripts/run.mjs live-check --shutdown --keep-open
```

此入口会释放唯一现有实例；校验失败阻止释放。不能把它用于普通本地验证。

读取已脱敏的地区历史记录做离线原生显示检查：

```bash
node client/scripts/run.mjs native-check --region-fixture <脱敏地区记录.json>
```

该入口仍禁用 Google 与授权存储，只读取公开实例元数据，检查地区和“上次分配地区”标签，不重新申请机器。

## 本机 MCP（第一版）

客户端启动后会在 `127.0.0.1` 的随机端口开一个 MCP 服务，并把地址和随机令牌写进数据目录的 `mcp.json`（权限 0600，每次启动重新生成）。MCP 和窗口共用同一份状态，界面上的变化和 MCP 的操作互相可见。

MCP 客户端通过同一个可执行文件的 `--mcp-stdio` 参数接入，它只负责把标准输入输出转发给正在运行的客户端；客户端没开时会返回明确错误。Codex 配置示例：

```toml
[mcp_servers.cloud-deck]
command = "/Applications/Cloud Deck.app/Contents/MacOS/cloud-deck"
args = ["--mcp-stdio"]
```

如果客户端使用了 `CLOUD_DECK_DATA_DIR`，桥接也要指向同一个目录：`args = ["--mcp-stdio", "--data-dir", "<数据目录>"]`。设置 `CLOUD_DECK_MCP=0` 可以关闭服务。

第一版提供的工具：

| 工具 | 作用 |
| --- | --- |
| `get_workspace` / `refresh_status` | 读取工作区；刷新实例、应用和连接状态 |
| `get_application` / `get_application_guide` | 读取应用清单；获取清单格式说明和最小示例 |
| `validate_application` / `save_application` | 校验、保存应用清单（`manifest` 或表单 `draft`） |
| `remove_application` | 删除应用 |
| `start_application` | 部署并启动应用，等待就绪、完成或失败（默认 600 秒，最多 1800 秒），失败时附带最近日志 |
| `stop_application` / `application_logs` | 停止应用；读取应用日志 |

申请、释放实例和登录 Google 只能在窗口里完成，MCP 不提供这些工具；部署应用前需要先在界面开机并连接实例。服务只接受带正确令牌的本机请求，带 `Origin` 头的浏览器请求一律拒绝。

## 版本发布与客户端更新

- 版本号同步：`node client/scripts/sync-version.mjs <version> [--check]` 统一校验并写入 `client/package.json`、`client/src-tauri/tauri.conf.json`、`client/src-tauri/Cargo.toml` 与 `client/src-tauri/Cargo.lock` 四个文件（自动剥离 `refs/tags/` 与前导 `v`）。
- 自动发版工作流：[Release Workflow](../.github/workflows/release.yml) 在推送 `v*` 标签时运行前后端测试，构建 `CloudDeck-<ver>-windows-x64-setup.exe`（NSIS 安装包）与 `CloudDeck-<ver>-windows-x64-portable.zip`（便携版），生成 `SHA256SUMS.txt` 并发布 GitHub Release（macOS 包由本机自行签名构建，暂不纳入 CI）；`workflow_dispatch` 手动触发时仅上传构建产物用于演练，不创建正式 Release。
- 客户端更新机制（`src-tauri/src/update.rs`）：暴露 `check_update`、`install_update`、`open_release_page` 三个 Tauri 命令。仅接受 HTTPS 与 GitHub 官方允许域名，跳过草稿与预发布版本，并通过 `SemVer` 比对当前版本与最新稳定版本。
- 安装形态与升级行为：在 Windows 下通过主程序同目录是否存在 `uninstall.exe` 区分 `windows_nsis`（NSIS 安装版）与 `windows_portable`（便携版）；macOS 为 `macos_bundle`。NSIS 安装版会拉取同版本 `SHA256SUMS.txt` 与安装程序（限制体积上限），在本地完成常量时间 SHA-256 核验后退出当前进程并以 `/P /UPDATE /R` 启动安装器完成覆盖升级并自动重启；便携版与 macOS 引导在系统浏览器打开对应 Release 页面下载替换。启动桌面客户端 3 秒后自动后台检查一次，设置环境变量 `CLOUD_DECK_AUTO_UPDATE_CHECK=0` 可禁用启动自动检查。

## 主要源码

- `src/components/`：HeroUI 适配控件、应用编辑器与窗口外框。
- `src/pages/`：工作台、应用、文件、活动和设置页面。
- `src/domain.ts` / `src/api.ts`：共用数据模型、配置校验、原生 IPC 与浏览器预览适配。
- `src-tauri/src/auth.rs`：系统浏览器 OAuth、安全存储与只读查询。
- `src-tauri/src/cloud.rs` / `transport.rs`：实例接入、Drive 挂载、守护进程部署、原生 SSH 和回环转发。
- `src-tauri/src/model.rs` / `lib.rs`：持久化、账号保护和原生命令。
- `src-tauri/src/mcp.rs` / `mcp_bridge.rs`：本机 MCP 服务和 `--mcp-stdio` 桥接。
- `src-tauri/src/update.rs`：GitHub Releases 检测、域名与 SHA-256 校验下载及 Windows NSIS 自更新。
- `applications/comfyui/`：普通应用包；清单定义资源、同步和命令，setup.py/start.sh 处理自身安装与启动。
- `scripts/run.mjs` / `sync-version.mjs`：外置工作根构建验证入口与四处版本号同步工具。

项目 `auth/` 是旧控制器的私有授权目录；新客户端不读取或覆盖它，打包也不包含它。不要打印、提交、上传或附带任何个人授权资料。

## 清单与源码维护

详见 [应用清单说明](applications/README.md)。应用页面提供清单编辑器；内置清单可以直接查看。`resources.files` 支持有序外部下载/Drive 回退，也可为空。准备与启动均执行普通 Linux 命令或任意脚本。

前端源码由 Prettier 固定格式，禁止提交压缩后的源码。`App.tsx` 负责页面和操作协调，`pages/Workspace.tsx` 负责实例工作台，应用页面、共享控件、简单表单和清单编辑器各自独立。

```sh
node client/scripts/run.mjs format
node client/scripts/run.mjs format-check
```

## macOS 签名与凭据

统一构建入口优先使用 `CLOUD_DECK_SIGNING_IDENTITY` 指定的签名；未指定时，自动使用本机唯一有效的 Apple Development / Developer ID Application 身份。存在多份证书时需明确指定，避免选错身份。证书和私钥不写入仓库。开发签名可保持多次构建的应用身份，但不等于 Developer ID 分发或公证。

应用打开窗口时只读取已保存的公开账号信息，不触碰钥匙串。首次账号/云端操作按需读取系统凭据，随后进程内复用；读取拒绝也会缓存，后台不循环弹窗。显式登录可重新尝试。登录保存和退出登录同步更新缓存，凭据不写入普通配置文件。

从旧临时签名迁移到个人签名，macOS 可能需要一次钥匙串授权；后续同签名构建复用系统已授予的访问。测试可使用 `node client/scripts/run.mjs live-check --manifest --reuse-build` 复用刚验证过的测试二进制，源码变化后必须重新构建。

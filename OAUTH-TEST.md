# Google OAuth 实测结果

2026-11-14 · macOS 实测通过

## Rust 客户端与真实部署实测补充

Tauri 客户端已通过真实系统浏览器登录、PKCE/state/nonce、ID Token 校验和 macOS 钥匙串保存；新进程也已成功读取该授权、刷新 access token 并核对 Google 身份。旧 `auth/` 未读写，令牌没有交给前端或写入报告。

2026-11-14 的原生部署联调现已通过。真实 CPU 实例 `m-s-usw1c4-vqi9dy6agd4c` 分配成功后，Rust 内置 SSH 经 Colab 受保护 WSS 上传四份随包资源，核对 SHA-256，启动守护进程，执行一次性任务并读取日志，再运行临时 HTTP 服务并通过本机回环转发验证响应。测试进程全部停止，测试机已释放，随后严格查询确认无实例且每小时消耗为零。报告 `passed: true`，真实窗口截图也已获得。

证据目录：`/Volumes/VoltDrive/AI/tmp/cloud-deck/client/live-check-R461fR/`，包含 `native-check.json`、逐阶段的 `live-deployment.jsonl` 和 `native-window.png`；JSON 报告与阶段记录已交给用户。记录含分配和回收的实际实例标识，不含授权凭据。

本轮修复了四个真实联调发现的问题：请求需明确 `Accept: application/json`；空 POST 需 `Content-Length: 0`；原生代理域名包括 `*.prod.r.colab.dev`；成功应用状态中的 `error: null` 不能当成拒绝。释放接口的成功空响应也已兼容，各项都有相应回归检查。

内置 ComfyUI 的整轮验收现已通过，报告为 `/Volumes/VoltDrive/AI/tmp/cloud-deck/client/live-check-M5NUPk/native-check.json`（`passed: true`）。同一 T4 上实际 Drive 挂载、固定 Git 提交、ComfyUI 0.30.0、Tesla T4 CUDA、三项清单模型大小，以及远端/本机转发的 `/system_stats` 均通过；原始准备阶段的实机证据还保留在 `live-check-HNrKfU/live-deployment.jsonl` 的 `comfyui_verified` 记录中。

Drive 会话使用已认证的内置 SSH 访问实例本机 Jupyter，避免外部 kernel WebSocket 代理超时；只读手动账号刷新会等待后台轮询结束，已解决验收收尾竞争。恢复已有受管应用时不重复启动应用。保留窗口模式也已通过：`live-check-ziOuzG/native-check.json` 为 `passed:true`、`nativeProcessRetained:true`，原生进程 PID `70453`，本机入口 `http://127.0.0.1:18188`。保留测试期间的只读查询为 1 台实例、1.15 点/小时；该旧窗口现已结束，T4 已在后续保存关机验证中释放，最新计费结论见下文。

模型准备位于应用进程启动前：先下载/检查清单模型并复制额外模型到云端本地盘，再生成启动参数，由守护进程启动 ComfyUI。离线回归验证了模型准备失败时不生成启动文件，也不启动应用。出图阶段的本地权重载入内存/显存不等于运行时联网下载。

实际 ComfyUI 网页也已加载，并通过运行按钮提交了现有 Wai 170 模型的单张测试：512×512、4 步、种子 24680，提示为无人山湖风景；历史返回 `execution_success`，输出 `CloudDeck_Deployment_Test_00001_.png`。图片已从本机受控入口导出，尺寸 512×512、402556 字节，SHA-256 `c98811c1b30f8e1ece157bacf9286d381c8986b7eef3c1ab2614d9448ef82bef`。对应图片和 `execution-report.json` 在 `/Volumes/VoltDrive/AI/artifacts/cloud-deck/ComfyUI-real-execution-j3HVeV/`，已交给用户；可导入示例源文件为 `examples/comfyui-wai170-deployment-test.json`。

前端默认 Z-Image 模板引用未配置模型，已使用现有模型示例验证，没有额外下载 Qwen/Z-Image。旧插件仍出现部分弃用提示和旧接口 404，不能把核心执行成功解释为全部 34 个插件零告警通过。Windows 与生成质量评价仍未验证。保存与持久化的后续结果如下。

### 保存、恢复与真实关机补充

已沿用 Swift 的先保存后关机顺序，接入停止整个应用进程组、原子复制、不同内容保留备份，以及通过现有 Colab Drive 授权直接读取 Drive API 服务端大小/MD5。凭据仅在云端使用，不回传桌面或报告。此结果不以挂载目录读回、`os.sync()` 或网络发送安静作为持久化证明。

`/Volumes/VoltDrive/AI/tmp/cloud-deck/client/live-check-s4jE3L/native-check.json` 为 `passed:true`：最终 86 个业务文件、24511766 字节服务端校验通过；改动已校验工作流后释放被阻止，再保存、移走仅本次的测试文件并重启应用后，工作流、设置、input 恢复且 SHA-256 一致，原始输出图保持不变。云端隔离目录的 15 项 Linux 检查也全部通过，包括忽略 SIGTERM 的子进程必须停止后才保存。版本备份不再被当作用户文件重复恢复；当前不传播删除。

`/Volumes/VoltDrive/AI/tmp/cloud-deck/client/live-check-9g3WBI/native-check.json` 为 `passed:true`：真实原生“保存并关闭实例”按钮再次完成 86 文件保存和 Drive 服务端校验，释放该 T4 后，新的 Colab 查询确认实例列表为空、每小时消耗为 0。`live-deployment.jsonl` 保留 `saved_instance_release_ready` 的最终凭证；独立只读 `get_usage` 同样确认 0 台、0 点/小时。旧文中的 1.15 点/小时是保留测试期间的历史状态。

该 T4 的真实分配地区为 **新加坡 · asia-southeast1**，已从当时该端点的实际 `list_runtimes` 返回提取，脱敏证据为 `/Volumes/VoltDrive/AI/artifacts/cloud-deck/instance-region-evidence.json`，不含代理地址或令牌。新版已沿用旧版地区解析；Rust 回归覆盖地区解析、无效来源、同一实例刷新、释放及重新打开后的历史保留。离线原生显示检查 `/Volumes/VoltDrive/AI/tmp/cloud-deck/client/native-smoke-eJJLR1/native-check.json` 5 项通过，截图明确标为“上次分配地区”；没有为界面验证再申请实例。

下文为之前独立 Python 探针的完整验证证据。

## 结论

**浏览器选择 Google 账号 → 自动回调本机 → 获取与刷新令牌 → 调用 consumer Colab 只读接口，整条链路已实测成功。** 用户不需要复制授权码、操作 Colab 笔记本或创建自己的 GCP 项目。

这次使用的是现有授权中记录的桌面 OAuth 客户端。已与 [Google 官方 Colab CLI 的配置](https://github.com/googlecolab/google-colab-cli/blob/main/src/colab_cli/oauth_config.json)核对：client ID 一致，配置类型为 `installed`，注册回调包含 `http://localhost`。测试没有把配置值、账号标识或令牌写进此报告。

## 本轮证据

| 检查 | 新浏览器登录结果 |
| --- | --- |
| 系统浏览器授权 | 收到真实授权回调 |
| PKCE S256 + loopback | Google 接受授权码兑换，HTTP 200 |
| state 验证 | 通过 |
| ID Token 签名、签发者、受众、有效期与 nonce | 通过 Google 验证库及 nonce 比较 |
| Google UserInfo | HTTP 200，subject 与已验证 ID Token 一致，email 已验证 |
| Colab `/tun/m/ccu-info` | HTTP 200 |
| Colab `/tun/m/assignments` | HTTP 200 |
| refresh token | 已返回 |
| 用新 refresh token 换取 access token | HTTP 200 |
| 实例创建/删除请求 | 0 次 |

在新浏览器登录之前，也用原有授权进行了独立检查：刷新令牌、Google 身份、Colab 用量与实例列表均成功，说明既有 OAuth 路径仍然可用。

Google 返回的实际范围是 `openid`、`https://www.googleapis.com/auth/userinfo.email`、`https://www.googleapis.com/auth/colaboratory`；未额外请求 Cloud Platform 或 Drive 范围。Drive 挂载授权仍是单独的运行时流程，本轮没有执行。

## 测试方式与数据边界

- 使用系统默认浏览器，明确请求账号选择和同意，未用内嵌 WebView 登录。
- 监听 `127.0.0.1:18773/callback`；每次生成独立 state、nonce 和 PKCE verifier。
- 当前请求只接受一次回调；拒绝超时、错误 state、重复参数、重复消费及 code/error 混用。
- 新得到的 ID/access/refresh token 只在进程内存使用，不覆盖 `auth/google-oauth-token.json`，不打印令牌或账号标识。
- 只落盘脱敏结果到外置盘，不保存回调授权码或 OAuth 请求日志。
- `oauth_probe.py --self-test` 的回调状态、取消、超时、重放和重复参数检查通过。
- 本机原 Google 库缺依赖，测试使用外置盘隔离 Python 环境；没有修改系统 Python。

## 对产品设计的影响

可以保留“在浏览器登录并自动返回”的主流程。现有 CLI 的远程复制授权码方式不必成为桌面产品的用户流程。

这次证明了 **当前官方 Colab CLI 客户端配置在 Mac 上可完成标准桌面 OAuth 和 consumer Colab 访问**。尚未验证产品方独立注册的新 OAuth 客户端、任意外部用户的发行审核，也未在 Windows 实机测试。不能把这些未测事项写成已完成，但它们不否定本次已成功的浏览器登录机制。

## 可重复的本地检查

```bash
python3 -I -B oauth_probe.py --self-test
```

实际浏览器测试配置名为 `.claude/launch.json` 中的 `cloud-deck-oauth-probe`。再次运行会发起新的真实授权，应只在用户要求测试登录时启动。测试结束后停止该服务，令牌随进程内存释放。

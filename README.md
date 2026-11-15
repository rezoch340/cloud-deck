# Cloud Deck

通用云端工作台，目标为 Windows / macOS。ComfyUI 是首个应用；实例、应用、客户端连接与文件同步分别管理。

新跨平台客户端在 `client/`，使用 Tauri 2 + React / TypeScript + HeroUI v3。首个增量包含浏览器登录机制、Colab 只读状态、通用应用配置、内置 ComfyUI 清单与随包云端守护脚本；已在 macOS arm64 验证真实窗口，Windows 尚未实机验证。原 Python 控制程序和 Swift macOS 客户端继续保留为迁移基线。

客户端已接通实例申请、内置远程执行与回环转发、守护进程部署及应用启动/停止；真实 CPU 的部署和回收、内置 ComfyUI 的 T4/Drive/固定版本验收及 512×512 实际出图均已通过。保存流程先停止应用写入，复制配置目录，再直接核对 Drive 服务端文件大小与校验值；全部通过后“保存并关闭实例”才会释放机器并查询计费。文件页面分别展示模型下载配置和实际保存校验清单。

- [客户端使用范围、构建与验证](client/README.md)
- [接手说明与原会话标识](HANDOFF.md)
- [浏览器 OAuth 实测结果](OAUTH-TEST.md)
- [通用化设计与改造建议](workstation-design/DESIGN.md)
- [可直接打开的 HTML 设计稿](workstation-design/index.html)
- [高清设计图](workstation-design/previews/)
- `colabctl` / `colab_mcp_server.py`：现有 Colab 控制程序。
- `cloud_agent.py`：通用 Linux 应用守护进程，随包提供并由原生连接层部署。
- `cloud_bootstrap.py` / `manifest.json`：云端准备、受管准备模式和固定版本配置。
- `ComfyCloudApp/` / `Comfy Cloud.app`：原 Swift 源码和 macOS 应用。
- `ui-preview/`：第一版 ComfyUI 专用界面稿。

连接、远程执行和端口转发由新客户端的 Rust 内置 SSH over Colab WSS 管理，不要求用户安装外部 SSH 客户端或打开外部终端。保留的旧 Python 基线仍使用系统 SSH，不能与新客户端同时操作同一实例和本机端口。

项目代码放在本目录，构建、缓存和大型临时文件放 `/Volumes/VoltDrive/AI/`。客户端安装、构建和测试统一通过 `node client/scripts/run.mjs <mode>`，外盘不可用时直接报错。本地授权资料保留在 `auth/` 并已加入 Git 忽略，切勿提交或上传。经用户授权的真实 OAuth 与实例部署验证结果见实测记录；本地构建和离线检查不会操作实例或重新授权。

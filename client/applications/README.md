# 应用 manifest.json

应用清单只描述运行命令、资源下载、启动恢复和关机保存。Cloud Deck 不解析某个应用的安装方式、仓库或插件；安装由普通命令或应用自己的脚本完成。

最小可运行例子见 [命令示例](../../examples/command-app/manifest.json)：

```json
{
  "schemaVersion": 1,
  "id": "hello-command",
  "name": "命令示例",
  "description": "不需要下载文件，直接运行命令",
  "kind": "task",
  "runtime": {
    "command": "printf 'Hello from Cloud Deck\\n'",
    "workingDirectory": "/",
    "environment": {},
    "remotePort": null,
    "localPort": null,
    "healthPath": null
  },
  "resources": { "files": [] },
  "startup": { "restore": [], "commands": [] },
  "shutdown": { "save": [] }
}
```

没有资源就写 `"files": []`，没有准备或同步步骤也写空数组。这种应用不需要连接 Google Drive。客户端“添加应用清单”保存配置后，在已开机的实例上部署即可。

## 只读描述

`description` 是应用卡片的简短介绍。可选 `details` 是详情页的 Markdown 说明（最多 64 KiB），支持标题、列表、代码和表格。例如：

```json
{ "details": "## 应用说明\n\n| 项目 | 内容 |\n| --- | --- |\n| 版本 | 1.0 |\n| 用途 | 批量处理文件 |" }
```

描述仅用于展示，不生成可编辑参数，也不会执行代码或参与安装。前端使用同一个组件渲染所有应用的说明，忽略原始 HTML。ComfyUI 的插件表也是这种普通描述。

## 执行命令或任意脚本

`runtime.command` 和 `startup.commands[].command` 都是在云端工作目录内通过 `/bin/sh -c` 执行的 Linux 命令。例如：

```json
{ "command": "mkdir -p /opt/my-app && python3 -m venv /opt/my-app/venv", "workingDirectory": "/" }
```

```json
{ "command": "bash /opt/my-app/setup.sh", "workingDirectory": "/opt/my-app" }
```

运行命令也可以是 `exec python3 /opt/my-app/server.py`、`exec bash /opt/my-app/start.sh` 或任意已安装解释器。脚本可以来自外部下载、Drive 恢复，或应用随包文件。执行器不需要知道脚本内部做了什么。

准备命令按顺序执行，每步有自己的工作目录。每一步都是独立 shell；共享环境变量放在 `runtime.environment` 中。任一步返回非零状态时，不再执行后续步骤或运行命令。运行进程保持前台，建议用 `exec` 启动长期服务；守护进程负责日志、退出状态、停止进程组及健康检查。

内置应用目录的普通文本文件随应用部署上传，开机阶段只上传守护进程和通用执行器。`$CLOUD_DECK_APP_DIR` 指向云端脚本目录，`$CLOUD_DECK_MANIFEST_FILE` 指向实际使用的清单。脚本可使用这些变量，也可完全不依赖它们。

## 所有应用的部署与运行日志

应用详情页自动展示共用日志面板，不需要清单增加日志配置。`startup.commands` 和 `runtime.command` 的标准输出、标准错误写入云端应用日志；下载/校验、Drive 恢复、准备命令序号与退出码、应用启动及就绪结果也会记录。界面每 2 秒更新，下载显示字节进度，可暂停自动跟随。

日志保留在当前实例 `/opt/cloud-deck/state/<应用 ID>.log`，重新连接后继续读取。界面取最近 64 KiB，云端日志按现有大小上限保留尾部；这是实例内运行记录，不会自动保存到 Drive，也不会在释放实例后继续存在。脚本本身若缓冲 stdout，需要使用解释器的无缓冲选项或主动 flush；Python 默认收到 `PYTHONUNBUFFERED=1`。

## ComfyUI 示例

```text
comfyui/
  manifest.json   # 三份模型、同步路径、普通命令
  setup.py        # ComfyUI 自己的固定仓库/插件/依赖安装
  start.sh        # ComfyUI 自己的 CUDA 环境和启动参数
```

[清单](comfyui/manifest.json) 中的准备命令是 `python3 "$CLOUD_DECK_APP_DIR/setup.py"`，运行命令是 `exec bash "$CLOUD_DECK_APP_DIR/start.sh"`。其版本和 34 个插件来源保留在应用自己的安装代码中，继续排除 DLSS。通用执行器不识别 ComfyUI、pip、Git 或任何插件格式。

旧 Swift/Python 控制器和项目根目录清单保留作迁移基线，不参与新客户端应用部署。

## 字段与执行顺序

| 字段 | 用途 |
| --- | --- |
| `schemaVersion` | 当前为 `1`，清单最大 512 KiB |
| `id`、`name`、`description` | 稳定标识、名称、说明 |
| `details` | 可选的只读 Markdown 详细说明，不参与执行 |
| `kind` | `service` 长期服务，或 `task` 一次性任务 |
| `runtime` | 命令、工作目录、环境变量；服务用 `readiness` 判断就绪，端口按需配置 |
| `resources.files` | 文件的目标、大小、SHA-256 和有序来源；可为 `[]` |
| `startup.restore` | Drive → 实例的文件或目录映射；可为 `[]` |
| `startup.commands` | 准备命令，按顺序执行；可为 `[]` |
| `shutdown.save` | 实例 → Drive 的保存映射；可为 `[]` |

1. 开机只选择 GPU/CPU，自动连接并准备守护进程。
2. 部署应用：准备资源 → 恢复文件 → 执行准备命令 → 执行运行命令。
3. 服务健康检查与本机转发检查通过后开放入口；一次性任务记录退出状态。
4. 保存并关机：停止应用 → 保存声明的文件 → 核验 Drive 服务端内容 → 释放实例 → 确认计费。

所有应用的保存列表都为空时，停止应用后直接核验可释放状态，不访问 Drive。只要有保存规则，就必须完成该规则的服务端校验；空目录不等于没有保存规则。

连接正常时复用传输。断线只恢复指定现有实例，不隐式分配。应用与实例生命周期独立。

## 启动完成的判断

服务在 `runtime.readiness` 声明检测方式；进程启动后保持“准备中”，检测成功才变为“运行中”。三种检测任选一种：

```json
{ "type": "http", "url": "http://127.0.0.1:8188/system_stats", "status": 200 }
```

```json
{ "type": "tcp", "host": "127.0.0.1", "port": 5432 }
```

```json
{ "type": "command", "command": "bash \"$CLOUD_DECK_APP_DIR/ready.sh\"", "workingDirectory": "/opt/my-app" }
```

命令可以直接写 `test -f /opt/my-app/ready`，也可以指向任意脚本或解释器。HTTP 精确匹配状态码（默认 200，不跟随重定向），TCP 建立连接即成功，命令退出码 0 即成功。命令默认使用应用工作目录，继承应用环境变量及应用包路径。

三种检测共有可选的 `timeoutSeconds`（总等待，默认 120，最大 86400）、`intervalSeconds`（检查间隔，默认 2，最大 300）和 `probeTimeoutSeconds`（单次上限，默认 5，最大 30），均为正整数。下载和准备命令不计入总等待。失败会重试；超时标记失败并保留应用进程供排查。慢检查在独立线程执行，停止或保存应用时会终止检查命令及其进程组。

无需网络入口的服务可将 `remotePort`、`localPort` 设为 null。配置两者时会建立端口转发；HTTP 检测指向被转发的回环服务时，客户端还会核对本机入口，再开放“打开应用”。TCP 服务不自动当作网页打开。旧清单的 `healthPath` 仍可读取，新清单使用 `readiness`。

`kind: "task"` 不设置 readiness：运行期间显示执行中，退出码 0 表示任务完成，非 0 表示失败。就绪只代表作者声明的检查通过，不保证所有插件、模型显存加载或一次实际任务都已通过。

## 资源优先级与回退

每个资源的 `sources` 按顺序尝试。例如先镜像，再原站，最后 Drive：

```json
{
  "target": "/opt/my-app/model.bin",
  "size": 123456,
  "sha256": "填写真实文件的64位SHA-256",
  "sources": [
    { "kind": "download", "url": "https://mirror.example/model.bin" },
    { "kind": "download", "url": "https://huggingface.co/owner/repository/resolve/revision/model.bin" },
    { "kind": "drive", "path": "/content/drive/MyDrive/MyApp/model.bin" }
  ]
}
```

上例需要替换为真实地址、大小和校验值。可按需要将 Drive 放到第一位。下载超时、HTTP 错误、文件缺失或校验失败会尝试下一来源；全部失败则停止部署。本地文件先校验，通过后直接复用。传输写入临时文件，校验后原子替换，失败保留原文件。日志区分 `local`、`download`、`drive`。

## 同步规则

`startup.restore` 只补缺失文件，保留实例上的修改，不传播删除。`optional: true` 允许来源缺失，省略时也视为可选。恢复普通文件，排除备份和同步临时文件，拒绝符号链接。

`shutdown.save` 明确指定实例来源与 Drive 目标。来源缺失、目标冲突、源文件变化或服务端大小/MD5 校验失败都会阻止释放。不同内容的旧目标保留版本备份；备份不再次同步或恢复。ComfyUI 保存 `user`、`input`、`output`，模型不重复上传。

## 开发检查

从项目根目录使用统一入口：

```sh
node client/scripts/run.mjs test
node client/scripts/run.mjs agent-test
node client/scripts/run.mjs rust-test
node client/scripts/run.mjs format-check
```

清单校验：`src/manifest.ts` 与 `src-tauri/src/manifest.rs`。通用资源、同步和命令执行：项目根 `cloud_lifecycle.py`；进程管理：`cloud_agent.py`。应用自己的安装代码放在应用目录内。

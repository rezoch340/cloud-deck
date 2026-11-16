use crate::{manifest, model, Backend};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Manager};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_BODY: usize = 1024 * 1024;
const MAX_WAIT_SECONDS: u64 = 1800;
const INSTRUCTIONS: &str = "Cloud Deck 本机 MCP：操作正在运行的 Cloud Deck 客户端，与窗口共用同一份状态。可以查看工作区、编写并校验应用配置、保存、启动/停止应用和读取日志。申请或释放 Colab 实例、登录 Google 都只能在客户端界面完成；启动应用前需要用户已在界面开机并连接实例。编写新应用前先调用 get_application_guide，保存前先用 validate_application 校验。运行中的应用不能编辑，先停止。";

pub fn discovery_path(root: &Path) -> PathBuf {
    root.join("mcp.json")
}

pub fn start(app: AppHandle, root: &Path) -> Result<(), String> {
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|_| "无法建立本机 MCP 端口。")?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("本机 MCP 端口无效。")?
        .port();
    let token = crate::auth::random_token(32)?;
    write_discovery(
        root,
        &json!({
            "url": format!("http://127.0.0.1:{port}/mcp"),
            "token": token,
            "pid": std::process::id(),
        }),
    )?;
    std::thread::Builder::new()
        .name("cloud-deck-mcp".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                let app = app.clone();
                let token = token.clone();
                let _ = std::thread::Builder::new()
                    .name("cloud-deck-mcp-request".into())
                    .spawn(move || handle(&app, &token, request));
            }
        })
        .map_err(|_| "无法启动本机 MCP 服务。")?;
    Ok(())
}

fn write_discovery(root: &Path, value: &Value) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|_| "无法创建数据目录。")?;
    let mut file =
        tempfile::NamedTempFile::new_in(root).map_err(|_| "无法创建 MCP 连接信息文件。")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "无法保护 MCP 连接信息文件权限。")?;
    }
    serde_json::to_writer(file.as_file_mut(), value).map_err(|_| "无法写入 MCP 连接信息。")?;
    file.flush().map_err(|_| "无法写入 MCP 连接信息。")?;
    file.persist(discovery_path(root))
        .map_err(|_| "无法保存 MCP 连接信息文件。")?;
    Ok(())
}

fn header(request: &tiny_http::Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|item| item.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|item| item.value.as_str().to_owned())
}

fn reply(request: tiny_http::Request, status: u16, body: Option<&Value>) {
    let mut response =
        tiny_http::Response::from_string(body.map(Value::to_string).unwrap_or_default())
            .with_status_code(status)
            .with_chunked_threshold(usize::MAX);
    for (name, value) in [
        ("Content-Type", "application/json"),
        ("Cache-Control", "no-store"),
    ] {
        response.add_header(tiny_http::Header::from_bytes(name, value).expect("static header"));
    }
    if status == 405 {
        response.add_header(tiny_http::Header::from_bytes("Allow", "POST").expect("static header"));
    }
    let _ = request.respond(response);
}

fn authorized(given: Option<&str>, token: &str) -> bool {
    given
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|value| bool::from(value.as_bytes().ct_eq(token.as_bytes())))
}

fn handle(app: &AppHandle, token: &str, mut request: tiny_http::Request) {
    if request.url() != "/mcp" {
        return reply(request, 404, None);
    }
    if header(&request, "Origin").is_some() {
        return reply(request, 403, None);
    }
    if !authorized(header(&request, "Authorization").as_deref(), token) {
        return reply(request, 401, None);
    }
    if request.method() != &tiny_http::Method::Post {
        return reply(request, 405, None);
    }
    if request
        .body_length()
        .is_some_and(|length| length > MAX_BODY)
    {
        return reply(request, 413, None);
    }
    let mut body = String::new();
    if request
        .as_reader()
        .take(MAX_BODY as u64 + 1)
        .read_to_string(&mut body)
        .is_err()
        || body.len() > MAX_BODY
    {
        return reply(request, 413, None);
    }
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        return reply(
            request,
            400,
            Some(&rpc_error(Value::Null, -32700, "请求不是有效的 JSON。")),
        );
    };
    match dispatch(app, message) {
        Some(response) => reply(request, 200, Some(&response)),
        None => reply(request, 202, None),
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn dispatch(app: &AppHandle, message: Value) -> Option<Value> {
    if message.is_array() {
        return Some(rpc_error(Value::Null, -32600, "不支持批量请求。"));
    }
    let id = message.get("id").cloned();
    let Some(method) = message["method"]
        .as_str()
        .filter(|_| message["jsonrpc"] == "2.0")
    else {
        return id
            .filter(|_| message.get("result").is_none() && message.get("error").is_none())
            .map(|id| rpc_error(id, -32600, "无效的 JSON-RPC 请求。"));
    };
    let id = id?;
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call(app, &params),
        _ => Err((-32601, "不支持的方法。".to_string())),
    };
    Some(match result {
        Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

fn initialize(params: &Value) -> Value {
    let version = params["protocolVersion"]
        .as_str()
        .filter(|value| PROTOCOL_VERSIONS.contains(value))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "cloud-deck", "title": "Cloud Deck", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        },
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": name == "remove_application" || name == "stop_application",
            "idempotentHint": read_only,
            "openWorldHint": matches!(name, "start_application" | "stop_application" | "application_logs" | "refresh_status"),
        },
    })
}

fn tools() -> Vec<Value> {
    let id = json!({"id": {"type": "string", "description": "应用 id，来自 get_workspace。"}});
    let config = json!({
        "draft": {"type": "object", "description": "简单表单格式（RecipeDraft），字段见 get_application_guide。带 id 表示修改已有应用。"},
        "manifest": {"type": "object", "description": "完整生命周期清单（schemaVersion 1），格式见 get_application_guide。"},
    });
    vec![
        tool("get_workspace", "查看账号、余额、实例连接和所有应用的当前状态与入口地址。", json!({}), &[], true),
        tool("refresh_status", "向云端守护进程刷新一次状态后返回工作区摘要。没有连接实例时只返回本地状态。", json!({}), &[], true),
        tool("get_application", "读取单个应用的完整配置（含清单）、运行状态和本机入口。", id.clone(), &["id"], true),
        tool("get_application_guide", "读取应用清单与表单格式说明及示例。编写新应用前先调用。", json!({}), &[], true),
        tool("validate_application", "按保存时的同一套规则校验 draft 或 manifest，返回规范化后的配置，不保存。", config.clone(), &[], true),
        tool("save_application", "保存 draft 或 manifest 为应用配置。运行中的应用需要先停止。", config, &[], false),
        tool("remove_application", "移除本地应用配置，不删除云端文件。内置应用不能移除。", id.clone(), &["id"], false),
        tool(
            "start_application",
            "在已连接的实例上部署并启动应用，然后等待结果：服务等到就绪检查通过，任务等到结束。失败或超时会附带日志尾部。不会申请实例。",
            json!({
                "id": {"type": "string", "description": "应用 id。"},
                "waitSeconds": {"type": "integer", "minimum": 0, "maximum": MAX_WAIT_SECONDS, "description": "最长等待秒数，默认 600；0 表示提交后立即返回。"},
            }),
            &["id"],
            false,
        ),
        tool("stop_application", "停止应用进程。实例和计费保留。", id, &["id"], false),
        tool(
            "application_logs",
            "读取应用的部署与运行日志（准备命令、下载进度、标准输出和错误）。",
            json!({
                "id": {"type": "string", "description": "应用 id。"},
                "tailLines": {"type": "integer", "minimum": 1, "maximum": 2000, "description": "只返回最后多少行，默认 200。"},
            }),
            &["id"],
            true,
        ),
    ]
}

fn call(app: &AppHandle, params: &Value) -> Result<Value, (i64, String)> {
    let name = params["name"]
        .as_str()
        .ok_or((-32602, "缺少工具名称。".to_string()))?;
    if !tools().iter().any(|tool| tool["name"] == name) {
        return Err((-32602, format!("未知工具：{name}")));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err((-32602, "arguments 必须是对象。".to_string()));
    }
    let outcome = tauri::async_runtime::block_on(run_tool(app, name, &arguments));
    Ok(match outcome {
        Ok(value) => json!({
            "content": [{"type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default()}],
            "isError": false,
        }),
        Err(message) => json!({"content": [{"type": "text", "text": message}], "isError": true}),
    })
}

fn text_argument(arguments: &Value, name: &str) -> Result<String, String> {
    arguments[name]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("缺少参数 {name}。"))
}

fn integer_argument(
    arguments: &Value,
    name: &str,
    range: (u64, u64),
    default: u64,
) -> Result<u64, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|number| (range.0..=range.1).contains(number))
            .ok_or_else(|| format!("{name} 必须为 {}–{} 的整数。", range.0, range.1)),
    }
}

async fn run_tool(app: &AppHandle, name: &str, arguments: &Value) -> Result<Value, String> {
    match name {
        "get_workspace" => Ok(summary(&crate::get_workspace(app.state())?)),
        "refresh_status" => Ok(summary(&crate::refresh_cloud(app.state()).await?)),
        "get_application" => {
            let id = text_argument(arguments, "id")?;
            application_detail(&app.state::<Backend>().snapshot()?, &id)
        }
        "get_application_guide" => Ok(guide()),
        "validate_application" => {
            let recipe = validate(&app.state::<Backend>(), arguments)?;
            Ok(json!({"valid": true, "application": recipe}))
        }
        "save_application" => save(app, arguments),
        "remove_application" => {
            let id = text_argument(arguments, "id")?;
            let snapshot = crate::remove_application(id.clone(), app.state())?;
            Ok(json!({"removed": id, "workspace": summary(&snapshot)}))
        }
        "start_application" => start_tool(app, arguments).await,
        "stop_application" => {
            let id = text_argument(arguments, "id")?;
            let snapshot = crate::stop_application(id.clone(), app.state()).await?;
            Ok(
                json!({"application": application_status(&snapshot, &id), "message": snapshot.runtime.message}),
            )
        }
        "application_logs" => {
            let id = text_argument(arguments, "id")?;
            let lines = integer_argument(arguments, "tailLines", (1, 2000), 200)?;
            logs_tail(app, &id, lines as usize).await
        }
        _ => Err(format!("未知工具：{name}")),
    }
}

fn application_status(snapshot: &model::Snapshot, id: &str) -> Value {
    let runtime = &snapshot.runtime;
    let Some(app) = snapshot.apps.iter().find(|app| app.id == id) else {
        return json!({"id": id, "state": "missing"});
    };
    json!({
        "id": app.id,
        "name": app.name,
        "kind": app.kind,
        "builtin": app.builtin,
        "state": runtime.applications.get(&app.id).map(String::as_str).unwrap_or("unknown"),
        "url": runtime.application_urls.get(&app.id),
        "localPort": app.local_port,
        "remotePort": app.remote_port,
    })
}

fn summary(snapshot: &model::Snapshot) -> Value {
    let runtime = &snapshot.runtime;
    let recent = snapshot.events.len().saturating_sub(20);
    json!({
        "account": snapshot.account.as_ref().map(|account| json!({
            "email": account.email,
            "name": account.name,
            "status": account.status,
        })),
        "observation": snapshot.observation,
        "instance": {
            "endpoint": runtime.endpoint,
            "connection": runtime.connection,
            "phase": runtime.phase,
            "message": runtime.message,
            "synchronization": runtime.synchronization,
            "needsDriveConsent": runtime.needs_drive_consent,
        },
        "applications": snapshot
            .apps
            .iter()
            .map(|app| application_status(snapshot, &app.id))
            .collect::<Vec<_>>(),
        "recentEvents": snapshot.events[recent..],
    })
}

fn application_detail(snapshot: &model::Snapshot, id: &str) -> Result<Value, String> {
    let app = snapshot
        .apps
        .iter()
        .find(|app| app.id == id)
        .ok_or("应用配置不存在。")?;
    Ok(json!({"status": application_status(snapshot, id), "configuration": app}))
}

fn guide() -> Value {
    json!({
        "manifestGuide": include_str!("../../applications/README.md"),
        "minimalManifestExample": serde_json::from_str::<Value>(include_str!("../../../examples/command-app/manifest.json")).unwrap_or(Value::Null),
        "draftFields": {
            "id": "可选；修改已有应用时填写，新建时省略。",
            "name": "1–60 个字符，不能与其他应用重名。",
            "description": "卡片简介，最多 1000 字节。",
            "kind": "service（长期运行的 Web 服务）或 task（一次性任务，按退出码判断完成）。",
            "prepareCommand": "可选；启动前执行的准备命令（/bin/sh -c），非零退出则不启动。",
            "environment": "可选；环境变量对象，最多 64 项。",
            "command": "启动命令（/bin/sh -c），长期服务建议用 exec 保持前台。",
            "workingDirectory": "云端 Linux 绝对路径。",
            "remotePort": "服务在云端监听的端口；task 填 null。",
            "localPort": "转发到本机的端口，不能与其他应用冲突；task 填 null。",
            "healthPath": "HTTP 健康检查路径，以 / 开头；也可改用 readiness。",
            "readiness": "可选；如 {\"type\":\"http\",\"url\":\"http://127.0.0.1:8188/\",\"status\":200,\"timeoutSeconds\":600}。service 必须有 healthPath 或 readiness。",
            "saveDirectories": "关机前保存到 Google Drive 的云端目录列表，不需要时填 []。",
            "storageDirectory": "保存目录在 Drive 中的存放位置（云端绝对路径）。",
            "autoStart": "布尔值。",
        },
        "notes": [
            "manifest 能描述资源下载、启动恢复和关机保存，功能比 draft 完整；简单应用两者都可以。",
            "内置 ComfyUI 的完整清单可通过 get_application(\"comfyui\") 读取，作为复杂应用的参考。",
            "保存前先调用 validate_application；校验规则与界面保存完全相同。",
            "start_application 只在已连接的实例上部署；没有实例时请用户在客户端界面开机。",
        ],
    })
}

fn config_arguments(arguments: &Value) -> Result<(Option<&Value>, Option<&Value>), String> {
    match (arguments.get("draft"), arguments.get("manifest")) {
        (Some(draft), None) => Ok((Some(draft), None)),
        (None, Some(manifest)) => Ok((None, Some(manifest))),
        _ => Err("请只提供 draft 或 manifest 其中之一。".into()),
    }
}

fn parse_draft(value: &Value) -> Result<model::RecipeDraft, String> {
    serde_json::from_value(value.clone()).map_err(|error| format!("draft 字段无效：{error}"))
}

fn validate(backend: &Backend, arguments: &Value) -> Result<model::Recipe, String> {
    let apps = backend.snapshot()?.apps;
    match config_arguments(arguments)? {
        (Some(draft), _) => {
            let draft = parse_draft(draft)?;
            let id = match &draft.id {
                Some(id) if apps.iter().any(|app| &app.id == id && !app.builtin) => id.clone(),
                Some(_) => return Err("要修改的应用不存在，或是不能修改的内置应用。".into()),
                None => "new-application".into(),
            };
            let mut recipe = model::validate(draft, &apps, id)?;
            recipe.manifest = Some(manifest::from_recipe(&recipe));
            Ok(recipe)
        }
        (_, Some(value)) => manifest::recipe(value.clone(), &apps, false),
        _ => Err("请只提供 draft 或 manifest 其中之一。".into()),
    }
}

fn save(app: &AppHandle, arguments: &Value) -> Result<Value, String> {
    let before: BTreeSet<String> = app
        .state::<Backend>()
        .snapshot()?
        .apps
        .into_iter()
        .map(|app| app.id)
        .collect();
    let (snapshot, requested) = match config_arguments(arguments)? {
        (Some(draft), _) => {
            let draft = parse_draft(draft)?;
            let requested = draft.id.clone();
            (crate::save_application(draft, app.state())?, requested)
        }
        (_, Some(value)) => {
            let requested = value["id"].as_str().map(str::to_owned);
            (
                crate::save_manifest(
                    value.clone(),
                    crate::ManifestSaveMode::Upsert,
                    &app.state::<Backend>(),
                )?,
                requested,
            )
        }
        _ => return Err("请只提供 draft 或 manifest 其中之一。".into()),
    };
    let id = snapshot
        .apps
        .iter()
        .map(|app| &app.id)
        .find(|id| !before.contains(*id))
        .cloned()
        .or(requested)
        .ok_or("配置已保存，但无法确定应用 id。")?;
    Ok(json!({"saved": application_status(&snapshot, &id)}))
}

fn settled(snapshot: &model::Snapshot, id: &str) -> Option<&'static str> {
    if snapshot.runtime.connection == "disconnected" {
        return Some("disconnected");
    }
    let app = snapshot.apps.iter().find(|app| app.id == id)?;
    let state = snapshot.runtime.applications.get(id).map(String::as_str);
    match state {
        Some("failed") => Some("failed"),
        Some("stopped") if app.kind == "task" => Some("completed"),
        Some("stopped") => Some("exited"),
        _ if app.kind == "service" && app.health_path.is_some() => snapshot
            .runtime
            .application_urls
            .contains_key(id)
            .then_some("ready"),
        Some("running") if app.kind == "service" => Some("running"),
        _ => None,
    }
}

async fn start_tool(app: &AppHandle, arguments: &Value) -> Result<Value, String> {
    let id = text_argument(arguments, "id")?;
    let wait = integer_argument(arguments, "waitSeconds", (0, MAX_WAIT_SECONDS), 600)?;
    let deadline = Instant::now() + Duration::from_secs(wait);
    let mut snapshot = crate::start_application(id.clone(), app.state()).await?;
    loop {
        let outcome = settled(&snapshot, &id);
        let expired = Instant::now() >= deadline;
        if outcome.is_some() || expired {
            let outcome = outcome.unwrap_or(if wait == 0 { "submitted" } else { "timeout" });
            let mut result = json!({
                "outcome": outcome,
                "application": application_status(&snapshot, &id),
                "message": snapshot.runtime.message,
            });
            if !matches!(outcome, "ready" | "running" | "completed" | "submitted") {
                result["logs"] = logs_tail(app, &id, 80)
                    .await
                    .unwrap_or_else(|error| json!({"error": error}));
            }
            return Ok(result);
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        snapshot = crate::refresh_cloud(app.state()).await?;
    }
}

fn strip_terminal_codes(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' && characters.peek() == Some(&'[') {
            characters.next();
            for next in characters.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else if character == '\r' {
            output.push('\n');
        } else {
            output.push(character);
        }
    }
    output
}

fn tail(text: &str, lines: usize) -> (String, usize) {
    let cleaned = strip_terminal_codes(text);
    let all: Vec<&str> = cleaned
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = all.len().saturating_sub(lines);
    (all[start..].join("\n"), start)
}

async fn logs_tail(app: &AppHandle, id: &str, lines: usize) -> Result<Value, String> {
    let mut value = crate::application_logs(id.into(), app.state()).await?;
    if let Some(text) = value["text"].as_str() {
        let (text, omitted) = tail(text, lines);
        value["text"] = text.into();
        value["omittedLines"] = omitted.into();
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_token_must_match_exactly() {
        assert!(authorized(Some("Bearer secret-token"), "secret-token"));
        assert!(!authorized(Some("Bearer secret-toke"), "secret-token"));
        assert!(!authorized(Some("Bearer secret-token2"), "secret-token"));
        assert!(!authorized(Some("secret-token"), "secret-token"));
        assert!(!authorized(None, "secret-token"));
    }

    #[test]
    fn log_tail_removes_terminal_codes_and_keeps_last_lines() {
        let (text, omitted) = tail("one\n\u{1b}[32mtwo\u{1b}[0m\r\nthree\rfour\n", 2);
        assert_eq!(text, "three\nfour");
        assert_eq!(omitted, 2);
    }

    #[test]
    fn every_tool_has_an_object_schema() {
        let tools = tools();
        let names: BTreeSet<_> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), tools.len());
        for tool in &tools {
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
        assert!(!names.contains("start_instance"));
        assert!(!names.contains("shutdown_cloud"));
    }
}

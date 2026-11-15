use crate::{
    auth, connection, drive,
    model::{Event, Observation, Recipe, RuntimeStatus},
    transport::{proxy_origin, Transport},
};
use reqwest::Method;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::{client_async, tungstenite::client::IntoClientRequest};

#[cfg(feature = "native-smoke")]
pub(crate) fn live_stage(stage: &str, details: Value) {
    use std::io::Write;
    if !crate::live_check_mode() {
        return;
    }
    if let Some(root) = std::env::var_os("CLOUD_DECK_DATA_DIR") {
        let path = std::path::PathBuf::from(root).join("live-deployment.jsonl");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                file,
                "{}",
                json!({"time": crate::model::now(), "stage": stage, "details": details})
            );
        }
    }
}

#[derive(Clone)]
struct Runtime {
    endpoint: String,
    url: String,
    token: String,
}
impl Runtime {
    fn parse(value: &Value) -> Result<Self, String> {
        let endpoint = value["endpoint"]
            .as_str()
            .filter(|v| valid_endpoint(v))
            .ok_or("Colab 实例标识无效。")?
            .to_owned();
        let proxy = &value["runtimeProxyInfo"];
        let url = proxy["url"]
            .as_str()
            .ok_or("Colab 未返回实例代理。")?
            .to_owned();
        proxy_origin(&url)?;
        let token = proxy["token"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("实例连接令牌不可用。")?
            .to_owned();
        Ok(Self {
            endpoint,
            url,
            token,
        })
    }
    async fn api(&self, path: &str, method: Method, body: Option<Value>) -> Result<Value, String> {
        let mut url = proxy_origin(&self.url)?;
        url.set_path(path);
        let mut request = auth::client()?
            .request(method, url)
            .header("X-Colab-Runtime-Proxy-Token", &self.token)
            .header("X-Colab-Client-Agent", "colab-cli");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| "实例 API 连接失败。")?;
        if !response.status().is_success() {
            return Err(format!(
                "实例 API 请求失败（HTTP {}）。",
                response.status().as_u16()
            ));
        }
        response
            .json()
            .await
            .map_err(|_| "实例 API 返回的数据无效。".into())
    }
}
fn valid_endpoint(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn kernel_websocket_url(kernel: &str, session: &str) -> Result<url::Url, String> {
    if !valid_endpoint(kernel) || !valid_endpoint(session) {
        return Err("Drive 会话标识无效。".into());
    }
    let mut url = url::Url::parse("ws://127.0.0.1:8080").expect("static loopback URL");
    url.set_path(&format!("/api/kernels/{kernel}/channels"));
    url.query_pairs_mut().append_pair("session_id", session);
    Ok(url)
}

struct Connected {
    runtime: Runtime,
    transport: Transport,
    created_here: bool,
    wrote_data: bool,
    awaiting_ready: Option<String>,
}

impl connection::Reusable for Connected {
    fn endpoint(&self) -> &str {
        &self.runtime.endpoint
    }
    fn is_closed(&self) -> bool {
        self.transport.is_closed()
    }
    async fn close(&mut self) {
        self.transport.close().await;
    }
}

struct DriveAuthorization<'session> {
    session: &'session mut auth::Session,
    endpoint: &'session str,
}

impl drive::Authorizer for DriveAuthorization<'_> {
    async fn propagate(&mut self) -> Result<Option<String>, String> {
        propagate_drive(self.session, self.endpoint).await
    }
}

#[derive(Default)]
pub struct Cloud {
    status: Mutex<RuntimeStatus>,
    current: tokio::sync::Mutex<Option<Connected>>,
    trusted: Arc<Mutex<BTreeMap<String, String>>>,
    drive_url: Mutex<Option<String>>,
    pending_new: Mutex<Option<String>>,
}
impl Cloud {
    pub fn snapshot(&self, apps: &[Recipe]) -> Result<RuntimeStatus, String> {
        let mut status = self.status.lock().map_err(|_| "云端状态不可用。")?.clone();
        for app in apps {
            status
                .applications
                .entry(app.id.clone())
                .or_insert_with(|| "unknown".into());
        }
        Ok(status)
    }
    fn phase(&self, phase: &str, message: &str) {
        if let Ok(mut state) = self.status.lock() {
            state.phase = phase.into();
            state.message = message.into();
        }
    }
    pub fn failed(&self, message: &str) {
        if let Ok(mut state) = self.status.lock() {
            state.phase = "error".into();
            state.message = message.into();
            state.application_urls.clear();
        }
    }
    pub async fn disconnect(&self) -> Result<(), String> {
        *self.current.lock().await = None;
        *self.status.lock().map_err(|_| "云端状态不可用。")? = RuntimeStatus::default();
        *self.drive_url.lock().map_err(|_| "Drive 状态不可用。")? = None;
        *self
            .pending_new
            .lock()
            .map_err(|_| "实例归属状态不可用。")? = None;
        Ok(())
    }
    pub fn assert_editable(&self, id: &str) -> Result<(), String> {
        let status = self.status.lock().map_err(|_| "云端状态不可用。")?;
        if status
            .applications
            .get(id)
            .is_some_and(|value| value != "stopped")
        {
            return Err("先确认云端应用已停止，再编辑或移除本地配置。".into());
        }
        Ok(())
    }
    pub async fn boot(
        &self,
        session: &mut auth::Session,
        observation: &Observation,
        accelerator: &str,
    ) -> Result<(), String> {
        self.phase("resolving", "正在查询当前账号的实例…");
        let assignments = session
            .colab("/tun/m/assignments", Method::GET, &[], None, None)
            .await?;
        let instances = assignments["assignments"]
            .as_array()
            .ok_or("实例列表无效，未分配新机器。")?;
        if instances.len() > 1 {
            return Err("当前账号有多台实例，本版本不自动选择或申请更多机器。".into());
        }
        let (runtime, created_here) = if let Some(existing) = instances.first() {
            #[cfg(feature = "native-smoke")]
            live_stage("instance_reused", json!({"endpoint": existing["endpoint"]}));
            (Runtime::parse(existing)?, false)
        } else {
            validate_accelerator(accelerator, observation)?;
            self.phase("allocating", "正在申请实例；分配成功后即可能开始计费…");
            let seed = auth::random_token(33)?;
            let parameters = vec![
                ("nbh", seed),
                (
                    "variant",
                    if accelerator == "NONE" {
                        "DEFAULT"
                    } else {
                        "GPU"
                    }
                    .into(),
                ),
                ("accelerator", accelerator.into()),
            ];
            let step = session
                .colab("/tun/m/assign", Method::GET, &parameters, None, None)
                .await?;
            let assigned = if step.get("runtimeProxyInfo").is_some() {
                step
            } else {
                let csrf = step["token"]
                    .as_str()
                    .ok_or("Colab 未返回分配确认，未继续申请。")?;
                session
                    .colab("/tun/m/assign", Method::POST, &parameters, Some(csrf), None)
                    .await?
            };
            #[cfg(feature = "native-smoke")]
            live_stage(
                "instance_allocated",
                json!({"endpoint": assigned["endpoint"], "accelerator": accelerator}),
            );
            // Retain the allocation receipt before proxy validation/connection can fail.
            if let Some(endpoint) = assigned["endpoint"].as_str().filter(|v| valid_endpoint(v)) {
                *self
                    .pending_new
                    .lock()
                    .map_err(|_| "实例归属状态不可用。")? = Some(endpoint.into());
            }
            (Runtime::parse(&assigned)?, true)
        };
        self.ensure_connected(runtime, created_here).await
    }

    pub async fn reconnect(
        &self,
        session: &mut auth::Session,
        endpoint: &str,
    ) -> Result<(), String> {
        self.phase("resolving", "正在确认已有实例，恢复守护进程连接…");
        let assignments = session
            .colab("/tun/m/assignments", Method::GET, &[], None, None)
            .await?;
        let runtime = existing_runtime(&assignments, endpoint)?;
        self.ensure_connected(runtime, false).await
    }

    async fn ensure_connected(&self, runtime: Runtime, created_here: bool) -> Result<(), String> {
        let mut current = self.current.lock().await;
        let previous = current
            .as_ref()
            .filter(|connection| connection.runtime.endpoint == runtime.endpoint);
        let pending_owned = self
            .pending_new
            .lock()
            .map_err(|_| "实例归属状态不可用。")?
            .as_deref()
            == Some(runtime.endpoint.as_str());
        let created_here = created_here
            || previous.is_some_and(|connection| connection.created_here)
            || pending_owned;
        let wrote_data = previous.is_some_and(|connection| connection.wrote_data);
        {
            let mut status = self.status.lock().map_err(|_| "云端状态不可用。")?;
            status.endpoint = Some(runtime.endpoint.clone());
            status.connection = "not_connected".into();
            status.application_urls.clear();
        }
        self.phase("connecting", "正在建立内置的 Colab 受保护连接…");
        connection::ensure(&mut current, &runtime.endpoint, || async {
            let transport = Transport::connect(
                &runtime.endpoint,
                &runtime.url,
                &runtime.token,
                self.trusted.clone(),
            )
            .await?;
            Ok(Connected {
                runtime: runtime.clone(),
                transport,
                created_here,
                wrote_data,
                awaiting_ready: None,
            })
        })
        .await?;
        *self
            .pending_new
            .lock()
            .map_err(|_| "实例归属状态不可用。")? = None;
        let connection = current.as_mut().unwrap();
        connection.runtime = runtime;
        if let Ok(state) = agent(&connection.transport, "/v1/state", None).await {
            if state["protocolVersion"] != 1 {
                return Err("云端守护协议不兼容，实例保留。".into());
            }
            let revision = format!(
                "{:x}",
                Sha256::digest(
                    concat!(
                        include_str!("../../../cloud_agent.py"),
                        include_str!("../../../cloud_lifecycle.py"),
                        include_str!("../../../cloud_heartbeat.py")
                    )
                    .as_bytes()
                )
            );
            if state["daemon"]["revision"] == revision {
                self.daemon_connected()?;
                return Ok(());
            }
        }
        self.phase("deploying", "正在上传随包脚本并启动云端守护进程…");
        let payload = json!({
            "cloud_agent.py": include_str!("../../../cloud_agent.py"),
            "cloud_lifecycle.py": include_str!("../../../cloud_lifecycle.py"),
            "cloud_heartbeat.py": include_str!("../../../cloud_heartbeat.py")
        });
        connection
            .transport
            .execute(
                DEPLOY,
                &serde_json::to_vec(&payload).map_err(|_| "无法编码云端资源。")?,
                Duration::from_secs(45),
            )
            .await?;
        let state = agent(&connection.transport, "/v1/state", None).await?;
        if state["protocolVersion"] != 1 {
            return Err("云端守护协议不兼容，实例保留。".into());
        }
        self.daemon_connected()?;
        #[cfg(feature = "native-smoke")]
        live_stage(
            "daemon_connected",
            json!({"endpoint": connection.runtime.endpoint, "nativeSshOverWss":true}),
        );
        Ok(())
    }
    fn daemon_connected(&self) -> Result<(), String> {
        {
            let mut status = self.status.lock().map_err(|_| "云端状态不可用。")?;
            status.connection = "connected".into();
            if status.heartbeat["status"] == "inactive" {
                status.heartbeat = json!({"status":"waiting", "intervalSeconds":60});
            }
        }
        self.phase("connected", "实例与守护进程已就绪，可以部署应用。");
        Ok(())
    }
    pub async fn refresh(&self, apps: &[Recipe]) -> Result<Vec<Event>, String> {
        let mut guard = self.current.lock().await;
        let connection = guard.as_mut().ok_or("尚未连接云端守护进程。")?;
        if connection.transport.is_closed() {
            self.disconnected();
            return Err("云端连接已中断；应用和实例可能仍在运行，请重新连接。".into());
        }
        let state = match agent(&connection.transport, "/v1/state", None).await {
            Ok(state) => state,
            Err(error) => {
                self.disconnected();
                return Err(error);
            }
        };
        if state["protocolVersion"] != 1 {
            return Err("云端守护协议不兼容。".into());
        }
        let remote = state["applications"]
            .as_array()
            .ok_or("云端应用状态无效。")?;
        let mut states = BTreeMap::new();
        let mut urls = BTreeMap::new();
        for app in apps {
            let remote_app = remote.iter().find(|v| v["id"].as_str() == Some(&app.id));
            let status = remote_app
                .and_then(|v| v["state"].as_str())
                .unwrap_or("unknown");
            let status = if ["preparing", "running", "stopped", "failed"].contains(&status) {
                status
            } else {
                "unknown"
            };
            if remote_app.is_some() {
                states.insert(app.id.clone(), status.into());
            }
            if status == "running" && app.kind == "service" {
                if let (Some(local), Some(remote)) = (app.local_port, app.remote_port) {
                    connection.transport.forward(local, remote).await?;
                    let Some(health) = app.health_path.as_deref() else {
                        continue;
                    };
                    let url = format!("http://127.0.0.1:{local}");
                    let response = reqwest::Client::builder()
                        .no_proxy()
                        .redirect(reqwest::redirect::Policy::none())
                        .timeout(Duration::from_secs(2))
                        .build()
                        .map_err(|_| "无法检查本机应用入口。")?
                        .get(format!("{url}{health}"))
                        .send()
                        .await;
                    let expected = app
                        .readiness
                        .as_ref()
                        .and_then(|check| check["status"].as_u64())
                        .unwrap_or(200);
                    if response
                        .is_ok_and(|response| u64::from(response.status().as_u16()) == expected)
                    {
                        urls.insert(app.id.clone(), url);
                    }
                }
            } else if let Some(local) = app.local_port {
                connection.transport.stop_forward(local);
            }
        }
        let ready_message = connection.awaiting_ready.as_ref().and_then(|id| {
            let app = apps.iter().find(|app| &app.id == id)?;
            awaiting_outcome(
                &app.kind,
                app.health_path.is_some(),
                states.get(id).map(String::as_str),
                urls.contains_key(id),
            )
        });
        if ready_message.is_some() {
            connection.awaiting_ready = None;
        }
        let mut status = self.status.lock().map_err(|_| "云端状态不可用。")?;
        status.connection = "connected".into();
        if let Some(message) = ready_message {
            if status.phase == "connected" {
                status.message = message.into();
            }
        }
        status.heartbeat = state["heartbeat"].clone();
        status.applications = states;
        status.application_urls = urls;
        status.synchronization = state["synchronization"]["status"]
            .as_str()
            .filter(|v| ["unobserved", "pending", "syncing", "verified", "failed"].contains(v))
            .unwrap_or("unobserved")
            .into();
        status.storage = state["synchronization"].clone();
        if status.phase == "syncing" && status.synchronization == "verified" {
            status.phase = "connected".into();
            status.message = if state["synchronization"]["method"] == "no_save_rules" {
                "清单未配置保存文件；应用已停止，实例仍保留。"
            } else {
                "文件已通过 Drive 服务端校验；应用已停止，实例仍保留。"
            }
            .into();
        } else if status.phase == "syncing" && status.synchronization == "failed" {
            status.phase = "error".into();
            status.message = "文件保存或服务端校验失败；实例与本地文件保留。".into();
        }
        Ok(state["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|event| {
                Some(Event {
                    time: event["time"].as_u64()?,
                    level: if event["level"] == "error" {
                        "error"
                    } else {
                        "info"
                    }
                    .into(),
                    category: "application".into(),
                    message: format!(
                        "{}: {}",
                        event["applicationId"].as_str().unwrap_or("守护进程"),
                        event["message"].as_str()?
                    ),
                })
            })
            .collect())
    }
    fn disconnected(&self) {
        if let Ok(mut status) = self.status.lock() {
            status.connection = "disconnected".into();
            status.phase = "disconnected".into();
            status.application_urls.clear();
            for value in status.applications.values_mut() {
                *value = "unknown".into();
            }
        }
    }
    pub async fn start(&self, session: &mut auth::Session, app: &Recipe) -> Result<(), String> {
        let mut guard = self.current.lock().await;
        let connection = guard.as_mut().ok_or("请先连接实例并部署守护进程。")?;
        if !app.save_directories.is_empty()
            || app
                .manifest
                .as_ref()
                .is_some_and(crate::manifest::requires_drive)
        {
            self.phase("mounting_drive", "正在连接 Google Drive，成功前不启动应用…");
            self.mount_drive(session, connection).await?;
        }
        self.phase("starting", "正在向守护进程提交应用配置…");
        // Set this before submission: a timeout cannot prove preparation did not write.
        connection.wrote_data = true;
        agent(
            &connection.transport,
            "/v1/applications/start",
            Some(json!({"recipe": app, "files": crate::bundle::files(app)})),
        )
        .await?;
        connection.awaiting_ready = Some(app.id.clone());
        self.phase("connected", "应用已提交，正在等待云端健康检查。");
        Ok(())
    }
    pub async fn stop(&self, id: &str, local_port: Option<u16>) -> Result<(), String> {
        let mut guard = self.current.lock().await;
        let connection = guard.as_mut().ok_or("尚未连接实例。")?;
        agent(
            &connection.transport,
            "/v1/applications/stop",
            Some(json!({"applicationId": id})),
        )
        .await?;
        if connection.awaiting_ready.as_deref() == Some(id) {
            connection.awaiting_ready = None;
        }
        if let Some(local) = local_port {
            connection.transport.stop_forward(local);
        }
        self.phase("connected", "应用已停止；实例保留，计费需单独确认。");
        Ok(())
    }
    pub async fn logs(&self, id: &str) -> Result<Value, String> {
        let guard = self.current.lock().await;
        let connection = guard.as_ref().ok_or("尚未连接实例。")?;
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("applicationId", id)
            .finish();
        agent(&connection.transport, &format!("/v1/logs?{query}"), None).await
    }
    pub fn consent_url(&self) -> Result<String, String> {
        self.drive_url
            .lock()
            .map_err(|_| "Drive 授权状态不可用。")?
            .clone()
            .ok_or_else(|| "当前没有待处理的 Drive 授权。".into())
    }
    fn set_drive_consent(&self, consent: Option<String>) -> Result<(), String> {
        self.status
            .lock()
            .map_err(|_| "Drive 状态不可用。")?
            .needs_drive_consent = consent.is_some();
        *self.drive_url.lock().map_err(|_| "Drive 状态不可用。")? = consent;
        Ok(())
    }
    pub async fn synchronize(&self) -> Result<(), String> {
        let guard = self.current.lock().await;
        let connection = guard.as_ref().ok_or("请先连接实例。")?;
        agent(
            &connection.transport,
            "/v1/synchronization/start",
            Some(json!({})),
        )
        .await?;
        self.phase(
            "syncing",
            "正在停止应用并按清单保存、核验文件；完成前保留实例…",
        );
        Ok(())
    }
    pub async fn release_saved(&self, session: &mut auth::Session) -> Result<(), String> {
        let mut guard = self.current.lock().await;
        let connection = guard.as_ref().ok_or("请先连接实例。")?;
        let proof = agent(
            &connection.transport,
            "/v1/instance/release-check",
            Some(json!({})),
        )
        .await?;
        if proof["allowed"] != true
            || !matches!(
                proof["method"].as_str(),
                Some("drive_api_md5" | "no_save_rules")
            )
            || proof["receiptId"].as_str().is_none_or(str::is_empty)
        {
            return Err("清单保存规则尚未完成核验，已阻止释放。".into());
        }
        #[cfg(feature = "native-smoke")]
        live_stage(
            "saved_instance_release_ready",
            json!({"endpoint":connection.runtime.endpoint,"proof":proof,"storage":self.status.lock().map_err(|_| "云端状态不可用。")?.storage.clone()}),
        );
        self.phase(
            "releasing",
            "文件保存已重新核验，正在释放实例；计费等待查询确认…",
        );
        unassign(session, &connection.runtime.endpoint).await?;
        *guard = None;
        *self.status.lock().map_err(|_| "云端状态不可用。")? = RuntimeStatus::default();
        Ok(())
    }
    pub async fn release_empty(&self, session: &mut auth::Session) -> Result<(), String> {
        let mut guard = self.current.lock().await;
        let connection = guard.as_ref().ok_or("请先重新连接并确认实例。")?;
        let state = agent(&connection.transport, "/v1/state", None).await?;
        // This shortcut remains limited to newly allocated, never-used instances.
        if !can_release_empty(connection.created_here, connection.wrote_data, &state) {
            return Err(
                "此入口只释放本次新建且从未运行应用的空实例；已有应用请使用保存并关闭实例。".into(),
            );
        }
        let endpoint = connection.runtime.endpoint.clone();
        unassign(session, &endpoint).await?;
        *guard = None;
        *self.status.lock().map_err(|_| "云端状态不可用。")? = RuntimeStatus::default();
        Ok(())
    }
    #[cfg(feature = "native-smoke")]
    pub async fn validate_live_deployment(
        &self,
        session: &mut auth::Session,
        observation: &Observation,
    ) -> Result<Value, String> {
        use sha2::{Digest, Sha256};
        let suffix = auth::random_token(6)?;
        let task_id = format!("validation-task-{suffix}");
        let service_id = format!("validation-service-{suffix}");
        let local = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "无法预留本机验证端口。")?;
        let local_port = local.local_addr().map_err(|_| "无法读取验证端口。")?.port();
        drop(local);
        let mut task = crate::model::builtin();
        task.manifest = None;
        task.id = task_id.clone();
        task.name = "部署验证任务".into();
        task.kind = "task".into();
        task.builtin = false;
        task.prepare_command = None;
        task.environment.clear();
        task.save_directories.clear();
        task.working_directory = "/content".into();
        task.auto_start = false;
        task.remote_port = None;
        task.local_port = None;
        task.health_path = None;
        task.readiness = None;
        task.command =
            "python3 -I -B -u -c 'print(\"CLOUD_DECK_LIVE_TASK_OK\", flush=True)'".into();
        let mut service = task.clone();
        service.id = service_id.clone();
        service.name = "部署验证 HTTP 服务".into();
        service.kind = "service".into();
        service.remote_port = Some(19587);
        service.local_port = Some(local_port);
        service.health_path = Some("/health".into());
        service.command = "python3 -I -B -u -c 'import http.server\nclass Handler(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  self.send_response(200); self.end_headers(); self.wfile.write(b\"CLOUD_DECK_LIVE_SERVICE_OK\")\n def log_message(self,*args): pass\nhttp.server.HTTPServer((\"127.0.0.1\",19587),Handler).serve_forever()'".into();
        let checks = async {
            self.boot(session, observation, "NONE").await?;
            {
                let guard = self.current.lock().await;
                let connection = guard.as_ref().ok_or("验证连接不可用。")?;
                let result = connection.transport.execute("python3 -I -B -c 'import hashlib,json,pathlib\nr=pathlib.Path(\"/opt/cloud-deck\"); print(json.dumps({p:hashlib.sha256((r/p).read_bytes()).hexdigest() for p in [\"cloud_agent.py\",\"cloud_lifecycle.py\",\"cloud_heartbeat.py\"]}))'", &[], Duration::from_secs(15)).await?;
                let hashes: Value = serde_json::from_slice(&result).map_err(|_| "云端资源校验响应无效。")?;
                for (name, text) in [("cloud_agent.py", include_str!("../../../cloud_agent.py")), ("cloud_lifecycle.py", include_str!("../../../cloud_lifecycle.py")), ("cloud_heartbeat.py", include_str!("../../../cloud_heartbeat.py"))] {
                    if hashes[name].as_str() != Some(format!("{:x}", Sha256::digest(text.as_bytes())).as_str()) { return Err("云端资源校验不一致。".into()); }
                }
            }
            live_stage("resource_hashes_verified", json!({"files":3}));
            self.start(session, &task).await?;
            live_stage("validation_task_started", json!({"applicationId":task_id}));
            self.start(session, &service).await?;
            live_stage("validation_service_started", json!({"applicationId":service_id, "localPort":local_port}));
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                self.refresh(&[task.clone(), service.clone()]).await?;
                let status = self.snapshot(&[])?;
                if status.applications.get(&task_id).is_some_and(|v| v == "failed") || status.applications.get(&service_id).is_some_and(|v| v == "failed") { return Err("云端验证应用运行失败。".into()); }
                if status.applications.get(&task_id).is_some_and(|v| v == "stopped") && status.application_urls.contains_key(&service_id) { break; }
                if tokio::time::Instant::now() > deadline { return Err("云端应用或转发健康检查超时。".into()); }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
            if !self.logs(&task_id).await?["text"].as_str().is_some_and(|text| text.contains("CLOUD_DECK_LIVE_TASK_OK")) { return Err("云端任务日志没有通过验证。".into()); }
            let body = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().map_err(|_| "无法检查验证入口。")?
                .get(format!("http://127.0.0.1:{local_port}/health")).send().await.map_err(|_| "内置转发请求失败。")?
                .text().await.map_err(|_| "无法读取内置转发响应。")?;
            if body != "CLOUD_DECK_LIVE_SERVICE_OK" { return Err("内置转发返回了非验证服务的内容。".into()); }
            live_stage("task_logs_and_http_forward_verified", json!({"passed":true}));
            Ok::<(), String>(())
        }.await;
        // Both commands are fixed above: no user files, models or Drive writes.
        // Stop only these IDs; never release a pre-existing or mixed-use instance.
        let service_stopped = self.stop(&service_id, Some(local_port)).await.is_ok();
        let task_stopped = self.stop(&task_id, None).await.is_ok();
        live_stage(
            "validation_cleanup",
            json!({"taskStopped":task_stopped, "serviceStopped":service_stopped, "testError":checks.as_ref().err()}),
        );
        let mut guard = self.current.lock().await;
        let mut released = false;
        let pending = self
            .pending_new
            .lock()
            .map_err(|_| "实例归属状态不可用。")?
            .clone();
        let created_here = guard
            .as_ref()
            .is_some_and(|connection| connection.created_here)
            || pending.is_some();
        if let Some(connection) = guard.as_ref() {
            if connection.created_here {
                if connection.wrote_data {
                    let state = agent(&connection.transport, "/v1/state", None).await?;
                    if !state["applications"].as_array().is_some_and(|apps| apps.iter().all(|app| {
                        let known = matches!(app["id"].as_str(), Some(id) if id == task_id || id == service_id);
                        known && app["state"] == "stopped"
                    })) { return Err("测试清理未确认，已保留实例以保护数据；请检查计费。".into()); }
                }
                unassign(session, &connection.runtime.endpoint).await?;
                live_stage(
                    "test_instance_released",
                    json!({"endpoint":connection.runtime.endpoint}),
                );
                released = true;
            }
        } else if let Some(endpoint) = pending {
            // Connection failed before any upload or application command ran.
            unassign(session, &endpoint).await?;
            live_stage(
                "unused_test_instance_released",
                json!({"endpoint":endpoint}),
            );
            released = true;
        }
        if released {
            *guard = None;
            *self
                .pending_new
                .lock()
                .map_err(|_| "实例归属状态不可用。")? = None;
            *self.status.lock().map_err(|_| "云端状态不可用。")? = RuntimeStatus::default();
        }
        checks?;
        Ok(
            json!({"resourceHashesVerified":true,"nativeSshOverWss":true,"taskAndLogVerified":true,"serviceHealthVerified":true,"loopbackForwardVerified":true,"testProcessesStopped":true,"createdInstance":created_here,"newTestInstanceReleased":released,"existingInstancePreserved":!created_here}),
        )
    }

    #[cfg(feature = "native-smoke")]
    pub async fn manifest_probe(&self, action: &str) -> Result<Value, String> {
        if !["preflight", "download", "evidence"].contains(&action) {
            return Err("未知清单验证操作。".into());
        }
        let guard = self.current.lock().await;
        let connection = guard.as_ref().ok_or("尚未连接实例。")?;
        let command = format!(
            "python3 -I -B -c {}",
            format!(
                "'{}'",
                include_str!("../../scripts/manifest-probe.py").replace('\'', "'\"'\"'")
            )
        );
        let output = connection
            .transport
            .execute(
                &command,
                &serde_json::to_vec(&json!({"action":action,"manifest":crate::model::builtin().manifest,"files":crate::bundle::files(&crate::model::builtin())})).map_err(|_| "无法编码清单验证。")?,
                Duration::from_secs(180),
            )
            .await?;
        let mut evidence: Value =
            serde_json::from_slice(&output).map_err(|_| "清单验证返回无效数据。")?;
        if action == "preflight" {
            evidence["testInstance"] = json!(
                std::env::var("CLOUD_DECK_MANIFEST_CLEANUP_ENDPOINT")
                    .ok()
                    .as_deref()
                    == Some(&connection.runtime.endpoint)
            );
        }
        Ok(evidence)
    }

    #[cfg(feature = "native-smoke")]
    pub async fn inspect_comfy(&self) -> Result<Value, String> {
        let guard = self.current.lock().await;
        let connection = guard.as_ref().ok_or("尚未连接用于验证的实例。")?;
        let expected: Value = serde_json::from_str(include_str!("../../../manifest.json"))
            .map_err(|_| "验证基线无效。")?;
        let bytes = connection.transport.execute(
            "python3 -I -B -c 'import json,os,pathlib,subprocess,sys,urllib.request\nr=pathlib.Path(\"/opt/ComfyUI\"); m=json.loads(pathlib.Path(\"/opt/cloud-deck/state/manifests/comfyui/manifest.json\").read_text()); expected=json.load(sys.stdin)\nwith urllib.request.urlopen(\"http://127.0.0.1:8188/system_stats\",timeout=5) as response: stats=json.load(response)\nrevision=subprocess.check_output([\"git\",\"-C\",str(r),\"rev-parse\",\"HEAD\"],text=True).strip()\nmodels=[{\"path\":resource[\"target\"],\"sizeMatches\":pathlib.Path(resource[\"target\"]).is_file() and pathlib.Path(resource[\"target\"]).stat().st_size==resource[\"size\"]} for resource in m[\"resources\"][\"files\"]]\nprint(json.dumps({\"driveMounted\":os.path.ismount(\"/content/drive\"),\"version\":stats.get(\"system\",{}).get(\"comfyui_version\"),\"revisionMatches\":revision==expected[\"commit\"],\"devices\":[{\"type\":device.get(\"type\"),\"name\":device.get(\"name\")} for device in stats.get(\"devices\",[])],\"models\":models}))'",
            &serde_json::to_vec(&expected["comfyui"]).map_err(|_| "验证基线无效。")?, Duration::from_secs(20)).await?;
        let evidence: Value =
            serde_json::from_slice(&bytes).map_err(|_| "ComfyUI 验证结果无法解析。")?;
        if evidence["driveMounted"] != true
            || evidence["revisionMatches"] != true
            || !evidence["devices"]
                .as_array()
                .is_some_and(|items| items.iter().any(|d| d["type"] == "cuda"))
            || !evidence["models"].as_array().is_some_and(|items| {
                !items.is_empty() && items.iter().all(|m| m["sizeMatches"] == true)
            })
        {
            return Err("ComfyUI 的真实挂载、固定提交、CUDA 或模型文件检查未通过。".into());
        }
        live_stage("comfyui_verified", evidence.clone());
        Ok(evidence)
    }

    #[cfg(feature = "native-smoke")]
    pub async fn persistence_probe(&self, action: &str) -> Result<Value, String> {
        let guard = self.current.lock().await;
        let connection = guard.as_ref().ok_or("尚未连接实例。")?;
        match action {
            "ready" => {
                agent(
                    &connection.transport,
                    "/v1/instance/release-check",
                    Some(json!({"inspectOnly":true})),
                )
                .await
            }
            "reject_changed" => {
                if agent(
                    &connection.transport,
                    "/v1/instance/release-check",
                    Some(json!({})),
                )
                .await
                .is_ok()
                {
                    return Err("文件发生变化却允许释放。".into());
                }
                let state = agent(&connection.transport, "/v1/state", None).await?;
                if state["synchronization"]["status"] != "failed" {
                    return Err("未确认文件变化触发了释放拦截。".into());
                }
                Ok(json!({"changedFilesBlockedRelease":true}))
            }
            "prepare" | "mutate" | "recover" | "inspect" | "linux_tests" => {
                let payload = json!({"action":action, "workflow":serde_json::from_str::<Value>(include_str!("../../../examples/comfyui-wai170-deployment-test.json")).map_err(|_| "示例工作流无效。")?, "tests": include_str!("../../../test_cloud_agent.py"), "heartbeatTests": include_str!("../../../test_cloud_heartbeat.py"), "lifecycleTests": include_str!("../../../test_cloud_lifecycle.py"), "canonicalManifest": include_str!("../../applications/comfyui/manifest.json"), "bootstrap": include_str!("../../../cloud_bootstrap.py"), "baseline":include_str!("../../../manifest.json"), "setup":include_str!("../../applications/comfyui/setup.py")});
                let bytes = connection
                    .transport
                    .execute(
                        PERSISTENCE_FIXTURE,
                        &serde_json::to_vec(&payload).map_err(|_| "无法编码验证请求。")?,
                        Duration::from_secs(120),
                    )
                    .await?;
                serde_json::from_slice(&bytes).map_err(|_| "保存验证返回了无效响应。".into())
            }
            _ => Err("未知保存验证操作。".into()),
        }
    }
    async fn mount_drive(
        &self,
        session: &mut auth::Session,
        connection: &Connected,
    ) -> Result<(), String> {
        self.set_drive_consent(None)?;
        let mounted = connection
            .transport
            .execute(MOUNT_CHECK, &[], Duration::from_secs(15))
            .await?;
        if mounted == b"MOUNTED\n" {
            return Ok(());
        }
        let runtime = &connection.runtime;
        let sessions = runtime.api("/api/sessions", Method::GET, None).await?;
        let kernel = if let Some(id) = sessions
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["path"] == "cloud-deck.ipynb")
            .and_then(|s| s["kernel"]["id"].as_str())
        {
            id.to_owned()
        } else {
            runtime.api("/api/sessions", Method::POST, Some(json!({"path":"cloud-deck.ipynb", "name":"cloud-deck.ipynb", "type":"notebook", "kernel":{"name":"python3"}}))).await?
                ["kernel"]["id"].as_str().ok_or("无法建立 Drive 挂载会话。")?.to_owned()
        };
        let client_session = auth::random_token(16)?;
        // Colab's external kernel WebSocket may time out even while Jupyter is healthy.
        // Use the authenticated SSH channel to reach Jupyter's loopback port directly.
        let url = kernel_websocket_url(&kernel, &client_session)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| "无法建立 Drive 会话。")?;
        request
            .headers_mut()
            .insert("X-Colab-Client-Agent", "colab-cli".parse().unwrap());
        let stream = connection.transport.open_local(8080).await?;
        let (mut socket, _) =
            tokio::time::timeout(Duration::from_secs(30), client_async(request, stream))
                .await
                .map_err(|_| "Drive 会话连接超时。")?
                .map_err(|_| "Drive 会话连接失败。")?;
        #[cfg(feature = "native-smoke")]
        live_stage(
            "drive_session_connected",
            json!({"transport":"ssh_loopback"}),
        );
        let mut authorizer = DriveAuthorization {
            session,
            endpoint: &runtime.endpoint,
        };
        let result = drive::mount(&mut socket, &mut authorizer, &client_session, |consent| {
            let waiting = consent.is_some();
            self.set_drive_consent(consent)?;
            if waiting {
                self.phase(
                    "drive_consent",
                    "请在浏览器允许 Drive 访问，完成后会自动继续启动；实例仍可能计费。",
                );
                #[cfg(feature = "native-smoke")]
                live_stage("drive_consent_required", json!({}));
            } else {
                self.phase("mounting_drive", "Drive 授权已确认，正在继续挂载…");
            }
            Ok(())
        })
        .await;
        self.set_drive_consent(None)?;
        if let Err(error) = result {
            // Stop our outstanding mount request on failure so retrying does not
            // queue behind a kernel still waiting for the old authorization.
            let _ = runtime
                .api(
                    &format!("/api/kernels/{kernel}/interrupt"),
                    Method::POST,
                    Some(json!({})),
                )
                .await;
            return Err(error);
        }
        if connection
            .transport
            .execute(MOUNT_CHECK, &[], Duration::from_secs(15))
            .await?
            != b"MOUNTED\n"
        {
            return Err("Drive 没有形成真实挂载，应用未启动。".into());
        }
        Ok(())
    }
}

fn existing_runtime(assignments: &Value, endpoint: &str) -> Result<Runtime, String> {
    if !valid_endpoint(endpoint) {
        return Err("实例标识无效。".into());
    }
    let instances = assignments["assignments"]
        .as_array()
        .ok_or("实例列表无效，未建立连接。")?;
    if instances.len() > 1 {
        return Err("当前账号有多台实例，本版本不自动切换。".into());
    }
    let existing = instances
        .iter()
        .find(|instance| instance["endpoint"] == endpoint)
        .ok_or("原实例已不存在，请刷新状态；本次重连没有申请新实例。")?;
    Runtime::parse(existing)
}

fn can_release_empty(created_here: bool, wrote_data: bool, agent: &Value) -> bool {
    created_here
        && !wrote_data
        && agent["protocolVersion"] == 1
        && agent["applications"].as_array().is_some_and(Vec::is_empty)
}

fn awaiting_outcome(
    kind: &str,
    has_health_check: bool,
    state: Option<&str>,
    healthy: bool,
) -> Option<&'static str> {
    match state {
        Some("failed") => Some("应用启动失败，请查看应用日志中的错误输出。"),
        Some("stopped") if kind == "task" => Some("任务已结束，退出状态正常。"),
        Some("stopped") => Some("应用进程已退出，未通过就绪检查；请查看应用日志。"),
        _ if kind == "service" && has_health_check => {
            healthy.then_some("应用已就绪，健康检查通过。")
        }
        Some("running") => Some("应用已就绪，进程正在运行。"),
        _ => None,
    }
}

pub fn validate_accelerator(accelerator: &str, observation: &Observation) -> Result<(), String> {
    if observation.status != "current" {
        return Err("先刷新并确认账号状态，再启动实例。".into());
    }
    if accelerator != "NONE"
        && (!observation.eligible_gpus.iter().any(|v| v == accelerator)
            || observation.balance.is_none_or(|v| v <= 0.0))
    {
        return Err("该 GPU 未在本次账号能力中确认，或没有可用计算点数。".into());
    }
    Ok(())
}
async fn unassign(session: &mut auth::Session, endpoint: &str) -> Result<(), String> {
    if !valid_endpoint(endpoint) {
        return Err("实例标识无效。".into());
    }
    let path = format!("/tun/m/unassign/{endpoint}");
    let step = session.colab(&path, Method::GET, &[], None, None).await?;
    let csrf = step["token"]
        .as_str()
        .ok_or("Colab 未返回释放确认，实例保留。")?;
    session
        .colab(&path, Method::POST, &[], Some(csrf), None)
        .await?;
    Ok(())
}
async fn propagate_drive(
    session: &mut auth::Session,
    endpoint: &str,
) -> Result<Option<String>, String> {
    let path = format!("/tun/m/credentials-propagation/{endpoint}");
    let mut parameters = vec![
        ("authtype", "dfs_ephemeral".into()),
        ("version", "2".into()),
        ("propagate", "true".into()),
        ("record", "false".into()),
        ("dryrun", "true".into()),
    ];
    let step = session
        .colab(&path, Method::GET, &parameters, None, None)
        .await?;
    let csrf = step["token"].as_str().ok_or("Drive 授权未返回确认凭据。")?;
    let boundary = format!("CloudDeck{}", auth::random_token(16)?);
    let body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file_id\"\r\n\r\nempty.ipynb\r\n--{boundary}--\r\n").into_bytes();
    let content_type = format!("multipart/form-data; boundary={boundary}");
    parameters.last_mut().unwrap().1 = "false".into();
    let real = session
        .colab(
            &path,
            Method::POST,
            &parameters,
            Some(csrf),
            Some((&content_type, body.clone())),
        )
        .await;
    if real.is_ok_and(|v| v["success"] == true) {
        return Ok(None);
    }
    parameters.last_mut().unwrap().1 = "true".into();
    let dry = session
        .colab(
            &path,
            Method::POST,
            &parameters,
            Some(csrf),
            Some((&content_type, body)),
        )
        .await?;
    let consent = dry["unauthorized_redirect_uri"]
        .as_str()
        .ok_or("Drive 访问未获允许，应用未启动。")?;
    let url = url::Url::parse(consent).map_err(|_| "Drive 授权地址无效。")?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("accounts.google.com" | "colab.research.google.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err("Drive 返回了非 Google 授权地址，未打开。".into());
    }
    Ok(Some(url.into()))
}
async fn agent(transport: &Transport, path: &str, body: Option<Value>) -> Result<Value, String> {
    let payload =
        serde_json::to_vec(&json!({"path":path,"body":body})).map_err(|_| "无法编码守护请求。")?;
    let bytes = transport
        .execute(AGENT_REQUEST, &payload, Duration::from_secs(20))
        .await?;
    parse_agent_response(&bytes)
}
fn parse_agent_response(bytes: &[u8]) -> Result<Value, String> {
    let result: Value = serde_json::from_slice(bytes).map_err(|_| "守护进程返回了无效响应。")?;
    if !result.is_object() {
        return Err("守护进程返回了无效响应。".into());
    }
    if result.get("error").is_some_and(|error| !error.is_null()) {
        return Err("云端守护进程拒绝操作，请检查应用目录、命令及端口；实例保留。".into());
    }
    Ok(result)
}
#[cfg(feature = "native-smoke")]
const PERSISTENCE_FIXTURE: &str = r#"python3 -I -B -c '
import hashlib,json,pathlib,secrets,shutil,sys
p=json.load(sys.stdin); root=pathlib.Path("/opt/comfy-local"); state_path=pathlib.Path("/opt/cloud-deck/state/persistence-test.json")
if p["action"]=="linux_tests":
 import os,subprocess,tempfile
 with tempfile.TemporaryDirectory(prefix="agent-check-",dir=str(state_path.parent)) as name:
  directory=pathlib.Path(name)
  for filename in ("cloud_agent.py","cloud_lifecycle.py","cloud_heartbeat.py"): shutil.copyfile(state_path.parent.parent/filename,directory/filename)
  (directory/"cloud_bootstrap.py").write_text(p["bootstrap"])
  (directory/"manifest.json").write_text(p["baseline"])
  tests=directory/"test_cloud_agent.py"; tests.write_text(p["tests"])
  (directory/"test_cloud_lifecycle.py").write_text(p["lifecycleTests"])
  (directory/"test_cloud_heartbeat.py").write_text(p["heartbeatTests"])
  canonical=directory/"client/applications/comfyui/manifest.json"; canonical.parent.mkdir(parents=True); canonical.write_text(p["canonicalManifest"])
  (canonical.parent/"setup.py").write_text(p["setup"])
  result=subprocess.run([sys.executable,"-I","-B","-m","unittest","discover","-s",str(directory),"-p","test_cloud*.py","-v"],env={**os.environ,"TMPDIR":name},capture_output=True,text=True,timeout=90)
  print(json.dumps({"passed":result.returncode==0,"exitCode":result.returncode,"output":(result.stdout+result.stderr)[-10000:]}))
 sys.exit(0)
def evidence(path):
 return {"path":str(path.relative_to(root)),"size":path.stat().st_size,"sha256":hashlib.sha256(path.read_bytes()).hexdigest()}
if p["action"]=="prepare":
 suffix=secrets.token_hex(6); prefix="CloudDeck_Persistence_"+suffix
 image=root/"output/CloudDeck_Deployment_Test_00001_.png"
 assert image.is_file(), "Expected real generated output is missing"
 workflow=root/("user/default/workflows/"+prefix+".json"); workflow.parent.mkdir(parents=True,exist_ok=True)
 settings=root/("user/default/"+prefix+"_settings.json")
 sample=root/("input/"+prefix+".png")
 for path in (workflow,settings,sample): assert not path.exists()
 workflow.write_text(json.dumps(p["workflow"])); settings.write_text(json.dumps({"test":"cloud-deck-save","theme":"dark","seed":24680})); shutil.copyfile(image,sample)
 state={"files":[evidence(path) for path in (workflow,settings,sample)],"output":evidence(image)}; state_path.write_text(json.dumps(state))
else:
 state=json.loads(state_path.read_text())
 if p["action"]=="mutate":
  path=root/state["files"][0]["path"]; assert path.name.startswith("CloudDeck_Persistence_")
  value=json.loads(path.read_text()); value["5"]["inputs"]["seed"]+=1; path.write_text(json.dumps(value)); state["files"][0]=evidence(path); state_path.write_text(json.dumps(state))
 elif p["action"]=="recover":
  backup=state_path.parent/("persistence-backup-"+secrets.token_hex(6)); backup.mkdir()
  for index,item in enumerate(state["files"]):
   path=root/item["path"]; assert path.name.startswith("CloudDeck_Persistence_") and evidence(path)==item; path.rename(backup/str(index))
 elif p["action"]=="inspect":
  for item in state["files"]: assert evidence(root/item["path"])==item, "Restored file did not match"
  assert evidence(root/state["output"]["path"])==state["output"]
print(json.dumps(state))
'"#;
const MOUNT_CHECK: &str = "python3 -I -B -c 'import os; print(\"MOUNTED\" if os.path.ismount(\"/content/drive\") else \"NOT_MOUNTED\")'";
const DEPLOY: &str = "python3 -I -B -c 'import json,os,pathlib,subprocess,sys\nroot=pathlib.Path(\"/opt/cloud-deck\"); root.mkdir(parents=True,exist_ok=True,mode=0o700)\nfor name,text in json.load(sys.stdin).items():\n p=root/name; p.parent.mkdir(parents=True,exist_ok=True,mode=0o700); t=p.with_suffix(\".upload\"); t.write_text(text); t.chmod(0o600); os.replace(t,p)\nsubprocess.run([sys.executable,\"-I\",\"-B\",str(root/\"cloud_agent.py\"),\"start\",\"--heartbeat\"],check=True,timeout=20)\n'";
const AGENT_REQUEST: &str = "python3 -I -B -c 'import json,pathlib,sys,urllib.request,urllib.error\np=json.load(sys.stdin); token=pathlib.Path(\"/opt/cloud-deck/state/agent-token\").read_text().strip(); body=p[\"body\"]; raw=None if body is None else json.dumps(body).encode(); req=urllib.request.Request(\"http://127.0.0.1:18775\"+p[\"path\"],data=raw,headers={\"Authorization\":\"Bearer \"+token,\"Content-Type\":\"application/json\"})\ntry:\n with urllib.request.urlopen(req,timeout=12) as response: print(response.read().decode())\nexcept urllib.error.HTTPError: print(json.dumps({\"error\":\"Agent request rejected\"}))\n'";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_requires_the_requested_existing_instance() {
        let runtime = json!({"endpoint":"existing-l4","runtimeProxyInfo":{"url":"https://runtime.prod.r.colab.dev","token":"test-token"}});
        let assignments = json!({"assignments":[runtime]});
        assert_eq!(
            existing_runtime(&assignments, "existing-l4")
                .unwrap()
                .endpoint,
            "existing-l4"
        );
        assert!(existing_runtime(&assignments, "released-instance").is_err());
        assert!(existing_runtime(&json!({"assignments":[]}), "existing-l4").is_err());
        assert!(existing_runtime(&json!({}), "existing-l4").is_err());
    }
    #[test]
    fn drive_kernel_connections_stay_on_the_instance_loopback() {
        let url = kernel_websocket_url("kernel-123", "client_456").unwrap();
        assert_eq!(url.scheme(), "ws");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.path(), "/api/kernels/kernel-123/channels");
        assert_eq!(url.query(), Some("session_id=client_456"));
        for invalid in ["../other", "kernel/other", "kernel?token=x", ""] {
            assert!(kernel_websocket_url(invalid, "client_456").is_err());
        }
    }
    #[test]
    fn awaiting_ready_settles_on_failure_exit_or_readiness() {
        assert!(awaiting_outcome("service", true, Some("preparing"), false).is_none());
        assert!(awaiting_outcome("service", true, Some("running"), false).is_none());
        assert!(awaiting_outcome("service", true, Some("running"), true)
            .is_some_and(|message| message.contains("健康检查通过")));
        assert!(awaiting_outcome("service", true, Some("failed"), false)
            .is_some_and(|message| message.contains("启动失败")));
        assert!(awaiting_outcome("service", true, Some("stopped"), false)
            .is_some_and(|message| message.contains("已退出")));
        assert!(awaiting_outcome("task", false, Some("stopped"), false)
            .is_some_and(|message| message.contains("任务已结束")));
        assert!(awaiting_outcome("task", false, Some("running"), false).is_some());
        assert!(awaiting_outcome("task", false, None, false).is_none());
    }
    #[test]
    fn successful_agent_start_with_null_error_is_not_a_rejection() {
        let started = br#"{"id":"task","state":"running","pid":123,"exitCode":null,"error":null}"#;
        assert_eq!(parse_agent_response(started).unwrap()["state"], "running");
        assert!(parse_agent_response(br#"{"protocolVersion":1,"applications":[]}"#).is_ok());
        assert!(parse_agent_response(br#"{"error":"Agent request rejected"}"#).is_err());
        assert!(parse_agent_response(b"[]").is_err());
    }
    #[test]
    fn release_requires_a_new_never_used_and_confirmed_empty_instance() {
        let empty = json!({"protocolVersion":1,"applications":[]});
        assert!(can_release_empty(true, false, &empty));
        assert!(!can_release_empty(false, false, &empty));
        assert!(!can_release_empty(true, true, &empty));
        assert!(!can_release_empty(true, false, &json!({})));
        assert!(!can_release_empty(
            true,
            false,
            &json!({"protocolVersion":1,"applications":[{"state":"stopped"}]})
        ));
    }
    #[test]
    fn requests_require_current_observation_and_actual_gpu_entitlement() {
        let mut observation = Observation::default();
        assert!(validate_accelerator("NONE", &observation).is_err());
        observation.status = "current".into();
        observation.balance = Some(10.0);
        observation.eligible_gpus = vec!["T4".into()];
        assert!(validate_accelerator("NONE", &observation).is_ok());
        assert!(validate_accelerator("T4", &observation).is_ok());
        assert!(validate_accelerator("H100", &observation).is_err());
        observation.balance = Some(0.0);
        assert!(validate_accelerator("T4", &observation).is_err());
        for endpoint in ["../other", "abc/def", "", "abc?token=other"] {
            assert!(!valid_endpoint(endpoint));
        }
    }
}

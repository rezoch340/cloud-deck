mod auth;
mod bundle;
mod cloud;
mod connection;
mod credentials;
mod drive;
mod manifest;
mod mcp;
mod mcp_bridge;
mod model;
#[cfg(feature = "native-smoke")]
mod native_smoke;
mod readiness;
mod transport;

/// `--mcp-stdio` 入口：把 MCP 客户端的标准输入输出转发给正在运行的客户端。
pub fn mcp_stdio(args: &[String]) -> i32 {
    mcp_bridge::run(args)
}

fn smoke_mode() -> bool {
    cfg!(feature = "native-smoke") && std::env::var("CLOUD_DECK_SMOKE").as_deref() == Ok("1")
}
fn live_check_mode() -> bool {
    cfg!(feature = "native-smoke")
        && std::env::var("CLOUD_DECK_LIVE_CHECK").as_deref() == Ok("1")
        && !smoke_mode()
}

#[tauri::command]
fn native_check_report(app: tauri::AppHandle, report: serde_json::Value) -> Result<(), String> {
    if !smoke_mode() && !live_check_mode() {
        return Err("Native check mode is not enabled".into());
    }
    #[cfg(feature = "native-smoke")]
    {
        native_smoke::finish(app, report)
    }
    #[cfg(not(feature = "native-smoke"))]
    {
        let _ = (app, report);
        Err("Native checks are not included in this build".into())
    }
}

use model::{Observation, RecipeDraft, Snapshot, Stored};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

struct Backend {
    path: PathBuf,
    data: Mutex<Result<Stored, String>>,
    operation: tokio::sync::Mutex<()>,
    login_cancel: Mutex<Option<Arc<AtomicBool>>>,
    login_url: Arc<Mutex<Option<String>>>,
    cloud: cloud::Cloud,
}

impl Backend {
    fn snapshot(&self) -> Result<Snapshot, String> {
        let guard = self.data.lock().map_err(|_| "工作空间状态不可用。")?;
        let data = guard.as_ref().map_err(Clone::clone)?;
        Ok(Snapshot {
            runtime: self.cloud.snapshot(&data.apps())?,
            apps: data.apps(),
            account: data.account.clone(),
            observation: data.observation.clone(),
            events: data.events.clone(),
            data_directory: self
                .path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_string_lossy()
                .into(),
            mode: "desktop".into(),
        })
    }

    fn change(&self, edit: impl FnOnce(&mut Stored) -> Result<(), String>) -> Result<(), String> {
        let mut guard = self.data.lock().map_err(|_| "工作空间状态不可用。")?;
        let mut next = guard.as_ref().map_err(Clone::clone)?.clone();
        edit(&mut next)?;
        next.save(&self.path)?;
        *guard = Ok(next);
        Ok(())
    }

    fn query_error(&self, error: &str) -> Result<(), String> {
        self.change(|data| {
            data.observation.status = "error".into();
            data.observation.error = Some(error.into());
            data.event("error", "connection", error);
            Ok(())
        })
    }
}

#[tauri::command]
fn get_workspace(backend: State<'_, Backend>) -> Result<Snapshot, String> {
    backend.snapshot()
}

#[tauri::command]
fn save_application(draft: RecipeDraft, backend: State<'_, Backend>) -> Result<Snapshot, String> {
    if let Some(id) = &draft.id {
        backend.cloud.assert_editable(id)?;
    }
    let id = match &draft.id {
        Some(id) => id.clone(),
        None => format!("app-{}", auth::random_token(9)?),
    };
    backend.change(|data| {
        if draft.id.is_some() && !data.custom_apps.iter().any(|a| a.id == id) {
            return Err("要修改的应用不存在，配置未保存。".into());
        }
        let mut recipe = model::validate(draft, &data.apps(), id.clone())?;
        recipe.manifest = Some(manifest::from_recipe(&recipe));
        let name = recipe.name.clone();
        if let Some(existing) = data.custom_apps.iter_mut().find(|a| a.id == id) {
            *existing = recipe;
        } else {
            data.custom_apps.push(recipe);
        }
        data.event("info", "application", format!("已保存应用配置：{name}"));
        Ok(())
    })?;
    backend.snapshot()
}

#[tauri::command]
fn save_application_manifest(
    manifest: serde_json::Value,
    backend: State<'_, Backend>,
) -> Result<Snapshot, String> {
    let identifier = manifest["id"]
        .as_str()
        .ok_or("应用清单缺少 id。")?
        .to_owned();
    backend.cloud.assert_editable(&identifier)?;
    backend.change(|data| {
        let recipe = manifest::recipe(manifest, &data.apps(), false)?;
        let name = recipe.name.clone();
        if let Some(existing) = data
            .custom_apps
            .iter_mut()
            .find(|application| application.id == identifier)
        {
            *existing = recipe;
        } else {
            data.custom_apps.push(recipe);
        }
        data.event(
            "info",
            "application",
            format!("已保存应用生命周期清单：{name}"),
        );
        Ok(())
    })?;
    backend.snapshot()
}

#[tauri::command]
fn remove_application(id: String, backend: State<'_, Backend>) -> Result<Snapshot, String> {
    backend.cloud.assert_editable(&id)?;
    if id == "comfyui" {
        return Err("内置应用不能移除。".into());
    }
    backend.change(|data| {
        let index = data
            .custom_apps
            .iter()
            .position(|a| a.id == id)
            .ok_or("应用不存在。")?;
        let app = data.custom_apps.remove(index);
        data.event(
            "info",
            "application",
            format!("已移除应用配置：{}；未删除云端文件。", app.name),
        );
        Ok(())
    })?;
    backend.snapshot()
}

#[tauri::command]
async fn sign_in(app: tauri::AppHandle, backend: State<'_, Backend>) -> Result<Snapshot, String> {
    if smoke_mode() {
        return Err("原生验证模式不访问 Google 或系统授权存储。".into());
    }
    let _operation = backend
        .operation
        .try_lock()
        .map_err(|_| "另一个账号操作正在进行，请稍候。")?;
    auth::Session::retry_credential_access()?;
    let secure = auth::Session::load_optional()?;
    let expected = {
        let guard = backend.data.lock().map_err(|_| "工作空间状态不可用。")?;
        let data = guard.as_ref().map_err(Clone::clone)?;
        data.expected_login_subject(secure.as_ref().map(|session| &session.account))
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    *backend
        .login_cancel
        .lock()
        .map_err(|_| "登录状态不可用。")? = Some(cancelled.clone());
    let result = auth::login(app.clone(), cancelled.clone(), backend.login_url.clone()).await;
    *backend
        .login_cancel
        .lock()
        .map_err(|_| "登录状态不可用。")? = None;
    *backend.login_url.lock().map_err(|_| "登录状态不可用。")? = None;
    let mut session = match result {
        Ok(session) => session,
        Err(error) => {
            backend.change(|data| {
                data.event("error", "account", &error);
                Ok(())
            })?;
            return Err(error);
        }
    };
    if expected
        .as_ref()
        .is_some_and(|subject| *subject != session.account.subject)
    {
        let error = "当前账号的实例或计费尚未确认，不能切换。请重新连接原来的 Google 账号。";
        backend.change(|data| {
            data.event("error", "account", error);
            Ok(())
        })?;
        return Err(error.into());
    }
    if cancelled.load(Ordering::SeqCst) {
        return Err("登录已取消。".into());
    }
    let _ = app.emit(
        "auth-progress",
        auth::Progress {
            phase: "checking".into(),
            message: "账号已验证，正在安全保存授权并查询 Colab…".into(),
        },
    );
    session.store()?;
    if backend.snapshot()?.account.as_ref().map(|a| &a.subject) != Some(&session.account.subject) {
        backend.cloud.disconnect().await?;
    }
    backend.change(|data| {
        if data.account.as_ref().map(|a| &a.subject) != Some(&session.account.subject) {
            data.observation = Observation::default();
        }
        data.account = Some(session.account.clone());
        data.event(
            "info",
            "account",
            "Google 账号已连接，授权由系统凭据存储保存。",
        );
        Ok(())
    })?;
    match session.observe().await {
        Ok(observation) => backend.change(|data| {
            data.observe(observation);
            data.event(
                "info",
                "instance",
                "Colab 只读状态查询完成；未申请或释放实例。",
            );
            Ok(())
        })?,
        Err(error) => backend.query_error(&error)?,
    }
    backend.snapshot()
}

#[tauri::command]
fn cancel_sign_in(backend: State<'_, Backend>) -> Result<(), String> {
    if let Some(cancel) = backend
        .login_cancel
        .lock()
        .map_err(|_| "登录状态不可用。")?
        .as_ref()
    {
        cancel.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
fn reopen_sign_in(app: tauri::AppHandle, backend: State<'_, Backend>) -> Result<(), String> {
    let guard = backend.login_url.lock().map_err(|_| "登录状态不可用。")?;
    let url = guard.as_ref().ok_or("没有正在等待的登录，请重新开始。")?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "无法打开系统浏览器。".into())
}

#[tauri::command]
async fn refresh_account(backend: State<'_, Backend>) -> Result<Snapshot, String> {
    if smoke_mode() {
        return Err("原生验证模式不访问 Google 或系统授权存储。".into());
    }
    // A read-only manual refresh queues behind a background status poll.
    let _operation = backend.operation.lock().await;
    let mut session = match auth::Session::load() {
        Ok(session) => session,
        Err(error) => {
            backend.query_error(&error)?;
            return backend.snapshot();
        }
    };
    let snapshot = backend.snapshot()?;
    if snapshot
        .account
        .as_ref()
        .is_some_and(|a| a.subject != session.account.subject)
    {
        return Err("本机授权与当前工作空间不一致，请重新登录原账号。".into());
    }
    match session.observe().await {
        Ok(observation) => backend.change(|data| {
            let mut account = session.account.clone();
            account.status = "connected".into();
            data.account = Some(account);
            data.observe(observation);
            data.event("info", "instance", "Colab 只读状态已刷新。");
            Ok(())
        })?,
        Err(error) => backend.query_error(&error)?,
    }
    backend.snapshot()
}

#[tauri::command]
async fn sign_out(backend: State<'_, Backend>) -> Result<Snapshot, String> {
    if smoke_mode() {
        return Err("原生验证模式不访问 Google 或系统授权存储。".into());
    }
    let _operation = backend
        .operation
        .try_lock()
        .map_err(|_| "另一个账号操作正在进行，请稍候。")?;
    let secure = auth::Session::load_optional()?;
    {
        let guard = backend.data.lock().map_err(|_| "工作空间状态不可用。")?;
        let data = guard.as_ref().map_err(Clone::clone)?;
        if data
            .expected_login_subject(secure.as_ref().map(|session| &session.account))
            .is_some()
        {
            return Err(
                "先刷新并确认当前授权账号没有运行中的实例和计费，再退出。退出不会关闭云端机器。"
                    .into(),
            );
        }
    }
    auth::Session::forget()?;
    backend.cloud.disconnect().await?;
    backend.change(|data| {
        data.account = None;
        data.observation = Observation::default();
        data.event("info", "account", "已移除本机 Google 授权，应用配置保留。");
        Ok(())
    })?;
    backend.snapshot()
}

async fn cloud_session(backend: &Backend) -> Result<(auth::Session, Observation), String> {
    if smoke_mode() {
        return Err("离线原生检查不访问 Google 或真实实例。".into());
    }
    let mut session = auth::Session::load()?;
    if backend.snapshot()?.account.as_ref().map(|a| &a.subject) != Some(&session.account.subject) {
        return Err("授权账号与工作空间不一致，请重新连接原账号。".into());
    }
    let observation = match session.observe().await {
        Ok(value) => value,
        Err(error) => {
            backend.query_error(&error)?;
            return Err(error);
        }
    };
    backend.change(|data| {
        data.observe(observation.clone());
        Ok(())
    })?;
    Ok((session, observation))
}
fn cloud_result(
    backend: &Backend,
    result: Result<(), String>,
    message: &str,
) -> Result<Snapshot, String> {
    match result {
        Ok(()) => {
            backend.change(|data| {
                data.event("info", "connection", message);
                Ok(())
            })?;
            backend.snapshot()
        }
        Err(error) => {
            backend.cloud.failed(&error);
            backend.change(|data| {
                data.event("error", "connection", &error);
                Ok(())
            })?;
            Err(error)
        }
    }
}
async fn poll_cloud(backend: &Backend) -> Result<(), String> {
    let events = backend.cloud.refresh(&backend.snapshot()?.apps).await?;
    backend.change(|data| {
        for event in events {
            if !data.events.iter().any(|old| {
                old.time == event.time
                    && old.category == event.category
                    && old.message == event.message
            }) {
                data.events.push(event);
            }
        }
        if data.events.len() > 500 {
            data.events.drain(..data.events.len() - 500);
        }
        Ok(())
    })
}

#[tauri::command]
async fn start_instance(
    accelerator: String,
    backend: State<'_, Backend>,
) -> Result<Snapshot, String> {
    let _operation = backend
        .operation
        .try_lock()
        .map_err(|_| "另一个操作正在进行。")?;
    let (mut session, observation) = cloud_session(&backend).await?;
    backend.change(|data| {
        data.observation.status = "cached".into();
        data.event(
            "info",
            "instance",
            "正在启动实例并连接守护进程；应用稍后单独部署。",
        );
        Ok(())
    })?;
    let result = backend
        .cloud
        .boot(&mut session, &observation, &accelerator)
        .await;
    match session.observe().await {
        Ok(value) => backend.change(|data| {
            data.observe(value);
            Ok(())
        })?,
        Err(error) => backend.query_error(&error)?,
    }
    let result = async {
        result?;
        poll_cloud(&backend).await
    }
    .await;
    cloud_result(
        &backend,
        result,
        "实例与云端守护进程已就绪，请按需部署应用。",
    )
}

#[tauri::command]
async fn connect_instance(
    endpoint: String,
    backend: State<'_, Backend>,
) -> Result<Snapshot, String> {
    let _operation = backend
        .operation
        .try_lock()
        .map_err(|_| "另一个操作正在进行。")?;
    let (mut session, _) = cloud_session(&backend).await?;
    let result = async {
        backend.cloud.reconnect(&mut session, &endpoint).await?;
        poll_cloud(&backend).await
    }
    .await;
    cloud_result(
        &backend,
        result,
        "已恢复现有实例与守护进程连接，应用保持原运行状态。",
    )
}

#[tauri::command]
async fn refresh_cloud(backend: State<'_, Backend>) -> Result<Snapshot, String> {
    let Ok(_operation) = backend.operation.try_lock() else {
        return backend.snapshot();
    };
    if backend.snapshot()?.runtime.endpoint.is_none() {
        return backend.snapshot();
    }
    let result = poll_cloud(&backend).await;
    if let Err(error) = result {
        backend.cloud.failed(&error);
        backend.change(|data| {
            if !data
                .events
                .iter()
                .any(|event| event.level == "error" && event.message == error)
            {
                data.event("error", "connection", &error);
            }
            Ok(())
        })?;
    }
    backend.snapshot()
}

#[tauri::command]
async fn start_application(id: String, backend: State<'_, Backend>) -> Result<Snapshot, String> {
    let _operation = backend
        .operation
        .try_lock()
        .map_err(|_| "另一个操作正在进行。")?;
    let (mut session, observation) = cloud_session(&backend).await?;
    let recipe = backend
        .snapshot()?
        .apps
        .into_iter()
        .find(|a| a.id == id)
        .ok_or("应用配置不存在。")?;
    if recipe.kind == "service" && observation.balance.is_none_or(|v| v <= 0.0) {
        return Err("没有确认可用计算余额，暂不启动 Web 服务。".into());
    }
    let result = async {
        backend.cloud.start(&mut session, &recipe).await?;
        poll_cloud(&backend).await?;
        Ok(())
    }
    .await;
    cloud_result(&backend, result, "应用启动请求已提交，实例不会因此释放。")
}

#[tauri::command]
async fn stop_application(id: String, backend: State<'_, Backend>) -> Result<Snapshot, String> {
    let _operation = backend
        .operation
        .try_lock()
        .map_err(|_| "另一个操作正在进行。")?;
    let _ = cloud_session(&backend).await?;
    let recipe = backend
        .snapshot()?
        .apps
        .into_iter()
        .find(|a| a.id == id)
        .ok_or("应用配置不存在。")?;
    let result = async {
        backend.cloud.stop(&id, recipe.local_port).await?;
        poll_cloud(&backend).await
    }
    .await;
    cloud_result(&backend, result, "已停止应用；实例与计费保留。")
}

#[tauri::command]
async fn application_logs(
    id: String,
    backend: State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    if !backend.snapshot()?.apps.iter().any(|a| a.id == id) {
        return Err("应用配置不存在。".into());
    }
    backend.cloud.logs(&id).await
}

#[tauri::command]
fn open_drive_consent(app: tauri::AppHandle, backend: State<'_, Backend>) -> Result<(), String> {
    let url = backend.cloud.consent_url()?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "无法打开 Drive 授权页面。")?;
    #[cfg(feature = "native-smoke")]
    cloud::live_stage("drive_consent_open_requested", serde_json::json!({}));
    Ok(())
}

#[tauri::command]
fn open_application(
    id: String,
    app: tauri::AppHandle,
    backend: State<'_, Backend>,
) -> Result<(), String> {
    let snapshot = backend.snapshot()?;
    let recipe = snapshot
        .apps
        .iter()
        .find(|a| a.id == id)
        .ok_or("应用配置不存在。")?;
    let url = snapshot
        .runtime
        .application_urls
        .get(&id)
        .ok_or("应用入口尚未通过健康检查。")?;
    if Some(url.as_str())
        != recipe
            .local_port
            .map(|port| format!("http://127.0.0.1:{port}"))
            .as_deref()
    {
        return Err("应用入口与本机配置不一致。".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "无法打开本机应用入口。".into())
}

#[tauri::command]
async fn release_empty_instance(backend: State<'_, Backend>) -> Result<Snapshot, String> {
    let _operation = backend
        .operation
        .try_lock()
        .map_err(|_| "另一个操作正在进行。")?;
    let (mut session, _) = cloud_session(&backend).await?;
    let result = backend.cloud.release_empty(&mut session).await;
    match session.observe().await {
        Ok(value) => backend.change(|data| {
            data.observe(value);
            Ok(())
        })?,
        Err(error) => backend.query_error(&error)?,
    }
    cloud_result(
        &backend,
        result,
        "已提交空实例释放；计费结论以本次 Colab 查询为准。",
    )
}

#[tauri::command]
async fn synchronize_files(backend: State<'_, Backend>) -> Result<Snapshot, String> {
    let _operation = backend.operation.lock().await;
    let _ = cloud_session(&backend).await?;
    let result = async {
        backend.cloud.synchronize().await?;
        poll_cloud(&backend).await
    }
    .await;
    cloud_result(
        &backend,
        result,
        "正在停止应用并按清单保存文件；完成保存核验前不会释放实例。",
    )
}

#[tauri::command]
async fn shutdown_cloud(backend: State<'_, Backend>) -> Result<Snapshot, String> {
    let _operation = backend.operation.lock().await;
    let (mut session, _) = cloud_session(&backend).await?;
    let result = async {
        backend.cloud.synchronize().await?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1200);
        loop {
            poll_cloud(&backend).await?;
            let snapshot = backend.snapshot()?;
            match snapshot.runtime.synchronization.as_str() {
                "verified" => break,
                "failed" => return Err("保存或 Drive 服务端校验失败，实例保留并继续计费。".into()),
                _ if tokio::time::Instant::now() > deadline => {
                    return Err("保存等待超时，未释放实例；请重新查询保存状态。".into())
                }
                _ => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
            }
        }
        backend.change(|data| {
            data.observation.status = "cached".into();
            Ok(())
        })?;
        backend.cloud.release_saved(&mut session).await?;
        for attempt in 0..12 {
            match session.observe().await {
                Ok(value) => {
                    let off = value.instances.is_empty() && value.hourly_rate == Some(0.0);
                    backend.change(|data| {
                        data.observe(value);
                        Ok(())
                    })?;
                    if off {
                        return Ok(());
                    }
                }
                Err(error) => {
                    backend.query_error(&error)?;
                    return Err(error);
                }
            }
            if attempt < 11 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
        Err("实例释放已提交，但计费尚未确认；不能显示已停止计费。".into())
    }
    .await;
    cloud_result(
        &backend,
        result,
        "已按清单完成停止与保存核验，实例已释放并确认停止计费。",
    )
}

#[tauri::command]
async fn live_persistence_probe(
    action: String,
    backend: State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    if !live_check_mode() {
        return Err("真实联调未启用。".into());
    }
    #[cfg(feature = "native-smoke")]
    {
        let _operation = backend.operation.lock().await;
        backend.cloud.persistence_probe(&action).await
    }
    #[cfg(not(feature = "native-smoke"))]
    {
        let _ = (action, backend);
        Err("本构建不包含真实联调驱动。".into())
    }
}

#[tauri::command]
async fn live_manifest_probe(
    action: String,
    backend: State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    if !live_check_mode() {
        return Err("真实联调未启用。".into());
    }
    #[cfg(feature = "native-smoke")]
    {
        let _operation = backend.operation.lock().await;
        backend.cloud.manifest_probe(&action).await
    }
    #[cfg(not(feature = "native-smoke"))]
    {
        let _ = (action, backend);
        Err("本构建不包含真实联调驱动。".into())
    }
}

#[tauri::command]
async fn live_comfy_inspect(backend: State<'_, Backend>) -> Result<serde_json::Value, String> {
    if !live_check_mode() {
        return Err("真实联调未启用。".into());
    }
    #[cfg(feature = "native-smoke")]
    {
        backend.cloud.inspect_comfy().await
    }
    #[cfg(not(feature = "native-smoke"))]
    {
        let _ = backend;
        Err("本构建不包含真实联调驱动。".into())
    }
}

#[tauri::command]
async fn live_deployment_check(backend: State<'_, Backend>) -> Result<serde_json::Value, String> {
    if !live_check_mode() {
        return Err("真实联调未启用，需要本次任务的明确授权。".into());
    }
    #[cfg(feature = "native-smoke")]
    {
        let _operation = backend
            .operation
            .try_lock()
            .map_err(|_| "另一个操作正在进行。")?;
        let (mut session, observation) = cloud_session(&backend).await?;
        backend.change(|data| {
            data.observation.status = "cached".into();
            data.event(
                "info",
                "instance",
                "开始已授权的短任务、HTTP 服务和内置转发联调；不启动 ComfyUI 或写入 Drive。",
            );
            Ok(())
        })?;
        let result = backend
            .cloud
            .validate_live_deployment(&mut session, &observation)
            .await;
        let must_confirm_off = result
            .as_ref()
            .is_ok_and(|value| value["newTestInstanceReleased"] == true);
        let mut confirmed_off = false;
        for attempt in 0..12 {
            match session.observe().await {
                Ok(value) => {
                    confirmed_off = value.instances.is_empty() && value.hourly_rate == Some(0.0);
                    backend.change(|data| {
                        data.observe(value);
                        Ok(())
                    })?;
                }
                Err(error) => {
                    backend.query_error(&error)?;
                    break;
                }
            }
            if !must_confirm_off || confirmed_off {
                break;
            }
            if attempt < 11 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
        cloud::live_stage(
            "billing_checked",
            serde_json::json!({"confirmedOff":confirmed_off}),
        );
        match result {
            Ok(mut value) => {
                if must_confirm_off && !confirmed_off {
                    return Err("部署与释放已执行，但计费尚未确认，不能报告已停止计费。".into());
                }
                value["billingConfirmedOff"] = serde_json::Value::Bool(confirmed_off);
                backend.change(|data| { data.event("info", "connection", "真实部署联调通过：脚本校验、任务日志、HTTP 健康检查、内置转发和测试进程清理。"); Ok(()) })?;
                Ok(value)
            }
            Err(error) => {
                backend.cloud.failed(&error);
                backend.change(|data| {
                    data.event("error", "connection", &error);
                    Ok(())
                })?;
                Err(error)
            }
        }
    }
    #[cfg(not(feature = "native-smoke"))]
    {
        let _ = backend;
        Err("本构建不包含真实联调驱动。".into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let root = match std::env::var_os("CLOUD_DECK_DATA_DIR") {
                Some(path) => PathBuf::from(path),
                None => app.path().app_data_dir()?,
            };
            let path = root.join("workspace.json");
            // Window startup uses saved public metadata. Credentials are read lazily
            // by an explicit account/cloud operation and then cached in memory.
            let data = Stored::load(&path);
            app.manage(Backend {
                path,
                data: Mutex::new(data),
                operation: tokio::sync::Mutex::new(()),
                login_cancel: Mutex::new(None),
                login_url: Arc::new(Mutex::new(None)),
                cloud: cloud::Cloud::default(),
            });
            if !smoke_mode()
                && !live_check_mode()
                && std::env::var("CLOUD_DECK_MCP").as_deref() != Ok("0")
            {
                if let Err(error) = mcp::start(app.handle().clone(), &root) {
                    eprintln!("Cloud Deck MCP 未启动：{error}");
                }
            }
            Ok(())
        })
        .on_page_load(|_webview, _payload| {
            #[cfg(feature = "native-smoke")]
            if (smoke_mode() || live_check_mode())
                && _payload.event() == tauri::webview::PageLoadEvent::Finished
                && _webview.label() == "main"
                && native_smoke::start_once()
            {
                if let Some(window) = _webview.app_handle().get_webview_window("main") {
                    let _ = window.set_focus();
                }
                let _ = _webview.eval(
                    if live_check_mode()
                        && std::env::var("CLOUD_DECK_MANIFEST_CHECK").as_deref() == Ok("1")
                    {
                        include_str!("../../scripts/manifest-check.js")
                    } else if live_check_mode()
                        && std::env::var("CLOUD_DECK_SHUTDOWN_CHECK").as_deref() == Ok("1")
                    {
                        include_str!("../../scripts/shutdown-check.js")
                    } else if live_check_mode()
                        && std::env::var("CLOUD_DECK_PERSISTENCE_CHECK").as_deref() == Ok("1")
                    {
                        include_str!("../../scripts/persistence-check.js")
                    } else if live_check_mode()
                        && std::env::var("CLOUD_DECK_COMFY_CHECK").as_deref() == Ok("1")
                    {
                        include_str!("../../scripts/comfy-check.js")
                    } else if live_check_mode() {
                        include_str!("../../scripts/live-check.js")
                    } else {
                        include_str!("../../scripts/native-smoke.js")
                    },
                );
            }
        })
        .invoke_handler(tauri::generate_handler![
            native_check_report,
            live_deployment_check,
            live_comfy_inspect,
            live_manifest_probe,
            live_persistence_probe,
            get_workspace,
            start_instance,
            connect_instance,
            refresh_cloud,
            start_application,
            stop_application,
            application_logs,
            open_drive_consent,
            open_application,
            release_empty_instance,
            synchronize_files,
            shutdown_cloud,
            save_application,
            save_application_manifest,
            remove_application,
            sign_in,
            cancel_sign_in,
            reopen_sign_in,
            refresh_account,
            sign_out
        ])
        .run(tauri::generate_context!())
        .expect("Cloud Deck could not start");
}

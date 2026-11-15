use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    #[serde(default)]
    pub prepare_command: Option<String>,
    #[serde(default)]
    pub environment: std::collections::BTreeMap<String, String>,
    pub command: String,
    pub working_directory: String,
    pub remote_port: Option<u16>,
    pub local_port: Option<u16>,
    pub health_path: Option<String>,
    #[serde(default)]
    pub readiness: Option<Value>,
    pub save_directories: Vec<String>,
    pub storage_directory: String,
    pub auto_start: bool,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub manifest: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeDraft {
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub kind: String,
    #[serde(default)]
    pub prepare_command: Option<String>,
    #[serde(default)]
    pub environment: std::collections::BTreeMap<String, String>,
    pub command: String,
    pub working_directory: String,
    pub remote_port: Option<u16>,
    pub local_port: Option<u16>,
    pub health_path: Option<String>,
    #[serde(default)]
    pub readiness: Option<Value>,
    pub save_directories: Vec<String>,
    pub storage_directory: String,
    pub auto_start: bool,
}

pub fn builtin() -> Recipe {
    let manifest = serde_json::from_str(include_str!("../../applications/comfyui/manifest.json"))
        .expect("bundled application manifest must be valid JSON");
    crate::manifest::recipe(manifest, &[], true)
        .expect("bundled application manifest must be valid")
}

fn directory(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 512
        && !value.contains(['\0', '\n', '\r'])
        && !value.split('/').any(|part| part == "..")
}

pub fn validate(draft: RecipeDraft, current: &[Recipe], id: String) -> Result<Recipe, String> {
    let name = draft.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 || name.contains(['\0', '\n', '\r']) {
        return Err("应用名称应为 1–60 个字符。".into());
    }
    if id == "comfyui" || current.iter().any(|r| r.id == id && r.builtin) {
        return Err("内置应用保留项目固定配置，不能覆盖。".into());
    }
    if current
        .iter()
        .any(|r| r.id != id && r.name.to_lowercase() == name.to_lowercase())
    {
        return Err("已有同名应用，请换一个名称。".into());
    }
    if !matches!(draft.kind.as_str(), "service" | "task") {
        return Err("不支持的应用类型。".into());
    }
    if draft.command.trim().is_empty() || draft.command.len() > 4000 || draft.command.contains('\0')
    {
        return Err("请填写有效的启动命令（最多 4000 字节）。".into());
    }
    if draft
        .prepare_command
        .as_ref()
        .is_some_and(|s| s.len() > 4000 || s.contains('\0'))
        || draft.environment.len() > 64
        || draft.environment.iter().any(|(k, v)| {
            k.is_empty() || k.contains(['=', '\0']) || v.contains('\0') || v.len() > 8192
        })
    {
        return Err("准备命令或环境变量无效。".into());
    }
    if draft.description.len() > 1000
        || !directory(&draft.working_directory)
        || !directory(&draft.storage_directory)
        || draft.save_directories.len() > 32
        || draft.save_directories.iter().any(|p| !directory(p))
    {
        return Err("目录必须是云端 Linux 的绝对路径，不能包含上级目录跳转。".into());
    }
    let readiness = crate::readiness::resolve(
        &draft.kind,
        draft.readiness.as_ref(),
        draft.remote_port,
        draft.health_path.as_deref(),
    )?;
    if draft.kind == "service" {
        if draft.remote_port == Some(0)
            || draft.local_port == Some(0)
            || (draft.local_port.is_some() && draft.remote_port.is_none())
        {
            return Err("端口必须为 1–65535；本机转发需要远端端口。".into());
        }
        if current.iter().any(|r| {
            r.id != id
                && ((draft.remote_port.is_some() && r.remote_port == draft.remote_port)
                    || (draft.local_port.is_some() && r.local_port == draft.local_port))
        }) {
            return Err("端口已被其他应用配置使用，请更换。".into());
        }
    }
    let service = draft.kind == "service";
    Ok(Recipe {
        id,
        name,
        description: draft.description.trim().into(),
        kind: draft.kind,
        prepare_command: draft.prepare_command.filter(|s| !s.trim().is_empty()),
        environment: draft.environment,
        command: draft.command.trim().into(),
        working_directory: draft.working_directory,
        remote_port: if service { draft.remote_port } else { None },
        local_port: if service { draft.local_port } else { None },
        health_path: crate::readiness::http_path(readiness.as_ref(), draft.remote_port),
        readiness,
        save_directories: draft.save_directories,
        storage_directory: draft.storage_directory,
        auto_start: draft.auto_start,
        builtin: false,
        manifest: None,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub subject: String,
    pub email: String,
    pub name: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub id: String,
    pub accelerator: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub region_label: Option<String>,
}

fn region_code(value: &str) -> Option<&str> {
    let (prefix, suffix) = value.split_once('-')?;
    let number = suffix.trim_start_matches(|c: char| c.is_ascii_lowercase());
    (!prefix.is_empty()
        && prefix.bytes().all(|b| b.is_ascii_lowercase())
        && number.len() < suffix.len()
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit()))
    .then_some(value)
}

fn assigned_region(item: &Value) -> (Option<String>, Option<String>) {
    let from_proxy = item["runtimeProxyInfo"]["url"].as_str().and_then(|raw| {
        let url = url::Url::parse(raw).ok()?;
        let host = url.host_str()?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
            || !(host.ends_with(".prod.r.colab.dev") || host.ends_with(".googleusercontent.com"))
        {
            return None;
        }
        let zone = host.split_once(".prod.")?.0.rsplit('.').next()?;
        let code = match zone.rsplit_once('-') {
            Some((code, zone)) if zone.len() == 1 && zone.as_bytes()[0].is_ascii_lowercase() => {
                code
            }
            _ => zone,
        };
        region_code(code).map(str::to_owned)
    });
    let code = from_proxy.or_else(|| {
        item["region"]
            .as_str()
            .and_then(region_code)
            .map(str::to_owned)
    });
    let label = code.as_ref().map(|code| {
        let names: Value = serde_json::from_str(include_str!("../../regions.json"))
            .expect("bundled regions must be valid");
        names[code]
            .as_str()
            .map(|name| format!("{name} · {code}"))
            .unwrap_or_else(|| code.clone())
    });
    (code, label)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub status: String,
    pub observed_at: Option<u64>,
    pub balance: Option<f64>,
    pub hourly_rate: Option<f64>,
    pub eligible_gpus: Vec<String>,
    pub instances: Vec<Instance>,
    #[serde(default)]
    pub last_instance: Option<Instance>,
    pub error: Option<String>,
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            status: "unobserved".into(),
            observed_at: None,
            balance: None,
            hourly_rate: None,
            eligible_gpus: vec![],
            instances: vec![],
            last_instance: None,
            error: None,
        }
    }
}

pub fn parse_observation(usage: &Value, runtimes: &Value) -> Result<Observation, String> {
    let assignments = runtimes
        .get("assignments")
        .and_then(Value::as_array)
        .ok_or("Colab 没有返回有效的实例列表，状态待确认。")?;
    let balance = usage
        .get("currentBalance")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0);
    let rate = usage
        .get("consumptionRateHourly")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0);
    if balance.is_none() || rate.is_none() {
        return Err("Colab 没有返回有效的用量数据，余额与计费待确认。".into());
    }
    let mut instances = vec![];
    for item in assignments {
        let id = item
            .get("endpoint")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("实例记录不完整，状态待确认。")?;
        let (region, region_label) = assigned_region(item);
        instances.push(Instance {
            id: id.into(),
            accelerator: item
                .get("accelerator")
                .and_then(Value::as_str)
                .map(str::to_owned),
            region,
            region_label,
        });
    }
    Ok(Observation {
        status: "current".into(),
        observed_at: Some(now()),
        balance,
        hourly_rate: rate,
        eligible_gpus: usage
            .get("eligibleGpus")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        last_instance: instances.first().cloned(),
        instances,
        error: None,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub time: u64,
    pub level: String,
    pub category: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stored {
    pub schema: u32,
    pub custom_apps: Vec<Recipe>,
    pub account: Option<Account>,
    pub observation: Observation,
    pub events: Vec<Event>,
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            schema: 1,
            custom_apps: vec![],
            account: None,
            observation: Observation::default(),
            events: vec![],
        }
    }
}

impl Stored {
    pub fn observe(&mut self, mut observation: Observation) {
        let previous = self
            .observation
            .last_instance
            .as_ref()
            .or_else(|| self.observation.instances.first());
        for instance in &mut observation.instances {
            if instance.region.is_none() {
                if let Some(old) = previous.filter(|old| old.id == instance.id) {
                    instance.region = old.region.clone();
                    instance.region_label = old.region_label.clone();
                }
            }
        }
        observation.last_instance = observation
            .instances
            .first()
            .cloned()
            .or_else(|| previous.cloned());
        self.observation = observation;
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read(path).map_err(|_| "无法读取本地配置，原文件未覆盖。")?;
        let mut saved: Self =
            serde_json::from_slice(&raw).map_err(|_| "本地配置损坏，原文件未覆盖。")?;
        if saved.schema != 1
            || saved
                .custom_apps
                .iter()
                .any(|r| r.builtin || r.id == "comfyui")
        {
            return Err("本地配置版本或应用记录无效，原文件未覆盖。".into());
        }
        if let Some(account) = &mut saved.account {
            account.status = "saved".into();
        }
        if saved.observation.observed_at.is_some() {
            saved.observation.status = "cached".into();
        }
        Ok(saved)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or("无效的数据目录。")?;
        fs::create_dir_all(parent).map_err(|_| "无法创建数据目录。")?;
        let mut file =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法创建配置临时文件。")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| "无法保护配置文件权限。")?;
        }
        serde_json::to_writer(file.as_file_mut(), self).map_err(|_| "无法序列化应用配置。")?;
        file.flush().map_err(|_| "写入配置失败，原文件未覆盖。")?;
        file.as_file()
            .sync_all()
            .map_err(|_| "保存配置失败，原文件未覆盖。")?;
        file.persist(path)
            .map_err(|_| "替换配置失败，原文件保留。")?;
        Ok(())
    }

    pub fn apps(&self) -> Vec<Recipe> {
        let mut all = vec![builtin()];
        all.extend(self.custom_apps.clone());
        all
    }

    pub fn event(&mut self, level: &str, category: &str, message: impl Into<String>) {
        self.events.push(Event {
            time: now(),
            level: level.into(),
            category: category.into(),
            message: message.into(),
        });
        if self.events.len() > 500 {
            self.events.drain(..self.events.len() - 500);
        }
    }

    pub fn expected_login_subject(&self, secure: Option<&Account>) -> Option<String> {
        if let Some(account) = secure {
            let same_account = self
                .account
                .as_ref()
                .is_some_and(|stored| stored.subject == account.subject);
            if !same_account || !self.may_leave_account() {
                return Some(account.subject.clone());
            }
        }
        if self.may_leave_account() {
            None
        } else {
            self.account.as_ref().map(|a| a.subject.clone())
        }
    }

    pub fn may_leave_account(&self) -> bool {
        self.account.is_none()
            || (self.observation.status == "current"
                && self.observation.instances.is_empty()
                && self.observation.hourly_rate == Some(0.0))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub heartbeat: Value,
    pub connection: String,
    pub synchronization: String,
    pub storage: Value,
    pub applications: std::collections::BTreeMap<String, String>,
    pub application_urls: std::collections::BTreeMap<String, String>,
    pub endpoint: Option<String>,
    pub phase: String,
    pub message: String,
    pub needs_drive_consent: bool,
}
impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            heartbeat: serde_json::json!({"status":"inactive"}),
            connection: "not_connected".into(),
            synchronization: "unobserved".into(),
            storage: serde_json::json!({}),
            applications: Default::default(),
            application_urls: Default::default(),
            endpoint: None,
            phase: "idle".into(),
            message: "尚未建立云端连接。".into(),
            needs_drive_consent: false,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub runtime: RuntimeStatus,
    pub apps: Vec<Recipe>,
    pub account: Option<Account>,
    pub observation: Observation,
    pub events: Vec<Event>,
    pub data_directory: String,
    pub mode: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_or_failed_queries_never_mean_zero_billing() {
        assert!(parse_observation(&json!({}), &json!({"assignments": []})).is_err());
        assert!(parse_observation(
            &json!({"currentBalance": 10, "consumptionRateHourly": 0}),
            &json!({})
        )
        .is_err());
        let mut stored = Stored::default();
        stored.account = Some(Account {
            subject: "test".into(),
            email: "test@example.com".into(),
            name: "Test".into(),
            status: "saved".into(),
        });
        assert!(!stored.may_leave_account());
        stored.observation = parse_observation(
            &json!({"currentBalance": 10, "consumptionRateHourly": 0}),
            &json!({"assignments": []}),
        )
        .unwrap();
        assert!(stored.may_leave_account());
        stored.observation.status = "error".into();
        assert!(!stored.may_leave_account());
    }

    #[test]
    fn secure_credentials_cannot_be_overwritten_when_local_metadata_is_missing() {
        let secure = Account {
            subject: "original".into(),
            email: "original@example.com".into(),
            name: "Original".into(),
            status: "saved".into(),
        };
        let mut stored = Stored::default();
        assert_eq!(
            stored.expected_login_subject(Some(&secure)),
            Some("original".into())
        );
        stored.account = Some(secure.clone());
        stored.observation = parse_observation(
            &json!({"currentBalance": 10, "consumptionRateHourly": 0}),
            &json!({"assignments": []}),
        )
        .unwrap();
        assert_eq!(stored.expected_login_subject(Some(&secure)), None);
        stored.account.as_mut().unwrap().subject = "other".into();
        assert_eq!(
            stored.expected_login_subject(Some(&secure)),
            Some("original".into())
        );
    }

    #[test]
    fn allocated_regions_follow_the_returned_google_proxy_without_guessing() {
        for (host, code, label) in [
            (
                "runtime.asia-southeast1-a.prod.r.colab.dev",
                "asia-southeast1",
                "新加坡 · asia-southeast1",
            ),
            (
                "runtime.us-central1-c.prod.r.colab.dev",
                "us-central1",
                "美国爱荷华 · us-central1",
            ),
            (
                "runtime.asia-east1.prod.colab.googleusercontent.com",
                "asia-east1",
                "台湾 · asia-east1",
            ),
            (
                "runtime.africa-south99-a.prod.r.colab.dev",
                "africa-south99",
                "africa-south99",
            ),
        ] {
            let item = json!({"runtimeProxyInfo":{"url":format!("https://{host}/")}});
            assert_eq!(
                assigned_region(&item),
                (Some(code.into()), Some(label.into()))
            );
        }
        for raw in [
            "https://runtime.asia-southeast1-a.prod.r.colab.dev.evil.example",
            "https://runtime.not-a-region.prod.r.colab.dev",
            "http://runtime.asia-southeast1-a.prod.r.colab.dev",
            "https://user:secret@runtime.asia-southeast1-a.prod.r.colab.dev",
            "invalid",
        ] {
            assert_eq!(
                assigned_region(&json!({"runtimeProxyInfo":{"url":raw}})),
                (None, None)
            );
        }
        assert_eq!(
            assigned_region(&json!({"region":"europe-west3"}))
                .1
                .as_deref(),
            Some("德国法兰克福 · europe-west3")
        );
        assert_eq!(
            assigned_region(&json!({"endpoint":"gpu-t4-s-ass1c1-random"})),
            (None, None)
        );
    }

    #[test]
    fn region_history_survives_refresh_release_and_reopen() {
        let mut stored = Stored::default();
        let usage = json!({"currentBalance":10,"consumptionRateHourly":1.15});
        stored.observe(parse_observation(&usage, &json!({"assignments":[{"endpoint":"real-t4","accelerator":"T4","runtimeProxyInfo":{"url":"https://runtime.asia-southeast1-a.prod.r.colab.dev/"}}]})).unwrap());
        assert_eq!(
            stored.observation.instances[0].region.as_deref(),
            Some("asia-southeast1")
        );
        stored.observe(
            parse_observation(
                &usage,
                &json!({"assignments":[{"endpoint":"real-t4","accelerator":"T4"}]}),
            )
            .unwrap(),
        );
        assert_eq!(
            stored.observation.instances[0].region.as_deref(),
            Some("asia-southeast1")
        );
        stored.observe(
            parse_observation(
                &json!({"currentBalance":10,"consumptionRateHourly":0}),
                &json!({"assignments":[]}),
            )
            .unwrap(),
        );
        assert!(stored.observation.instances.is_empty());
        assert_eq!(
            stored
                .observation
                .last_instance
                .as_ref()
                .unwrap()
                .region_label
                .as_deref(),
            Some("新加坡 · asia-southeast1")
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        stored.save(&path).unwrap();
        let reopened = Stored::load(&path).unwrap();
        assert_eq!(
            reopened
                .observation
                .last_instance
                .unwrap()
                .region
                .as_deref(),
            Some("asia-southeast1")
        );
        let mut old = serde_json::to_value(&stored).unwrap();
        old["observation"]
            .as_object_mut()
            .unwrap()
            .remove("lastInstance");
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(Stored::load(&path)
            .unwrap()
            .observation
            .last_instance
            .is_none());
    }

    #[test]
    fn state_persists_events_and_reopens_cloud_observations_as_cached() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let mut saved = Stored::default();
        saved.event("error", "connection", "断线");
        saved.observation.observed_at = Some(now());
        saved.observation.status = "current".into();
        saved.save(&path).unwrap();
        let loaded = Stored::load(&path).unwrap();
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.observation.status, "cached");
        fs::write(&path, b"broken").unwrap();
        assert!(Stored::load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
    }
}

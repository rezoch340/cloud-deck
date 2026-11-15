//! 客户端版本检查与自更新模块：查询 GitHub Releases、比对语义化版本，并在 Windows NSIS 安装环境下完成 SHA-256 校验后静默覆盖更新。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use subtle::ConstantTimeEq;
use url::Url;

use crate::model::now;

/// 默认 GitHub Releases 列表接口（使用列表接口避免仓库尚无 Release 时返回 404）。
pub const RELEASES_API_URL: &str =
    "https://api.github.com/repos/rezoch340/cloud-deck/releases?per_page=10";
/// 默认 GitHub Releases 页面地址。
pub const RELEASES_PAGE_URL: &str = "https://github.com/rezoch340/cloud-deck/releases";

/// 校验和文件最大允许体积（1 MiB）。
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;
/// 安装包最大允许体积（512 MiB）。
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
/// 界面展示的发行说明最大字符数。
const MAX_NOTES_CHARS: usize = 4000;

/// 客户端安装形态：区分可自动覆盖更新的 Windows NSIS 安装版与需要前往下载页的便携版 / macOS 包。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallMode {
    WindowsNsis,
    WindowsPortable,
    MacosBundle,
}

impl InstallMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowsNsis => "windows_nsis",
            Self::WindowsPortable => "windows_portable",
            Self::MacosBundle => "macos_bundle",
        }
    }
}

/// 返回给前端的客户端更新状态快照。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub status: String,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_name: Option<String>,
    pub release_notes: Option<String>,
    pub release_url: String,
    pub published_at: Option<String>,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
    pub asset_size: Option<u64>,
    pub checksum_url: Option<String>,
    pub can_self_update: bool,
    pub install_mode: String,
    pub checked_at: Option<u64>,
    pub error: Option<String>,
}

impl UpdateStatus {
    pub fn initial() -> Self {
        let mode = detect_install_mode();
        Self::initial_for(env!("CARGO_PKG_VERSION"), mode)
    }

    pub fn initial_for(current_version: &str, mode: InstallMode) -> Self {
        Self {
            status: "idle".into(),
            current_version: current_version.into(),
            latest_version: None,
            release_name: None,
            release_notes: None,
            release_url: RELEASES_PAGE_URL.into(),
            published_at: None,
            download_url: None,
            asset_name: None,
            asset_size: None,
            checksum_url: None,
            can_self_update: false,
            install_mode: mode.as_str().into(),
            checked_at: None,
            error: None,
        }
    }
}

/// 根据当前进程路径识别安装形态：Tauri NSIS 默认将 `uninstall.exe` 写入主程序同级目录。
pub fn detect_install_mode() -> InstallMode {
    let current_exe = std::env::current_exe().ok();
    detect_install_mode_from_exe(std::env::consts::OS, current_exe.as_deref())
}

pub fn detect_install_mode_from_exe(target_os: &str, exe_path: Option<&Path>) -> InstallMode {
    if target_os == "windows" {
        let has_uninstaller = exe_path
            .and_then(Path::parent)
            .is_some_and(|directory| directory.join("uninstall.exe").is_file());
        if has_uninstaller {
            InstallMode::WindowsNsis
        } else {
            InstallMode::WindowsPortable
        }
    } else {
        InstallMode::MacosBundle
    }
}

/// 语义化版本号解析结构，支持比较 `major.minor.patch` 与可选预发布标识。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Vec<String>,
}

impl SemVer {
    pub fn parse(raw_tag: &str) -> Option<Self> {
        let trimmed = raw_tag.trim();
        let without_ref = trimmed.strip_prefix("refs/tags/").unwrap_or(trimmed);
        let normalized = without_ref
            .strip_prefix('v')
            .or_else(|| without_ref.strip_prefix('V'))
            .unwrap_or(without_ref);
        let without_build = normalized
            .split_once('+')
            .map(|(head, _)| head)
            .unwrap_or(normalized);
        let (core, prerelease_part) = match without_build.split_once('-') {
            Some((head, tail)) => {
                if tail.is_empty() {
                    return None;
                }
                (head, Some(tail))
            }
            None => (without_build, None),
        };
        let mut segments = core.split('.');
        let major = segments.next()?.parse::<u64>().ok()?;
        let minor = segments.next()?.parse::<u64>().ok()?;
        let patch = segments.next()?.parse::<u64>().ok()?;
        if segments.next().is_some() {
            return None;
        }
        let prerelease = match prerelease_part {
            Some(raw_pre) => {
                let parts: Vec<String> = raw_pre.split('.').map(str::to_owned).collect();
                if parts.iter().any(|part| {
                    part.is_empty()
                        || !part
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric() || character == '-')
                }) {
                    return None;
                }
                parts
            }
            None => Vec::new(),
        };
        Some(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }

    pub fn canonical_string(&self) -> String {
        if self.prerelease.is_empty() {
            format!("{}.{}.{}", self.major, self.minor, self.patch)
        } else {
            format!(
                "{}.{}.{}-{}",
                self.major,
                self.minor,
                self.patch,
                self.prerelease.join(".")
            )
        }
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)) {
            Ordering::Equal => match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
                (true, true) => Ordering::Equal,
                // 正式版高于同主次补丁号的预发布版。
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (left, right) in self.prerelease.iter().zip(other.prerelease.iter()) {
                        let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
                            (Ok(left_num), Ok(right_num)) => left_num.cmp(&right_num),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => left.cmp(right),
                        };
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    self.prerelease.len().cmp(&other.prerelease.len())
                }
            },
            non_equal => non_equal,
        }
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 校验更新元数据与安装包下载地址必须属于受信任的 GitHub HTTPS 域名。
pub fn validate_github_url(raw_url: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw_url).map_err(|_| "更新地址格式无效。".to_string())?;
    validate_parsed_github_url(&parsed)?;
    Ok(parsed)
}

fn validate_parsed_github_url(parsed: &Url) -> Result<(), String> {
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("更新地址必须使用受保护的 HTTPS 连接。".into());
    }
    let host = parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "更新地址缺少主机名。".to_string())?;
    let allowed = host == "github.com"
        || host == "api.github.com"
        || host.ends_with(".github.com")
        || host.ends_with(".githubusercontent.com");
    if !allowed {
        return Err(format!("拒绝访问非 GitHub 官方更新域名：{host}"));
    }
    Ok(())
}

fn build_http_client(
    timeout_seconds: u64,
    allow_loopback_test: bool,
) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
    );
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 8 {
            return attempt.error("更新下载重定向次数过多。");
        }
        let target = attempt.url();
        if allow_loopback_test
            && target.scheme() == "http"
            && matches!(target.host_str(), Some("127.0.0.1" | "localhost"))
        {
            return attempt.follow();
        }
        match validate_parsed_github_url(target) {
            Ok(()) => attempt.follow(),
            Err(reason) => attempt.error(reason),
        }
    });
    reqwest::Client::builder()
        .user_agent(concat!("CloudDeck/", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(policy)
        .build()
        .map_err(|_| "无法初始化版本检查网络客户端。".into())
}

fn truncate_notes(raw_body: &str) -> Option<String> {
    let trimmed = raw_body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= MAX_NOTES_CHARS {
        return Some(trimmed.to_owned());
    }
    let shortened: String = trimmed.chars().take(MAX_NOTES_CHARS).collect();
    Some(format!("{shortened}\n…"))
}

#[derive(Clone, Debug)]
struct SelectedAsset {
    name: String,
    url: String,
    size: Option<u64>,
}

fn select_platform_asset(assets: &[Value], mode: InstallMode) -> Option<SelectedAsset> {
    let valid_assets: Vec<SelectedAsset> = assets
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.trim();
            let raw_url = entry.get("browser_download_url")?.as_str()?.trim();
            if name.is_empty() || validate_github_url(raw_url).is_err() {
                return None;
            }
            let size = entry.get("size").and_then(Value::as_u64);
            Some(SelectedAsset {
                name: name.to_owned(),
                url: raw_url.to_owned(),
                size,
            })
        })
        .collect();

    let matchers: &[fn(&str) -> bool] = match mode {
        InstallMode::WindowsNsis => &[
            |lower| lower.ends_with("-windows-x64-setup.exe"),
            |lower| lower.ends_with("_x64-setup.exe") || lower.ends_with("-setup.exe"),
        ],
        InstallMode::WindowsPortable => &[
            |lower| lower.ends_with("-windows-x64-portable.zip"),
            |lower| lower.ends_with("-windows-x64-setup.exe") || lower.ends_with("_x64-setup.exe"),
        ],
        InstallMode::MacosBundle => &[
            |lower| lower.ends_with("-macos-universal.dmg") || lower.ends_with(".dmg"),
            |lower| lower.ends_with("-macos-universal.app.zip") || lower.ends_with(".app.zip"),
        ],
    };

    for matcher in matchers {
        if let Some(found) = valid_assets
            .iter()
            .find(|candidate| matcher(&candidate.name.to_ascii_lowercase()))
        {
            return Some(found.clone());
        }
    }
    None
}

fn select_checksum_url(assets: &[Value]) -> Option<String> {
    assets.iter().find_map(|entry| {
        let name = entry.get("name")?.as_str()?.trim();
        if !name.eq_ignore_ascii_case("SHA256SUMS.txt") {
            return None;
        }
        let raw_url = entry.get("browser_download_url")?.as_str()?.trim();
        validate_github_url(raw_url).ok()?;
        Some(raw_url.to_owned())
    })
}

/// 解析 GitHub Releases 列表响应，返回最新稳定版对比结论。
pub fn evaluate_releases(
    payload: &Value,
    current_version: &str,
    mode: InstallMode,
) -> Result<UpdateStatus, String> {
    let current_semver = SemVer::parse(current_version)
        .ok_or_else(|| format!("当前客户端版本号无效：{current_version}"))?;
    let items = payload
        .as_array()
        .ok_or_else(|| "GitHub Releases 响应格式无效。".to_string())?;

    let checked_time = now();
    let mut best_release: Option<(SemVer, &Value)> = None;
    for entry in items {
        if entry.get("draft").and_then(Value::as_bool) == Some(true)
            || entry.get("prerelease").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(tag_name) = entry.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let Some(parsed_version) = SemVer::parse(tag_name) else {
            continue;
        };
        let replace = match &best_release {
            Some((existing, _)) => &parsed_version > existing,
            None => true,
        };
        if replace {
            best_release = Some((parsed_version, entry));
        }
    }

    let Some((latest_semver, release)) = best_release else {
        let mut empty_state = UpdateStatus::initial_for(current_version, mode);
        empty_state.status = "no_release".into();
        empty_state.checked_at = Some(checked_time);
        return Ok(empty_state);
    };

    let latest_version = latest_semver.canonical_string();
    let release_url = release
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|raw| validate_github_url(raw).is_ok())
        .unwrap_or(RELEASES_PAGE_URL)
        .to_owned();
    let release_name = release
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);
    let release_notes = release
        .get("body")
        .and_then(Value::as_str)
        .and_then(truncate_notes);
    let published_at = release
        .get("published_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|timestamp| !timestamp.is_empty())
        .map(str::to_owned);
    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let selected_asset = select_platform_asset(assets, mode);
    let checksum_url = select_checksum_url(assets);
    let has_newer = latest_semver > current_semver;
    let can_self_update = has_newer
        && mode == InstallMode::WindowsNsis
        && selected_asset.is_some()
        && checksum_url.is_some();

    Ok(UpdateStatus {
        status: if has_newer {
            "available".into()
        } else {
            "up_to_date".into()
        },
        current_version: current_version.into(),
        latest_version: Some(latest_version),
        release_name,
        release_notes,
        release_url,
        published_at,
        download_url: selected_asset.as_ref().map(|asset| asset.url.clone()),
        asset_name: selected_asset.as_ref().map(|asset| asset.name.clone()),
        asset_size: selected_asset.and_then(|asset| asset.size),
        checksum_url,
        can_self_update,
        install_mode: mode.as_str().into(),
        checked_at: Some(checked_time),
        error: None,
    })
}

/// 从 `SHA256SUMS.txt` 文本中提取指定安装包文件名的 64 位十六进制 SHA-256。
pub fn parse_checksum_for_asset(
    checksum_text: &str,
    expected_asset: &str,
) -> Result<String, String> {
    if expected_asset.is_empty()
        || expected_asset.contains(['/', '\\', '\0', '\n', '\r'])
        || expected_asset.contains("..")
    {
        return Err("安装包文件名不安全，拒绝校验。".into());
    }
    for raw_line in checksum_text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(digest_hex) = parts.next() else {
            continue;
        };
        let remainder = trimmed[digest_hex.len()..].trim_start();
        let file_label = remainder.strip_prefix('*').unwrap_or(remainder).trim();
        let clean_name = file_label.strip_prefix("./").unwrap_or(file_label);
        if clean_name == expected_asset {
            if digest_hex.len() != 64
                || !digest_hex
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            {
                return Err(format!(
                    "SHA256SUMS.txt 中 {expected_asset} 的摘要格式无效。"
                ));
            }
            return Ok(digest_hex.to_ascii_lowercase());
        }
    }
    Err(format!(
        "SHA256SUMS.txt 中未找到安装包 {expected_asset} 的校验记录。"
    ))
}

/// 向 GitHub 查询最新发布版本并生成 `UpdateStatus`。
pub async fn check_latest() -> Result<UpdateStatus, String> {
    let mode = detect_install_mode();
    check_latest_from(RELEASES_API_URL, env!("CARGO_PKG_VERSION"), mode, false).await
}

async fn check_latest_from(
    api_url: &str,
    current_version: &str,
    mode: InstallMode,
    allow_loopback_test: bool,
) -> Result<UpdateStatus, String> {
    if !allow_loopback_test {
        validate_github_url(api_url)?;
    }
    let client = build_http_client(25, allow_loopback_test)?;
    let response = client
        .get(api_url)
        .send()
        .await
        .map_err(|error| format!("无法连接 GitHub 检查更新：{error}"))?;
    let status_code = response.status();
    if !status_code.is_success() {
        return Err(format!(
            "GitHub Releases 查询返回 HTTP {}，请稍后再试。",
            status_code.as_u16()
        ));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|_| "无法解析 GitHub Releases 响应内容。".to_string())?;
    evaluate_releases(&payload, current_version, mode)
}

async fn download_limited_bytes(
    client: &reqwest::Client,
    target_url: &str,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(target_url)
        .send()
        .await
        .map_err(|error| format!("下载{label}失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "下载{label}返回 HTTP {}。",
            response.status().as_u16()
        ));
    }
    if let Some(content_length) = response.content_length() {
        if content_length > max_bytes {
            return Err(format!("{label}体积超出安全上限。"));
        }
    }
    let mut buffer = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取{label}数据流失败：{error}"))?
    {
        if (buffer.len() as u64) + (chunk.len() as u64) > max_bytes {
            return Err(format!("{label}体积超出安全上限。"));
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok(buffer)
}

/// 下载 `SHA256SUMS.txt` 与安装包到数据目录下的 `updates/<version>/`，通过常量时间摘要比对后落盘返回。
pub async fn download_verified_installer(
    status: &UpdateStatus,
    data_root: &Path,
) -> Result<PathBuf, String> {
    if !status.can_self_update || status.install_mode != InstallMode::WindowsNsis.as_str() {
        return Err("当前客户端环境不支持自动覆盖更新，请前往 GitHub Release 下载安装包。".into());
    }
    let latest_version = status
        .latest_version
        .as_deref()
        .ok_or("缺少目标更新版本号。")?;
    let _ = SemVer::parse(latest_version).ok_or("目标更新版本号无效。")?;
    let asset_name = status
        .asset_name
        .as_deref()
        .ok_or("缺少更新安装包文件名。")?;
    let download_url = status
        .download_url
        .as_deref()
        .ok_or("缺少更新安装包下载地址。")?;
    let checksum_url = status
        .checksum_url
        .as_deref()
        .ok_or("缺少 SHA256SUMS.txt 校验文件地址。")?;

    download_verified_installer_from(
        download_url,
        checksum_url,
        asset_name,
        latest_version,
        data_root,
        false,
    )
    .await
}

async fn download_verified_installer_from(
    download_url: &str,
    checksum_url: &str,
    asset_name: &str,
    latest_version: &str,
    data_root: &Path,
    allow_loopback_test: bool,
) -> Result<PathBuf, String> {
    if !allow_loopback_test {
        validate_github_url(download_url)?;
        validate_github_url(checksum_url)?;
    }
    if asset_name.is_empty()
        || asset_name.contains(['/', '\\', '\0', '\n', '\r'])
        || asset_name.contains("..")
    {
        return Err("安装包文件名无效。".into());
    }
    let client = build_http_client(600, allow_loopback_test)?;
    let checksum_bytes =
        download_limited_bytes(&client, checksum_url, MAX_CHECKSUM_BYTES, "校验文件").await?;
    let checksum_text =
        String::from_utf8(checksum_bytes).map_err(|_| "校验文件编码无效。".to_string())?;
    let expected_sha256 = parse_checksum_for_asset(&checksum_text, asset_name)?;

    let version_dir = data_root.join("updates").join(latest_version);
    fs::create_dir_all(&version_dir).map_err(|_| "无法创建本地更新缓存目录。".to_string())?;
    let part_path = version_dir.join(format!("{asset_name}.part"));
    let final_path = version_dir.join(asset_name);

    let mut response = client
        .get(download_url)
        .send()
        .await
        .map_err(|error| format!("下载更新安装包失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "下载更新安装包返回 HTTP {}。",
            response.status().as_u16()
        ));
    }
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_INSTALLER_BYTES {
            return Err("更新安装包体积超出安全上限。".into());
        }
    }

    let write_result: Result<String, String> = async {
        let mut output_file =
            fs::File::create(&part_path).map_err(|_| "无法写入临时更新安装包。".to_string())?;
        let mut hasher = Sha256::new();
        let mut received_bytes: u64 = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("下载安装包中断：{error}"))?
        {
            received_bytes += chunk.len() as u64;
            if received_bytes > MAX_INSTALLER_BYTES {
                return Err("更新安装包体积超出安全上限。".into());
            }
            hasher.update(&chunk);
            output_file
                .write_all(&chunk)
                .map_err(|_| "写入更新安装包失败，请检查磁盘剩余空间。".to_string())?;
        }
        if received_bytes == 0 {
            return Err("下载到的更新安装包为空。".into());
        }
        output_file
            .flush()
            .map_err(|_| "刷新更新安装包缓存失败。".to_string())?;
        output_file
            .sync_all()
            .map_err(|_| "同步更新安装包到磁盘失败。".to_string())?;
        let digest = hasher.finalize();
        let mut hex_output = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut hex_output, "{byte:02x}");
        }
        Ok(hex_output)
    }
    .await;

    let actual_sha256 = match write_result {
        Ok(hash_value) => hash_value,
        Err(failure) => {
            let _ = fs::remove_file(&part_path);
            return Err(failure);
        }
    };

    if actual_sha256
        .as_bytes()
        .ct_eq(expected_sha256.as_bytes())
        .unwrap_u8()
        != 1
    {
        let _ = fs::remove_file(&part_path);
        return Err("更新安装包 SHA-256 校验未通过，已拒绝安装。".into());
    }

    fs::rename(&part_path, &final_path).map_err(|_| {
        let _ = fs::remove_file(&part_path);
        "无法完成更新安装包落盘。".to_string()
    })?;
    Ok(final_path)
}

/// 在 Windows 上以 Tauri NSIS 的被动覆盖升级模式启动安装包（安装完成后自动重启客户端）。
pub fn launch_windows_installer(installer_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new(installer_path)
            .args(["/P", "/UPDATE", "/R"])
            .spawn()
            .map_err(|error| format!("无法启动 Windows 更新安装程序：{error}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = installer_path;
        Err("当前系统不支持直接运行 NSIS 更新程序，请前往 GitHub Release 下载更新。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::thread;

    #[test]
    fn semver_parses_tags_and_orders_releases_accurately() {
        let base = SemVer::parse("0.1.0").unwrap();
        let prefixed = SemVer::parse("v0.1.0").unwrap();
        let full_ref = SemVer::parse("refs/tags/v0.1.0+20261115").unwrap();
        assert_eq!(base, prefixed);
        assert_eq!(base, full_ref);

        let patch_next = SemVer::parse("v0.1.1").unwrap();
        let minor_next = SemVer::parse("v0.2.0").unwrap();
        let prerelease = SemVer::parse("v0.2.0-rc.1").unwrap();
        let prerelease_next = SemVer::parse("v0.2.0-rc.2").unwrap();
        assert!(patch_next > base);
        assert!(prerelease > patch_next);
        assert!(prerelease_next > prerelease);
        assert!(minor_next > prerelease_next);

        for invalid in ["", "v1.2", "1.2.3.4", "v1.2.x", "v1.2.3-", "v1.2.3-bad$tag"] {
            assert!(SemVer::parse(invalid).is_none(), "should reject {invalid}");
        }
    }

    #[test]
    fn empty_draft_and_prerelease_lists_do_not_fail_or_trigger_updates() {
        let empty = evaluate_releases(&json!([]), "0.1.0", InstallMode::MacosBundle).unwrap();
        assert_eq!(empty.status, "no_release");
        assert!(!empty.can_self_update);

        let drafts_only = json!([
            {"tag_name": "v0.2.0", "draft": true, "prerelease": false},
            {"tag_name": "v0.3.0-beta.1", "draft": false, "prerelease": true}
        ]);
        let skipped = evaluate_releases(&drafts_only, "0.1.0", InstallMode::WindowsNsis).unwrap();
        assert_eq!(skipped.status, "no_release");
    }

    #[test]
    fn selects_highest_stable_release_and_matches_install_mode_assets() {
        let releases = json!([
            {
                "tag_name": "v0.1.1",
                "name": "v0.1.1 Bugfix",
                "draft": false,
                "prerelease": false,
                "html_url": "https://github.com/rezoch340/cloud-deck/releases/tag/v0.1.1",
                "assets": []
            },
            {
                "tag_name": "v0.2.0",
                "name": "Cloud Deck 0.2.0",
                "body": "新增标签自动发版与客户端更新检查",
                "draft": false,
                "prerelease": false,
                "published_at": "2026-11-15T16:00:00Z",
                "html_url": "https://github.com/rezoch340/cloud-deck/releases/tag/v0.2.0",
                "assets": [
                    {
                        "name": "CloudDeck-0.2.0-windows-x64-setup.exe",
                        "size": 4456448,
                        "browser_download_url": "https://github.com/rezoch340/cloud-deck/releases/download/v0.2.0/CloudDeck-0.2.0-windows-x64-setup.exe"
                    },
                    {
                        "name": "CloudDeck-0.2.0-macos-universal.dmg",
                        "size": 9123456,
                        "browser_download_url": "https://github.com/rezoch340/cloud-deck/releases/download/v0.2.0/CloudDeck-0.2.0-macos-universal.dmg"
                    },
                    {
                        "name": "SHA256SUMS.txt",
                        "size": 320,
                        "browser_download_url": "https://github.com/rezoch340/cloud-deck/releases/download/v0.2.0/SHA256SUMS.txt"
                    }
                ]
            }
        ]);

        let nsis = evaluate_releases(&releases, "0.1.0", InstallMode::WindowsNsis).unwrap();
        assert_eq!(nsis.status, "available");
        assert_eq!(nsis.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(
            nsis.asset_name.as_deref(),
            Some("CloudDeck-0.2.0-windows-x64-setup.exe")
        );
        assert!(nsis.can_self_update);

        let portable = evaluate_releases(&releases, "0.1.0", InstallMode::WindowsPortable).unwrap();
        assert_eq!(portable.status, "available");
        assert!(!portable.can_self_update);

        let macos = evaluate_releases(&releases, "0.1.0", InstallMode::MacosBundle).unwrap();
        assert_eq!(macos.status, "available");
        assert_eq!(
            macos.asset_name.as_deref(),
            Some("CloudDeck-0.2.0-macos-universal.dmg")
        );
        assert!(!macos.can_self_update);

        let current = evaluate_releases(&releases, "0.2.0", InstallMode::WindowsNsis).unwrap();
        assert_eq!(current.status, "up_to_date");
        assert!(!current.can_self_update);
    }

    #[test]
    fn install_mode_distinguishes_nsis_uninstaller_from_portable_binary() {
        let temp = tempfile::tempdir().unwrap();
        let portable_exe = temp.path().join("cloud-deck.exe");
        fs::write(&portable_exe, b"binary").unwrap();
        assert_eq!(
            detect_install_mode_from_exe("windows", Some(&portable_exe)),
            InstallMode::WindowsPortable
        );
        fs::write(temp.path().join("uninstall.exe"), b"uninstaller").unwrap();
        assert_eq!(
            detect_install_mode_from_exe("windows", Some(&portable_exe)),
            InstallMode::WindowsNsis
        );
        assert_eq!(
            detect_install_mode_from_exe("macos", Some(&portable_exe)),
            InstallMode::MacosBundle
        );
    }

    #[test]
    fn github_url_allowlist_blocks_untrusted_hosts_and_plain_http() {
        for allowed in [
            "https://api.github.com/repos/rezoch340/cloud-deck/releases",
            "https://github.com/rezoch340/cloud-deck/releases/download/v0.1.1/SHA256SUMS.txt",
            "https://objects.githubusercontent.com/github-production-release-asset/file.exe",
        ] {
            assert!(
                validate_github_url(allowed).is_ok(),
                "should allow {allowed}"
            );
        }
        for blocked in [
            "http://github.com/rezoch340/cloud-deck/releases",
            "https://github.com.evil.example/file.exe",
            "https://token@github.com/rezoch340/cloud-deck",
            "file:///tmp/installer.exe",
        ] {
            assert!(
                validate_github_url(blocked).is_err(),
                "should block {blocked}"
            );
        }
    }

    #[test]
    fn checksum_parser_matches_exact_filename_and_validates_hex() {
        let sample = "\
# Cloud Deck release checksums
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  CloudDeck-0.2.0-macos-universal.dmg
ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789 *CloudDeck-0.2.0-windows-x64-setup.exe
";
        let parsed =
            parse_checksum_for_asset(sample, "CloudDeck-0.2.0-windows-x64-setup.exe").unwrap();
        assert_eq!(
            parsed,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert!(parse_checksum_for_asset(sample, "missing.exe").is_err());
        assert!(parse_checksum_for_asset(sample, "../evil.exe").is_err());
    }

    #[tokio::test]
    async fn verified_download_accepts_matching_sha256_and_rejects_tampered_payload() {
        let payload = b"verified-nsis-installer-payload".to_vec();
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let digest = hasher.finalize();
        let mut expected_hex = String::new();
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut expected_hex, "{byte:02x}");
        }
        let checksum_doc = format!("{expected_hex}  CloudDeck-0.2.0-windows-x64-setup.exe\n");

        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let handle = thread::spawn(move || {
            for index in 0..4 {
                let Ok(request) = server.recv() else {
                    break;
                };
                let url_path = request.url().to_owned();
                if url_path.ends_with("SHA256SUMS.txt") {
                    let response = tiny_http::Response::from_string(checksum_doc.clone());
                    let _ = request.respond(response);
                } else if index < 2 {
                    let response = tiny_http::Response::from_data(payload.clone());
                    let _ = request.respond(response);
                } else {
                    let response = tiny_http::Response::from_data(b"tampered-installer".to_vec());
                    let _ = request.respond(response);
                }
            }
        });

        let temp = tempfile::tempdir().unwrap();
        let base_url = format!("http://{address}");
        let saved_path = download_verified_installer_from(
            &format!("{base_url}/CloudDeck-0.2.0-windows-x64-setup.exe"),
            &format!("{base_url}/SHA256SUMS.txt"),
            "CloudDeck-0.2.0-windows-x64-setup.exe",
            "0.2.0",
            temp.path(),
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read(&saved_path).unwrap(),
            b"verified-nsis-installer-payload"
        );

        let tampered_dir = tempfile::tempdir().unwrap();
        let tampered = download_verified_installer_from(
            &format!("{base_url}/CloudDeck-0.2.0-windows-x64-setup.exe"),
            &format!("{base_url}/SHA256SUMS.txt"),
            "CloudDeck-0.2.0-windows-x64-setup.exe",
            "0.2.0",
            tampered_dir.path(),
            true,
        )
        .await;
        assert!(tampered.is_err());
        assert!(!tampered_dir
            .path()
            .join("updates/0.2.0/CloudDeck-0.2.0-windows-x64-setup.exe")
            .exists());
        assert!(!tampered_dir
            .path()
            .join("updates/0.2.0/CloudDeck-0.2.0-windows-x64-setup.exe.part")
            .exists());
        let _ = handle.join();
    }
}

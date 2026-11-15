use crate::model::{parse_observation, Account, Observation};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use url::Url;

const SERVICE: &str = "Cloud Deck";
const CREDENTIAL: &str = "google-oauth-v1";
const CONFIG_URL: &str = "https://raw.githubusercontent.com/googlecolab/google-colab-cli/main/src/colab_cli/oauth_config.json";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPES: &str = "openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/colaboratory";

pub fn random_token(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0; bytes];
    getrandom::fill(&mut buffer).map_err(|_| "无法生成安全随机数。")?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

pub(crate) fn client() -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法建立安全网络客户端。".into())
}

async fn json(response: reqwest::Response, operation: &str) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{operation}失败（HTTP {}）。", status.as_u16()));
    }
    let raw = response
        .text()
        .await
        .map_err(|_| format!("{operation}响应读取失败。"))?;
    json_body(&raw).map_err(|_| format!("{operation}返回了无法识别的数据。"))
}

fn json_body(raw: &str) -> Result<Value, serde_json::Error> {
    let text = if raw.starts_with(")]}'") {
        raw.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        raw
    };
    if text.trim().is_empty() {
        Ok(serde_json::json!({}))
    } else {
        serde_json::from_str(text)
    }
}

fn colab_body(
    request: reqwest::RequestBuilder,
    method: &reqwest::Method,
    body: Option<(&str, Vec<u8>)>,
) -> reqwest::RequestBuilder {
    if let Some((content_type, bytes)) = body {
        request
            .header(reqwest::header::CONTENT_LENGTH, bytes.len())
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
    } else if *method == reqwest::Method::POST {
        // Google's tunnel backend requires this even for a zero-byte HTTP/2 POST.
        request
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .body(Vec::new())
    } else {
        request
    }
}

#[derive(Serialize, Deserialize)]
struct DesktopClient {
    client_id: String,
    client_secret: String,
}

#[derive(Serialize, Deserialize)]
pub struct Session {
    client: DesktopClient,
    refresh_token: String,
    pub account: Account,
    #[serde(skip)]
    access_token: String,
}

impl Session {
    pub fn store(&self) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|_| "无法编码授权资料。")?;
        let mut cache = crate::credentials::SESSION
            .lock()
            .map_err(|_| "授权缓存不可用。")?;
        keyring::Entry::new(SERVICE, CREDENTIAL)
            .map_err(|_| "无法访问系统凭据存储。")?
            .set_password(&text)
            .map_err(|_| "系统凭据存储未能保存授权。请检查钥匙串或凭据管理器权限。")?;
        cache.replace(Some(text));
        Ok(())
    }

    pub fn load_optional() -> Result<Option<Self>, String> {
        let text = crate::credentials::SESSION
            .lock()
            .map_err(|_| "授权缓存不可用。")?
            .read(Self::read_credential)?;
        text.map(|text| {
            serde_json::from_str(&text).map_err(|_| "保存的授权资料无效，未覆盖原授权。".into())
        })
        .transpose()
    }

    pub fn retry_credential_access() -> Result<(), String> {
        crate::credentials::SESSION
            .lock()
            .map_err(|_| "授权缓存不可用。")?
            .retry();
        Ok(())
    }

    fn read_credential() -> Result<Option<String>, String> {
        let entry =
            keyring::Entry::new(SERVICE, CREDENTIAL).map_err(|_| "无法访问系统凭据存储。")?;
        let text = match entry.get_password() {
            Ok(text) => text,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(_) => {
                return Err(
                    "无法读取系统凭据存储，原授权未改动。请检查钥匙串或凭据管理器权限。".into(),
                )
            }
        };
        Ok(Some(text))
    }

    pub fn load() -> Result<Self, String> {
        Self::load_optional()?.ok_or_else(|| "没有可用的本机授权，请重新登录 Google。".into())
    }

    pub fn forget() -> Result<(), String> {
        let mut cache = crate::credentials::SESSION
            .lock()
            .map_err(|_| "授权缓存不可用。")?;
        let entry =
            keyring::Entry::new(SERVICE, CREDENTIAL).map_err(|_| "无法访问系统凭据存储。")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {
                cache.replace(None);
                Ok(())
            }
            Err(_) => Err("无法移除本机授权，请检查系统凭据存储。".into()),
        }
    }

    async fn access(&mut self, http: &reqwest::Client) -> Result<(), String> {
        if !self.access_token.is_empty() {
            return Ok(());
        }
        let response = http
            .post(TOKEN_URL)
            .form(&[
                ("client_id", self.client.client_id.as_str()),
                ("client_secret", &self.client.client_secret),
                ("refresh_token", &self.refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|_| "Google 授权刷新网络请求失败。")?;
        let data = json(response, "Google 授权刷新").await?;
        self.access_token = data["access_token"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("Google 未返回访问令牌。")?
            .into();
        if let Some(refresh) = data["refresh_token"].as_str().filter(|v| !v.is_empty()) {
            self.refresh_token = refresh.into();
            self.store()?;
        }
        Ok(())
    }

    pub async fn colab(
        &mut self,
        path: &str,
        method: reqwest::Method,
        params: &[(&str, String)],
        csrf: Option<&str>,
        body: Option<(&str, Vec<u8>)>,
    ) -> Result<Value, String> {
        let http = client()?;
        self.access(&http).await?;
        let mut request = http
            .request(
                method.clone(),
                format!("https://colab.research.google.com{path}"),
            )
            .timeout(Duration::from_secs(90))
            .query(&[("authuser", "0")])
            .query(params)
            .bearer_auth(&self.access_token)
            .header("X-Colab-Client-Agent", "colab-cli");
        if let Some(token) = csrf {
            request = request.header("X-Goog-Colab-Token", token);
        }
        request = colab_body(request, &method, body);
        let response = request
            .send()
            .await
            .map_err(|_| "Colab 请求未完成，实例与计费状态待确认；未自动重试分配。")?;
        let operation = match path {
            "/tun/m/ccu-info" => "Colab 用量查询",
            "/tun/m/assignments" => "Colab 实例查询",
            "/tun/m/assign" => "Colab 实例分配",
            value if value.starts_with("/tun/m/unassign/") => "Colab 实例释放",
            value if value.starts_with("/tun/m/credentials-propagation/") => "Drive 授权",
            _ => "Colab 请求",
        };
        json(response, operation).await
    }

    pub async fn observe(&mut self) -> Result<Observation, String> {
        let http = client()?;
        self.access(&http).await?;
        let identity = json(
            http.get("https://openidconnect.googleapis.com/v1/userinfo")
                .bearer_auth(&self.access_token)
                .send()
                .await
                .map_err(|_| "无法确认 Google 账号身份。")?,
            "Google 身份查询",
        )
        .await?;
        if identity["sub"].as_str() != Some(self.account.subject.as_str()) {
            return Err("授权账号与当前工作空间不一致，未更新云端状态。".into());
        }
        let usage = self
            .colab("/tun/m/ccu-info", reqwest::Method::GET, &[], None, None)
            .await?;
        let runtimes = self
            .colab("/tun/m/assignments", reqwest::Method::GET, &[], None, None)
            .await?;
        #[cfg(feature = "native-smoke")]
        if crate::live_check_mode() {
            // QA diagnostics contain field names/types and quota scalars, never credentials.
            if let Some(root) = std::env::var_os("CLOUD_DECK_DATA_DIR") {
                let fields: serde_json::Map<String, Value> = usage
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(key, value)| {
                        let kind = match value {
                            Value::Null => "null",
                            Value::Bool(_) => "boolean",
                            Value::Number(_) => "number",
                            Value::String(_) => "string",
                            Value::Array(_) => "array",
                            Value::Object(_) => "object",
                        };
                        (key.clone(), Value::String(kind.into()))
                    })
                    .collect();
                let shape = serde_json::json!({"fields":fields,"currentBalance":usage["currentBalance"],"consumptionRateHourly":usage["consumptionRateHourly"],"assignmentCount":runtimes["assignments"].as_array().map(Vec::len)});
                let _ = std::fs::write(
                    std::path::PathBuf::from(root).join("live-usage-shape.json"),
                    shape.to_string(),
                );
            }
        }
        parse_observation(&usage, &runtimes)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub message: String,
}

fn progress(app: &AppHandle, phase: &str, message: &str) {
    let _ = app.emit(
        "auth-progress",
        Progress {
            phase: phase.into(),
            message: message.into(),
        },
    );
}

pub fn callback_code(path: &str, expected: &str) -> Result<Option<String>, String> {
    let url = Url::parse(&format!("http://127.0.0.1{path}")).map_err(|_| "无效的授权回调。")?;
    if url.path() != "/callback" {
        return Ok(None);
    }
    let query: Vec<_> = url.query_pairs().collect();
    let values = |key: &str| {
        query
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
            .collect::<Vec<_>>()
    };
    let state = values("state");
    if state.len() != 1 || !bool::from(state[0].as_bytes().ct_eq(expected.as_bytes())) {
        return Err("授权请求不匹配。".into());
    }
    let code = values("code");
    let error = values("error");
    if code.is_empty() && error.len() == 1 {
        return Err("Google 授权未完成或被取消。".into());
    }
    if !error.is_empty() || code.len() != 1 || code[0].is_empty() || code[0].len() > 8192 {
        return Err("授权回调参数无效。".into());
    }
    Ok(Some(code[0].to_owned()))
}

fn browser_response(status: u16, message: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = format!("<!doctype html><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Cloud Deck</title><h1>Cloud Deck</h1><p>{message}</p><p>可以关闭这个页面并回到客户端。</p>");
    let mut response = tiny_http::Response::from_string(body).with_status_code(status);
    for (name, value) in [
        ("Content-Type", "text/html; charset=utf-8"),
        ("Cache-Control", "no-store"),
        ("Referrer-Policy", "no-referrer"),
        (
            "Content-Security-Policy",
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
        ),
    ] {
        response.add_header(tiny_http::Header::from_bytes(name, value).expect("static header"));
    }
    response
}

#[derive(Clone, Debug, Deserialize)]
struct Claims {
    sub: String,
    nonce: String,
    email: String,
    email_verified: bool,
    #[serde(default)]
    name: String,
}

pub async fn login(
    app: AppHandle,
    cancelled: Arc<AtomicBool>,
    url_slot: Arc<std::sync::Mutex<Option<String>>>,
) -> Result<Session, String> {
    progress(&app, "preparing", "正在准备 Google 登录…");
    let http = client()?;
    let config = json(
        http.get(CONFIG_URL)
            .send()
            .await
            .map_err(|_| "无法取得 Colab 登录配置，请检查网络。")?,
        "Google 登录配置查询",
    )
    .await?;
    let desktop: DesktopClient = serde_json::from_value(config["installed"].clone())
        .map_err(|_| "Colab 桌面登录配置无效。")?;
    if !desktop.client_id.ends_with(".apps.googleusercontent.com")
        || desktop.client_secret.is_empty()
    {
        return Err("Colab 登录客户端配置无效。".into());
    }
    if cancelled.load(Ordering::SeqCst) {
        return Err("登录已取消。".into());
    }
    let server =
        tiny_http::Server::http("127.0.0.1:0").map_err(|_| "无法建立本机登录回调端口。")?;
    let address = server
        .server_addr()
        .to_ip()
        .ok_or("无法读取本机回调端口。")?;
    let redirect = format!("http://127.0.0.1:{}/callback", address.port());
    let state = random_token(32)?;
    let nonce = random_token(32)?;
    let verifier = random_token(64)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut authorize =
        Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("static URL");
    authorize.query_pairs_mut().extend_pairs([
        ("client_id", desktop.client_id.as_str()),
        ("redirect_uri", &redirect),
        ("response_type", "code"),
        ("scope", SCOPES),
        ("access_type", "offline"),
        ("prompt", "select_account consent"),
        ("state", &state),
        ("nonce", &nonce),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    *url_slot.lock().map_err(|_| "登录状态不可用。")? = Some(authorize.to_string());
    app.opener()
        .open_url(authorize.as_str(), None::<&str>)
        .map_err(|_| "无法打开系统浏览器。请重试登录。")?;
    progress(&app, "waiting", "请在系统浏览器选择 Google 账号并授权。");
    let cancel = cancelled.clone();
    let code = tauri::async_runtime::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(600);
        while Instant::now() < deadline {
            if cancel.load(Ordering::SeqCst) {
                return Err("登录已取消。".to_string());
            }
            let Some(request) = server
                .recv_timeout(Duration::from_millis(250))
                .map_err(|_| "授权回调读取失败。")?
            else {
                continue;
            };
            let host = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Host"))
                .map(|h| h.value.as_str());
            let expected_host = format!("127.0.0.1:{}", address.port());
            if request.method() != &tiny_http::Method::Get || host != Some(expected_host.as_str()) {
                let _ = request.respond(browser_response(400, "回调请求无效。"));
                continue;
            }
            match callback_code(request.url(), &state) {
                Ok(Some(code)) => {
                    let _ = request.respond(browser_response(200, "已收到授权，客户端正在验证。"));
                    return Ok(code);
                }
                Ok(None) => {
                    let _ =
                        request.respond(browser_response(404, "这个地址只用于 Google 登录返回。"));
                }
                Err(error) => {
                    let valid_state = Url::parse(&format!("http://127.0.0.1{}", request.url()))
                        .ok()
                        .is_some_and(|u| {
                            u.query_pairs().filter(|(k, _)| k == "state").count() == 1
                                && u.query_pairs().any(|(k, v)| {
                                    k == "state" && bool::from(v.as_bytes().ct_eq(state.as_bytes()))
                                })
                        });
                    let _ = request.respond(browser_response(
                        400,
                        "本次授权未完成或请求无效，请回到客户端重试。",
                    ));
                    if valid_state {
                        return Err(error);
                    }
                }
            }
        }
        Err("登录等待已超时，请重新开始。".into())
    })
    .await
    .map_err(|_| "登录回调任务已中断。")??;
    if cancelled.load(Ordering::SeqCst) {
        return Err("登录已取消。".into());
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    progress(&app, "verifying", "正在验证 Google 身份与授权…");
    let token = json(
        http.post(TOKEN_URL)
            .form(&[
                ("client_id", desktop.client_id.as_str()),
                ("client_secret", &desktop.client_secret),
                ("code", &code),
                ("redirect_uri", &redirect),
                ("grant_type", "authorization_code"),
                ("code_verifier", &verifier),
            ])
            .send()
            .await
            .map_err(|_| "Google 授权码兑换网络请求失败。")?,
        "Google 授权码兑换",
    )
    .await?;
    let id_token = token["id_token"]
        .as_str()
        .ok_or("Google 未返回身份凭证。")?;
    let header = decode_header(id_token).map_err(|_| "Google 身份凭证格式无效。")?;
    if header.alg != Algorithm::RS256 {
        return Err("不支持的身份凭证签名方式。".into());
    }
    let keys: JwkSet = serde_json::from_value(
        json(
            http.get("https://www.googleapis.com/oauth2/v3/certs")
                .send()
                .await
                .map_err(|_| "无法取得 Google 身份验证公钥。")?,
            "Google 公钥查询",
        )
        .await?,
    )
    .map_err(|_| "Google 公钥数据无效。")?;
    let key = keys
        .find(header.kid.as_deref().ok_or("身份凭证缺少签名标识。")?)
        .ok_or("未找到身份凭证的验证公钥。")?;
    let decoding = DecodingKey::from_jwk(key).map_err(|_| "无法加载 Google 身份验证公钥。")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&desktop.client_id]);
    validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = decode::<Claims>(id_token, &decoding, &validation)
        .map_err(|_| "Google 身份凭证校验失败。")?
        .claims;
    if !bool::from(claims.nonce.as_bytes().ct_eq(nonce.as_bytes()))
        || claims.sub.is_empty()
        || !claims.email_verified
    {
        return Err("Google 账号或授权请求校验失败。".into());
    }
    if cancelled.load(Ordering::SeqCst) {
        return Err("登录已取消。".into());
    }
    let refresh = token["refresh_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("Google 未返回离线授权，请重新登录并同意授权。")?;
    let access = token["access_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("Google 未返回访问令牌。")?;
    Ok(Session {
        client: desktop,
        refresh_token: refresh.into(),
        access_token: access.into(),
        account: Account {
            subject: claims.sub,
            name: if claims.name.is_empty() {
                claims
                    .email
                    .split('@')
                    .next()
                    .unwrap_or("Google 账号")
                    .into()
            } else {
                claims.name
            },
            email: claims.email,
            status: "connected".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_empty_responses_do_not_mask_release_results() {
        assert_eq!(json_body("").unwrap(), serde_json::json!({}));
        assert_eq!(json_body("\r\n").unwrap(), serde_json::json!({}));
        assert_eq!(
            json_body(")]}'\n{\"ok\":true}").unwrap(),
            serde_json::json!({"ok":true})
        );
        assert!(json_body("<html>Service unavailable</html>").is_err());
    }
    #[test]
    fn tunnel_posts_include_length_even_when_the_body_is_empty() {
        let http = client().unwrap();
        let empty = colab_body(
            http.post("https://colab.research.google.com/tun/m/assign"),
            &reqwest::Method::POST,
            None,
        )
        .build()
        .unwrap();
        assert_eq!(empty.headers()[reqwest::header::CONTENT_LENGTH], "0");
        let payload = colab_body(
            http.post("https://colab.research.google.com/test"),
            &reqwest::Method::POST,
            Some(("application/json", b"{}".to_vec())),
        )
        .build()
        .unwrap();
        assert_eq!(payload.headers()[reqwest::header::CONTENT_LENGTH], "2");
        assert_eq!(
            payload.headers()[reqwest::header::CONTENT_TYPE],
            "application/json"
        );
    }
    #[test]
    fn callback_rejects_mismatched_duplicate_and_mixed_parameters() {
        assert_eq!(
            callback_code("/callback?state=expected&code=one", "expected").unwrap(),
            Some("one".into())
        );
        for path in [
            "/callback?state=wrong&code=one",
            "/callback?state=expected&state=expected&code=one",
            "/callback?state=expected&code=one&code=two",
            "/callback?state=expected&code=one&error=denied",
        ] {
            assert!(callback_code(path, "expected").is_err());
        }
        assert!(callback_code("/callback?state=expected&error=access_denied", "expected").is_err());
        assert_eq!(callback_code("/", "expected").unwrap(), None);
        let a = random_token(64).unwrap();
        assert!(a.len() >= 43 && a.len() <= 128);
        assert_ne!(a, random_token(64).unwrap());
    }
}

//! SSH bytes stay inside the authenticated Colab WSS connection; no executables.
use futures_util::{SinkExt, StreamExt};
use russh::{
    client,
    keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    ChannelMsg,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use url::Url;

type HostKeys = Arc<Mutex<BTreeMap<String, String>>>;
struct Handler {
    endpoint: String,
    trusted: HostKeys,
}
impl client::Handler for Handler {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // First trust is established by Google's TLS + account-bound runtime token.
        // Pin the key for this endpoint so reconnecting cannot silently replace it.
        let encoded = format!("{key:?}");
        let Ok(mut trusted) = self.trusted.lock() else {
            return Ok(false);
        };
        Ok(match trusted.get(&self.endpoint) {
            Some(existing) => existing == &encoded,
            None => {
                trusted.insert(self.endpoint.clone(), encoded);
                true
            }
        })
    }
}

fn protected_connection_error(error: tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error;
    let reason = match error {
        Error::Http(response) => format!("HTTP {}", response.status().as_u16()),
        Error::Io(error) => format!("网络 {:?}", error.kind()),
        Error::Tls(_) => "TLS 握手".into(),
        Error::Protocol(_) => "WebSocket 协议".into(),
        _ => "连接协商".into(),
    };
    // Never format the request/response: proxy URLs and headers contain credentials.
    format!("Colab 受保护连接失败（{reason}）；实例与计费状态待确认。")
}

pub fn proxy_origin(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "实例代理地址无效。")?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !(host.ends_with(".googleusercontent.com") || host.ends_with(".prod.r.colab.dev"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("实例代理必须是 Google 提供的 HTTPS 地址。".into());
    }
    Ok(url)
}

pub struct Transport {
    handle: Arc<client::Handle<Handler>>,
    bridge: JoinHandle<()>,
    forwards: BTreeMap<u16, JoinHandle<()>>,
}
impl Drop for Transport {
    fn drop(&mut self) {
        for task in self.forwards.values() {
            task.abort();
        }
        self.bridge.abort();
    }
}
impl Transport {
    pub async fn connect(
        endpoint: &str,
        proxy: &str,
        token: &str,
        trusted: HostKeys,
    ) -> Result<Self, String> {
        let mut url = proxy_origin(proxy)?;
        url.set_scheme("wss").map_err(|_| "无法建立受保护连接。")?;
        url.set_path("/colab/ssh");
        url.query_pairs_mut()
            .append_pair("colab-runtime-proxy-token", token)
            .append_pair("colab-client-agent", "colab-cli")
            .append_pair("colab-client-agent", "ssh-mode-proxy");
        let mut seed = [0; 32];
        getrandom::fill(&mut seed).map_err(|_| "无法生成临时连接密钥。")?;
        let key = PrivateKey::from(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(
            &seed,
        ));
        seed.fill(0);
        let public = key
            .public_key()
            .to_openssh()
            .map_err(|_| "无法编码连接公钥。")?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| "无法建立受保护连接。")?;
        request.headers_mut().insert(
            "X-Colab-Ssh-Pubkey",
            public.parse().map_err(|_| "连接公钥无效。")?,
        );
        request
            .headers_mut()
            .insert("X-Colab-Client-Agent", "colab-cli".parse().unwrap());
        // Preserve the failure class without exposing private proxy URLs or headers.
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(30), connect_async(request))
            .await
            .map_err(|_| "Colab 连接超时；实例可能仍在计费。")?
            .map_err(protected_connection_error)?;
        let (stream, pipe) = tokio::io::duplex(256 * 1024);
        let (mut reader, mut writer) = tokio::io::split(pipe);
        let bridge = tokio::spawn(async move {
            let mut bytes = vec![0; 32 * 1024];
            let mut ping = tokio::time::interval(Duration::from_secs(20));
            let mut last_response = tokio::time::Instant::now();
            loop {
                tokio::select! {
                    result = reader.read(&mut bytes) => match result {
                        Ok(0) | Err(_) => break,
                        Ok(count) => if socket.send(Message::Binary(bytes[..count].to_vec().into())).await.is_err() { break; },
                    },
                    frame = socket.next() => {
                        last_response = tokio::time::Instant::now();
                        match frame {
                            Some(Ok(Message::Binary(data))) => if writer.write_all(&data).await.is_err() { break; },
                            Some(Ok(Message::Text(text))) => if writer.write_all(text.as_bytes()).await.is_err() { break; },
                            Some(Ok(Message::Ping(data))) => if socket.send(Message::Pong(data)).await.is_err() { break; },
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {},
                        }
                    },
                    _ = ping.tick() => {
                        if last_response.elapsed() > Duration::from_secs(70)
                            || socket.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
                    }
                }
            }
            let _ = writer.shutdown().await;
            let _ = socket.close(None).await;
        });
        let connected = async {
            let config = Arc::new(client::Config {
                inactivity_timeout: Some(Duration::from_secs(90)),
                ..Default::default()
            });
            let mut handle = client::connect_stream(
                config,
                stream,
                Handler {
                    endpoint: endpoint.into(),
                    trusted,
                },
            )
            .await
            .map_err(|_| "内置 SSH 握手失败。")?;
            let authenticated = handle
                .authenticate_publickey("root", PrivateKeyWithHashAlg::new(Arc::new(key), None))
                .await
                .map_err(|_| "内置连接认证失败。")?;
            if !authenticated.success() {
                return Err("实例拒绝连接认证。".into());
            }
            Ok(Arc::new(handle))
        };
        match tokio::time::timeout(Duration::from_secs(35), connected).await {
            Ok(Ok(handle)) => Ok(Self {
                handle,
                bridge,
                forwards: BTreeMap::new(),
            }),
            result => {
                bridge.abort();
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => "内置 SSH 握手超时。".into(),
                })
            }
        }
    }
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed() || self.bridge.is_finished()
    }
    pub async fn close(&mut self) {
        for task in self.forwards.values() {
            task.abort();
        }
        self.forwards.clear();
        if self.bridge.is_finished() {
            return;
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            self.handle
                .disconnect(russh::Disconnect::ByApplication, "Reconnecting", "en"),
        )
        .await;
        if tokio::time::timeout(Duration::from_secs(3), &mut self.bridge)
            .await
            .is_err()
        {
            self.bridge.abort();
            let _ = (&mut self.bridge).await;
        }
    }
    pub async fn open_local(&self, port: u16) -> Result<russh::ChannelStream<client::Msg>, String> {
        let channel = tokio::time::timeout(
            Duration::from_secs(15),
            self.handle
                .channel_open_direct_tcpip("127.0.0.1", u32::from(port), "127.0.0.1", 0),
        )
        .await
        .map_err(|_| "实例本机连接超时。")?
        .map_err(|_| "无法连接实例本机服务。")?;
        Ok(channel.into_stream())
    }
    pub async fn execute(
        &self,
        command: &str,
        input: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let operation = async {
            let mut channel = self
                .handle
                .channel_open_session()
                .await
                .map_err(|_| "无法建立远程命令通道。")?;
            channel
                .exec(true, command)
                .await
                .map_err(|_| "远程命令未能提交。")?;
            if !input.is_empty() {
                channel
                    .data(input)
                    .await
                    .map_err(|_| "远程数据上传失败。")?;
            }
            channel.eof().await.map_err(|_| "远程输入未能完成。")?;
            let mut output = Vec::new();
            let mut status = None;
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => {
                        output.extend_from_slice(&data);
                        if output.len() > 2 * 1024 * 1024 {
                            return Err("远程响应超过大小限制。".into());
                        }
                    }
                    ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                    _ => {}
                }
            }
            match status {
                Some(0) => Ok(output),
                Some(code) => Err(format!("远程命令失败（退出码 {code}）；实例保留。")),
                None => Err("远程命令结果未确认；实例保留。".into()),
            }
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| "远程命令超时；实例和文件保留。")?
    }
    pub async fn forward(&mut self, local: u16, remote: u16) -> Result<(), String> {
        if self
            .forwards
            .get(&local)
            .is_some_and(|task| !task.is_finished())
        {
            return Ok(());
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, local))
            .await
            .map_err(|_| format!("本机端口 {local} 已被占用，未接管现有进程。"))?;
        let handle = self.handle.clone();
        self.forwards.insert(local, tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => match accepted {
                        Ok((mut socket, peer)) => {
                            let handle = handle.clone();
                            clients.spawn(async move {
                                if let Ok(channel) = handle.channel_open_direct_tcpip("127.0.0.1", remote as u32, "127.0.0.1", peer.port() as u32).await {
                                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut channel.into_stream()).await;
                                }
                            });
                        },
                        Err(_) => break,
                    },
                    _ = clients.join_next(), if !clients.is_empty() => {},
                }
            }
        }));
        Ok(())
    }
    pub fn stop_forward(&mut self, local: u16) {
        if let Some(task) = self.forwards.remove(&local) {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_errors_report_status_without_private_urls_or_headers() {
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .status(429)
            .header("location", "https://private.example/?token=DO_NOT_LOG")
            .body(None)
            .unwrap();
        let message = protected_connection_error(tokio_tungstenite::tungstenite::Error::Http(
            Box::new(response),
        ));
        assert!(message.contains("HTTP 429"));
        assert!(!message.contains("private.example"));
        assert!(!message.contains("DO_NOT_LOG"));
        let message = protected_connection_error(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::new(std::io::ErrorKind::ConnectionReset, "DO_NOT_LOG"),
        ));
        assert!(message.contains("ConnectionReset"));
        assert!(!message.contains("DO_NOT_LOG"));
    }
    #[test]
    fn proxy_credentials_only_go_to_google_tls_origins() {
        assert!(proxy_origin("https://runtime.colab.googleusercontent.com/").is_ok());
        assert!(proxy_origin("https://8080-runtime.us-east1.prod.r.colab.dev").is_ok());
        for value in [
            "http://runtime.prod.r.colab.dev",
            "https://runtime.prod.r.colab.dev.attacker.example",
            "https://runtime.prod.r.colab.dev@attacker.example",
            "http://runtime.colab.googleusercontent.com",
            "https://googleusercontent.com",
            "https://runtime.colab.googleusercontent.com.attacker.example",
            "https://user@runtime.colab.googleusercontent.com",
            "https://runtime.colab.googleusercontent.com:444",
            "https://runtime.colab.googleusercontent.com/?other=1",
        ] {
            assert!(proxy_origin(value).is_err(), "{value}");
        }
    }
}

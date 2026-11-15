use crate::auth;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{future::Future, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::{sleep, timeout_at, Instant},
};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

pub(crate) trait Authorizer {
    fn propagate(&mut self) -> impl Future<Output = Result<Option<String>, String>> + Send;
}

#[derive(Clone, Copy)]
struct Timing {
    consent: Duration,
    mount: Duration,
    poll: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            consent: Duration::from_secs(300),
            mount: Duration::from_secs(240),
            poll: Duration::from_secs(3),
        }
    }
}

pub(crate) async fn mount<Connection, Authorization, Notify>(
    socket: &mut WebSocketStream<Connection>,
    authorizer: &mut Authorization,
    client_session: &str,
    notify: Notify,
) -> Result<(), String>
where
    Connection: AsyncRead + AsyncWrite + Unpin,
    Authorization: Authorizer,
    Notify: FnMut(Option<String>) -> Result<(), String>,
{
    mount_with_timing(
        socket,
        authorizer,
        client_session,
        notify,
        Timing::default(),
    )
    .await
}

async fn receive<Connection>(
    socket: &mut WebSocketStream<Connection>,
) -> Result<Option<Value>, String>
where
    Connection: AsyncRead + AsyncWrite + Unpin,
{
    let frame = socket
        .next()
        .await
        .ok_or("Drive 会话已断开。")?
        .map_err(|_| "Drive 会话已断开。")?;
    let bytes = match frame {
        Message::Text(text) => text.as_bytes().to_vec(),
        Message::Binary(bytes) => bytes.to_vec(),
        Message::Ping(data) => {
            socket
                .send(Message::Pong(data))
                .await
                .map_err(|_| "Drive 会话已断开。")?;
            return Ok(None);
        }
        Message::Close(_) => return Err("Drive 挂载会话提前结束。".into()),
        _ => return Ok(None),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "Drive 会话返回了无效消息。".into())
}

fn kind(message: &Value) -> &str {
    message["header"]["msg_type"]
        .as_str()
        .or_else(|| message["msg_type"].as_str())
        .unwrap_or("")
}

fn check_execution(message: &Value, execution: &str) -> Result<(), String> {
    if message["parent_header"]["msg_id"] == execution
        && (kind(message) == "error"
            || (kind(message) == "execute_reply" && message["content"]["status"] == "error"))
    {
        return Err("Drive 挂载失败；应用未启动，实例保留。".into());
    }
    Ok(())
}

async fn authorize<Connection, Authorization, Notify>(
    socket: &mut WebSocketStream<Connection>,
    authorizer: &mut Authorization,
    notify: &mut Notify,
    execution: &str,
    poll: Duration,
) -> Result<(), String>
where
    Connection: AsyncRead + AsyncWrite + Unpin,
    Authorization: Authorizer,
    Notify: FnMut(Option<String>) -> Result<(), String>,
{
    let mut delay = Duration::ZERO;
    loop {
        let consent = {
            let probe = async {
                sleep(delay).await;
                authorizer.propagate().await
            };
            tokio::pin!(probe);
            // Keep servicing the same kernel socket while the user is in the browser.
            // A ping must not cancel an in-flight credential propagation request.
            loop {
                tokio::select! {
                    result = &mut probe => break result?,
                    message = receive(socket) => {
                        if let Some(message) = message? {
                            check_execution(&message, execution)?;
                        }
                    }
                }
            }
        };
        let ready = consent.is_none();
        notify(consent)?;
        if ready {
            return Ok(());
        }
        delay = poll;
    }
}

async fn mount_with_timing<Connection, Authorization, Notify>(
    socket: &mut WebSocketStream<Connection>,
    authorizer: &mut Authorization,
    client_session: &str,
    mut notify: Notify,
    timing: Timing,
) -> Result<(), String>
where
    Connection: AsyncRead + AsyncWrite + Unpin,
    Authorization: Authorizer,
    Notify: FnMut(Option<String>) -> Result<(), String>,
{
    let execution = auth::random_token(16)?;
    // Colab uses timeout_ms for its own authorization wait too. Allow enough time
    // for the browser step; local deadlines remain bounded and fail the startup.
    socket.send(Message::Text(json!({"header":{"msg_id":execution,"username":"cloud-deck","session":client_session,"msg_type":"execute_request","version":"5.3"},"parent_header":{},"metadata":{},"content":{"code":"from google.colab import drive\ndrive.mount('/content/drive', timeout_ms=600000)\n","silent":false,"store_history":false,"user_expressions":{},"allow_stdin":true,"stop_on_error":true},"buffers":[],"channel":"shell"}).to_string().into()))
        .await.map_err(|_| "Drive 挂载请求未能发送。")?;
    let mut deadline = Instant::now() + timing.mount;
    let mut replied = false;
    let mut idle = false;
    loop {
        let Some(message) = timeout_at(deadline, receive(socket))
            .await
            .map_err(|_| "Drive 挂载超时，实例保留。")??
        else {
            continue;
        };
        check_execution(&message, &execution)?;
        let matching_execution = message["parent_header"]["msg_id"] == execution;
        // Colab may route request_auth with only the parent session, without msg_id.
        let matching_session = message["parent_header"]["msg_id"].is_null()
            && message["parent_header"]["session"] == client_session;
        if kind(&message) == "colab_request"
            && message["content"]["request"]["authType"] == "dfs_ephemeral"
            && (matching_execution || matching_session)
        {
            if message["metadata"]["colab_msg_id"].is_null() {
                return Err("Drive 授权请求缺少会话标识。".into());
            }
            timeout_at(
                Instant::now() + timing.consent,
                authorize(socket, authorizer, &mut notify, &execution, timing.poll),
            )
            .await
            .map_err(|_| {
                "等待 Drive 授权超时；请重新启动应用后完成授权，实例保留并可能继续计费。"
            })??;
            socket.send(Message::Text(json!({"header":{"msg_id":auth::random_token(16)?,"username":"cloud-deck","session":client_session,"msg_type":"input_reply","version":"5.3"},"parent_header":message["header"],"metadata":{},"content":{"value":{"type":"colab_reply","colab_msg_id":message["metadata"]["colab_msg_id"]}},"buffers":[],"channel":"stdin"}).to_string().into()))
                .await.map_err(|_| "Drive 授权确认未能发送。")?;
            deadline = Instant::now() + timing.mount;
            continue;
        }
        if !matching_execution {
            continue;
        }
        if kind(&message) == "execute_reply" {
            replied = message["content"]["status"] == "ok";
        }
        if kind(&message) == "status" && message["content"]["execution_state"] == "idle" {
            idle = true;
        }
        if replied && idle {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use tokio::io::{duplex, DuplexStream};
    use tokio_tungstenite::tungstenite::protocol::Role;

    const CONSENT_URL: &str = "https://accounts.google.com/consent";

    struct BrowserAuthorization {
        results: VecDeque<Result<Option<String>, String>>,
        attempts: usize,
    }

    impl Authorizer for BrowserAuthorization {
        async fn propagate(&mut self) -> Result<Option<String>, String> {
            self.attempts += 1;
            sleep(Duration::from_secs(1)).await;
            self.results
                .pop_front()
                .unwrap_or_else(|| Ok(Some(CONSENT_URL.into())))
        }
    }

    async fn sockets() -> (WebSocketStream<DuplexStream>, WebSocketStream<DuplexStream>) {
        let (client, server) = duplex(8192);
        tokio::join!(
            WebSocketStream::from_raw_socket(client, Role::Client, None),
            WebSocketStream::from_raw_socket(server, Role::Server, None),
        )
    }

    async fn send(socket: &mut WebSocketStream<DuplexStream>, message: Value) {
        socket
            .send(Message::Text(message.to_string().into()))
            .await
            .unwrap();
    }

    async fn request_authorization(socket: &mut WebSocketStream<DuplexStream>) -> Value {
        let request = receive(socket).await.unwrap().unwrap();
        assert_eq!(kind(&request), "execute_request");
        assert!(request["content"]["code"]
            .as_str()
            .unwrap()
            .contains("timeout_ms=600000"));
        // Messages from other browser/kernel sessions must never trigger consent.
        send(socket, json!({"header":{"msg_type":"colab_request"},"parent_header":{"session":"unrelated"},"metadata":{"colab_msg_id":99},"content":{"request":{"authType":"dfs_ephemeral"}}})).await;
        // Colab's real request_auth messages can contain only a parent session.
        send(socket, json!({"header":{"msg_id":"auth-request","msg_type":"colab_request"},"parent_header":{"session":request["header"]["session"]},"metadata":{"colab_msg_id":42},"content":{"request":{"authType":"dfs_ephemeral"}}})).await;
        request
    }

    async fn complete_mount(socket: &mut WebSocketStream<DuplexStream>, request: &Value) {
        let reply = loop {
            if let Some(message) = receive(socket).await.unwrap() {
                break message;
            }
        };
        assert_eq!(kind(&reply), "input_reply");
        assert_eq!(reply["parent_header"]["msg_id"], "auth-request");
        assert_eq!(reply["content"]["value"]["colab_msg_id"], 42);
        assert_eq!(reply["header"]["session"], request["header"]["session"]);
        send(socket, json!({"header":{"msg_type":"status"},"parent_header":request["header"],"content":{"execution_state":"idle"}})).await;
        send(socket, json!({"header":{"msg_type":"execute_reply"},"parent_header":request["header"],"content":{"status":"ok"}})).await;
    }

    #[tokio::test(start_paused = true)]
    async fn browser_consent_resumes_the_same_mount_and_keeps_pings_alive() {
        let (mut client, mut server) = sockets().await;
        let mut authorizer = BrowserAuthorization {
            results: VecDeque::from([Ok(Some(CONSENT_URL.into())), Ok(None)]),
            attempts: 0,
        };
        let mut notices = Vec::new();
        let mounting = mount(&mut client, &mut authorizer, "test-session", |consent| {
            notices.push(consent);
            Ok(())
        });
        let kernel = async {
            let request = request_authorization(&mut server).await;
            // Pings arrive during the HTTP probe, then again while waiting for consent.
            for _attempt in 0..2 {
                server
                    .send(Message::Ping(vec![1, 2, 3].into()))
                    .await
                    .unwrap();
                assert_eq!(
                    server.next().await.unwrap().unwrap(),
                    Message::Pong(vec![1, 2, 3].into())
                );
                sleep(Duration::from_secs(2)).await;
            }
            complete_mount(&mut server, &request).await;
        };
        let (result, ()) = tokio::join!(mounting, kernel);
        assert_eq!(result, Ok(()));
        assert_eq!(authorizer.attempts, 2);
        assert_eq!(notices, vec![Some(CONSENT_URL.into()), None]);
    }

    #[tokio::test(start_paused = true)]
    async fn existing_authorization_continues_without_a_browser_prompt() {
        let (mut client, mut server) = sockets().await;
        let mut authorizer = BrowserAuthorization {
            results: VecDeque::from([Ok(None)]),
            attempts: 0,
        };
        let mounting = mount(&mut client, &mut authorizer, "test-session", |consent| {
            assert!(consent.is_none());
            Ok(())
        });
        let kernel = async {
            let request = request_authorization(&mut server).await;
            complete_mount(&mut server, &request).await;
        };
        let (result, ()) = tokio::join!(mounting, kernel);
        assert_eq!(result, Ok(()));
        assert_eq!(authorizer.attempts, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn consent_timeout_fails_startup_without_acknowledging_authorization() {
        let (mut client, mut server) = sockets().await;
        let mut authorizer = BrowserAuthorization {
            results: VecDeque::new(),
            attempts: 0,
        };
        let mounting = mount_with_timing(
            &mut client,
            &mut authorizer,
            "test-session",
            |_| Ok(()),
            Timing {
                consent: Duration::from_secs(6),
                ..Timing::default()
            },
        );
        let (result, _) = tokio::join!(mounting, request_authorization(&mut server));
        assert!(result.unwrap_err().contains("等待 Drive 授权超时"));
        assert!(tokio::time::timeout(Duration::from_secs(1), server.next())
            .await
            .is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn propagation_failure_does_not_report_a_successful_start() {
        let (mut client, mut server) = sockets().await;
        let mut authorizer = BrowserAuthorization {
            results: VecDeque::from([Ok(Some(CONSENT_URL.into())), Err("授权查询失败".into())]),
            attempts: 0,
        };
        let (result, _) = tokio::join!(
            mount(&mut client, &mut authorizer, "test-session", |_| Ok(())),
            request_authorization(&mut server),
        );
        assert_eq!(result.unwrap_err(), "授权查询失败");
        assert!(tokio::time::timeout(Duration::from_secs(1), server.next())
            .await
            .is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn kernel_failure_ends_browser_wait_immediately() {
        let (mut client, mut server) = sockets().await;
        let mut authorizer = BrowserAuthorization {
            results: VecDeque::new(),
            attempts: 0,
        };
        let kernel = async {
            let request = request_authorization(&mut server).await;
            sleep(Duration::from_secs(2)).await;
            send(&mut server, json!({"header":{"msg_type":"execute_reply"},"parent_header":request["header"],"content":{"status":"error"}})).await;
        };
        let (result, ()) = tokio::join!(
            mount(&mut client, &mut authorizer, "test-session", |_| Ok(())),
            kernel
        );
        assert!(result.unwrap_err().contains("Drive 挂载失败"));
        assert_eq!(authorizer.attempts, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn disconnected_kernel_ends_browser_wait() {
        let (mut client, mut server) = sockets().await;
        let mut authorizer = BrowserAuthorization {
            results: VecDeque::new(),
            attempts: 0,
        };
        let kernel = async {
            request_authorization(&mut server).await;
            server.close(None).await.unwrap();
        };
        let (result, ()) = tokio::join!(
            mount(&mut client, &mut authorizer, "test-session", |_| Ok(())),
            kernel
        );
        assert!(result.unwrap_err().contains("会话提前结束"));
    }
}

use serde_json::{json, Value};
use std::{
    io::{BufRead, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

const IDENTIFIER: &str = "app.clouddeck.desktop";
const NOT_RUNNING: &str = "Cloud Deck 客户端没有运行，或本机 MCP 未开启。请先打开客户端再重试。";

fn data_dir(args: &[String]) -> Option<PathBuf> {
    if let Some(index) = args.iter().position(|item| item == "--data-dir") {
        return args.get(index + 1).map(PathBuf::from);
    }
    if let Some(path) = std::env::var_os("CLOUD_DECK_DATA_DIR") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|path| path.join("Library/Application Support").join(IDENTIFIER))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|path| PathBuf::from(path).join(IDENTIFIER))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|path| path.join(".local/share")))
            .map(|path| path.join(IDENTIFIER))
    }
}

struct Target {
    address: SocketAddr,
    path: String,
    token: String,
}

fn target(root: &PathBuf) -> Option<Target> {
    let text = std::fs::read_to_string(crate::mcp::discovery_path(root)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let url = url::Url::parse(value.get("url")?.as_str()?).ok()?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
        return None;
    }
    Some(Target {
        address: SocketAddr::from(([127, 0, 0, 1], url.port()?)),
        path: url.path().to_owned(),
        token: value.get("token")?.as_str()?.to_owned(),
    })
}

// 返回 None 表示服务端已接受通知（202），无需输出。
fn forward(target: &Target, body: &str) -> Result<Option<String>, String> {
    let mut stream = TcpStream::connect_timeout(&target.address, Duration::from_secs(3))
        .map_err(|_| NOT_RUNNING.to_owned())?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        target.path,
        target.address,
        target.token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| NOT_RUNNING.to_owned())?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|_| "读取本机 MCP 响应失败。".to_owned())?;
    let text = String::from_utf8_lossy(&raw);
    let (head, payload) = text
        .split_once("\r\n\r\n")
        .ok_or("本机 MCP 响应格式无效。")?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("本机 MCP 响应格式无效。")?;
    match status {
        200 => Ok(Some(payload.trim().to_owned())),
        202 => Ok(None),
        401 => Err("本机 MCP 凭据已失效，客户端可能已重启，请重试。".into()),
        _ => Err(format!("本机 MCP 返回 HTTP {status}。")),
    }
}

fn handle_line(root: &Option<PathBuf>, line: &str) -> Option<String> {
    let id = serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| value.get("id").cloned());
    let result = root
        .as_ref()
        .and_then(target)
        .ok_or_else(|| NOT_RUNNING.to_owned())
        .and_then(|target| forward(&target, line));
    match result {
        Ok(output) => output,
        Err(message) => id.map(|id| {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}})
                .to_string()
        }),
    }
}

pub fn run(args: &[String]) -> i32 {
    let root = Arc::new(data_dir(args));
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let root = root.clone();
        let output = output.clone();
        workers.retain(|worker| !worker.is_finished());
        workers.push(std::thread::spawn(move || {
            if let Some(reply) = handle_line(&root, &line) {
                let mut stdout = output.lock().unwrap_or_else(|error| error.into_inner());
                let _ = writeln!(stdout, "{reply}");
                let _ = stdout.flush();
            }
        }));
    }
    for worker in workers {
        let _ = worker.join();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_data_dir_wins() {
        let args = vec!["--data-dir".to_owned(), "/x/cloud".to_owned()];
        assert_eq!(data_dir(&args), Some(PathBuf::from("/x/cloud")));
    }

    #[test]
    fn missing_client_returns_error_only_for_requests() {
        let root = Some(PathBuf::from("/nonexistent/cloud-deck-mcp-test"));
        let reply = handle_line(&root, r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#)
            .expect("request needs a reply");
        let value: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(value["error"]["code"], -32000);
        assert!(handle_line(
            &root,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .is_none());
    }
}

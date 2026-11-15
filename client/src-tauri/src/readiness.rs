use serde_json::{json, Value};

pub fn resolve(
    kind: &str,
    value: Option<&Value>,
    remote: Option<u16>,
    legacy: Option<&str>,
) -> Result<Option<Value>, String> {
    if value.is_none_or(Value::is_null)
        && kind == "service"
        && legacy.is_some_and(|path| {
            !path.starts_with('/')
                || path.starts_with("//")
                || path.len() > 512
                || path.contains(['\0', '\r', '\n'])
        })
    {
        return Err("健康检查路径必须以 / 开头。".into());
    }
    let value = value.filter(|value| !value.is_null()).cloned().or_else(|| {
        (kind == "service").then(|| remote.zip(legacy).map(|(port, path)| json!({"type":"http", "url":format!("http://127.0.0.1:{port}{path}")}))).flatten()
    });
    if kind == "task" {
        return if value.is_none() {
            Ok(None)
        } else {
            Err("一次性任务使用退出码判断完成，不能配置 readiness。".into())
        };
    }
    let check = value.ok_or("服务需要 readiness 启动就绪检查。")?;
    for (field, maximum) in [
        ("timeoutSeconds", 86400),
        ("intervalSeconds", 300),
        ("probeTimeoutSeconds", 30),
    ] {
        if check.get(field).is_some_and(|value| {
            !value
                .as_u64()
                .is_some_and(|number| (1..=maximum).contains(&number))
        }) {
            return Err(format!("readiness.{field} 必须为 1–{maximum} 的整数。"));
        }
    }
    match check["type"].as_str() {
        Some("http") => {
            let address = check["url"].as_str().ok_or("HTTP 就绪检查需要 url。")?;
            let parsed = url::Url::parse(address).map_err(|_| "HTTP 就绪检查地址无效。")?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || address.contains(['\0', '\r', '\n'])
            {
                return Err("就绪检查需要不含账号密码的 HTTP(S) URL。".into());
            }
            if check.get("status").is_some_and(|status| {
                !status
                    .as_u64()
                    .is_some_and(|number| (100..=599).contains(&number))
            }) {
                return Err("HTTP 预期状态码必须为 100–599。".into());
            }
        }
        Some("tcp") => {
            if !check["host"].as_str().is_some_and(|host| {
                !host.is_empty()
                    && host.len() <= 253
                    && !host.chars().any(|character| {
                        character.is_whitespace() || character == '/' || character == '\0'
                    })
            }) || !check["port"]
                .as_u64()
                .is_some_and(|port| (1..=65535).contains(&port))
            {
                return Err("TCP 就绪检查需要有效的 host 和 port。".into());
            }
        }
        Some("command") => {
            if !check["command"].as_str().is_some_and(|command| {
                !command.trim().is_empty() && command.len() <= 4000 && !command.contains('\0')
            }) {
                return Err("命令就绪检查需要有效的 command。".into());
            }
            if check.get("workingDirectory").is_some_and(|value| {
                !value.as_str().is_some_and(|directory| {
                    directory.starts_with('/')
                        && directory.len() <= 512
                        && !directory.contains(['\0', '\r', '\n'])
                        && !directory.split('/').any(|part| part == "..")
                })
            }) {
                return Err("就绪检查工作目录必须为云端绝对路径。".into());
            }
        }
        _ => return Err("readiness.type 只能是 http、tcp 或 command。".into()),
    }
    Ok(Some(check))
}

/// Only a probe aimed at the forwarded loopback HTTP service supplies a browser entry.
pub fn http_path(check: Option<&Value>, remote: Option<u16>) -> Option<String> {
    let check = check?;
    if check["type"] != "http" {
        return None;
    }
    let address = url::Url::parse(check["url"].as_str()?).ok()?;
    if address.scheme() != "http"
        || !matches!(address.host_str(), Some("127.0.0.1" | "localhost"))
        || address.port_or_known_default() != remote
    {
        return None;
    }
    Some(format!(
        "{}{}",
        address.path(),
        address
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_generic_checks_and_legacy_http_migration() {
        for check in [
            json!({"type":"http","url":"http://127.0.0.1:8188/ready","status":204}),
            json!({"type":"tcp","host":"127.0.0.1","port":8188}),
            json!({"type":"command","command":"bash ready.sh"}),
        ] {
            assert_eq!(
                resolve("service", Some(&check), None, None).unwrap(),
                Some(check)
            );
        }
        let check = resolve("service", None, Some(8188), Some("/ready")).unwrap();
        assert_eq!(http_path(check.as_ref(), Some(8188)), Some("/ready".into()));
        assert_eq!(http_path(check.as_ref(), Some(9000)), None);
    }
    #[test]
    fn refuses_invalid_or_missing_checks_and_task_readiness() {
        for check in [
            json!(null),
            json!({"type":"command","command":" "}),
            json!({"type":"tcp","host":"localhost","port":0}),
            json!({"type":"http","url":"file:///ready"}),
            json!({"type":"command","command":"true","probeTimeoutSeconds":31}),
        ] {
            assert!(resolve("service", Some(&check), None, None).is_err());
        }
        assert!(resolve(
            "task",
            Some(&json!({"type":"command","command":"true"})),
            None,
            None
        )
        .is_err());
        assert!(resolve("task", None, None, None).unwrap().is_none());
    }
}

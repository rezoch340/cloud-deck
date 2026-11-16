use tauri::{utils::config::FrontendDist, Context, Runtime};

pub fn check<R: Runtime>(context: &Context<R>) -> Result<(), String> {
    require_directory(context.config().build.frontend_dist.as_ref())?;
    let assets = context.assets();
    let index = assets
        .get(&"index.html".into())
        .ok_or("发行包缺少内嵌 index.html。")?;
    let html = std::str::from_utf8(&index).map_err(|_| "内嵌 index.html 编码无效。")?;
    let mut script_found = false;
    let mut style_found = false;
    for (asset_key, _) in assets.iter() {
        let asset_path = asset_key.as_ref();
        if !asset_path.ends_with(".js") && !asset_path.ends_with(".css") {
            continue;
        }
        if html.contains(asset_path.trim_start_matches('/')) {
            if assets
                .get(&asset_path.into())
                .is_none_or(|content| content.is_empty())
            {
                return Err(format!("发行包缺少可读取的界面资源：{asset_path}"));
            }
            script_found |= asset_path.ends_with(".js");
            style_found |= asset_path.ends_with(".css");
        }
    }
    if !script_found || !style_found {
        return Err("发行包没有同时嵌入首页引用的 JavaScript 和 CSS。".into());
    }
    Ok(())
}

fn require_directory(frontend: Option<&FrontendDist>) -> Result<(), String> {
    match frontend {
        Some(FrontendDist::Directory(_)) => Ok(()),
        _ => Err("发行包必须从本地目录嵌入界面，frontendDist 不能是 URL。".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_drive_paths_are_rejected_as_external_urls() {
        // 使用 Tauri 实际的反序列化规则复现 Windows 构建路径误判。
        for path in [r"D:\a\_temp\cloud-deck-work\dist", "D:/build/dist"] {
            let frontend: FrontendDist = serde_json::from_value(serde_json::json!(path)).unwrap();
            assert!(matches!(frontend, FrontendDist::Url(_)));
            assert!(require_directory(Some(&frontend)).is_err());
        }
    }

    #[test]
    fn project_frontend_path_embeds_a_directory() {
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            config.build.frontend_dist,
            Some(FrontendDist::Directory("../dist".into()))
        );
        assert!(require_directory(config.build.frontend_dist.as_ref()).is_ok());
        assert!(require_directory(None).is_err());
    }
}

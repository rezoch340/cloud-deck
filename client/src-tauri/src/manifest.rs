use crate::model::{self, Recipe, RecipeDraft};
use serde_json::{json, Value};

fn path(value: &Value) -> Result<&str, String> {
    value
        .as_str()
        .filter(|value| {
            value.starts_with('/')
                && *value != "/"
                && value.len() <= 512
                && !value.contains(['\0', '\n', '\r'])
                && !value.split('/').any(|part| part == "..")
        })
        .ok_or_else(|| "清单目录必须是明确的云端绝对路径。".into())
}

fn drive_path(value: &Value) -> Result<&str, String> {
    path(value)?
        .strip_prefix("/content/drive/MyDrive/")
        .filter(|relative| !relative.is_empty())
        .ok_or_else(|| "云盘路径必须位于 Google Drive /MyDrive 下。".into())
}

fn instance_path(value: &Value) -> Result<&str, String> {
    let value = path(value)?;
    if value == "/content/drive" || value.starts_with("/content/drive/") {
        return Err("实例本地目标不能位于云盘挂载内。".into());
    }
    Ok(value)
}

fn list<'value>(value: &'value Value, label: &str) -> Result<&'value Vec<Value>, String> {
    value
        .as_array()
        .filter(|items| items.len() <= 128)
        .ok_or_else(|| format!("清单 {label} 必须是最多 128 项的列表。"))
}

pub fn recipe(manifest: Value, current: &[Recipe], builtin: bool) -> Result<Recipe, String> {
    if manifest["schemaVersion"] != 1 || manifest.to_string().len() > 512 * 1024 {
        return Err("应用清单版本必须为 1，大小不能超过 512 KiB。".into());
    }
    let identifier = manifest["id"]
        .as_str()
        .filter(|identifier| {
            !identifier.is_empty()
                && identifier.len() <= 80
                && identifier.as_bytes()[0].is_ascii_alphanumeric()
                && identifier.bytes().all(|character| {
                    character.is_ascii_alphanumeric() || b"_-".contains(&character)
                })
        })
        .ok_or("清单应用标识只能包含字母、数字、下划线和连字符。")?;
    let runtime = &manifest["runtime"];
    if manifest
        .get("details")
        .is_some_and(|details| !details.as_str().is_some_and(|text| text.len() <= 64 * 1024))
    {
        return Err("details 必须为不超过 64 KiB 的 Markdown 描述。".into());
    }
    if !runtime["environment"].is_object() {
        return Err("environment 必须为对象。".into());
    }
    if manifest.get("installation").is_some() || runtime.get("usePreparedLaunch").is_some() {
        return Err("请使用 command 执行安装和启动命令或脚本。".into());
    }
    let commands = list(&manifest["startup"]["commands"], "startup.commands")?;
    for step in commands {
        if !step["command"].as_str().is_some_and(|command| {
            !command.trim().is_empty() && command.len() <= 4000 && !command.contains('\0')
        }) {
            return Err("清单启动准备命令无效。".into());
        }
        if step["workingDirectory"] != "/" {
            path(&step["workingDirectory"])?;
        }
    }
    for rule in list(&manifest["startup"]["restore"], "startup.restore")? {
        drive_path(&rule["source"])?;
        instance_path(&rule["target"])?;
        if !rule["optional"].is_null() && !rule["optional"].is_boolean() {
            return Err("optional 必须为布尔值。".into());
        }
    }
    let save = list(&manifest["shutdown"]["save"], "shutdown.save")?;
    let mut save_sources = std::collections::BTreeSet::new();
    for rule in save {
        if !save_sources.insert(instance_path(&rule["source"])?) {
            return Err("保存规则的来源目录不能重复。".into());
        }
        drive_path(&rule["target"])?;
    }
    let mut targets = std::collections::BTreeSet::new();
    for resource in list(&manifest["resources"]["files"], "resources.files")? {
        if !targets.insert(instance_path(&resource["target"])?) {
            return Err("资源目标不能重复。".into());
        }
        if !resource["size"].as_u64().is_some_and(|size| size > 0)
            || !resource["sha256"].as_str().is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|character| character.is_ascii_hexdigit())
            })
        {
            return Err("资源必须包含大小与完整 SHA-256。".into());
        }
        let sources = list(&resource["sources"], "resource.sources")?;
        if sources.is_empty() {
            return Err("资源至少需要一个来源。".into());
        }
        for source in sources {
            match source["kind"].as_str() {
                Some("drive") => {
                    drive_path(&source["path"])?;
                }
                Some("download") => {
                    let address = source["url"].as_str().ok_or("下载地址无效。")?;
                    let address = url::Url::parse(address).map_err(|_| "下载地址无效。")?;
                    if address.scheme() != "https"
                        || address.host_str().is_none()
                        || !address.username().is_empty()
                        || address.password().is_some()
                    {
                        return Err("资源下载地址必须为不含账号密码的 HTTPS URL。".into());
                    }
                }
                _ => return Err("资源来源只能是 download 或 drive。".into()),
            }
        }
    }
    let storage = save
        .first()
        .and_then(|rule| rule["target"].as_str())
        .and_then(|target| target.rsplit_once('/').map(|(parent, _)| parent.to_owned()))
        .unwrap_or_else(|| format!("/content/drive/MyDrive/CloudDeck/{identifier}"));
    let draft: RecipeDraft = serde_json::from_value(json!({
        "id": identifier, "name": manifest["name"], "description": manifest["description"].as_str().unwrap_or(""), "kind": manifest["kind"],
        "command": runtime["command"], "workingDirectory": runtime["workingDirectory"], "environment": runtime["environment"].as_object().cloned().unwrap_or_default(),
        "remotePort":runtime["remotePort"], "localPort":runtime["localPort"], "healthPath":runtime["healthPath"], "readiness":runtime["readiness"],
        "saveDirectories":save.iter().map(|rule| rule["source"].clone()).collect::<Vec<_>>(), "storageDirectory":storage, "autoStart":false
    })).map_err(|_| "应用清单的运行参数无效。")?;
    let mut recipe = model::validate(
        draft,
        current,
        if builtin {
            "bundled-application".into()
        } else {
            identifier.into()
        },
    )?;
    recipe.id = identifier.into();
    recipe.builtin = builtin;
    recipe.prepare_command = (!commands.is_empty()).then(|| {
        commands
            .iter()
            .filter_map(|step| step["command"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    });
    recipe.manifest = Some(manifest);
    Ok(recipe)
}

pub fn from_recipe(recipe: &Recipe) -> Value {
    let readiness = crate::readiness::resolve(
        &recipe.kind,
        recipe.readiness.as_ref(),
        recipe.remote_port,
        recipe.health_path.as_deref(),
    )
    .unwrap_or(None);
    let save: Vec<Value> = recipe.save_directories.iter().map(|source| {
        let name = source.rsplit('/').next().unwrap_or("files");
        json!({"source":source,"target":format!("{}/{name}", recipe.storage_directory.trim_end_matches('/'))})
    }).collect();
    let restore: Vec<Value> = save
        .iter()
        .map(|rule| json!({"source":rule["target"],"target":rule["source"],"optional":true}))
        .collect();
    let commands: Vec<Value> = recipe
        .prepare_command
        .iter()
        .map(|command| json!({"command":command,"workingDirectory":"/"}))
        .collect();
    json!({"schemaVersion":1,"id":recipe.id,"name":recipe.name,"description":recipe.description,"kind":recipe.kind,
        "runtime":{"command":recipe.command,"workingDirectory":recipe.working_directory,"environment":recipe.environment,"remotePort":recipe.remote_port,"localPort":recipe.local_port,"readiness":readiness},
        "resources":{"files":[]},"startup":{"restore":restore,"commands":commands},"shutdown":{"save":save}})
}

pub fn requires_drive(manifest: &Value) -> bool {
    manifest["startup"]["restore"]
        .as_array()
        .is_some_and(|rules| !rules.is_empty())
        || manifest["shutdown"]["save"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty())
        || manifest["resources"]["files"]
            .as_array()
            .is_some_and(|files| {
                files.iter().any(|resource| {
                    resource["sources"].as_array().is_some_and(|sources| {
                        sources.iter().any(|source| source["kind"] == "drive")
                    })
                })
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn builtin_manifest() -> Value {
        serde_json::from_str(include_str!("../../applications/comfyui/manifest.json")).unwrap()
    }
    #[test]
    fn comfyui_manifest_keeps_pinned_resources_and_external_first_fallback() {
        let manifest = builtin_manifest();
        let baseline: Value = serde_json::from_str(include_str!("../../../manifest.json")).unwrap();
        assert!(manifest.get("installation").is_none());
        assert!(manifest["resources"].get("documents").is_none());
        for (resource, original) in manifest["resources"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .zip(baseline["models"].as_array().unwrap())
        {
            assert_eq!(resource["sha256"], original["sha256"]);
            assert_eq!(resource["size"], original["size"]);
        }
        assert_eq!(
            manifest["resources"]["files"][0]["sources"][0]["kind"],
            "download"
        );
        assert_eq!(
            manifest["resources"]["files"][0]["sources"][1]["kind"],
            "drive"
        );
        let recipe = recipe(manifest, &[], true).unwrap();
        assert!(!recipe.auto_start);
        assert_eq!(recipe.save_directories.len(), 3);
        assert!(requires_drive(recipe.manifest.as_ref().unwrap()));
    }
    #[test]
    fn invalid_save_targets_and_unverified_downloads_are_rejected() {
        let mut manifest = builtin_manifest();
        manifest["shutdown"]["save"][0]["target"] = json!("/etc/config");
        assert!(recipe(manifest, &[], true).is_err());
        let mut manifest = builtin_manifest();
        manifest["resources"]["files"][0]["sha256"] = json!("");
        assert!(recipe(manifest, &[], true).is_err());
    }
}

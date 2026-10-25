//! Metadata adapters for launchers whose game directories are already isolated.
use super::{ExternalInstance, ExternalRuntime, invalid, profile, read_json};
use crate::{InstanceKind, LauncherError, LoaderKind};
use serde_json::Value;
use std::path::Path;

pub(super) fn probe(root: &Path) -> Result<Option<ExternalInstance>, LauncherError> {
    if root.join("minecraftinstance.json").is_file() {
        let v = read_json(&root.join("minecraftinstance.json"))?;
        let mc = required(&v["gameVersion"], "CurseForge gameVersion")?;
        let (loader, loader_version) = match v["baseModLoader"]["name"].as_str() {
            None => (LoaderKind::Vanilla, None),
            Some(name) => {
                let (kind, version) = name
                    .split_once('-')
                    .ok_or_else(|| invalid("invalid CurseForge baseModLoader name"))?;
                let kind: LoaderKind = kind.to_ascii_lowercase().parse()?;
                let version = version.strip_prefix(&format!("{mc}-")).unwrap_or(version);
                let version = version.strip_suffix(&format!("-{mc}")).unwrap_or(version);
                (kind, Some(version.into()))
            }
        };
        return Ok(Some(candidate(
            root,
            root,
            "curseforge",
            required(&v["name"], "CurseForge name")?,
            ExternalRuntime {
                minecraft: mc,
                loader,
                loader_version,
            },
        )));
    }
    if root.join("instance.json").is_file() {
        let v = read_json(&root.join("instance.json"))?;
        if v["game_configuration"].is_object() {
            if v["_version"].as_str() != Some("1") {
                return Err(invalid("unsupported GDLauncher instance schema"));
            }
            let version = &v["game_configuration"]["version"];
            let minecraft = required(
                &version["release"],
                "GDLauncher standard game version (custom runtime is unsupported)",
            )?;
            let mut loaders = Vec::new();
            if let Some(values) = version["modloaders"].as_array() {
                for value in values {
                    loaders.push((
                        required(&value["type"], "GDLauncher loader type")?
                            .to_ascii_lowercase()
                            .parse::<LoaderKind>()?,
                        required(&value["version"], "GDLauncher loader version")?,
                    ));
                }
            }
            if loaders.len() > 1 {
                return Err(invalid("GDLauncher instance has multiple loaders"));
            }
            let (loader, loader_version) = loaders
                .pop()
                .map(|(k, v)| (k, Some(v)))
                .unwrap_or((LoaderKind::Vanilla, None));
            return Ok(Some(candidate(
                root,
                &root.join("instance"),
                "gdlauncher",
                required(&v["name"], "GDLauncher name")?,
                ExternalRuntime {
                    minecraft,
                    loader,
                    loader_version,
                },
            )));
        }
        if v["launcher"].is_object() {
            let runtime = profile::runtime(
                root,
                required(&v["id"], "ATLauncher Minecraft id")?.as_str(),
                &v,
            )?;
            return Ok(Some(candidate(
                root,
                root,
                "atlauncher",
                required(&v["launcher"]["name"], "ATLauncher name")?,
                runtime,
            )));
        }
        return Err(invalid(
            "unrecognised instance.json schema; not a GDLauncher standard or ATLauncher instance",
        ));
    }
    Ok(None)
}

fn required(value: &Value, field: &str) -> Result<String, LauncherError> {
    value
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("missing {field}")))
}

fn candidate(
    root: &Path,
    game: &Path,
    format: &str,
    name: String,
    runtime: ExternalRuntime,
) -> ExternalInstance {
    ExternalInstance { id: format.into(), name, format: format.into(), kind: InstanceKind::Client,
        profile_directory: root.into(), game_directory: game.is_dir().then(|| game.to_path_buf()), runtime: Some(runtime),
        directory_evidence: format!("{format} isolated instance metadata"), warnings: vec!["Runtime is rebuilt from official metadata. Launcher accounts, custom patches, JVM arguments and launch hooks are not imported.".into()], error: None }
}

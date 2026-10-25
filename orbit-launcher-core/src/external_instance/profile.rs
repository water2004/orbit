use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use super::{ExternalInstance, ExternalRuntime, directories, invalid, read_json};
use crate::{InstanceKind, LauncherError, LoaderKind};

pub(super) fn standard(repository: &Path, directory: &Path, id: &str) -> ExternalInstance {
    let mut result = ExternalInstance {
        id: id.into(),
        name: id.into(),
        format: "minecraft".into(),
        kind: InstanceKind::Client,
        profile_directory: directory.to_path_buf(),
        game_directory: None,
        runtime: None,
        directory_evidence: String::new(),
        warnings: Vec::new(),
        error: None,
    };
    let parsed = (|| {
        let profile = read_json(&directory.join(format!("{id}.json")))?;
        result.runtime = Some(runtime(repository, id, &profile)?);
        let (format, game, evidence) = directories::resolve(repository, directory, id)?;
        result.format = format;
        result.game_directory = game;
        result.directory_evidence = evidence;
        Ok::<_, LauncherError>(())
    })();
    if let Err(e) = parsed {
        result.error = Some(e.to_string());
    }
    result.warnings.push("Accounts, tokens, launcher hooks and custom JVM arguments are not imported. Select an Orbit account and Java after import.".into());
    result
}

pub(super) fn runtime(
    repository: &Path,
    id: &str,
    profile: &Value,
) -> Result<ExternalRuntime, LauncherError> {
    let mut minecraft = BTreeSet::new();
    if let Some(version) = minecraft_jar_version(repository, id, profile)? {
        minecraft.insert(version);
    }
    let mut loaders = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![(id.to_string(), profile.clone())];
    while let Some((id, value)) = pending.pop() {
        if !seen.insert(id.clone()) || seen.len() > 32 {
            return Err(invalid("cyclic or excessive version inheritance"));
        }
        for key in ["clientVersion", "gameVersion"] {
            if let Some(v) = value[key].as_str() {
                minecraft.insert(v.to_string());
            }
        }
        let patches = value["patches"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        for node in std::iter::once(&value).chain(patches.iter()) {
            if node["id"].as_str() == Some("game")
                && let Some(v) = node["version"].as_str()
            {
                minecraft.insert(v.into());
            }
            if node["downloads"]["client"].is_object()
                && patches.is_empty()
                && value["inheritsFrom"].is_null()
                && minecraft.is_empty()
                && let Some(v) = node["id"].as_str().filter(|v| *v != "game")
            {
                minecraft.insert(v.into());
            }
            if let Some(libraries) = node["libraries"].as_array() {
                for lib in libraries {
                    if let Some(name) = lib["name"].as_str() {
                        loader_coordinate(name, &mut loaders)?;
                    }
                }
            }
        }
        if let Some(parent) = value["inheritsFrom"].as_str() {
            crate::layout::validate_directory_name(parent).map_err(invalid)?;
            let path = repository
                .join("versions")
                .join(parent)
                .join(format!("{parent}.json"));
            if path.is_file() {
                pending.push((parent.into(), read_json(&path)?));
            } else {
                minecraft.insert(parent.into());
            }
        } else if minecraft.is_empty()
            && value["mainClass"].as_str() == Some("net.minecraft.client.main.Main")
        {
            minecraft.insert(id);
        }
    }
    if minecraft.len() != 1 {
        return Err(invalid(format!(
            "expected one exact Minecraft version, found {minecraft:?}"
        )));
    }
    if loaders.len() > 1 {
        return Err(invalid(format!("multiple loader identities: {loaders:?}")));
    }
    let (loader, loader_version) = if let Some((kind, version)) = loaders.into_iter().next() {
        (kind.parse()?, Some(version))
    } else {
        // Do not silently turn unrecognised custom runtimes (OptiFine etc.) into Vanilla.
        let main = profile["mainClass"].as_str().unwrap_or_default();
        if !matches!(
            main,
            "" | "net.minecraft.client.main.Main" | "net.minecraft.launchwrapper.Launch"
        ) || main.contains("launchwrapper")
        {
            return Err(invalid(format!("unsupported client entrypoint '{main}'")));
        }
        (LoaderKind::Vanilla, None)
    };
    Ok(ExternalRuntime {
        minecraft: minecraft.into_iter().next().expect("one version checked"),
        loader,
        loader_version,
    })
}

fn minecraft_jar_version(
    repository: &Path,
    id: &str,
    profile: &Value,
) -> Result<Option<String>, LauncherError> {
    use std::io::Read;
    let jar_id = profile["jar"].as_str().unwrap_or(id);
    crate::layout::validate_directory_name(jar_id).map_err(invalid)?;
    let jar = repository
        .join("versions")
        .join(jar_id)
        .join(format!("{jar_id}.jar"));
    if !jar.is_file() {
        return Ok(None);
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&jar)?)
        .map_err(|e| invalid(format!("{}: {e}", jar.display())))?;
    let file = match zip.by_name("version.json") {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(invalid(e.to_string())),
    };
    if file.size() > 1024 * 1024 {
        return Err(invalid("Minecraft version.json exceeds 1 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    value["id"]
        .as_str()
        .map(|v| Some(v.to_string()))
        .ok_or_else(|| invalid("Minecraft JAR version.json has no id"))
}

fn loader_coordinate(
    coordinate: &str,
    found: &mut BTreeSet<(String, String)>,
) -> Result<(), LauncherError> {
    let parts: Vec<_> = coordinate.split(':').collect();
    if parts.len() < 3 {
        return Ok(());
    }
    let kind = match (parts[0], parts[1]) {
        ("net.fabricmc", "fabric-loader") => "fabric",
        ("org.quiltmc", "quilt-loader") => "quilt",
        ("net.minecraftforge", "forge") => "forge",
        ("net.neoforged", "neoforge" | "forge") => "neoforge",
        _ => return Ok(()),
    };
    let version = if kind == "forge" || parts[1] == "forge" {
        parts[2]
            .split_once('-')
            .map(|(_, v)| v)
            .ok_or_else(|| invalid("Forge coordinate has no Minecraft prefix"))?
    } else {
        parts[2]
    };
    found.insert((kind.into(), version.into()));
    Ok(())
}

pub(super) fn prism(root: &Path) -> Result<ExternalInstance, LauncherError> {
    let pack = read_json(&root.join("mmc-pack.json"))?;
    if pack["formatVersion"].as_u64() != Some(1) {
        return Err(invalid("unsupported Prism/MultiMC formatVersion"));
    }
    let components = pack["components"]
        .as_array()
        .ok_or_else(|| invalid("mmc-pack.json has no components"))?;
    let mut seen = BTreeSet::new();
    for component in components {
        let uid = component["uid"]
            .as_str()
            .ok_or_else(|| invalid("Prism component has no uid"))?;
        if !seen.insert(uid) {
            return Err(invalid(format!("duplicate Prism component '{uid}'")));
        }
        if !matches!(
            uid,
            "net.minecraft"
                | "org.lwjgl"
                | "org.lwjgl3"
                | "net.fabricmc.intermediary"
                | "net.fabricmc.fabric-loader"
                | "org.quiltmc.quilt-loader"
                | "net.minecraftforge"
                | "net.neoforged"
        ) {
            return Err(invalid(format!(
                "unsupported custom Prism runtime component '{uid}'; refusing to silently omit it"
            )));
        }
    }
    let version = |uid: &str| {
        components
            .iter()
            .find(|c| c["uid"].as_str() == Some(uid))
            .and_then(|c| c["version"].as_str())
            .map(str::to_string)
    };
    let minecraft = version("net.minecraft")
        .ok_or_else(|| invalid("Prism instance has no net.minecraft version"))?;
    let mut loaders = Vec::new();
    for (uid, kind) in [
        ("net.fabricmc.fabric-loader", LoaderKind::Fabric),
        ("org.quiltmc.quilt-loader", LoaderKind::Quilt),
        ("net.minecraftforge", LoaderKind::Forge),
        ("net.neoforged", LoaderKind::Neoforge),
    ] {
        if let Some(v) = version(uid) {
            loaders.push((kind, v));
        }
    }
    if loaders.len() > 1 {
        return Err(invalid("Prism instance has multiple loaders"));
    }
    let (loader, loader_version) = loaders
        .pop()
        .map(|(k, v)| (k, Some(v)))
        .unwrap_or((LoaderKind::Vanilla, None));
    let game_directories: Vec<_> = [".minecraft", "minecraft"]
        .iter()
        .map(|p| root.join(p))
        .filter(|p| p.is_dir())
        .collect();
    let game_directory = match game_directories.as_slice() {
        [p] => Some(dunce::canonicalize(p)?),
        _ => None,
    };
    let settings = directories::read_ini(&root.join("instance.cfg"))?;
    Ok(ExternalInstance {
        id: "prism".into(), name: settings.get("name").cloned().unwrap_or_else(|| root.file_name().unwrap_or_default().to_string_lossy().into()),
        format: "prism".into(), kind: InstanceKind::Client, profile_directory: root.to_path_buf(), game_directory,
        runtime: Some(ExternalRuntime { minecraft, loader, loader_version }),
        directory_evidence: "Prism/MultiMC mmc-pack.json and instance game directory".into(),
        warnings: vec!["Custom components, patches, accounts and launch hooks are not imported; runtime is rebuilt from official metadata.".into()], error: None,
    })
}

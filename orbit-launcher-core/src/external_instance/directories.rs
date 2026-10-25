use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{invalid, read_json};
use crate::LauncherError;

pub(super) fn resolve(
    repository: &Path,
    version: &Path,
    id: &str,
) -> Result<(String, Option<PathBuf>, String), LauncherError> {
    let mut evidence = Vec::new();
    let mut formats = Vec::new();
    let mut paths = Vec::new();
    let hmcl = version.join("hmclversion.cfg");
    let modern_hmcl = version.join(".hmcl/config/instance-game-settings.json");
    if version.join("modpack.cfg").is_file() {
        formats.push("hmcl");
        paths.push(version.to_path_buf());
        evidence.push("HMCL modpack forces version isolation");
    } else if modern_hmcl.is_file() {
        formats.push("hmcl");
        let settings = read_json(&modern_hmcl)?;
        if settings["overrideProperties"]
            .as_array()
            .is_some_and(|p| p.iter().any(|v| v.as_str() == Some("runningDirectory")))
        {
            let custom = settings["runningDirectory"].as_str().unwrap_or_default();
            paths.push(if custom.is_empty() {
                version.to_path_buf()
            } else {
                absolute(custom)?
            });
            evidence.push("HMCL instance runningDirectory override");
        } else {
            evidence
                .push("HMCL inherits an external global preset; select the actual game directory");
        }
    } else if hmcl.is_file() {
        formats.push("hmcl");
        let settings = read_json(&hmcl)?;
        // Modpacks force isolation in HMCL, independent of ordinary settings.
        if settings["usesGlobal"].as_bool().unwrap_or(true) {
            evidence.push("HMCL usesGlobal selects settings outside this instance; select the actual game directory");
        } else {
            let mode = settings["gameDirType"]
                .as_i64()
                .map(|n| n.to_string())
                .or_else(|| settings["gameDirType"].as_str().map(str::to_string))
                .or_else(|| Some("0".into()));
            match mode.as_deref() {
                Some("1" | "VERSION_FOLDER") => { paths.push(version.to_path_buf()); evidence.push("HMCL VERSION_FOLDER"); }
                Some("2" | "CUSTOM") => { paths.push(absolute(settings["gameDir"].as_str().unwrap_or_default())?); evidence.push("HMCL CUSTOM gameDir"); }
                Some("0" | "ROOT_FOLDER") if settings["usesGlobal"].as_bool() == Some(false) => {
                    paths.push(repository.to_path_buf()); evidence.push("HMCL local ROOT_FOLDER");
                }
                _ => evidence.push("HMCL global settings are outside this instance; select the actual game directory"),
            }
        }
    }
    let pcl = version.join("PCL/Setup.ini");
    if pcl.is_file() {
        formats.push("pcl");
        let settings = read_key_values(&pcl, ':')?;
        if let Some(value) = settings.get("VersionArgumentIndieV2") {
            let isolated = match value.to_ascii_lowercase().as_str() {
                "true" | "1" => true,
                "false" | "0" => false,
                _ => return Err(invalid("invalid PCL VersionArgumentIndieV2 boolean")),
            };
            paths.push(if isolated { version } else { repository }.to_path_buf());
            evidence.push("PCL VersionArgumentIndieV2");
        } else {
            match settings.get("VersionArgumentIndie").map(String::as_str) {
                Some("1") => { paths.push(version.to_path_buf()); evidence.push("PCL explicit version isolation"); }
                Some("2") => { paths.push(repository.to_path_buf()); evidence.push("PCL explicit shared directory"); }
                _ => evidence.push("PCL global/default isolation is not persisted here; select the actual game directory"),
            }
        }
    }
    if formats.is_empty() {
        // The official launcher's profiles explicitly bind version IDs to gameDir.
        let file = repository.join("launcher_profiles.json");
        if file.is_file() {
            let profiles = read_json(&file)?;
            if let Some(profiles) = profiles["profiles"].as_object() {
                for profile in profiles
                    .values()
                    .filter(|p| p["lastVersionId"].as_str() == Some(id))
                {
                    paths.push(match profile["gameDir"].as_str() {
                        Some(p) => absolute(p)?,
                        None => repository.to_path_buf(),
                    });
                }
            }
            evidence.push(
                "Official launcher profile gameDir (multiple differing profiles require a choice)",
            );
        } else {
            evidence.push("Version JSON does not define the working directory; select the actual game directory");
        }
    }
    let paths = paths
        .into_iter()
        .map(dunce::canonicalize)
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    let game = if paths.len() == 1 {
        paths.into_iter().next()
    } else {
        None
    };
    formats.sort_unstable();
    formats.dedup();
    Ok((
        if formats.is_empty() {
            "minecraft".into()
        } else {
            formats.join("+")
        },
        game,
        evidence.join("; "),
    ))
}

fn absolute(value: &str) -> Result<PathBuf, LauncherError> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(invalid(
            "custom game directory is relative or empty; select an explicit absolute game directory",
        ));
    }
    Ok(path)
}

pub(super) fn read_ini(path: &Path) -> Result<BTreeMap<String, String>, LauncherError> {
    read_key_values(path, '=')
}

fn read_key_values(
    path: &Path,
    separator: char,
) -> Result<BTreeMap<String, String>, LauncherError> {
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("launcher INI exceeds 1 MiB"));
    }
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return Err(invalid("invalid UTF-16 INI"));
        }
        let words: Vec<_> = bytes[2..]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16(&words).map_err(|e| invalid(e.to_string()))?
    } else {
        String::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?
    };
    let mut values = BTreeMap::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with([';', '#', '[']) {
            continue;
        }
        if let Some((key, value)) = line.split_once(separator) {
            values.insert(key.trim().into(), value.trim().into());
        }
    }
    Ok(values)
}

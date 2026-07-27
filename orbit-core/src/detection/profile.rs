use std::path::Path;

use super::{Confidence, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::LoaderKind;
use crate::metadata::version_profile::VersionProfile;

pub(super) struct ProfileSignature {
    pub group: &'static str,
    pub artifacts: &'static [&'static str],
    pub main_class_markers: &'static [&'static str],
    pub component_uids: &'static [&'static str],
}

pub(super) fn detect_profile_loader(
    instance_dir: &Path,
    mc_version: Option<&str>,
    loader: LoaderKind,
    signature: &ProfileSignature,
) -> Result<LoaderInfo, OrbitError> {
    let layout = crate::launcher::LauncherLayout::discover(instance_dir)?;
    let mut versions = Vec::new();
    let mut evidence = Vec::new();

    for component in &layout.components {
        if signature.component_uids.contains(&component.uid.as_str()) {
            versions.push(component.version.clone());
            evidence.push(format!(
                "found {} {} in mmc-pack.json",
                component.uid, component.version
            ));
        }
    }

    for profile_path in &layout.profile_paths {
        let Some(scan) = scan_profile(profile_path, mc_version, signature) else {
            continue;
        };
        if let Some(version) = scan.version {
            versions.push(version);
        }
        evidence.extend(scan.evidence);
    }

    versions.sort();
    versions.dedup();
    evidence.sort();
    evidence.dedup();
    Ok(LoaderInfo {
        loader,
        confidence: if !versions.is_empty() {
            Confidence::Certain
        } else if evidence.is_empty() {
            Confidence::None
        } else {
            Confidence::Low
        },
        versions,
        evidence,
    })
}

pub(super) fn strip_minecraft_version_prefix(version: String) -> String {
    version
        .split_once('-')
        .filter(|(minecraft, loader)| {
            minecraft.contains('.')
                && minecraft
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
                && loader
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
        })
        .map(|(_, loader)| loader.to_string())
        .unwrap_or(version)
}

struct ProfileScan {
    version: Option<String>,
    evidence: Vec<String>,
}

fn scan_profile(
    path: &Path,
    mc_version: Option<&str>,
    signature: &ProfileSignature,
) -> Option<ProfileScan> {
    let mut evidence = Vec::new();
    let profile = VersionProfile::from_path(path).ok()?;
    if let Some(expected) = mc_version
        && profile
            .inherits_from
            .as_deref()
            .is_some_and(|actual| actual != expected)
    {
        return None;
    }
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    for marker in signature.main_class_markers {
        if profile.main_class_contains(marker) {
            evidence.push(format!("mainClass contains '{marker}' in {filename}"));
        }
    }
    for artifact in signature.artifacts {
        if let Some(version) = profile.find_library(signature.group, artifact) {
            evidence.push(format!(
                "found {}:{}:{} in {}",
                signature.group, artifact, version, filename
            ));
            return Some(ProfileScan {
                version: Some(version),
                evidence,
            });
        }
    }
    (!evidence.is_empty()).then_some(ProfileScan {
        version: None,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("orbit-detection-{name}-{}", std::process::id()));
        if path.exists() {
            std::fs::remove_dir_all(&path).unwrap();
        }
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn detects_a_loader_in_versions_subdirectory() {
        let root = temp_dir("certain");
        let version = root.join("versions").join("quilt");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(
            version.join("quilt.json"),
            r#"{
  "id": "quilt",
  "mainClass": "org.quiltmc.loader.impl.launch.knot.KnotClient",
  "libraries": [{"name": "org.quiltmc:quilt-loader:0.28.0"}]
}"#,
        )
        .unwrap();

        let result = detect_profile_loader(
            &root,
            Some("1.21.11"),
            LoaderKind::Quilt,
            &ProfileSignature {
                group: "org.quiltmc",
                artifacts: &["quilt-loader"],
                main_class_markers: &["quiltmc"],
                component_uids: &["org.quiltmc.quilt-loader"],
            },
        )
        .unwrap();

        assert_eq!(result.confidence, Confidence::Certain);
        assert_eq!(result.versions, vec!["0.28.0"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn main_class_without_library_is_only_weak_evidence() {
        let root = temp_dir("weak");
        std::fs::write(
            root.join("forge.json"),
            r#"{"id":"forge","mainClass":"net.minecraftforge.bootstrap.Main"}"#,
        )
        .unwrap();

        let result = detect_profile_loader(
            &root,
            None,
            LoaderKind::Forge,
            &ProfileSignature {
                group: "net.minecraftforge",
                artifacts: &["forge"],
                main_class_markers: &["minecraftforge"],
                component_uids: &["net.minecraftforge"],
            },
        )
        .unwrap();

        assert_eq!(result.confidence, Confidence::Low);
        assert!(result.versions.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn strips_minecraft_prefix_without_damaging_prereleases() {
        assert_eq!(
            strip_minecraft_version_prefix("1.21.1-52.0.0".to_string()),
            "52.0.0"
        );
        assert_eq!(
            strip_minecraft_version_prefix("26.1-61.0.3".to_string()),
            "61.0.3"
        );
        assert_eq!(
            strip_minecraft_version_prefix("21.1.0-beta".to_string()),
            "21.1.0-beta"
        );
    }
}

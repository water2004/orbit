//! FabricDetector — 检测 Fabric 加载器环境。

use super::{Confidence, LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::ModLoader;
use crate::metadata::version_profile::VersionProfile;

const FABRIC_GROUP: &str = "net.fabricmc";
const FABRIC_ARTIFACT: &str = "fabric-loader";

pub struct FabricDetector;

impl LoaderDetector for FabricDetector {
    fn name(&self) -> &'static str {
        "Fabric"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Fabric
    }

    fn detect(&self, instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError> {
        // 1. 当前目录
        if let Some((ver, ev)) = scan_for_fabric(instance_dir) {
            return Ok(LoaderInfo {
                loader: ModLoader::Fabric,
                version: Some(ver),
                confidence: Confidence::Certain,
                evidence: ev,
            });
        }

        // 2. versions/ 子目录（不递归）
        let versions_dir = instance_dir.join("versions");
        if versions_dir.is_dir() {
            for entry in std::fs::read_dir(&versions_dir).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!("cannot read {}: {e}", versions_dir.display()))
            })? {
                let entry = entry.map_err(|e| {
                    OrbitError::Other(anyhow::anyhow!("cannot read entry: {e}"))
                })?;
                if entry.path().is_dir() {
                    if let Some((ver, ev)) = scan_for_fabric(&entry.path()) {
                        return Ok(LoaderInfo {
                            loader: ModLoader::Fabric,
                            version: Some(ver),
                            confidence: Confidence::Certain,
                            evidence: ev,
                        });
                    }
                }
            }
        }

        // 3. 无任何证据
        Ok(LoaderInfo {
            loader: ModLoader::Fabric,
            version: None,
            confidence: Confidence::None,
            evidence: vec![],
        })
    }
}

/// 扫描一个目录，查找 Fabric 证据。
/// 遍历目录下的所有 JSON 文件，检查其 libraries 中是否有 fabric-loader。
fn scan_for_fabric(dir: &std::path::Path) -> Option<(String, Vec<String>)> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut evidence = vec![];

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let profile = match VersionProfile::from_path(&path) {
            Ok(p) => p,
            Err(_) => continue,
        };

        // mainClass 包含 fabricmc → 辅助证据
        if profile.main_class_contains("fabricmc") {
            evidence.push(format!(
                "mainClass contains 'fabricmc' in {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
        }

        // libraries 中有 fabric-loader → 确凿证据 + 版本号
        if let Some(ver) = profile.find_library(FABRIC_GROUP, FABRIC_ARTIFACT) {
            evidence.push(format!(
                "found {FABRIC_GROUP}:{FABRIC_ARTIFACT}:{ver}"
            ));
            return Some((ver, evidence));
        }
    }

    None
}

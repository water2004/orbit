//! FabricDetector — 检测 Fabric 加载器环境。

use super::{Confidence, LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::ModLoader;

pub struct FabricDetector;

impl LoaderDetector for FabricDetector {
    fn name(&self) -> &'static str {
        "Fabric"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Fabric
    }

    fn detect(&self, _instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError> {
        // Phase 2 — 实现实际检测逻辑：
        // - 查找 versions/ 下是否有 fabric-loader-*.jar
        // - 扫描 mods/ 下 JAR 的 fabric.mod.json
        // - 读取 launcher_profiles.json 等启动器配置
        //
        // Phase 1：始终返回 None，走交互式选择
        Ok(LoaderInfo {
            loader: ModLoader::Fabric,
            version: None,
            confidence: Confidence::None,
            evidence: vec![],
        })
    }
}

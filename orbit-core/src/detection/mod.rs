//! 实例环境检测层。
//!
//! 策略模式：每个加载器实现 `LoaderDetector` trait，
//! `LoaderDetectionService` 负责编排检测并选择置信度最高的结果。

pub mod fabric;

use crate::error::OrbitError;
use crate::metadata::ModLoader;

// ---------------------------------------------------------------------------
// 类型
// ---------------------------------------------------------------------------

/// 加载器检测信息
#[derive(Debug, Clone)]
pub struct LoaderInfo {
    pub loader: ModLoader,
    pub version: Option<String>,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
}

/// 检测置信度
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// 无任何线索 — 走交互式选择
    None = 0,
    /// 猜测 — 目录名或版本名包含关键词
    Low = 1,
    /// 很可能 — mods/ 下全部是特定加载器的模组
    High = 2,
    /// 确定 — 找到了加载器专属文件
    Certain = 3,
}

// ---------------------------------------------------------------------------
// trait
// ---------------------------------------------------------------------------

/// 每个加载器实现此 trait
pub trait LoaderDetector: Send + Sync {
    /// 探测器名称（如 "Fabric"）
    fn name(&self) -> &'static str;

    /// 对应的加载器类型
    fn loader_type(&self) -> ModLoader;

    /// 检测目标目录，返回该加载器的证据和置信度
    fn detect(&self, instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError>;
}

// ---------------------------------------------------------------------------
// 编排层
// ---------------------------------------------------------------------------

pub struct LoaderDetectionService {
    detectors: Vec<Box<dyn LoaderDetector>>,
}

impl LoaderDetectionService {
    pub fn new() -> Self {
        Self {
            detectors: vec![
                Box::new(self::fabric::FabricDetector),
                // Phase 2: Box::new(super::forge::ForgeDetector),
                // Phase 2: Box::new(super::neoforge::NeoForgeDetector),
                // Phase 2: Box::new(super::quilt::QuiltDetector),
            ],
        }
    }

    /// 遍历所有 detector，返回按置信度降序排列的结果
    pub fn detect_all(
        &self,
        instance_dir: &std::path::Path,
    ) -> Result<Vec<LoaderInfo>, OrbitError> {
        let mut results: Vec<LoaderInfo> = self
            .detectors
            .iter()
            .map(|d| d.detect(instance_dir))
            .collect::<Result<_, _>>()?;

        results.sort_by(|a, b| b.confidence.cmp(&a.confidence));
        Ok(results)
    }

    /// 返回已知加载器列表（供交互式选择使用）
    pub fn known_loaders(&self) -> Vec<(ModLoader, &'static str)> {
        self.detectors
            .iter()
            .map(|d| (d.loader_type(), d.name()))
            .collect()
    }

    /// 按名称查找 detector（用于 `--modloader` 手动指定时验证）
    pub fn find_by_name(&self, name: &str) -> Option<&dyn LoaderDetector> {
        let name_lower = name.to_lowercase();
        self.detectors
            .iter()
            .find(|d| d.name().to_lowercase() == name_lower)
            .map(|d| d.as_ref())
    }
}

impl Default for LoaderDetectionService {
    fn default() -> Self {
        Self::new()
    }
}

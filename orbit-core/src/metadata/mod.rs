//! 模组元数据解析层。
//!
//! 策略模式：每个加载器实现 `MetadataParser` trait，
//! `MetadataExtractor` 负责选择合适的 parser 并提取统一元数据。

pub mod fabric;

use indexmap::IndexMap;

use crate::error::OrbitError;

// ---------------------------------------------------------------------------
// 统一类型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ModLoader {
    Fabric,
    Forge,
    NeoForge,
    Quilt,
    Unknown,
}

impl ModLoader {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModLoader::Fabric => "fabric",
            ModLoader::Forge => "forge",
            ModLoader::NeoForge => "neoforge",
            ModLoader::Quilt => "quilt",
            ModLoader::Unknown => "",
        }
    }
}

/// 统一模组元数据——所有加载器解析后都归一化为此结构
#[derive(Debug, Clone)]
pub struct ModMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: String,
    pub license: Option<String>,
    /// 运行环境: "client" | "server" | "both"
    pub environment: String,
    /// 依赖映射: mod_id → version_constraint
    pub dependencies: IndexMap<String, String>,
    pub loader: ModLoader,
    pub sha256: String,
}

// ---------------------------------------------------------------------------
// Parser trait
// ---------------------------------------------------------------------------

/// 每个加载器实现此 trait
pub trait MetadataParser: Send + Sync {
    /// JAR 内的目标文件名（如 "fabric.mod.json"）
    fn target_file(&self) -> &str;

    /// 此 parser 对应的加载器类型
    fn loader_type(&self) -> ModLoader;

    /// 解析文件内容为统一元数据
    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError>;
}

// ---------------------------------------------------------------------------
// Extractor — 策略选择器
// ---------------------------------------------------------------------------

pub struct MetadataExtractor {
    parsers: Vec<Box<dyn MetadataParser>>,
}

pub struct MetadataExtractorBuilder {
    parsers: Vec<Box<dyn MetadataParser>>,
}

impl MetadataExtractorBuilder {
    pub fn new() -> Self {
        Self { parsers: vec![] }
    }

    pub fn with(mut self, parser: impl MetadataParser + 'static) -> Self {
        self.parsers.push(Box::new(parser));
        self
    }

    pub fn build(self) -> MetadataExtractor {
        MetadataExtractor { parsers: self.parsers }
    }
}

impl MetadataExtractor {
    pub fn builder() -> MetadataExtractorBuilder {
        MetadataExtractorBuilder::new()
    }

    /// 从 ZIP 归档中提取元数据。
    ///
    /// `modloader_context` 用于多加载器 JAR 的歧义消除：
    /// 当同一个 jar 同时包含 fabric.mod.json 和 mods.toml 时，
    /// 优先返回与当前实例 loader 匹配的解析结果。
    pub fn extract(
        &self,
        archive: &mut zip::ZipArchive<std::fs::File>,
        modloader_context: Option<&str>,
    ) -> Result<ModMetadata, OrbitError> {
        // 1. 收集所有能匹配的 parser（O(1) by_name 直接查找）
        let mut candidates: Vec<(&dyn MetadataParser, String)> = vec![];
        for parser in &self.parsers {
            if let Ok(mut file) = archive.by_name(parser.target_file()) {
                let mut content = String::new();
                std::io::Read::read_to_string(&mut file, &mut content)
                    .map_err(|e| OrbitError::Other(anyhow::anyhow!(
                        "failed to read {}: {e}", parser.target_file()
                    )))?;
                candidates.push((parser.as_ref(), content));
            }
        }

        // 2. 消除歧义
        match candidates.len() {
            0 => Err(OrbitError::Other(anyhow::anyhow!(
                "unrecognized JAR: no metadata file found for any known loader"
            ))),
            1 => {
                let (parser, content) = candidates.into_iter().next().unwrap();
                parser.parse(&content)
            }
            _ => {
                // 多加载器 JAR — 按 modloader_context 选择
                if let Some(ctx) = modloader_context {
                    let ctx_lower = ctx.to_lowercase();
                    for (parser, content) in &candidates {
                        if parser.loader_type().as_str() == ctx_lower {
                            return parser.parse(content);
                        }
                    }
                }
                Err(OrbitError::Other(anyhow::anyhow!(
                    "ambiguous JAR: contains multiple metadata files ({}). \
                     Specify --modloader to disambiguate.",
                    candidates.iter().map(|(p, _)| p.target_file())
                        .collect::<Vec<_>>().join(", ")
                )))
            }
        }
    }
}

/// 默认 extractor：注册所有已知 parser
pub fn default_extractor() -> MetadataExtractor {
    MetadataExtractor::builder()
        .with(self::fabric::FabricParser)
        // .with(super::forge::ForgeParser)
        // .with(super::neoforge::NeoForgeParser)
        // .with(super::quilt::QuiltParser)
        .build()
}

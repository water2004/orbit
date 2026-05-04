//! JAR 文件处理模块。
//!
//! 提供：
//! - 哈希计算（SHA-256 / SHA-512）
//! - 模组元数据提取：根据 loader 类型分发到对应 reader（fabric → fabric.mod.json, etc.）

pub mod fabric;

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

use crate::error::OrbitError;

// ── 统一元数据结构 ──────────────────────────────────────────────

/// 从 JAR 中提取的模组元数据（与 loader 无关的公共结构）
#[derive(Debug, Clone)]
pub struct JarModMetadata {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    /// (mod_id, version_constraint, required)
    pub dependencies: Vec<(String, String, bool)>,
    pub embedded_jars: Vec<String>,
}

// ── 顶层 API ────────────────────────────────────────────────────

/// 从 JAR 文件路径读取模组元数据。`loader` 由调用者根据实例配置传入。
pub fn read_mod_metadata(path: &Path, loader: &str) -> Result<JarModMetadata, OrbitError> {
    let file = std::fs::File::open(path).map_err(OrbitError::Io)?;
    let mut archive = zip::ZipArchive::new(file).map_err(OrbitError::Zip)?;

    read_mod_metadata_from_archive(&mut archive, loader)
        .transpose()
        .unwrap_or_else(|| Err(OrbitError::Other(anyhow::anyhow!(
            "no {} mod metadata found in {}",
            loader,
            path.display()
        ))))
}

/// 下载 JAR 并解析 fabric.mod.json。
/// 校验 SHA-512，失败则返回 `ChecksumMismatch`。
pub async fn download_and_parse(
    url: &str,
    expected_sha512: &str,
    loader: &str,
) -> Result<JarModMetadata, crate::error::OrbitError> {
    let client = reqwest::Client::builder()
        .user_agent(format!("orbit/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| crate::error::OrbitError::Other(e.into()))?;

    let bytes = client.get(url).send().await
        .map_err(crate::error::OrbitError::Network)?
        .bytes().await
        .map_err(crate::error::OrbitError::Network)?;

    if !expected_sha512.is_empty() {
        let actual = sha512_digest(&bytes);
        if actual != expected_sha512 {
            return Err(crate::error::OrbitError::ChecksumMismatch {
                name: url.to_string(),
                expected: expected_sha512.to_string(),
                actual,
            });
        }
    }

    read_mod_metadata_from_bytes(&bytes, loader)
}

/// 从字节数据读取模组元数据（用于内嵌 JAR）。`loader` 由调用者传入。
pub fn read_mod_metadata_from_bytes(data: &[u8], loader: &str) -> Result<JarModMetadata, OrbitError> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(OrbitError::Zip)?;

    read_mod_metadata_from_archive(&mut archive, loader)
        .transpose()
        .unwrap_or_else(|| Err(OrbitError::Other(anyhow::anyhow!(
            "no {} mod metadata found in embedded JAR",
            loader
        ))))
}

/// 根据 loader 分发到对应 reader
fn read_mod_metadata_from_archive<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    loader: &str,
) -> Result<Option<JarModMetadata>, OrbitError> {
    match loader {
        "fabric" | "quilt" => fabric::try_read(archive),
        _ => Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported mod loader: {loader}"
        ))),
    }
}

// ── 哈希计算 ────────────────────────────────────────────────────

/// 计算文件 SHA-512
pub fn compute_sha512(path: &Path) -> Result<String, std::io::Error> {
    use sha2::Sha512;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha512::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 计算文件 SHA-256
pub fn compute_sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 计算字节数据的 SHA-512
pub fn sha512_digest(data: &[u8]) -> String {
    use sha2::Sha512;
    let mut hasher = Sha512::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// 计算字节数据的 SHA-256
pub fn sha256_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

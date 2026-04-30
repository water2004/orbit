//! JAR 文件解析：读取 fabric.mod.json 并计算 SHA-256

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use zip::ZipArchive;

/// 从 JAR 中解析出的 Fabric 模组元数据
#[derive(Debug, Clone, Deserialize)]
pub struct FabricModInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub depends: HashMap<String, String>,
    #[serde(default, skip_deserializing)]
    pub sha256: String,
}

/// 从 JAR 文件中提取 fabric.mod.json 并计算 SHA-256。
///
/// 流式读取文件以计算哈希，然后回退读取 ZIP 条目。
/// 文件大小无关，不会将整个 JAR 加载到内存。
pub fn get_fabric_mod_info(file: std::fs::File) -> Result<FabricModInfo, zip::result::ZipError> {
    let mut file = file;

    // 流式计算 SHA-256
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let sha_hex = hex::encode(hasher.finalize());

    // 回退文件指针以便 ZipArchive 使用
    file.seek(SeekFrom::Start(0))?;

    let mut archive = ZipArchive::new(file)?;
    let target_file_name = "fabric.mod.json";

    if let Ok(mut zip_file) = archive.by_name(target_file_name) {
        let mut content = String::new();
        zip_file.read_to_string(&mut content)?;

        let mut info: FabricModInfo = serde_json::from_str(&content).map_err(|e| {
            zip::result::ZipError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            ))
        })?;

        info.sha256 = sha_hex;
        Ok(info)
    } else {
        Err(zip::result::ZipError::FileNotFound)
    }
}

/// 计算任意文件的 SHA-256
pub fn compute_sha256(path: &std::path::Path) -> Result<String, std::io::Error> {
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

/// 计算字节数据的 SHA-256
pub fn sha256_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

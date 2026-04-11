use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use zip::ZipArchive;
use sha2::{Digest, Sha256};
use hex;

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

pub fn get_fabric_mod_info(mut file: File) -> zip::result::ZipResult<FabricModInfo> {
    // 流式读取计算 SHA-256（不会把整个文件保存在内存中）
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }

    let sha_hex = hex::encode(hasher.finalize());

    // 读取后移动回文件开头以便 ZipArchive 使用同一个 File
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

        // 填充计算得到的 sha256
        info.sha256 = sha_hex;

        Ok(info)
    } else {
        Err(zip::result::ZipError::FileNotFound)
    }
}

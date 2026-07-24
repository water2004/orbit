//! JAR 文件处理模块。
//!
//! 提供：
//! - 哈希计算（SHA-256 / SHA-512）
//! - 模组元数据提取：根据 loader 类型分发到对应 reader（fabric → fabric.mod.json, etc.）

pub mod fabric;
pub mod forge;
pub mod quilt;

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
    /// 从 META-INF/jars/ 中解出的内嵌子模组元数据
    pub implanted_mods: Vec<JarModMetadata>,
}

// ── 顶层 API ────────────────────────────────────────────────────

/// 从 JAR 文件路径读取模组元数据。`loader` 由调用者根据实例配置传入。
pub fn read_mod_metadata(path: &Path, loader: &str) -> Result<JarModMetadata, OrbitError> {
    let file = std::fs::File::open(path).map_err(OrbitError::Io)?;
    let mut archive = zip::ZipArchive::new(file).map_err(OrbitError::Zip)?;

    read_mod_metadata_from_archive(&mut archive, loader)
        .transpose()
        .unwrap_or_else(|| {
            Err(OrbitError::Other(anyhow::anyhow!(
                "no {} mod metadata found in {}",
                loader,
                path.display()
            )))
        })
}

/// 下载 JAR 并解析 fabric.mod.json。
/// 校验 SHA-512，失败则返回 `ChecksumMismatch`。
/// 优先从全局缓存读取，未命中才走 HTTP；下载后自动存入缓存。
pub async fn download_and_parse(
    url: &str,
    filename: &str,
    expected_sha512: &str,
    loader: &str,
) -> Result<JarModMetadata, crate::error::OrbitError> {
    // 缓存查询
    if let Ok(cache) = crate::jar_cache::JarCache::load()
        && let Some(bytes) = cache.get_bytes(expected_sha512)
    {
        return read_mod_metadata_from_bytes(&bytes, loader);
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("orbit/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| crate::error::OrbitError::Other(e.into()))?;

    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(crate::error::OrbitError::Network)?
        .bytes()
        .await
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

    // 存入缓存
    let _ = crate::jar_cache::JarCache::load().map(|mut c| {
        let _ = c.store_bytes(expected_sha512, filename, &bytes);
    });

    read_mod_metadata_from_bytes(&bytes, loader)
}

/// 从字节数据读取模组元数据（用于内嵌 JAR）。`loader` 由调用者传入。
pub fn read_mod_metadata_from_bytes(
    data: &[u8],
    loader: &str,
) -> Result<JarModMetadata, OrbitError> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(OrbitError::Zip)?;

    read_mod_metadata_from_archive(&mut archive, loader)
        .transpose()
        .unwrap_or_else(|| {
            Err(OrbitError::Other(anyhow::anyhow!(
                "no {} mod metadata found in embedded JAR",
                loader
            )))
        })
}

/// 根据 loader 分发到对应 reader
fn read_mod_metadata_from_archive<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    loader: &str,
) -> Result<Option<JarModMetadata>, OrbitError> {
    let normalized_loader = loader.to_ascii_lowercase();
    let meta_opt = match normalized_loader.as_str() {
        "fabric" => fabric::try_read(archive)?,
        "quilt" => quilt::try_read(archive)?,
        "forge" => forge::try_read(archive, crate::metadata::ModLoader::Forge)?,
        "neoforge" => forge::try_read(archive, crate::metadata::ModLoader::NeoForge)?,
        _ => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "unsupported mod loader: {loader}"
            )));
        }
    };

    if let Some(mut meta) = meta_opt {
        let mut implanted = Vec::new();
        for emb_path in &meta.embedded_jars {
            if let Ok(mut entry) = archive.by_name(emb_path) {
                let mut bytes = Vec::new();
                if std::io::Read::read_to_end(&mut entry, &mut bytes).is_ok()
                    && let Ok(inner_meta) = read_mod_metadata_from_bytes(&bytes, loader)
                {
                    implanted.push(inner_meta);
                }
            }
        }
        meta.implanted_mods = implanted;
        return Ok(Some(meta));
    }

    Ok(None)
}

/// Read the first matching UTF-8 metadata entry.
///
/// Exact paths win. As a compatibility fallback, metadata located one directory
/// below the archive root is also accepted.
pub(super) fn read_metadata_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    targets: &[&str],
) -> Result<Option<(String, String)>, OrbitError> {
    for target in targets {
        if let Ok(mut entry) = archive.by_name(target) {
            let mut content = String::new();
            entry.read_to_string(&mut content).map_err(|error| {
                OrbitError::Other(anyhow::anyhow!("cannot read {target}: {error}"))
            })?;
            return Ok(Some(((*target).to_string(), content)));
        }
    }

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!("cannot read ZIP entry: {error}"))
        })?;
        let name = entry.name().to_string();
        let Some(target) = targets.iter().find(|target| {
            name.ends_with(**target)
                && name
                    .strip_suffix(**target)
                    .is_some_and(|prefix| prefix.matches('/').count() <= 1)
        }) else {
            continue;
        };
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .map_err(|error| OrbitError::Other(anyhow::anyhow!("cannot read {target}: {error}")))?;
        return Ok(Some(((*target).to_string(), content)));
    }
    Ok(None)
}

// ── 哈希计算 ────────────────────────────────────────────────────

/// 计算文件 SHA-1
pub fn compute_sha1(path: &Path) -> Result<String, std::io::Error> {
    use sha1::Sha1;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
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

/// 计算字节数据的 SHA-1
pub fn sha1_digest(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// 计算字节数据的 SHA-256
pub fn sha256_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    const SODIUM_LIKE: &str = include_str!("../../tests/fixtures/sodium_like.fabric.mod.json");
    const SODIUM_FABRIC_API_BASE: &str =
        include_str!("../../tests/fixtures/sodium_fabric_api_base.fabric.mod.json");
    const PRINTER_PARENT: &str =
        include_str!("../../tests/fixtures/printer_parent.fabric.mod.json");
    const PRINTER_EMBEDDED: &str =
        include_str!("../../tests/fixtures/printer_embedded.fabric.mod.json");

    fn jar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();

        for (name, contents) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents).unwrap();
        }

        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_realistic_fabric_metadata_from_jar_bytes() {
        let bytes = jar_bytes(&[("fabric.mod.json", SODIUM_LIKE.as_bytes())]);

        let meta = read_mod_metadata_from_bytes(&bytes, "fabric").unwrap();

        assert_eq!(meta.mod_id, "sodium");
        assert_eq!(meta.version, "0.8.11+mc1.21.11");
        assert_eq!(meta.name, "Sodium");
        assert!(meta.dependencies.iter().any(|(name, version, required)| {
            name == "fabric-rendering-fluids-v1" && version == ">=2.0.0" && *required
        }));
        assert_eq!(meta.embedded_jars.len(), 9);
        assert_eq!(
            meta.embedded_jars[0],
            "META-INF/jars/fabric-api-base-1.0.5+4ebb5c083e.jar"
        );
        assert!(meta.implanted_mods.is_empty());
    }

    #[test]
    fn reads_realistic_sodium_embedded_fabric_modules() {
        let embedded_path = "META-INF/jars/fabric-api-base-1.0.5+4ebb5c083e.jar";
        let embedded = jar_bytes(&[("fabric.mod.json", SODIUM_FABRIC_API_BASE.as_bytes())]);
        let parent_entries: [(&str, &[u8]); 2] = [
            ("fabric.mod.json", SODIUM_LIKE.as_bytes()),
            (embedded_path, embedded.as_slice()),
        ];
        let parent = jar_bytes(&parent_entries);

        let meta = read_mod_metadata_from_bytes(&parent, "fabric").unwrap();

        assert_eq!(meta.mod_id, "sodium");
        assert_eq!(meta.embedded_jars.len(), 9);
        assert_eq!(meta.implanted_mods.len(), 1);
        let implanted = &meta.implanted_mods[0];
        assert_eq!(implanted.mod_id, "fabric-api-base");
        assert_eq!(implanted.version, "1.0.5+4ebb5c083e");
        assert!(
            implanted
                .dependencies
                .iter()
                .any(|(name, version, required)| name == "fabricloader"
                    && version == ">=0.17.3"
                    && *required)
        );
    }

    #[test]
    fn reads_implanted_fabric_jars_declared_by_parent_metadata() {
        let embedded_path = "META-INF/jars/litematica-printer-1.21.11-2.4+20260330.10.jar";
        let embedded = jar_bytes(&[("fabric.mod.json", PRINTER_EMBEDDED.as_bytes())]);
        let parent_entries: [(&str, &[u8]); 2] = [
            ("fabric.mod.json", PRINTER_PARENT.as_bytes()),
            (embedded_path, embedded.as_slice()),
        ];
        let parent = jar_bytes(&parent_entries);

        let meta = read_mod_metadata_from_bytes(&parent, "fabric").unwrap();

        assert_eq!(meta.mod_id, "litematica-printer-all");
        assert_eq!(meta.embedded_jars.len(), 13);
        assert!(meta.embedded_jars.iter().any(|path| path == embedded_path));
        assert_eq!(meta.implanted_mods.len(), 1);
        let implanted = &meta.implanted_mods[0];
        assert_eq!(implanted.mod_id, "litematica-printer");
        assert_eq!(implanted.version, "2.4+20260330.10");
        assert_eq!(
            implanted.embedded_jars,
            vec!["META-INF/jars/pinyin4j-2.5.1.jar"]
        );
        assert!(
            implanted
                .dependencies
                .iter()
                .any(|(name, version, required)| name == "minecraft"
                    && version == "1.21.11"
                    && *required)
        );
    }

    #[test]
    fn reads_forge_metadata_and_jarjar_children() {
        let child_metadata = br#"
[[mods]]
modId = "child"
version = "1"
displayName = "Child"
"#;
        let child = jar_bytes(&[("META-INF/mods.toml", child_metadata)]);
        let parent_metadata = br#"
license = "MIT"
[[mods]]
modId = "parent"
version = "${file.jarVersion}"
displayName = "Parent"
authors = "Alice"
[[dependencies.parent]]
modId = "forge"
mandatory = true
versionRange = "[47,)"
[[dependencies.parent]]
modId = "optional-api"
mandatory = false
"#;
        let jarjar = br#"{"jars":[{"path":"META-INF/jarjar/child.jar"}]}"#;
        let manifest = b"Manifest-Version: 1.0\r\nImplementation-Version: 2.0.1\r\n";
        let entries: [(&str, &[u8]); 4] = [
            ("META-INF/mods.toml", parent_metadata),
            ("META-INF/MANIFEST.MF", manifest),
            ("META-INF/jarjar/metadata.json", jarjar),
            ("META-INF/jarjar/child.jar", child.as_slice()),
        ];

        let metadata = read_mod_metadata_from_bytes(&jar_bytes(&entries), "forge").unwrap();

        assert_eq!(metadata.mod_id, "parent");
        assert_eq!(metadata.version, "2.0.1");
        assert_eq!(
            metadata.dependencies,
            [
                ("forge".to_string(), "[47,)".to_string(), true),
                ("optional-api".to_string(), "*".to_string(), false),
            ]
        );
        assert_eq!(metadata.implanted_mods.len(), 1);
        assert_eq!(metadata.implanted_mods[0].mod_id, "child");
    }

    #[test]
    fn reads_modern_and_legacy_neoforge_metadata_names() {
        let metadata = br#"
[[mods]]
modId = "neo-example"
version = "1"
displayName = "Neo Example"
[[dependencies.neo-example]]
modId = "neoforge"
type = "required"
versionRange = "[21,)"
"#;
        for target in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
            let jar = jar_bytes(&[(target, metadata)]);
            let parsed = read_mod_metadata_from_bytes(&jar, "neoforge").unwrap();
            assert_eq!(parsed.mod_id, "neo-example");
            assert!(parsed.dependencies[0].2);
        }
    }

    #[test]
    fn reads_quilt_metadata_and_falls_back_to_fabric() {
        let quilt_metadata = br#"{
  "quilt_loader": {
    "id": "quilt-example",
    "version": "1",
    "metadata": {"name": "Quilt Example"},
    "depends": [{"id": "quilt_loader", "versions": ">=0.20"}]
  }
}"#;
        let quilt = jar_bytes(&[("quilt.mod.json", quilt_metadata)]);
        let parsed = read_mod_metadata_from_bytes(&quilt, "quilt").unwrap();
        assert_eq!(parsed.mod_id, "quilt-example");

        let fabric = jar_bytes(&[("fabric.mod.json", PRINTER_EMBEDDED.as_bytes())]);
        let parsed = read_mod_metadata_from_bytes(&fabric, "quilt").unwrap();
        assert_eq!(parsed.mod_id, "litematica-printer");
    }
}

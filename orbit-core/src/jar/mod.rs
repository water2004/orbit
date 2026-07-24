//! JAR 文件处理模块。
//!
//! 提供：
//! - 哈希计算（SHA-1 / SHA-256 / SHA-512 / CurseForge fingerprint）
//! - 模组元数据提取：根据 loader 类型分发到对应 reader（fabric → fabric.mod.json, etc.）

pub mod fabric;
pub mod fingerprint;
pub mod forge;
pub mod quilt;

pub use fingerprint::{compute_curseforge_fingerprint, curseforge_fingerprint};

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
    pub environment: crate::metadata::Environment,
    pub dependencies: Vec<crate::metadata::DependencyExpression>,
    pub provides: Vec<crate::metadata::ProvidedMod>,
    pub language_loader: Option<crate::metadata::LanguageLoaderRequirement>,
    pub embedded_jars: Vec<String>,
    pub embedded_artifacts: Vec<crate::metadata::EmbeddedArtifact>,
    /// 同一物理 JAR 提供的其他模组，包括多模组声明和嵌套 JAR。
    pub bundled_mods: Vec<JarModMetadata>,
}

pub(super) fn from_mod_file(
    mut file: crate::metadata::ModFileMetadata,
) -> Result<JarModMetadata, OrbitError> {
    let primary = file.mods.drain(..1).next().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!("loader metadata does not declare any mods"))
    })?;
    let bundled_mods = file
        .mods
        .into_iter()
        .map(|metadata| JarModMetadata {
            mod_id: metadata.id,
            name: metadata.name,
            version: metadata.version,
            environment: metadata.environment,
            dependencies: metadata.dependencies,
            provides: metadata.provides,
            language_loader: file.language_loader.clone(),
            embedded_jars: Vec::new(),
            embedded_artifacts: Vec::new(),
            bundled_mods: Vec::new(),
        })
        .collect();

    Ok(JarModMetadata {
        mod_id: primary.id,
        name: primary.name,
        version: primary.version,
        environment: primary.environment,
        dependencies: primary.dependencies,
        provides: primary.provides,
        language_loader: file.language_loader,
        embedded_jars: file.embedded_jars,
        embedded_artifacts: Vec::new(),
        bundled_mods,
    })
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

/// 下载 JAR 并按实例 loader 解析元数据。
/// 校验来源提供的 SHA-512 或 SHA-1，失败则返回 `ChecksumMismatch`。
/// 优先从全局缓存读取，未命中才走 HTTP；下载后自动存入缓存。
pub async fn download_and_parse(
    cache: &crate::jar_cache::JarCache,
    downloader: &crate::providers::ArtifactDownloadClient,
    url: &str,
    filename: &str,
    expected_sha1: &str,
    expected_sha512: &str,
    loader: &str,
) -> Result<JarModMetadata, crate::error::OrbitError> {
    // 缓存查询
    if let Some(bytes) = cache.get_bytes(expected_sha512, expected_sha1)
        && verify_source_hash(&bytes, expected_sha1, expected_sha512, filename).is_ok()
    {
        return read_mod_metadata_from_bytes(&bytes, loader);
    }

    let bytes = downloader.download(url, filename).await?;

    verify_source_hash(&bytes, expected_sha1, expected_sha512, url)?;

    // 存入缓存
    cache.store_bytes(&bytes)?;

    read_mod_metadata_from_bytes(&bytes, loader)
}

pub(crate) fn verify_source_hash(
    bytes: &[u8],
    expected_sha1: &str,
    expected_sha512: &str,
    name: &str,
) -> Result<(), OrbitError> {
    let (expected, actual) = if !expected_sha512.is_empty() {
        (expected_sha512, sha512_digest(bytes))
    } else if !expected_sha1.is_empty() {
        (expected_sha1, sha1_digest(bytes))
    } else {
        return Ok(());
    };
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(OrbitError::ChecksumMismatch {
            name: name.to_string(),
            expected: expected.to_string(),
            actual,
        })
    }
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
        inject_bytecode_requirement(archive, &mut meta, &normalized_loader)?;
        let mut bundled = Vec::new();
        for emb_path in &meta.embedded_jars {
            let Ok(mut entry) = archive.by_name(emb_path) else {
                continue;
            };
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(OrbitError::Io)?;
            let cursor = std::io::Cursor::new(bytes);
            let mut embedded = zip::ZipArchive::new(cursor).map_err(OrbitError::Zip)?;
            if let Some(inner_meta) = read_mod_metadata_from_archive(&mut embedded, loader)? {
                bundled.push(inner_meta);
            }
        }
        meta.bundled_mods.extend(bundled);
        return Ok(Some(meta));
    }

    Ok(None)
}

fn inject_bytecode_requirement<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    metadata: &mut JarModMetadata,
    loader: &str,
) -> Result<(), OrbitError> {
    let mut highest_major = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(OrbitError::Zip)?;
        let name = entry.name();
        if !name.ends_with(".class") || name.starts_with("META-INF/versions/") {
            continue;
        }
        let mut header = [0_u8; 8];
        if entry.read_exact(&mut header).is_ok() && header[..4] == [0xca, 0xfe, 0xba, 0xbe] {
            let major = u16::from_be_bytes([header[6], header[7]]);
            highest_major = Some(highest_major.map_or(major, |current: u16| current.max(major)));
        }
    }

    let Some(java_version) = highest_major.and_then(class_major_to_java) else {
        return Ok(());
    };
    let requirement = if matches!(loader, "forge" | "neoforge") {
        format!("[{java_version},)")
    } else {
        format!(">={java_version}")
    };
    metadata.dependencies.push(
        crate::metadata::ModDependency {
            id: "java".to_string(),
            requirement,
            kind: crate::metadata::DependencyKind::Required,
            environment: crate::metadata::Environment::Both,
            ordering: crate::metadata::DependencyOrdering::None,
            reason: Some(format!(
                "class-file major version {} requires Java {java_version} or newer",
                highest_major.expect("checked above")
            )),
            unless: None,
        }
        .into(),
    );
    Ok(())
}

fn class_major_to_java(major: u16) -> Option<u16> {
    match major {
        45 => Some(1),
        46.. => Some(major - 44),
        _ => None,
    }
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

#[cfg(test)]
mod source_hash_tests {
    use super::verify_source_hash;

    #[test]
    fn verifies_whichever_source_hash_is_available() {
        let bytes = b"orbit";
        assert!(
            verify_source_hash(bytes, "", &super::sha512_digest(bytes), "artifact.jar").is_ok()
        );
        assert!(verify_source_hash(bytes, &super::sha1_digest(bytes), "", "artifact.jar").is_ok());
        assert!(verify_source_hash(bytes, "wrong", "", "artifact.jar").is_err());
    }
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

    fn has_dependency(
        metadata: &JarModMetadata,
        id: &str,
        requirement: &str,
        required: bool,
    ) -> bool {
        metadata
            .dependencies
            .iter()
            .flat_map(|dependency| dependency.relations())
            .any(|dependency| {
                dependency.id == id
                    && dependency.requirement == requirement
                    && dependency.kind.installs_target() == required
            })
    }

    #[test]
    fn reads_realistic_fabric_metadata_from_jar_bytes() {
        let bytes = jar_bytes(&[("fabric.mod.json", SODIUM_LIKE.as_bytes())]);

        let meta = read_mod_metadata_from_bytes(&bytes, "fabric").unwrap();

        assert_eq!(meta.mod_id, "sodium");
        assert_eq!(meta.version, "0.8.11+mc1.21.11");
        assert_eq!(meta.name, "Sodium");
        assert!(has_dependency(
            &meta,
            "fabric-rendering-fluids-v1",
            ">=2.0.0",
            true
        ));
        assert_eq!(meta.embedded_jars.len(), 9);
        assert_eq!(
            meta.embedded_jars[0],
            "META-INF/jars/fabric-api-base-1.0.5+4ebb5c083e.jar"
        );
        assert!(meta.bundled_mods.is_empty());
    }

    #[tokio::test]
    async fn cached_artifact_is_parsed_without_contacting_its_remote_url() {
        let bytes = jar_bytes(&[("fabric.mod.json", PRINTER_EMBEDDED.as_bytes())]);
        let cache_dir =
            std::env::temp_dir().join(format!("orbit-download-cache-test-{}", std::process::id()));
        let cache = crate::jar_cache::JarCache::open(cache_dir.clone()).unwrap();
        cache.store_bytes(&bytes).unwrap();
        let sha512 = sha512_digest(&bytes);
        let downloader = crate::providers::ArtifactDownloadClient::anonymous("orbit-test").unwrap();

        let metadata = download_and_parse(
            &cache,
            &downloader,
            "https://example.invalid/must-not-be-requested.jar",
            "cached.jar",
            "",
            &sha512,
            "fabric",
        )
        .await
        .unwrap();

        assert_eq!(metadata.mod_id, "litematica-printer");
        std::fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn derives_java_requirement_from_root_class_bytecode() {
        let class = [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 65];
        let multi_release = [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 66];
        let entries: [(&str, &[u8]); 3] = [
            ("fabric.mod.json", PRINTER_EMBEDDED.as_bytes()),
            ("example/Main.class", &class),
            ("META-INF/versions/22/example/Main.class", &multi_release),
        ];

        let metadata = read_mod_metadata_from_bytes(&jar_bytes(&entries), "fabric").unwrap();

        assert!(has_dependency(&metadata, "java", ">=21", true));
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
        assert_eq!(meta.bundled_mods.len(), 1);
        let bundled = &meta.bundled_mods[0];
        assert_eq!(bundled.mod_id, "fabric-api-base");
        assert_eq!(bundled.version, "1.0.5+4ebb5c083e");
        assert!(has_dependency(bundled, "fabricloader", ">=0.17.3", true));
    }

    #[test]
    fn reads_bundled_fabric_jars_declared_by_parent_metadata() {
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
        assert_eq!(meta.bundled_mods.len(), 1);
        let bundled = &meta.bundled_mods[0];
        assert_eq!(bundled.mod_id, "litematica-printer");
        assert_eq!(bundled.version, "2.4+20260330.10");
        assert_eq!(
            bundled.embedded_jars,
            vec!["META-INF/jars/pinyin4j-2.5.1.jar"]
        );
        assert!(has_dependency(bundled, "minecraft", "1.21.11", true));
    }

    #[test]
    fn reads_forge_metadata_and_jarjar_children() {
        let child_metadata = br#"
modLoader = "javafml"
loaderVersion = "[47,)"
license = "MIT"
[[mods]]
modId = "child"
version = "1"
displayName = "Child"
"#;
        let child = jar_bytes(&[("META-INF/mods.toml", child_metadata)]);
        let parent_metadata = br#"
modLoader = "javafml"
loaderVersion = "[47,)"
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
modId = "optional_api"
mandatory = false
"#;
        let jarjar = br#"{"jars":[{
            "identifier":{"group":"org.example","artifact":"child"},
            "version":{"range":"[1,2)","artifactVersion":"1"},
            "path":"META-INF/jarjar/child.jar",
            "isObfuscated":false
        }]}"#;
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
        assert!(has_dependency(&metadata, "forge", "[47,)", true));
        assert!(has_dependency(&metadata, "optional_api", "*", false));
        assert_eq!(metadata.bundled_mods.len(), 1);
        assert_eq!(metadata.bundled_mods[0].mod_id, "child");
        assert_eq!(metadata.embedded_artifacts[0].id, "org.example:child");
    }

    #[test]
    fn reads_modern_and_legacy_neoforge_metadata_names() {
        let metadata = br#"
license = "MIT"
[[mods]]
modId = "neo_example"
version = "1"
displayName = "Neo Example"
[[dependencies.neo_example]]
modId = "neoforge"
type = "required"
versionRange = "[21,)"
"#;
        for target in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
            let jar = jar_bytes(&[(target, metadata)]);
            let parsed = read_mod_metadata_from_bytes(&jar, "neoforge").unwrap();
            assert_eq!(parsed.mod_id, "neo_example");
            assert!(
                parsed.dependencies[0]
                    .relations()
                    .iter()
                    .any(|dependency| dependency.kind.installs_target())
            );
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

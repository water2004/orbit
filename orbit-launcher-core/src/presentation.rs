//! Instance presentation is persisted by Launcher, not inferred by the GUI.
use std::io::Cursor;
use std::path::Path;

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;
use crate::instance::ManifestFile;
use crate::operations::ConfigureInstanceResult;
use crate::registry::InstanceRegistry;
use crate::runtime::RuntimePaths;

pub const INSTANCE_ICON: &str = ".orbit-launcher/instance-icon.png";

pub fn set_instance_icon(
    paths: &RuntimePaths,
    selector: &str,
    source: Option<&Path>,
) -> Result<ConfigureInstanceResult, LauncherError> {
    let registry = InstanceRegistry::load(&paths.instances_file())?;
    let entry = registry
        .find(selector)
        .cloned()
        .ok_or_else(|| LauncherError::InstanceNotFound(selector.into()))?;
    let mut file = ManifestFile::open(entry.instance_directory())?;
    if file.inner.id != entry.id {
        return Err(LauncherError::InstanceRegistryMismatch(
            "instance icon target has a different ID".into(),
        ));
    }
    let png = source.map(read_icon).transpose()?;
    let target = entry.instance_directory().join(INSTANCE_ICON);
    let previous = match std::fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if let Some(bytes) = &png {
        write_atomic(&target, bytes)?;
    } else if previous.is_some() {
        std::fs::remove_file(&target)?;
    }
    file.inner.icon = png.as_ref().map(|_| INSTANCE_ICON.into());
    if let Err(error) = file.save() {
        let rollback = if let Some(bytes) = previous {
            write_atomic(&target, &bytes)
        } else if target.exists() {
            std::fs::remove_file(&target).map_err(LauncherError::Io)
        } else {
            Ok(())
        };
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(LauncherError::Transaction(format!(
                "icon update failed: {error}; restoring the previous icon failed: {rollback}"
            ))),
        };
    }
    Ok(ConfigureInstanceResult {
        entry,
        manifest: file.inner,
    })
}

fn read_icon(path: &Path) -> Result<Vec<u8>, LauncherError> {
    if std::fs::metadata(path)?.len() > 16 * 1024 * 1024 {
        return Err(LauncherError::InvalidManifest(
            "instance icon exceeds 16 MiB".into(),
        ));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| LauncherError::InvalidManifest(format!("cannot decode instance icon: {e}")))?;
    let mut png = Cursor::new(Vec::new());
    decoded
        .thumbnail(512, 512)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| LauncherError::InvalidManifest(format!("cannot encode instance icon: {e}")))?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InstanceKind, LoaderKind, RuntimePathOptions};
    #[test]
    fn image_updates_are_cli_owned_and_bad_input_preserves_existing_icon() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(dir.path().join("config")),
            data_dir: Some(dir.path().join("data")),
            cache_dir: Some(dir.path().join("cache")),
        })
        .unwrap();
        let created = crate::operations::create_instance(
            &paths,
            crate::operations::CreateInstanceRequest {
                directory: dir.path().join("server"),
                name: "Icon test".into(),
                kind: InstanceKind::Server,
                minecraft_requirement: "26.2".into(),
                loader_kind: LoaderKind::Vanilla,
                loader_requirement: None,
            },
        )
        .unwrap();
        let source = dir.path().join("test.png");
        image::RgbaImage::new(1000, 500).save(&source).unwrap();
        let selector = created.entry.id.to_string();
        let result = set_instance_icon(&paths, &selector, Some(&source)).unwrap();
        assert_eq!(result.manifest.icon.as_deref(), Some(INSTANCE_ICON));
        let installed = created.entry.instance_directory().join(INSTANCE_ICON);
        let original = std::fs::read(&installed).unwrap();
        assert_eq!(image::open(&installed).unwrap().width(), 512);
        let bad = dir.path().join("bad.png");
        std::fs::write(&bad, b"not an image").unwrap();
        assert!(set_instance_icon(&paths, &selector, Some(&bad)).is_err());
        assert_eq!(std::fs::read(&installed).unwrap(), original);
        set_instance_icon(&paths, &selector, None).unwrap();
        assert!(!installed.exists());
        assert!(
            ManifestFile::open(created.entry.instance_directory())
                .unwrap()
                .inner
                .icon
                .is_none()
        );
    }
}

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::AccountMetadata;
use crate::atomic_io::write_atomic;
use crate::error::LauncherError;
use crate::runtime::RuntimePaths;

const MAX_SKIN_BYTES: usize = 4 * 1024 * 1024;
const AVATAR_SIZE: u32 = 72;
const FACE_SIZE: u32 = 64;
const FACE_OFFSET: i64 = 4;

pub fn account_avatar_path(paths: &RuntimePaths, account: &AccountMetadata) -> Option<PathBuf> {
    let skin_url = account.skin_url.as_deref()?;
    let digest = Sha256::digest(skin_url.as_bytes());
    let fingerprint = hex::encode(&digest[..8]);
    Some(
        paths
            .account_avatars_dir()
            .join(format!("{}-{fingerprint}.png", account.id)),
    )
}

pub async fn ensure_account_avatar(
    paths: &RuntimePaths,
    client: &reqwest::Client,
    account: &AccountMetadata,
) -> Result<Option<PathBuf>, LauncherError> {
    let Some(skin_url) = account.skin_url.as_deref() else {
        return Ok(None);
    };
    let Some(path) = account_avatar_path(paths, account) else {
        return Ok(None);
    };
    if path.is_file() {
        return Ok(Some(path));
    }

    let response = client.get(skin_url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SKIN_BYTES as u64)
    {
        return Err(LauncherError::Authentication(
            "account skin exceeds the 4 MiB presentation limit".to_string(),
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_SKIN_BYTES {
        return Err(LauncherError::Authentication(
            "account skin exceeds the 4 MiB presentation limit".to_string(),
        ));
    }
    let avatar = render_avatar(&bytes)?;
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(avatar)
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|error| {
            LauncherError::Authentication(format!("failed to encode account avatar: {error}"))
        })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    remove_other_account_avatars(&paths.account_avatars_dir(), account.id, Some(&path))?;
    write_atomic(&path, encoded.get_ref())?;
    Ok(Some(path))
}

pub fn remove_account_avatars(paths: &RuntimePaths, account_id: Uuid) -> Result<(), LauncherError> {
    remove_other_account_avatars(&paths.account_avatars_dir(), account_id, None)
}

fn remove_other_account_avatars(
    directory: &Path,
    account_id: Uuid,
    keep: Option<&Path>,
) -> Result<(), LauncherError> {
    if !directory.is_dir() {
        return Ok(());
    }
    let prefix = format!("{account_id}-");
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if keep.is_some_and(|keep| keep == path) {
            continue;
        }
        let matches = path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".png"));
        if matches {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn render_avatar(bytes: &[u8]) -> Result<RgbaImage, LauncherError> {
    let skin = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|error| LauncherError::Authentication(format!("invalid PNG skin: {error}")))?;
    let (width, height) = skin.dimensions();
    if width < 64 || width % 64 != 0 {
        return Err(LauncherError::Authentication(format!(
            "invalid Minecraft skin width {width}; expected a multiple of 64"
        )));
    }
    let scale = width / 64;
    if height < 16 * scale {
        return Err(LauncherError::Authentication(format!(
            "invalid Minecraft skin dimensions {width}x{height}"
        )));
    }

    let base = skin.crop_imm(8 * scale, 8 * scale, 8 * scale, 8 * scale);
    let hat = skin.crop_imm(40 * scale, 8 * scale, 8 * scale, 8 * scale);
    let base = base.resize_exact(FACE_SIZE, FACE_SIZE, FilterType::Nearest);
    let hat = hat.resize_exact(AVATAR_SIZE, AVATAR_SIZE, FilterType::Nearest);
    let mut avatar = RgbaImage::new(AVATAR_SIZE, AVATAR_SIZE);
    image::imageops::overlay(&mut avatar, &base, FACE_OFFSET, FACE_OFFSET);
    image::imageops::overlay(&mut avatar, &hat, 0, 0);
    Ok(avatar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    #[test]
    fn minecraft_face_and_hat_layers_are_composited() {
        let mut skin = ImageBuffer::from_pixel(64, 64, Rgba([0, 0, 0, 0]));
        for y in 8..16 {
            for x in 8..16 {
                skin.put_pixel(x, y, Rgba([20, 40, 60, 255]));
            }
        }
        for y in 8..16 {
            for x in 40..48 {
                skin.put_pixel(x, y, Rgba([200, 10, 30, 128]));
            }
        }
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(skin)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();

        let avatar = render_avatar(bytes.get_ref()).unwrap();
        assert_eq!(avatar.dimensions(), (72, 72));
        assert_eq!(avatar.get_pixel(36, 36).0, [110, 24, 44, 254]);
        assert_eq!(avatar.get_pixel(1, 1).0, [200, 10, 30, 128]);
    }

    #[test]
    fn high_resolution_skin_uses_the_same_logical_regions() {
        let mut skin = ImageBuffer::from_pixel(128, 128, Rgba([0, 0, 0, 0]));
        for y in 16..32 {
            for x in 16..32 {
                skin.put_pixel(x, y, Rgba([1, 2, 3, 255]));
            }
        }
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(skin)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let avatar = render_avatar(bytes.get_ref()).unwrap();
        assert_eq!(avatar.get_pixel(36, 36).0, [1, 2, 3, 255]);
    }
}

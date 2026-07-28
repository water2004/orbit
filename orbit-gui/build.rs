use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../assets/orbit.svg");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    if let Err(error) = embed_windows_icon() {
        panic!("failed to embed the Orbit GUI icon: {error}");
    }
}

fn embed_windows_icon() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .ok_or("Cargo did not provide CARGO_MANIFEST_DIR to the Orbit GUI build script")?,
    );
    let output_dir =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?);
    let svg = std::fs::read(manifest_dir.join("../assets/orbit.svg"))?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;
    let icon_path = output_dir.join("orbit-gui.ico");
    write_icon(&tree, &icon_path)?;

    let resource_path = output_dir.join("orbit-gui.rc");
    std::fs::write(
        &resource_path,
        format!(
            "1 ICON \"{}\"\n",
            icon_path.display().to_string().replace('\\', "\\\\")
        ),
    )?;
    embed_resource::compile(resource_path, embed_resource::NONE)
        .manifest_required()
        .map_err(|error| format!("Windows resource compiler failed: {error}"))?;
    Ok(())
}

fn write_icon(
    tree: &resvg::usvg::Tree,
    destination: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    let mut images = Vec::with_capacity(SIZES.len());
    for size in SIZES {
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(size, size).ok_or("could not allocate an icon raster")?;
        let scale_x = size as f32 / tree.size().width();
        let scale_y = size as f32 / tree.size().height();
        resvg::render(
            tree,
            resvg::tiny_skia::Transform::from_scale(scale_x, scale_y),
            &mut pixmap.as_mut(),
        );
        images.push((size, pixmap.encode_png()?));
    }

    let directory_bytes = 6 + images.len() * 16;
    let mut offset = u32::try_from(directory_bytes)?;
    let mut file = std::fs::File::create(destination)?;
    file.write_all(&0_u16.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&u16::try_from(images.len())?.to_le_bytes())?;
    for (size, png) in &images {
        file.write_all(&[if *size == 256 {
            0
        } else {
            u8::try_from(*size)?
        }])?;
        file.write_all(&[if *size == 256 {
            0
        } else {
            u8::try_from(*size)?
        }])?;
        file.write_all(&[0, 0])?;
        file.write_all(&1_u16.to_le_bytes())?;
        file.write_all(&32_u16.to_le_bytes())?;
        file.write_all(&u32::try_from(png.len())?.to_le_bytes())?;
        file.write_all(&offset.to_le_bytes())?;
        offset = offset
            .checked_add(u32::try_from(png.len())?)
            .ok_or("icon is too large")?;
    }
    for (_, png) in images {
        file.write_all(&png)?;
    }
    file.flush()?;
    Ok(())
}

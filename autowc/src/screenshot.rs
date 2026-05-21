use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};
use tracing::debug;

pub fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let expected_len = width as usize * height as usize * 4;
    if rgba.len() != expected_len {
        return Err(format!(
            "expected {expected_len} screenshot bytes, got {}",
            rgba.len()
        ));
    }

    debug!(
        path = %display_path(path),
        width,
        height,
        byte_count = rgba.len(),
        "encoding screenshot png"
    );
    let file = File::create(path).map_err(|err| format!("{}: {err}", display_path(path)))?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder
        .write_header()
        .map_err(|err| format!("{}: {err}", display_path(path)))?;
    writer
        .write_image_data(rgba)
        .map_err(|err| format!("{}: {err}", display_path(path)))
}

pub fn display_path(path: &Path) -> String {
    path.to_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| PathBuf::from(path).to_string_lossy().into_owned())
}

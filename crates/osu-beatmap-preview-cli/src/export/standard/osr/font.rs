//! Argon 字体适配器；默认内嵌完整 Torus Bold，显式本地配置可以覆盖。
use osu_beatmap_preview_core::{
    render::{canvas::Img, cpu::modes::standard::osr::font::ArgonFont},
    PreviewError, Result,
};
use std::io::{Cursor, Read};

const FONT: &[u8] = include_bytes!("../../../../../../assets/fonts/argon/Torus-Bold.bin");
const ATLAS: &[u8] = include_bytes!("../../../../../../assets/fonts/argon/Torus-Bold_0.png");
const LABELS: &str = "0123456789.%:/-x GREAT OK MEH MISS REPLAY FAILED SS A B C D F M1 M2 K1 K2";
const MAX_RESOURCE_BYTES: u64 = 4 * 1024 * 1024;

pub(super) fn load() -> Result<Option<ArgonFont>> {
    let directory = &crate::config::current().advance.replay.ARGON_FONT_DIRECTORY;
    if directory.is_empty() {
        return embedded().map(Some);
    }
    let directory = crate::config::resolve_path(directory);
    let bytes = read_resource(&directory.join("Torus-Bold.bin"))?;
    let atlas = read_resource(&directory.join("Torus-Bold_0.png"))?;
    decode(&bytes, &atlas).map(Some)
}

fn read_resource(path: &std::path::Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)
        .map_err(|e| PreviewError::new(format!("ArgonFontUnavailable: {e}")))?;
    let mut bytes = Vec::new();
    file.take(MAX_RESOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PreviewError::new(format!("ArgonFontUnavailable: {e}")))?;
    if bytes.len() as u64 > MAX_RESOURCE_BYTES {
        return Err(PreviewError::new(
            "InvalidArgonFont: resource size limit exceeded",
        ));
    }
    Ok(bytes)
}

fn decode(bytes: &[u8], atlas: &[u8]) -> Result<ArgonFont> {
    // 在 PNG 分配像素前设置尺寸与内存限制；外部覆盖不能绕过内嵌资源的安全边界。
    let mut reader = image::ImageReader::with_format(Cursor::new(atlas), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let atlas = reader
        .decode()
        .map_err(|e| PreviewError::new(format!("ArgonFontUnavailable: {e}")))?
        .to_rgba8();
    let (w, h) = atlas.dimensions();
    let font = ArgonFont::from_bmfont(
        bytes,
        Img {
            w,
            h,
            data: atlas.into_raw(),
        },
    )?;
    if !font.supports(LABELS) {
        return Err(PreviewError::new(
            "InvalidArgonFont: missing required replay glyphs",
        ));
    }
    Ok(font)
}

fn embedded() -> Result<ArgonFont> {
    decode(FONT, ATLAS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_embedded_font_covers_every_replay_label() {
        let font = embedded().unwrap();
        assert!(font
            .supports("0123456789.%:/-x GREAT OK MEH MISS REPLAY FAILED SS A B C D F M1 M2 K1 K2"));
        // 保留完整原资源，防止再次误用只包含 HUD 字符的子集。
        assert!(font.supports("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"));
        assert_eq!(FONT.len(), 150_046);
        assert_eq!(ATLAS.len(), 172_972);
    }
}

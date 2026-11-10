//! 限制 LZMA 字典和解压结果，避免未知长度流或恶意头部造成无限分配。

use crate::support::error::{PreviewError, Result};
use std::io::{Cursor, Write};
pub(super) const MAX_COMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 128 * 1024 * 1024;

/// 对照 StreamReader 的 BOM 检测和替代字符回退；未标记的文本使用 UTF-8。
pub(super) fn text(bytes: &[u8]) -> String {
    if let Some((bytes, little)) = bytes
        .strip_prefix(&[0xff, 0xfe, 0, 0])
        .map(|b| (b, true))
        .or_else(|| bytes.strip_prefix(&[0, 0, 0xfe, 0xff]).map(|b| (b, false)))
    {
        return bytes
            .chunks(4)
            .map(|chunk| {
                let Ok(chunk) = <[u8; 4]>::try_from(chunk) else {
                    return char::REPLACEMENT_CHARACTER;
                };
                char::from_u32(if little {
                    u32::from_le_bytes(chunk)
                } else {
                    u32::from_be_bytes(chunk)
                })
                .unwrap_or(char::REPLACEMENT_CHARACTER)
            })
            .collect();
    }
    if let Some((bytes, little)) = bytes
        .strip_prefix(&[0xff, 0xfe])
        .map(|b| (b, true))
        .or_else(|| bytes.strip_prefix(&[0xfe, 0xff]).map(|b| (b, false)))
    {
        return char::decode_utf16(bytes.chunks(2).map(|chunk| {
            if chunk.len() < 2 {
                0xfffd
            } else if little {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        }))
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    }
    String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)).into_owned()
}

struct LimitedOutput(Vec<u8>);
impl Write for LimitedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_DECODED_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("OSR decompression limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() < 13 {
        return Err(PreviewError::parse("InvalidReplay: short LZMA stream"));
    }
    let dictionary = u32::from_le_bytes(bytes[1..5].try_into().unwrap());
    let size = u64::from_le_bytes(bytes[5..13].try_into().unwrap());
    if dictionary > MAX_COMPRESSED_BYTES as u32
        || (size != u64::MAX && size > MAX_DECODED_BYTES as u64)
    {
        return Err(PreviewError::parse(
            "InvalidReplay: LZMA memory limit exceeded",
        ));
    }
    let mut output = LimitedOutput(Vec::new());
    lzma_rs::lzma_decompress(&mut Cursor::new(bytes), &mut output).map_err(|error| {
        PreviewError::parse(format!("InvalidReplay: LZMA decode failed: {error}"))
    })?;
    Ok(output.0)
}

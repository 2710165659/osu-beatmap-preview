//! OSR 二进制、LZMA、帧与 lazer 扩展 JSON 的纯字节解析。

mod decode;
mod extension;
mod frames;
mod header;
use crate::replay::model::ReplayDocument;
use crate::support::error::{PreviewError, Result};

pub fn parse_osr(bytes: &[u8]) -> Result<ReplayDocument> {
    if bytes.len() > decode::MAX_COMPRESSED_BYTES {
        return Err(PreviewError::parse(
            "InvalidReplay: OSR exceeds the size limit",
        ));
    }
    let mut reader = header::Reader::new(bytes);
    let mut header = reader.header()?;
    let decoded = decode::decompress(reader.byte_array()?)?;
    let (frames, seed) = frames::parse(&decoded)?;
    header.online_id = match header.version {
        20140721.. => reader.i64()?,
        20121008.. => i64::from(reader.i32()?),
        _ => 0,
    };
    // Target Practice 在旧格式尾部另存 double，必须读出后才能定位扩展 JSON。
    if header.mods_bits & (1 << 23) != 0 {
        header.target_practice_accuracy = Some(reader.f64()?);
    }
    let metadata = extension::parse(&mut reader, header.version)?;
    if !reader.is_finished() {
        return Err(PreviewError::parse(
            "InvalidReplay: unexpected trailing OSR data",
        ));
    }
    Ok(ReplayDocument {
        header,
        metadata,
        frames,
        seed,
        diagnostics: Vec::new(),
    })
}

#[cfg(test)]
mod tests;

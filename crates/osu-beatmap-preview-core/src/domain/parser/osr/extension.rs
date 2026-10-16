//! lazer OSR 尾部扩展 JSON 的解析。

use super::{decode, header::Reader};
use crate::replay::model::ReplayMetadata;
use crate::support::error::{PreviewError, Result};

/// 读取指定版本的扩展字段；stable 文件没有该字段。
pub(super) fn parse(reader: &mut Reader<'_>, version: i32) -> Result<Option<ReplayMetadata>> {
    if version < 30_000_001 {
        return Ok(None);
    }
    let compressed = reader.byte_array()?;
    if compressed.is_empty() {
        return Ok(None);
    }
    let json = decode::decompress(compressed)?;
    serde_json::from_slice(&json).map(Some).map_err(|error| {
        PreviewError::parse(format!("InvalidReplay: invalid extension JSON: {error}"))
    })
}

//! 小端字段与 ULEB128 字符串的有界读取；长度检查先于任何分配。

use crate::replay::model::ReplayHeader;
use crate::support::error::{PreviewError, Result};

pub(super) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(invalid_length)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| PreviewError::parse("InvalidReplay: truncated OSR field"))?;
        self.position = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        match self.byte()? {
            0 => return Ok(String::new()),
            0x0b => (),
            _ => return Err(PreviewError::parse("InvalidReplay: invalid string marker")),
        }
        let mut length = 0_usize;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            length |= usize::from(byte & 127) << shift;
            if byte < 128 {
                if length > 1024 * 1024 {
                    return Err(invalid_length());
                }
                return String::from_utf8(self.take(length)?.to_vec())
                    .map_err(|_| PreviewError::parse("InvalidReplay: non-UTF-8 string"));
            }
        }
        Err(invalid_length())
    }
    pub fn byte_array(&mut self) -> Result<&'a [u8]> {
        let length = usize::try_from(self.i32()?).map_err(|_| invalid_length())?;
        if length > super::decode::MAX_COMPRESSED_BYTES {
            return Err(invalid_length());
        }
        self.take(length)
    }
    pub fn is_finished(&self) -> bool {
        self.position == self.bytes.len()
    }
    pub fn header(&mut self) -> Result<ReplayHeader> {
        let ruleset_id = self.byte()?;
        let version = self.i32()?;
        let beatmap_md5 = self.string()?;
        if beatmap_md5.len() != 32 || !beatmap_md5.bytes().all(|v| v.is_ascii_hexdigit()) {
            return Err(PreviewError::parse("InvalidReplay: invalid beatmap MD5"));
        }
        let player = self.string()?;
        let replay_hash = self.string()?;
        let counts = [
            self.u16()?,
            self.u16()?,
            self.u16()?,
            self.u16()?,
            self.u16()?,
            self.u16()?,
        ];
        let total_score = u32::from_le_bytes(self.take(4)?.try_into().unwrap());
        let max_combo = self.u16()?;
        let perfect = match self.byte()? {
            0 => false,
            1 => true,
            _ => return Err(PreviewError::parse("InvalidReplay: invalid boolean")),
        };
        let mods_bits = u32::from_le_bytes(self.take(4)?.try_into().unwrap());
        let life_graph = self.string()?;
        let timestamp_ticks = self.i64()?;
        Ok(ReplayHeader {
            ruleset_id,
            version,
            beatmap_md5,
            player,
            replay_hash,
            counts,
            total_score,
            max_combo,
            perfect,
            mods_bits,
            life_graph,
            timestamp_ticks,
            online_id: 0,
            target_practice_accuracy: None,
        })
    }
}
fn invalid_length() -> PreviewError {
    PreviewError::parse("InvalidReplay: invalid field length")
}

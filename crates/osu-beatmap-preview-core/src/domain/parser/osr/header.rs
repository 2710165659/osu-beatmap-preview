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
    pub fn field<T>(&mut self, name: &str, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        read(self).map_err(|error| match error {
            PreviewError::Parse(reason) => PreviewError::parse(format!("{reason} ({name})")),
            other => other,
        })
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
        if self.byte()? == 0 {
            return Ok(String::new());
        }
        let mut length = 0_usize;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            length |= usize::from(byte & 127) << shift;
            if byte < 128 {
                if length > 1024 * 1024 {
                    return Err(invalid_length());
                }
                return Ok(String::from_utf8_lossy(self.take(length)?).into_owned());
            }
        }
        Err(invalid_length())
    }
    pub fn byte_array(&mut self) -> Result<&'a [u8]> {
        let length = self.i32()?;
        if length <= 0 {
            return Ok(&[]);
        }
        let length = length as usize;
        if length > super::decode::MAX_COMPRESSED_BYTES {
            return Err(invalid_length());
        }
        // BinaryReader.ReadBytes 在末尾允许少于声明长度；是否完整交给解压器判断。
        self.take(length.min(self.remaining()))
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
    pub fn header(&mut self) -> Result<ReplayHeader> {
        let ruleset_id = self.field("ruleset_id", Self::byte)?;
        let version = self.field("version", Self::i32)?;
        let beatmap_md5 = self.field("beatmap_md5", Self::string)?;
        let player = self.field("player", Self::string)?;
        let replay_hash = self.field("replay_hash", Self::string)?;
        let counts = [
            self.field("count300", Self::u16)?,
            self.field("count100", Self::u16)?,
            self.field("count50", Self::u16)?,
            self.field("countGeki", Self::u16)?,
            self.field("countKatu", Self::u16)?,
            self.field("countMiss", Self::u16)?,
        ];
        let total_score = self.field("total_score", Self::i32)?;
        let max_combo = self.field("max_combo", Self::u16)?;
        let perfect = self.field("perfect", Self::byte)? != 0;
        let mods_bits = self.field("mods_bits", Self::i32)? as u32;
        let life_graph = self.field("life_graph", Self::string)?;
        let timestamp_ticks = self.field("timestamp_ticks", Self::i64)?;
        if !(0..=3_155_378_975_999_999_999).contains(&timestamp_ticks) {
            return Err(PreviewError::parse(
                "InvalidReplay: timestamp ticks outside DateTime range",
            ));
        }
        validate_health_graph(&life_graph)?;
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

fn validate_health_graph(graph: &str) -> Result<()> {
    // LegacyScoreDecoder 仅解析每点的前两列 decimal；不限制血量必须在 0..1 内。
    for point in graph.split(',').filter(|p| !p.trim().is_empty()) {
        let mut values = point.split('|');
        for _ in 0..2 {
            let value = values.next().unwrap_or("").trim();
            let value = value
                .strip_prefix(['+', '-'])
                .or_else(|| value.strip_suffix(['+', '-']))
                .unwrap_or(value);
            let mut digits = false;
            let mut dot = false;
            let valid = value.chars().all(|c| {
                if c.is_ascii_digit() {
                    digits = true;
                    true
                } else if c == '.' && !dot {
                    dot = true;
                    true
                } else {
                    false
                }
            });
            let integer = value
                .split('.')
                .next()
                .unwrap_or("")
                .trim_start_matches('0');
            let max = "79228162514264337593543950335";
            let rounds_above_max = integer == max
                && value.split_once('.').is_some_and(|(_, fraction)| {
                    fraction
                        .as_bytes()
                        .first()
                        .is_some_and(|digit| *digit >= b'5')
                });
            if !valid
                || !digits
                || integer.len() > max.len()
                || (integer.len() == max.len() && integer > max)
                || rounds_above_max
            {
                return Err(PreviewError::parse(
                    "InvalidReplay: invalid life graph decimal",
                ));
            }
        }
    }
    Ok(())
}

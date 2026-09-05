//! 共享的 H.264 NAL 单元解析与 MP4 封装辅助函数。
//!
//! 所有编码后端（NVENC、AMF、openh264）都输出 Annex-B 字节流
//!（以起始码标记的 NAL 单元）。本模块将其拆分为 SPS / PPS / slice，
//! 并为 slice 添加长度前缀，以匹配 `mp4` crate 的 `Mp4Sample` 格式。

use crate::domain::errors::{PreviewError, Result};
use std::path::Path;

/// NAL 单元类型是首字节的低 5 位（起始码已由 `nal_units` 移除）。
#[inline]
pub(crate) fn nal_type(nal: &[u8]) -> u8 {
    if nal.is_empty() {
        0
    } else {
        nal[0] & 0x1F
    }
}

/// 移除硬件/软件编码器为每个 NAL 添加的 Annex-B 起始码前缀
///（`00 00 00 01` 或 `00 00 01`）。
#[allow(dead_code)]
pub(crate) fn nal_payload(nal: &[u8]) -> &[u8] {
    if nal.len() >= 4 && nal[0..4] == [0, 0, 0, 1] {
        &nal[4..]
    } else if nal.len() >= 3 && nal[0..3] == [0, 0, 1] {
        &nal[3..]
    } else {
        nal
    }
}

/// 将 Annex-B 字节流拆分为独立的 NAL 单元负载（移除起始码）。
/// 同时处理 4 字节（`00 00 00 01`）和 3 字节（`00 00 01`）起始码，
/// 因为不同编码器可能输出不同变体。
pub(crate) fn split_nals(annexb: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut i = 0;
    let len = annexb.len();
    while i + 3 <= len {
        // 检测起始码。
        let sc_len = if i + 4 <= len && annexb[i..i + 4] == [0, 0, 0, 1] {
            4
        } else if annexb[i..i + 3] == [0, 0, 1] {
            3
        } else {
            i += 1;
            continue;
        };
        let payload_start = i + sc_len;
        // 扫描下一个起始码。
        let mut j = payload_start + 1;
        while j + 2 < len {
            if (j + 4 <= len && annexb[j..j + 4] == [0, 0, 0, 1]) || annexb[j..j + 3] == [0, 0, 1] {
                // 防止将 00 00 00 01 中属于前一个 NAL 的尾部 0 当成起始码，
                // 通过检查真实边界处理（3 字节检查已覆盖 4 字节情况）。
                break;
            }
            j += 1;
        }
        let end = if j + 2 < len { j } else { len };
        // 移除属于 RBSP 停止/填充的尾部零字节。
        let mut nal_end = end;
        while nal_end > payload_start && annexb[nal_end - 1] == 0 {
            // 只移除起始码边界前的尾部零：单个 0x00 可能是合法 RBSP，
            // 而起始码前连续两个 0 通常是填充。仅在满足尾部至少两个零
            // 且确实遇到边界时执行移除。
            if nal_end - payload_start >= 2 && annexb[nal_end - 2] == 0 && end < len {
                nal_end -= 1;
            } else {
                break;
            }
        }
        nals.push(&annexb[payload_start..nal_end]);
        i = end;
    }
    nals
}

/// 从 Annex-B 编码字节流中提取 SPS（类型 7）、PPS（类型 8）和带长度前缀的 slice NAL。
///
/// NAL 类型 6（SEI）、9（AUD）和 12（填充数据）会被静默丢弃，
/// 因为 MP4 封装不需要它们，且部分硬件编码器默认会输出。slice NAL（类型 1–5）
/// 使用大端 4 字节长度前缀拼接，以匹配 `mp4` crate 的 AVC sample 格式。
/// 最后一个返回值表示该访问单元是否包含 IDR slice。
pub(crate) fn extract_nals_from_annexb(
    annexb: &[u8],
) -> (Option<Vec<u8>>, Option<Vec<u8>>, Vec<u8>, bool) {
    let mut sps = None;
    let mut pps = None;
    let mut slice = Vec::new();
    let mut is_keyframe = false;
    for nal in split_nals(annexb) {
        match nal_type(nal) {
            7 => sps = Some(nal.to_vec()),
            8 => pps = Some(nal.to_vec()),
            // 丢弃 SEI(6)、AUD(9)、填充(12)，MP4 封装不需要这些数据。
            6 | 9 | 12 => {}
            _ => {
                is_keyframe |= nal_type(nal) == 5;
                slice.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                slice.extend_from_slice(nal);
            }
        }
    }
    (sps, pps, slice, is_keyframe)
}

#[derive(Debug, Clone, Copy)]
struct Mp4BoxInfo {
    start: usize,
    size: usize,
    end: usize,
    typ: [u8; 4],
}

/// 将 MP4 末尾的 `moov` box 移到 `mdat` 前，使文件无需完整下载即可播放。
/// `moov` 内的块偏移量会增加被移动 box 的大小，确保 `mdat` 前移后仍指向相同媒体数据。
pub(crate) fn make_mp4_faststart(path: &Path) -> Result<()> {
    let data = std::fs::read(path).map_err(|e| {
        PreviewError::render(format!("failed to read mp4 for faststart rewrite: {e}"))
    })?;
    let boxes = top_level_boxes(&data)
        .ok_or_else(|| PreviewError::render("mp4 faststart failed: invalid top-level boxes"))?;
    let ftyp = boxes
        .first()
        .filter(|b| &b.typ == b"ftyp")
        .ok_or_else(|| PreviewError::render("mp4 faststart failed: missing leading ftyp box"))?;
    let moov = boxes
        .iter()
        .find(|b| &b.typ == b"moov")
        .copied()
        .ok_or_else(|| PreviewError::render("mp4 faststart failed: missing moov box"))?;
    let mdat = boxes
        .iter()
        .find(|b| &b.typ == b"mdat")
        .copied()
        .ok_or_else(|| PreviewError::render("mp4 faststart failed: missing mdat box"))?;

    if moov.start < mdat.start {
        return Ok(());
    }

    let mut moov_bytes = data[moov.start..moov.end].to_vec();
    patch_chunk_offsets(&mut moov_bytes, moov.size as u64)?;

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..ftyp.end]);
    out.extend_from_slice(&moov_bytes);
    out.extend_from_slice(&data[ftyp.end..moov.start]);
    out.extend_from_slice(&data[moov.end..]);

    std::fs::write(path, out)
        .map_err(|e| PreviewError::render(format!("failed to write faststart mp4 rewrite: {e}")))
}

fn top_level_boxes(data: &[u8]) -> Option<Vec<Mp4BoxInfo>> {
    let mut boxes = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let b = read_box(data, pos)?;
        pos = b.end;
        boxes.push(b);
    }
    Some(boxes)
}

fn read_box(data: &[u8], start: usize) -> Option<Mp4BoxInfo> {
    if start.checked_add(8)? > data.len() {
        return None;
    }
    let size32 = u32::from_be_bytes(data[start..start + 4].try_into().ok()?);
    let typ: [u8; 4] = data[start + 4..start + 8].try_into().ok()?;
    let size = match size32 {
        0 => data.len().checked_sub(start)?,
        1 => {
            if start.checked_add(16)? > data.len() {
                return None;
            }
            let large = u64::from_be_bytes(data[start + 8..start + 16].try_into().ok()?);
            usize::try_from(large).ok()?
        }
        n => n as usize,
    };
    if size < 8 {
        return None;
    }
    let end = start.checked_add(size)?;
    if end > data.len() {
        return None;
    }
    Some(Mp4BoxInfo {
        start,
        size,
        end,
        typ,
    })
}

fn patch_chunk_offsets(data: &mut [u8], delta: u64) -> Result<()> {
    let mut i = 4;
    while i + 4 <= data.len() {
        if &data[i..i + 4] == b"stco" {
            if let Some(end) = patch_stco(data, i, delta)? {
                i = end;
                continue;
            }
        } else if &data[i..i + 4] == b"co64" {
            if let Some(end) = patch_co64(data, i, delta)? {
                i = end;
                continue;
            }
        }
        i += 1;
    }
    Ok(())
}

fn patch_stco(data: &mut [u8], typ_pos: usize, delta: u64) -> Result<Option<usize>> {
    let Some(info) = read_embedded_box(data, typ_pos) else {
        return Ok(None);
    };
    if info.size < 16 {
        return Ok(None);
    }
    let count_pos = typ_pos + 8;
    let entries_pos = typ_pos + 12;
    let count = u32::from_be_bytes(data[count_pos..count_pos + 4].try_into().unwrap()) as usize;
    if entries_pos
        .checked_add(count.saturating_mul(4))
        .is_none_or(|end| end > info.end)
    {
        return Ok(None);
    }
    for n in 0..count {
        let pos = entries_pos + n * 4;
        let old = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as u64;
        let new = old
            .checked_add(delta)
            .ok_or_else(|| PreviewError::render("mp4 faststart failed: stco offset overflow"))?;
        if new > u32::MAX as u64 {
            return Err(PreviewError::render(
                "mp4 faststart failed: stco offset needs co64 conversion",
            ));
        }
        data[pos..pos + 4].copy_from_slice(&(new as u32).to_be_bytes());
    }
    Ok(Some(info.end))
}

fn patch_co64(data: &mut [u8], typ_pos: usize, delta: u64) -> Result<Option<usize>> {
    let Some(info) = read_embedded_box(data, typ_pos) else {
        return Ok(None);
    };
    if info.size < 16 {
        return Ok(None);
    }
    let count_pos = typ_pos + 8;
    let entries_pos = typ_pos + 12;
    let count = u32::from_be_bytes(data[count_pos..count_pos + 4].try_into().unwrap()) as usize;
    if entries_pos
        .checked_add(count.saturating_mul(8))
        .is_none_or(|end| end > info.end)
    {
        return Ok(None);
    }
    for n in 0..count {
        let pos = entries_pos + n * 8;
        let old = u64::from_be_bytes(data[pos..pos + 8].try_into().unwrap());
        let new = old
            .checked_add(delta)
            .ok_or_else(|| PreviewError::render("mp4 faststart failed: co64 offset overflow"))?;
        data[pos..pos + 8].copy_from_slice(&new.to_be_bytes());
    }
    Ok(Some(info.end))
}

fn read_embedded_box(data: &[u8], typ_pos: usize) -> Option<Mp4BoxInfo> {
    let start = typ_pos.checked_sub(4)?;
    let info = read_box(data, start)?;
    if info.start + 4 == typ_pos {
        Some(info)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_nals_handles_4byte_startcodes() {
        // 两个 NAL：SPS（类型 7）+ slice（类型 5）。
        let annexb: &[u8] = &[
            0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1e, // SPS
            0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00, // IDR slice
        ];
        let nals = split_nals(annexb);
        assert_eq!(nals.len(), 2);
        assert_eq!(nal_type(nals[0]), 7);
        assert_eq!(nal_type(nals[1]), 5);
    }

    #[test]
    fn split_nals_handles_3byte_startcodes() {
        let annexb: &[u8] = &[
            0, 0, 1, 0x67, 0x42, // SPS
            0, 0, 1, 0x68, 0xCE, // PPS
        ];
        let nals = split_nals(annexb);
        assert_eq!(nals.len(), 2);
        assert_eq!(nal_type(nals[0]), 7);
        assert_eq!(nal_type(nals[1]), 8);
    }

    #[test]
    fn extract_drops_sei_aud_filler() {
        // SPS + SEI + AUD + slice。
        let annexb: &[u8] = &[
            0, 0, 0, 1, 0x67, 0x01, // SPS (type 7)
            0, 0, 0, 1, 0x06, 0x02, // SEI (type 6)
            0, 0, 0, 1, 0x09, 0x10, // AUD (type 9)
            0, 0, 0, 1, 0x65, 0xAA, // slice (type 5)
        ];
        let (sps, pps, slice, is_keyframe) = extract_nals_from_annexb(annexb);
        assert_eq!(sps, Some(vec![0x67, 0x01]));
        assert_eq!(pps, None);
        assert!(is_keyframe);
        // slice 应带长度前缀：4 字节大端长度 + 2 字节负载。
        assert_eq!(slice, vec![0, 0, 0, 2, 0x65, 0xAA]);
    }

    #[test]
    fn extract_reports_non_idr_slice_as_non_keyframe() {
        let annexb: &[u8] = &[0, 0, 0, 1, 0x41, 0xAA];
        let (_, _, slice, is_keyframe) = extract_nals_from_annexb(annexb);
        assert!(!is_keyframe);
        assert_eq!(slice, vec![0, 0, 0, 2, 0x41, 0xAA]);
    }

    #[test]
    fn faststart_moves_moov_before_mdat_and_patches_stco() {
        let dir = std::env::temp_dir().join(format!("osu-preview-mux-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tail-moov.mp4");

        let mut data = Vec::new();
        data.extend_from_slice(&box_bytes(b"ftyp", &[0u8; 16]));
        data.extend_from_slice(&box_bytes(b"mdat", &[1, 2, 3, 4]));
        let stco = stco_box(&[36]);
        let moov = box_bytes(b"moov", &stco);
        data.extend_from_slice(&moov);
        std::fs::write(&path, data).unwrap();

        make_mp4_faststart(&path).unwrap();
        let rewritten = std::fs::read(&path).unwrap();
        assert_eq!(&rewritten[4..8], b"ftyp");
        assert_eq!(&rewritten[28..32], b"moov");
        let mdat_type_pos = 24 + moov.len() + 4;
        assert_eq!(&rewritten[mdat_type_pos..mdat_type_pos + 4], b"mdat");

        let stco_pos = rewritten.windows(4).position(|w| w == b"stco").unwrap();
        let offset_pos = stco_pos + 12;
        let patched = u32::from_be_bytes(rewritten[offset_pos..offset_pos + 4].try_into().unwrap());
        assert_eq!(patched, 36 + moov.len() as u32);

        let _ = std::fs::remove_file(path);
    }

    fn box_bytes(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(typ);
        out.extend_from_slice(payload);
        out
    }

    fn stco_box(offsets: &[u32]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&[0u8; 4]); // version + flags
        payload.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for offset in offsets {
            payload.extend_from_slice(&offset.to_be_bytes());
        }
        box_bytes(b"stco", &payload)
    }
}

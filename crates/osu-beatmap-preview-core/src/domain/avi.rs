//! AVI（RIFF）容器解封装：取出 H.264 视频轨的样本与时间轴。
//!
//! osu! 谱面的背景视频常见两种容器：MP4 与 AVI（`Video` 事件直接指向 `.avi`，
//! 多为老谱面）。AVI 是 RIFF 家族的古董格式，本模块只做「拆容器」这一件事：
//! - 解析 `hdrl`（`avih` / `strl`）拿编码、分辨率与帧率；
//! - 走 `movi` 列出每个视频样本的字节区间（`rec ` 交错分组同样要认）；
//! - 扫样本里的 NAL 单元区分「真实画面 / 重复帧占位 / IDR 关键帧」，并提取 SPS/PPS。
//!
//! 不做解码：CLI 侧把样本喂给 openh264，Web 侧重封装成 MP4 交给浏览器硬解。
//!
//! ## 两个容易踩的坑
//!
//! - **空样本**：`movi` 里零长度的视频 chunk 是「重复上一帧」占位（常见于帧率
//!   转换产物，一半槽位都可能是空的）。它占一个显示槽位，但不产生解码画面——
//!   取画面时要跳过，算时间时要保留。
//! - **NAL 布局**：AVI 里的 H.264 帧既可能是 Annex-B（起始码分隔，最常见），
//!   也可能照搬 MP4 的 4 字节长度前缀；[`split_sample_nals`] 两种都认。

use crate::domain::errors::{PreviewError, Result};

/// AVI 里的一个视频样本（= `movi` 里的一个视频 chunk，占一个显示槽位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AviSample {
    /// 样本字节在文件里的 `[start, end)` 区间；空样本两者相等。
    pub start: usize,
    pub end: usize,
    /// 是否含 VCL NAL（真实画面）。空样本（重复帧占位）为 `false`。
    pub has_frame: bool,
    /// 是否 IDR 关键帧：解码段的合法起点（段间可并行解码）。
    pub is_sync: bool,
}

/// AVI 视频轨索引：容器解析结果，配合原始文件字节随机取样本。
#[derive(Debug, Clone)]
pub struct AviVideo {
    pub width: u32,
    pub height: u32,
    /// 每帧时长 = `frame_scale / frame_rate` 秒（AVI 是恒定帧率）。
    pub frame_scale: u32,
    pub frame_rate: u32,
    /// 全部视频样本（含重复帧占位），下标即显示槽位。
    pub samples: Vec<AviSample>,
    /// SPS/PPS（裸 NAL，不含起始码）；重封装与并行解码段开头都要用。
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

impl AviVideo {
    /// 显示槽位 `index` 的显示时间（毫秒）：恒定帧率，槽位即显示序号。
    ///
    /// 重复帧占位也占槽位，所以这里用**样本下标**而不是「第几个画面」推时间。
    pub fn sample_time_ms(&self, index: usize) -> i64 {
        let numerator = index as u64 * self.frame_scale as u64 * 1000;
        (numerator / self.frame_rate.max(1) as u64) as i64
    }

    /// 视频总时长（毫秒）：全部槽位（含重复帧占位）之和。
    pub fn duration_ms(&self) -> i64 {
        self.sample_time_ms(self.samples.len())
    }

    /// 样本字节（配合构造时的文件字节使用）。
    pub fn sample_bytes<'a>(&self, index: usize, bytes: &'a [u8]) -> Option<&'a [u8]> {
        let sample = self.samples.get(index)?;
        bytes.get(sample.start..sample.end)
    }
}

/// 解析 AVI 容器，得到 H.264 视频轨索引。
///
/// 只支持 H.264 视频轨（H.264 之外的 AVI——如 XviD/WMV——直接报错，由调用方
/// 回退静态背景）；音频轨、索引块等一律忽略。
pub fn parse_avi(bytes: &[u8]) -> Result<AviVideo> {
    let mut headers = Headers::default();
    let mut ranges: Vec<(usize, usize)> = Vec::new();

    // 顶层可以有多个 RIFF 块：首个 `AVI ` 含 hdrl+movi，OpenDML 的 `AVIX`
    // 续块再补 movi。位置非法时停在当前块（宁可少收样本，不越界）。
    let mut pos = 0_usize;
    let mut forms = 0_usize;
    while pos + 12 <= bytes.len() {
        if &bytes[pos..pos + 4] != b"RIFF" {
            break;
        }
        let size = read_u32_le(bytes, pos + 4)? as usize;
        let Some(form_end) = (pos + 8).checked_add(size).filter(|end| *end <= bytes.len()) else {
            break;
        };
        let form = &bytes[pos + 8..pos + 12];
        if forms == 0 && form != b"AVI " {
            return Err(PreviewError::parse(
                "background video is not an AVI (RIFF) file",
            ));
        }
        forms += 1;
        scan_region(bytes, pos + 12, form_end, &mut headers, &mut ranges)?;
        pos = form_end + (size & 1);
    }
    if forms == 0 {
        return Err(PreviewError::parse(
            "background video is not an AVI (RIFF) file",
        ));
    }

    // 帧率：strh 的 scale/rate 是权威值，avih 的 usecPerFrame 兜底。
    let (frame_scale, frame_rate) = if headers.scale > 0 && headers.rate > 0 {
        (headers.scale, headers.rate)
    } else if headers.usec_per_frame > 0 {
        (headers.usec_per_frame, 1_000_000)
    } else {
        return Err(PreviewError::parse(
            "AVI background video has no usable frame timing",
        ));
    };
    let (width, height) = (headers.width, headers.height);
    if width == 0 || height == 0 {
        return Err(PreviewError::parse(
            "AVI background video has invalid dimensions",
        ));
    }
    if ranges.is_empty() {
        return Err(PreviewError::parse(
            "AVI background video has no video samples",
        ));
    }

    // 逐样本扫 NAL：区分真实画面 / 重复帧占位 / IDR，顺带兜底提取 SPS/PPS。
    let (mut sps, mut pps) = headers.avcc_parameter_sets().unwrap_or_default();
    let mut samples = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        let mut sample = AviSample {
            start,
            end,
            has_frame: false,
            is_sync: false,
        };
        // 解析失败的样本按「无画面」处理：它不产生解码输出，跳过即可。
        for nal in split_sample_nals(bytes.get(start..end).unwrap_or_default()).unwrap_or_default() {
            match nal_type(nal) {
                1..=5 => {
                    sample.has_frame = true;
                    sample.is_sync |= nal_type(nal) == 5;
                }
                7 => {
                    if sps.is_empty() {
                        sps = nal.to_vec();
                    }
                }
                8 => {
                    if pps.is_empty() {
                        pps = nal.to_vec();
                    }
                }
                _ => {}
            }
        }
        samples.push(sample);
    }
    if sps.is_empty() || pps.is_empty() {
        return Err(PreviewError::parse(
            "AVI background video is not H.264 (missing SPS/PPS)",
        ));
    }
    if !samples.iter().any(|sample| sample.has_frame) {
        return Err(PreviewError::parse(
            "AVI background video has no decodable H.264 frames",
        ));
    }

    Ok(AviVideo {
        width,
        height,
        frame_scale,
        frame_rate,
        samples,
        sps,
        pps,
    })
}

/// `hdrl` 里的视频流头信息。
#[derive(Default)]
struct Headers {
    /// `avih`：每帧微秒数（帧率兜底）。
    usec_per_frame: u32,
    /// `strf` 的 BITMAPINFOHEADER 宽高（可能为负，取绝对值）。
    width: u32,
    height: u32,
    /// `strh` 的 dwScale/dwRate：每帧 = scale/rate 秒。
    scale: u32,
    rate: u32,
    /// `strf` 尾部扩展：H.264 时可能是 AVCDecoderConfigurationRecord。
    extradata: Vec<u8>,
}

impl Headers {
    /// 扩展数据是 avcC 时取出 SPS/PPS；老 AVI 通常没有，靠样本内嵌兜底。
    fn avcc_parameter_sets(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        parse_avc_parameter_sets(&self.extradata)
    }
}

/// 扫描一个 RIFF 区间：`hdrl` 解析头信息，`movi`/`rec ` 收集视频样本。
fn scan_region(
    bytes: &[u8],
    start: usize,
    end: usize,
    headers: &mut Headers,
    ranges: &mut Vec<(usize, usize)>,
) -> Result<()> {
    let mut pos = start;
    while pos + 8 <= end {
        let id = &bytes[pos..pos + 4];
        let size = read_u32_le(bytes, pos + 4)? as usize;
        let content = pos + 8;
        // 尺寸越界按截断容错：收下已读到的部分，不为单个坏块放弃整个文件。
        let content_end = content
            .checked_add(size)
            .filter(|candidate| *candidate <= end)
            .unwrap_or(end);
        if id == b"LIST" {
            if content + 4 <= content_end {
                match &bytes[content..content + 4] {
                    b"hdrl" => scan_hdrl(bytes, content + 4, content_end, headers)?,
                    // `rec ` 是交错 AVI 的样本分组，与 movi 同构，继续往里走。
                    b"movi" | b"rec " => {
                        scan_region(bytes, content + 4, content_end, headers, ranges)?
                    }
                    _ => {}
                }
            }
        } else if id[2..4] == *b"dc" || id[2..4] == *b"db" {
            // `NNdc`/`NNdb` 是视频 chunk；空 chunk（重复帧占位）也要占槽位。
            ranges.push((content, content_end));
        }
        pos = content_end + (size & 1);
    }
    Ok(())
}

/// 解析 `hdrl`：`avih` 给兜底帧率与分辨率，`strl` 里的视频流给权威值。
fn scan_hdrl(bytes: &[u8], start: usize, end: usize, headers: &mut Headers) -> Result<()> {
    let mut pos = start;
    while pos + 8 <= end {
        let id = &bytes[pos..pos + 4];
        let size = read_u32_le(bytes, pos + 4)? as usize;
        let content = pos + 8;
        let content_end = content
            .checked_add(size)
            .filter(|candidate| *candidate <= end)
            .unwrap_or(end);
        if id == b"avih" && content + 40 <= content_end {
            headers.usec_per_frame = read_u32_le(bytes, content)?;
            if headers.width == 0 {
                headers.width = read_u32_le(bytes, content + 32)?;
                headers.height = read_u32_le(bytes, content + 36)?;
            }
        } else if id == b"LIST" && content + 4 <= content_end && &bytes[content..content + 4] == b"strl"
        {
            scan_strl(bytes, content + 4, content_end, headers)?;
        }
        pos = content_end + (size & 1);
    }
    Ok(())
}

/// 解析视频流的 `strl`：`strh` 给帧率，`strf` 给分辨率与编码扩展数据。
fn scan_strl(bytes: &[u8], start: usize, end: usize, headers: &mut Headers) -> Result<()> {
    let mut pos = start;
    let mut is_video_stream = false;
    while pos + 8 <= end {
        let id = &bytes[pos..pos + 4];
        let size = read_u32_le(bytes, pos + 4)? as usize;
        let content = pos + 8;
        let content_end = content
            .checked_add(size)
            .filter(|candidate| *candidate <= end)
            .unwrap_or(end);
        if id == b"strh" && content + 40 <= content_end {
            // fccType == "vids" 才是视频流；只认第一个（AVI 单视频轨）。
            if &bytes[content..content + 4] == b"vids" && headers.scale == 0 && headers.rate == 0 {
                is_video_stream = true;
                headers.scale = read_u32_le(bytes, content + 20)?;
                headers.rate = read_u32_le(bytes, content + 24)?;
            }
        } else if id == b"strf" && is_video_stream {
            if content + 40 <= content_end {
                // BITMAPINFOHEADER：biWidth/biHeight 可能为负（自上而下位图）。
                let width = read_i32_le(bytes, content + 4)?.unsigned_abs();
                let height = read_i32_le(bytes, content + 8)?.unsigned_abs();
                headers.width = width;
                headers.height = height;
            }
            if content_end > content + 40 {
                headers.extradata = bytes[content + 40..content_end].to_vec();
            }
        }
        pos = content_end + (size & 1);
    }
    Ok(())
}

/// 解析 `AVCDecoderConfigurationRecord`（avcC）里的 SPS/PPS；结构不对返回 `None`。
fn parse_avc_parameter_sets(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if data.len() < 8 || data[0] != 1 {
        return None;
    }
    // 4: configurationVersion/Profile/Compat/Level，5: 长度大小，6: SPS 个数。
    let mut pos = 6;
    let sps_count = data.get(5)? & 0x1F;
    let mut sps = Vec::new();
    for index in 0..sps_count {
        let length = u16::from_be_bytes([*data.get(pos)?, *data.get(pos + 1)?]) as usize;
        pos += 2;
        let nal = data.get(pos..pos + length)?;
        if index == 0 {
            sps = nal.to_vec();
        }
        pos += length;
    }
    let pps_count = *data.get(pos)?;
    pos += 1;
    let mut pps = Vec::new();
    for index in 0..pps_count {
        let length = u16::from_be_bytes([*data.get(pos)?, *data.get(pos + 1)?]) as usize;
        pos += 2;
        let nal = data.get(pos..pos + length)?;
        if index == 0 {
            pps = nal.to_vec();
        }
        pos += length;
    }
    (!sps.is_empty() && !pps.is_empty()).then_some((sps, pps))
}

/// 切分一个样本里的 NAL 单元（裸负载，不含起始码/长度前缀）。
///
/// AVI 样本既可能是 Annex-B（起始码分隔）也可能是 4 字节长度前缀（MP4 sample
/// 同款），按样本开头判断布局；长度前缀的样本必须严丝合缝铺满，否则视为损坏
/// 返回 `None`（调用方停止解码本段）。空样本返回空列表（重复帧占位）。
pub fn split_sample_nals(sample: &[u8]) -> Option<Vec<&[u8]>> {
    if sample.is_empty() {
        return Some(Vec::new());
    }
    if start_code_len(sample).is_some() {
        return Some(split_annexb(sample));
    }
    split_length_prefixed(sample)
}

/// NAL 单元类型（首字节低 5 位）；空负载记 0。
pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map_or(0, |header| header & 0x1F)
}

/// NAL 的 `nal_ref_idc`（首字节 6~5 位）：0 表示非参考帧（B 帧特征）。
pub fn nal_ref_idc(nal: &[u8]) -> u8 {
    nal.first().map_or(0, |header| (header >> 5) & 0x03)
}

/// 起始码长度：`00 00 00 01`（4）或 `00 00 01`（3）。
fn start_code_len(data: &[u8]) -> Option<usize> {
    match data {
        [0, 0, 0, 1, ..] => Some(4),
        [0, 0, 1, ..] => Some(3),
        _ => None,
    }
}

/// Annex-B 样本切分：按起始码取负载；起始码前的尾部零字节属于流填充，保留。
fn split_annexb(sample: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut pos = 0;
    while let Some(sc) = start_code_len(&sample[pos..]) {
        let payload_start = pos + sc;
        // 找下一个起始码：3 字节检查覆盖 4 字节（`00 00 00 01` 含 `00 00 01`），
        // 但要多留一拍，让 4 字节起始码按 4 字节对齐切。
        let mut next = payload_start;
        while next < sample.len() {
            if start_code_len(&sample[next..]).is_some() {
                break;
            }
            next += 1;
        }
        // 尾部残留的空起始码不是 NAL，跳过。
        if payload_start < next {
            nals.push(&sample[payload_start..next]);
        }
        if next >= sample.len() {
            break;
        }
        pos = next;
    }
    nals
}

/// 长度前缀样本切分：4 字节大端长度必须恰好铺满整个样本。
fn split_length_prefixed(sample: &[u8]) -> Option<Vec<&[u8]>> {
    let mut nals = Vec::new();
    let mut pos = 0;
    while pos < sample.len() {
        let length = u32::from_be_bytes(sample.get(pos..pos + 4)?.try_into().ok()?) as usize;
        pos += 4;
        if length == 0 || length > sample.len() - pos {
            return None;
        }
        nals.push(&sample[pos..pos + length]);
        pos += length;
    }
    (!nals.is_empty()).then_some(nals)
}

fn read_u32_le(bytes: &[u8], pos: usize) -> Result<u32> {
    let raw = bytes
        .get(pos..pos + 4)
        .ok_or_else(|| PreviewError::parse("truncated AVI header"))?;
    Ok(u32::from_le_bytes(raw.try_into().expect("长度固定")))
}

fn read_i32_le(bytes: &[u8], pos: usize) -> Result<i32> {
    let raw = bytes
        .get(pos..pos + 4)
        .ok_or_else(|| PreviewError::parse("truncated AVI header"))?;
    Ok(i32::from_le_bytes(raw.try_into().expect("长度固定")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一段 H.264 样本字节：Annex-B 或长度前缀布局。
    fn frame_bytes(nals: &[&[u8]], annexb: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            if annexb {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(nal);
            } else {
                out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                out.extend_from_slice(nal);
            }
        }
        out
    }

    fn chunk(id: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(id);
        out.extend_from_slice(&(content.len() as u32).to_le_bytes());
        out.extend_from_slice(content);
        if content.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(list_type: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut inner = Vec::from(*list_type);
        inner.extend_from_slice(content);
        chunk(b"LIST", &inner)
    }

    /// 最小可用 AVI：hdrl（avih + 视频 strl）+ movi（按帧表逐槽位放 chunk）+ idx1。
    fn build_avi(frames: &[Option<Vec<u8>>], scale: u32, rate: u32) -> Vec<u8> {
        let avih = {
            let mut content = vec![0_u8; 56];
            content[0..4].copy_from_slice(&20_833_u32.to_le_bytes());
            content[16..20].copy_from_slice(&(frames.len() as u32).to_le_bytes());
            content[24..28].copy_from_slice(&1_u32.to_le_bytes());
            content[32..36].copy_from_slice(&1280_u32.to_le_bytes());
            content[36..40].copy_from_slice(&720_u32.to_le_bytes());
            chunk(b"avih", &content)
        };
        let strh = {
            let mut content = vec![0_u8; 56];
            content[0..4].copy_from_slice(b"vids");
            content[4..8].copy_from_slice(b"H264");
            content[20..24].copy_from_slice(&scale.to_le_bytes());
            content[24..28].copy_from_slice(&rate.to_le_bytes());
            content[32..36].copy_from_slice(&(frames.len() as u32).to_le_bytes());
            chunk(b"strh", &content)
        };
        let strf = {
            let mut content = vec![0_u8; 40];
            content[0..4].copy_from_slice(&40_u32.to_le_bytes());
            content[4..8].copy_from_slice(&1280_i32.to_le_bytes());
            content[8..12].copy_from_slice(&720_i32.to_le_bytes());
            content[16..20].copy_from_slice(b"H264");
            chunk(b"strf", &content)
        };
        let strl = list(b"strl", &[strh, strf].concat());
        let hdrl = list(b"hdrl", &[avih, strl].concat());
        let mut movi_content = Vec::new();
        for frame in frames {
            movi_content.extend_from_slice(&chunk(b"00dc", frame.as_deref().unwrap_or_default()));
        }
        let movi = list(b"movi", &movi_content);
        let idx1 = chunk(b"idx1", &[]);
        let mut body = Vec::from(*b"AVI ");
        body.extend_from_slice(&hdrl);
        body.extend_from_slice(&movi);
        body.extend_from_slice(&idx1);
        let mut out = Vec::from(*b"RIFF");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x1F, 0xAC];
    const PPS: &[u8] = &[0x68, 0xEB, 0xE0, 0xB4];
    const IDR: &[u8] = &[0x65, 0x88, 0x84];
    const P_SLICE: &[u8] = &[0x41, 0x9B, 0x91];
    const B_SLICE: &[u8] = &[0x01, 0xA9, 0x9F];

    /// 容器解析：尺寸、帧率、样本槽位与画面/关键帧标记。
    #[test]
    fn parses_video_stream_with_repeat_placeholders() {
        let idr = frame_bytes(&[SPS, PPS, IDR], true);
        let p = frame_bytes(&[P_SLICE], true);
        let b = frame_bytes(&[B_SLICE], true);
        let frames = vec![
            Some(idr),
            None,
            Some(b),
            None,
            Some(p.clone()),
            Some(Vec::new()),
            Some(p),
        ];
        let avi = build_avi(&frames, 1001, 48_000);
        let video = parse_avi(&avi).expect("AVI 必须可解析");

        assert_eq!((video.width, video.height), (1280, 720));
        assert_eq!((video.frame_scale, video.frame_rate), (1001, 48_000));
        assert_eq!(video.samples.len(), 7);
        assert_eq!(video.sps, SPS);
        assert_eq!(video.pps, PPS);
        // 槽位 0：IDR（真实画面 + 关键帧）；槽位 1/3：重复帧占位；槽位 5：空 chunk。
        assert!(video.samples[0].has_frame && video.samples[0].is_sync);
        assert!(!video.samples[1].has_frame && !video.samples[1].is_sync);
        assert!(video.samples[2].has_frame && !video.samples[2].is_sync);
        assert!(!video.samples[5].has_frame);
        assert!(video.samples[6].has_frame);
        // 时间按槽位算：48000/1001 ≈ 47.95fps。
        assert_eq!(video.sample_time_ms(0), 0);
        assert_eq!(video.sample_time_ms(1), 20);
        assert_eq!(video.duration_ms(), video.sample_time_ms(7));
    }

    /// 长度前缀布局的样本同样可解析（MP4 sample 同款）。
    #[test]
    fn parses_length_prefixed_samples() {
        let idr = frame_bytes(&[SPS, PPS, IDR], false);
        let p = frame_bytes(&[P_SLICE], false);
        let avi = build_avi(&[Some(idr), Some(p)], 1, 24);
        let video = parse_avi(&avi).expect("AVI 必须可解析");
        assert_eq!(video.samples.len(), 2);
        assert!(video.samples[0].is_sync);
        assert!(!video.samples[1].is_sync);
        assert!(video.samples[1].has_frame);
    }

    /// 没有 SPS/PPS 的 AVI（非 H.264）直接报错，不猜。
    #[test]
    fn rejects_non_h264_streams() {
        let bogus = frame_bytes(&[&[0x21, 0x00]], true);
        let avi = build_avi(&[Some(bogus)], 1, 24);
        let error = parse_avi(&avi).expect_err("非 H.264 必须报错");
        assert!(error.to_string().contains("H.264"), "{error}");
    }

    /// 非 AVI 字节（如 mp4）不认。
    #[test]
    fn rejects_non_riff_files() {
        let error = parse_avi(&[0, 0, 0, 24, b'f', b't', b'y', b'p']).expect_err("必须报错");
        assert!(error.to_string().contains("AVI"), "{error}");
        assert!(parse_avi(&[]).is_err());
    }

    /// NAL 切分：Annex-B（含 3/4 字节混合起始码）与长度前缀两种布局。
    #[test]
    fn splits_annexb_and_length_prefixed_samples() {
        let mixed = [0, 0, 0, 1, 0x67, 0x01, 0, 0, 1, 0x65, 0x02, 0x03];
        let nals = split_sample_nals(&mixed).expect("Annex-B 必须可切分");
        assert_eq!(nals, vec![&[0x67, 0x01][..], &[0x65, 0x02, 0x03][..]]);
        assert_eq!(nal_type(nals[0]), 7);
        assert_eq!(nal_type(nals[1]), 5);

        let prefixed = frame_bytes(&[&[0x67, 0x01], &[0x65, 0x02]], false);
        assert_eq!(
            split_sample_nals(&prefixed).expect("长度前缀必须可切分"),
            vec![&[0x67, 0x01][..], &[0x65, 0x02][..]]
        );

        assert_eq!(split_sample_nals(&[]), Some(Vec::new()));
        // 长度前缀不铺满视为损坏。
        assert_eq!(split_sample_nals(&[0, 0, 0, 9, 0x41]), None);
    }

    /// avcC 扩展数据里的 SPS/PPS 优先于样本内嵌。
    #[test]
    fn avcc_parameter_sets_take_precedence() {
        let mut avcc = vec![1, 0x64, 0x00, 0x1F, 0xFF, 0xE1];
        avcc.extend_from_slice(&(SPS.len() as u16).to_be_bytes());
        avcc.extend_from_slice(SPS);
        avcc.push(1);
        avcc.extend_from_slice(&(PPS.len() as u16).to_be_bytes());
        avcc.extend_from_slice(PPS);
        assert_eq!(
            parse_avc_parameter_sets(&avcc),
            Some((SPS.to_vec(), PPS.to_vec()))
        );
        assert_eq!(parse_avc_parameter_sets(&[0, 1, 2, 3]), None);
    }
}

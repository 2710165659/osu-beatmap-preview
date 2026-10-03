//! AVI 背景视频 → MP4 重封装：H.264 只换容器，不转码。
//!
//! 浏览器 `<video>` 认 MP4/WebM，但不认 AVI(RIFF)——老谱面的背景视频恰恰是
//! `.avi`（osu! 自己靠 DirectShow 播）。好在这些 AVI 里装的就是 H.264，
//! 浏览器的硬解码器完全解得动，缺的只是容器转换：这里把样本原样搬进 MP4，
//! `moov` 前置（faststart），Blob 播放立刻可 seek。
//!
//! ## 显示顺序：为什么必须读码流
//!
//! H.264 有 B 帧时**解码顺序 ≠ 显示顺序**，MP4 靠 `ctts`（合成偏移）告诉播放器
//! 谁先谁后；而 AVI 里没有任何重排元数据。唯一的权威信息在码流的 POC
//! （`pic_order_cnt`）里，所以要读 SPS/PPS 与每个画面首个 slice 的头，按
//! H.264 8.2.1.1 的推导恢复 POC，再把 POC 排序得到显示顺序。CLI 导出侧不需要
//! 这一步——软解码器直接按显示序吐画面，取帧按序对齐即可。
//!
//! ## 时间轴：空槽位是「重复上一帧」
//!
//! AVI 是恒定帧率，`movi` 里的零长度视频 chunk（帧率转换产物）占一个显示槽位
//! 但不产生画面。重封装时这些槽位不进轨道（无法凭空造解码画面），时间轴靠
//! 两处补偿：画面的 PTS 仍按原始槽位算（空档由播放器沿用上一帧），轨道总时长
//! 补到最后一个样本的时长上。

use osu_beatmap_preview_core::processing::avi::{
    nal_ref_idc, nal_type, parse_avi, split_sample_nals,
};

/// 文件是否是 AVI（RIFF/AVI ）容器。
pub fn is_avi(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"AVI ")
}

/// AVI → MP4 重封装（H.264 不转码）；输入必须是 AVI 字节。
pub fn remux_avi_to_mp4(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let avi = parse_avi(bytes).map_err(|error| error.to_string())?;
    // 画面 = 含 VCL NAL 的样本（重复帧占位不进轨道），下标即解码顺序。
    let slots: Vec<usize> = avi
        .samples
        .iter()
        .enumerate()
        .filter(|(_, sample)| sample.has_frame)
        .map(|(index, _)| index)
        .collect();
    if slots.is_empty() {
        return Err("AVI 背景视频没有可解码的画面".to_string());
    }

    let sps_params = parse_sps(&avi.sps).ok_or("无法解析 AVI 背景视频的 SPS")?;
    let bottom_field_poc = pps_bottom_field_poc(&avi.pps).unwrap_or(false);
    let frames: Vec<&[u8]> = slots
        .iter()
        .map(|&slot| bytes.get(avi.samples[slot].start..avi.samples[slot].end))
        .collect::<Option<_>>()
        .ok_or("AVI 背景视频样本越界")?;
    let pocs = picture_pocs(&frames, &sps_params, bottom_field_poc);
    let ranks = display_ranks(&pocs);

    // 每帧时长 = frame_scale 刻度；PTS 按原始槽位（含重复帧留下的空档）。
    let frame_scale = avi.frame_scale;
    let samples = frames
        .iter()
        .map(|frame| sample_payload(frame))
        .collect::<Option<Vec<_>>>()
        .ok_or("AVI 背景视频样本结构损坏")?;
    let pts_offsets = ranks
        .iter()
        .enumerate()
        .map(|(index, &rank)| {
            // PTS 按原始槽位、DTS 按解码序；B 帧的 PTS 早于 DTS，允许为负。
            // ctts 条目本身就是 i32 字段，刻度换算后再落位。
            ((slots[rank] as i64 - index as i64) * i64::from(frame_scale)) as i32
        })
        .collect::<Vec<_>>();
    let mut durations = vec![frame_scale; slots.len()];
    // 轨道总时长要盖住全部槽位（含尾部重复帧），差额记到最后一个样本上。
    durations[slots.len() - 1] = (avi.samples.len() - slots.len() + 1) as u32 * frame_scale;
    let sync = ranks
        .iter()
        .enumerate()
        .filter(|(_, &rank)| avi.samples[slots[rank]].is_sync)
        .map(|(index, _)| index as u32 + 1)
        .collect();

    build_mp4(&Track {
        width: avi.width,
        height: avi.height,
        timescale: avi.frame_rate,
        durations,
        pts_offsets,
        sync,
        samples,
        sps: avi.sps,
        pps: avi.pps,
    })
}

/// 重封装目标轨道：样本已是「长度前缀 VCL NAL」格式，时间刻度 = `timescale`。
struct Track {
    width: u32,
    height: u32,
    timescale: u32,
    /// 每样本的 stts 时长（刻度）。
    durations: Vec<u32>,
    /// 每样本的 ctts 合成偏移（PTS − DTS，刻度，可为负）。
    pts_offsets: Vec<i32>,
    /// 关键帧样本号（1 起）。
    sync: Vec<u32>,
    samples: Vec<Vec<u8>>,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

/// 一个画面的 slice 头信息（POC 推导所需）。
struct PocInfo {
    /// `pic_order_cnt_lsb`（POC type 0）；其余 type 记 0。
    poc_lsb: u32,
    is_idr: bool,
    nal_ref_idc: u8,
}

/// SPS 里与 slice 头/POC 推导相关的字段。
#[derive(Debug, Clone, Copy)]
struct SpsParams {
    /// `log2_max_frame_num`：slice 头里 `frame_num` 的位宽。
    log2_max_frame_num: u32,
    poc_type: u32,
    /// `log2_max_pic_order_cnt_lsb`（POC type 0）。
    log2_max_poc_lsb: u32,
    frame_mbs_only: bool,
}

/// 解析 SPS 中 POC 推导需要的字段；结构不对（非预期的流）返回 `None`。
fn parse_sps(nal: &[u8]) -> Option<SpsParams> {
    let rbsp = remove_emulation(nal.get(1..)?);
    let mut reader = BitReader::new(&rbsp);
    let profile_idc = reader.read_bits(8)?;
    reader.read_bits(8)?; // constraint_set flags
    reader.read_bits(8)?; // level_idc
    reader.read_ue()?; // seq_parameter_set_id
    // High 系 profile 多一组色度/位深/缩放矩阵字段。
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        if reader.read_ue()? == 3 {
            reader.read_flag()?; // separate_colour_plane_flag
        }
        reader.read_ue()?; // bit_depth_luma_minus8
        reader.read_ue()?; // bit_depth_chroma_minus8
        reader.read_flag()?; // qpprime_y_zero_transform_bypass_flag
        if reader.read_flag()? {
            // 缩放矩阵字段长且罕见：这种流按不支持处理。
            return None;
        }
    }
    let log2_max_frame_num = reader.read_ue()? + 4;
    let poc_type = reader.read_ue()?;
    let log2_max_poc_lsb = if poc_type == 0 {
        reader.read_ue()? + 4
    } else {
        0
    };
    reader.read_ue()?; // max_num_ref_frames
    reader.read_flag()?; // gaps_in_frame_num_value_allowed_flag
    reader.read_ue()?; // pic_width_in_mbs_minus1
    reader.read_ue()?; // pic_height_in_map_units_minus1
    let frame_mbs_only = reader.read_flag()?;
    Some(SpsParams {
        log2_max_frame_num,
        poc_type,
        log2_max_poc_lsb,
        frame_mbs_only,
    })
}

/// PPS 里唯一的 POC 相关字段：`bottom_field_pic_order_in_frame_present_flag`。
fn pps_bottom_field_poc(nal: &[u8]) -> Option<bool> {
    let rbsp = remove_emulation(nal.get(1..)?);
    let mut reader = BitReader::new(&rbsp);
    reader.read_ue()?; // pic_parameter_set_id
    reader.read_ue()?; // seq_parameter_set_id
    reader.read_flag()?; // entropy_coding_mode_flag
    Some(reader.read_flag()?)
}

/// 取画面首个 VCL NAL 的 slice 头信息；解析不动时返回 `None`。
fn slice_poc(nal: &[u8], sps: &SpsParams, bottom_field_poc: bool) -> Option<PocInfo> {
    let is_idr = nal_type(nal) == 5;
    let rbsp = remove_emulation(nal.get(1..)?);
    let mut reader = BitReader::new(&rbsp);
    reader.read_ue()?; // first_mb_in_slice
    reader.read_ue()?; // slice_type
    reader.read_ue()?; // pic_parameter_set_id（单参数集流，不查表）
    reader.read_bits(sps.log2_max_frame_num)?; // frame_num
    let mut field_pic = false;
    if !sps.frame_mbs_only {
        field_pic = reader.read_flag()?;
        if field_pic {
            reader.read_flag()?; // bottom_field_flag
        }
    }
    if is_idr {
        reader.read_ue()?; // idr_pic_id
    }
    // 只有 POC type 0 从 slice 头取 lsb；其余 type 的推导不依赖 slice 头，
    // 这里统一按解码序近似（POC 全 0，排序退化为解码序）。
    let poc_lsb = if sps.poc_type == 0 {
        let lsb = reader.read_bits(sps.log2_max_poc_lsb)?;
        if bottom_field_poc && !field_pic {
            reader.read_se()?; // delta_pic_order_cnt_bottom
        }
        lsb
    } else {
        0
    };
    Some(PocInfo {
        poc_lsb,
        is_idr,
        nal_ref_idc: nal_ref_idc(nal),
    })
}

/// 一个画面的显示顺序坐标：`(GOP 序号, GOP 内 POC)`。
///
/// POC 每遇到 IDR 就从 0 重来，只能决定 **GOP 内**的相对顺序；GOP 之间按
/// 解码序（IDR 划分的闭合 GOP，显示顺序与 GOP 解码顺序一致）。
type PocPosition = (i64, i64);

/// 逐画面恢复显示顺序坐标（H.264 8.2.1.1，type 0 的换带推导）。
///
/// 解析不动的画面按解码序插队（GOP 内 POC + 1），不为一个坏 slice 放弃整条
/// 视频；POC type 1/2 的流退化为解码序（见 [`slice_poc`]）。
fn picture_pocs(frames: &[&[u8]], sps: &SpsParams, bottom_field_poc: bool) -> Vec<PocPosition> {
    let max_lsb = 1_i64 << sps.log2_max_poc_lsb.min(31);
    let mut prev_lsb = 0_u32;
    let mut prev_msb = 0_i64;
    let mut gop = -1_i64;
    let mut last_poc = -1_i64;
    let mut pocs = Vec::with_capacity(frames.len());
    for frame in frames {
        let vcl = split_sample_nals(frame)
            .unwrap_or_default()
            .into_iter()
            .find(|nal| matches!(nal_type(nal), 1..=5));
        let parsed = vcl.and_then(|nal| slice_poc(nal, sps, bottom_field_poc));
        let Some(info) = parsed else {
            last_poc += 1;
            pocs.push((gop.max(0), last_poc));
            continue;
        };
        if info.is_idr {
            // IDR 开新 GOP：POC 高位归零，GOP 计数 +1。
            gop += 1;
            prev_lsb = 0;
            prev_msb = 0;
        }
        // 带回绕的 POC 高位：与上一个**参考**画面的 lsb 差超过半个周期即换带。
        let lsb = info.poc_lsb as i64;
        let msb = if info.is_idr {
            0
        } else if lsb < prev_lsb as i64 && prev_lsb as i64 - lsb >= max_lsb / 2 {
            prev_msb + max_lsb
        } else if lsb > prev_lsb as i64 && lsb - prev_lsb as i64 > max_lsb / 2 {
            prev_msb - max_lsb
        } else {
            prev_msb
        };
        let poc = msb + lsb;
        if info.nal_ref_idc != 0 {
            prev_lsb = info.poc_lsb;
            prev_msb = msb;
        }
        last_poc = poc;
        pocs.push((gop.max(0), poc));
    }
    pocs
}

/// 显示顺序：按 `(GOP 序号, POC)` 升序（同值按解码序），返回每个画面的显示序号。
fn display_ranks(pocs: &[PocPosition]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..pocs.len()).collect();
    order.sort_by_key(|&index| (pocs[index], index));
    let mut ranks = vec![0_usize; pocs.len()];
    for (rank, &index) in order.iter().enumerate() {
        ranks[index] = rank;
    }
    ranks
}

/// 样本转「长度前缀 VCL NAL」：MP4 sample 只要 slice，参数集走 avcC。
fn sample_payload(frame: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for nal in split_sample_nals(frame)? {
        if matches!(nal_type(nal), 1..=5) {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// 去掉 RBSP 的防竞争字节（`00 00 03` 中的 `03`），供位解析使用。
fn remove_emulation(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0_usize;
    for &byte in data {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

/// 最小位读取器：只实现 H.264 头解析用到的定长位与 exp-Golomb。
struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn read_bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..count {
            let byte = *self.data.get(self.bit / 8)?;
            let bit = (byte >> (7 - self.bit % 8)) & 1;
            self.bit += 1;
            value = (value << 1) | u32::from(bit);
        }
        Some(value)
    }

    fn read_flag(&mut self) -> Option<bool> {
        Some(self.read_bits(1)? != 0)
    }

    /// 无符号 exp-Golomb：`前导零个数 + 1` 个位为一组。
    fn read_ue(&mut self) -> Option<u32> {
        let mut zeros = 0_u32;
        while self.read_bits(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some(((1_u32 << zeros) | self.read_bits(zeros)?) - 1)
    }

    fn read_se(&mut self) -> Option<i32> {
        let code = self.read_ue()?;
        // se 映射：0 → 0，偶数为负，奇数为正。
        Some(if code % 2 == 0 {
            -(code as i64 / 2) as i32
        } else {
            ((code + 1) / 2) as i32
        })
    }
}

/// 组装 faststart MP4：`ftyp + moov + mdat`。
fn build_mp4(track: &Track) -> Result<Vec<u8>, String> {
    if track.durations.len() != track.samples.len() {
        return Err("重封装轨道的时间表与样本数不一致".to_string());
    }
    let ftyp = box_of(
        b"ftyp",
        &[
            b"isom".as_slice(),
            &512_u32.to_be_bytes(),
            b"isomavc1mp41",
        ]
        .concat(),
    );
    // stco 依赖 mdat 起点，mdat 起点又依赖 moov 大小；moov 的长度与偏移**值**
    // 无关，先用占位 0 量出长度，再按真实起点生成一次。
    let moov_len = moov_box(track, 0)?.len();
    let mdat_start = ftyp.len() + moov_len + 8;
    let payload_len: usize = track.samples.iter().map(|sample| sample.len()).sum();
    if mdat_start + payload_len > u32::MAX as usize {
        return Err("重封装后的 MP4 超过 4 GiB".to_string());
    }
    let moov = moov_box(track, mdat_start as u32)?;

    let mut out = ftyp;
    out.extend_from_slice(&moov);
    out.extend_from_slice(&((payload_len + 8) as u32).to_be_bytes());
    out.extend_from_slice(b"mdat");
    for sample in &track.samples {
        out.extend_from_slice(sample);
    }
    Ok(out)
}

/// `moov` = `mvhd + trak`。
fn moov_box(track: &Track, chunk_base: u32) -> Result<Vec<u8>, String> {
    let duration: u64 = track.durations.iter().map(|&value| u64::from(value)).sum();
    let duration = u32::try_from(duration).map_err(|_| "轨道时长溢出".to_string())?;
    let mut payload = box_of(b"mvhd", &mvhd_payload(track.timescale, duration));
    payload.extend_from_slice(&trak_box(track, chunk_base, duration)?);
    Ok(box_of(b"moov", &payload))
}

/// `mvhd`：只带一条视频轨，rate/volume 用常规默认值。
fn mvhd_payload(timescale: u32, duration: u32) -> Vec<u8> {
    let mut payload = full_box_header(0, 0);
    payload.extend_from_slice(&0_u32.to_be_bytes()); // creation_time
    payload.extend_from_slice(&0_u32.to_be_bytes()); // modification_time
    payload.extend_from_slice(&timescale.to_be_bytes());
    payload.extend_from_slice(&duration.to_be_bytes());
    payload.extend_from_slice(&0x0001_0000_u32.to_be_bytes()); // rate 1.0
    payload.extend_from_slice(&0x0100_u16.to_be_bytes()); // volume 1.0
    payload.extend_from_slice(&0_u16.to_be_bytes());
    payload.extend_from_slice(&[0; 8]); // reserved
    payload.extend_from_slice(&identity_matrix());
    payload.extend_from_slice(&[0; 24]); // pre_defined
    payload.extend_from_slice(&2_u32.to_be_bytes()); // next_track_ID
    payload
}

/// `trak` = `tkhd + mdia`。
fn trak_box(track: &Track, chunk_base: u32, duration: u32) -> Result<Vec<u8>, String> {
    let mut payload = box_of(b"tkhd", &tkhd_payload(track, duration));
    payload.extend_from_slice(&mdia_box(track, chunk_base, duration)?);
    Ok(box_of(b"trak", &payload))
}

/// `tkhd`：flags = enabled | in_movie。
fn tkhd_payload(track: &Track, duration: u32) -> Vec<u8> {
    let mut payload = full_box_header(0, 0x0000_0003);
    payload.extend_from_slice(&0_u32.to_be_bytes()); // creation_time
    payload.extend_from_slice(&0_u32.to_be_bytes()); // modification_time
    payload.extend_from_slice(&1_u32.to_be_bytes()); // track_ID
    payload.extend_from_slice(&0_u32.to_be_bytes()); // reserved
    payload.extend_from_slice(&duration.to_be_bytes());
    payload.extend_from_slice(&[0; 8]); // reserved
    payload.extend_from_slice(&0_u16.to_be_bytes()); // layer
    payload.extend_from_slice(&0_u16.to_be_bytes()); // alternate_group
    payload.extend_from_slice(&0_u16.to_be_bytes()); // volume（非音频）
    payload.extend_from_slice(&0_u16.to_be_bytes());
    payload.extend_from_slice(&identity_matrix());
    payload.extend_from_slice(&(track.width << 16).to_be_bytes()); // 16.16 定点
    payload.extend_from_slice(&(track.height << 16).to_be_bytes());
    payload
}

/// `mdia` = `mdhd + hdlr + minf`。
fn mdia_box(track: &Track, chunk_base: u32, duration: u32) -> Result<Vec<u8>, String> {
    let mut mdhd = full_box_header(0, 0);
    mdhd.extend_from_slice(&0_u32.to_be_bytes()); // creation_time
    mdhd.extend_from_slice(&0_u32.to_be_bytes()); // modification_time
    mdhd.extend_from_slice(&track.timescale.to_be_bytes());
    mdhd.extend_from_slice(&duration.to_be_bytes());
    // language 'und'：五位一组打包（u=21, n=14, d=4）。
    mdhd.extend_from_slice(&((21_u16 << 10) | (14_u16 << 5) | 4_u16).to_be_bytes());
    mdhd.extend_from_slice(&0_u16.to_be_bytes()); // quality

    let mut hdlr = full_box_header(0, 0);
    hdlr.extend_from_slice(&0_u32.to_be_bytes()); // pre_defined
    hdlr.extend_from_slice(b"vide");
    hdlr.extend_from_slice(&[0; 12]); // reserved
    hdlr.extend_from_slice(b"VideoHandler\0");

    let mut minf_payload = box_of(b"vmhd", &{
        let mut vmhd = full_box_header(0, 1);
        vmhd.extend_from_slice(&0_u16.to_be_bytes()); // graphicsmode
        vmhd.extend_from_slice(&[0; 6]); // opcolor
        vmhd
    });
    let mut dref = full_box_header(0, 0);
    dref.extend_from_slice(&1_u32.to_be_bytes()); // entry_count
    dref.extend_from_slice(&full_box_of(b"url ", 0, 1, &[])); // self-contained
    minf_payload.extend_from_slice(&box_of(b"dinf", &box_of(b"dref", &dref)));
    minf_payload.extend_from_slice(&stbl_box(track, chunk_base)?);

    let mut payload = box_of(b"mdhd", &mdhd);
    payload.extend_from_slice(&box_of(b"hdlr", &hdlr));
    payload.extend_from_slice(&box_of(b"minf", &minf_payload));
    Ok(box_of(b"mdia", &payload))
}

/// `stbl` = `stsd + stts + ctts? + stss + stsc + stsz + stco`。
fn stbl_box(track: &Track, chunk_base: u32) -> Result<Vec<u8>, String> {
    let mut payload = box_of(b"stsd", &{
        let mut stsd = full_box_header(0, 0);
        stsd.extend_from_slice(&1_u32.to_be_bytes()); // entry_count
        stsd.extend_from_slice(&avc1_box(track)?);
        stsd
    });
    payload.extend_from_slice(&stts_box(&track.durations));
    if let Some(ctts) = ctts_box(&track.pts_offsets) {
        payload.extend_from_slice(&ctts);
    }
    if !track.sync.is_empty() {
        let mut stss = full_box_header(0, 0);
        stss.extend_from_slice(&(track.sync.len() as u32).to_be_bytes());
        for &number in &track.sync {
            stss.extend_from_slice(&number.to_be_bytes());
        }
        payload.extend_from_slice(&box_of(b"stss", &stss));
    }
    payload.extend_from_slice(&box_of(b"stsc", &{
        let mut stsc = full_box_header(0, 0);
        stsc.extend_from_slice(&1_u32.to_be_bytes()); // entry_count
        stsc.extend_from_slice(&1_u32.to_be_bytes()); // first_chunk
        stsc.extend_from_slice(&1_u32.to_be_bytes()); // samples_per_chunk
        stsc.extend_from_slice(&1_u32.to_be_bytes()); // sample_description_index
        stsc
    }));
    payload.extend_from_slice(&box_of(b"stsz", &{
        let mut stsz = full_box_header(0, 0);
        stsz.extend_from_slice(&0_u32.to_be_bytes()); // sample_size：0 = 逐样本
        stsz.extend_from_slice(&(track.samples.len() as u32).to_be_bytes());
        for sample in &track.samples {
            stsz.extend_from_slice(&(sample.len() as u32).to_be_bytes());
        }
        stsz
    }));
    // 一个样本一个 chunk：偏移 = mdat 起点 + 前面样本的累计长度。
    let mut stco = full_box_header(0, 0);
    stco.extend_from_slice(&(track.samples.len() as u32).to_be_bytes());
    let mut offset = u64::from(chunk_base);
    for sample in &track.samples {
        stco.extend_from_slice(&u32::try_from(offset).map_err(|_| "chunk 偏移溢出")?.to_be_bytes());
        offset += sample.len() as u64;
    }
    payload.extend_from_slice(&box_of(b"stco", &stco));
    Ok(box_of(b"stbl", &payload))
}

/// `stts`：连续相同时长合并成条目（恒定帧率时只剩一两条）。
fn stts_box(durations: &[u32]) -> Vec<u8> {
    let mut entries: Vec<(u32, u32)> = Vec::new();
    for &duration in durations {
        match entries.last_mut() {
            Some((count, delta)) if *delta == duration => *count += 1,
            _ => entries.push((1, duration)),
        }
    }
    let mut stts = full_box_header(0, 0);
    stts.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (count, delta) in entries {
        stts.extend_from_slice(&count.to_be_bytes());
        stts.extend_from_slice(&delta.to_be_bytes());
    }
    box_of(b"stts", &stts)
}

/// `ctts` version 1（有符号偏移，B 帧的 PTS 可能早于 DTS）；全 0 时省略该 box。
fn ctts_box(offsets: &[i32]) -> Option<Vec<u8>> {
    if offsets.iter().all(|&offset| offset == 0) {
        return None;
    }
    let mut entries: Vec<(u32, i32)> = Vec::new();
    for &offset in offsets {
        match entries.last_mut() {
            Some((count, value)) if *value == offset => *count += 1,
            _ => entries.push((1, offset)),
        }
    }
    let mut ctts = full_box_header(1, 0);
    ctts.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (count, offset) in entries {
        ctts.extend_from_slice(&count.to_be_bytes());
        ctts.extend_from_slice(&offset.to_be_bytes());
    }
    Some(box_of(b"ctts", &ctts))
}

/// `avc1` 视频样本条目 + `avcC`（参数集）。
fn avc1_box(track: &Track) -> Result<Vec<u8>, String> {
    let mut payload = vec![0_u8; 6]; // reserved
    payload.extend_from_slice(&1_u16.to_be_bytes()); // data_reference_index
    payload.extend_from_slice(&[0; 16]); // pre_defined/reserved
    payload.extend_from_slice(&(track.width as u16).to_be_bytes());
    payload.extend_from_slice(&(track.height as u16).to_be_bytes());
    payload.extend_from_slice(&0x0048_0000_u32.to_be_bytes()); // 72 dpi
    payload.extend_from_slice(&0x0048_0000_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes()); // reserved
    payload.extend_from_slice(&1_u16.to_be_bytes()); // frame_count
    payload.extend_from_slice(&[0; 32]); // compressorname（空）
    payload.extend_from_slice(&0x0018_u16.to_be_bytes()); // depth
    payload.extend_from_slice(&(-1_i16).to_be_bytes()); // pre_defined
    payload.extend_from_slice(&avc_c_box(track)?);
    Ok(box_of(b"avc1", &payload))
}

/// `avcC`（AVCDecoderConfigurationRecord）：参数集 + 4 字节 NAL 长度。
fn avc_c_box(track: &Track) -> Result<Vec<u8>, String> {
    if track.sps.len() < 4 || track.pps.is_empty() {
        return Err("缺少 SPS/PPS，无法写 avcC".to_string());
    }
    let mut payload = vec![1_u8]; // configurationVersion
    payload.extend_from_slice(&track.sps[1..4]); // profile/compat/level
    payload.push(0xFF); // reserved + lengthSizeMinusOne = 3
    payload.push(0xE1); // reserved + numOfSPS = 1
    payload.extend_from_slice(
        &u16::try_from(track.sps.len())
            .map_err(|_| "SPS 过长".to_string())?
            .to_be_bytes(),
    );
    payload.extend_from_slice(&track.sps);
    payload.push(1); // numOfPPS
    payload.extend_from_slice(
        &u16::try_from(track.pps.len())
            .map_err(|_| "PPS 过长".to_string())?
            .to_be_bytes(),
    );
    payload.extend_from_slice(&track.pps);
    Ok(box_of(b"avcC", &payload))
}

/// 标准 3×3 单位矩阵（定点 16.16 / 2.30）。
fn identity_matrix() -> [u8; 36] {
    let mut matrix = [0_u8; 36];
    matrix[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    matrix[16..20].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    matrix[32..36].copy_from_slice(&0x4000_0000_u32.to_be_bytes());
    matrix
}

/// 普通 box：`size + type + payload`。
fn box_of(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(typ);
    out.extend_from_slice(payload);
    out
}

/// FullBox 的 `version + flags` 头。
fn full_box_header(version: u8, flags: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(4);
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out
}

/// FullBox：普通 box 头 + `version + flags` + payload。
fn full_box_of(typ: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = full_box_header(version, flags);
    body.extend_from_slice(payload);
    box_of(typ, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试位写入器：按 H.264 语法造 SPS/PPS/slice 头。
    struct BitWriter {
        bytes: Vec<u8>,
        current: u8,
        used: u32,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                current: 0,
                used: 0,
            }
        }

        fn write_bits(&mut self, value: u32, count: u32) {
            for shift in (0..count).rev() {
                self.current = (self.current << 1) | ((value >> shift) & 1) as u8;
                self.used += 1;
                if self.used == 8 {
                    self.bytes.push(self.current);
                    self.current = 0;
                    self.used = 0;
                }
            }
        }

        fn write_ue(&mut self, value: u32) {
            let code = value + 1;
            let bits = u32::BITS - code.leading_zeros();
            self.write_bits(0, bits - 1);
            self.write_bits(code, bits);
        }

        fn finish(mut self) -> Vec<u8> {
            if self.used > 0 {
                self.bytes.push(self.current << (8 - self.used));
            }
            self.bytes
        }
    }

    /// Baseline SPS：POC type 0，`frame_num` 4 位、`poc_lsb` 4 位、单 slice 场。
    fn test_sps() -> Vec<u8> {
        let mut writer = BitWriter::new();
        writer.write_bits(66, 8); // profile_idc：Baseline，无 High 扩展字段
        writer.write_bits(0, 8); // constraint flags
        writer.write_bits(30, 8); // level_idc
        writer.write_ue(0); // sps_id
        writer.write_ue(0); // log2_max_frame_num_minus4
        writer.write_ue(0); // pic_order_cnt_type
        writer.write_ue(0); // log2_max_pic_order_cnt_lsb_minus4
        writer.write_ue(1); // max_num_ref_frames
        writer.write_bits(0, 1); // gaps_in_frame_num
        writer.write_ue(0); // pic_width_in_mbs_minus1
        writer.write_ue(0); // pic_height_in_map_units_minus1
        writer.write_bits(1, 1); // frame_mbs_only_flag
        let mut nal = vec![0x67];
        nal.extend_from_slice(&writer.finish());
        nal
    }

    fn test_pps() -> Vec<u8> {
        let mut writer = BitWriter::new();
        writer.write_ue(0); // pps_id
        writer.write_ue(0); // sps_id
        writer.write_bits(0, 1); // entropy_coding_mode_flag
        writer.write_bits(0, 1); // bottom_field_pic_order_in_frame_present_flag
        let mut nal = vec![0x68];
        nal.extend_from_slice(&writer.finish());
        nal
    }

    /// 造一个画面的 slice 头：`slice_type` 0=P / 1=B / 2=I。
    fn test_slice(slice_type: u32, frame_num: u32, poc_lsb: u32, idr: bool) -> Vec<u8> {
        let mut writer = BitWriter::new();
        writer.write_ue(0); // first_mb_in_slice
        writer.write_ue(slice_type);
        writer.write_ue(0); // pps_id
        writer.write_bits(frame_num, 4); // frame_num
        if idr {
            writer.write_ue(0); // idr_pic_id
        }
        writer.write_bits(poc_lsb, 4); // pic_order_cnt_lsb
        let header = if idr { 0x65 } else { 0x41 };
        let mut nal = vec![header];
        nal.extend_from_slice(&writer.finish());
        nal
    }

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }

    /// POC 推导：解码序 I0 P3 B1 B2 的显示序是 I0 B1 B2 P3，POC 换带也能还原。
    #[test]
    fn picture_pocs_follow_the_spec_reordering() {
        let sps = parse_sps(&test_sps()).expect("SPS 必须可解析");
        assert_eq!(sps.poc_type, 0);
        assert_eq!(sps.log2_max_poc_lsb, 4);
        // 解码序：I0(poc 0) P3(poc 6) B1(poc 2) B2(poc 4)。
        let frames = [
            annexb(&[&test_slice(2, 0, 0, true)]),
            annexb(&[&test_slice(0, 1, 6, false)]),
            annexb(&[&test_slice(1, 1, 2, false)]),
            annexb(&[&test_slice(1, 1, 4, false)]),
        ];
        let refs: Vec<&[u8]> = frames.iter().map(<Vec<u8>>::as_slice).collect();
        let pocs = picture_pocs(&refs, &sps, false);
        assert_eq!(pocs, vec![(0, 0), (0, 6), (0, 2), (0, 4)]);
        assert_eq!(display_ranks(&pocs), vec![0, 3, 1, 2]);
    }

    /// POC 每遇 IDR 从 0 重来：GOP 之间按解码序，不能把两个 GOP 的 POC 混排。
    #[test]
    fn gop_restart_keeps_display_order_across_idrs() {
        let sps = parse_sps(&test_sps()).expect("SPS 必须可解析");
        // 两个 GOP：I0 P2 | I0' P2'，POC 都是 0/2，显示序仍是解码序。
        let frames = [
            annexb(&[&test_slice(2, 0, 0, true)]),
            annexb(&[&test_slice(0, 1, 2, false)]),
            annexb(&[&test_slice(2, 0, 0, true)]),
            annexb(&[&test_slice(0, 1, 2, false)]),
        ];
        let refs: Vec<&[u8]> = frames.iter().map(<Vec<u8>>::as_slice).collect();
        let pocs = picture_pocs(&refs, &sps, false);
        assert_eq!(pocs, vec![(0, 0), (0, 2), (1, 0), (1, 2)]);
        assert_eq!(display_ranks(&pocs), vec![0, 1, 2, 3]);
    }

    /// POC type 0 的回绕：lsb 从 14 跳到 0 是 +2（换带），不是 −14。
    #[test]
    fn picture_pocs_handle_lsb_wraparound() {
        let sps = parse_sps(&test_sps()).expect("SPS 必须可解析");
        let frames = [
            annexb(&[&test_slice(2, 0, 14, true)]),
            annexb(&[&test_slice(0, 1, 0, false)]),
            annexb(&[&test_slice(0, 2, 2, false)]),
        ];
        let refs: Vec<&[u8]> = frames.iter().map(<Vec<u8>>::as_slice).collect();
        assert_eq!(
            picture_pocs(&refs, &sps, false),
            vec![(0, 14), (0, 16), (0, 18)]
        );
    }

    /// 在盒子里找子 box（递归进容器 box），返回其内容（不含箱头）。
    fn find_box<'a>(data: &'a [u8], typ: &[u8; 4]) -> Option<&'a [u8]> {
        let mut pos = 0;
        while pos + 8 <= data.len() {
            let size = u32::from_be_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
            let name: &[u8; 4] = data[pos + 4..pos + 8].try_into().ok()?;
            let end = (pos + size).min(data.len());
            if size < 8 {
                return None;
            }
            let payload = &data[pos + 8..end];
            if name == typ {
                return Some(payload);
            }
            if matches!(
                name,
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"dinf" | b"stsd" | b"avc1"
            ) {
                // 容器 box 的子 box 前有固定头：stsd 有 8 字节，avc1 样本条目 78 字节。
                let skip = match name {
                    b"stsd" => 8,
                    b"avc1" => 78,
                    _ => 0,
                };
                if let Some(found) = payload.get(skip..).and_then(|rest| find_box(rest, typ)) {
                    return Some(found);
                }
            }
            pos = end;
        }
        None
    }

    /// MP4 骨架：顶层 box 顺序 faststart，stts/ctts/stss 与轨道数据一致。
    #[test]
    fn build_mp4_layout_is_faststart_and_consistent() {
        let track = Track {
            width: 640,
            height: 360,
            timescale: 48_000,
            durations: vec![1000, 1000, 3000],
            pts_offsets: vec![0, -1000, 2000],
            sync: vec![1, 3],
            samples: vec![vec![0, 0, 0, 1, 0x65], vec![0, 0, 0, 1, 0x41], vec![0, 0, 0, 2, 0x41, 1]],
            sps: test_sps(),
            pps: test_pps(),
        };
        let mp4 = build_mp4(&track).expect("MP4 必须可组装");

        // 顶层顺序：ftyp → moov → mdat（faststart）。
        assert_eq!(&mp4[4..8], b"ftyp");
        let ftyp_size = u32::from_be_bytes(mp4[0..4].try_into().unwrap()) as usize;
        assert_eq!(&mp4[ftyp_size + 4..ftyp_size + 8], b"moov");
        let moov_size = u32::from_be_bytes(mp4[ftyp_size..ftyp_size + 4].try_into().unwrap()) as usize;
        let mdat_start = ftyp_size + moov_size;
        assert_eq!(&mp4[mdat_start + 4..mdat_start + 8], b"mdat");

        // mdat 载荷 = 样本原样拼接。
        let mdat_payload = &mp4[mdat_start + 8..];
        let joined = track.samples.concat();
        assert_eq!(mdat_payload, &joined);

        // stco：每样本一个 chunk，指向 mdat 内对应样本。
        let stco = find_box(&mp4, b"stco").expect("stco 必须存在");
        let count = u32::from_be_bytes(stco[4..8].try_into().unwrap());
        assert_eq!(count, 3);
        let mut expected = (mdat_start + 8) as u32;
        for index in 0..3 {
            let offset =
                u32::from_be_bytes(stco[8 + index * 4..12 + index * 4].try_into().unwrap());
            assert_eq!(offset, expected);
            expected += track.samples[index].len() as u32;
        }

        // stts/ctts/stss 的条目数与数据一致（ctts 是 version 1）。
        let stts = find_box(&mp4, b"stts").expect("stts 必须存在");
        // 时长 [1000, 1000, 3000] 游程压缩成 (2×1000, 1×3000) 两条。
        assert_eq!(u32::from_be_bytes(stts[4..8].try_into().unwrap()), 2);
        let ctts = find_box(&mp4, b"ctts").expect("ctts 必须存在");
        assert_eq!(ctts[0], 1, "B 帧偏移为负，ctts 必须是 version 1");
        assert_eq!(u32::from_be_bytes(ctts[4..8].try_into().unwrap()), 3);
        let stss = find_box(&mp4, b"stss").expect("stss 必须存在");
        assert_eq!(u32::from_be_bytes(stss[4..8].try_into().unwrap()), 2);

        // avcC 带 SPS/PPS，长度大小 4 字节。
        let avcc = find_box(&mp4, b"avcC").expect("avcC 必须存在");
        assert_eq!(avcc[0], 1);
        assert_eq!(avcc[4], 0xFF);
    }

    /// ctts 全 0（无 B 帧重排）时省略该 box。
    #[test]
    fn build_mp4_omits_ctts_without_reordering() {
        let track = Track {
            width: 16,
            height: 16,
            timescale: 24,
            durations: vec![1, 1],
            pts_offsets: vec![0, 0],
            sync: vec![1],
            samples: vec![vec![0, 0, 0, 1, 0x65], vec![0, 0, 0, 1, 0x41]],
            sps: test_sps(),
            pps: test_pps(),
        };
        let mp4 = build_mp4(&track).expect("MP4 必须可组装");
        assert!(find_box(&mp4, b"ctts").is_none());
    }

    /// 端到端：合成 AVI（重复帧占位 + B 帧）→ MP4，PTS 按显示序落回槽位。
    #[test]
    fn remuxes_synthetic_avi_with_b_frames() {
        // 解码序 I0 P3 B1 B2；槽位 0/2/4/6 是画面、1/3/5/7 是重复帧占位。
        let idr = annexb(&[&test_sps(), &test_pps(), &test_slice(2, 0, 0, true)]);
        let p = annexb(&[&test_slice(0, 1, 6, false)]);
        let b1 = annexb(&[&test_slice(1, 1, 2, false)]);
        let b2 = annexb(&[&test_slice(1, 1, 4, false)]);
        let samples = vec![
            Some(idr),
            None,
            Some(p),
            None,
            Some(b1),
            None,
            Some(b2),
            None,
        ];
        let avi = build_avi(&samples, 1001, 48_000);
        let mp4 = remux_avi_to_mp4(&avi).expect("重封装必须成功");

        // 4 个画面：显示序 I0 B1 B2 P3 → 槽位 0/2/4/6 → PTS = 槽位 × 1001。
        // DTS = 解码序 × 1001，ctts = PTS − DTS：P 虽先解，显示排最后。
        let ctts = find_box(&mp4, b"ctts").expect("B 帧必须带 ctts");
        let offsets: Vec<i32> = (0..4)
            .map(|index| {
                // ctts 条目是 (count u32, offset i32) 游程对，offset 在 +4。
                i32::from_be_bytes(ctts[8 + index * 8 + 4..8 + index * 8 + 8].try_into().unwrap())
            })
            .collect();
        assert_eq!(offsets, vec![0, 5005, 0, 1001]);

        // 轨道总时长盖住 8 个槽位（含重复帧占位）。
        let mdhd = find_box(&mp4, b"mdhd").expect("mdhd 必须存在");
        let duration = u32::from_be_bytes(mdhd[16..20].try_into().unwrap());
        assert_eq!(duration, 8 * 1001);

        // mdat 里的样本只保留 VCL（长度前缀），参数集不进样本。
        let mdat_start = {
            let ftyp_size =
                u32::from_be_bytes(mp4[0..4].try_into().unwrap()) as usize;
            let moov_size =
                u32::from_be_bytes(mp4[ftyp_size..ftyp_size + 4].try_into().unwrap()) as usize;
            ftyp_size + moov_size + 8
        };
        let mdat = &mp4[mdat_start..];
        let first_len = u32::from_be_bytes(mdat[0..4].try_into().unwrap()) as usize;
        assert_eq!(mdat[4] & 0x1f, 5, "样本只剩 slice NAL，SPS/PPS 已进 avcC");
        assert!(first_len > 0 && 4 + first_len <= mdat.len());
    }

    /// 最小 AVI 打包（hdrl + movi）：空样本即零长度视频 chunk。
    fn build_avi(frames: &[Option<Vec<u8>>], scale: u32, rate: u32) -> Vec<u8> {
        fn chunk(id: &[u8; 4], content: &[u8]) -> Vec<u8> {
            let mut out = Vec::from(*id);
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

        let mut avih = vec![0_u8; 56];
        avih[0..4].copy_from_slice(&20_833_u32.to_le_bytes());
        avih[24..28].copy_from_slice(&1_u32.to_le_bytes());
        avih[32..36].copy_from_slice(&16_u32.to_le_bytes());
        avih[36..40].copy_from_slice(&16_u32.to_le_bytes());
        let mut strh = vec![0_u8; 56];
        strh[0..4].copy_from_slice(b"vids");
        strh[4..8].copy_from_slice(b"H264");
        strh[20..24].copy_from_slice(&scale.to_le_bytes());
        strh[24..28].copy_from_slice(&rate.to_le_bytes());
        strh[32..36].copy_from_slice(&(frames.len() as u32).to_le_bytes());
        let mut strf = vec![0_u8; 40];
        strf[0..4].copy_from_slice(&40_u32.to_le_bytes());
        strf[4..8].copy_from_slice(&16_i32.to_le_bytes());
        strf[8..12].copy_from_slice(&16_i32.to_le_bytes());
        strf[16..20].copy_from_slice(b"H264");
        let strl = list(
            b"strl",
            &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat(),
        );
        let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), strl].concat());
        let mut movi_content = Vec::new();
        for frame in frames {
            movi_content.extend_from_slice(&chunk(b"00dc", frame.as_deref().unwrap_or_default()));
        }
        let movi = list(b"movi", &movi_content);
        let mut body = Vec::from(*b"AVI ");
        body.extend_from_slice(&hdrl);
        body.extend_from_slice(&movi);
        let mut out = Vec::from(*b"RIFF");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// 真实 AVI 的端到端验证（可选）：设置环境变量
    /// `OSU_BMP_AVI_FIXTURE=<路径>` 时跑重封装并把结果写到
    /// `OSU_BMP_AVI_OUT`（缺省为输入旁的 `.remux.mp4`），供 ffprobe 复核；
    /// 没有环境变量时直接通过。
    #[test]
    fn remux_real_fixture_when_provided() {
        let Ok(path) = std::env::var("OSU_BMP_AVI_FIXTURE") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("fixture 必须可读");
        assert!(is_avi(&bytes));
        let mp4 = remux_avi_to_mp4(&bytes).expect("真实 AVI 必须可重封装");
        let out = std::env::var("OSU_BMP_AVI_OUT")
            .unwrap_or_else(|_| format!("{path}.remux.mp4"));
        std::fs::write(&out, &mp4).expect("重封装结果必须可写");
    }
}

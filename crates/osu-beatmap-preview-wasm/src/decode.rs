//! 音频与图片解码：`.osz` 里取出的字节 → core 直接可用的 PCM / RGBA。
//!
//! 与 CLI 的 `media/hitsound.rs`、`media/audio.rs` 是同一套 symphonia / image 解码逻辑
//! 的字节版（CLI 从文件解，这里从内存解），错误语义一致并按 Web 预览的容错要求放宽：
//!
//! - **音乐**解码失败是致命错误（没有声音的整包预览没有意义）；
//! - **单个音效**失败按静音处理（旧 Web 端 `decodeSample` 同样如此），不影响其他样本；
//! - **背景**失败退化成纯色背景。
//!
//! 这个模块不依赖 wasm 运行时，可以直接 `cargo test`。

use std::io::Cursor;

use osu_beatmap_preview_core::hitsound::{sample_loop_len, SampleData, SAMPLE_RATE};
use osu_beatmap_preview_core::ImageData;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// 解码音乐为交错立体声 f32（`SampleData`，`loop_len = 0`：整段只播一次）。
///
/// `extension` 只作为 symphonia 探测格式的提示（内容仍是最终依据）。
pub fn decode_music(bytes: &[u8], extension: Option<&str>) -> Result<SampleData, String> {
    let (stereo, sample_rate) = decode_stereo_f32(bytes, extension)?;
    if stereo.is_empty() {
        return Err("音乐解码结果为空".to_string());
    }
    Ok(SampleData::stereo(stereo, sample_rate))
}

/// 解码打击音样本；任何失败都按静音处理（返回空样本），不影响其他音效。
///
/// `name` 决定是否需要整段循环（滑条滑行音、转盘旋转音），判定由 core 统一提供，
/// 宿主不再各自猜名字。
pub fn decode_sample(name: &str, bytes: &[u8], extension: Option<&str>) -> SampleData {
    match decode_stereo_f32(bytes, extension) {
        Ok((stereo, sample_rate)) => {
            let loop_len = sample_loop_len(name, stereo.len() / 2);
            SampleData::stereo(stereo, sample_rate).with_loop(loop_len)
        }
        Err(_) => SampleData::stereo(Vec::new(), SAMPLE_RATE),
    }
}

/// 背景暗化系数：沿用 osu! 视频预览的默认值——暗化 70%，保留 30% 亮度。
///
/// 原来由宿主在交给 WASM 前逐像素调暗（Web 端 `BACKGROUND_DIM`），背景解码移进
/// WASM 后在这里做，与 CLI 导出的 `BACKGROUND_DIM`（`shared_config.yml`）同一语义。
/// 背景视频（`video.rs`）也用同一个系数。
pub(crate) const BACKGROUND_DIM: f64 = 0.7;

/// 解码背景图（png / jpeg 等 image crate 支持的格式）为**已暗化**的 RGBA；
/// 解码失败或尺寸非法返回 `None`，宿主退化成纯色背景。
pub fn decode_background(bytes: &[u8]) -> Option<ImageData> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgba = image.into_raw();
    dim_rgba(&mut rgba);
    Some(ImageData {
        width,
        height,
        rgba,
    })
}

/// 按 [`BACKGROUND_DIM`] 压暗 RGBA 像素（alpha 保持不变）。
fn dim_rgba(rgba: &mut [u8]) {
    let brightness = (1.0 - BACKGROUND_DIM) * 255.0;
    for pixel in rgba.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            // 每个通道独立按比例压暗，alpha 保持不变（透明背景仍要参与合成）。
            *channel = (*channel as f64 * brightness / 255.0).round() as u8;
        }
    }
}

/// 故事板贴图的解码像素上限（4096×4096）：防止病态大图撑爆 WASM 线性内存。
const MAX_STORYBOARD_PIXELS: u64 = 4096 * 4096;

/// 解码故事板贴图（png / jpeg）为**原始** RGBA（不做暗化：故事板不在暗化层内）。
///
/// 解码失败、空图或超像素上限返回 `None`，对应精灵按缺图静默跳过（与 osu! 一致）。
pub fn decode_image(bytes: &[u8]) -> Option<ImageData> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_STORYBOARD_PIXELS {
        return None;
    }
    Some(ImageData {
        width,
        height,
        rgba: image.into_raw(),
    })
}

/// 把一段音频字节解码成交错立体声 f32。
///
/// 声道数为 1 时复制成双声道；多声道取前两个声道（与 CLI 一致）。中途的坏包跳过，
/// 读到流尾正常结束；完全解不出音频才报错。
fn decode_stereo_f32(bytes: &[u8], extension: Option<&str>) -> Result<(Vec<f32>, u32), String> {
    if bytes.is_empty() {
        return Err("音频字节为空".to_string());
    }
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("音频格式无法识别：{error}"))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "音频没有可解码的轨道".to_string())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| format!("音频编码不受支持：{error}"))?;

    let mut sample_rate = 0_u32;
    let mut stereo = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            // 静音样本（argon pro 的 `*-sliderslide` 等）在部分文件里就是「解不出任何
            // 音频页」；这类读到流尾的错误按已解出的内容收尾，不当成失败。
            Err(SymphoniaError::IoError(_)) | Err(SymphoniaError::DecodeError(_)) => break,
            Err(error) => return Err(format!("音频读取失败：{error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(format!("音频解码失败：{error}")),
        };
        let spec = *decoded.spec();
        if spec.rate == 0 || spec.channels.count() == 0 {
            return Err("音频采样格式无效".to_string());
        }
        if sample_rate != 0 && sample_rate != spec.rate {
            return Err("音频中途改变采样率".to_string());
        }
        sample_rate = spec.rate;
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        append_stereo(buffer.samples(), spec.channels.count(), &mut stereo);
    }
    if sample_rate == 0 {
        return Err("音频解码结果为空".to_string());
    }
    Ok((stereo, sample_rate))
}

/// 把任意声道数的交错采样整理成双声道交错。
fn append_stereo(input: &[f32], channels: usize, output: &mut Vec<f32>) {
    if channels == 1 {
        output.reserve(input.len() * 2);
        for &sample in input {
            output.extend_from_slice(&[sample, sample]);
        }
    } else {
        output.reserve(input.len() / channels * 2);
        for frame in input.chunks_exact(channels) {
            output.extend_from_slice(&frame[..2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成一段正弦 WAV（PCM 16-bit 立体声），返回可解码的文件字节。
    fn wav_bytes(sample_rate: u32, frames: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        let data_len = (frames * 2 * 2) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&2_u16.to_le_bytes()); // 立体声
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for index in 0..frames {
            let value = if index % 2 == 0 {
                8_000_i16
            } else {
                -8_000_i16
            };
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// 音乐解码：立体声、帧数与采样率都正确。
    #[test]
    fn music_decodes_to_stereo_frames() {
        let sample = decode_music(&wav_bytes(8_000, 10), Some("wav")).expect("WAV 必须可解码");
        assert_eq!(sample.sample_rate, 8_000);
        assert_eq!(sample.frames(), 10);
        assert_eq!(sample.loop_len, 0);
        // 双声道交错：偶数位是左声道。
        assert!(matches!(
            sample.channels,
            osu_beatmap_preview_core::hitsound::Channels::Stereo(_)
        ));
    }

    /// 音乐解码失败必须报错，不能静悄悄没有声音。
    #[test]
    fn broken_music_reports_an_error() {
        assert!(decode_music(b"not audio at all", Some("mp3")).is_err());
        assert!(decode_music(&[], None).is_err());
    }

    /// 内嵌 ogg 皮肤必须可解码（真实资源回归）。
    #[test]
    fn embedded_ogg_skin_decodes() {
        let bytes = include_bytes!("../../../assets/hitsound/normal-hitnormal.ogg");
        let sample = decode_music(bytes, Some("ogg")).expect("内嵌 ogg 必须可解码");
        assert!(sample.frames() > 0);
    }

    /// 音效失败按静音处理；循环语义由 core 判定（含 bank 前缀与索引后缀）。
    #[test]
    fn sample_failures_are_silent_and_loop_semantics_come_from_core() {
        let silent = decode_sample("normal-hitnormal", b"garbage", Some("ogg"));
        assert_eq!(silent.frames(), 0);

        let slide = decode_sample("normal-sliderslide", &wav_bytes(8_000, 10), Some("wav"));
        assert_eq!(slide.frames(), 10);
        assert_eq!(slide.loop_len, slide.frames());

        let spinner = decode_sample("taiko-spinnerspin2", &wav_bytes(8_000, 10), Some("wav"));
        assert_eq!(spinner.loop_len, spinner.frames());

        let clap = decode_sample("normal-hitclap", &wav_bytes(8_000, 10), Some("wav"));
        assert_eq!(clap.loop_len, 0);
    }

    /// 背景解码为**暗化后的** RGBA；坏图返回 None。
    #[test]
    fn background_decodes_to_dimmed_rgba_or_none() {
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([100, 200, 50, 255]));
        let mut cursor = Cursor::new(Vec::new());
        image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .expect("PNG 编码必须成功");
        let decoded = decode_background(&cursor.into_inner()).expect("PNG 必须可解码");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.rgba.len(), 2 * 1 * 4);
        // 暗化 70%（保留 30% 亮度），alpha 不动。
        assert_eq!(&decoded.rgba[..4], &[30, 60, 15, 255]);

        assert!(decode_background(b"not an image").is_none());
        assert!(decode_background(&[]).is_none());
    }
}

//! 内嵌打击音资源的解码与样本库构建。
//!
//! CLI 的 MP4 音频是一次性离线混音，所以这里只在需要时解码「时间轴实际引用到」
//! 的样本，并且任何单个样本解码失败都按静音处理——资源损坏不应该让整次导出失败。

use osu_beatmap_preview_core::hitsound::{referenced_names, SampleData, SampleLibrary};
use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use std::io::Cursor;

/// build.rs 生成的内嵌样本表。
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/hitsound_assets.rs"));
}

/// 判断某个样本名是否随二进制一起分发。
#[cfg(test)]
pub(crate) fn has_embedded(name: &str) -> bool {
    embedded::HITSOUND_ASSETS
        .iter()
        .any(|(candidate, _)| *candidate == name)
}

fn embedded_bytes(name: &str) -> Option<&'static [u8]> {
    embedded::HITSOUND_ASSETS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, bytes)| *bytes)
}

/// 配置中的输出采样率；空样本也需要一个合法采样率。
fn default_sample_rate() -> u32 {
    crate::config::current()
        .advance
        .video_audio
        .AUDIO_SAMPLE_RATE
}

/// 判断解码错误是否属于「这个样本没声音」这一类可恢复情况。
///
/// symphonia 对空/损坏的 OGG 会给出 IO、解码或「不支持」错误（例如静音样本的页在
/// 部分机器上无法读出），这些都不应该中断整次导出。
fn is_recoverable(error: &SymphoniaError) -> bool {
    matches!(
        error,
        SymphoniaError::IoError(_) | SymphoniaError::DecodeError(_) | SymphoniaError::Unsupported(_)
    )
}

/// 为给定谱面构建打击音样本库。
///
/// 只解码时间轴引用到的名字；缺失或解码失败的样本会被跳过，混音阶段按静音处理。
pub(crate) fn build_library(beatmap: &Beatmap) -> SampleLibrary {
    let mut library = SampleLibrary::new();
    for name in referenced_names(beatmap) {
        let Some(bytes) = embedded_bytes(&name) else {
            continue;
        };
        match decode_sample(&name, bytes) {
            // 解码失败按静音处理：单个坏文件不应该让整次导出失败。
            // 这里刻意不写日志——日志是全局单例，媒体层的坏样本属于预期情况，
            // 交给调用方按需统计即可。
            Ok(sample) => library.insert(name, sample),
            Err(_) => continue,
        }
    }
    library
}

/// 解码一段 OGG 打击音为 f32 立体声。
///
/// `name` 用于判定是否需要整段循环：滑条滑行音与转盘旋转音在游戏里都是持续循环播放的。
fn decode_sample(name: &str, bytes: &[u8]) -> Result<SampleData> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let probed = match symphonia::default::get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        Ok(probed) => probed,
        // 无法探测/解码的样本（静音样本在部分机器上就是这种状态）按「静音」而不是
        // 「失败」处理：单个坏文件不应该让整次导出没有声音或直接报错。
        Err(error) if is_recoverable(&error) => {
            return Ok(SampleData::stereo(Vec::new(), default_sample_rate()));
        }
        Err(error) => {
            return Err(PreviewError::render(format!(
                "unsupported hitsound format: {error}"
            )))
        }
    };
    let mut format = probed.format;
    let track = match format.default_track() {
        Some(track) => track,
        None => return Ok(SampleData::stereo(Vec::new(), default_sample_rate())),
    };
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| PreviewError::render(format!("unsupported hitsound codec: {e}")))?;

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
            // 静音样本（argon pro 的 `*-sliderslide` / `*-sliderwhistle`）在部分机器上
            // 解不出任何音频页；这类「读到流尾」的错误按静音处理，不影响其它样本。
            Err(SymphoniaError::IoError(_)) | Err(SymphoniaError::DecodeError(_)) => break,
            Err(error) => {
                return Err(PreviewError::render(format!(
                    "failed to read hitsound: {error}"
                )))
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => {
                return Err(PreviewError::render(format!(
                    "failed to decode hitsound: {error}"
                )))
            }
        };
        let spec = *decoded.spec();
        if spec.rate == 0 || spec.channels.count() == 0 {
            return Err(PreviewError::render("hitsound has an invalid sample format"));
        }
        sample_rate = spec.rate;
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        append_f32_stereo(buffer.samples(), spec.channels.count(), &mut stereo);
    }
    // 显式静音的样本解出 0 帧是正常结果（游戏里就是静音），注册成空样本即可。
    if sample_rate == 0 {
        sample_rate = default_sample_rate();
    }
    let frames = stereo.len() / 2;
    let loop_length = if name.contains("sliderslide") || name == "spinnerspin" {
        frames
    } else {
        0
    };
    Ok(SampleData::stereo(stereo, sample_rate).with_loop(loop_length))
}

fn append_f32_stereo(input: &[f32], channels: usize, output: &mut Vec<f32>) {
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

    #[test]
    fn 内嵌样本表覆盖四模式的全部打击音() {
        // 资源目录下的 ogg 全部内嵌：缺一个都会让某个模式的某个音效静音。
        assert_eq!(embedded::HITSOUND_ASSETS.len(), 36);
        for name in [
            "normal-hitnormal",
            "normal-sliderslide",
            "soft-hitfinish",
            "drum-hitclap",
            "spinnerspin",
            "spinnerbonus",
            "taiko-normal-hitnormal",
            "spinnerbonus-max",
        ] {
            assert!(has_embedded(name), "缺少内嵌样本 {name}");
        }
    }

    #[test]
    fn 静音滑行音解码为零帧且不报错() {
        // argon pro 用静音样本关闭滑行音；这种文件可能解不出任何音频页，
        // 但必须按「静音」而不是「失败」处理，否则整条混音会被跳过。
        for name in ["normal-sliderslide", "soft-sliderwhistle"] {
            let bytes = embedded_bytes(name).expect("必须内嵌静音滑行音");
            let sample = decode_sample(name, bytes).expect("静音样本不应报错");
            assert_eq!(sample.frames(), 0);
            assert_eq!(sample.loop_len, 0);
        }
    }

    #[test]
    fn 解码普通打击音不循环() {
        let bytes = embedded_bytes("normal-hitnormal").expect("必须内嵌普通打击音");
        let sample = decode_sample("normal-hitnormal", bytes).expect("普通打击音必须可解码");
        assert!(sample.frames() > 0);
        assert_eq!(sample.loop_len, 0);
    }

    #[test]
    fn 转盘旋转音标记为循环() {
        let bytes = embedded_bytes("spinnerspin").expect("必须内嵌转盘旋转音");
        let sample = decode_sample("spinnerspin", bytes).expect("转盘旋转音必须可解码");
        assert!(sample.frames() > 0);
        assert_eq!(sample.loop_len, sample.frames());
    }

    #[test]
    fn 损坏数据按静音处理而不是panic() {
        // 截断/垃圾数据在真实环境里出现过：必须退化成静音样本，而不是 panic 或中断导出。
        let sample = decode_sample("normal-hitnormal", b"not an ogg file")
            .expect("损坏样本必须按静音处理");
        assert_eq!(sample.frames(), 0);

        let truncated = &embedded_bytes("normal-hitnormal").expect("必须内嵌样本")[..64];
        let sample = decode_sample("normal-hitnormal", truncated).expect("截断样本必须按静音处理");
        assert_eq!(sample.frames(), 0);
    }

    #[test]
    fn 各模式引用的样本名都有内嵌资源() {
        // 裸名与「无 bank 前缀的专用音效」是 osu! 的次级回退查找（共享 Gameplay 目录），
        // 本套皮肤只提供带 bank 前缀的版本，因此这些名字允许缺失。
        const ALLOWED_MISSING: &[&str] = &[
            "hitnormal",
            "hitwhistle",
            "hitfinish",
            "hitclap",
            "slidertick",
            "sliderslide",
            "sliderwhistle",
            "normal-spinnerbonus",
            "normal-spinnerbonus-max",
            "normal-spinnerspin",
        ];
        for beatmap in [
            beatmap_for_names(0),
            beatmap_for_names(1),
            beatmap_for_names(2),
            beatmap_for_names(3),
        ] {
            for name in referenced_names(&beatmap) {
                if ALLOWED_MISSING.contains(&name.as_str()) {
                    continue;
                }
                assert!(
                    has_embedded(&name),
                    "模式 {} 引用了未内嵌的样本 {name}",
                    beatmap.mode()
                );
            }
        }
    }

    /// 构造一个引用全部常规音效名的合成谱面，用于检查资源覆盖。
    fn beatmap_for_names(mode: i32) -> Beatmap {
        use osu_beatmap_preview_core::model::{
            Beatmap, CatchHitObject, HitAddition, HitObjects, HitSample, ManiaHitObject,
            SampleBank, StandardHitObject, TaikoHitObject, TimingPoint,
        };

        let samples = |bank: SampleBank| {
            vec![
                HitSample::new(bank, HitAddition::None, 100, None),
                HitSample::new(bank, HitAddition::Whistle, 100, None),
                HitSample::new(bank, HitAddition::Finish, 100, None),
                HitSample::new(bank, HitAddition::Clap, 100, None),
            ]
        };
        let mut builder = Beatmap {
            metadata: Default::default(),
            difficulty: Default::default(),
            general: Default::default(),
            timing_points: vec![TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 3,
                sample_index: 0,
                sample_volume: 100,
            }],
            hit_objects: HitObjects::Standard(Vec::new()),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        };
        builder.general.insert("Mode", mode.to_string());
        let standard: Vec<StandardHitObject> = [SampleBank::Normal, SampleBank::Soft, SampleBank::Drum]
            .into_iter()
            .enumerate()
            .map(|(index, bank)| StandardHitObject {
                x: 0,
                y: 0,
                start_time: 1000 * index as i64,
                end_time: 1000 * index as i64 + 1000,
                // 2 = 滑条，保证滑行音、tick 与节点音效都被引用到。
                hit_type: 2,
                hitsound: 1,
                slider_repeats: 2,
                slider_pixel_length: 200.0,
                samples: samples(bank),
                slider_edge_samples: vec![samples(bank)],
                ..Default::default()
            })
            .collect();

        builder.hit_objects = match mode {
            1 => HitObjects::Taiko(
                [SampleBank::Normal, SampleBank::Soft, SampleBank::Drum]
                    .into_iter()
                    .enumerate()
                    .map(|(index, bank)| TaikoHitObject {
                        start_time: 1000 * index as i64,
                        end_time: 1000 * index as i64,
                        hit_type: 0,
                        hitsound: 1,
                        samples: samples(bank),
                    })
                    .collect(),
            ),
            2 => HitObjects::Catch(
                [SampleBank::Normal, SampleBank::Soft, SampleBank::Drum]
                    .into_iter()
                    .enumerate()
                    .map(|(index, bank)| CatchHitObject {
                        x: 0,
                        y: 0,
                        start_time: 1000 * index as i64,
                        end_time: 1000 * index as i64 + 1000,
                        hit_type: 2,
                        slider_repeats: 2,
                        slider_pixel_length: 200.0,
                        samples: samples(bank),
                        ..Default::default()
                    })
                    .collect(),
            ),
            3 => HitObjects::Mania(vec![
                ManiaHitObject {
                    lane: 0,
                    start_time: 1000,
                    end_time: 2000,
                    is_long_note: true,
                    samples: Vec::new(),
                },
            ]),
            // standard：滑条覆盖滑行音/tick/节点；转盘覆盖旋转与奖励音。
            _ => {
                let mut objects = standard;
                objects.push(StandardHitObject {
                    x: 0,
                    y: 0,
                    start_time: 5000,
                    end_time: 8000,
                    hit_type: 8,
                    hitsound: 1,
                    samples: samples(SampleBank::Normal),
                    ..Default::default()
                });
                HitObjects::Standard(objects)
            }
        };
        builder
    }
}

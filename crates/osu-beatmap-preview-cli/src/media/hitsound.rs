//! 打击音样本的解码与样本库构建。
//!
//! 样本有两个来源：二进制内嵌的默认皮肤，以及**谱面自带**的打击音文件（`ENABLE_BEATMAP_HITSOUND`
//! 打开时从该谱面的 OSZ 里取）。优先级是「OSZ 内同名条目 > 内嵌皮肤 > 静音」，与 osu! 的
//! 谱面皮肤一致：谱面自定义的音效应当盖过默认音色，只有它缺失时才回退。
//!
//! CLI 的 MP4 音频是一次性离线混音，所以这里只在需要时解码「时间轴实际引用到」
//! 的样本，并且任何单个样本解码失败都按静音处理——资源损坏不应该让整次导出失败。

use osu_beatmap_preview_core::hitsound::{referenced_names, SampleData, SampleLibrary};
use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::processing::media::{
    entry_extension, normalize_entry_path, sample_entry_matches,
};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use std::fs::File;
use std::io::Read;
use std::path::Path;
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

/// 单个谱面自带样本的大小上限。
///
/// 打击音都是几十 KB 的短音效，8 MiB 足够容纳罕见的整段人声采样，又能拦住
/// 「压缩包里塞了异常大的文件」这种情况，避免一次导出把内存吃满。
const MAX_SAMPLE_BYTES: u64 = 8 * 1024 * 1024;

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

/// 谱面自带打击音所在的 OSZ。
///
/// 打开一次并缓存条目名，再按候选名逐个抽取：一次导出要查二十多个候选名，
/// 每次都重新打开压缩包就会反复解析中央目录。
struct SampleArchive {
    archive: zip::ZipArchive<File>,
    /// 归一化后的条目名；目录或非法路径为 `None`，下标与 zip 条目下标一一对应。
    names: Vec<Option<String>>,
}

impl SampleArchive {
    /// 打开压缩包并建立条目名索引；文件打不开或不是合法 ZIP 时返回 `None`
    ///（调用方退化成只用内嵌资源，不让整次导出失败）。
    fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let mut names = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            let name = archive
                .by_index(index)
                .ok()
                .filter(|entry| !entry.is_dir())
                .and_then(|entry| normalize_entry_path(entry.name()));
            names.push(name);
        }
        Some(Self { archive, names })
    }

    /// 找到候选样本名对应的条目下标。
    fn find(&self, candidate: &str) -> Option<usize> {
        self.names.iter().position(|name| {
            name.as_deref()
                .is_some_and(|name| sample_entry_matches(name, candidate))
        })
    }

    /// 取出条目字节与扩展名；条目过大或读取失败时返回 `None`。
    fn extract(&mut self, index: usize) -> Option<(Vec<u8>, Option<String>)> {
        let extension = self.names.get(index)?.as_deref().and_then(entry_extension);
        let mut entry = self.archive.by_index(index).ok()?;
        if entry.size() == 0 || entry.size() > MAX_SAMPLE_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .by_ref()
            .take(MAX_SAMPLE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() as u64 <= MAX_SAMPLE_BYTES).then_some((bytes, extension))
    }
}

/// 为给定谱面构建打击音样本库。
///
/// `beatmap_samples` 为 `Some` 时表示启用谱面自带音效（对应配置项
/// `ENABLE_BEATMAP_HITSOUND`），传的是该谱面的 OSZ 路径；每个候选名按
///「OSZ 内同名条目 > 内嵌皮肤」的顺序取字节，两者都没有时跳过（混音阶段按静音处理）。
/// 为 `None` 时只用内嵌资源。
///
/// 只解包时间轴引用到的那几十个短音效，所以不需要截止时间：整体耗时在毫秒级。
pub(crate) fn build_library(beatmap: &Beatmap, beatmap_samples: Option<&Path>) -> SampleLibrary {
    let mut archive = beatmap_samples.and_then(SampleArchive::open);
    let mut library = SampleLibrary::new();
    for name in referenced_names(beatmap) {
        if let Some(archive) = archive.as_mut() {
            if let Some(index) = archive.find(&name) {
                if let Some((bytes, extension)) = archive.extract(index) {
                    if let Ok(sample) = decode_sample(&name, &bytes, extension.as_deref()) {
                        library.insert(name, sample);
                        continue;
                    }
                    // 谱面自带的文件坏了（格式不支持、被截断）时回退到内嵌音效：
                    // 与其让这个音效整场静音，不如用默认音色顶上。
                }
            }
        }
        let Some(bytes) = embedded_bytes(&name) else {
            continue;
        };
        match decode_sample(&name, bytes, Some("ogg")) {
            // 解码失败按静音处理：单个坏文件不应该让整次导出失败。
            // 这里刻意不写日志——日志是全局单例，媒体层的坏样本属于预期情况，
            // 交给调用方按需统计即可。
            Ok(sample) => library.insert(name, sample),
            Err(_) => continue,
        }
    }
    library
}

/// 解码一段打击音为 f32 立体声。
///
/// `name` 用于判定是否需要整段循环：滑条滑行音与转盘旋转音在游戏里都是持续循环播放的。
/// `extension` 是文件真实扩展名，只作为 symphonia 探测格式的提示（内容仍是最终依据）；
/// 谱面自带的样本可能是 wav/mp3，不能一律按 ogg 猜。
fn decode_sample(name: &str, bytes: &[u8], extension: Option<&str>) -> Result<SampleData> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }
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
    use std::io::Write;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    /// 一段 16bit 单声道 PCM WAV；样本值在正负之间交替，便于确认解出的帧数。
    fn wav_bytes(sample_rate: u32, frames: usize) -> Vec<u8> {
        let data_len = (frames * 2) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for index in 0..frames {
            let value = if index % 2 == 0 { 8000_i16 } else { -8000_i16 };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// 建一个只存在于本次测试的临时目录。
    fn temp_dir(tag: &str) -> PathBuf {
        let unique = format!(
            "osu-preview-hitsound-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 把若干条目写成一个 OSZ。
    fn write_osz(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    /// 谱面：soft 音效组，物件带自定义文件名，另有 clap 加成音与自定义音效索引。
    fn beatmap_with_custom_samples() -> Beatmap {
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,2,0:0:0:100:Custom-Hit.WAV\n256,192,2000,1,8,0:0:0:0:\n256,192,3000,1,0,0:0:20:0:\n";
        osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes()).expect("fixture 必须可解析")
    }

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
            let sample = decode_sample(name, bytes, Some("ogg")).expect("静音样本不应报错");
            assert_eq!(sample.frames(), 0);
            assert_eq!(sample.loop_len, 0);
        }
    }

    #[test]
    fn 解码普通打击音不循环() {
        let bytes = embedded_bytes("normal-hitnormal").expect("必须内嵌普通打击音");
        let sample =
            decode_sample("normal-hitnormal", bytes, Some("ogg")).expect("普通打击音必须可解码");
        assert!(sample.frames() > 0);
        assert_eq!(sample.loop_len, 0);
    }

    #[test]
    fn 转盘旋转音标记为循环() {
        let bytes = embedded_bytes("spinnerspin").expect("必须内嵌转盘旋转音");
        let sample = decode_sample("spinnerspin", bytes, Some("ogg")).expect("转盘旋转音必须可解码");
        assert!(sample.frames() > 0);
        assert_eq!(sample.loop_len, sample.frames());
    }

    #[test]
    fn 损坏数据按静音处理而不是panic() {
        // 截断/垃圾数据在真实环境里出现过：必须退化成静音样本，而不是 panic 或中断导出。
        let sample = decode_sample("normal-hitnormal", b"not an ogg file", Some("ogg"))
            .expect("损坏样本必须按静音处理");
        assert_eq!(sample.frames(), 0);

        let truncated = &embedded_bytes("normal-hitnormal").expect("必须内嵌样本")[..64];
        let sample = decode_sample("normal-hitnormal", truncated, Some("ogg"))
            .expect("截断样本必须按静音处理");
        assert_eq!(sample.frames(), 0);
    }

    #[test]
    fn 谱面自带的同名音效优先于内嵌皮肤() {
        let dir = temp_dir("override");
        let osz = dir.join("fixture.osz");
        // 8kHz 4 帧的 wav：与内嵌 ogg 的采样率和长度都不同，足以区分来源。
        let custom = wav_bytes(8_000, 4);
        write_osz(
            &osz,
            &[
                ("Soft-Hitnormal.WAV", &custom),
                // 自定义音效索引 20：文件名是 `soft-hitnormal20`（无分隔符）。
                ("Soft-Hitnormal20.WAV", &custom),
                // 自定义文件名可以带子目录：按文件名匹配，仍然命中候选 `Custom-Hit.WAV`。
                ("sub/Custom-Hit.WAV", &custom),
            ],
        );
        let beatmap = beatmap_with_custom_samples();

        let with_beatmap = build_library(&beatmap, Some(&osz));
        let embedded_only = build_library(&beatmap, None);

        let overridden = with_beatmap.get("soft-hitnormal").expect("必须解析出音效");
        assert_eq!(overridden.sample_rate, 8_000);
        assert_eq!(overridden.frames(), 4);
        // 自定义音效索引同样来自谱面：候选名是 `{bank}-{name}{index}`。
        let suffixed = with_beatmap.get("soft-hitnormal20").expect("必须解析出带索引的音效");
        assert_eq!(suffixed.sample_rate, 8_000);
        assert!(embedded_only.get("soft-hitnormal20").is_none());
        // 自定义文件名同样来自谱面：候选名就是 `hitSample` 里写的那个名字。
        let custom_sample = with_beatmap
            .get("Custom-Hit.WAV")
            .expect("必须解析出自定义音效");
        assert_eq!(custom_sample.sample_rate, 8_000);
        // 谱面没提供的音效回退到内嵌皮肤，内容与关闭谱面音效时完全一致。
        let fallback = with_beatmap.get("soft-hitclap").expect("clap 必须回退到内嵌");
        let embedded_clap = embedded_only.get("soft-hitclap").expect("clap 必须内嵌");
        assert_eq!(fallback, embedded_clap);
        assert_ne!(embedded_only.get("soft-hitnormal"), Some(overridden));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 缺少压缩包或条目不全时仍然可用() {
        // 关闭谱面音效（`None`）与压缩包打不开都必须退化成内嵌资源，而不是空库。
        let beatmap = beatmap_with_custom_samples();
        let embedded_only = build_library(&beatmap, None);
        assert!(!embedded_only.is_empty());

        let missing = build_library(&beatmap, Some(Path::new("不存在的.osz")));
        assert_eq!(missing.len(), embedded_only.len());
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
        let standard: Vec<StandardHitObject> =
            [SampleBank::Normal, SampleBank::Soft, SampleBank::Drum]
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
            3 => HitObjects::Mania(vec![ManiaHitObject {
                lane: 0,
                start_time: 1000,
                end_time: 2000,
                is_long_note: true,
                samples: Vec::new(),
            }]),
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

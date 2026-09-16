use crate::export::canvas::Img;
use fdk_aac::enc::{AudioObjectType, BitRate, ChannelMode, Encoder, EncoderParams, Transport};
use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::processing::media::{normalize_entry_path, BeatmapMedia, MediaEntry};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone)]
pub(crate) struct AudioSource {
    pub path: PathBuf,
    pub lead_in_ms: i64,
    /// 音频所在的 OSZ；谱面自带打击音也从同一个压缩包里取。
    pub osz_path: PathBuf,
}

pub(crate) struct AudioSourceJob {
    handle: Option<std::thread::JoinHandle<Result<AudioSource>>>,
    deadline: RequestDeadline,
    background: Option<Img>,
}

impl AudioSourceJob {
    pub(crate) fn start(
        request_bid: &str,
        beatmap: Beatmap,
        cache_dir: PathBuf,
        no_cache: bool,
        deadline: RequestDeadline,
        mode: crate::export::geometry::GameMode,
    ) -> Result<Self> {
        let set_id = beatmap.beatmap_set_id().ok_or_else(|| {
            PreviewError::parse("missing or invalid BeatmapSetID required for MP4 audio")
        })?;
        let request_bid = request_bid.to_string();
        let audio_filename = beatmap
            .audio_filename()
            .ok_or_else(|| PreviewError::parse("missing AudioFilename required for MP4 audio"))?;
        crate::logging::event(
            "audio-prepare",
            "start",
            None,
            &format!("set_id={set_id} audio={audio_filename}"),
        );

        let osz_path = crate::download::download_beatmapset_archive(
            &request_bid,
            set_id,
            &cache_dir,
            no_cache,
            &deadline,
        )?;
        let media = BeatmapMedia::from_beatmap(&beatmap);
        let background = if super::video_style(mode).enable_background_image {
            load_background_image(media.background.as_ref(), &osz_path, &deadline)?
        } else {
            None
        };
        let worker_deadline = deadline.clone();
        let handle = std::thread::spawn(move || {
            worker_deadline.check()?;
            prepare_audio_source(&beatmap, &osz_path, &cache_dir, no_cache, &worker_deadline)
        });
        Ok(Self {
            handle: Some(handle),
            deadline,
            background,
        })
    }

    pub(crate) fn take_background(&mut self) -> Option<Img> {
        self.background.take()
    }

    pub(crate) fn wait(mut self) -> Result<AudioSource> {
        self.handle
            .take()
            .expect("audio source job waited more than once")
            .join()
            .map_err(|_| PreviewError::render("audio preparation worker panicked"))?
    }
}

impl Drop for AudioSourceJob {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        self.deadline.cancel();
        let _ = handle.join();
    }
}

#[derive(Debug)]
pub(crate) struct EncodedAudio {
    pub frames: Vec<EncodedAudioFrame>,
}

#[derive(Debug)]
pub(crate) struct EncodedAudioFrame {
    pub bytes: Vec<u8>,
    pub duration: u32,
}

struct DecodedAudio {
    sample_rate: u32,
    stereo_samples: Vec<i16>,
}

const MAX_BACKGROUND_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn prepare_audio_source(
    beatmap: &Beatmap,
    osz_path: &Path,
    cache_dir: &Path,
    no_cache: bool,
    deadline: &RequestDeadline,
) -> Result<AudioSource> {
    deadline.check()?;
    let set_id = beatmap.beatmap_set_id().ok_or_else(|| {
        PreviewError::parse("missing or invalid BeatmapSetID required for MP4 audio")
    })?;
    let Some(audio) = BeatmapMedia::from_beatmap(beatmap).audio else {
        return Err(PreviewError::parse(
            "missing or unusable AudioFilename required for MP4 audio",
        ));
    };
    // 缓存文件名 = 条目名哈希 + 扩展名；扩展名由 core 的策略给出，缺失时沿用 `audio`。
    let extension = audio.extension.as_deref().unwrap_or("audio");
    let key = fnv1a64(audio.name.as_bytes());
    let set_cache = cache_dir.join(set_id.to_string());
    let target_path = set_cache.join(format!("{key:016x}.{extension}"));

    if !no_cache
        && target_path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() > 0)
    {
        crate::logging::event(
            "audio-prepare",
            "done",
            None,
            &format!("audio cache hit: {}", target_path.display()),
        );
        crate::logging::record_cache(crate::logging::CacheKind::Audio, "hit");
        return Ok(AudioSource {
            path: target_path,
            lead_in_ms: beatmap.audio_lead_in_ms(),
            osz_path: osz_path.to_path_buf(),
        });
    }

    std::fs::create_dir_all(&set_cache)
        .map_err(|e| PreviewError::download(format!("failed to create audio cache dir: {e}")))?;
    extract_audio_entry(osz_path, &audio.name, &target_path, deadline)?;
    crate::logging::event(
        "audio-prepare",
        "done",
        None,
        &format!("extracted audio: {}", target_path.display()),
    );
    crate::logging::record_cache(crate::logging::CacheKind::Audio, "downloaded");
    Ok(AudioSource {
        path: target_path,
        lead_in_ms: beatmap.audio_lead_in_ms(),
        osz_path: osz_path.to_path_buf(),
    })
}

/// 从 OSZ 中读取并解码谱面背景图；缺少背景图时回退到纯色背景。
///
/// 条目名由 core 的媒体策略（[`BeatmapMedia`]）给出，这里只负责打开压缩包、
/// 按名（不区分大小写）查找并解码。
pub(crate) fn load_background_image(
    background: Option<&MediaEntry>,
    osz_path: &Path,
    deadline: &RequestDeadline,
) -> Result<Option<Img>> {
    let Some(entry) = background else {
        crate::logging::event(
            "background-prepare",
            "skip",
            None,
            "beatmap has no usable background event",
        );
        return Ok(None);
    };
    let file = File::open(osz_path)
        .map_err(|e| PreviewError::download(format!("failed to open osz archive: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| PreviewError::download(format!("invalid osz archive: {e}")))?;
    let wanted = entry.name.as_str();
    let index = (0..archive.len()).find(|&index| {
        archive
            .by_index(index)
            .ok()
            .and_then(|entry| normalize_entry_path(entry.name()))
            .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
    });
    let Some(index) = index else {
        crate::logging::event(
            "background-prepare",
            "skip",
            None,
            &format!("background file was not found in osz: {wanted}"),
        );
        return Ok(None);
    };
    let mut entry = archive
        .by_index(index)
        .map_err(|e| PreviewError::download(format!("failed to open background entry: {e}")))?;
    if entry.is_dir() || entry.size() == 0 || entry.size() > MAX_BACKGROUND_IMAGE_BYTES {
        crate::logging::event(
            "background-prepare",
            "skip",
            None,
            "invalid background image size",
        );
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(MAX_BACKGROUND_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PreviewError::download(format!("failed to extract background image: {e}")))?;
    if bytes.len() as u64 > MAX_BACKGROUND_IMAGE_BYTES {
        crate::logging::event(
            "background-prepare",
            "skip",
            None,
            "background image is too large",
        );
        return Ok(None);
    }
    deadline.check()?;
    let decoded = match image::load_from_memory(&bytes) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            crate::logging::event(
                "background-prepare",
                "skip",
                None,
                &format!("failed to decode background image: {error}"),
            );
            return Ok(None);
        }
    };
    let (w, h) = decoded.dimensions();
    if w == 0 || h == 0 {
        return Ok(None);
    }
    crate::logging::event(
        "background-prepare",
        "done",
        None,
        &format!("loaded {wanted} ({w}x{h})"),
    );
    Ok(Some(Img {
        w,
        h,
        data: decoded.into_raw(),
    }))
}

pub(crate) fn encode_audio_segment(
    source: &AudioSource,
    beatmap: &Beatmap,
    hitsound: Option<HitsoundSettings>,
    chart_start_ms: i64,
    frame_count: usize,
    fps: u32,
    speed: f64,
    deadline: &RequestDeadline,
) -> Result<EncodedAudio> {
    deadline.check()?;
    if fps == 0 || !speed.is_finite() || speed <= 0.0 {
        return Err(PreviewError::render(
            "invalid video timing for audio encoding",
        ));
    }
    let decoded = decode_audio(&source.path, deadline)?;
    let sample_rate = crate::config::current()
        .advance
        .video_audio
        .AUDIO_SAMPLE_RATE;
    let target_samples = (frame_count as u64 * sample_rate as u64).div_ceil(fps as u64);
    if target_samples == 0 {
        return Err(PreviewError::render("audio segment is empty"));
    }
    // 打击音与音乐共用同一个 48kHz 时间轴：整个片段一次性混好，按输出帧号取样，
    // 因此不需要在编码循环里做任何时间换算，也不会出现累计漂移。
    // 谱面自带音效与音乐来自同一个 OSZ；配置关闭时只用内嵌皮肤。
    let hitsound = hitsound.map(|settings| {
        let osz = settings.beatmap.then(|| source.osz_path.clone());
        settings.with_beatmap_samples(osz.as_deref())
    });
    let hitsound = render_hitsound_segment(
        beatmap,
        hitsound,
        chart_start_ms,
        frame_count,
        fps,
        speed,
        sample_rate,
    )?;

    let encoder = Encoder::new(EncoderParams {
        bit_rate: BitRate::Cbr(crate::config::current().advance.video_audio.AUDIO_BITRATE),
        sample_rate,
        transport: Transport::Raw,
        channels: ChannelMode::Stereo,
        audio_object_type: AudioObjectType::Mpeg4LowComplexity,
    })
    .map_err(|e| PreviewError::render(format!("failed to initialize AAC encoder: {e}")))?;
    let info = encoder
        .info()
        .map_err(|e| PreviewError::render(format!("failed to query AAC encoder: {e}")))?;
    let samples_per_frame = info.frameLength.max(1) as usize;
    let target_frame_count = target_samples.div_ceil(samples_per_frame as u64) as usize;
    let max_output_bytes = (info.maxOutBufBytes.max(8192)) as usize;
    let encoder_delay_samples = info.nDelay as usize;
    let mut input = vec![0_i16; samples_per_frame * 2];
    let mut output = vec![0_u8; max_output_bytes];
    let mut frames = Vec::with_capacity(target_frame_count);

    // FDK 在内部缓冲编码器前瞻数据，因此请求输入区间结束后可能还需执行几次补零调用，
    // 才能取出全部访问单元。
    let max_calls = target_frame_count + 16;
    for input_frame_index in 0..max_calls {
        deadline.check()?;
        fill_audio_frame(
            &mut input,
            input_frame_index * samples_per_frame,
            target_samples,
            &decoded,
            chart_start_ms,
            speed,
            encoder_delay_samples,
            hitsound.as_deref(),
        );
        let encoded = encoder
            .encode(&input, &mut output)
            .map_err(|e| PreviewError::render(format!("AAC encoding failed: {e}")))?;
        if encoded.output_size > 0 {
            let remaining =
                target_samples.saturating_sub(frames.len() as u64 * samples_per_frame as u64);
            frames.push(EncodedAudioFrame {
                bytes: output[..encoded.output_size].to_vec(),
                duration: remaining.min(samples_per_frame as u64) as u32,
            });
            if frames.len() == target_frame_count {
                break;
            }
        }
    }
    if frames.len() != target_frame_count {
        return Err(PreviewError::render(format!(
            "AAC encoder produced {} of {target_frame_count} required frames",
            frames.len()
        )));
    }

    Ok(EncodedAudio { frames })
}

/// MP4 导出使用的打击音开关、音量与样本来源（来自各模式 `mp4.style` 配置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HitsoundSettings {
    pub enabled: bool,
    /// 音量百分比（0～100）。
    pub volume: i32,
    /// 是否使用谱面自带的自定义打击音（`ENABLE_BEATMAP_HITSOUND`）。
    pub beatmap: bool,
    /// 谱面自带打击音所在的 OSZ；`None` 表示只用内嵌皮肤。
    pub beatmap_samples: Option<PathBuf>,
}

impl HitsoundSettings {
    /// 从配置构造；`beatmap` 对应 `ENABLE_BEATMAP_HITSOUND`。此处的样本来源为空，
    /// 音频准备完成后由 [`HitsoundSettings::with_beatmap_samples`] 补上压缩包路径。
    pub(crate) fn new(enabled: bool, volume: i32, beatmap: bool) -> Self {
        Self {
            enabled,
            volume,
            beatmap,
            beatmap_samples: None,
        }
    }

    /// 附带谱面自带音效所在的 OSZ；传 `None` 时只用内嵌皮肤。
    pub(crate) fn with_beatmap_samples(mut self, path: Option<&Path>) -> Self {
        self.beatmap_samples = path.map(Path::to_path_buf);
        self
    }
}

/// 按视频输出时间轴混出整段打击音。
///
/// 返回 `None` 表示本次导出没有启用打击音（配置关闭或没有任何可用样本）。
///
/// 缓冲区下标即输出帧下标，与音乐（`source_frame_position`）共用同一时间换算：
/// 第 i 帧对应的谱面时间是 `chart_start_ms + i * 1000 * speed / sample_rate`。
/// 倍速通过「混音器的内部采样率取 `sample_rate / speed`」实现，音高随之变化，
/// 与 Web 端音频线程的倍速播放一致。
fn render_hitsound_segment(
    beatmap: &Beatmap,
    settings: Option<HitsoundSettings>,
    chart_start_ms: i64,
    frame_count: usize,
    fps: u32,
    speed: f64,
    sample_rate: u32,
) -> Result<Option<Vec<f32>>> {
    let Some(settings) = settings.filter(|settings| settings.enabled) else {
        return Ok(None);
    };
    let library = super::hitsound::build_library(beatmap, settings.beatmap_samples.as_deref());
    if library.is_empty() {
        return Ok(None);
    }
    let timeline = osu_beatmap_preview_core::hitsound::build_timeline(beatmap, &library);
    if timeline.is_empty() {
        return Ok(None);
    }

    let frames = (frame_count as u64 * sample_rate as u64).div_ceil(fps as u64);
    if frames == 0 {
        return Ok(None);
    }
    let frames = frames.min(u32::MAX as u64) as usize;
    // 混音器采样率必须取整：奇数倍速会有最多半帧的换算误差（整段漂移几毫秒，
    // 打击音听不出来），而 0.75 / 1.5 / 2.0 这些常见倍速都是整除的。
    let mixer_rate = ((sample_rate as f64 / speed).round() as u32).max(1);
    let mut mixer =
        osu_beatmap_preview_core::hitsound::HitsoundMixer::new(library, timeline, mixer_rate);
    mixer.set_master_gain(osu_beatmap_preview_core::hitsound::volume_gain(
        settings.volume.clamp(0, 100),
    ));
    // 视频区间起点可能为负（首个物件前 2000ms 的预卷）：混音位置允许为负，
    // 缓冲区第 0 帧必须对应该起点，否则整段打击音会提前 |起点|。
    mixer.seek(chart_start_ms as f64);

    // 分块渲染：混音器每个窗口只保留本窗口内还在响的声音，一次渲染整段会把所有事件
    // 都压在 voices 里、逐输出帧遍历一遍（O(输出帧 × 事件数)），三分钟的谱面就能让
    // 音频线程比视频编码还慢。窗口取 1 秒。
    let mut buffer = vec![0.0_f32; frames * 2];
    let window_frames = sample_rate.max(1) as usize;
    for chunk in buffer.chunks_mut(window_frames * 2) {
        mixer.render_into(chunk);
    }
    Ok(Some(buffer))
}

#[allow(clippy::too_many_arguments)]
fn fill_audio_frame(
    output: &mut [i16],
    output_frame_start: usize,
    target_samples: u64,
    decoded: &DecodedAudio,
    chart_start_ms: i64,
    speed: f64,
    encoder_delay_samples: usize,
    hitsound: Option<&[f32]>,
) {
    for (frame_offset, stereo) in output.chunks_exact_mut(2).enumerate() {
        let output_index = output_frame_start + frame_offset;
        if output_index as u64 >= target_samples {
            stereo.fill(0);
            continue;
        }
        let source_frame = source_frame_position(
            output_index,
            encoder_delay_samples,
            decoded.sample_rate,
            chart_start_ms,
            speed,
        );
        let [left, right] = sample_stereo(decoded, source_frame);
        // 打击音与音乐在同一输出下标处相加：两者都按同一时间轴换算，
        // 所以即使倍速播放也保持同步。
        let (hit_left, hit_right) = match hitsound {
            Some(buffer) => {
                let index = output_index + encoder_delay_samples;
                (
                    buffer.get(index * 2).copied().unwrap_or(0.0),
                    buffer.get(index * 2 + 1).copied().unwrap_or(0.0),
                )
            }
            None => (0.0, 0.0),
        };
        stereo[0] = mix_i16(left, hit_left);
        stereo[1] = mix_i16(right, hit_right);
    }
}

/// 把线性 PCM 与 f32 打击音相加并夹紧到 i16。
fn mix_i16(music: i16, hitsound: f32) -> i16 {
    if hitsound == 0.0 {
        return music;
    }
    let mixed = music as f32 + hitsound * i16::MAX as f32;
    mixed.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

fn source_frame_position(
    output_index: usize,
    encoder_delay_samples: usize,
    source_sample_rate: u32,
    chart_start_ms: i64,
    speed: f64,
) -> f64 {
    let chart_time_ms = chart_start_ms as f64
        + (output_index + encoder_delay_samples) as f64 * 1000.0 * speed
            / crate::config::current()
                .advance
                .video_audio
                .AUDIO_SAMPLE_RATE as f64;
    chart_time_ms * source_sample_rate as f64 / 1000.0
}

fn sample_stereo(decoded: &DecodedAudio, source_frame: f64) -> [i16; 2] {
    if !source_frame.is_finite() || source_frame < 0.0 {
        return [0, 0];
    }
    let frame_count = decoded.stereo_samples.len() / 2;
    let index = source_frame.floor() as usize;
    if index >= frame_count {
        return [0, 0];
    }
    let next = (index + 1).min(frame_count - 1);
    let fraction = source_frame - index as f64;
    let interpolate = |channel: usize| {
        let a = decoded.stereo_samples[index * 2 + channel] as f64;
        let b = decoded.stereo_samples[next * 2 + channel] as f64;
        (a + (b - a) * fraction)
            .round()
            .clamp(i16::MIN as f64, i16::MAX as f64) as i16
    };
    [interpolate(0), interpolate(1)]
}

fn decode_audio(path: &Path, deadline: &RequestDeadline) -> Result<DecodedAudio> {
    let file = File::open(path)
        .map_err(|e| PreviewError::render(format!("failed to open beatmap audio: {e}")))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| PreviewError::render(format!("unsupported beatmap audio format: {e}")))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| PreviewError::render("beatmap audio has no decodable track"))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| PreviewError::render(format!("unsupported beatmap audio codec: {e}")))?;
    let mut sample_rate = None;
    let mut stereo_samples = Vec::new();

    loop {
        deadline.check()?;
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(error) => {
                return Err(PreviewError::render(format!(
                    "failed to read beatmap audio: {error}"
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
                    "failed to decode beatmap audio: {error}"
                )))
            }
        };
        let spec = *decoded.spec();
        if spec.rate == 0 || spec.channels.count() == 0 {
            return Err(PreviewError::render(
                "beatmap audio has an invalid sample format",
            ));
        }
        if sample_rate.is_some_and(|rate| rate != spec.rate) {
            return Err(PreviewError::render(
                "beatmap audio changes sample rate mid-stream",
            ));
        }
        sample_rate = Some(spec.rate);
        let mut sample_buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
        sample_buffer.copy_interleaved_ref(decoded);
        append_as_stereo(
            sample_buffer.samples(),
            spec.channels.count(),
            &mut stereo_samples,
        );
    }
    if stereo_samples.is_empty() {
        return Err(PreviewError::render("beatmap audio decoded to no samples"));
    }
    Ok(DecodedAudio {
        sample_rate: sample_rate.unwrap_or(
            crate::config::current()
                .advance
                .video_audio
                .AUDIO_SAMPLE_RATE,
        ),
        stereo_samples,
    })
}

fn append_as_stereo(input: &[i16], channels: usize, output: &mut Vec<i16>) {
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

fn extract_audio_entry(
    osz_path: &Path,
    wanted: &str,
    target_path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    let file = File::open(osz_path)
        .map_err(|e| PreviewError::download(format!("failed to open osz archive: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| PreviewError::download(format!("invalid osz archive: {e}")))?;
    let index = find_audio_entry(&mut archive, wanted)?;
    let mut entry = archive
        .by_index(index)
        .map_err(|e| PreviewError::download(format!("failed to open audio entry: {e}")))?;
    if entry.is_dir()
        || entry.size() == 0
        || entry.size()
            > crate::config::current()
                .advance
                .video_audio
                .MAX_EXTRACTED_AUDIO_BYTES
    {
        return Err(PreviewError::download(format!(
            "invalid extracted audio size: {} bytes",
            entry.size()
        )));
    }

    let part_path = target_path.with_extension("part");
    let mut output = File::create(&part_path)
        .map_err(|e| PreviewError::download(format!("failed to create audio cache file: {e}")))?;
    let copied = std::io::copy(
        &mut entry.by_ref().take(
            crate::config::current()
                .advance
                .video_audio
                .MAX_EXTRACTED_AUDIO_BYTES
                + 1,
        ),
        &mut output,
    )
    .map_err(|e| PreviewError::download(format!("failed to extract beatmap audio: {e}")))?;
    deadline.check().inspect_err(|_| {
        let _ = std::fs::remove_file(&part_path);
    })?;
    output
        .flush()
        .map_err(|e| PreviewError::download(format!("failed to flush audio cache: {e}")))?;
    if copied == 0
        || copied
            > crate::config::current()
                .advance
                .video_audio
                .MAX_EXTRACTED_AUDIO_BYTES
    {
        let _ = std::fs::remove_file(&part_path);
        return Err(PreviewError::download(
            "extracted audio is empty or too large",
        ));
    }
    if target_path.exists() {
        std::fs::remove_file(target_path)
            .map_err(|e| PreviewError::download(format!("failed to replace audio cache: {e}")))?;
    }
    std::fs::rename(&part_path, target_path)
        .map_err(|e| PreviewError::download(format!("failed to commit audio cache: {e}")))?;
    Ok(())
}

fn find_audio_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    wanted: &str,
) -> Result<usize> {
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| PreviewError::download(format!("failed to inspect osz entry: {e}")))?;
        // 条目名与 .osu 声明都要先归一化：反斜杠、多余的 `.` 与空段都不影响匹配，
        // 规则由 core 的媒体策略统一给出。
        if let Some(name) = normalize_entry_path(entry.name()) {
            if name.eq_ignore_ascii_case(wanted) {
                return Ok(index);
            }
        }
    }
    Err(PreviewError::download(format!(
        "AudioFilename '{wanted}' was not found in the osz archive"
    )))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;
    use zip::write::SimpleFileOptions;

    #[test]
    fn finds_nested_audio_case_insensitively() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            writer
                .start_file("Audio/Song.MP3", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"not-real-audio").unwrap();
            writer.finish().unwrap();
        }
        bytes.set_position(0);
        let mut archive = zip::ZipArchive::new(bytes).unwrap();
        assert_eq!(find_audio_entry(&mut archive, "audio/song.mp3").unwrap(), 0);
    }

    #[test]
    fn extracts_only_the_requested_audio_entry() {
        let unique = format!(
            "osu-preview-audio-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        let osz = dir.join("fixture.osz");
        let output = dir.join("song.mp3");
        {
            let file = File::create(&osz).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("other.bin", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"other").unwrap();
            writer
                .start_file("Audio/Song.MP3", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"requested-audio").unwrap();
            writer.finish().unwrap();
        }
        let deadline = RequestDeadline::new(
            std::time::Instant::now(),
            "mp4",
            std::time::Duration::from_secs(300),
        );
        extract_audio_entry(&osz, "audio/song.mp3", &output, &deadline).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"requested-audio");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loads_referenced_background_from_osz_case_insensitively() {
        let _log_guard = crate::logging::test_guard();
        let unique = format!(
            "osu-preview-background-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        let osz = dir.join("fixture.osz");
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255])
                .unwrap();
        }
        {
            let file = File::create(&osz).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("Backgrounds/BG.PNG", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&png_bytes).unwrap();
            writer.finish().unwrap();
        }
        let deadline = RequestDeadline::new(
            std::time::Instant::now(),
            "mp4",
            std::time::Duration::from_secs(300),
        );
        let background = load_background_image(
            MediaEntry::new(r"backgrounds\bg.png").as_ref(),
            &osz,
            &deadline,
        )
        .unwrap()
        .unwrap();
        assert_eq!((background.w, background.h), (2, 1));
        assert_eq!(background.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(background.get(1, 0), [0, 255, 0, 255]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 真实谱面的打击音混音必须产生非零样本。
    #[test]
    fn hitsound_mix_on_real_beatmap_produces_non_silent_samples() {
        // 回归：这条链路一旦断掉（样本没内嵌、时间轴为空、位置换算错），
        // 导出的 MP4 就会完全没有打击音，而单元测试以外很难发现。
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n";
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes())
            .expect("fixture 必须可解析");

        let library = crate::media::hitsound::build_library(&beatmap, None);
        assert!(!library.is_empty(), "内嵌资源没有解码出任何样本");

        let settings = Some(HitsoundSettings::new(true, 100, false));
        // `frame_count` 是视频帧数：48 帧 @48fps = 1 秒音频。
        // 起点取 900ms，让第一个打击音（谱面时间 1000ms）落在缓冲**中间**——
        // 放在窗口边界上测不出问题（边界事件由混音器自己的测试覆盖）。
        let sample_rate = crate::config::current()
            .advance
            .video_audio
            .AUDIO_SAMPLE_RATE;
        let video_frames = 48_u32;
        let fps = 48_u32;
        let chart_start_ms = 900;
        let expected_samples = (video_frames as u64 * sample_rate as u64).div_ceil(fps as u64);
        let mixed = render_hitsound_segment(
            &beatmap,
            settings,
            chart_start_ms,
            video_frames as usize,
            fps,
            1.0,
            sample_rate,
        )
        .expect("混音不应失败")
        .expect("启用且样本充足时必须产生混音缓冲");
        assert_eq!(mixed.len(), expected_samples as usize * 2);
        let peak = mixed.iter().fold(0.0_f32, |acc, value| acc.max(value.abs()));
        assert!(peak > 0.01, "打击音缓冲全为零（peak={peak}）");

        // 关闭开关时不产生缓冲。
        let disabled = render_hitsound_segment(
            &beatmap,
            Some(HitsoundSettings::new(false, 100, false)),
            chart_start_ms,
            video_frames as usize,
            fps,
            1.0,
            sample_rate,
        )
        .expect("关闭开关不应失败");
        assert!(disabled.is_none());
    }

    /// 每 10ms 统计一次能量，返回「由静转动」的起音点（缓冲区时间，毫秒）。
    fn onsets_ms(mixed: &[f32], sample_rate: u32) -> Vec<f64> {
        let window = sample_rate as usize / 100;
        let mut onsets = Vec::new();
        let mut previous_loud = false;
        for (index, chunk) in mixed.chunks(window * 2).enumerate() {
            let peak = chunk
                .iter()
                .fold(0.0_f32, |acc, value| acc.max(value.abs()));
            let loud = peak > 0.01;
            if loud && !previous_loud {
                onsets.push(index as f64 * window as f64 * 1000.0 / sample_rate as f64);
            }
            previous_loud = loud;
        }
        onsets
    }

    /// 长谱面混音不得退化成平方复杂度。
    #[test]
    fn long_beatmap_hitsound_mix_stays_linear() {
        // 回归：整段只用一个混音窗口时，所有事件都会压在 voices 里、逐输出帧遍历一遍
        // （O(输出帧 × 事件数)）：release 下「60 秒 + 1200 个事件」要 6.6 秒，三分钟的
        // 普通谱面要 10 秒上下，导出总耗时因此翻倍。分块渲染后同样的工作量约 0.45 秒。
        // 下面的上限留了二十倍余量，既容得下慢机器，又能拦住「退回单窗口」的改动。
        let mut source = String::from("osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n");
        for index in 0..1_200 {
            source.push_str(&format!("256,192,{},1,0,0:0:0:0:\n", index * 50));
        }
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes())
            .expect("fixture 必须可解析");
        let sample_rate = 48_000_u32;
        // 60 秒输出、每 50ms 一个音符。
        let frames = sample_rate as usize * 60;

        let started = std::time::Instant::now();
        let mixed = render_hitsound_segment(
            &beatmap,
            Some(HitsoundSettings::new(true, 100, false)),
            0,
            frames,
            sample_rate,
            1.0,
            sample_rate,
        )
        .expect("混音不应失败")
        .expect("必须产生混音缓冲");
        let elapsed = started.elapsed();
        assert_eq!(mixed.len(), frames * 2);
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "60 秒 / 1200 个事件的打击音混音耗时 {elapsed:?}，混音窗口可能又退化成了整段"
        );
    }

    /// 打击音必须落在预期的谱面时间上。
    #[test]
    fn hitsound_events_land_on_expected_chart_times() {
        // 回归：混音窗口分组、时间轴位置、缓冲区下标三者一旦错位，导出的 MP4 就会
        // 要么没有打击音、要么错位；这里逐个检查每个打击音的能量峰出现在预期位置。
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n";
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes())
            .expect("fixture 必须可解析");
        let sample_rate = 48_000_u32;
        let frames = sample_rate as usize * 3;
        let mixed = render_hitsound_segment(
            &beatmap,
            Some(HitsoundSettings::new(true, 100, false)),
            0,
            frames,
            sample_rate,
            1.0,
            sample_rate,
        )
        .expect("混音不应失败")
        .expect("必须产生混音缓冲");
        assert_eq!(mixed.len(), frames * 2);

        let onsets = onsets_ms(&mixed, sample_rate);
        assert_eq!(onsets.len(), 2, "预期两个打击音，实际 {onsets:?}");
        for (onset, expected) in onsets.iter().zip([1000.0_f64, 2000.0]) {
            assert!(
                (onset - expected).abs() < 20.0,
                "打击音落在 {onset}ms，预期 {expected}ms"
            );
        }
    }

    /// 视频区间起点为负时打击音不得提前出声。
    #[test]
    fn hitsound_does_not_advance_before_negative_video_start() {
        // 回归：完整视频的起点是「首个物件前 2000ms」，首个物件很早时它就是负数。
        // 此前混音器把负位置夹到 0，缓冲区第 0 帧对应谱面 0，整段打击音提前了 |起点|。
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,3000,1,0,0:0:0:0:\n";
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes())
            .expect("fixture 必须可解析");
        let sample_rate = 48_000_u32;
        let frames = sample_rate as usize * 5;
        let mixed = render_hitsound_segment(
            &beatmap,
            Some(HitsoundSettings::new(true, 100, false)),
            -1_000,
            frames,
            sample_rate,
            1.0,
            sample_rate,
        )
        .expect("混音不应失败")
        .expect("必须产生混音缓冲");

        // 缓冲区时间 = 谱面时间 - chart_start：1000ms 的音符落在 2000ms 处。
        let onsets = onsets_ms(&mixed, sample_rate);
        assert_eq!(onsets.len(), 2, "预期两个打击音，实际 {onsets:?}");
        for (onset, expected) in onsets.iter().zip([2000.0_f64, 4000.0]) {
            assert!(
                (onset - expected).abs() < 20.0,
                "打击音落在 {onset}ms，预期 {expected}ms"
            );
        }
    }

    /// 倍速导出时打击音与谱面一起被压缩。
    #[test]
    fn hitsound_compresses_with_beatmap_speed() {
        // 回归：打击音此前按 1x 混好再 1:1 取样，倍速下会与音乐/画面按 speed 倍漂移。
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,3000,1,0,0:0:0:0:\n";
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes())
            .expect("fixture 必须可解析");
        let sample_rate = 48_000_u32;
        let cases = [(2.0, [500.0, 1500.0]), (0.5, [2000.0, 6000.0])];
        for (speed, expected) in cases {
            // 缓冲区取 7 秒输出时长：0.5x 时最后一个音符落在 6000ms 处。
            let frames = sample_rate as usize * 7;
            let mixed = render_hitsound_segment(
                &beatmap,
                Some(HitsoundSettings::new(true, 100, false)),
                0,
                frames,
                sample_rate,
                speed,
                sample_rate,
            )
            .expect("混音不应失败")
            .expect("必须产生混音缓冲");
            assert_eq!(mixed.len(), frames * 2);
            let onsets = onsets_ms(&mixed, sample_rate);
            // 变速后鼓声被拉长/压缩，样本自身的内部结构可能被算成第二个起音点，
            // 因此这里只要求每个预期时间附近都能检测到能量峰。
            for expected in expected {
                assert!(
                    onsets.iter().any(|onset| (onset - expected).abs() < 20.0),
                    "{speed}x：{expected}ms 处没有检测到打击音，实际 {onsets:?}"
                );
            }
        }
    }

    #[test]
    fn timeline_sampling_honours_lead_in_and_speed() {
        let decoded = DecodedAudio {
            sample_rate: 1_000,
            stereo_samples: (0..2_000_i16).flat_map(|v| [v, v]).collect(),
        };
        let mut output = [0_i16; 6];
        fill_audio_frame(&mut output, 0, 3, &decoded, -500, 2.0, 0, None);
        assert_eq!(output, [0, 0, 0, 0, 0, 0]);

        fill_audio_frame(&mut output, 0, 3, &decoded, 1_000, 2.0, 0, None);
        assert_eq!(output[0], 1_000);
        assert_eq!(source_frame_position(0, 0, 1_000, 1_000, 1.0), 1_000.0);
        assert_eq!(source_frame_position(48_000, 0, 1_000, 1_000, 1.5), 2_500.0);
        assert_eq!(
            source_frame_position(48_000, 0, 1_000, 1_000, 0.75),
            1_750.0
        );
        assert_eq!(source_frame_position(0, 0, 1_000, -500, 1.0), -500.0);
        assert_eq!(source_frame_position(0, 48_000, 1_000, 1_000, 1.0), 2_000.0);
    }

    #[test]
    fn negative_chart_start_is_silent_until_audio_time_zero() {
        let decoded = DecodedAudio {
            sample_rate: 1_000,
            stereo_samples: (0..100_i16)
                .flat_map(|value| {
                    let sample = 100 + value * 100;
                    [sample, sample]
                })
                .collect(),
        };
        let mut output = [0_i16; 8];

        fill_audio_frame(&mut output, 23_999, 24_003, &decoded, -500, 1.0, 0, None);

        assert_eq!(&output[..2], &[0, 0]);
        assert_eq!(&output[2..4], &[100, 100]);
        assert!(output[4] > 100);
    }
}

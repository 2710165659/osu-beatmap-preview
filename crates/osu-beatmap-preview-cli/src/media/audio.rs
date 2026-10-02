use crate::export::canvas::Img;
use crate::media::background_video::{BackgroundVideo, MediaBackground};
use fdk_aac::enc::{AudioObjectType, BitRate, ChannelMode, Encoder, EncoderParams, Transport};
use osu_beatmap_preview_core::hitsound::{
    self, HitsoundTimeline, MusicPlayer, MusicRate, StereoSource,
};
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

/// MP4 音频、背景与打击音所在的谱面包来源。
pub(crate) enum OszLocation {
    /// 按谱面集 ID 走下载缓存（未命中时从镜像下载）。
    Download { set_id: u64 },
    /// 用户提供的本地 `.osz`，直接读取，不再联网。
    Local(PathBuf),
}

impl OszLocation {
    /// 取得谱面包路径与音频缓存键。
    ///
    /// 下载来源的缓存键沿用谱面集 ID（与既有缓存目录兼容）；本地 `.osz` 没有稳定的
    /// 数字 ID，改用「路径 + 大小 + 修改时间」的哈希：同一路径的文件被替换后缓存键
    /// 随之变化，不会把旧音频误当成新谱面的。
    fn materialize(
        self,
        request_bid: &str,
        cache_dir: &Path,
        no_cache: bool,
        deadline: &RequestDeadline,
        download_video: bool,
    ) -> Result<(PathBuf, String)> {
        match self {
            Self::Download { set_id } => {
                let path = crate::download::download_beatmapset_archive(
                    request_bid,
                    set_id,
                    cache_dir,
                    no_cache,
                    deadline,
                    download_video,
                )?;
                Ok((path, set_id.to_string()))
            }
            Self::Local(path) => {
                let meta = std::fs::metadata(&path).map_err(|error| {
                    PreviewError::download(format!(
                        "failed to open local .osz {}: {error}",
                        path.display()
                    ))
                })?;
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0);
                let label = format!("{}|{}|{}", path.display(), meta.len(), mtime);
                Ok((path, format!("local-{:016x}", fnv1a64(label.as_bytes()))))
            }
        }
    }
}

pub(crate) struct AudioSourceJob {
    handle: Option<std::thread::JoinHandle<Result<AudioSource>>>,
    deadline: RequestDeadline,
    background: MediaBackground,
    storyboard: Option<super::storyboard::MediaStoryboard>,
}

impl AudioSourceJob {
    pub(crate) fn start(
        request_bid: &str,
        beatmap: Beatmap,
        osu_text: &str,
        osz: OszLocation,
        cache_dir: PathBuf,
        no_cache: bool,
        deadline: RequestDeadline,
        mode: crate::export::geometry::GameMode,
    ) -> Result<Self> {
        let request_bid = request_bid.to_string();
        let audio_filename = beatmap
            .audio_filename()
            .ok_or_else(|| PreviewError::parse("missing AudioFilename required for MP4 audio"))?;
        // 是否下载带视频的完整包由当前模式的 ENABLE_BACKGROUND_VIDEO 决定：
        // 背景视频开启才需要视频素材，否则用去视频包省流量。
        let style = super::video_style(mode);
        let (osz_path, cache_key) = osz.materialize(
            &request_bid,
            &cache_dir,
            no_cache,
            &deadline,
            style.enable_background_video,
        )?;
        crate::logging::event(
            "audio-prepare",
            "start",
            None,
            &format!("osz={cache_key} audio={audio_filename}"),
        );
        let media = BeatmapMedia::from_beatmap(&beatmap);
        let image = if style.enable_background_image {
            load_background_image(media.background.as_ref(), &osz_path, &deadline)?
        } else {
            None
        };
        let video = if style.enable_background_video {
            load_background_video(&beatmap, &osz_path, &deadline)?
        } else {
            None
        };
        // 故事板（默认关闭）：.osu 的 [Events] 与包内 .osb 一起解析，贴图同包取用。
        let storyboard = if style.enable_storyboard {
            super::storyboard::MediaStoryboard::load(osu_text, &osz_path, &deadline)?
        } else {
            None
        };
        // osu! 的 ReplacesBackground：背景层存在与谱面背景同名的精灵时隐藏背景图，
        // 由故事板里的同名精灵接管，避免两层同图叠画。
        let mut image = image;
        if storyboard.as_ref().is_some_and(|storyboard| {
            storyboard.replaces_background(beatmap.background_filename.as_deref())
        }) {
            image = None;
        }
        let worker_deadline = deadline.clone();
        let handle = std::thread::spawn(move || {
            worker_deadline.check()?;
            prepare_audio_source(
                &beatmap,
                &osz_path,
                &cache_key,
                &cache_dir,
                no_cache,
                &worker_deadline,
            )
        });
        Ok(Self {
            handle: Some(handle),
            deadline,
            background: MediaBackground { image, video },
            storyboard,
        })
    }

    pub(crate) fn take_background(&mut self) -> MediaBackground {
        MediaBackground {
            image: self.background.image.take(),
            video: self.background.video.take(),
        }
    }

    /// 取走已装载的故事板（`ENABLE_STORYBOARD` 关闭时恒为 `None`）。
    pub(crate) fn take_storyboard(&mut self) -> Option<super::storyboard::MediaStoryboard> {
        self.storyboard.take()
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

/// 背景视频条目上限：osu! 素材规范建议 ≤1280×720、体积多在几十 MB，
/// 超限直接放弃（回退背景图），避免把进程内存吃光。
const MAX_BACKGROUND_VIDEO_BYTES: u64 = 256 * 1024 * 1024;

/// 从谱面包中解出歌曲音频（带缓存），返回可供解码的本地文件。
///
/// `cache_key` 是音频缓存的子目录名：下载来源用谱面集 ID，本地 `.osz` 用文件指纹
/// （见 [`OszLocation::materialize`]）。
pub(crate) fn prepare_audio_source(
    beatmap: &Beatmap,
    osz_path: &Path,
    cache_key: &str,
    cache_dir: &Path,
    no_cache: bool,
    deadline: &RequestDeadline,
) -> Result<AudioSource> {
    deadline.check()?;
    let Some(audio) = BeatmapMedia::from_beatmap(beatmap).audio else {
        return Err(PreviewError::parse(
            "missing or unusable AudioFilename required for MP4 audio",
        ));
    };
    // 缓存文件名 = 条目名哈希 + 扩展名；扩展名由 core 的策略给出，缺失时沿用 `audio`。
    let extension = audio.extension.as_deref().unwrap_or("audio");
    let key = fnv1a64(audio.name.as_bytes());
    let entry_cache = cache_dir.join(cache_key);
    let target_path = entry_cache.join(format!("{key:016x}.{extension}"));

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

    std::fs::create_dir_all(&entry_cache)
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
    let wanted = entry.name.as_str();
    let Some(bytes) = read_osz_entry(
        osz_path,
        wanted,
        MAX_BACKGROUND_IMAGE_BYTES,
        deadline,
        ("background-prepare", "background image"),
    )?
    else {
        return Ok(None);
    };
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

/// 从 OSZ 中读取并解析谱面背景视频；缺失或不受支持时回退静态背景图。
///
/// 解码失败（非 H.264 mp4、容器损坏等）只记日志并返回 `None`：背景视频是
/// 纯装饰，不能让它挡住 MP4 导出（与 osu! 的降级行为一致）。
pub(crate) fn load_background_video(
    beatmap: &Beatmap,
    osz_path: &Path,
    deadline: &RequestDeadline,
) -> Result<Option<BackgroundVideo>> {
    let media = BeatmapMedia::from_beatmap(beatmap);
    let Some(entry) = media.video else {
        crate::logging::event(
            "video-prepare",
            "skip",
            None,
            "beatmap has no usable background video event",
        );
        return Ok(None);
    };
    let wanted = entry.name.as_str();
    let Some(bytes) = read_osz_entry(
        osz_path,
        wanted,
        MAX_BACKGROUND_VIDEO_BYTES,
        deadline,
        ("video-prepare", "background video"),
    )?
    else {
        return Ok(None);
    };
    // 起始偏移来自 `Video` 事件；解析器保证事件存在时字段齐全。
    let start_ms = beatmap.video.as_ref().map(|video| video.start_ms).unwrap_or(0);
    match BackgroundVideo::open(bytes, start_ms) {
        Ok(video) => {
            crate::logging::event(
                "video-prepare",
                "done",
                None,
                &format!(
                    "loaded {wanted} (duration={}ms, start={}ms)",
                    video.duration_ms, video.start_ms
                ),
            );
            Ok(Some(video))
        }
        Err(error) => {
            crate::logging::event(
                "video-prepare",
                "skip",
                None,
                &format!("failed to open background video: {error}"),
            );
            Ok(None)
        }
    }
}

/// 从 OSZ 中按名（不区分大小写）读取一个条目的字节。
///
/// 条目缺失、尺寸非法或过大都返回 `Ok(None)`（调用方各自降级）；只有压缩包
/// 本身不可读才是错误。`labels` 是（日志事件名，条目用途文案）。
fn read_osz_entry(
    osz_path: &Path,
    wanted: &str,
    max_bytes: u64,
    deadline: &RequestDeadline,
    labels: (&str, &str),
) -> Result<Option<Vec<u8>>> {
    let (event, label) = labels;
    let file = File::open(osz_path)
        .map_err(|e| PreviewError::download(format!("failed to open osz archive: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| PreviewError::download(format!("invalid osz archive: {e}")))?;
    let index = (0..archive.len()).find(|&index| {
        archive
            .by_index(index)
            .ok()
            .and_then(|entry| normalize_entry_path(entry.name()))
            .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
    });
    let Some(index) = index else {
        crate::logging::event(
            event,
            "skip",
            None,
            &format!("{label} file was not found in osz: {wanted}"),
        );
        return Ok(None);
    };
    let mut entry = archive
        .by_index(index)
        .map_err(|e| PreviewError::download(format!("failed to open {label} entry: {e}")))?;
    if entry.is_dir() || entry.size() == 0 || entry.size() > max_bytes {
        crate::logging::event(event, "skip", None, &format!("invalid {label} size"));
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PreviewError::download(format!("failed to extract {label}: {e}")))?;
    if bytes.len() as u64 > max_bytes {
        crate::logging::event(event, "skip", None, &format!("{label} is too large"));
        return Ok(None);
    }
    deadline.check()?;
    Ok(Some(bytes))
}

/// 把音乐与打击音混合成 AAC。
///
/// `music` 是本次 Mod 组合下音乐的重采样/时间伸缩倍率（DT/HT 保调、NC/DC 固定音高偏移）；
/// `speed` 同时决定谱面时间推进与打击音的重采样（游戏里 `ModRateAdjust.ApplyToSample`
/// 给样本加的是 `Frequency = SpeedChange`）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_audio_segment(
    source: &AudioSource,
    beatmap: &Beatmap,
    hitsound: Option<HitsoundSettings>,
    chart_start_ms: i64,
    frame_count: usize,
    fps: u32,
    speed: f64,
    music: MusicRate,
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
    // 音乐是有状态的时间伸缩流：按块顺序渲染，块起点带上编码器延迟补偿（与旧下标换算式
    // `output_index + encoder_delay_samples` 等价）。
    let mut music_player = MusicPlayer::new(sample_rate);
    music_player.set_rate(music);
    let mut music_block = vec![0.0_f32; samples_per_frame * 2];

    // FDK 在内部缓冲编码器前瞻数据，因此请求输入区间结束后可能还需执行几次补零调用，
    // 才能取出全部访问单元。
    let max_calls = target_frame_count + 16;
    for input_frame_index in 0..max_calls {
        deadline.check()?;
        let block_start = input_frame_index * samples_per_frame;
        music_player.render_at(
            block_content_ms(
                chart_start_ms,
                block_start,
                encoder_delay_samples,
                speed,
                sample_rate,
            ),
            &decoded,
            decoded.sample_rate,
            samples_per_frame,
            &mut music_block,
        );
        fill_audio_frame(
            &mut input,
            block_start,
            target_samples,
            &music_block,
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
    /// 是否启用 NC 的节拍鼓点（来自 Mod，与上面的开关无关）。
    pub nightcore: bool,
    /// 谱面自带打击音所在的 OSZ；`None` 表示只用内嵌皮肤。
    pub beatmap_samples: Option<PathBuf>,
}

impl HitsoundSettings {
    /// 从配置构造；`beatmap` 对应 `ENABLE_BEATMAP_HITSOUND`。此处的样本来源为空，
    /// 音频准备完成后由 [`HitsoundSettings::with_beatmap_samples`] 补上压缩包路径。
    /// NC 鼓点开关由调用方按当前 Mod 设置。
    pub(crate) fn new(enabled: bool, volume: i32, beatmap: bool) -> Self {
        Self {
            enabled,
            volume,
            beatmap,
            nightcore: false,
            beatmap_samples: None,
        }
    }

    /// 设置 NC 的节拍鼓点开关。
    pub(crate) fn with_nightcore(mut self, nightcore: bool) -> Self {
        self.nightcore = nightcore;
        self
    }

    /// 附带谱面自带音效所在的 OSZ；传 `None` 时只用内嵌皮肤。
    pub(crate) fn with_beatmap_samples(mut self, path: Option<&Path>) -> Self {
        self.beatmap_samples = path.map(Path::to_path_buf);
        self
    }
}

/// 按视频输出时间轴混出整段打击音。
///
/// 返回 `None` 表示本次导出既没有启用打击音、也没有需要发声的事件（配置关闭且没有 NC）。
///
/// 缓冲区下标即输出帧下标，与音乐（`MusicPlayer`）共用同一时间换算：
/// 第 i 帧对应的谱面时间是 `chart_start_ms + i * 1000 * speed / sample_rate`。
/// 倍速通过「混音器的内部采样率取 `sample_rate / speed`」实现，音高随之变化，
/// 与游戏里 `ModRateAdjust.ApplyToSample`（`Frequency = SpeedChange`）一致。
/// NC 的节拍鼓点属于 Mod：即使 `ENABLE_HITSOUND` 关闭，鼓点也会照常混进来。
fn render_hitsound_segment(
    beatmap: &Beatmap,
    settings: Option<HitsoundSettings>,
    chart_start_ms: i64,
    frame_count: usize,
    fps: u32,
    speed: f64,
    sample_rate: u32,
) -> Result<Option<Vec<f32>>> {
    let Some(settings) = settings.filter(|settings| settings.enabled || settings.nightcore) else {
        return Ok(None);
    };
    let library =
        super::hitsound::build_library(beatmap, settings.beatmap_samples.as_deref(), settings.nightcore);
    if library.is_empty() {
        return Ok(None);
    }
    let mut timeline = if settings.enabled {
        hitsound::build_timeline(beatmap, &library)
    } else {
        HitsoundTimeline::default()
    };
    if settings.nightcore {
        // 本次混音窗口的结尾（谱面绝对毫秒）：鼓点只需覆盖到这里。
        let end_ms = chart_start_ms as f64 + frame_count as f64 * 1000.0 * speed / fps as f64;
        timeline.merge(hitsound::nightcore_events(beatmap, &library, end_ms));
    }
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
    let mut mixer = hitsound::HitsoundMixer::new(library, timeline, mixer_rate);
    mixer.set_master_gain(hitsound::volume_gain(settings.volume.clamp(0, 100)));
    // 视频区间起点可能为负（首个物件前 2000ms 的预卷）：混音位置允许为负，
    // 缓冲区第 0 帧必须对应该起点，否则整段打击音会提前 |起点|。
    mixer.seek(chart_start_ms as f64);

    // 分块渲染：混音器每个窗口只保留本窗口内还在响的声音，一次渲染整段会把所有事件
    // 都压在 voices 里、逐输出帧遍历一遍（O(输出帧 × 事件数)），三分钟的谱面就能让
    // 音频线程比视频编码还慢。窗口取 1 秒。
    let mut buffer = vec![0.0_f32; frames * 2];
    let window_frames = sample_rate.max(1) as usize;
    for chunk in buffer.chunks_mut(window_frames * 2) {
        #[cfg(test)]
        tests::record_mix_window(chunk.len() / 2);
        mixer.render_into(chunk);
    }
    Ok(Some(buffer))
}

/// 把本编码块的音乐与打击音相加写入 `output`（交错 i16）。
///
/// `music` 是本块已经渲染好的音乐（交错 f32，长度 = 本块帧数 × 2），它已经包含编码器
/// 延迟补偿；打击音缓冲区仍按输出帧下标读取（`output_index + encoder_delay_samples`）。
#[allow(clippy::too_many_arguments)]
fn fill_audio_frame(
    output: &mut [i16],
    output_frame_start: usize,
    target_samples: u64,
    music: &[f32],
    encoder_delay_samples: usize,
    hitsound: Option<&[f32]>,
) {
    for (frame_offset, stereo) in output.chunks_exact_mut(2).enumerate() {
        let output_index = output_frame_start + frame_offset;
        if output_index as u64 >= target_samples {
            stereo.fill(0);
            continue;
        }
        let music_left = music.get(frame_offset * 2).copied().unwrap_or(0.0);
        let music_right = music.get(frame_offset * 2 + 1).copied().unwrap_or(0.0);
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
        stereo[0] = mix_sample(music_left, hit_left);
        stereo[1] = mix_sample(music_right, hit_right);
    }
}

/// 一个编码块起点的音乐内容毫秒。
///
/// `speed` 是本次 Mod 组合的总倍速：输出帧按实时帧推进，内容按 `1000 * speed / sample_rate`
/// 毫秒每帧推进（与画面、打击音共用同一条被压缩过的谱面时间轴）。
/// `encoder_delay_samples` 抵消 AAC 编码器的前瞻延迟，等价于旧实现里的
/// `output_index + encoder_delay_samples` 下标补偿。
fn block_content_ms(
    chart_start_ms: i64,
    block_start: usize,
    encoder_delay_samples: usize,
    speed: f64,
    sample_rate: u32,
) -> f64 {
    chart_start_ms as f64
        + (block_start + encoder_delay_samples) as f64 * 1000.0 * speed / sample_rate as f64
}

/// 把 f32 音乐与 f32 打击音相加并夹紧到 i16。
///
/// 音乐的 f32 值来自整数量化（`i16 / 32767.0`），乘回 `32767` 再 `round` 后与原值逐位
/// 相同，因此无 Mod（`hitsound = 0`）时输出与改动前的 i16 路径一致。
fn mix_sample(music: f32, hitsound: f32) -> i16 {
    let scaled = (music + hitsound) * i16::MAX as f32;
    scaled.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// 让 [`MusicPlayer`] 直接读取解码后的音乐：位置按源采样帧插值。
impl StereoSource for DecodedAudio {
    fn sample_at(&self, frame: f64) -> (f32, f32) {
        let [left, right] = sample_stereo(self, frame);
        (
            left as f32 / i16::MAX as f32,
            right as f32 / i16::MAX as f32,
        )
    }
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

        let library = crate::media::hitsound::build_library(&beatmap, None, false);
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
        let peak = mixed
            .iter()
            .fold(0.0_f32, |acc, value| acc.max(value.abs()));
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

    /// 长谱面必须使用有界混音窗口，避免退回整段渲染。
    #[test]
    fn long_hitsound_exports_keep_bounded_mix_windows() {
        // 回归：整段只用一个混音窗口时，所有事件都会压在 voices 里、逐输出帧遍历一遍
        // （O(输出帧 × 事件数)）。旧用例用 release 的耗时设置 10 秒上限，未优化的
        // macOS CI 会在正确分块时越线。直接观测真实混音调用，保留退化检查并去除机器依赖。
        let mut source = String::from("osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n");
        for index in 0..1_200 {
            source.push_str(&format!("256,192,{},1,0,0:0:0:0:\n", index * 50));
        }
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes())
            .expect("fixture 必须可解析");
        // 较低采样率减少测试计算量；仍保留 1200 个事件及 60 秒以上的混音跨度。
        let sample_rate = 4_000_u32;
        // 加上不满一秒的尾块，检查输出不会被截断。
        let frames = sample_rate as usize * 60 + sample_rate as usize / 4;
        MIX_WINDOWS.with(|windows| windows.set(MixWindows::default()));

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
        let windows = MIX_WINDOWS.with(std::cell::Cell::get);
        assert_eq!(mixed.len(), frames * 2);
        assert!(
            windows.max_frames <= sample_rate as usize,
            "混音窗口必须至多一秒，实际 {windows:?}"
        );
        assert_eq!(windows.frames, frames, "所有采样帧和尾块都必须送入混音器");
        assert!(windows.calls > 1, "长谱面不能一次渲染整段");
        for (index, chunk) in mixed.chunks_exact(sample_rate as usize * 2).enumerate() {
            assert!(
                chunk.iter().any(|sample| sample.abs() > 0.01),
                "第 {index} 秒缺少打击音"
            );
        }
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

    /// 音乐按「内容毫秒 × 源采样率 / 1000」取样；倍速决定内容推进速率（保调）。
    #[test]
    fn music_sampling_honours_lead_in_rate_and_pitch() {
        // 1kHz 源、1kHz 输出：1 帧 = 1ms。线性斜坡便于直接断言内容位置（线性函数插值无损）。
        let decoded = DecodedAudio {
            sample_rate: 1_000,
            stereo_samples: (0..2_000_i16)
                .flat_map(|index| {
                    let value = index * 2;
                    [value, value]
                })
                .collect(),
        };
        let value_of = |content: f64| content * 2.0;

        // 恒等倍率：内容位置 = 起点 + 帧号。
        let mut player = MusicPlayer::new(1_000);
        let mut output = vec![0.0_f32; 4 * 2];
        player.render_at(1_000.0, &decoded, 1_000, 4, &mut output);
        for (frame, pair) in output.chunks_exact(2).enumerate() {
            let expected = value_of(1_000.0 + frame as f64);
            assert!(
                (pair[0] as f64 * i16::MAX as f64 - expected).abs() < 12.0,
                "帧 {frame}: {} 对不上内容位置 {expected}",
                pair[0]
            );
        }

        // DT（输出域 1.5x 保调）：内容平均按 1.5 帧/输出帧推进。
        //
        // 时间伸缩的内容位置是「每跳前进 hop·stretch、段内 1:1」的锯齿，且相似度搜索允许
        // 几毫秒的局部偏移，因此这里只看**平均速率**（长窗口 + 宽松容差）：倍率写错时偏差
        // 会随帧数线性增长，很快越过容差。
        let mut dt = MusicPlayer::new(1_000);
        dt.set_rate(MusicRate::output_domain(1.5, 1.0));
        let stretched_frames = 400;
        let mut stretched = vec![0.0_f32; stretched_frames * 2];
        dt.render_at(1_000.0, &decoded, 1_000, stretched_frames, &mut stretched);
        for frame in [8_usize, 100, 200, 399] {
            let expected = value_of(1_000.0 + frame as f64 * 1.5);
            let value = stretched[frame * 2] as f64 * i16::MAX as f64;
            assert!(
                (value - expected).abs() < 40.0,
                "帧 {frame}: {value} 对不上平均内容位置 {expected}"
            );
        }

        // HT（输出域 0.75x 保调）：内容平均按 0.75 帧/输出帧推进。
        let mut ht = MusicPlayer::new(1_000);
        ht.set_rate(MusicRate::output_domain(0.75, 1.0));
        let mut slowed = vec![0.0_f32; stretched_frames * 2];
        ht.render_at(1_000.0, &decoded, 1_000, stretched_frames, &mut slowed);
        for frame in [8_usize, 100, 200, 399] {
            let expected = value_of(1_000.0 + frame as f64 * 0.75);
            let value = slowed[frame * 2] as f64 * i16::MAX as f64;
            assert!(
                (value - expected).abs() < 40.0,
                "帧 {frame}: {value} 对不上平均内容位置 {expected}"
            );
        }

        // 负起点（首个物件前的预卷）：内容小于 0 时整段静音。
        let mut pre_roll = MusicPlayer::new(1_000);
        let mut silent = vec![1.0_f32; 3 * 2];
        pre_roll.render_at(-500.0, &decoded, 1_000, 3, &mut silent);
        assert!(silent.iter().all(|value| *value == 0.0), "{silent:?}");
    }

    /// 导出链路按编码块推进音乐：两个静音标记之间的距离必须等于「标记间隔 / 内容速率」。
    ///
    /// 1kHz 源、1kHz 输出：1 帧 = 1ms；源在 1000ms 与 3000ms 处各有一段 100ms 静音。
    /// 时间伸缩的内容位置是锯齿（段内 1:1、每次跳进 `hop·(stretch-1)`），单个标记的绝对位置
    /// 会因此有几毫秒到十几毫秒的偏差，但两个标记的**间距**不受锯齿影响，正好反映平均速率。
    #[test]
    fn export_music_blocks_follow_the_rate() {
        let frames = 6_000;
        let gaps = [(1_000_usize, 1_100_usize), (3_000, 3_100)];
        let decoded = DecodedAudio {
            sample_rate: 1_000,
            stereo_samples: (0..frames)
                .flat_map(|index| {
                    let silent = gaps.iter().any(|(start, end)| (*start..*end).contains(&index));
                    let value = if silent {
                        0.0
                    } else {
                        (index as f64 * 0.2).sin()
                    };
                    let sample = (value * i16::MAX as f64) as i16;
                    [sample, sample]
                })
                .collect(),
        };

        // (speed, pitch, 内容速率)：DT 保调、HT 保调、NC/DC 的固定音高、以及非默认倍速。
        let cases = [
            (1.5_f64, 1.0_f64, 1.5_f64),
            (0.75, 1.0, 0.75),
            (1.5, 1.5, 1.5),
            (0.75, 0.75, 0.75),
            (2.0, 1.5, 2.0),
        ];
        let block = 256_usize;
        for (speed, pitch, content_rate) in cases {
            let mut player = MusicPlayer::new(1_000);
            player.set_rate(MusicRate::output_domain(speed, pitch));
            let mut output = vec![0.0_f32; (frames as f64 / content_rate) as usize * 2 + block * 4];
            let mut block_buffer = vec![0.0_f32; block * 2];
            let blocks = output.len() / 2 / block;
            for index in 0..blocks {
                let start = index * block;
                player.render_at(
                    block_content_ms(0, start, 0, speed, 1_000),
                    &decoded,
                    1_000,
                    block,
                    &mut block_buffer,
                );
                output[start * 2..(start + block) * 2].copy_from_slice(&block_buffer);
            }

            let silent: Vec<usize> = output
                .chunks_exact(2)
                .enumerate()
                .filter(|(_, pair)| pair[0].abs() < 1e-6)
                .map(|(index, _)| index)
                .collect();
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for index in silent {
                match runs.last_mut() {
                    Some(last) if last.1 + 1 == index => last.1 = index,
                    _ => runs.push((index, index)),
                }
            }
            // 正弦过零点只有 1 帧，源播完后的静音在最后：只取前两个成型（≥ 20 帧）的静音段。
            let markers: Vec<usize> = runs
                .iter()
                .filter(|(start, end)| end - start >= 20)
                .map(|(start, _)| *start)
                .take(2)
                .collect();
            assert_eq!(
                markers.len(),
                2,
                "speed={speed}, pitch={pitch}：没有找到两个静音标记（{runs:?}）"
            );
            let expected = (2_000.0 / content_rate) as isize;
            let measured = markers[1] as isize - markers[0] as isize;
            assert!(
                (measured - expected).abs() <= 20,
                "speed={speed}, pitch={pitch}：标记 {markers:?} 间距 {measured}，预期 {expected}"
            );
            // 单个标记的绝对位置只允许锯齿范围内的偏差。
            let expected_first = 1_000.0 / content_rate;
            assert!(
                (markers[0] as f64 - expected_first).abs() <= 25.0,
                "speed={speed}, pitch={pitch}：首个标记 {}，预期 {expected_first}",
                markers[0]
            );
        }
    }

    /// `fill_audio_frame` 按输出帧下标取打击音（含编码器延迟），超出目标长度的帧清零。
    #[test]
    fn fill_audio_frame_applies_encoder_delay_and_zero_fills_the_tail() {
        // 音乐与打击音都是交错立体声：打击音第 1 帧（下标 2、3）为 0.5。
        let music = [0.25_f32; 10];
        let hitsound = [0.0_f32, 0.0, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let music_only = (0.25 * i16::MAX as f32).round() as i16;
        let with_hitsound = ((0.25 + 0.5) * i16::MAX as f32).round() as i16;

        // 无延迟：输出帧 1 取到打击音第 1 帧；帧 4 超出 target_samples 被清零。
        let mut plain = [0_i16; 10];
        fill_audio_frame(&mut plain, 0, 4, &music, 0, Some(&hitsound));
        assert_eq!(plain[0], music_only);
        assert_eq!(plain[2], with_hitsound);
        assert_eq!(plain[4], music_only);
        assert_eq!(&plain[8..10], &[0, 0]);

        // 延迟 1 帧：打击音整体提前一格（补偿编码器前瞻）。
        let mut delayed = [0_i16; 10];
        fill_audio_frame(&mut delayed, 0, 4, &music, 1, Some(&hitsound));
        assert_eq!(delayed[0], with_hitsound);
        assert_eq!(delayed[2], music_only);

        // 从输出帧 1 开始：下标继续按绝对帧号走，帧 1 取到打击音第 1 帧。
        let mut offset = [0_i16; 4];
        fill_audio_frame(&mut offset, 1, 3, &music, 0, Some(&hitsound));
        assert_eq!(offset[0], with_hitsound);
        assert_eq!(offset[2], music_only);
    }

    /// 只在测试构建中观测实际送进混音器的窗口；各测试线程独立计数。
    #[derive(Clone, Copy, Debug, Default)]
    struct MixWindows {
        calls: usize,
        frames: usize,
        max_frames: usize,
    }

    std::thread_local! {
        static MIX_WINDOWS: std::cell::Cell<MixWindows> = const {
            std::cell::Cell::new(MixWindows { calls: 0, frames: 0, max_frames: 0 })
        };
    }

    pub(super) fn record_mix_window(frames: usize) {
        MIX_WINDOWS.with(|windows| {
            let previous = windows.get();
            windows.set(MixWindows {
                calls: previous.calls + 1,
                frames: previous.frames + frames,
                max_frames: previous.max_frames.max(frames),
            });
        });
    }
}

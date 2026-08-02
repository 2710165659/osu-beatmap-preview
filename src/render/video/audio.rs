use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use fdk_aac::enc::{AudioObjectType, BitRate, ChannelMode, Encoder, EncoderParams, Transport};
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

pub(crate) const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub(crate) const AUDIO_BITRATE: u32 = 96_000;
const MAX_EXTRACTED_AUDIO_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct AudioSource {
    pub path: PathBuf,
    pub lead_in_ms: i64,
}

pub(crate) struct AudioSourceJob {
    handle: std::thread::JoinHandle<Result<AudioSource>>,
}

impl AudioSourceJob {
    pub(crate) fn start(
        request_bid: &str,
        beatmap: Beatmap,
        cache_dir: PathBuf,
        no_cache: bool,
    ) -> Result<Self> {
        let set_id = beatmap.beatmap_set_id().ok_or_else(|| {
            PreviewError::parse("missing or invalid BeatmapSetID required for MP4 audio")
        })?;
        let request_bid = request_bid.to_string();
        let audio_filename = beatmap
            .audio_filename()
            .ok_or_else(|| PreviewError::parse("missing AudioFilename required for MP4 audio"))?;
        crate::log::event(
            "audio-prepare",
            "start",
            None,
            &format!("set_id={set_id} audio={audio_filename}"),
        );

        let handle = std::thread::spawn(move || {
            let osz_path = crate::pipeline::downloader::download_beatmapset_archive(
                &request_bid,
                set_id,
                &cache_dir,
                no_cache,
            )?;
            prepare_audio_source(&beatmap, &osz_path, &cache_dir, no_cache)
        });
        Ok(Self { handle })
    }

    pub(crate) fn wait(self) -> Result<AudioSource> {
        self.handle
            .join()
            .map_err(|_| PreviewError::render("audio preparation worker panicked"))?
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

pub(crate) fn prepare_audio_source(
    beatmap: &Beatmap,
    osz_path: &Path,
    cache_dir: &Path,
    no_cache: bool,
) -> Result<AudioSource> {
    let set_id = beatmap.beatmap_set_id().ok_or_else(|| {
        PreviewError::parse("missing or invalid BeatmapSetID required for MP4 audio")
    })?;
    let audio_filename = beatmap
        .audio_filename()
        .ok_or_else(|| PreviewError::parse("missing AudioFilename required for MP4 audio"))?;
    let normalized = normalize_archive_path(audio_filename).map_err(PreviewError::parse)?;
    let extension = Path::new(&normalized)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("audio")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    let key = fnv1a64(normalized.as_bytes());
    let set_cache = cache_dir.join(set_id.to_string());
    let target_path = set_cache.join(format!("{key:016x}.{extension}"));

    if !no_cache
        && target_path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() > 0)
    {
        crate::log::event(
            "audio-prepare",
            "done",
            None,
            &format!("audio cache hit: {}", target_path.display()),
        );
        crate::log::record_cache(crate::log::CacheKind::Audio, "hit");
        return Ok(AudioSource {
            path: target_path,
            lead_in_ms: beatmap.audio_lead_in_ms(),
        });
    }

    std::fs::create_dir_all(&set_cache)
        .map_err(|e| PreviewError::download(format!("failed to create audio cache dir: {e}")))?;
    extract_audio_entry(osz_path, &normalized, &target_path)?;
    crate::log::event(
        "audio-prepare",
        "done",
        None,
        &format!("extracted audio: {}", target_path.display()),
    );
    crate::log::record_cache(crate::log::CacheKind::Audio, "downloaded");
    Ok(AudioSource {
        path: target_path,
        lead_in_ms: beatmap.audio_lead_in_ms(),
    })
}

pub(crate) fn encode_audio_segment(
    source: &AudioSource,
    chart_start_ms: i64,
    frame_count: usize,
    fps: u32,
    speed: f64,
) -> Result<EncodedAudio> {
    if fps == 0 || !speed.is_finite() || speed <= 0.0 {
        return Err(PreviewError::render(
            "invalid video timing for audio encoding",
        ));
    }
    let decoded = decode_audio(&source.path)?;
    let target_samples =
        ((frame_count as u64 * AUDIO_SAMPLE_RATE as u64) + fps as u64 - 1) / fps as u64;
    if target_samples == 0 {
        return Err(PreviewError::render("audio segment is empty"));
    }

    let encoder = Encoder::new(EncoderParams {
        bit_rate: BitRate::Cbr(AUDIO_BITRATE),
        sample_rate: AUDIO_SAMPLE_RATE,
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
    let encoder_delay_samples = info.nDelay.max(0) as usize;
    let mut input = vec![0_i16; samples_per_frame * 2];
    let mut output = vec![0_u8; max_output_bytes];
    let mut frames = Vec::with_capacity(target_frame_count);
    let mut input_frame_index = 0usize;

    // FDK buffers encoder look-ahead internally, so a few zero-padded calls may
    // be needed after the requested input span before every access unit appears.
    let max_calls = target_frame_count + 16;
    for _ in 0..max_calls {
        fill_audio_frame(
            &mut input,
            input_frame_index * samples_per_frame,
            target_samples,
            &decoded,
            chart_start_ms,
            speed,
            encoder_delay_samples,
        );
        input_frame_index += 1;
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

fn fill_audio_frame(
    output: &mut [i16],
    output_frame_start: usize,
    target_samples: u64,
    decoded: &DecodedAudio,
    chart_start_ms: i64,
    speed: f64,
    encoder_delay_samples: usize,
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
        stereo[0] = left;
        stereo[1] = right;
    }
}

fn source_frame_position(
    output_index: usize,
    encoder_delay_samples: usize,
    source_sample_rate: u32,
    chart_start_ms: i64,
    speed: f64,
) -> f64 {
    let chart_time_ms = chart_start_ms as f64
        + (output_index + encoder_delay_samples) as f64 * 1000.0 * speed / AUDIO_SAMPLE_RATE as f64;
    chart_time_ms * source_sample_rate as f64 / 1000.0
}

/// osu! uses AudioLeadIn to choose how early gameplay starts; it does not
/// offset the audio file relative to beatmap time zero.
pub(crate) fn full_video_start_time(first_object_ms: i64, audio_lead_in_ms: i64) -> i64 {
    let default_start = first_object_ms - 2_000;
    if audio_lead_in_ms > 0 {
        default_start.min(first_object_ms - audio_lead_in_ms)
    } else {
        default_start
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

fn decode_audio(path: &Path) -> Result<DecodedAudio> {
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
        sample_rate: sample_rate.unwrap_or(AUDIO_SAMPLE_RATE),
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

fn extract_audio_entry(osz_path: &Path, wanted: &str, target_path: &Path) -> Result<()> {
    let file = File::open(osz_path)
        .map_err(|e| PreviewError::download(format!("failed to open osz archive: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| PreviewError::download(format!("invalid osz archive: {e}")))?;
    let index = find_audio_entry(&mut archive, wanted)?;
    let mut entry = archive
        .by_index(index)
        .map_err(|e| PreviewError::download(format!("failed to open audio entry: {e}")))?;
    if entry.is_dir() || entry.size() == 0 || entry.size() > MAX_EXTRACTED_AUDIO_BYTES {
        return Err(PreviewError::download(format!(
            "invalid extracted audio size: {} bytes",
            entry.size()
        )));
    }

    let part_path = target_path.with_extension("part");
    let mut output = File::create(&part_path)
        .map_err(|e| PreviewError::download(format!("failed to create audio cache file: {e}")))?;
    let copied = std::io::copy(
        &mut entry.by_ref().take(MAX_EXTRACTED_AUDIO_BYTES + 1),
        &mut output,
    )
    .map_err(|e| PreviewError::download(format!("failed to extract beatmap audio: {e}")))?;
    output
        .flush()
        .map_err(|e| PreviewError::download(format!("failed to flush audio cache: {e}")))?;
    if copied == 0 || copied > MAX_EXTRACTED_AUDIO_BYTES {
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
        if let Ok(name) = normalize_archive_path(entry.name()) {
            if name.eq_ignore_ascii_case(wanted) {
                return Ok(index);
            }
        }
    }
    Err(PreviewError::download(format!(
        "AudioFilename '{wanted}' was not found in the osz archive"
    )))
}

fn normalize_archive_path(path: &str) -> std::result::Result<String, String> {
    let replaced = path.trim().replace('\\', "/");
    if replaced.starts_with('/') {
        return Err("AudioFilename must be relative".to_string());
    }
    let mut segments = Vec::new();
    for segment in replaced.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return Err("AudioFilename contains a parent-directory component".to_string()),
            value if value.contains(':') => {
                return Err("AudioFilename contains an absolute path prefix".to_string())
            }
            value => segments.push(value),
        }
    }
    if segments.is_empty() {
        return Err("AudioFilename is empty".to_string());
    }
    Ok(segments.join("/"))
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
    fn archive_paths_are_normalized_and_traversal_is_rejected() {
        assert_eq!(
            normalize_archive_path(r"audio\\song.mp3").unwrap(),
            "audio/song.mp3"
        );
        assert!(normalize_archive_path("../song.mp3").is_err());
        assert!(normalize_archive_path("C:/song.mp3").is_err());
        assert!(normalize_archive_path("/song.mp3").is_err());
    }

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
        extract_audio_entry(&osz, "audio/song.mp3", &output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"requested-audio");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn timeline_sampling_honours_lead_in_and_speed() {
        let decoded = DecodedAudio {
            sample_rate: 1_000,
            stereo_samples: (0..2_000_i16).flat_map(|v| [v, v]).collect(),
        };
        let mut output = [0_i16; 6];
        fill_audio_frame(&mut output, 0, 3, &decoded, -500, 2.0, 0);
        assert_eq!(output, [0, 0, 0, 0, 0, 0]);

        fill_audio_frame(&mut output, 0, 3, &decoded, 1_000, 2.0, 0);
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

        fill_audio_frame(&mut output, 23_999, 24_003, &decoded, -500, 1.0, 0);

        assert_eq!(&output[..2], &[0, 0]);
        assert_eq!(&output[2..4], &[100, 100]);
        assert!(output[4] > 100);
    }

    #[test]
    fn audio_lead_in_extends_full_video_start_without_shifting_audio() {
        assert_eq!(full_video_start_time(5_000, 0), 3_000);
        assert_eq!(full_video_start_time(5_000, 1_000), 3_000);
        assert_eq!(full_video_start_time(5_000, 4_000), 1_000);
        assert_eq!(full_video_start_time(5_000, -500), 3_000);
    }
}

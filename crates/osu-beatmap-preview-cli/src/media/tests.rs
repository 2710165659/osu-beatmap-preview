#![cfg(test)]
use super::*;
use osu_beatmap_preview_core::model::{HitObjects, KvSection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn beatmap_with_preview(preview_time: Option<&str>, lead_in: Option<&str>) -> Beatmap {
    let mut general = KvSection::default();
    if let Some(value) = preview_time {
        general.insert("PreviewTime", value.to_string());
    }
    if let Some(value) = lead_in {
        general.insert("AudioLeadIn", value.to_string());
    }
    Beatmap {
        metadata: KvSection::default(),
        difficulty: KvSection::default(),
        general,
        timing_points: Vec::new(),
        hit_objects: HitObjects::Standard(Vec::new()),
        break_periods: Vec::new(),
        background_filename: None,
        combo_colors: Vec::new(),
        beat_divisor: 0,
    }
}

#[test]
fn preview_start_uses_preview_time_and_duration() {
    let beatmap = beatmap_with_preview(Some("45000"), None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Preview),
        Some(30.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 45_000,
            end: 75_000
        }
    );
}

#[test]
fn default_start_uses_requested_duration_on_a_long_chart() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(60.0), 1.0).unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 10_000,
            end: 70_000
        }
    );
}

#[test]
fn numeric_start_shifts_backward_when_it_runs_past_chart_tail() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Seconds(50.0)),
        Some(60.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 42_000,
            end: 102_000
        }
    );
}

#[test]
fn short_chart_returns_full_playable_range_instead_of_padding_to_duration() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(&beatmap, 10_000, 20_000, None, Some(60.0), 1.0).unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 8_000,
            end: 22_000
        }
    );
}

#[test]
fn short_chart_uses_full_range_even_with_a_negative_requested_start() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        20_000,
        Some(TimePoint::Seconds(-20.0)),
        Some(60.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 8_000,
            end: 22_000
        }
    );
}

#[test]
fn negative_start_is_preserved_when_tail_adjustment_is_not_needed() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Seconds(-20.0)),
        Some(60.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: -10_000,
            end: 50_000
        }
    );
}

#[test]
fn speed_multiplier_scales_chart_span() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(30.0), 1.5).unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 10_000,
            end: 55_000
        }
    );
}

#[test]
fn numeric_start_is_relative_to_first_object() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Seconds(-2.0)),
        Some(10.0),
        1.5,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 8_000,
            end: 23_000
        }
    );
}

#[test]
fn full_range_start_follows_audio_lead_in_like_the_preview() {
    use osu_beatmap_preview_core::preview_start_ms;

    // 完整区间起点与实时预览（core 会话的 absolute_start_ms）共用同一个规则：
    // 首个物件前 2000ms，谱面 AudioLeadIn 更大时按它提前。
    // AudioLeadIn 更大：起点提前到 10000 - 4000。
    let beatmap = beatmap_with_preview(None, Some("4000"));
    let range =
        resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(600.0), 1.0).unwrap();
    assert_eq!(range.start, 6_000);
    assert_eq!(range.start, preview_start_ms(10_000, 4_000));

    // AudioLeadIn 更小：仍用默认的 2000ms。
    let beatmap = beatmap_with_preview(None, Some("1000"));
    let range =
        resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(600.0), 1.0).unwrap();
    assert_eq!(range.start, 8_000);
    assert_eq!(range.start, preview_start_ms(10_000, 1_000));
}

#[test]
fn progress_label_uses_current_skin_time_and_full_playable_duration() {
    let time_axis = TimeAxis::new(12_500);
    let total_ms = time_axis.to_display(102_500);

    assert_eq!(total_ms, 90_000);
    assert_eq!(
        format_progress_label(time_axis.to_display(12_000), total_ms),
        "-0:01/1:30"
    );
    assert_eq!(
        format_progress_label(time_axis.to_display(92_500), total_ms),
        "1:20/1:30"
    );
}

#[test]
fn video_background_fits_entire_image_and_keeps_black_borders() {
    let mut source = Img::new(4, 2, [255, 100, 0, 255]);
    for y in 0..2 {
        for x in 2..4 {
            source.put(x, y, [0, 100, 255, 255]);
        }
    }
    let background = prepare_video_background(
        &source,
        4,
        4,
        video_style(crate::export::geometry::GameMode::Standard),
    );
    assert_eq!((background.w, background.h), (4, 4));
    assert_eq!(background.get(0, 0), [0, 0, 0, 255]);
    assert_eq!(background.get(0, 1), [77, 30, 0, 255]);
    assert_eq!(background.get(0, 2), [77, 30, 0, 255]);
    assert_eq!(background.get(3, 1), [0, 30, 77, 255]);
    assert_eq!(background.get(0, 3), [0, 0, 0, 255]);
}

#[test]
fn dropping_audio_task_cancels_and_joins_worker() {
    let deadline = RequestDeadline::new(Instant::now(), "mp4", Duration::from_secs(300));
    let worker_deadline = deadline.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = finished.clone();
    let task = JoinedAudioTask::new(
        std::thread::spawn(move || loop {
            if let Err(error) = worker_deadline.check() {
                worker_finished.store(true, Ordering::Relaxed);
                return Err(error);
            }
            std::thread::yield_now();
        }),
        deadline,
    );

    drop(task);
    assert!(finished.load(Ordering::Relaxed));
}

#[test]
fn canvas_sized_layer_composites_at_origin_and_keeps_label_in_corner() {
    let style = video_style(crate::export::geometry::GameMode::Standard);
    let (width, height) = (684u32, 384u32);
    let mut layer = Img::new(width, height, [0, 0, 0, 0]);
    // 物件层左上角画一像素：如果合成时又居中一次，它会跑到 (77, 0)。
    layer.put(0, 0, [10, 20, 30, 255]);

    let composed = compose_frame(layer, 0, 60_000, width, height, None, style);

    assert_eq!(composed.get(0, 0), [10, 20, 30, 255]);
    let label = format_progress_label(0, 60_000);
    let (label_w, _) = text_size(&label, style.label_font_size);
    let label_left = width as i64 - label_w as i64 - style.label_pad;
    let label_painted = (label_left..width as i64).any(|x| {
        (style.label_pad..style.label_pad + style.label_font_size as i64).any(|y| {
            composed.get(x as u32, y as u32)[3] > 0 && composed.get(x as u32, y as u32)[0] > 100
        })
    });
    assert!(label_painted, "时间标签仍应绘制在画布右上角");
}

/// 编码线程按帧号顺序编码全部在途帧，并正确交还编码器与 MP4 writer。
///
/// 流水线把「编码」放到独立线程后，帧号即 MP4 的 sample `start_time`。
/// 第 0 帧由调用方单独写入，因此 worker 必须从 1 开始编号；
/// 这里用真实 CPU 编码器（无需 GPU）验证帧数、编号与文件是否写出。
#[test]
fn encode_stream_worker_consumes_frames_in_order_and_returns_encoder() {
    use crate::media::cpu::CpuEncoder;
    use crate::media::video::encode_stream_worker;

    let (width, height) = (64u32, 64u32);
    let mut encoder: Box<dyn FrameEncoder> = Box::new(CpuEncoder::new(width, height, 15).unwrap());

    let directory = std::env::temp_dir().join(format!(
        "osu-preview-worker-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("worker.mp4");
    let mut writer = mp4::Mp4Writer::write_start(
        BufWriter::new(std::fs::File::create(&path).unwrap()),
        &mp4::Mp4Config {
            major_brand: mp4::FourCC::from(*b"isom"),
            minor_version: 512,
            compatible_brands: vec![mp4::FourCC::from(*b"isom")],
            timescale: 15,
        },
    )
    .unwrap();

    // 第 0 帧由调用方直接编码并写入，worker 只处理其余三帧。
    let first = encoder
        .encode(&Img::new(width, height, [0, 0, 0, 255]))
        .unwrap();
    let first_sps = first.sps.clone().unwrap();
    let first_pps = first.pps.clone().unwrap();
    writer
        .add_track(&mp4::TrackConfig {
            track_type: mp4::TrackType::Video,
            timescale: 15,
            language: "und".to_string(),
            media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
                width: width as u16,
                height: height as u16,
                seq_param_set: first_sps,
                pic_param_set: first_pps,
            }),
        })
        .unwrap();
    writer
        .write_sample(
            1,
            &mp4::Mp4Sample {
                start_time: 0,
                duration: 1,
                rendering_offset: 0,
                is_sync: true,
                bytes: Bytes::copy_from_slice(&first.slice),
            },
        )
        .unwrap();

    let (sender, receiver) = std::sync::mpsc::sync_channel::<Img>(4);
    let stages = Arc::new(crate::media::video::PipelineStages::default());
    let worker = std::thread::spawn(move || -> Result<EncoderOutcome> {
        let mut encoder = encoder;
        let mut writer = writer;
        encode_stream_worker(
            &mut *encoder,
            &mut writer,
            receiver,
            "test",
            width,
            height,
            stages,
        )?;
        Ok(EncoderOutcome { encoder, writer })
    });

    for offset in 1..=3u8 {
        sender
            .send(Img::new(
                width,
                height,
                [offset * 40, offset * 20, 255, 255],
            ))
            .unwrap();
    }
    drop(sender);

    let outcome = worker.join().unwrap().unwrap();
    assert_eq!(outcome.encoder.name(), "openh264");
    // worker 不负责收尾，这里补上 write_end 并落盘，再确认样本数据确实写进了文件。
    let written = outcome.finish().unwrap();
    assert!(
        written > 40,
        "收尾后文件应包含 MP4 头与样本数据，实际 {written} 字节"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

//! egui 调试播放器；仅提供播放/暂停、seek 和运行时倍速。

use std::fs::File;
use std::time::{Duration, Instant};

use eframe::egui;

use super::{OffscreenRenderer, RealtimeError, RealtimeRequest, RealtimeSession};

pub fn run(mut request: RealtimeRequest) -> Result<(), RealtimeError> {
    request.media_policy = super::MediaPolicy::AudioAndBackground;
    let session = RealtimeSession::load(request)?;
    let renderer = pollster::block_on(OffscreenRenderer::new(session.offscreen_config()))?;
    let audio = session.audio_path().and_then(AudioController::open);
    let app = PlayerApp::new(session, renderer, audio);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("osu! beatmap preview")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "osu! beatmap preview",
        options,
        Box::new(move |_context| Ok(Box::new(app))),
    )
    .map_err(|error| RealtimeError::Device(format!("player window failed: {error}")))
}

struct PlayerApp {
    session: RealtimeSession,
    renderer: OffscreenRenderer,
    audio: Option<AudioController>,
    texture: Option<egui::TextureHandle>,
    current_ms: i64,
    rendered_ms: Option<i64>,
    playing: bool,
    resume_after_drag: bool,
    runtime_speed: f32,
    last_tick: Instant,
    error: Option<String>,
}

impl PlayerApp {
    fn new(
        session: RealtimeSession,
        renderer: OffscreenRenderer,
        audio: Option<AudioController>,
    ) -> Self {
        Self {
            current_ms: session.timeline().absolute_start_ms,
            session,
            renderer,
            audio,
            texture: None,
            rendered_ms: None,
            playing: false,
            resume_after_drag: false,
            runtime_speed: 1.0,
            last_tick: Instant::now(),
            error: None,
        }
    }

    fn combined_speed(&self) -> f32 {
        self.runtime_speed * self.session.timeline().beatmap_speed as f32
    }

    fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        self.last_tick = Instant::now();
        let combined_speed = self.combined_speed();
        if let Some(audio) = &mut self.audio {
            audio.set_speed(combined_speed);
            if playing && self.current_ms >= 0 {
                audio.seek(self.current_ms);
                audio.play();
            } else {
                audio.pause();
            }
        }
    }

    fn seek(&mut self, target_ms: i64) {
        self.current_ms = target_ms;
        self.rendered_ms = None;
        let combined_speed = self.combined_speed();
        if let Some(audio) = &mut self.audio {
            audio.pause();
            audio.set_speed(combined_speed);
            audio.seek(target_ms);
            if self.playing && target_ms >= 0 {
                audio.play();
            }
        }
        self.last_tick = Instant::now();
    }

    fn advance_clock(&mut self) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        if !self.playing {
            return;
        }
        if self.current_ms >= 0 {
            if let Some(audio) = &self.audio {
                self.current_ms = audio.position_ms();
            } else {
                self.current_ms = self.current_ms.saturating_add(
                    (elapsed.as_secs_f64() * 1000.0 * self.combined_speed() as f64).round() as i64,
                );
            }
        } else {
            self.current_ms = self.current_ms.saturating_add(
                (elapsed.as_secs_f64() * 1000.0 * self.combined_speed() as f64).round() as i64,
            );
            if self.current_ms >= 0 {
                self.current_ms = 0;
                let combined_speed = self.combined_speed();
                if let Some(audio) = &mut self.audio {
                    audio.seek(0);
                    audio.set_speed(combined_speed);
                    audio.play();
                }
            }
        }
        if self.current_ms >= self.session.timeline().last_object_ms {
            self.current_ms = self.session.timeline().last_object_ms;
            self.set_playing(false);
        }
    }

    fn render_current(&mut self, context: &egui::Context) {
        if self.rendered_ms == Some(self.current_ms) {
            return;
        }
        match pollster::block_on(
            self.renderer
                .render_at_absolute(&self.session, self.current_ms),
        ) {
            Ok(frame) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width() as usize, frame.height() as usize],
                    frame.as_bytes(),
                );
                match &mut self.texture {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.texture = Some(context.load_texture(
                            "beatmap-frame",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                self.rendered_ms = Some(self.current_ms);
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

impl eframe::App for PlayerApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.advance_clock();
        self.render_current(context);
        if self.playing {
            context.request_repaint_after(Duration::from_millis(8));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("player-controls").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let command = if self.playing { "Pause" } else { "Play" };
                if ui.button(command).clicked() {
                    self.set_playing(!self.playing);
                }
                let old_speed = self.runtime_speed;
                ui.add(
                    egui::Slider::new(&mut self.runtime_speed, 0.5..=2.0)
                        .step_by(0.05)
                        .suffix("x"),
                );
                if old_speed != self.runtime_speed {
                    let combined_speed = self.combined_speed();
                    if let Some(audio) = &mut self.audio {
                        audio.set_speed(combined_speed);
                    }
                }
            });
            let timeline = self.session.timeline();
            let mut target = self.current_ms as f64;
            let response = ui.add(
                egui::Slider::new(
                    &mut target,
                    timeline.absolute_start_ms as f64..=timeline.last_object_ms as f64,
                )
                .show_value(false),
            );
            if response.drag_started() {
                self.resume_after_drag = self.playing;
                self.set_playing(false);
            }
            if response.changed() {
                self.seek(target.round() as i64);
            }
            if response.drag_stopped() && self.resume_after_drag {
                self.resume_after_drag = false;
                self.set_playing(true);
            }
            ui.add_space(6.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(texture) = &self.texture {
                let available = ui.available_size();
                let image_size = texture.size_vec2();
                let scale = (available.x / image_size.x)
                    .min(available.y / image_size.y)
                    .max(0.01);
                ui.centered_and_justified(|ui| {
                    ui.add(egui::Image::new(texture).fit_to_exact_size(image_size * scale));
                });
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });
    }
}

struct AudioController {
    _device: rodio::MixerDeviceSink,
    player: rodio::Player,
}

impl AudioController {
    fn open(path: &std::path::Path) -> Option<Self> {
        let mut device = rodio::DeviceSinkBuilder::open_default_sink().ok()?;
        device.log_on_drop(false);
        let player = rodio::Player::connect_new(device.mixer());
        let source = rodio::Decoder::try_from(File::open(path).ok()?).ok()?;
        player.append(source);
        player.pause();
        Some(Self {
            _device: device,
            player,
        })
    }

    fn play(&self) {
        self.player.play();
    }

    fn pause(&self) {
        self.player.pause();
    }

    fn seek(&self, absolute_time_ms: i64) {
        let _ = self
            .player
            .try_seek(Duration::from_millis(absolute_time_ms.max(0) as u64));
    }

    fn set_speed(&self, speed: f32) {
        self.player.set_speed(speed.clamp(0.01, 8.0));
    }

    fn position_ms(&self) -> i64 {
        self.player.get_pos().as_millis().min(i64::MAX as u128) as i64
    }
}

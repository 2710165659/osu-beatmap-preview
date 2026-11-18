//! 将后台下载的媒体包补入现有会话，不重建谱面、渲染器或播放时钟。

use super::*;

#[wasm_bindgen]
impl WebGpuSession {
    /// 媒体按创建会话时的原始谱面绑定；解包或音乐解码失败时保留原会话。
    #[wasm_bindgen(js_name = loadResources)]
    pub fn load_resources(&mut self, bytes: Vec<u8>, bid: Option<String>) -> Result<(), JsValue> {
        if self.inner.replay_timeline().is_some() {
            return Err(JsValue::from_str("回放资源必须在创建会话时按 MD5 校验"));
        }
        let selector = bid
            .as_deref()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(archive::DifficultySelector::ById)
            .unwrap_or_default();
        let content = archive::read_input_with_osu(
            &bytes,
            Some(&self.beatmap_bytes),
            &selector,
            true,
        )
        .map_err(|error| JsValue::from_str(&error))?;
        let music = content
            .audio
            .as_deref()
            .map(|audio| decode::decode_music(audio, extension_of(&content.audio_name)))
            .transpose()
            .map_err(|error| JsValue::from_str(&error))?;
        let storyboard =
            build_storyboard(&content, &bytes).map_err(|error| JsValue::from_str(&error))?;
        let background = content
            .background
            .as_deref()
            .and_then(decode::decode_background);
        let dim = self.inner.options().video_style.background_dim;
        let video_start_ms = self
            .inner
            .beatmap()
            .video
            .as_ref()
            .map_or(0, |video| video.start_ms);
        let mut video = content
            .video
            .and_then(|bytes| video::BackgroundVideo::open(bytes, video_start_ms, 1.0 - dim).ok());
        if let Some(video) = video.as_mut() {
            video.set_capture_limit(self.gpu.config.width, self.gpu.config.height);
        }

        if let Some(source) = background.as_ref() {
            self.inner
                .set_background(decode::dim_background(source, dim))
                .map_err(js_error)?;
        }
        self.background_source = background;
        if let Some(video) = self.video.as_ref() {
            video.pause();
        }
        self.inner.clear_background_video();
        self.video_showing = false;
        self.video = video;
        self.inner.set_storyboard(storyboard);
        self.inner.set_music(music);
        self.custom_samples = content.samples;
        load_samples(
            &mut self.inner,
            &self.custom_samples,
            self.use_beatmap_samples,
        );
        // 已预读的静音 PCM 不能继续占用输出缓冲，按当前谱面位置开启新纪元。
        let now = wall_ms();
        let position = self.inner.clock_ms(now);
        self.inner.seek(now, position);
        Ok(())
    }
}

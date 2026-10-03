//! 会话创建所需的输入资源和选项。

use std::sync::Arc;

use crate::gameplay::GameplayOptions;
use crate::hitsound::SAMPLE_RATE;
use crate::storyboard::Storyboard;
use crate::{config, Beatmap};

/// 会话创建时可以提供的谱面与背景资源。音乐不在这里：宿主解码后经
/// [`crate::api::session::RealtimeSession::set_music`] 放进混音器。
#[derive(Debug, Clone, Default)]
pub struct ResourceBundle {
    pub beatmap: Option<Beatmap>,
    pub background: Option<ImageData>,
    /// 故事板（解析结果 + 贴图）；是否绘制由 [`RealtimeOptions::storyboard_enabled`] 控制。
    pub storyboard: Option<StoryboardBundle>,
}

impl ResourceBundle {
    pub fn new(beatmap: Beatmap) -> Self {
        Self {
            beatmap: Some(beatmap),
            ..Self::default()
        }
    }
}

/// 宿主提供的故事板资源：解析结果与贴图。
///
/// 解析由 [`crate::storyboard::parse_storyboard`] 完成（.osu 的 `[Events]` 与
/// `.osb` 合并），贴图由宿主解码为 RGBA 后按归一化路径放入。
#[derive(Debug, Clone)]
pub struct StoryboardBundle {
    pub storyboard: Storyboard,
    /// 贴图：归一化路径 → RGBA 图像。
    pub textures: Vec<(String, ImageData)>,
}

/// 宿主提供的 RGBA 背景图像。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 视频输出尺寸与逻辑缩放。
#[derive(Debug, Clone, PartialEq)]
pub struct RenderConfig {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            scale: 1.0,
        }
    }
}

impl RenderConfig {
    /// 返回至少为 1x1 的实际输出尺寸。
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width.max(1), self.height.max(1))
    }
}

/// 音频输出配置：音乐与打击音由 core 统一混音，宿主只把混音结果送到音频设备。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioConfig {
    /// 混音输出采样率，必须等于宿主音频设备的实际采样率，混音结果才能直接播放。
    pub sample_rate: u32,
    pub hitsound_enabled: bool,
    /// 打击音音量百分比（0..=100）。
    pub hitsound_volume: i32,
    /// 音乐音量百分比（0..=100）。
    pub music_volume: i32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: SAMPLE_RATE,
            hitsound_enabled: true,
            hitsound_volume: 100,
            music_volume: 50,
        }
    }
}

/// 实时预览会话的渲染、转换和核心配置。
#[derive(Debug, Clone)]
pub struct RealtimeOptions {
    pub convert: Option<String>,
    pub mods: Vec<String>,
    pub render: RenderConfig,
    pub video_style: crate::render::wgpu::VideoStyle,
    pub core_config: Arc<config::CoreConfig>,
    /// 音频输出配置（采样率、开关与初始音量）。
    pub audio: AudioConfig,
    /// 游玩/回放配置。目前只保留配置位（默认 `GameplayMode::Preview`，不改变任何
    /// 现有行为），判定引擎与画面叠加后续接入，接口见 `docs/architecture.md`。
    pub gameplay: GameplayOptions,
    /// 是否绘制故事板；默认关闭（素材与合成开销不小，且不是所有谱面都有故事板）。
    pub storyboard_enabled: bool,
}

impl Default for RealtimeOptions {
    fn default() -> Self {
        Self {
            convert: None,
            mods: Vec::new(),
            render: RenderConfig::default(),
            video_style: crate::render::wgpu::VideoStyle::default(),
            core_config: Arc::new(config::CoreConfig::default()),
            audio: AudioConfig::default(),
            gameplay: GameplayOptions::default(),
            storyboard_enabled: false,
        }
    }
}

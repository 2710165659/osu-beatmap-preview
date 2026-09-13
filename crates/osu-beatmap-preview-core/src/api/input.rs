//! 会话创建所需的输入资源和选项。

use std::sync::Arc;

use crate::gameplay::GameplayOptions;
use crate::{config, Beatmap};

/// 会话创建时可以提供的谱面与背景资源。
///
/// 音乐不在这里：预览由宿主自己播放（Web 的 `<audio>` 元素），导出由宿主解码后与
/// 打击音混音，core 只负责「什么时候、多大声」的打击音部分。
#[derive(Debug, Clone, Default)]
pub struct ResourceBundle {
    pub beatmap: Option<Beatmap>,
    pub background: Option<ImageData>,
}

impl ResourceBundle {
    pub fn new(beatmap: Beatmap) -> Self {
        Self {
            beatmap: Some(beatmap),
            ..Self::default()
        }
    }
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

/// 实时预览会话的渲染、转换和核心配置。
#[derive(Debug, Clone)]
pub struct RealtimeOptions {
    pub convert: Option<String>,
    pub mods: Vec<String>,
    pub render: RenderConfig,
    pub video_style: crate::render::wgpu::VideoStyle,
    pub core_config: Arc<config::CoreConfig>,
    /// 游玩/回放配置。
    ///
    /// 目前只保留配置位（默认 `GameplayMode::Preview`，不改变任何现有行为）：
    /// 判定引擎与画面叠加的实现在后续阶段接入，接口见
    /// [`crate::gameplay`](crate::gameplay) 与 `docs/architecture.md` 的「后续功能接口」。
    pub gameplay: GameplayOptions,
}

impl Default for RealtimeOptions {
    fn default() -> Self {
        Self {
            convert: None,
            mods: Vec::new(),
            render: RenderConfig::default(),
            video_style: crate::render::wgpu::VideoStyle::default(),
            core_config: Arc::new(config::CoreConfig::default()),
            gameplay: GameplayOptions::default(),
        }
    }
}

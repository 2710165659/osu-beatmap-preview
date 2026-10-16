//! osu 与 osr 共用的请求来源、视图、输出和执行选项。

use osu_beatmap_preview_core::processing::validation::TimePoint;

#[derive(Debug, Clone)]
pub struct SourceOptions {
    /// 谱面 ID；回放玩家查询时表示目标难度 ID。
    pub bid: String,
    /// 本地 `.osu` 路径。
    pub osu_file: Option<String>,
    /// 本地 `.osz` 路径。
    pub osz_file: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ViewOptions {
    pub time_points: Vec<TimePoint>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct OutputOptions {
    pub format: Option<String>,
    pub fps: Option<u32>,
    pub scale: Option<f64>,
    pub output_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    pub no_cache: bool,
    pub logging: bool,
    pub config: Option<String>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            no_cache: false,
            logging: true,
            config: None,
        }
    }
}

mod application;
pub(crate) mod domain;
pub(crate) mod infrastructure;
mod render;

pub use application::request::{parse_fps, parse_positive_finite};
pub use application::{
    ExecutionOptions, OutputOptions, RenderRequest, RulesetOptions, SourceOptions, ViewOptions,
};
pub use domain::errors::{ErrorKind, PreviewError};
pub use domain::validate::{parse_time_point, TimePoint};

#[derive(Debug, Clone)]
pub struct PreviewOptions {
    pub bid: String,
    pub convert: Option<String>,
    pub mods: Vec<String>,
    pub format: Option<String>,
    pub time_points: Vec<TimePoint>,
    pub duration_time: Option<f64>,
    pub no_cache: bool,
    pub fps: Option<u32>,
    pub config: Option<String>,
    pub scale: Option<f64>,
    pub output_dir: Option<String>,
}

impl PreviewOptions {
    pub fn new(bid: impl Into<String>) -> Self {
        Self {
            bid: bid.into(),
            convert: None,
            mods: Vec::new(),
            format: None,
            time_points: Vec::new(),
            duration_time: None,
            no_cache: false,
            fps: None,
            config: None,
            scale: None,
            output_dir: None,
        }
    }
}

impl From<PreviewOptions> for RenderRequest {
    fn from(options: PreviewOptions) -> Self {
        Self {
            source: SourceOptions { bid: options.bid },
            ruleset: RulesetOptions {
                convert: options.convert,
                mods: options.mods,
            },
            view: ViewOptions {
                time_points: options.time_points,
                duration_seconds: options.duration_time,
            },
            output: OutputOptions {
                format: options.format,
                fps: options.fps,
                scale: options.scale,
                output_dir: options.output_dir,
            },
            execution: ExecutionOptions {
                no_cache: options.no_cache,
                logging: true,
                config: options.config,
            },
        }
    }
}

pub fn generate_preview(
    request: impl Into<RenderRequest>,
) -> Result<serde_json::Value, PreviewError> {
    application::engine::execute(request.into())
}

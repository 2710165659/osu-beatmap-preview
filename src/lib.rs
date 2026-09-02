mod common;
mod config;
mod core;
mod log;
mod parser;
mod pipeline;
mod render;

pub use core::errors::{ErrorKind, PreviewError};
pub use core::validate::{parse_time_point, TimePoint};

#[derive(Debug, Clone)]
pub struct PreviewOptions {
    pub bid: String,
    pub convert: Option<String>,
    pub mods: Vec<String>,
    pub format: Option<String>,
    pub time_points: Vec<TimePoint>,
    pub duration_time: Option<f64>,
    pub no_cache: bool,
    pub config: Option<String>,
    pub scale: Option<f64>,
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
            config: None,
            scale: None,
        }
    }
}

pub fn generate_preview(options: PreviewOptions) -> Result<serde_json::Value, PreviewError> {
    let mods = core::validate::validate_cli_options(
        &options.bid,
        options.convert.as_deref(),
        options.format.as_deref(),
        &options.mods,
        options.duration_time,
        options.scale,
    )?;
    if let Err(error) = config::initialize_for_cli(options.config.as_deref(), options.scale) {
        if options.config.is_some() || !error.contains("already been initialized") {
            return Err(PreviewError::new(format!("configuration error: {error}")));
        }
    }

    pipeline::service::generate_preview(
        &options.bid,
        options.format.as_deref(),
        options.convert.as_deref(),
        mods,
        options.time_points,
        options.duration_time,
        options.no_cache,
    )
}

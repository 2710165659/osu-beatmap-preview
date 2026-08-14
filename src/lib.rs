mod common;
mod core;
mod log;
mod parser;
mod pipeline;
mod render;


pub use core::errors::{PreviewError, ErrorKind};

#[derive(Debug, Clone)]
pub struct PreviewOptions {
    pub bid: String,
    pub convert: Option<String>,
    pub mods: Option<String>,
    pub format: Option<String>,
    pub times: Option<String>,

    pub gif_clip: bool,
    pub gif_clip_label: bool,
    pub preview_30s: bool,

    pub gap: Option<f64>,
    pub no_cache: bool,
}

impl PreviewOptions {
    pub fn new(bid: impl Into<String>) -> Self {
        Self {
            bid: bid.into(),
            convert: None,
            mods: None,
            format: None,
            times: None,
            gif_clip: false,
            gif_clip_label: false,
            preview_30s: false,
            gap: None,
            no_cache: false,
        }
    }
}

pub fn generate_preview(
    options: PreviewOptions,
) -> Result<serde_json::Value, PreviewError> {
    let mods = match &options.mods {
        Some(value) => Some(core::mods::parse_mods(value)?),
        None => None,
    };

    let times = match &options.times {
        Some(value) => Some(core::validate::parse_times(value)?),
        None => None,
    };

    pipeline::service::generate_preview(
        &options.bid,
        options.format.as_deref(),
        options.convert.as_deref(),
        mods,
        times,
        options.gif_clip,
        options.gif_clip_label,
        options.preview_30s,
        options.gap,
        options.no_cache,
    )
}
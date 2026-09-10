//! 谱面处理能力的公开门面。
//!
//! 这里按“解析、规则转换、时间选择、校验”组织能力，避免调用者依赖历史上的
//! `domain::parser`、`domain::rulesets` 和 `domain::shared` 嵌套路径。

pub mod parse {
    pub use crate::domain::parser::{
        default_metadata, parse_background_filename, parse_beatmap_bytes, parse_break_periods,
        parse_combo_colors, parse_format_version, parse_key_value, parse_timing_points,
        resolve_slider_timing, round_half_even, split_sections,
    };
}

pub mod conversion {
    pub use crate::domain::rulesets::{catch_convert, mania_convert, taiko_convert};
}

pub mod timeline {
    pub use crate::domain::shared::time_selection::{
        snap_to_beat_grid, GifRenderOptions, PreviewSegmentTiming, PreviewTimeSelector, TimeAxis,
    };
}

pub mod validation {
    pub use crate::domain::validate::{
        parse_positive_finite, parse_time_point, validate_bid, validate_convert_value,
        validate_fmt_value, validate_positive_finite, validate_with_context, TimePoint,
        ValidateContext,
    };
}

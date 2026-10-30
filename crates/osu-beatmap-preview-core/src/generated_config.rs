// 由 assets/shared_config.yml 自动生成，请勿手动修改。

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardPngStructureConfig {
    pub ROW_COUNT: usize,
    pub IMAGES_PER_ROW: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardPngStyleConfig {
    pub SHOW_CURSOR: bool,
    pub MS_PER_IMAGE: i64,
    pub SLIDER_BODY_SUPERSAMPLE: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardPngConfig {
    pub SCALE: f64,
    pub structure: RenderStandardPngStructureConfig,
    pub style: RenderStandardPngStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardGifStructureConfig {
    pub ROW_COUNT: usize,
    pub IMAGES_PER_ROW: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardGifStyleConfig {
    pub SHOW_CURSOR: bool,
    pub DURATION_MS: i64,
    pub FPS: i64,
    pub SLIDER_BODY_SUPERSAMPLE: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardGifConfig {
    pub SCALE: f64,
    pub structure: RenderStandardGifStructureConfig,
    pub style: RenderStandardGifStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardMp4StyleConfig {
    pub SHOW_CURSOR: bool,
    pub FPS: i64,
    pub ENABLE_HITSOUND: bool,
    pub ENABLE_BEATMAP_HITSOUND: bool,
    pub HITSOUND_VOLUME: i64,
    pub ENABLE_BACKGROUND_IMAGE: bool,
    pub ENABLE_BACKGROUND_VIDEO: bool,
    pub ENABLE_STORYBOARD: bool,
    pub BACKGROUND_DIM: f64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardMp4Config {
    pub SCALE: f64,
    pub style: RenderStandardMp4StyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderStandardConfig {
    pub png: RenderStandardPngConfig,
    pub gif: RenderStandardGifConfig,
    pub mp4: RenderStandardMp4Config,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoPngSizingConfig {
    pub BASE_ROW_WIDTH_0_TO_1_MINUTES: i64,
    pub BASE_ROW_WIDTH_1_TO_2_MINUTES: i64,
    pub BASE_ROW_WIDTH_2_TO_3_MINUTES: i64,
    pub BASE_ROW_WIDTH_3_TO_4_MINUTES: i64,
    pub BASE_ROW_WIDTH_4_TO_5_MINUTES: i64,
    pub BASE_ROW_WIDTH_5_TO_6_MINUTES: i64,
    pub BASE_ROW_WIDTH_6_TO_10_MINUTES: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoPngStyleConfig {
    pub MAX_SUPPORTED_DURATION_MS: i64,
    pub ROW_WIDTH_MULTIPLIER_BPM_0_TO_180: f64,
    pub ROW_WIDTH_MULTIPLIER_BPM_180_TO_240: f64,
    pub ROW_WIDTH_MULTIPLIER_BPM_240_TO_300: f64,
    pub ROW_WIDTH_MULTIPLIER_BPM_300_PLUS: f64,
    pub SPACING_PER_BPM: f64,
    pub TIME_LABEL_MIN_INTERVAL_MS: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoPngConfig {
    pub SCALE: f64,
    pub sizing: RenderTaikoPngSizingConfig,
    pub style: RenderTaikoPngStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoGifStructureConfig {
    pub ROW_COUNT: usize,
    pub IMAGES_PER_ROW: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoGifStyleConfig {
    pub SHOW_HIT_EFFECTS: bool,
    pub SHOW_KEY_OVERLAY: bool,
    pub SHOW_MEASURE_LINES: bool,
    pub DURATION_MS: f64,
    pub FPS: f64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoGifConfig {
    pub SCALE: f64,
    pub structure: RenderTaikoGifStructureConfig,
    pub style: RenderTaikoGifStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoMp4StyleConfig {
    pub SHOW_HIT_EFFECTS: bool,
    pub SHOW_KEY_OVERLAY: bool,
    pub ENABLE_BACKGROUND_IMAGE: bool,
    pub ENABLE_BACKGROUND_VIDEO: bool,
    pub ENABLE_STORYBOARD: bool,
    pub BACKGROUND_DIM: f64,
    pub SHOW_MEASURE_LINES: bool,
    pub FPS: f64,
    pub ENABLE_HITSOUND: bool,
    pub ENABLE_BEATMAP_HITSOUND: bool,
    pub HITSOUND_VOLUME: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoMp4Config {
    pub SCALE: f64,
    pub style: RenderTaikoMp4StyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderTaikoConfig {
    pub png: RenderTaikoPngConfig,
    pub gif: RenderTaikoGifConfig,
    pub mp4: RenderTaikoMp4Config,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchPngSizingConfig {
    pub MAX_AREA_HEIGHT_0_TO_1_MINUTES: i64,
    pub MAX_AREA_HEIGHT_1_TO_2_MINUTES: i64,
    pub MAX_AREA_HEIGHT_2_TO_3_MINUTES: i64,
    pub MAX_AREA_HEIGHT_3_TO_4_MINUTES: i64,
    pub MAX_AREA_HEIGHT_4_TO_5_MINUTES: i64,
    pub MAX_AREA_HEIGHT_5_TO_6_MINUTES: i64,
    pub MAX_TOTAL_CHART_HEIGHT: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchPngStyleConfig {
    pub SHOW_BANANA_ROUTE: bool,
    pub MAX_SUPPORTED_DURATION_MS: i64,
    pub TIME_LABEL_MIN_INTERVAL_MS: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchPngConfig {
    pub SCALE: f64,
    pub sizing: RenderCatchPngSizingConfig,
    pub style: RenderCatchPngStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchGifStructureConfig {
    pub ROW_COUNT: usize,
    pub IMAGES_PER_ROW: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchGifStyleConfig {
    pub SHOW_CATCHER: bool,
    pub DURATION_MS: f64,
    pub FPS: f64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchGifConfig {
    pub SCALE: f64,
    pub structure: RenderCatchGifStructureConfig,
    pub style: RenderCatchGifStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchMp4StyleConfig {
    pub SHOW_CATCHER: bool,
    pub ENABLE_BACKGROUND_IMAGE: bool,
    pub ENABLE_BACKGROUND_VIDEO: bool,
    pub ENABLE_STORYBOARD: bool,
    pub BACKGROUND_DIM: f64,
    pub FPS: f64,
    pub ENABLE_HITSOUND: bool,
    pub ENABLE_BEATMAP_HITSOUND: bool,
    pub HITSOUND_VOLUME: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchMp4Config {
    pub SCALE: f64,
    pub style: RenderCatchMp4StyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderCatchConfig {
    pub png: RenderCatchPngConfig,
    pub gif: RenderCatchGifConfig,
    pub mp4: RenderCatchMp4Config,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaPngStructureConfig {
    pub FIXED_COLUMN_COUNT_6_TO_10_MINUTES: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaPngSizingConfig {
    pub PIXELS_PER_MS: f64,
    pub MAX_AREA_HEIGHT_0_TO_1_MINUTES: i64,
    pub MAX_AREA_HEIGHT_1_TO_2_MINUTES: i64,
    pub MAX_AREA_HEIGHT_2_TO_3_MINUTES: i64,
    pub MAX_AREA_HEIGHT_3_TO_4_MINUTES: i64,
    pub MAX_AREA_HEIGHT_4_TO_5_MINUTES: i64,
    pub MAX_AREA_HEIGHT_5_TO_6_MINUTES: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaPngStyleConfig {
    pub SHOW_SV_LABEL: bool,
    pub TIME_LABEL_MIN_INTERVAL_MS: i64,
    pub MAX_SUPPORTED_DURATION_MS: i64,
    pub BOTTOM_PADDING_MS: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaPngConfig {
    pub SCALE: f64,
    pub structure: RenderManiaPngStructureConfig,
    pub sizing: RenderManiaPngSizingConfig,
    pub style: RenderManiaPngStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaGifStructureConfig {
    pub ROW_COUNT: usize,
    pub IMAGES_PER_ROW: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaGifStyleConfig {
    pub DURATION_MS: i64,
    pub FPS: i64,
    pub SCROLL_SPEED: f64,
    pub SHOW_SV_LABEL: bool,
    pub SHOW_HIT_FLASH: bool,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaGifConfig {
    pub SCALE: f64,
    pub structure: RenderManiaGifStructureConfig,
    pub style: RenderManiaGifStyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaMp4StyleConfig {
    pub ENABLE_BACKGROUND_IMAGE: bool,
    pub ENABLE_BACKGROUND_VIDEO: bool,
    pub ENABLE_STORYBOARD: bool,
    pub BACKGROUND_DIM: f64,
    pub FPS: i64,
    pub ENABLE_HITSOUND: bool,
    pub ENABLE_BEATMAP_HITSOUND: bool,
    pub HITSOUND_VOLUME: i64,
    pub SHOW_SV_LABEL: bool,
    pub SHOW_HIT_FLASH: bool,
    pub LANE_DARKEN_ALPHA: f64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaMp4Config {
    pub SCALE: f64,
    pub style: RenderManiaMp4StyleConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderManiaConfig {
    pub png: RenderManiaPngConfig,
    pub gif: RenderManiaGifConfig,
    pub mp4: RenderManiaMp4Config,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct RenderConfig {
    pub standard: RenderStandardConfig,
    pub taiko: RenderTaikoConfig,
    pub catch: RenderCatchConfig,
    pub mania: RenderManiaConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_1Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_2Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_3Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_4Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_5Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_6Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_7Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_8Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_9Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_10Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_11Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_12Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_13Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_14Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_15Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_16Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_17Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAKEYS_18Config {
    pub COLUMN_WIDTHS: Vec<i64>,
    pub COLUMN_LINE_WIDTHS: Vec<i64>,
    pub HIT_POSITION: i64,
    pub COMBO_POSITION: i64,
    pub SCORE_POSITION: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinMANIAConfig {
    pub KEYS_1: SkinMANIAKEYS_1Config,
    pub KEYS_2: SkinMANIAKEYS_2Config,
    pub KEYS_3: SkinMANIAKEYS_3Config,
    pub KEYS_4: SkinMANIAKEYS_4Config,
    pub KEYS_5: SkinMANIAKEYS_5Config,
    pub KEYS_6: SkinMANIAKEYS_6Config,
    pub KEYS_7: SkinMANIAKEYS_7Config,
    pub KEYS_8: SkinMANIAKEYS_8Config,
    pub KEYS_9: SkinMANIAKEYS_9Config,
    pub KEYS_10: SkinMANIAKEYS_10Config,
    pub KEYS_11: SkinMANIAKEYS_11Config,
    pub KEYS_12: SkinMANIAKEYS_12Config,
    pub KEYS_13: SkinMANIAKEYS_13Config,
    pub KEYS_14: SkinMANIAKEYS_14Config,
    pub KEYS_15: SkinMANIAKEYS_15Config,
    pub KEYS_16: SkinMANIAKEYS_16Config,
    pub KEYS_17: SkinMANIAKEYS_17Config,
    pub KEYS_18: SkinMANIAKEYS_18Config,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, non_snake_case, non_camel_case_types)]
pub struct SkinConfig {
    pub COMBO_COLORS: Vec<[u8; 3]>,
    pub HIT_CIRCLE_OVERLAP: i64,
    pub HYPER_DASH: [u8; 3],
    pub MANIA: SkinMANIAConfig,
}

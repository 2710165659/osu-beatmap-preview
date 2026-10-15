//! 保留 OSR 原始成绩与输入，动态判定结果另存于 gameplay。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayHeader {
    pub ruleset_id: u8,
    pub version: i32,
    pub beatmap_md5: String,
    pub player: String,
    pub replay_hash: String,
    /// 依次为 300、100、50、Geki、Katu、Miss；不套用其他模式统计。
    pub counts: [u16; 6],
    pub total_score: u32,
    pub max_combo: u16,
    pub perfect: bool,
    pub mods_bits: u32,
    pub life_graph: String,
    pub timestamp_ticks: i64,
    pub online_id: i64,
    pub target_practice_accuracy: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ReplayMod {
    pub acronym: String,
    #[serde(default)]
    pub settings: BTreeMap<String, serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ReplayMetadata {
    #[serde(default, deserialize_with = "optional_string")]
    pub client_version: String,
    #[serde(default, deserialize_with = "optional_string")]
    pub rank: String,
    #[serde(default)]
    pub mods: Vec<ReplayMod>,
    #[serde(default)]
    pub statistics: BTreeMap<String, u32>,
    #[serde(default)]
    pub maximum_statistics: BTreeMap<String, u32>,
    #[serde(default)]
    pub pauses: Vec<serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplayFrame {
    pub delta_ms: i64,
    pub time_ms: i64,
    pub cursor: [f32; 2],
    pub keys: u32,
    pub sequence: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayDocument {
    pub header: ReplayHeader,
    pub metadata: Option<ReplayMetadata>,
    pub frames: Vec<ReplayFrame>,
    pub seed: Option<i64>,
    pub diagnostics: Vec<String>,
}

/// 官方历史扩展允许 rank/client_version 缺失或为 null；不影响输入帧播放。
fn optional_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

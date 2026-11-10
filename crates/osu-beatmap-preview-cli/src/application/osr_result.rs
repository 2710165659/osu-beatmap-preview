//! CLI 回放成绩 JSON：将重算值与 OSR 原记录并列，缺失的原记录以 - 表示。

use osu_beatmap_preview_core::{gameplay::state::GameplayState, replay::ReplayDocument};
use serde_json::{json, Value};

pub(super) fn state(calculated: &GameplayState, document: &ReplayDocument) -> Value {
    let mut state = serde_json::to_value(calculated).expect("回放状态可序列化");
    let recorded = document.recorded_statistics();
    let mut statistics = serde_json::Map::new();
    for name in calculated.statistics.keys().chain(recorded.keys()) {
        statistics.insert(
            name.clone(),
            pair(
                json!(calculated.statistics.get(name).copied().unwrap_or(0)),
                recorded.get(name).map(|count| json!(count)),
            ),
        );
    }
    state["statistics"] = Value::Object(statistics);
    state["score"] = pair(
        json!(calculated.score),
        Some(json!(document.header.total_score)),
    );
    state["max_combo"] = pair(
        json!(calculated.max_combo),
        Some(json!(document.header.max_combo)),
    );
    state["combo"] = pair(json!(calculated.combo), None);
    let metadata = document.metadata.as_ref();
    // 只使用文件直接保存的准确率，不把从判定统计推导的值当作原始记录。
    let accuracy = metadata
        .and_then(|metadata| metadata.extra.get("accuracy"))
        .filter(|value| value.is_number())
        .cloned();
    state["accuracy"] = pair(json!(calculated.accuracy), accuracy);
    state["rank"] = pair(
        json!(calculated.rank),
        metadata
            .filter(|metadata| !metadata.rank.is_empty())
            .map(|metadata| json!(metadata.rank)),
    );
    state
}

fn pair(calculated: Value, recorded: Option<Value>) -> Value {
    fn text(value: &Value) -> String {
        match value {
            Value::Null => "-".into(),
            Value::String(value) => value.clone(),
            _ => value.to_string(),
        }
    }
    Value::String(format!(
        "{}/{}",
        text(&calculated),
        recorded.as_ref().map_or_else(|| "-".into(), text)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use osu_beatmap_preview_core::replay::{parse_osr, ReplayMetadata};

    #[test]
    fn recorded_counts_follow_each_ruleset_and_preserve_missing_fields() {
        let mut document = parse_osr(include_bytes!(
            "../../../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
        ))
        .unwrap();
        document.metadata = None;
        document.header.counts = [10, 2, 3, 4, 5, 6];
        let calculated = GameplayState {
            accuracy: Some(0.95),
            statistics: [
                ("great".into(), 9),
                ("perfect".into(), 3),
                ("miss".into(), 7),
                ("large_tick_miss".into(), 2),
            ]
            .into(),
            ..GameplayState::default()
        };
        for mode in 0..=3 {
            document.header.ruleset_id = mode;
            let result = state(&calculated, &document);
            assert_eq!(result["statistics"]["great"], "9/10");
            assert_eq!(
                result["statistics"]["perfect"],
                if mode == 3 { "3/4" } else { "3/-" }
            );
            assert_eq!(
                result["statistics"]["miss"],
                if mode == 2 { "7/-" } else { "7/6" }
            );
            assert_eq!(result["statistics"]["large_tick_miss"], "2/-");
            assert_eq!(result["accuracy"], "0.95/-");
        }
        document.metadata = Some(
            serde_json::from_value::<ReplayMetadata>(json!({
                "rank": "S",
                "accuracy": 0.96,
                "statistics": {"perfect": 8, "great": 0, "slider_tail_hit": 4}
            }))
            .unwrap(),
        );
        let result = state(&calculated, &document);
        assert_eq!(result["statistics"]["perfect"], "3/8");
        assert_eq!(result["statistics"]["great"], "9/0");
        assert_eq!(result["statistics"]["slider_tail_hit"], "0/4");
        assert_eq!(result["statistics"]["miss"], "7/-");
        assert_eq!(result["accuracy"], "0.95/0.96");
        assert_eq!(result["rank"], "-/S");
        assert_eq!(
            result["score"],
            format!("0/{}", document.header.total_score)
        );
        assert_eq!(
            result["max_combo"],
            format!("0/{}", document.header.max_combo)
        );
    }
}

//! lazer OSR 尾部扩展 JSON：已知字段沿用游戏的数据类型，未知字段保留而不拒绝。
use super::{decode, header::Reader};
use crate::replay::model::ReplayMetadata;
use crate::support::error::{PreviewError, Result};
use serde::{
    de::{MapAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Value};

pub(super) fn parse(reader: &mut Reader<'_>, version: i32) -> Result<Option<ReplayMetadata>> {
    if version < 30_000_001 {
        return Ok(None);
    }
    let compressed = reader.field("score_info", Reader::byte_array)?;
    if compressed.is_empty() {
        return Ok(None);
    }
    let bytes = decode::decompress(compressed)?;
    let json: ScoreObject =
        serde_json::from_str(&decode::text(&bytes)).map_err(|e| invalid(e.to_string()))?;
    serde_json::from_value(Value::Object(json.0))
        .map(Some)
        .map_err(|e| invalid(e.to_string()))
}

struct ScoreObject(Map<String, Value>);
impl<'de> Deserialize<'de> for ScoreObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ScoreVisitor;
        impl<'de> Visitor<'de> for ScoreVisitor {
            type Value = ScoreObject;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a score object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut object = Map::new();
                while let Some((key, mut value)) = entries.next_entry::<String, Value>()? {
                    let name = canonical_field(&key);
                    normalize_field(name, &mut value).map_err(serde::de::Error::custom)?;
                    object.insert(name.to_owned(), value);
                }
                Ok(ScoreObject(object))
            }
        }
        // 按出现顺序校验并赋值，保留 Newtonsoft 的大小写容错和后值覆盖；错误前值不能被覆盖隐藏。
        deserializer.deserialize_map(ScoreVisitor)
    }
}
fn canonical_field(name: &str) -> &str {
    [
        "client_version",
        "rank",
        "statistics",
        "maximum_statistics",
        "pauses",
        "online_id",
        "user_id",
        "total_score_without_mods",
        "mods",
    ]
    .into_iter()
    .find(|known| known.eq_ignore_ascii_case(name))
    .unwrap_or(name)
}
fn normalize_field(name: &str, value: &mut Value) -> Result<()> {
    match name {
        "online_id" | "user_id" | "total_score_without_mods" => {
            if name != "total_score_without_mods" || !value.is_null() {
                *value = integer(value, name, name == "user_id")?.into();
            }
        }
        "client_version" if !value.is_null() && !value.is_string() => {
            *value = match value {
                Value::Number(n) => Value::String(n.to_string()),
                Value::Bool(v) => Value::String(if *v { "True" } else { "False" }.into()),
                _ => return Err(invalid("client_version must be a string value")),
            };
        }
        "rank" if !value.is_null() => {
            *value = Value::String(rank(value)?);
        }
        "statistics" | "maximum_statistics" => {
            let values = value
                .as_object_mut()
                .ok_or_else(|| invalid(format!("{name} must be a dictionary")))?;
            let mut normalized = Map::new();
            for (key, value) in std::mem::take(values) {
                let key = hit_result(&key)
                    .ok_or_else(|| invalid(format!("{name}: unknown HitResult {key}")))?;
                normalized.insert(key, integer(&value, name, true)?.into());
            }
            *values = normalized;
        }
        "pauses" => {
            let values = value
                .as_array_mut()
                .ok_or_else(|| invalid("pauses must be an Int32 array"))?;
            for value in values {
                *value = integer(value, "pauses", true)?.into();
            }
        }
        _ => (),
    }
    Ok(())
}
fn rank(value: &Value) -> Result<String> {
    let text = if let Some(text) = value.as_str() {
        text.trim().to_owned()
    } else if let Some(number) = value.as_i64().and_then(|v| i32::try_from(v).ok()) {
        number.to_string()
    } else {
        return Err(invalid("invalid rank enum"));
    };
    if text.is_empty() {
        return Ok(String::new());
    }
    let names = ["F", "D", "C", "B", "A", "S", "SH", "X", "XH"];
    let number = if let Ok(number) = text.parse::<i32>() {
        number
    } else {
        // StringEnumConverter 也接受逗号分隔的枚举名称，以位或组合，即使 ScoreRank 未标记 Flags。
        text.split(',').try_fold(0, |value, name| {
            names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(name.trim()))
                .map(|i| value | (i as i32 - 1))
                .ok_or_else(|| invalid("invalid rank enum"))
        })?
    };
    Ok(names
        .get((i64::from(number) + 1) as usize)
        .map_or_else(|| number.to_string(), |s| (*s).into()))
}
fn integer(value: &Value, field: &str, int32: bool) -> Result<i64> {
    let parsed = if let Some(v) = value.as_i64() {
        Some(v)
    } else if let Some(v) = value.as_str() {
        v.trim().parse().ok()
    } else if !int32 && value.is_boolean() {
        value.as_bool().map(i64::from)
    } else if !int32 && value.is_number() {
        value
            .as_f64()
            .filter(|v| v.is_finite() && *v >= i64::MIN as f64 && *v < -(i64::MIN as f64))
            .map(|v| v.round_ties_even() as i64)
    } else {
        None
    };
    parsed
        .filter(|v| !int32 || i32::try_from(*v).is_ok())
        .ok_or_else(|| {
            invalid(format!(
                "{field} outside {} range or wrong type",
                if int32 { "Int32" } else { "Int64" }
            ))
        })
}
fn hit_result(key: &str) -> Option<String> {
    let key = key.trim();
    let names = [
        "none",
        "miss",
        "meh",
        "ok",
        "good",
        "great",
        "perfect",
        "small_tick_miss",
        "small_tick_hit",
        "large_tick_miss",
        "large_tick_hit",
        "small_bonus",
        "large_bonus",
        "ignore_miss",
        "ignore_hit",
        "combo_break",
        "slider_tail_hit",
    ];
    if let Ok(number) = key.parse::<i32>() {
        return Some(if number == 99 {
            "legacy_combo_increase".into()
        } else {
            names
                .get(number as usize)
                .map_or_else(|| number.to_string(), |s| (*s).into())
        });
    }
    names
        .into_iter()
        .chain(["legacy_combo_increase"])
        .find(|name| {
            name.eq_ignore_ascii_case(key) || name.replace('_', "").eq_ignore_ascii_case(key)
        })
        .map(str::to_owned)
}
fn invalid(reason: impl std::fmt::Display) -> PreviewError {
    PreviewError::parse(format!("InvalidReplay: invalid extension JSON: {reason}"))
}

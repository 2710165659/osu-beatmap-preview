//! 太鼓 PNG 排版 BPM 的配置解析，兼容旧版数字配置。

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TaikoPngSpacing {
    Auto,
    Most,
    Fixed(f64),
}

impl<'de> serde::Deserialize<'de> for TaikoPngSpacing {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Input {
            Number(f64),
            Text(String),
        }
        let bpm = match Input::deserialize(deserializer)? {
            Input::Number(value) => value,
            Input::Text(value) => {
                let value = value.trim().to_ascii_lowercase();
                match value.as_str() {
                    "auto" => return Ok(Self::Auto),
                    "most" => return Ok(Self::Most),
                    _ => {}
                }
                value
                    .strip_suffix("bpm")
                    .unwrap_or(&value)
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| {
                        serde::de::Error::custom(
                            "expected auto, most or a positive BPM such as 180BPM",
                        )
                    })?
            }
        };
        if !bpm.is_finite() || bpm < 0.0 {
            return Err(serde::de::Error::custom(
                "spacing BPM must be finite and non-negative",
            ));
        }
        Ok(if bpm == 0.0 {
            Self::Auto
        } else {
            Self::Fixed(bpm)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_modes_units_and_legacy_numbers() {
        for (source, expected) in [
            ("\"most\"", TaikoPngSpacing::Most),
            ("\" AuTo \"", TaikoPngSpacing::Auto),
            ("\"180bPm\"", TaikoPngSpacing::Fixed(180.0)),
            ("\" 179.5 BPM \"", TaikoPngSpacing::Fixed(179.5)),
            ("180", TaikoPngSpacing::Fixed(180.0)),
            ("0", TaikoPngSpacing::Auto),
            ("\"0\"", TaikoPngSpacing::Auto),
        ] {
            assert_eq!(
                serde_json::from_str::<TaikoPngSpacing>(source).unwrap(),
                expected
            );
        }
        for source in ["-1", "\"NaNBPM\"", "\"inf\"", "\"fast\"", "true"] {
            assert!(
                serde_json::from_str::<TaikoPngSpacing>(source).is_err(),
                "{source}"
            );
        }
    }
}

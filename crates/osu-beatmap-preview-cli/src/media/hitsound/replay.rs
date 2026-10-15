//! 回放专用提示音和循环策略；普通谱面预览仍使用原样本集合与循环规则。
use super::*;

pub(crate) fn build_library(
    beatmap_samples: Option<&Path>,
    names: impl IntoIterator<Item = String>,
) -> SampleLibrary {
    let names: Vec<_> = names.into_iter().collect();
    let mut library = build_library_with_names(beatmap_samples, names.iter().cloned());
    for name in names {
        if let Some(sample) = library.get(&name) {
            if name.to_ascii_lowercase().contains("sliderwhistle") {
                let sample = sample.clone().with_loop(sample.frames());
                library.insert(name, sample);
            }
            continue;
        }
        let bytes: &[u8] = match name.as_str() {
            "combobreak" => include_bytes!("../../../../../assets/hitsound/replay/combobreak.wav"),
            "failsound" => include_bytes!("../../../../../assets/hitsound/replay/failsound.wav"),
            _ => continue,
        };
        if let Ok(sample) = decode_sample(&name, bytes, Some("wav")) {
            library.insert(name, sample);
        }
    }
    library
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_prompts_decode_and_whistle_loops_without_changing_preview_library() {
        let names = ["combobreak", "failsound", "normal-sliderwhistle"].map(String::from);
        let replay = build_library(None, names.clone());
        for name in ["combobreak", "failsound"] {
            assert!(replay.get(name).unwrap().frames() > 0);
        }
        let whistle = replay.get("normal-sliderwhistle").unwrap();
        assert_eq!(whistle.loop_len, whistle.frames());
        let preview = build_library_with_names(None, names);
        assert!(!preview.contains("combobreak"));
        assert_eq!(preview.get("normal-sliderwhistle").unwrap().loop_len, 0);
    }
}

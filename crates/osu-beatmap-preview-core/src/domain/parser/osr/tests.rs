#![cfg(test)]
//! 合成 OSR fixture 覆盖二进制长度、压缩、负首帧和扩展 JSON。
use super::*;
fn string(bytes: &mut Vec<u8>, text: &str) {
    bytes.push(0x0b);
    let mut len = text.len();
    while len >= 128 {
        bytes.push((len as u8 & 127) | 128);
        len >>= 7;
    }
    bytes.push(len as u8);
    bytes.extend_from_slice(text.as_bytes());
}
fn compressed(bytes: &mut Vec<u8>, text: &str) {
    let mut output = Vec::new();
    lzma_rs::lzma_compress(&mut std::io::Cursor::new(text.as_bytes()), &mut output).unwrap();
    bytes.extend_from_slice(&(output.len() as i32).to_le_bytes());
    bytes.extend(output);
}
fn fixture(frames: &str) -> Vec<u8> {
    fixture_version(
        frames,
        30_000_016,
        Some(
            r#"{"client_version":"2026.119.0-lazer-windows","mods":[],"rank":"S","statistics":{},"maximum_statistics":{},"unknown":42}"#,
        ),
    )
}
fn fixture_version(frames: &str, version: i32, metadata: Option<&str>) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend_from_slice(&version.to_le_bytes());
    string(&mut bytes, "0123456789abcdef0123456789abcdef");
    string(&mut bytes, "玩家");
    string(&mut bytes, "hash");
    bytes.extend_from_slice(&[0; 23]);
    string(&mut bytes, "");
    bytes.extend_from_slice(&0_i64.to_le_bytes());
    compressed(&mut bytes, frames);
    if version >= 20140721 {
        bytes.extend_from_slice(&42_i64.to_le_bytes());
    } else if version >= 20121008 {
        bytes.extend_from_slice(&42_i32.to_le_bytes());
    }
    if version >= 30_000_001 {
        if let Some(metadata) = metadata {
            compressed(&mut bytes, metadata);
        } else {
            bytes.extend_from_slice(&0_i32.to_le_bytes());
        }
    }
    bytes
}
#[test]
fn decodes_header_frames_and_lzma_extension() {
    let document = parse_osr(&fixture("-10|-5|2|0,20|10|20|5,-12345|0|0|42")).unwrap();
    assert_eq!(document.header.player, "玩家");
    assert_eq!(document.header.online_id, 42);
    assert_eq!(document.frames[0].time_ms, -10);
    assert_eq!(document.seed, Some(42));
    assert_eq!(document.metadata.unwrap().extra["unknown"], 42);
}
#[test]
fn rejects_truncation_malformed_empty_nonfinite_and_corrupt_compression() {
    let bytes = fixture("0|1|2|0,");
    for len in [0, 5, 50, bytes.len() - 1] {
        assert!(parse_osr(&bytes[..len]).is_err());
    }
    for frames in ["0|NaN|2|0", "0|1|2|bad", "bad|1|2|0", "0|131073|2|0"] {
        assert!(parse_osr(&fixture(frames)).is_err(), "{frames}");
    }
    assert!(decode::decompress(&[255; 20]).is_err());
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(parse_osr(&bad).is_ok());
}
#[test]
fn backwards_start_frames_are_repaired_like_official_decoder() {
    let document = parse_osr(&fixture("0|1|2|0,-1|1|2|0")).unwrap();
    assert_eq!(
        document
            .frames
            .iter()
            .map(|f| f.time_ms)
            .collect::<Vec<_>>(),
        [0, 0]
    );
    assert!(crate::replay::compatibility::select(&document).is_ok());
}

#[test]
fn historic_fractional_frame_deltas_round_individually_like_lazer() {
    let document = parse_osr(&fixture("0|1|2|0,1.5|1|2|0,2.5|1|2|0,0.5|1|2|0")).unwrap();
    assert_eq!(
        document
            .frames
            .iter()
            .map(|f| f.time_ms)
            .collect::<Vec<_>>(),
        [0, 2, 4, 4]
    );
    for invalid in ["NaN|1|2|0", "inf|1|2|0", "90000000.5|1|2|0"] {
        assert!(parse_osr(&fixture(invalid)).is_err());
    }
}

#[test]
#[ignore = "需要 OSR_SAMPLE_DIR；只核验容器解析，不声明其它模式/Mod 的回放支持"]
fn parses_all_official_sample_containers() {
    let root = std::path::PathBuf::from(std::env::var("OSR_SAMPLE_DIR").unwrap());
    let inspection: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("inspection.json")).unwrap()).unwrap();
    for sample in inspection.as_array().unwrap() {
        let file = sample["file"].as_str().unwrap();
        let replay = parse_osr(&std::fs::read(root.join(file)).unwrap()).unwrap();
        assert_eq!(
            u64::from(replay.header.ruleset_id),
            sample["mode"].as_u64().unwrap()
        );
        assert_eq!(
            i64::from(replay.header.version),
            sample["version"].as_i64().unwrap()
        );
        assert_eq!(
            replay.header.beatmap_md5,
            sample["beatmap_md5"].as_str().unwrap()
        );
        assert_eq!(
            replay.frames.len() as u64,
            sample["frame_count"].as_u64().unwrap()
        );
        assert_eq!(
            replay.frames[0].time_ms,
            sample["first_frames"][0][0]
                .as_str()
                .unwrap()
                .parse::<i64>()
                .unwrap(),
        );
    }
}

#[test]
fn stable_taiko_osr_implicitly_uses_classic_through_replay_preparation() {
    use crate::replay::{
        compatibility::{ReplayPolicy, TaikoPolicy},
        preparation::PreparedReplay,
    };

    let beatmap = crate::parse_beatmap_bytes(b"osu file format v14\n[General]\nMode:1\n[Difficulty]\nOverallDifficulty:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n0,0,1000,1,0\n0,0,2000,1,8\n0,0,3000,1,0\n0,0,4000,1,8\n").unwrap();
    for version in [20070916, 20121008, 20140721, 20200101] {
        let mut bytes = fixture_version("0|256|-500|0,1000|0|0|1,1|0|0|0,999|0|0|2,1|0|0|0,999|0|0|4,1|0|0|0,999|0|0|8,200|0|0|0,", version, None);
        bytes[0] = 1;
        let document = std::sync::Arc::new(parse_osr(&bytes).unwrap());
        let prepared = PreparedReplay::prepare(&beatmap, &document).unwrap();
        assert!(prepared.mods().settings.classic);
        let timeline = prepared.compile(document).unwrap();
        assert_eq!(
            timeline.policy,
            ReplayPolicy::Taiko(TaikoPolicy { classic: true })
        );
        let state = timeline.state_at(f64::INFINITY);
        assert_eq!(state.statistics["great"], 4);
        assert_eq!(state.max_combo, 4);
        assert_eq!(state.accuracy, Some(1.0));
        assert_eq!(state.score, 1_000_000);
        assert_eq!(
            state.status,
            crate::gameplay::state::ReplayStatus::Completed
        );
    }
}

#[test]
fn reads_legacy_online_id_widths_and_optional_lazer_metadata() {
    for version in [
        20070916, 20121008, 20140721, 20200101, 30_000_000, 30_000_001, 30_000_019, 40_000_000,
    ] {
        let document = parse_osr(&fixture_version(
            "0|100|100|0,1000|100|100|1,",
            version,
            None,
        ))
        .unwrap();
        assert_eq!(
            document.header.online_id,
            if version < 20121008 { 0 } else { 42 }
        );
        assert!(document.metadata.is_none());
        assert!(crate::replay::compatibility::select(&document).is_ok());
    }
    for metadata in [
        r#"{}"#,
        r#"{"rank":null,"client_version":null}"#,
        r#"{"client_version":"2026.821.0-tachyon-windows","mods":[{"acronym":"CL"}]}"#,
    ] {
        let document = parse_osr(&fixture_version(
            "0|100|100|0,1000|100|100|1,",
            30_000_019,
            Some(metadata),
        ))
        .unwrap();
        assert!(crate::replay::compatibility::select(&document).is_ok());
    }
}

#[test]
fn skips_short_tokens_and_seed_rows_and_ignores_extra_frame_columns() {
    let replay = parse_osr(&fixture(
        ",short,0| 1 | 2 | 33 |ignored,,1|2,-12345|bad|bad|seed,10|3|4|-1,-12345|0|0|7,10|5|6|0,",
    ))
    .unwrap();
    assert_eq!(
        replay
            .frames
            .iter()
            .map(|f| (f.time_ms, f.keys))
            .collect::<Vec<_>>(),
        [(0, 33), (10, u32::MAX), (20, 0)]
    );
    assert_eq!(replay.seed, Some(7));
    for frames in ["", ",,", "0|1|2", "-12345|bad|bad|bad", "0|256|-500|0"] {
        let replay = parse_osr(&fixture(frames)).unwrap();
        assert!(replay.frames.is_empty());
        assert!(crate::replay::compatibility::select(&replay).is_err());
    }
}

#[test]
fn game_numeric_limits_apply_before_mode_specific_button_conversion() {
    for (mode, x, y, keys, valid) in [
        (0, "131072", "-131072", "2147483647", true),
        (0, "131073", "0", "0", false),
        (1, "0", "131073", "0", false),
        (2, "-131072", "0", "-2147483647", true),
        (2, "0", "0", "-2147483648", false),
        (0, "0", "0", "2147483648", false),
        (3, "1048575", "0", "32", true),
        (3, "-1048575", "0", "0", true),
        (3, "1048576", "0", "0", false),
        (3, "1.9", "0", "0", true),
        (0, "Infinity", "0", "0", false),
    ] {
        let mut bytes = fixture(&format!("0|{x}|{y}|{keys}"));
        bytes[0] = mode;
        assert_eq!(parse_osr(&bytes).is_ok(), valid, "{mode}/{x}/{y}/{keys}");
    }
}

fn header_end(bytes: &[u8]) -> usize {
    let mut reader = header::Reader::new(bytes);
    reader.header().unwrap();
    bytes.len() - reader.remaining()
}

#[test]
fn binary_header_uses_signed_score_nonzero_boolean_and_datetime_bounds() {
    let original = fixture("0|1|2|0");
    let end = header_end(&original);
    let score_offset = end - 8 - 2 - 4 - 1 - 2 - 4;
    let bool_offset = end - 8 - 2 - 4 - 1;
    let mut bytes = original.clone();
    bytes[5] = 1; // SerializationReader 接受任意非零字符串标记。
    bytes[score_offset..score_offset + 4].copy_from_slice(&(-42_i32).to_le_bytes());
    bytes[bool_offset] = 255;
    let replay = parse_osr(&bytes).unwrap();
    assert_eq!(replay.header.total_score, -42);
    assert!(replay.header.perfect);
    for (ticks, valid) in [
        (0_i64, true),
        (3_155_378_975_999_999_999, true),
        (-1, false),
        (3_155_378_976_000_000_000, false),
    ] {
        let mut bytes = original.clone();
        bytes[end - 8..end].copy_from_slice(&ticks.to_le_bytes());
        assert_eq!(parse_osr(&bytes).is_ok(), valid, "ticks={ticks}");
    }
}

#[test]
fn life_graph_checks_decimal_fields_but_allows_extra_columns_and_unbounded_health() {
    let original = fixture("0|1|2|0");
    let graph_offset = header_end(&original) - 8 - 2;
    for (graph, valid) in [
        ("", true),
        (" , ", true),
        ("100|2.5|extra,200|-1,", true),
        ("100|0.5,broken", false),
        ("100|NaN", false),
        ("1e3|1", false),
        ("79228162514264337593543950336|1", false),
        ("79228162514264337593543950335.1|1", true),
        ("79228162514264337593543950335.5|1", false),
    ] {
        let mut bytes = original[..graph_offset].to_vec();
        string(&mut bytes, graph);
        bytes.extend_from_slice(&original[graph_offset + 2..]);
        assert_eq!(parse_osr(&bytes).is_ok(), valid, "{graph}");
    }
}

#[test]
fn null_payloads_are_readable_but_required_binary_fields_cannot_be_omitted() {
    let original = fixture("0|1|2|0");
    let end = header_end(&original);
    for length in [-2_i32, -1, 0] {
        let mut bytes = original[..end].to_vec();
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&0_i64.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        let replay = parse_osr(&bytes).unwrap();
        assert!(replay.frames.is_empty() && replay.metadata.is_none());
        assert!(parse_osr(&bytes[..end + 4]).is_err());
    }
    // ReadBytes 在文件末尾返回已有字节；扩展流完整时不因声明长度过长而拒绝。
    let mut bytes = fixture_version("0|1|2|0", 30_000_019, None);
    bytes.truncate(bytes.len() - 4);
    let offset = bytes.len();
    compressed(&mut bytes, "{}");
    bytes[offset..offset + 4].copy_from_slice(&10000_i32.to_le_bytes());
    assert!(parse_osr(&bytes).unwrap().metadata.is_some());
}

#[test]
fn extension_defaults_unknown_fields_and_known_field_types_match_game_schema() {
    for json in [
        "{}",
        r#"{"future":{"data":[1,2]},"rank":null}"#,
        r#"{"rank":"s","statistics":{"Great":-1,"99":2},"pauses":[1,2,"3"]}"#,
        r#"{"online_id":"42","user_id":2,"total_score_without_mods":null}"#,
        r#"{"online_id":true,"total_score_without_mods":2.5}"#,
        r#"{"rank":"A","rank":"S","client_version":123}"#,
        r#"{"mods":[{"acronym":"NF","future_field":{"x":1}}]}"#,
        r#"{"mods":[{}]}"#,
    ] {
        assert!(
            parse_osr(&fixture_version("0|1|2|0", 30_000_019, Some(json))).is_ok(),
            "{json}"
        );
    }
    for json in [
        "null",
        "[]",
        r#"{"rank":"wat"}"#,
        r#"{"pauses":[{}]}"#,
        r#"{"statistics":{"invented":1}}"#,
        r#"{"statistics":{"great":2147483648}}"#,
        r#"{"maximum_statistics":null}"#,
        r#"{"online_id":{}}"#,
        r#"{"user_id":2147483648}"#,
        r#"{"total_score_without_mods":{}}"#,
        r#"{"pauses":[2.5]}"#,
        r#"{"statistics":{"great":2.5}}"#,
        r#"{"client_version":[]}"#,
        r#"{"rank":"bad","rank":"S"}"#,
        r#"{"rank":2.5}"#,
    ] {
        let error = parse_osr(&fixture_version("0|1|2|0", 30_000_019, Some(json)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("extension JSON"), "{json}: {error}");
    }
    let replay = parse_osr(&fixture_version(
        "0|1|2|0",
        30_000_019,
        Some(r#"{"rank":"s","statistics":{"Great":-1},"pauses":[2]}"#),
    ))
    .unwrap();
    let metadata = replay.metadata.unwrap();
    assert_eq!(metadata.rank, "S");
    assert_eq!(metadata.statistics["great"], -1);
    assert_eq!(metadata.pauses, [serde_json::json!(2)]);
    let replay = parse_osr(&fixture_version(
        "0|1|2|0",
        30_000_019,
        Some(r#"{"pauses":[1],"Pauses":[2],"rank":"S, A"}"#),
    ))
    .unwrap();
    let metadata = replay.metadata.unwrap();
    assert_eq!(metadata.pauses, [serde_json::json!(2)]);
    assert_eq!(metadata.rank, "XH");
    for mode in 0..=3 {
        let mut bytes = fixture_version(
            "0|1|2|0",
            30_000_019,
            Some(r#"{"mods":[{"acronym":"NF","future_field":true}]}"#),
        );
        bytes[0] = mode;
        assert!(crate::replay::compatibility::select_replay(&parse_osr(&bytes).unwrap()).is_ok());
    }
}

#[test]
fn stream_reader_bom_and_invalid_utf8_fallback_apply_to_replay_text() {
    let text = "0|1|2|0";
    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(frames::parse(&utf16, 0).unwrap().0.len(), 1);
    assert_eq!(frames::parse(b"\xef\xbb\xbf0|1|2|0", 0).unwrap().0.len(), 1);
    assert_eq!(frames::parse(b"\xff,0|1|2|0", 0).unwrap().0.len(), 1);
}

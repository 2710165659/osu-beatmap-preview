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
    for frames in [
        "",
        "0|1|2",
        "0|NaN|2|0",
        "0|1|2|0,,",
        "0|1|2|0,1|1|2|0|extra",
    ] {
        assert!(parse_osr(&fixture(frames)).is_err(), "{frames}");
    }
    assert!(decode::decompress(&[255; 20]).is_err());
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(parse_osr(&bad).is_err());
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
fn parses_all_official_sample_containers_and_rejects_unregistered_variants() {
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
        let supported = crate::replay::compatibility::select(&replay).is_ok();
        assert_eq!(supported, file == "xuan_yuan-osu-6086051920.osr");
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

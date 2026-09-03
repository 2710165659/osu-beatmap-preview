#![cfg(test)]

//! 外部谱面 fixture 的 Standard 转谱回归测试。

use super::{catch, mania, taiko};
use crate::core::models::{Beatmap, HitObjects};
use crate::core::mods::{mods_for_mode, parse_mods, ModSettings};
use std::path::{Path, PathBuf};

struct ConversionCase {
    name: &'static str,
    osu_file: &'static str,
    golden_file: &'static str,
    target_mode: i32,
    mod_tokens: &'static [&'static str],
}

const CASES: &[ConversionCase] = &[
    ConversionCase {
        name: "1946909_taiko",
        osu_file: "1946909.osu",
        golden_file: "1946909_taiko.golden",
        target_mode: 1,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "1946909_mania_4k",
        osu_file: "1946909.osu",
        golden_file: "1946909_mania_4k.golden",
        target_mode: 3,
        mod_tokens: &["4K"],
    },
    ConversionCase {
        name: "1946909_mania_7k",
        osu_file: "1946909.osu",
        golden_file: "1946909_mania_7k.golden",
        target_mode: 3,
        mod_tokens: &["7K"],
    },
    ConversionCase {
        name: "2374098_catch",
        osu_file: "2374098.osu",
        golden_file: "2374098_catch.golden",
        target_mode: 2,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "2374098_mania_5k_ds",
        osu_file: "2374098.osu",
        golden_file: "2374098_mania_5k_ds.golden",
        target_mode: 3,
        mod_tokens: &["5K", "DS"],
    },
    ConversionCase {
        name: "2374098_mania_6k_ds",
        osu_file: "2374098.osu",
        golden_file: "2374098_mania_6k_ds.golden",
        target_mode: 3,
        mod_tokens: &["6K", "DS"],
    },
    ConversionCase {
        name: "1024742_catch",
        osu_file: "1024742.osu",
        golden_file: "1024742_catch.golden",
        target_mode: 2,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "1024742_taiko",
        osu_file: "1024742.osu",
        golden_file: "1024742_taiko.golden",
        target_mode: 1,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "1024742_mania_default",
        osu_file: "1024742.osu",
        golden_file: "1024742_mania_default.golden",
        target_mode: 3,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "5051189_catch",
        osu_file: "5051189.osu",
        golden_file: "5051189_catch.golden",
        target_mode: 2,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "5051189_taiko",
        osu_file: "5051189.osu",
        golden_file: "5051189_taiko.golden",
        target_mode: 1,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "5051189_mania_default",
        osu_file: "5051189.osu",
        golden_file: "5051189_mania_default.golden",
        target_mode: 3,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "5051189_mania_default_ds",
        osu_file: "5051189.osu",
        golden_file: "5051189_mania_default_ds.golden",
        target_mode: 3,
        mod_tokens: &["DS"],
    },
    ConversionCase {
        name: "260177_taiko",
        osu_file: "260177.osu",
        golden_file: "260177_taiko.golden",
        target_mode: 1,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "260177_catch",
        osu_file: "260177.osu",
        golden_file: "260177_catch.golden",
        target_mode: 2,
        mod_tokens: &[],
    },
    ConversionCase {
        name: "260177_mania_default",
        osu_file: "260177.osu",
        golden_file: "260177_mania_default.golden",
        target_mode: 3,
        mod_tokens: &[],
    },
];

macro_rules! conversion_test {
    ($name:ident, $case:expr) => {
        #[test]
        fn $name() {
            assert_conversion_case($case);
        }
    };
}

conversion_test!(conversion_1946909_taiko, &CASES[0]);
conversion_test!(conversion_1946909_mania_4k, &CASES[1]);
conversion_test!(conversion_1946909_mania_7k, &CASES[2]);
conversion_test!(conversion_2374098_catch, &CASES[3]);
conversion_test!(conversion_2374098_mania_5k_ds, &CASES[4]);
conversion_test!(conversion_2374098_mania_6k_ds, &CASES[5]);
conversion_test!(conversion_1024742_catch, &CASES[6]);
conversion_test!(conversion_1024742_taiko, &CASES[7]);
conversion_test!(conversion_1024742_mania_default, &CASES[8]);
conversion_test!(conversion_5051189_catch, &CASES[9]);
conversion_test!(conversion_5051189_taiko, &CASES[10]);
conversion_test!(conversion_5051189_mania_default, &CASES[11]);
conversion_test!(conversion_5051189_mania_default_ds, &CASES[12]);
conversion_test!(conversion_260177_taiko, &CASES[13]);
conversion_test!(conversion_260177_catch, &CASES[14]);
conversion_test!(conversion_260177_mania_default, &CASES[15]);

fn assert_conversion_case(case: &ConversionCase) {
    let osu_path = fixture_path(case.osu_file);
    let golden_path = fixture_path(case.golden_file);
    let osu = read_fixture(&osu_path);
    let beatmap = crate::parser::parse_beatmap_str_for_tests(&osu).unwrap_or_else(|| {
        panic!(
            "failed to parse .osu fixture for {}: {}",
            case.name,
            osu_path.display()
        )
    });
    let mods = test_mods(case.mod_tokens, case.target_mode);
    let converted = convert(&beatmap, case.target_mode, mods.as_ref())
        .unwrap_or_else(|error| panic!("conversion failed for {}: {error}", case.name));
    let expected = read_fixture(&golden_path);
    let actual = snapshot(&converted);
    assert_eq!(actual, expected, "golden mismatch for {}", case.name);
}

fn convert(
    beatmap: &Beatmap,
    target_mode: i32,
    mods: Option<&ModSettings>,
) -> crate::core::errors::Result<Beatmap> {
    match target_mode {
        1 => taiko::conv::taiko_convert(beatmap, target_mode, mods),
        2 => catch::conv::catch_convert(beatmap, target_mode, mods),
        3 => mania::conv::mania_convert(beatmap, target_mode, mods),
        _ => panic!("unsupported test target mode: {target_mode}"),
    }
}

fn test_mods(tokens: &[&str], target_mode: i32) -> Option<ModSettings> {
    if tokens.is_empty() {
        return None;
    }
    let tokens: Vec<String> = tokens.iter().map(|token| (*token).to_string()).collect();
    let settings = parse_mods(&tokens).expect("test mod tokens must be valid");
    Some(mods_for_mode(&settings, target_mode))
}

fn snapshot(beatmap: &Beatmap) -> String {
    let mut output = String::new();
    output.push_str(&format!("mode={}\n", beatmap.mode()));
    output.push_str(&format!(
        "circle_size={}\n",
        beatmap.difficulty.get("CircleSize").unwrap_or("")
    ));
    output.push_str(&format!(
        "timing_points_count={}\n",
        beatmap.timing_points.len()
    ));
    output.push_str("timing_points:\n");
    for point in &beatmap.timing_points {
        output.push_str(&format!("{point:?}\n"));
    }
    output.push_str(&format!(
        "hit_objects_count={}\n",
        beatmap.hit_objects.len()
    ));
    output.push_str("hit_objects:\n");
    match &beatmap.hit_objects {
        HitObjects::Taiko(objects) => {
            for object in objects {
                output.push_str(&format!("{object:?}\n"));
            }
        }
        HitObjects::Catch(objects) => {
            for object in objects {
                output.push_str(&format!("{object:?}\n"));
            }
        }
        HitObjects::Mania(objects) => {
            for object in objects {
                output.push_str(&format!("{object:?}\n"));
            }
        }
        HitObjects::Standard(_) => panic!("conversion result is still Standard"),
    }
    output
}

fn fixture_path(filename: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("testdata_conversion")
        .join(filename)
}

fn read_fixture(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()))
}

mod canvas;
mod catch;
mod composer;
mod convert;
mod digits;
mod downloader;
mod errors;
mod legacy_random;
mod mania;
mod models;
mod mods;
mod parser;
mod service;
mod skin;
mod slider_path;
mod standard;
mod taiko;
mod text;
mod time_selection;
mod validate;

mod utils;

use errors::Result;

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Args {
    bid: String,
    convert: Option<String>,
    mods: Option<String>,
    fmt: Option<String>,
    time: Option<String>,
    bpm: Option<f64>,
    no_cache: bool,
}

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!(
        "usage: osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko] \
         [--mods=<MODS>] [--fmt=png|gif] [--time=<T1+T2+...>] [--bpm=<BPM>] [--no-cache]\
       osu-beatmap-preview --version"
    );
    std::process::exit(code)
}

fn parse_args() -> Args {
    let mut bid: Option<String> = None;
    let mut convert: Option<String> = None;
    let mut mods: Option<String> = None;
    let mut fmt: Option<String> = None;
    let mut time: Option<String> = None;
    let mut bpm: Option<f64> = None;

    let mut no_cache: bool = false;

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        let (key, value) = if let Some((k, v)) = arg.split_once('=') {
            (k.to_string(), Some(v.to_string()))
        } else {
            (arg.clone(), None)
        };
        let mut take_value = |inline: Option<String>| -> String {
            if let Some(v) = inline {
                v
            } else {
                i += 1;
                if i >= argv.len() {
                    eprintln!("error: missing value for {key}");
                    print_usage_and_exit(2)
                }
                argv[i].clone()
            }
        };
        match key.as_str() {
            "--bid" => bid = Some(take_value(value)),
            "--convert" => {
                let v = take_value(value);
                if let Err(e) = validate::validate_convert_value(&v) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2)
                }
                convert = Some(v);
            }
            "--mod" | "--mods" => mods = Some(take_value(value)),
            "--fmt" | "--format" => {
                let v = take_value(value);
                if let Err(e) = validate::validate_fmt_value(&v) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2)
                }
                fmt = Some(v);
            }
            "--time" | "--times" => time = Some(take_value(value)),
            "--bpm" => {
                let v = take_value(value);
                let val: f64 = v
                    .parse()
                    .map_err(|_| {
                        eprintln!("error: --bpm must be a number, got '{v}'");
                        print_usage_and_exit(2)
                    })
                    .unwrap();
                if let Err(e) = validate::validate_bpm_value(val) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2)
                }
                bpm = Some(val);
            }
            "--no-cache" => {
                let v = value.as_deref().unwrap_or("true");
                no_cache = v == "true" || v == "1";
            }
            "--version" => {
                println!("osu-beatmap-preview v{} (built {})", VERSION, BUILD_TIMESTAMP);
                std::process::exit(0)
            }
            _ => {
                eprintln!("error: unknown argument: {arg}");
                print_usage_and_exit(2)
            }
        }
        i += 1;
    }

    let Some(bid) = bid else {
        eprintln!("error: --bid is required");
        print_usage_and_exit(2)
    };
    Args {
        bid,
        convert,
        mods,
        fmt,
        time,
        bpm,
        no_cache,
    }
}

fn run(args: &Args) -> Result<serde_json::Value> {
    let mods_unvalidated = match &args.mods {
        Some(mod_str) => Some(mods::parse_mods(mod_str)?),
        None => None,
    };

    let times = match &args.time {
        Some(raw) => Some(validate::parse_times(raw)?),
        None => None,
    };

    service::generate_preview(
        &args.bid,
        args.fmt.as_deref(),
        args.convert.as_deref(),
        mods_unvalidated,
        times,
        args.bpm,
        args.no_cache,
    )
}

fn build_info() -> serde_json::Value {
    serde_json::json!({
        "version": VERSION,
        "build_time": BUILD_TIMESTAMP
    })
}

fn dump_convert_signatures(osu_path: &str) {
    let beatmap = parser::parse_beatmap(std::path::Path::new(osu_path)).unwrap();
    for target in [1, 2, 3] {
        let conv = convert::convert_beatmap(&beatmap, target, None).unwrap();
        let mut lines = Vec::new();
        match &conv.hit_objects {
            models::HitObjects::Taiko(v) => {
                for o in v.iter().take(2000) {
                    lines.push(format!("[{}, {}, {}, {}]", o.start_time, o.end_time, o.hit_type, o.hitsound));
                }
            }
            models::HitObjects::Catch(v) => {
                for o in v.iter().take(2000) {
                    lines.push(format!("[{}, {}, {}, {}, {}]", o.x, o.y, o.start_time, o.end_time, o.hit_type));
                }
            }
            models::HitObjects::Mania(v) => {
                for o in v.iter().take(2000) {
                    lines.push(format!("[{}, {}, {}, {}]", o.lane, o.start_time, o.end_time, if o.is_long_note { "true" } else { "false" }));
                }
            }
            _ => {}
        }
        println!("{} {} [{}]", target, conv.hit_objects.len(), lines.join(", "));
    }
}

fn main() {
    if let Ok(path) = std::env::var("OSU_PREVIEW_DUMP_CONVERT") {
        dump_convert_signatures(&path);
        return;
    }
    let args = parse_args();
    match run(&args) {
        Ok(mut result) => {
            if let Some(obj) = result.as_object_mut() {
                obj.insert("build-info".to_string(), build_info());
            }
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        Err(exc) => {
            let payload = serde_json::json!({
                "status": "error",
                "msg": exc.to_string(),
                "preview-img": "",
                "beatmap-info": {},
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            std::process::exit(1);
        }
    }
}

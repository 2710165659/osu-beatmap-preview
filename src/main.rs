mod build_time;
mod cache;
mod canvas;
mod catch;
mod common;
mod composer;
mod downloader;
mod errors;
mod mania;
mod models;
mod mods;
mod parser;
mod service;
mod skin;
mod standard;
mod taiko;
mod text;
mod validate;
mod video;

use errors::Result;
use lexopt::prelude::*;

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
         [--mods=<MODS>] [--fmt=png|gif|mp4] [--time=<T1+T2+...>] [--bpm=<BPM>] [--no-cache]\
       osu-beatmap-preview --version"
    );
    std::process::exit(code)
}

fn parse_args() -> Args {
    let mut parser = lexopt::Parser::from_env();
    let mut bid: Option<String> = None;
    let mut convert: Option<String> = None;
    let mut mods: Option<String> = None;
    let mut fmt: Option<String> = None;
    let mut time: Option<String> = None;
    let mut bpm: Option<f64> = None;
    let mut no_cache: bool = false;

    while let Some(arg) = parser.next().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        print_usage_and_exit(2);
    }) {
        match arg {
            Long("bid") => {
                bid = Some(take_value(&mut parser, "--bid"));
            }
            Long("convert") => {
                let v = take_value(&mut parser, "--convert");
                if let Err(e) = validate::validate_convert_value(&v) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2);
                }
                convert = Some(v);
            }
            Long("mod") | Long("mods") => {
                mods = Some(take_value(&mut parser, "--mods"));
            }
            Long("fmt") | Long("format") => {
                let v = take_value(&mut parser, "--fmt");
                if let Err(e) = validate::validate_fmt_value(&v) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2);
                }
                fmt = Some(v);
            }
            Long("time") | Long("times") => {
                time = Some(take_value(&mut parser, "--time"));
            }
            Long("bpm") => {
                let v = take_value(&mut parser, "--bpm");
                let val: f64 = v.parse().unwrap_or_else(|_| {
                    eprintln!("error: --bpm must be a number, got '{v}'");
                    print_usage_and_exit(2);
                });
                if let Err(e) = validate::validate_bpm_value(val) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2);
                }
                bpm = Some(val);
            }
            Long("no-cache") => {
                no_cache = true;
            }
            Long("version") => {
                println!("osu-beatmap-preview v{} (built {})", VERSION, BUILD_TIMESTAMP);
                std::process::exit(0);
            }
            Short('h') | Long("help") => {
                print_usage_and_exit(0);
            }
            Short(c) => {
                eprintln!("error: unknown flag: -{c}");
                print_usage_and_exit(2);
            }
            Value(val) => {
                eprintln!(
                    "error: unexpected argument: {}",
                    val.to_string_lossy()
                );
                print_usage_and_exit(2);
            }
            Long(unknown) => {
                eprintln!("error: unknown argument: --{unknown}");
                print_usage_and_exit(2);
            }
        }
    }

    let Some(bid) = bid else {
        eprintln!("error: --bid is required");
        print_usage_and_exit(2);
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

fn take_value(parser: &mut lexopt::Parser, name: &str) -> String {
    parser
        .value()
        .unwrap_or_else(|e| {
            eprintln!("error: {name} requires a value: {e}");
            print_usage_and_exit(2);
        })
        .to_string_lossy()
        .into_owned()
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

fn main() {
    let args = parse_args();
    match run(&args) {
        Ok(mut result) => {
            if let Some(obj) = result.as_object_mut() {
                obj.insert("build-info".to_string(), build_info());
            }
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        Err(exc) => {
            let kind_label = match exc.kind() {
                errors::ErrorKind::Download => "download error",
                errors::ErrorKind::Parse => "parse error",
                errors::ErrorKind::Render => "render error",
                errors::ErrorKind::Other => "error",
            };
            let msg = format!("{kind_label}: {}", exc);
            let payload = serde_json::json!({
                "status": "error",
                "msg": msg,
                "preview-img": "",
                "beatmap-info": {},
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            std::process::exit(1);
        }
    }
}

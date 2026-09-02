mod common;
mod config;
mod core;
mod log;
mod parser;
mod pipeline;
mod render;

use core::errors::Result;
use lexopt::prelude::*;

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Args {
    bid: String,
    convert: Option<String>,
    mods: Option<core::mods::ModSettings>,
    fmt: Option<String>,
    time_points: Vec<core::validate::TimePoint>,
    duration_time: Option<f64>,
    no_cache: bool,
    no_log: bool,
    config: Option<String>,
    scale: Option<f64>,
}

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!(
        "usage: osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard] \
         [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] \
         [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>] [--scale=<POSITIVE_NUMBER>]\n\
         osu-beatmap-preview --version\n\
         --mod and --time-points may be repeated to provide lists"
    );
    std::process::exit(code)
}

fn parse_args() -> Args {
    let mut parser = lexopt::Parser::from_env();
    let mut bid: Option<String> = None;
    let mut convert: Option<String> = None;
    let mut mods: Vec<String> = Vec::new();
    let mut fmt: Option<String> = None;
    let mut time_points = Vec::new();
    let mut duration_time = None;
    let mut no_cache: bool = false;
    let mut no_log: bool = false;
    let mut config: Option<String> = None;
    let mut scale: Option<f64> = None;

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
                convert = Some(v);
            }
            Long("mod") => {
                let value = take_value(&mut parser, "--mod");
                mods.push(value);
            }
            Long("fmt") => {
                let v = take_value(&mut parser, "--fmt");
                fmt = Some(v);
            }
            Long("time-points") => {
                let value = take_value(&mut parser, "--time-points");
                time_points.push(
                    core::validate::parse_time_point(&value).unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        print_usage_and_exit(2);
                    }),
                );
            }
            Long("duration-time") => {
                let value = take_value(&mut parser, "--duration-time");
                let parsed = core::validate::parse_positive_finite("--duration-time", &value)
                    .unwrap_or_else(|error| {
                        eprintln!("error: {error}");
                        print_usage_and_exit(2);
                    });
                duration_time = Some(parsed);
            }
            Long("no-cache") => {
                no_cache = true;
            }
            Long("no-log") => {
                no_log = true;
            }
            Long("config") => {
                if config.is_some() {
                    eprintln!("error: --config may only be specified once");
                    print_usage_and_exit(2);
                }
                config = Some(take_value(&mut parser, "--config"));
            }
            Long("scale") => {
                let value = take_value(&mut parser, "--scale");
                let parsed = core::validate::parse_positive_finite("--scale", &value)
                    .unwrap_or_else(|error| {
                        eprintln!("error: {error}");
                        print_usage_and_exit(2);
                    });
                scale = Some(parsed);
            }
            Long("version") => {
                println!(
                    "osu-beatmap-preview v{} (built {})",
                    VERSION, BUILD_TIMESTAMP
                );
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
                eprintln!("error: unexpected argument: {}", val.to_string_lossy());
                print_usage_and_exit(2);
            }
            Long(unknown) => {
                eprintln!("error: unknown argument: --{unknown}");
                print_usage_and_exit(2);
            }
        }
    }

    let bid = match bid {
        Some(bid) => bid,
        None => {
            eprintln!("error: --bid is required");
            print_usage_and_exit(2);
        }
    };
    let mods = core::validate::validate_cli_options(
        &bid,
        convert.as_deref(),
        fmt.as_deref(),
        &mods,
        duration_time,
        scale,
    )
    .unwrap_or_else(|error| {
        eprintln!("error: {error}");
        print_usage_and_exit(2);
    });
    Args {
        bid,
        convert,
        mods,
        fmt,
        time_points,
        duration_time,
        no_cache,
        no_log,
        config,
        scale,
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
    pipeline::service::generate_preview(
        &args.bid,
        args.fmt.as_deref(),
        args.convert.as_deref(),
        args.mods.clone(),
        args.time_points.clone(),
        args.duration_time,
        args.no_cache,
    )
}

fn main() {
    let args = parse_args();
    if let Err(error) = config::initialize_for_cli(args.config.as_deref(), args.scale) {
        let payload = serde_json::json!({
            "status": "error",
            "msg": format!("configuration error: {error}"),
            "preview-img": "",
            "beatmap-info": {},
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        std::process::exit(1);
    }
    if !args.no_log {
        log::config::init();
    }
    match run(&args) {
        Ok(result) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        Err(exc) => {
            let msg = match exc.kind() {
                core::errors::ErrorKind::Other => format!("error: {exc}"),
                _ => exc.to_string(),
            };
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

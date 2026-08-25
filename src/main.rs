mod common;
mod config;
mod core;
mod log;
mod parser;
mod pipeline;
mod render;

use core::errors::Result;
use lexopt::prelude::*;
use std::path::PathBuf;

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Args {
    bid: String,
    convert: Option<String>,
    mods: Option<String>,
    fmt: Option<String>,
    time: Option<String>,
    gif_clip: bool,
    gif_clip_label: bool,
    preview_30s: bool,
    gap: Option<f64>,
    no_cache: bool,
    log_dir: Option<PathBuf>,
    no_log: bool,
}

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!(
        "usage: osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko] \
         [--mods=<MODS>] [--fmt=png|gif|mp4] [--time=<T1+T2+...>] [--gif-clip] [--gif-clip-label] [--preview-30s] [--gap=<BPM>] \
         [--log-dir=<DIR>] [--no-log] [--no-cache]\n\
         osu-beatmap-preview --version\n\
         --time uses first-object-relative gameplay time and accepts negative values"
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
    let mut gif_clip: bool = false;
    let mut gif_clip_label: bool = false;
    let mut preview_30s: bool = false;
    let mut gap: Option<f64> = None;
    let mut no_cache: bool = false;
    let mut log_dir: Option<PathBuf> = None;
    let mut no_log: bool = false;

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
                if let Err(e) = core::validate::validate_convert_value(&v) {
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
                if let Err(e) = core::validate::validate_fmt_value(&v) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2);
                }
                fmt = Some(v);
            }
            Long("time") | Long("times") => {
                time = Some(take_value(&mut parser, "--time"));
            }
            Long("gif-clip") => {
                gif_clip = true;
            }
            Long("gif-clip-label") => {
                gif_clip_label = true;
            }
            Long("preview-30s") => {
                preview_30s = true;
            }
            Long("gap") => {
                let v = take_value(&mut parser, "--gap");
                let val: f64 = v.parse().unwrap_or_else(|_| {
                    eprintln!("error: --gap must be a number, got '{v}'");
                    print_usage_and_exit(2);
                });
                if let Err(e) = core::validate::validate_gap_value(val) {
                    eprintln!("error: {e}");
                    print_usage_and_exit(2);
                }
                gap = Some(val);
            }
            Long("no-cache") => {
                no_cache = true;
            }
            Long("log-dir") => {
                log_dir = Some(PathBuf::from(take_value(&mut parser, "--log-dir")));
            }
            Long("no-log") => {
                no_log = true;
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
        gif_clip,
        gif_clip_label,
        preview_30s,
        gap,
        no_cache,
        log_dir,
        no_log,
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
        Some(mod_str) => Some(core::mods::parse_mods(mod_str)?),
        None => None,
    };

    let times = match &args.time {
        Some(raw) => Some(core::validate::parse_times(raw)?),
        None => None,
    };

    pipeline::service::generate_preview(
        &args.bid,
        args.fmt.as_deref(),
        args.convert.as_deref(),
        mods_unvalidated,
        times,
        args.gif_clip,
        args.gif_clip_label,
        args.preview_30s,
        args.gap,
        args.no_cache,
    )
}

fn build_info() -> serde_json::Value {
    serde_json::json!({
        "version": VERSION,
        "build_time": BUILD_TIMESTAMP
    })
}

/// 把日志文件路径附加到 stdout 的 JSON 结果中（可选字段，不影响现有解析）。
fn attach_log_info(mut result: serde_json::Value) -> serde_json::Value {
    if let Some((progress, render)) = log::paths() {
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "log".to_string(),
                serde_json::json!({
                    "progress": progress.to_string_lossy(),
                    "render": render.to_string_lossy(),
                }),
            );
        }
    }
    result
}

/// `OSU_PREVIEW_LOG_TEST=<N>`：测试钩子，只写 N 条进度事件 + 1 条汇总后退出，
/// 供多进程并发集成测试使用（不渲染谱面、不访问网络）。
fn log_test_lines() -> Option<usize> {
    std::env::var("OSU_PREVIEW_LOG_TEST")
        .ok()
        .and_then(|value| value.parse().ok())
}

fn run_log_test(lines: usize) {
    let dir = std::env::var("OSU_PREVIEW_LOG_DIR").ok().map(PathBuf::from);
    log::init(dir.as_deref());
    for i in 0..lines {
        log::event("test", "info", Some("test-bid"), &format!("test line {i}"));
    }
    let mut rec = log::SummaryRecord::default();
    rec.status = "success".to_string();
    rec.bid = "test-bid".to_string();
    rec.duration_ms = 1.0;
    rec.fmt = Some("png".to_string());
    log::write_summary(&rec);
    std::process::exit(0);
}

fn main() {
    if let Some(lines) = log_test_lines() {
        run_log_test(lines);
        return;
    }
    let args = parse_args();
    if !args.no_log {
        log::init(args.log_dir.as_deref());
    }
    match run(&args) {
        Ok(mut result) => {
            if let Some(obj) = result.as_object_mut() {
                obj.insert("build-info".to_string(), build_info());
            }
            result = attach_log_info(result);
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
            let payload = attach_log_info(payload);
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            std::process::exit(1);
        }
    }
}

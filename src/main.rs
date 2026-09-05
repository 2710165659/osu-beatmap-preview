//! 命令行适配器：只负责词法解析，校验与业务编排统一交给库入口。

use lexopt::prelude::*;
use osu_beatmap_preview::{
    generate_preview, parse_fps, parse_positive_finite, parse_time_point, ErrorKind, RenderRequest,
};

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!(
        "usage: osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard] \
         [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] \
         [--fps=<1-60>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>] [--scale=<POSITIVE_NUMBER>] \
         [--output-dir=<DIR>]\n\
         osu-beatmap-preview --version\n\
         --mod and --time-points may be repeated to provide lists"
    );
    std::process::exit(code)
}

fn parse_args() -> RenderRequest {
    let mut parser = lexopt::Parser::from_env();
    let mut bid = None;
    let mut request = RenderRequest::new("");

    while let Some(argument) = parser.next().unwrap_or_else(|error| {
        eprintln!("error: {error}");
        print_usage_and_exit(2);
    }) {
        match argument {
            Long("bid") => bid = Some(take_value(&mut parser, "--bid")),
            Long("convert") => {
                request.ruleset.convert = Some(take_value(&mut parser, "--convert"));
            }
            Long("mod") => request.ruleset.mods.push(take_value(&mut parser, "--mod")),
            Long("fmt") => request.output.format = Some(take_value(&mut parser, "--fmt")),
            Long("time-points") => {
                let value = take_value(&mut parser, "--time-points");
                request.view.time_points.push(
                    parse_time_point(&value)
                        .unwrap_or_else(|error| exit_with_argument_error(error)),
                );
            }
            Long("duration-time") => {
                let value = take_value(&mut parser, "--duration-time");
                request.view.duration_seconds = Some(
                    parse_positive_finite("--duration-time", &value)
                        .unwrap_or_else(|error| exit_with_argument_error(error)),
                );
            }
            Long("no-cache") => request.execution.no_cache = true,
            Long("fps") => {
                let value = take_value(&mut parser, "--fps");
                request.output.fps =
                    Some(parse_fps(&value).unwrap_or_else(|error| exit_with_argument_error(error)));
            }
            Long("no-log") => request.execution.logging = false,
            Long("config") => {
                if request.execution.config.is_some() {
                    eprintln!("error: --config may only be specified once");
                    print_usage_and_exit(2);
                }
                request.execution.config = Some(take_value(&mut parser, "--config"));
            }
            Long("scale") => {
                let value = take_value(&mut parser, "--scale");
                request.output.scale = Some(
                    parse_positive_finite("--scale", &value)
                        .unwrap_or_else(|error| exit_with_argument_error(error)),
                );
            }
            Long("output-dir") => {
                if request.output.output_dir.is_some() {
                    eprintln!("error: --output-dir may only be specified once");
                    print_usage_and_exit(2);
                }
                request.output.output_dir = Some(take_value(&mut parser, "--output-dir"));
            }
            Long("version") => {
                println!("osu-beatmap-preview v{VERSION} (built {BUILD_TIMESTAMP})");
                std::process::exit(0);
            }
            Short('h') | Long("help") => print_usage_and_exit(0),
            Short(flag) => {
                eprintln!("error: unknown flag: -{flag}");
                print_usage_and_exit(2);
            }
            Value(value) => {
                eprintln!("error: unexpected argument: {}", value.to_string_lossy());
                print_usage_and_exit(2);
            }
            Long(unknown) => {
                eprintln!("error: unknown argument: --{unknown}");
                print_usage_and_exit(2);
            }
        }
    }

    request.source.bid = bid.unwrap_or_else(|| {
        eprintln!("error: --bid is required");
        print_usage_and_exit(2);
    });
    request
}

fn take_value(parser: &mut lexopt::Parser, name: &str) -> String {
    parser
        .value()
        .unwrap_or_else(|error| {
            eprintln!("error: {name} requires a value: {error}");
            print_usage_and_exit(2);
        })
        .to_string_lossy()
        .into_owned()
}

fn exit_with_argument_error(error: osu_beatmap_preview::PreviewError) -> ! {
    eprintln!("error: {error}");
    print_usage_and_exit(2)
}

fn main() {
    match generate_preview(parse_args()) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            let message = match error.kind() {
                ErrorKind::Other => format!("error: {error}"),
                _ => error.to_string(),
            };
            let payload = serde_json::json!({
                "status": "error",
                "msg": message,
                "preview-img": "",
                "beatmap-info": {},
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            std::process::exit(1);
        }
    }
}

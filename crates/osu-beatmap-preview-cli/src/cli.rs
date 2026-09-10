//! 命令行参数解析与进程入口。

use crate::{
    parse_fps, parse_positive_finite, ExecutionOptions, OutputOptions, RenderRequest,
    RulesetOptions, SourceOptions, ViewOptions,
};
use lexopt::prelude::*;
use osu_beatmap_preview_core::processing::validation::parse_time_point;

pub const USAGE: &str = "usage: osu-beatmap-preview --bid=<BID> [--convert=<MODE>] [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] [--fps=<1-60>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>] [--scale=<POSITIVE_NUMBER>] [--output-dir=<DIR>]";

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum CliAction {
    Render(Box<RenderRequest>),
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(String);
impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for CliError {}

pub fn parse_env() -> Result<CliAction, CliError> {
    parse_args(std::env::args_os().skip(1))
}

pub fn parse_args(
    args: impl IntoIterator<Item = impl Into<std::ffi::OsString>>,
) -> Result<CliAction, CliError> {
    let mut parser = lexopt::Parser::from_args(args.into_iter().map(Into::into));
    let mut request = RenderRequest {
        source: SourceOptions { bid: String::new() },
        ruleset: RulesetOptions::default(),
        view: ViewOptions::default(),
        output: OutputOptions::default(),
        execution: ExecutionOptions::default(),
    };
    let mut bid = None;
    while let Some(argument) = parser.next().map_err(argument_error)? {
        match argument {
            Long("bid") => bid = Some(value(&mut parser, "--bid")?),
            Long("convert") => request.ruleset.convert = Some(value(&mut parser, "--convert")?),
            Long("mod") => request.ruleset.mods.push(value(&mut parser, "--mod")?),
            Long("fmt") => request.output.format = Some(value(&mut parser, "--fmt")?),
            Long("time-points") => {
                let point = value(&mut parser, "--time-points")?;
                request
                    .view
                    .time_points
                    .push(parse_time_point(&point).map_err(argument_error)?);
            }
            Long("duration-time") => {
                request.view.duration_seconds = Some(
                    parse_positive_finite(
                        "--duration-time",
                        &value(&mut parser, "--duration-time")?,
                    )
                    .map_err(argument_error)?,
                );
            }
            Long("fps") => {
                request.output.fps =
                    Some(parse_fps(&value(&mut parser, "--fps")?).map_err(argument_error)?)
            }
            Long("scale") => {
                request.output.scale = Some(
                    parse_positive_finite("--scale", &value(&mut parser, "--scale")?)
                        .map_err(argument_error)?,
                )
            }
            Long("output-dir") => {
                request.output.output_dir = Some(value(&mut parser, "--output-dir")?)
            }
            Long("no-log") => request.execution.logging = false,
            Long("no-cache") => request.execution.no_cache = true,
            Long("config") => request.execution.config = Some(value(&mut parser, "--config")?),
            Long("version") => return Ok(CliAction::Version),
            Short('h') | Long("help") => return Ok(CliAction::Help),
            Short(flag) => return Err(CliError(format!("unknown flag: -{flag}"))),
            Long(name) => return Err(CliError(format!("unknown argument: --{name}"))),
            Value(value) => {
                return Err(CliError(format!(
                    "unexpected argument: {}",
                    value.to_string_lossy()
                )))
            }
        }
    }
    request.source.bid = bid.ok_or_else(|| CliError("--bid is required".into()))?;
    Ok(CliAction::Render(Box::new(request)))
}

pub fn run(action: CliAction) -> i32 {
    match action {
        CliAction::Help => {
            println!("{USAGE}");
            0
        }
        CliAction::Version => {
            println!("osu-beatmap-preview v{}", env!("CARGO_PKG_VERSION"));
            0
        }
        CliAction::Render(request) => {
            let result = crate::generate_preview(*request);
            match result {
                Ok(value) => {
                    println!("{}", serde_json::to_string_pretty(&value).unwrap());
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
    }
}

fn value(parser: &mut lexopt::Parser, name: &str) -> Result<String, CliError> {
    parser
        .value()
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| CliError(format!("{name} requires a value: {error}")))
}
fn argument_error(error: impl std::fmt::Display) -> CliError {
    CliError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgpu参数不再属于cli() {
        let error = parse_args(["--bid=123", "--wgpu"]).expect_err("WGPU 参数必须被拒绝");
        assert_eq!(error.to_string(), "unknown argument: --wgpu");
    }
}

//! 两个命令行程序共用的纯词法适配层。

use std::ffi::OsString;

use lexopt::prelude::*;

use crate::{parse_fps, parse_positive_finite, parse_time_point, RenderRequest};

pub const USAGE: &str = "usage: osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard] [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] [--fps=<1-60>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>] [--scale=<POSITIVE_NUMBER>] [--output-dir=<DIR>]\n       osu-beatmap-preview --version\n       --mod and --time-points may be repeated to provide lists";

#[derive(Debug, Clone)]
pub enum CliAction {
    Render(RenderRequest),
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
    args: impl IntoIterator<Item = impl Into<OsString>>,
) -> Result<CliAction, CliError> {
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let mut parser = lexopt::Parser::from_args(args);
    let mut bid = None;
    let mut request = RenderRequest::new("");

    while let Some(argument) = parser.next().map_err(argument_error)? {
        match argument {
            Long("bid") => bid = Some(take_value(&mut parser, "--bid")?),
            Long("convert") => {
                request.ruleset.convert = Some(take_value(&mut parser, "--convert")?)
            }
            Long("mod") => request.ruleset.mods.push(take_value(&mut parser, "--mod")?),
            Long("fmt") => request.output.format = Some(take_value(&mut parser, "--fmt")?),
            Long("time-points") => {
                let value = take_value(&mut parser, "--time-points")?;
                request
                    .view
                    .time_points
                    .push(parse_time_point(&value).map_err(argument_error)?);
            }
            Long("duration-time") => {
                let value = take_value(&mut parser, "--duration-time")?;
                request.view.duration_seconds =
                    Some(parse_positive_finite("--duration-time", &value).map_err(argument_error)?);
            }
            Long("no-cache") => request.execution.no_cache = true,
            Long("fps") => {
                let value = take_value(&mut parser, "--fps")?;
                request.output.fps = Some(parse_fps(&value).map_err(argument_error)?);
            }
            Long("no-log") => request.execution.logging = false,
            Long("config") => {
                if request.execution.config.is_some() {
                    return Err(CliError("--config may only be specified once".to_string()));
                }
                request.execution.config = Some(take_value(&mut parser, "--config")?);
            }
            Long("scale") => {
                let value = take_value(&mut parser, "--scale")?;
                request.output.scale =
                    Some(parse_positive_finite("--scale", &value).map_err(argument_error)?);
            }
            Long("output-dir") => {
                if request.output.output_dir.is_some() {
                    return Err(CliError(
                        "--output-dir may only be specified once".to_string(),
                    ));
                }
                request.output.output_dir = Some(take_value(&mut parser, "--output-dir")?);
            }
            Long("version") => return Ok(CliAction::Version),
            Short('h') | Long("help") => return Ok(CliAction::Help),
            Short(flag) => return Err(CliError(format!("unknown flag: -{flag}"))),
            Value(value) => {
                return Err(CliError(format!(
                    "unexpected argument: {}",
                    value.to_string_lossy()
                )))
            }
            Long(unknown) => return Err(CliError(format!("unknown argument: --{unknown}"))),
        }
    }

    request.source.bid = bid.ok_or_else(|| CliError("--bid is required".to_string()))?;
    Ok(CliAction::Render(request))
}

fn take_value(parser: &mut lexopt::Parser, name: &str) -> Result<String, CliError> {
    parser
        .value()
        .map_err(|error| CliError(format!("{name} requires a value: {error}")))
        .map(|value| value.to_string_lossy().into_owned())
}

fn argument_error(error: impl std::fmt::Display) -> CliError {
    CliError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 两个命令行入口可复用同一参数规则() {
        let CliAction::Render(request) = parse_args([
            "--bid=738063",
            "--fmt=mp4",
            "--mod=HD",
            "--fps=30",
            "--scale=1.5",
        ])
        .unwrap() else {
            panic!("应解析为渲染请求");
        };
        assert_eq!(request.source.bid, "738063");
        assert_eq!(request.output.format.as_deref(), Some("mp4"));
        assert_eq!(request.ruleset.mods, ["HD"]);
        assert_eq!(request.output.fps, Some(30));
        assert_eq!(request.output.scale, Some(1.5));
    }

    #[test]
    fn 重复的单值参数会被拒绝() {
        let error =
            parse_args(["--bid=1", "--config={}", "--config={}"]).expect_err("重复配置必须失败");
        assert!(error.to_string().contains("only be specified once"));
    }
}

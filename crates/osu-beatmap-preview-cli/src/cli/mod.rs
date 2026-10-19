//! 命令行参数解析与进程入口。

use crate::{
    parse_fps, parse_positive_finite, ExecutionOptions, OutputOptions, RenderRequest,
    RulesetOptions, SourceOptions, ViewOptions,
};
use lexopt::prelude::*;
mod osr;
mod osu;
use osu_beatmap_preview_core::gameplay::GameplayOptions;
use osu_beatmap_preview_core::processing::validation::parse_time_point;

pub const USAGE: &str = "usage: osu-beatmap-preview-cli [--type=osu|osr] [--osr-file=<PATH> | --score-id=<ID> | --user=<NAME> [--bid=<BID>] [--mode=osu|taiko|fruits|mania] [--score-select=best|recent] [--include-fails]] [--bid=<BID>] [--osu-file=<PATH>] [--osz-file=<PATH>] [--convert=<MODE>] [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] [--fps=<1-60>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>] [--scale=<POSITIVE_NUMBER>] [--output-dir=<DIR>] [--version] [--help]\n\
    \x20 --type=osr --osr-file=<FILE.osr>  原生 Standard 回放；默认 MP4，也支持 PNG/GIF；禁止 --convert 和 --mod\n\
    \x20 OSR 来源三选一：--osr-file、--score-id、--user；带 --bid 默认 best，不带 --bid 默认 recent\n\
    \x20 玩家最近成绩：--user=<NAME> [--mode=osu|taiko|fruits|mania]；省略模式使用玩家默认模式，--include-fails 开启失败成绩（默认排除）\n\
    \x20 OSR 可选 --osu-file/--osz-file，按原始 MD5 绑定；缺少资源时查询官方 API 并下载，失败报错\n\
    \x20 以下谱面来源规则用于默认 --type=osu：\n\
    \x20 --osu-file=<FILE.osu>  本地谱面；--bid 可省略，提供时必须与 BeatmapID 相同；单独使用只支持 PNG/GIF\n\
    \x20 --osz-file=<FILE.osz>  本地谱面包；只有一个顶层 .osu 时可省略 --bid；支持 PNG/GIF/MP4\n\
    \x20 同时提供两个文件时，以外部 .osu 渲染，.osz 提供媒体资源，包内必须存在相同 BeatmapID\n\
    \x20 未提供本地文件时必须提供 --bid，并按 ID 下载谱面";

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
        kind: crate::RequestKind::Osu,
        source: SourceOptions {
            bid: String::new(),
            osu_file: None,
            osz_file: None,
        },
        ruleset: RulesetOptions::default(),
        view: ViewOptions::default(),
        output: OutputOptions::default(),
        execution: ExecutionOptions::default(),
        gameplay: GameplayOptions::default(),
    };
    let mut input_type = String::new();
    let mut osr = osr::Arguments::default();
    let mut osu = osu::Arguments::default();
    while let Some(argument) = parser.next().map_err(argument_error)? {
        match argument {
            Long("type") => input_type = value(&mut parser, "--type")?,
            Long("osr-file") => osr.file = Some(value(&mut parser, "--osr-file")?),
            Long("score-id") => osr.score = Some(value(&mut parser, "--score-id")?),
            Long("user") => osr.user = Some(value(&mut parser, "--user")?),
            Long("score-select") => osr.selection = Some(value(&mut parser, "--score-select")?),
            Long("mode") => osr.mode = Some(value(&mut parser, "--mode")?),
            Long("include-fails") => osr.include_fails = true,
            Long("bid") => osu.bid = Some(value(&mut parser, "--bid")?),
            Long("osu-file") => osu.osu_file = Some(value(&mut parser, "--osu-file")?),
            Long("osz-file") => osu.osz_file = Some(value(&mut parser, "--osz-file")?),
            Long("convert") => osu.convert = Some(value(&mut parser, "--convert")?),
            Long("mod") => osu.mods.push(value(&mut parser, "--mod")?),
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
    // 参数格式错误走退出码 2；文件内容相关的 ID 和难度数量校验在读取阶段执行。
    // 库调用方会再次执行同一参数校验，避免入口间规则不同。
    osu.apply(&mut request);
    request.kind = osr.kind(&input_type, &request.source.bid)?;
    request.clone().validate().map_err(argument_error)?;
    Ok(CliAction::Render(Box::new(request)))
}

pub fn run(action: CliAction) -> i32 {
    match action {
        CliAction::Help => {
            println!("{USAGE}");
            0
        }
        CliAction::Version => {
            println!("osu-beatmap-preview-cli v{}", env!("CARGO_PKG_VERSION"));
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

    /// CLI 不再接受 WGPU 相关参数。
    #[test]
    fn wgpu_options_are_not_part_of_cli() {
        let error = parse_args(["--bid=123", "--wgpu"]).expect_err("WGPU 参数必须被拒绝");
        assert_eq!(error.to_string(), "unknown argument: --wgpu");
    }

    /// 参数解析只收集来源路径，不读取文件内容。
    #[test]
    fn local_files_are_collected_without_requiring_bid() {
        let CliAction::Render(request) = parse_args(["--osu-file=map.osu"]).unwrap() else {
            panic!("应解析为渲染请求");
        };
        assert_eq!(request.source.osu_file.as_deref(), Some("map.osu"));
        assert!(request.source.osz_file.is_none());
        assert_eq!(request.source.bid, "");

        let CliAction::Render(request) = parse_args(["--osz-file=a.osz"]).unwrap() else {
            panic!("应解析为渲染请求");
        };
        assert_eq!(request.source.osz_file.as_deref(), Some("a.osz"));
        assert!(request.source.osu_file.is_none());
        assert_eq!(request.source.bid, "");

        let CliAction::Render(request) =
            parse_args(["--osu-file=map.osu", "--osz-file=a.osz", "--bid=7"]).unwrap()
        else {
            panic!("应解析为渲染请求");
        };
        assert_eq!(request.source.osu_file.as_deref(), Some("map.osu"));
        assert_eq!(request.source.osz_file.as_deref(), Some("a.osz"));
        assert_eq!(request.source.bid, "7");
    }

    /// 来源组合错误在参数解析阶段就报出（命令行参数错误的退出码路径）。
    #[test]
    fn source_combination_errors_surface_at_parse_time() {
        let error = parse_args([] as [&str; 0]).unwrap_err();
        assert!(error.to_string().contains("--bid is required"), "{error}");
        for argument in [
            "--osu-file=map.osk",
            "--osu-file=map.osz",
            "--osz-file=map.osu",
            "--osz-file=",
        ] {
            assert!(parse_args([argument, "--bid=7"]).is_err(), "{argument}");
        }
        let error = parse_args(["--input-file=map.osu"]).unwrap_err();
        assert_eq!(error.to_string(), "unknown argument: --input-file");
    }
}

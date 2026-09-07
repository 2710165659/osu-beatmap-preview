//! 命令行适配器；参数规则由库内共享模块统一维护。

use osu_beatmap_preview::cli::{CliAction, USAGE};
#[cfg(feature = "wgpu-renderer")]
use osu_beatmap_preview::generate_preview_wgpu;
use osu_beatmap_preview::{generate_preview, ErrorKind, PreviewError, RenderRequest};

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn main() {
    let action = osu_beatmap_preview::cli::parse_env().unwrap_or_else(|error| {
        eprintln!("error: {error}\n{USAGE}");
        std::process::exit(2);
    });
    match action {
        CliAction::Help => eprintln!("{USAGE}"),
        CliAction::Version => println!("osu-beatmap-preview v{VERSION} (built {BUILD_TIMESTAMP})"),
        CliAction::Render(request) => print_result(render_request(request)),
    }
}

#[cfg(feature = "wgpu-renderer")]
fn render_request(request: RenderRequest) -> Result<serde_json::Value, PreviewError> {
    if request.execution.use_wgpu && request.output.format.as_deref() == Some("mp4") {
        generate_preview_wgpu(request)
    } else {
        generate_preview(request)
    }
}

#[cfg(not(feature = "wgpu-renderer"))]
fn render_request(request: RenderRequest) -> Result<serde_json::Value, PreviewError> {
    generate_preview(request)
}

fn print_result(result: Result<serde_json::Value, PreviewError>) {
    match result {
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

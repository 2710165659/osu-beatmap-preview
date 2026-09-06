//! WGPU exporter；PNG/GIF 保持 CPU 路径，MP4 使用离屏 GPU 路径。

use osu_beatmap_preview::cli::{CliAction, USAGE};
use osu_beatmap_preview::{generate_preview, generate_preview_wgpu, ErrorKind};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let action = osu_beatmap_preview::cli::parse_env().unwrap_or_else(|error| {
        eprintln!("error: {error}\n{USAGE}");
        std::process::exit(2);
    });
    match action {
        CliAction::Help => eprintln!("{USAGE}"),
        CliAction::Version => println!("osu-beatmap-preview-wgpu v{VERSION}"),
        CliAction::Render(request) => {
            let use_wgpu = request.output.format.as_deref() == Some("mp4");
            let result = if use_wgpu {
                generate_preview_wgpu(request)
            } else {
                generate_preview(request)
            };
            match result {
                Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
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
    }
}

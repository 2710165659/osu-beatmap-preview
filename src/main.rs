//! CPU 命令行适配器；参数规则由库内共享模块统一维护。

use osu_beatmap_preview::cli::{CliAction, USAGE};
use osu_beatmap_preview::{generate_preview, ErrorKind};

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let action = osu_beatmap_preview::cli::parse_env().unwrap_or_else(|error| {
        eprintln!("error: {error}\n{USAGE}");
        std::process::exit(2);
    });
    match action {
        CliAction::Help => eprintln!("{USAGE}"),
        CliAction::Version => println!("osu-beatmap-preview v{VERSION} (built {BUILD_TIMESTAMP})"),
        CliAction::Render(request) => match generate_preview(request) {
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
        },
    }
}

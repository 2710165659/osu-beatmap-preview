//! 本地 Web 调试播放器，不参与正式 Release 产物。

mod server;

use osu_beatmap_preview::cli::{CliAction, USAGE};
use osu_beatmap_preview::realtime::RealtimeRequest;

fn main() {
    let request = if std::env::args_os().nth(1).is_none() {
        RealtimeRequest::new("")
    } else {
        match osu_beatmap_preview::cli::parse_env() {
            Ok(CliAction::Help) => {
                println!("{USAGE}");
                return;
            }
            Ok(CliAction::Version) => {
                println!("osu-beatmap-preview-player v{}", env!("CARGO_PKG_VERSION"));
                return;
            }
            Ok(CliAction::Render(request)) => request.into(),
            Err(error) => {
                eprintln!("error: {error}\n{USAGE}");
                std::process::exit(2);
            }
        }
    };

    if let Err(error) = server::run(request, "127.0.0.1:8787") {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

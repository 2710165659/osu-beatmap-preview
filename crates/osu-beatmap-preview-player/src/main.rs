//! 本地调试播放器，不参与正式 Release 产物。

use osu_beatmap_preview::cli::{CliAction, USAGE};

fn main() {
    let action = osu_beatmap_preview::cli::parse_env().unwrap_or_else(|error| {
        eprintln!("error: {error}\n{USAGE}");
        std::process::exit(2);
    });
    match action {
        CliAction::Help => eprintln!("{USAGE}"),
        CliAction::Version => println!("osu-beatmap-preview-player v{}", env!("CARGO_PKG_VERSION")),
        CliAction::Render(request) => {
            if let Err(error) = osu_beatmap_preview::realtime::player::run(request.into()) {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
        }
    }
}

//! CLI 二进制入口。

fn main() {
    let action = osu_beatmap_preview_cli::cli::parse_env().unwrap_or_else(|error| {
        eprintln!("error: {error}\n{}", osu_beatmap_preview_cli::cli::USAGE);
        std::process::exit(2);
    });
    std::process::exit(osu_beatmap_preview_cli::cli::run(action));
}

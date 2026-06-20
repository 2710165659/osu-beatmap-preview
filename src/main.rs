mod canvas;
mod catch;
mod composer;
mod convert;
mod digits;
mod downloader;
mod errors;
mod legacy_random;
mod mania;
mod models;
mod mods;
mod parser;
mod service;
mod skin;
mod slider_path;
mod standard;
mod taiko;
mod text;
mod time_selection;

use errors::{PreviewError, Result};

struct Args {
    bid: String,
    convert: Option<String>,
    mods: Option<String>,
    fmt: Option<String>,
    time: Option<String>,
}

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!(
        "usage: osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko] \
         [--mods=<MODS>] [--fmt=png|gif] [--time=<T1+T2+...>]"
    );
    std::process::exit(code)
}

fn parse_args() -> Args {
    let mut bid: Option<String> = None;
    let mut convert: Option<String> = None;
    let mut mods: Option<String> = None;
    let mut fmt: Option<String> = None;
    let mut time: Option<String> = None;

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        let (key, value) = if let Some((k, v)) = arg.split_once('=') {
            (k.to_string(), Some(v.to_string()))
        } else {
            (arg.clone(), None)
        };
        let mut take_value = |inline: Option<String>| -> String {
            if let Some(v) = inline {
                v
            } else {
                i += 1;
                if i >= argv.len() {
                    eprintln!("error: missing value for {key}");
                    print_usage_and_exit(2)
                }
                argv[i].clone()
            }
        };
        match key.as_str() {
            "--bid" => bid = Some(take_value(value)),
            "--convert" => {
                let v = take_value(value);
                if !["mania", "ctb", "taiko"].contains(&v.as_str()) {
                    eprintln!("error: --convert must be one of mania, ctb, taiko");
                    print_usage_and_exit(2)
                }
                convert = Some(v);
            }
            "--mod" | "--mods" => mods = Some(take_value(value)),
            "--fmt" | "--format" => {
                let v = take_value(value);
                if v != "png" && v != "gif" {
                    eprintln!("error: --fmt must be png or gif");
                    print_usage_and_exit(2)
                }
                fmt = Some(v);
            }
            "--time" | "--times" => time = Some(take_value(value)),
            "-h" | "--help" => print_usage_and_exit(0),
            _ => {
                eprintln!("error: unknown argument: {arg}");
                print_usage_and_exit(2)
            }
        }
        i += 1;
    }

    let Some(bid) = bid else {
        eprintln!("error: --bid is required");
        print_usage_and_exit(2)
    };
    Args {
        bid,
        convert,
        mods,
        fmt,
        time,
    }
}

fn parse_times(raw: &str) -> Result<Vec<f64>> {
    let parts: Vec<&str> = raw
        .split('+')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() > 4 {
        return Err(PreviewError::new("--time accepts at most 4 time points"));
    }
    let mut result = Vec::with_capacity(parts.len());
    for p in parts {
        let val: f64 = p
            .parse()
            .map_err(|_| PreviewError::new(format!("invalid time value: '{p}'")))?;
        if val < 0.0 {
            return Err(PreviewError::new(format!(
                "time must be non-negative, got {val}"
            )));
        }
        result.push(val);
    }
    Ok(result)
}

fn run(args: &Args) -> Result<serde_json::Value> {
    let mod_settings = match &args.mods {
        Some(mod_str) => {
            let settings = mods::parse_mods(mod_str)?;
            let errors = mods::validate_mods(&settings, None, None);
            if !errors.is_empty() {
                return Err(PreviewError::new(format!(
                    "mod conflict: {}",
                    errors.join("; ")
                )));
            }
            Some(settings)
        }
        None => None,
    };

    let times = match &args.time {
        Some(raw) => Some(parse_times(raw)?),
        None => None,
    };

    service::generate_preview(
        &args.bid,
        args.fmt.as_deref(),
        args.convert.as_deref(),
        mod_settings,
        times,
    )
}

fn dump_convert_signatures(osu_path: &str) {
    let beatmap = parser::parse_beatmap(std::path::Path::new(osu_path)).unwrap();
    for target in [1, 2, 3] {
        let conv = convert::convert_beatmap(&beatmap, target, None).unwrap();
        let mut lines = Vec::new();
        match &conv.hit_objects {
            models::HitObjects::Taiko(v) => {
                for o in v.iter().take(2000) {
                    lines.push(format!("[{}, {}, {}, {}]", o.start_time, o.end_time, o.hit_type, o.hitsound));
                }
            }
            models::HitObjects::Catch(v) => {
                for o in v.iter().take(2000) {
                    lines.push(format!("[{}, {}, {}, {}, {}]", o.x, o.y, o.start_time, o.end_time, o.hit_type));
                }
            }
            models::HitObjects::Mania(v) => {
                for o in v.iter().take(2000) {
                    lines.push(format!("[{}, {}, {}, {}]", o.lane, o.start_time, o.end_time, if o.is_long_note { "true" } else { "false" }));
                }
            }
            _ => {}
        }
        println!("{} {} [{}]", target, conv.hit_objects.len(), lines.join(", "));
    }
}

fn main() {
    if let Ok(path) = std::env::var("OSU_PREVIEW_DUMP_CONVERT") {
        dump_convert_signatures(&path);
        return;
    }
    let args = parse_args();
    match run(&args) {
        Ok(result) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        Err(exc) => {
            let payload = serde_json::json!({
                "status": "error",
                "msg": exc.to_string(),
                "preview-img": "",
                "beatmap-info": {},
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            std::process::exit(1);
        }
    }
}

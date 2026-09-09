//! Web 播放器的本地静态服务和媒体资源适配。
//!
//! 浏览器只消费同源的 `.osu`、背景和音频；下载、压缩包解析与缓存都由 native 宿主处理。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use osu_beatmap_preview_core::parse_beatmap_bytes;

const INDEX: &str = include_str!("../web/index.html");
const STYLE: &str = include_str!("../web/style.css");
const SCRIPT: &str = include_str!("../web/app.js");
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;

pub fn run(address: &str) -> Result<(), String> {
    let pkg = build_wasm()?;
    let media_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/player-web/media");
    fs::create_dir_all(&media_root).map_err(|error| format!("创建媒体缓存目录失败：{error}"))?;
    let listener = TcpListener::bind(address).map_err(|error| error.to_string())?;
    println!("osu! beatmap preview WebGPU player: http://{address}");
    for mut stream in listener.incoming().flatten() {
        let mut request = [0u8; 4096];
        let size = stream.read(&mut request).unwrap_or(0);
        let request_text = String::from_utf8_lossy(&request[..size]);
        let path = request_text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        handle_request(&mut stream, path, &request_text, &pkg, &media_root);
    }
    Ok(())
}

fn handle_request(
    stream: &mut impl Write,
    path: &str,
    request: &str,
    pkg: &Path,
    media_root: &Path,
) {
    let route = path.split('?').next().unwrap_or("/");
    if route.starts_with("/resource/") {
        match media_request(route, query_value(path, "bid"), media_root) {
            Ok((content_type, bytes)) => write_media_bytes(
                stream,
                content_type,
                &bytes,
                request_header(request, "Range"),
            ),
            Err(error) => write_text(stream, "502 Bad Gateway", &error),
        }
        return;
    }
    if let Some(name) = route.strip_prefix("/pkg/") {
        if is_safe_file_name(name) {
            let file = pkg.join(name);
            if let Ok(bytes) = fs::read(&file) {
                let content_type =
                    if file.extension().and_then(|value| value.to_str()) == Some("wasm") {
                        "application/wasm"
                    } else {
                        "application/javascript; charset=utf-8"
                    };
                write_bytes(stream, "200 OK", content_type, &bytes);
                return;
            }
        }
    }
    let (content_type, body) = match route {
        "/" | "/index.html" => ("text/html; charset=utf-8", INDEX),
        "/style.css" => ("text/css; charset=utf-8", STYLE),
        "/app.js" => ("application/javascript; charset=utf-8", SCRIPT),
        _ => ("text/plain; charset=utf-8", "not found"),
    };
    let status = if body == "not found" {
        "404 Not Found"
    } else {
        "200 OK"
    };
    write_bytes(stream, status, content_type, body.as_bytes());
}

fn media_request(
    route: &str,
    bid: Option<&str>,
    media_root: &Path,
) -> Result<(&'static str, Vec<u8>), String> {
    let bid = bid.ok_or_else(|| "缺少 BID".to_string())?;
    let media = ensure_media(bid, media_root)?;
    match route {
        "/resource/beatmap" => Ok((
            "text/plain; charset=utf-8",
            fs::read(media.beatmap).map_err(|error| error.to_string())?,
        )),
        "/resource/audio" => Ok((
            mime_for(&media.audio),
            fs::read(media.audio).map_err(|error| error.to_string())?,
        )),
        "/resource/background" => Ok((
            mime_for(&media.background),
            fs::read(media.background).map_err(|error| error.to_string())?,
        )),
        _ => Err("未知资源请求".to_string()),
    }
}

struct MediaFiles {
    beatmap: PathBuf,
    audio: PathBuf,
    background: PathBuf,
}

fn ensure_media(bid: &str, media_root: &Path) -> Result<MediaFiles, String> {
    if bid.is_empty() || !bid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("BID 必须是数字".to_string());
    }
    let directory = media_root.join(bid);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let beatmap_path = directory.join("beatmap.osu");
    if !beatmap_path.is_file() {
        download_to(&format!("https://osu.ppy.sh/osu/{bid}"), &beatmap_path)?;
    }
    let beatmap_bytes =
        fs::read(&beatmap_path).map_err(|error| format!("读取谱面失败：{error}"))?;
    let beatmap =
        parse_beatmap_bytes(&beatmap_bytes).map_err(|error| format!("解析谱面失败：{error}"))?;
    let set_id = beatmap
        .beatmap_set_id()
        .ok_or_else(|| "谱面缺少 BeatmapSetID，无法下载媒体资源".to_string())?;
    let archive = directory.join(format!("{set_id}.osz"));
    if !archive.is_file() {
        download_to(&format!("https://osu.direct/api/d/{set_id}"), &archive)?;
    }
    let audio = extract_named_file(
        &archive,
        beatmap
            .audio_filename()
            .ok_or_else(|| "谱面未指定音频文件".to_string())?,
        &directory,
        "audio",
    )?;
    let background_name = beatmap
        .background_filename
        .as_deref()
        .ok_or_else(|| "谱面未指定背景文件".to_string())?;
    let background = extract_named_file(&archive, background_name, &directory, "background")?;
    Ok(MediaFiles {
        beatmap: beatmap_path,
        audio,
        background,
    })
}

fn download_to(url: &str, target: &Path) -> Result<(), String> {
    let response = ureq::get(url)
        .call()
        .map_err(|error| format!("下载资源失败：{error}"))?;
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length == 0 || length > MAX_DOWNLOAD_BYTES {
            return Err("下载资源为空或超过 128 MiB 限制".to_string());
        }
    }
    let part = target.with_extension("part");
    let mut input = response.into_reader();
    let mut output = File::create(&part).map_err(|error| error.to_string())?;
    let copied = std::io::copy(
        &mut input.by_ref().take(MAX_DOWNLOAD_BYTES + 1),
        &mut output,
    )
    .map_err(|error| error.to_string())?;
    if copied == 0 || copied > MAX_DOWNLOAD_BYTES {
        let _ = fs::remove_file(&part);
        return Err("下载资源为空或超过 128 MiB 限制".to_string());
    }
    fs::rename(part, target).map_err(|error| error.to_string())
}

fn extract_named_file(
    archive_path: &Path,
    wanted: &str,
    directory: &Path,
    stem: &str,
) -> Result<PathBuf, String> {
    let extension = Path::new(wanted)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let target = directory.join(format!("{stem}.{extension}"));
    if target.is_file() {
        return Ok(target);
    }
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| format!("无效的 OSZ 文件：{error}"))?;
    let wanted = normalize_archive_path(wanted)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if !entry.is_dir()
            && normalize_archive_path(entry.name())
                .is_ok_and(|name| name.eq_ignore_ascii_case(&wanted))
        {
            let part = target.with_extension("part");
            let mut output = File::create(&part).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
            fs::rename(part, &target).map_err(|error| error.to_string())?;
            return Ok(target);
        }
    }
    Err(format!("OSZ 中找不到资源：{wanted}"))
}

fn normalize_archive_path(path: &str) -> Result<String, String> {
    let value = path.trim().replace('\\', "/");
    if value.starts_with('/')
        || value
            .split('/')
            .any(|part| part == ".." || part.contains(':'))
    {
        return Err("压缩包资源路径非法".to_string());
    }
    Ok(value
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/"))
}

fn query_value<'a>(path: &'a str, name: &str) -> Option<&'a str> {
    path.split('?')
        .nth(1)?
        .split('&')
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
}

fn request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}
fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.contains("..")
}
fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
fn write_text(stream: &mut impl Write, status: &str, body: &str) {
    write_bytes(stream, status, "text/plain; charset=utf-8", body.as_bytes());
}
fn write_bytes(stream: &mut impl Write, status: &str, content_type: &str, body: &[u8]) {
    // 播放器把 HTML、JS 和 WASM 都内嵌在本地服务二进制中。开发时若允许浏览器
    // 缓存这些固定 URL，重启服务后仍可能运行旧前端，表现为占位画面或旧接口。
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

/// 音频元素依赖 byte range 才能在未完整下载时 seek；缺少它会让浏览器把
/// `seekable` 保持在 0，随后播放时钟无法对齐到谱面预热时间。
fn write_media_bytes(
    stream: &mut impl Write,
    content_type: &str,
    body: &[u8],
    range_header: Option<&str>,
) {
    let Some(range) = range_header.and_then(|value| parse_byte_range(value, body.len())) else {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
        return;
    };
    let slice = &body[range.start..=range.end];
    let head = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
        slice.len(),
        range.start,
        range.end,
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(slice);
}

struct ByteRange {
    start: usize,
    end: usize,
}

fn parse_byte_range(value: &str, length: usize) -> Option<ByteRange> {
    let range = value.trim().strip_prefix("bytes=")?;
    let range = range.split_once(',').map_or(range, |(first, _)| first);
    let (start, end) = range.split_once('-')?;
    if length == 0 {
        return None;
    }
    if start.is_empty() {
        let suffix = end.parse::<usize>().ok()?.min(length);
        return (suffix > 0).then_some(ByteRange {
            start: length - suffix,
            end: length - 1,
        });
    }
    let start = start.parse::<usize>().ok()?;
    if start >= length {
        return None;
    }
    let end = end
        .parse::<usize>()
        .ok()
        .unwrap_or(length - 1)
        .min(length - 1);
    (end >= start).then_some(ByteRange { start, end })
}

fn build_wasm() -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "osu-beatmap-preview-wasm",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(&root)
        .status()
        .map_err(|error| format!("构建 WASM 失败：{error}"))?;
    if !status.success() {
        return Err("WASM 编译失败".into());
    }
    // Web 端每一帧都在浏览器主线程调用 WASM。调试构建会让一次场景光栅化
    // 阻塞数秒，音频和进度条也随之停住，因此播放器只发布优化后的产物。
    let wasm = root.join("target/wasm32-unknown-unknown/release/osu_beatmap_preview_wasm.wasm");
    let pkg = root.join("target/player-web/pkg");
    fs::create_dir_all(&pkg).map_err(|error| format!("创建 WASM 输出目录失败：{error}"))?;
    let status = Command::new("wasm-bindgen")
        .arg(&wasm)
        .args(["--target", "web", "--out-dir"])
        .arg(&pkg)
        .status()
        .map_err(|error| format!("找不到 wasm-bindgen，请安装 wasm-bindgen-cli：{error}"))?;
    if !status.success() {
        return Err("wasm-bindgen 生成胶水代码失败".into());
    }
    Ok(pkg)
}

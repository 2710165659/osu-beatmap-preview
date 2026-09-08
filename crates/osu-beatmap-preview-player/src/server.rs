//! 播放器 HTTP 服务：谱面会话由 Rust 创建，帧和音频由浏览器消费。

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use osu_beatmap_preview::realtime::{
    MediaPolicy, OffscreenRenderer, RealtimeRequest, RealtimeSession,
};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const INDEX: &str = include_str!("../web/index.html");
const STYLE: &str = include_str!("../web/style.css");
const SCRIPT: &str = include_str!("../web/app.js");

#[derive(Default)]
struct AppState {
    session: Option<RealtimeSession>,
    renderer: Option<OffscreenRenderer>,
    audio_path: Option<PathBuf>,
    errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LoadInput {
    bid: String,
    #[serde(default)]
    mods: Vec<String>,
    convert: Option<String>,
    scale: Option<f64>,
    #[serde(default)]
    no_cache: bool,
}

#[derive(Debug, Serialize)]
struct SessionInfo {
    mode: String,
    absolute_start_ms: i64,
    first_object_ms: i64,
    last_object_ms: i64,
    duration_ms: i64,
    beatmap_speed: f64,
    target_fps: u32,
    has_audio: bool,
}

pub fn run(request: RealtimeRequest, address: &str) -> Result<(), String> {
    let server = Server::http(address).map_err(|error| error.to_string())?;
    println!("osu! beatmap preview web player: http://{address}");
    let mut state = AppState::default();
    if !request.bid.trim().is_empty() {
        let _ = load_session(&mut state, request);
    }
    for request in server.incoming_requests() {
        handle_request(request, &mut state);
    }
    Ok(())
}

fn handle_request(mut request: Request, state: &mut AppState) {
    let path = request.url().split('?').next().unwrap_or("/");
    let response = match (request.method(), path) {
        (&Method::Get, "/") => text_response(INDEX, "text/html; charset=utf-8"),
        (&Method::Get, "/style.css") => text_response(STYLE, "text/css; charset=utf-8"),
        (&Method::Get, "/app.js") => text_response(SCRIPT, "application/javascript; charset=utf-8"),
        (&Method::Get, "/api/state") => state_response(state),
        (&Method::Get, "/api/frame") => frame_response(&request, state),
        (&Method::Get, "/api/audio") => audio_response(state),
        (&Method::Post, "/api/load") => load_response(&mut request, state),
        _ => Response::from_string("not found").with_status_code(StatusCode(404)),
    };
    let _ = request.respond(response);
}

fn load_response(request: &mut Request, state: &mut AppState) -> Response<Cursor<Vec<u8>>> {
    let mut body = String::new();
    if let Err(error) = request.as_reader().read_to_string(&mut body) {
        return error_response(format!("读取请求失败：{error}"));
    }
    let input: LoadInput = match serde_json::from_str(&body) {
        Ok(input) => input,
        Err(error) => return error_response(format!("请求格式错误：{error}")),
    };
    let request = RealtimeRequest {
        bid: input.bid,
        mods: input.mods,
        convert: input.convert,
        config: None,
        scale: input.scale,
        no_cache: input.no_cache,
        media_policy: MediaPolicy::AudioAndBackground,
    };
    match load_session(state, request) {
        Ok(info) => json_response(&info),
        Err(error) => error_response(error),
    }
}

fn load_session(state: &mut AppState, request: RealtimeRequest) -> Result<SessionInfo, String> {
    let session = RealtimeSession::load(request).map_err(|error| record_error(state, error))?;
    let renderer = pollster::block_on(OffscreenRenderer::new(session.offscreen_config()))
        .map_err(|error| record_error(state, error))?;
    let timeline = session.timeline();
    let info = SessionInfo {
        mode: format!("{:?}", session.mode()),
        absolute_start_ms: timeline.absolute_start_ms,
        first_object_ms: timeline.first_object_ms,
        last_object_ms: timeline.last_object_ms,
        duration_ms: timeline
            .last_object_ms
            .saturating_sub(timeline.absolute_start_ms),
        beatmap_speed: timeline.beatmap_speed,
        target_fps: session.offscreen_config().target_fps,
        has_audio: session.audio_path().is_some(),
    };
    state.audio_path = session.audio_path().map(PathBuf::from);
    state.renderer = Some(renderer);
    state.session = Some(session);
    Ok(info)
}

fn frame_response(request: &Request, state: &mut AppState) -> Response<Cursor<Vec<u8>>> {
    let Some(session) = &state.session else {
        return error_response("尚未加载谱面".to_string());
    };
    let Some(renderer) = &mut state.renderer else {
        return error_response("WGPU 渲染器未初始化".to_string());
    };
    let query = request.url().split('?').nth(1).unwrap_or("");
    let time = query
        .split('&')
        .find_map(|part| part.strip_prefix("time="))
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(session.timeline().absolute_start_ms);
    let frame = match pollster::block_on(renderer.render_at_absolute(session, time)) {
        Ok(frame) => frame,
        Err(error) => return error_response(record_error(state, error)),
    };
    let mut bytes = Vec::new();
    let result = {
        let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), frame.width(), frame.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // 播放器帧是本机回环传输，优先降低编码延迟，避免压缩阻塞播放时钟。
        encoder.set_compression(png::Compression::Fast);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(frame.as_bytes()))
    };
    match result {
        Ok(()) => binary_response(bytes, "image/png"),
        Err(error) => error_response(format!("PNG 编码失败：{error}")),
    }
}

fn audio_response(state: &AppState) -> Response<Cursor<Vec<u8>>> {
    let Some(path) = &state.audio_path else {
        return error_response("当前谱面没有音频".to_string());
    };
    match fs::read(path) {
        Ok(bytes) => binary_response(bytes, "audio/mpeg"),
        Err(error) => error_response(format!("读取音频失败：{error}")),
    }
}

fn state_response(state: &AppState) -> Response<Cursor<Vec<u8>>> {
    let Some(session) = &state.session else {
        return json_response(&serde_json::json!({ "loaded": false, "errors": state.errors }));
    };
    let timeline = session.timeline();
    json_response(&serde_json::json!({
        "loaded": true,
        "mode": format!("{:?}", session.mode()),
        "absolute_start_ms": timeline.absolute_start_ms,
        "first_object_ms": timeline.first_object_ms,
        "last_object_ms": timeline.last_object_ms,
        "duration_ms": timeline.last_object_ms.saturating_sub(timeline.absolute_start_ms),
        "beatmap_speed": timeline.beatmap_speed,
        "target_fps": session.offscreen_config().target_fps,
        "has_audio": state.audio_path.is_some(),
        "errors": state.errors,
    }))
}

fn record_error(state: &mut AppState, error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    state.errors.push(message.clone());
    message
}

fn text_response(body: &str, content_type: &str) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(body).with_header(header("Content-Type", content_type))
}

fn json_response(value: &impl Serialize) -> Response<Cursor<Vec<u8>>> {
    match serde_json::to_string(value) {
        Ok(body) => text_response(&body, "application/json; charset=utf-8"),
        Err(error) => error_response(error.to_string()),
    }
}

fn binary_response(bytes: Vec<u8>, content_type: &str) -> Response<Cursor<Vec<u8>>> {
    Response::from_data(bytes).with_header(header("Content-Type", content_type))
}

fn error_response(message: String) -> Response<Cursor<Vec<u8>>> {
    json_response(&serde_json::json!({ "error": message })).with_status_code(StatusCode(500))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("静态 HTTP 头必须有效")
}

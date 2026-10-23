//! 浏览器回放输入桥：兼容性先检查，回放字节只在浏览器内解析。
#[cfg(target_arch = "wasm32")]
use super::WebGpuSession;
use osu_beatmap_preview_core::{
    render::shared::replay::font::ArgonFont,
    render::Img,
    replay::{self, ReplayDocument},
    PreviewError, Result,
};
use std::sync::Arc;

pub fn parse(bytes: &[u8]) -> Result<Arc<ReplayDocument>> {
    let document = replay::parse_osr(bytes)?;
    replay::compatibility::select_replay(&document)?;
    Ok(Arc::new(document))
}
pub fn font() -> Result<ArgonFont> {
    let atlas = crate::decode::decode_image(include_bytes!(
        "../../../assets/fonts/argon/Torus-Bold_0.png"
    ))
    .ok_or_else(|| PreviewError::new("Argon 字体图集损坏"))?;
    ArgonFont::from_bmfont(
        include_bytes!("../../../assets/fonts/argon/Torus-Bold.bin"),
        Img {
            w: atlas.width,
            h: atlas.height,
            data: atlas.rgba,
        },
    )
}
pub fn sample_bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "combobreak" => Some(include_bytes!(
            "../../../assets/hitsound/replay/combobreak.wav"
        )),
        "failsound" => Some(include_bytes!(
            "../../../assets/hitsound/replay/failsound.wav"
        )),
        _ => None,
    }
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn document_option(
    options: &wasm_bindgen::JsValue,
) -> std::result::Result<Option<Arc<ReplayDocument>>, wasm_bindgen::JsValue> {
    use wasm_bindgen::{JsCast, JsValue};
    let value = js_sys::Reflect::get(options, &JsValue::from_str("replayBytes"))
        .unwrap_or(JsValue::UNDEFINED);
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    if !value.is_instance_of::<js_sys::Uint8Array>() {
        return Err(JsValue::from_str("回放必须是 Uint8Array"));
    }
    parse(&js_sys::Uint8Array::new(&value).to_vec())
        .map(Some)
        .map_err(super::js_error)
}
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = replayInfo)]
pub fn replay_info(
    bytes: Vec<u8>,
) -> std::result::Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsValue;
    let document = parse(&bytes).map_err(super::js_error)?;
    let result = js_sys::Object::new();
    js_sys::Reflect::set(
        &result,
        &JsValue::from_str("rulesetId"),
        &JsValue::from_f64(document.header.ruleset_id as f64),
    )?;
    js_sys::Reflect::set(
        &result,
        &JsValue::from_str("checksum"),
        &JsValue::from_str(&document.header.beatmap_md5),
    )?;
    js_sys::Reflect::set(
        &result,
        &JsValue::from_str("version"),
        &JsValue::from_f64(document.header.version as f64),
    )?;
    Ok(result.into())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
impl WebGpuSession {
    /// 当前播放时刻的只读结果，供宿主展示回放状态；查询不推进判定。
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = replayState)]
    pub fn replay_state(
        &self,
    ) -> std::result::Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue> {
        use serde::Serialize;
        let Some(replay) = self.inner.replay_timeline() else {
            return Ok(wasm_bindgen::JsValue::NULL);
        };
        let serializer = serde_wasm_bindgen::Serializer::new()
            .serialize_maps_as_objects(true)
            .serialize_missing_as_null(true);
        replay
            .state_at(self.inner.clock_ms(super::wall_ms()))
            .serialize(&serializer)
            .map_err(super::js_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_font_contains_all_replay_labels() {
        assert!(font()
            .unwrap()
            .supports("0123456789x%.-/: SSABCD F REPLAY FAILED GREAT OK MEH MISS M1M2K1K2"));
    }
    #[test]
    fn invalid_replay_rejected_before_session_creation() {
        assert!(parse(b"not an osr").is_err());
    }
}

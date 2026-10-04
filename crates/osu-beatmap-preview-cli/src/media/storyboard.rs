//! MP4 导出用的故事板素材装载与逐帧绘制。
//!
//! 素材来源是谱面包（.osz）：`.osb` 与 `.osu` 的 `[Events]` 解析由 core 的
//! `storyboard` 模块完成，这里只负责「按引用路径取条目 → 解码 PNG/JPG →
//! 交给逐帧绘制」。贴图缺失、解码失败都按缺失处理（osu! 对缺图静默跳过），
//! 不影响 MP4 导出。

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use osu_beatmap_preview_core::processing::media::normalize_entry_path;
use osu_beatmap_preview_core::storyboard::{
    draw_sprites, parse_storyboard, SpriteDraw, Storyboard, StoryboardViewport, Textures,
};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use osu_beatmap_preview_core::Img;

/// 单张贴图的压缩字节上限（超出按缺图处理）。
const MAX_TEXTURE_BYTES: u64 = 64 * 1024 * 1024;
/// 单张贴图的解码像素上限（4096×4096）：防止病态大图撑爆逐帧合成的内存。
const MAX_TEXTURE_PIXELS: u64 = 4096 * 4096;
/// 贴图数量上限：storyboard 通常只有几十张，超限部分按缺图跳过。
const MAX_TEXTURES: usize = 512;
/// `.osb` 的字节上限。
const MAX_OSB_BYTES: u64 = 64 * 1024 * 1024;

/// 已装载的故事板：解析结果 + 贴图。
pub(crate) struct MediaStoryboard {
    pub storyboard: Storyboard,
    pub textures: Textures,
}

impl MediaStoryboard {
    /// 解析 `.osu` 的 `[Events]` 与谱面包里的 `.osb`，并解码引用到的贴图。
    ///
    /// 没有可绘制元素时返回 `None`（storyboard 关闭时调用方根本不进来）。
    pub(crate) fn load(
        osu_text: &str,
        osz_path: &Path,
        deadline: &RequestDeadline,
    ) -> Result<Option<Self>> {
        let file = File::open(osz_path)
            .map_err(|e| PreviewError::download(format!("failed to open osz archive: {e}")))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| PreviewError::download(format!("invalid osz archive: {e}")))?;

        // 一次扫描建立「归一化小写名 → 条目下标」索引，之后按名取 .osb 与贴图。
        let mut names: HashMap<String, usize> = HashMap::new();
        let mut osb_index = None;
        for index in 0..archive.len() {
            let Ok(entry) = archive.by_index(index) else {
                continue;
            };
            let Some(name) = normalize_entry_path(entry.name()) else {
                continue;
            };
            // .osb 没有固定命名规则：取第一个扩展名为 osb 的条目（通常在包根）。
            if osb_index.is_none()
                && name
                    .rsplit('.')
                    .next()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("osb"))
            {
                osb_index = Some(index);
            }
            names.insert(name.to_lowercase(), index);
        }

        let osb_text = match osb_index {
            Some(index) => {
                let mut bytes = Vec::new();
                let entry = archive.by_index(index).map_err(|e| {
                    PreviewError::download(format!("failed to open .osb entry: {e}"))
                })?;
                if entry.size() > MAX_OSB_BYTES {
                    crate::logging::event(
                        "storyboard-prepare",
                        "skip",
                        None,
                        "storyboard .osb entry is too large",
                    );
                    None
                } else {
                    entry
                        .take(MAX_OSB_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| {
                            PreviewError::download(format!("failed to extract .osb: {e}"))
                        })?;
                    Some(String::from_utf8_lossy(&bytes).into_owned())
                }
            }
            None => None,
        };

        let storyboard = parse_storyboard(osu_text, osb_text.as_deref());
        if !storyboard.has_drawable_elements() {
            crate::logging::event(
                "storyboard-prepare",
                "skip",
                None,
                "beatmap has no drawable storyboard elements",
            );
            return Ok(None);
        }

        // 贴图按引用路径逐张取用；无扩展名的路径按 osu! 的 .jpg → .jpeg → .png 顺序尝试。
        let mut textures = Textures::new();
        for path in storyboard.referenced_paths() {
            if textures.len() >= MAX_TEXTURES {
                break;
            }
            deadline.check()?;
            let candidates = texture_candidates(&path);
            let mut bytes = None;
            for candidate in &candidates {
                if let Some(index) = names.get(&candidate.to_lowercase()) {
                    if let Some(raw) = read_entry(&mut archive, *index)? {
                        bytes = Some(raw);
                        break;
                    }
                }
            }
            let Some(bytes) = bytes else {
                continue;
            };
            let Some(image) = decode_texture(&bytes) else {
                continue;
            };
            textures.insert(path, Arc::new(image));
        }
        crate::logging::event(
            "storyboard-prepare",
            "done",
            None,
            &format!("loaded {} textures", textures.len()),
        );
        Ok(Some(Self {
            storyboard,
            textures,
        }))
    }

    /// osu! 的 `ReplacesBackground` 语义：背景层存在与谱面背景同名的元素时，
    /// 宿主应隐藏谱面背景图（故事板里的同名精灵接管背景）。
    pub(crate) fn replaces_background(&self, background_path: Option<&str>) -> bool {
        background_path.is_some_and(|path| self.storyboard.replaces_background(path))
    }

    /// 是否有画在玩法层之下的可绘制元素（Background/Pass/Foreground 层）。
    ///
    /// 有则玩法层帧必须保持透明：这些层画在玩法层之下，玩法层自填的不透明
    /// 内容框底色会把它们整块盖住（见 [`super::MediaBackground::needs_playfield_base`]）。
    pub(crate) fn draws_behind_playfield(&self) -> bool {
        self.storyboard.has_behind_drawable_elements()
    }

    /// 求值 `chart_ms` 时刻的两半绘制列表：`0` 是物件下层（Background/Pass/
    /// Foreground），`1` 是 Overlay 层。
    ///
    /// 每帧只调一次：[`Self::draw_sprites`] 要把两半分别画在物件层两侧，
    /// 分两次求值会让另一半结果直接丢弃，元素多的谱面上每帧白算一半。
    /// `chart_ms` 是谱面绝对毫秒（.osu 时间轴）。
    pub(crate) fn sprites_at(&self, chart_ms: i64) -> (Vec<SpriteDraw<'_>>, Vec<SpriteDraw<'_>>) {
        self.storyboard.sprites_at(chart_ms as f64)
    }

    /// 把 [`Self::sprites_at`] 的一个半区画到画布上。
    ///
    /// `brightness` 是用户暗度亮度（1 − `BACKGROUND_DIM`），与背景图同一亮度
    /// 预暗化（lazer 的 `UserDimContainer.FadeColour(Gray(1-DimLevel))` 语义）。
    /// 视口按画布尺寸换算（SCALE 已体现在画布尺寸里，映射只做等比缩放）。
    pub(crate) fn draw_sprites(&self, canvas: &mut Img, draws: &[SpriteDraw<'_>], brightness: f32) {
        let view =
            StoryboardViewport::new(canvas.w as f32, canvas.h as f32, self.storyboard.widescreen);
        draw_sprites(canvas, draws, &self.textures, &view, brightness);
    }
}

/// 按 osu! 的路径解析规则生成候选条目名：无扩展名时依次尝试 .jpg/.jpeg/.png。
fn texture_candidates(path: &str) -> Vec<String> {
    if path.contains('.') {
        vec![path.to_string()]
    } else {
        ["jpg", "jpeg", "png"]
            .into_iter()
            .map(|ext| format!("{path}.{ext}"))
            .collect()
    }
}

/// 读取指定下标的条目字节（带大小上限；超限按缺图返回 `None`）。
fn read_entry(archive: &mut zip::ZipArchive<File>, index: usize) -> Result<Option<Vec<u8>>> {
    let mut entry = archive
        .by_index(index)
        .map_err(|e| PreviewError::download(format!("failed to open osz entry: {e}")))?;
    if entry.is_dir() || entry.size() == 0 || entry.size() > MAX_TEXTURE_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(MAX_TEXTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PreviewError::download(format!("failed to extract osz entry: {e}")))?;
    if bytes.len() as u64 > MAX_TEXTURE_BYTES {
        return Ok(None);
    }
    Ok(Some(bytes))
}

/// 解码 PNG/JPG 贴图为 RGBA；损坏或超限返回 `None`（缺图静默跳过）。
fn decode_texture(bytes: &[u8]) -> Option<Img> {
    let decoded = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = decoded.dimensions();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_TEXTURE_PIXELS {
        return None;
    }
    Some(Img {
        w,
        h,
        data: decoded.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无扩展名贴图路径按 .jpg → .jpeg → .png 顺序生成候选。
    #[test]
    fn texture_candidates_try_image_extensions_in_osu_order() {
        assert_eq!(texture_candidates("SB/a.png"), vec!["SB/a.png"]);
        assert_eq!(
            texture_candidates("SB/a"),
            vec!["SB/a.jpg", "SB/a.jpeg", "SB/a.png"]
        );
    }

    /// 解码失败与空图都按缺图处理。
    #[test]
    fn broken_texture_bytes_decode_to_none() {
        assert!(decode_texture(b"not an image").is_none());
        assert!(decode_texture(&[]).is_none());
    }

    /// 超过旧 16 MiB 的大 `.osb` 必须照常装载：真实故事板（如 world.execute(me) 的
    /// 18.9 MB、Spin Eternally 的 40.9 MB）会超过旧上限，整份丢弃后表现是
    /// 「谱面明明有故事板却完全不显示」。填充用注释行：夹具足够大但解析开销小。
    #[test]
    fn oversized_osb_beyond_legacy_cap_is_still_loaded() {
        use std::io::Write as _;
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        use osu_beatmap_preview_core::support::timeout::RequestDeadline;
        use zip::write::SimpleFileOptions;

        let padding = "// padding line for an oversized storyboard fixture\n".repeat(350_000);
        let osb = format!(
            "[Events]\nSprite,Background,Centre,\"sb/white.png\",320,240\n F,0,0,1000,1\n{padding}"
        );
        assert!(
            osb.len() as u64 > 16 * 1024 * 1024,
            "夹具必须超过旧的 16 MiB 上限"
        );

        // 临时 .osz 用 Stored 存储：测试只关心字节上限，不需要压缩耗时。
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "osu-beatmap-preview-storyboard-{}-{unique}.osz",
            std::process::id()
        ));
        let file = File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "map.osb",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(osb.as_bytes()).unwrap();
        writer.finish().unwrap();

        let deadline = RequestDeadline::new(Instant::now(), "mp4", Duration::from_secs(30));
        let loaded = MediaStoryboard::load("[Events]\n", &path, &deadline).unwrap();
        let _ = std::fs::remove_file(&path);
        let loaded = loaded.expect("大 .osb 不能被整份丢弃");
        assert!(loaded.storyboard.has_drawable_elements());
        assert_eq!(loaded.storyboard.layers[0].elements.len(), 1);
    }
}

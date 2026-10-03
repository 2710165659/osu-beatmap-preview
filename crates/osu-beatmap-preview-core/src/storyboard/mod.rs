//! osu! storyboard（故事板）解析、时间求值与绘制。
//!
//! 语义对齐 osu! 参考实现（osu.Game `Storyboards` + osu.Framework `Transform`/`Interpolation`）：
//! - 元素坐标系是 640×480 虚拟空间，屏幕映射按高度等比缩放并居中（[`draw::StoryboardViewport`]）；
//! - `L` 循环组按 lazer 公式展开（周期 = 组内 max(end) − min(start)，总次数 = loopCount）；
//! - 命令求值取「已开始命令中排序最后一条」（[`eval`] 里有精确公式）；
//! - 生命周期：元素只在其命令时间跨度内存在——命令未开始不靠初值提前出现
//!   （alpha 先 0 后亮的元素等到第一条可见 alpha 命令才登场），命令结束后也不残留；
//! - 层序：Background/Fail/Pass/Foreground 全部在游玩物件之下（underlay），只有 Overlay
//!   层代理到物件之上、HUD 之下；用户暗度（`BACKGROUND_DIM`）乘在所有精灵颜色上。
//!
//! 有意裁剪（均在文档中标注）：`T` 触发组只解析保留不求值（逐帧乱序并行出帧要求
//! 求值是纯函数）、`Sample` 事件不播放（音频归 `hitsound` 模块）、`UseSkinSprites`
//! 贴图只从谱面包取。

pub mod draw;
pub mod eval;
pub mod parse;

pub use draw::{
    draw_sprite, draw_sprites, draw_storyboard, draw_transformed_sprite, SpriteTransform,
    StoryboardViewport,
};
pub use eval::ElementState;
pub use parse::parse_storyboard;

use crate::render::canvas::Img;
use std::collections::HashMap;
use std::sync::Arc;

/// 故事板虚拟坐标系宽度（osu! 设计空间）。
pub const VIRTUAL_WIDTH: f32 = 640.0;
/// 故事板虚拟坐标系高度（osu! 设计空间）。
pub const VIRTUAL_HEIGHT: f32 = 480.0;

/// 元素所在层；数值与 `LegacyStoryLayer` 一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Background = 0,
    Fail = 1,
    Pass = 2,
    Foreground = 3,
    Overlay = 4,
    Video = 5,
}

impl Layer {
    /// 解析层名（枚举名或数字，与 osu! 的 `Enum.Parse` 兼容）。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "Background" | "0" => Some(Self::Background),
            "Fail" | "1" => Some(Self::Fail),
            "Pass" | "2" => Some(Self::Pass),
            "Foreground" | "3" => Some(Self::Foreground),
            "Overlay" | "4" => Some(Self::Overlay),
            "Video" | "5" => Some(Self::Video),
            _ => None,
        }
    }
}

/// 元素原点（锚点）；数值与 `LegacyOrigins` 的 stable 编号一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    TopLeft = 0,
    Centre = 1,
    CentreLeft = 2,
    TopRight = 3,
    BottomCentre = 4,
    TopCentre = 5,
    Custom = 6,
    CentreRight = 7,
    BottomLeft = 8,
    BottomRight = 9,
}

impl Origin {
    /// 解析原点（枚举名或数字）；`Custom` 与未知值按 osu! 回退到左上角。
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "Centre" | "1" => Self::Centre,
            "CentreLeft" | "2" => Self::CentreLeft,
            "TopRight" | "3" => Self::TopRight,
            "BottomCentre" | "4" => Self::BottomCentre,
            "TopCentre" | "5" => Self::TopCentre,
            "CentreRight" | "7" => Self::CentreRight,
            "BottomLeft" | "8" => Self::BottomLeft,
            "BottomRight" | "9" => Self::BottomRight,
            _ => Self::TopLeft,
        }
    }

    /// 归一化锚点偏移（0 / 0.5 / 1），与 osu.Framework `Anchor` 一致。
    pub fn anchor(self) -> [f32; 2] {
        match self {
            Self::TopLeft | Self::Custom => [0.0, 0.0],
            Self::TopCentre => [0.5, 0.0],
            Self::TopRight => [1.0, 0.0],
            Self::CentreLeft => [0.0, 0.5],
            Self::Centre => [0.5, 0.5],
            Self::CentreRight => [1.0, 0.5],
            Self::BottomLeft => [0.0, 1.0],
            Self::BottomCentre => [0.5, 1.0],
            Self::BottomRight => [1.0, 1.0],
        }
    }
}

/// `Animation` 元素的循环方式；数值与 osu! 的 `AnimationLoopType` 一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationLoop {
    LoopForever = 0,
    LoopOnce = 1,
}

impl AnimationLoop {
    /// 解析循环方式（枚举名或数字）；未定义值回退 `LoopForever`。
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "LoopOnce" | "1" => Self::LoopOnce,
            _ => Self::LoopForever,
        }
    }
}

/// 元素种类：静态精灵或逐帧动画。
#[derive(Clone, Debug, PartialEq)]
pub enum ElementKind {
    Sprite,
    Animation {
        frame_count: u32,
        /// 每帧显示时长（毫秒）。
        frame_delay_ms: f64,
        loop_type: AnimationLoop,
    },
}

/// 命令影响的属性；顺序与 osu.Game `StoryboardCommandGroup` 一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    X,
    Y,
    Scale,
    VectorScale,
    Rotation,
    Colour,
    Alpha,
    Additive,
    FlipH,
    FlipV,
}

/// 属性个数（命令求值按属性分桶）。
pub const PROPERTY_COUNT: usize = 10;

/// 属性枚举与求值桶下标的映射。
pub(crate) const PROPERTIES: [Property; PROPERTY_COUNT] = [
    Property::X,
    Property::Y,
    Property::Scale,
    Property::VectorScale,
    Property::Rotation,
    Property::Colour,
    Property::Alpha,
    Property::Additive,
    Property::FlipH,
    Property::FlipV,
];

impl Property {
    pub(crate) fn index(self) -> usize {
        PROPERTIES
            .iter()
            .position(|p| *p == self)
            .expect("属性必须在常量表内")
    }
}

/// 命令取值；标量属性只用 `Scalar`，以此类推。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Scalar(f32),
    Vector(f32, f32),
    /// 0..1 的 RGB（`C` 命令的 0..255 已换算）。
    Rgb(f32, f32, f32),
    Flag(bool),
}

/// 一条变换命令：`{TYPE},{easing},{start},{end},{values...}`。
///
/// 时间语义与 osu! 一致：`start` 时刻属性跳变到 `start_value`，`[start, end]`
/// 内按 easing 插值到 `end_value`，之后保持 `end_value`；`end` 缺省即瞬时命令。
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub property: Property,
    /// 缓动编号（0..=35，对应 osu.Framework `Easing` 枚举序号）。
    pub easing: u32,
    pub start_ms: f64,
    pub end_ms: f64,
    pub start_value: Value,
    pub end_value: Value,
}

impl Command {
    /// 瞬时命令（start == end）。
    pub fn is_instant(&self) -> bool {
        self.start_ms >= self.end_ms
    }
}

/// `T` 触发组：命令时间相对触发时刻。
///
/// 本项目只解析保留、不参与求值（见模块文档）。
#[derive(Clone, Debug)]
pub struct TriggerGroup {
    pub name: String,
    pub start_ms: f64,
    pub end_ms: f64,
    /// osu! 解析时取负的分组号（stable 怪癖），原样保留。
    pub group_number: i32,
    pub commands: Vec<Command>,
}

/// 元素的命令集合：循环组已展开为绝对时间命令并按属性分桶排序。
#[derive(Clone, Debug, Default)]
pub struct ElementCommands {
    /// 每个属性一桶，桶内稳定排序 `(start_ms, end_ms)`；同键保留定义顺序。
    lists: Vec<Vec<Command>>,
    /// 触发组原样保留。
    pub triggers: Vec<TriggerGroup>,
}

impl ElementCommands {
    /// 某属性的命令序列（已展开循环、已排序）。
    pub fn list(&self, property: Property) -> &[Command] {
        self.lists
            .get(property.index())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// 是否含任何会生效的命令（无命令的元素不绘制，与 osu! 的 `HasCommands` 一致）。
    pub fn has_commands(&self) -> bool {
        self.lists.iter().any(|list| !list.is_empty())
    }

    /// 由解析器填充分桶与排序。
    pub(crate) fn from_sorted(lists: Vec<Vec<Command>>, triggers: Vec<TriggerGroup>) -> Self {
        Self { lists, triggers }
    }
}

/// 一个故事板元素（`Sprite` 或 `Animation`）。
#[derive(Clone, Debug)]
pub struct Element {
    pub kind: ElementKind,
    pub layer: Layer,
    /// 贴图路径（已归一化为 `/` 分隔）。
    pub path: String,
    pub origin: Origin,
    /// 元素声明位置（640×480 虚拟坐标）。
    pub x: f32,
    pub y: f32,
    pub commands: ElementCommands,
}

impl Element {
    /// 无命令的元素在 osu! 中完全不绘制（`IsDrawable => HasCommands`）。
    pub fn is_drawable(&self) -> bool {
        self.commands.has_commands()
    }

    /// 元素最早命令开始时间（osu! 的 `EarliestTransformTime`）。
    pub fn earliest_start_ms(&self) -> f64 {
        self.commands
            .lists
            .iter()
            .flatten()
            .map(|command| command.start_ms)
            .fold(f64::INFINITY, f64::min)
    }

    /// 元素的绘制起点（osu! 的 `StoryboardSprite.StartTime`，即 `LifetimeStart`）。
    ///
    /// 与「最早命令」不同是 lazer 的 alpha 优化：最早 alpha 命令起点值为 0（先隐形
    /// 后现身）时，元素要等到第一条**可见** alpha 命令才存在，否则「几秒后才登场」
    /// 的精灵会被初值提前画出来（比如转场黑幕盖住整首歌）。
    pub fn start_time_ms(&self) -> f64 {
        let alphas = self.commands.list(Property::Alpha);
        let visible = |command: &Command| {
            matches!(command.start_value, Value::Scalar(start) if start > 0.0)
                || matches!(command.end_value, Value::Scalar(end) if end > 0.0)
        };
        if let Some(first) = alphas.first() {
            if matches!(first.start_value, Value::Scalar(start) if start == 0.0) {
                if let Some(real) = alphas.iter().find(|command| visible(command)) {
                    return real.start_ms;
                }
            }
        }
        self.earliest_start_ms()
    }

    /// 元素的绘制终点（osu! 的 `EndTimeForDisplay`，即 `LifetimeEnd`）。
    /// 循环组已展开进命令桶，取所有命令的最大结束时间即可；超出这个时刻元素不再
    /// 存在——即使数值语义上它还停在最后一帧的状态。
    pub fn end_time_ms(&self) -> f64 {
        self.commands
            .lists
            .iter()
            .flatten()
            .map(|command| command.end_ms)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// 元素在 `time_ms` 是否存在（生命周期窗口内才绘制，与 osu! 一致）。
    pub fn is_alive_at(&self, time_ms: f64) -> bool {
        self.start_time_ms() <= time_ms && time_ms <= self.end_time_ms()
    }

    /// 该时刻实际要取用的贴图路径（动画元素为当前帧路径）。
    ///
    /// 帧号在扩展名前插入（`sprite.png` → `sprite0.png`，多点文件名按最后一个点插入）；
    /// lazer 的 `String.Replace` 会在每个点前都插帧号，与格式文档不符，这里取直觉行为。
    pub fn texture_path_at(&self, frame_index: u32) -> String {
        match self.kind {
            ElementKind::Sprite => self.path.clone(),
            ElementKind::Animation { .. } => match self.path.rfind('.') {
                Some(dot) => format!("{}{}{}", &self.path[..dot], frame_index, &self.path[dot..]),
                None => format!("{}{}", self.path, frame_index),
            },
        }
    }

    /// 汇总元素引用的全部贴图路径（动画为逐帧全部路径）。
    pub fn referenced_paths(&self) -> Vec<String> {
        match self.kind {
            ElementKind::Sprite => vec![self.path.clone()],
            ElementKind::Animation { frame_count, .. } => (0..frame_count.max(1))
                .map(|frame| self.texture_path_at(frame))
                .collect(),
        }
    }
}

/// `Sample` 事件（当前只解析保留、不播放）。
#[derive(Clone, Debug)]
pub struct Sample {
    pub time_ms: f64,
    pub layer: Layer,
    pub path: String,
    /// 音量百分比（osu! 默认 100）。
    pub volume: i32,
}

/// 按层聚合的故事板元素。
#[derive(Clone, Debug)]
pub struct LayerElements {
    pub layer: Layer,
    /// 文件定义顺序；层内后定义的元素画在上层（与 osu.Framework 的绘制顺序一致）。
    pub elements: Vec<Element>,
}

/// 解析完成的故事板。
#[derive(Clone, Debug, Default)]
pub struct Storyboard {
    /// `WidescreenStoryboard`：宽屏时层遮罩放宽到 x∈[-106.67, 746.67]。
    pub widescreen: bool,
    /// `UseSkinSprites`：贴图优先取自皮肤（当前实现只查谱面包，见模块文档）。
    pub use_skin_sprites: bool,
    /// 各层元素，按绘制顺序从底到顶排列。
    pub layers: Vec<LayerElements>,
    /// `Sample` 事件（只解析保留）。
    pub samples: Vec<Sample>,
}

/// 某时刻一个待绘制精灵的求值结果。
#[derive(Clone, Debug)]
pub struct SpriteDraw<'a> {
    pub element: &'a Element,
    pub state: ElementState,
}

impl SpriteDraw<'_> {
    /// 该时刻实际要取用的贴图路径。
    pub fn texture_path(&self) -> String {
        self.element.texture_path_at(self.state.frame_index)
    }
}

impl Storyboard {
    /// 是否包含可绘制元素（无命令的元素不算）。
    pub fn has_drawable_elements(&self) -> bool {
        self.layers
            .iter()
            .flat_map(|layer| &layer.elements)
            .any(Element::is_drawable)
    }

    /// osu! 的 `ReplacesBackground` 语义：背景层存在与谱面背景同名（大小写不敏感）
    /// 的元素时，宿主应隐藏谱面背景图。
    pub fn replaces_background(&self, background_path: &str) -> bool {
        let target = normalize_path(background_path);
        self.layers
            .iter()
            .filter(|layer| layer.layer == Layer::Background)
            .flat_map(|layer| &layer.elements)
            .any(|element| normalize_path(&element.path) == target)
    }

    /// 收集引用的全部贴图路径（已去重、保持首次出现顺序），供宿主批量装载。
    pub fn referenced_paths(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut paths = Vec::new();
        for element in self.layers.iter().flat_map(|layer| &layer.elements) {
            for path in element.referenced_paths() {
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
        }
        paths
    }

    /// 求值 `time_ms` 时刻的绘制列表：`behind` 画在游玩物件之下（Background/Pass/
    /// Foreground，osu! 的 underlay），`front` 画在物件之上、HUD 之下（Overlay），
    /// 两者都按绘制顺序从底到顶。`Fail` 层仅在失败时可见，自动游玩预览恒为通过，
    /// 因此跳过。
    pub fn sprites_at(&self, time_ms: f64) -> (Vec<SpriteDraw<'_>>, Vec<SpriteDraw<'_>>) {
        let mut behind = Vec::new();
        let mut front = Vec::new();
        for layer in &self.layers {
            let target = match layer.layer {
                Layer::Background | Layer::Pass | Layer::Foreground => &mut behind,
                Layer::Overlay => &mut front,
                Layer::Fail | Layer::Video => continue,
            };
            for element in &layer.elements {
                if !element.is_drawable() {
                    continue;
                }
                // 生命周期窗口外元素不存在（osu! 的 LifetimeStart/LifetimeEnd）：
                // 命令还没开始的精灵不能靠初值提前出现，命令结束的精灵也不残留。
                if !element.is_alive_at(time_ms) {
                    continue;
                }
                let state = element.state_at(time_ms);
                // 完全透明的精灵不进绘制列表：既省合成，也避免加色精灵贡献 0。
                if state.alpha <= 0.0 {
                    continue;
                }
                target.push(SpriteDraw { element, state });
            }
        }
        (behind, front)
    }
}

/// 贴图表：归一化路径 → RGBA 图像（宿主解码后交给求值/绘制）。
pub type Textures = HashMap<String, Arc<Img>>;

/// 归一化贴图路径：去引号、反斜杠转 `/`，与 `domain::media::normalize_entry_path` 同语义。
pub fn normalize_path(path: &str) -> String {
    path.trim()
        .trim_matches('"')
        .replace('\\', "/")
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 原点锚点按 stable 编号映射，未知值回退左上角。
    #[test]
    fn origin_parses_stable_numbers_and_falls_back_to_top_left() {
        assert_eq!(Origin::parse("Centre"), Origin::Centre);
        assert_eq!(Origin::parse("1"), Origin::Centre);
        assert_eq!(Origin::parse("BottomRight"), Origin::BottomRight);
        assert_eq!(Origin::parse("9"), Origin::BottomRight);
        assert_eq!(Origin::parse("Custom"), Origin::TopLeft);
        assert_eq!(Origin::parse("nope"), Origin::TopLeft);
        assert_eq!(Origin::Centre.anchor(), [0.5, 0.5]);
    }

    /// 层与循环方式同时接受枚举名和数字。
    #[test]
    fn layer_and_animation_loop_parse_names_and_numbers() {
        assert_eq!(Layer::parse("Foreground"), Some(Layer::Foreground));
        assert_eq!(Layer::parse("3"), Some(Layer::Foreground));
        assert_eq!(Layer::parse("Video"), Some(Layer::Video));
        assert_eq!(Layer::parse("nope"), None);
        assert_eq!(AnimationLoop::parse("LoopOnce"), AnimationLoop::LoopOnce);
        assert_eq!(AnimationLoop::parse("1"), AnimationLoop::LoopOnce);
        assert_eq!(AnimationLoop::parse("nope"), AnimationLoop::LoopForever);
    }

    /// 动画帧路径在扩展名前插入帧序号。
    #[test]
    fn animation_frame_paths_insert_index_before_extension() {
        let element = Element {
            kind: ElementKind::Animation {
                frame_count: 3,
                frame_delay_ms: 100.0,
                loop_type: AnimationLoop::LoopForever,
            },
            layer: Layer::Background,
            path: "sb/anim.png".to_string(),
            origin: Origin::Centre,
            x: 0.0,
            y: 0.0,
            commands: ElementCommands::default(),
        };
        assert_eq!(element.texture_path_at(0), "sb/anim0.png");
        assert_eq!(element.texture_path_at(2), "sb/anim2.png");
    }

    /// 路径归一化去引号并统一为 `/` 分隔。
    #[test]
    fn normalize_path_unifies_separators_and_strips_quotes() {
        assert_eq!(normalize_path(r#""SB\Lyrics\1.png""#), "SB/Lyrics/1.png");
    }

    /// 生命周期：命令未开始的元素不靠初值提前出现，命令结束后不残留
    ///（实测 bug：转场黑幕的命令都在歌曲尾段，却被初值提前画满全屏）。
    #[test]
    fn elements_only_exist_within_their_command_lifetime() {
        let storyboard = parse_storyboard(
            "[Events]\n\
             Sprite,Foreground,Centre,\"late.png\",232,231\n\
             _S,0,62741,,0.44\n\
             _R,0,62741,,1.57\n\
             _M,0,62741,63296,232,231,851,230\n\
             _F,0,63574,,1\n\
             Sprite,Foreground,Centre,\"swipe.png\",206,240\n\
             _S,0,24025,,0.43\n\
             _M,0,24025,24114,206,240,-239,240",
            None,
        );
        let elements = &storyboard.layers[0].elements;
        // #1：最早 alpha 命令起点值非 0 → 起点取最早命令；25.5s 时它还没登场。
        assert_eq!(elements[0].start_time_ms(), 62741.0);
        assert!(!elements[0].is_alive_at(25_500.0));
        assert!(elements[0].is_alive_at(63_000.0));
        // #2：89ms 的擦除动画命令结束即消失，不留残影。
        assert_eq!(elements[1].start_time_ms(), 24025.0);
        assert_eq!(elements[1].end_time_ms(), 24114.0);
        assert!(!elements[1].is_alive_at(25_500.0));
        assert!(elements[1].is_alive_at(24_050.0));
        // 求值列表同样受生命周期约束。
        let (behind, _) = storyboard.sprites_at(25_500.0);
        assert!(behind.is_empty(), "25.5s 时两个元素都未登场");
    }

    /// alpha 先 0 后亮的元素等到第一条「可见」alpha 命令才登场。
    #[test]
    fn fade_in_elements_wait_for_first_visible_alpha_command() {
        let storyboard = parse_storyboard(
            "[Events]\n\
             Sprite,Foreground,Centre,\"fade.png\",320,240\n\
             _M,0,1000,4000,0,0,100,100\n\
             _F,0,2000,3000,0,1",
            None,
        );
        let element = &storyboard.layers[0].elements[0];
        assert_eq!(
            element.start_time_ms(),
            2000.0,
            "最早 alpha 命令起点值为 0 → 等第一条可见 alpha 命令"
        );
        assert!(element.is_alive_at(2500.0));
        assert!(!element.is_alive_at(1500.0));
    }
}

//! .osu / .osb 的 `[Events]`、`[Variables]` 解析。
//!
//! 语法与 osu! 的 `LegacyStoryboardDecoder` 一致：
//! - 行首空格/下划线计为缩进层级：0 级是元素头行，≥1 级是命令行，`L`/`T`
//!   组头通常缩进 1 级、其子命令缩进 2 级（深度 < 2 时命令回到元素根组）；
//! - `[Variables]` 的 `key=value` 对后续行做单轮文本替换（不递归，防自引用死循环）；
//! - 事件类型/层/原点同时接受枚举名与数字（stable 编号）；解析失败的行跳过不报错。
//!
//! `.osu` 的 `[Events]` 与 `.osb` 全文都会被解析，`.osb` 元素排在 `.osu` 元素之后
//! （osu! 的主/次流解码顺序），层内后定义的元素绘制在上层。

use super::{
    normalize_path, Command, Element, ElementCommands, ElementKind, Layer, Origin, Property,
    Sample, Storyboard, TriggerGroup, Value, AnimationLoop, LayerElements, PROPERTY_COUNT,
};
use crate::domain::parser::round_half_even;

/// osu! 对事件坐标的取值上限（`Parsing.MAX_COORDINATE_VALUE`）。
const MAX_COORDINATE_VALUE: f32 = 131_072.0;

/// 解析 .osu（必需）与 .osb（可选）文本为故事板。
///
/// `.osu` 提供格式版本、`[General]`（WidescreenStoryboard / UseSkinSprites）与内联
/// 故事板事件；`.osb` 的对应内容在其后合并。
pub fn parse_storyboard(osu_text: &str, osb_text: Option<&str>) -> Storyboard {
    let mut builder = StoryboardBuilder::default();
    parse_stream(osu_text, &mut builder);
    if let Some(osb_text) = osb_text {
        parse_stream(osb_text, &mut builder);
    }
    builder.finish()
}

/// 单个文本流的解析产物聚合。
#[derive(Default)]
struct StoryboardBuilder {
    widescreen: bool,
    use_skin_sprites: bool,
    variables: Vec<(String, String)>,
    elements: Vec<Element>,
    samples: Vec<Sample>,
}

impl StoryboardBuilder {
    fn finish(self) -> Storyboard {
        // 按 lazer 的层顺序聚合（Depth 大者靠后）；层内保持定义顺序，后定义在上。
        const ORDER: [Layer; 5] = [
            Layer::Background,
            Layer::Fail,
            Layer::Pass,
            Layer::Foreground,
            Layer::Overlay,
        ];
        let layers = ORDER
            .into_iter()
            .map(|layer| LayerElements {
                layer,
                elements: self
                    .elements
                    .iter()
                    .filter(|element| element.layer == layer)
                    .cloned()
                    .collect(),
            })
            .filter(|layer| !layer.elements.is_empty())
            .collect();
        Storyboard {
            widescreen: self.widescreen,
            use_skin_sprites: self.use_skin_sprites,
            layers,
            samples: self.samples,
        }
    }
}

/// 元素头行与命令行之间的解析状态。
struct ElementBuilder {
    kind: ElementKind,
    layer: Layer,
    path: String,
    origin: Origin,
    x: f32,
    y: f32,
    root: Vec<Command>,
    loops: Vec<LoopBuilder>,
    triggers: Vec<TriggerGroup>,
    active: ActiveGroup,
}

/// 命令当前归属的组。
enum ActiveGroup {
    Root,
    Loop(usize),
    Trigger(usize),
}

struct LoopBuilder {
    start_ms: f64,
    iterations: u32,
    commands: Vec<Command>,
}

impl ElementBuilder {
    /// 展开循环组并按属性分桶排序，产出可求值的元素。
    ///
    /// 循环展开按 lazer 语义：组周期 = 组内 `max(end) - min(start)`，第 k 次播放把
    /// 子命令平移 `loopStart + k * 周期`（子命令时间相对 loopStart）。
    fn finish(self) -> Element {
        let mut lists = vec![Vec::<Command>::new(); PROPERTY_COUNT];
        let mut push = |command: Command| lists[command.property.index()].push(command);
        for command in self.root {
            push(command);
        }
        for group in &self.loops {
            if group.commands.is_empty() {
                continue;
            }
            let min_start = group
                .commands
                .iter()
                .map(|command| command.start_ms)
                .fold(f64::INFINITY, f64::min);
            let max_end = group
                .commands
                .iter()
                .map(|command| command.end_ms)
                .fold(f64::NEG_INFINITY, f64::max);
            let period = (max_end - min_start).max(0.0);
            for iteration in 0..group.iterations {
                let offset = group.start_ms + period * f64::from(iteration);
                for command in &group.commands {
                    push(Command {
                        start_ms: command.start_ms + offset,
                        end_ms: command.end_ms + offset,
                        ..*command
                    });
                }
            }
        }
        // 稳定排序 (start, end)：同键保留定义顺序，求值取「最后开始」的一条，
        // 因此同起点同时长的命令后定义者优先（lazer 对该情况行为未定义，取稳定语义）。
        for list in &mut lists {
            list.sort_by(|left, right| {
                left.start_ms
                    .partial_cmp(&right.start_ms)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        left.end_ms
                            .partial_cmp(&right.end_ms)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
        }
        Element {
            kind: self.kind,
            layer: self.layer,
            path: self.path,
            origin: self.origin,
            x: self.x,
            y: self.y,
            commands: ElementCommands::from_sorted(lists, self.triggers),
        }
    }
}

/// 解析一个文本流（.osu 或 .osb）。
fn parse_stream(text: &str, builder: &mut StoryboardBuilder) {
    // 没有文件头时按最新格式处理（只影响 v<6 的动画帧时长换算）。
    let format_version = parse_header_version(text).unwrap_or(14);
    let mut section = String::new();
    let mut current: Option<ElementBuilder> = None;

    for raw in text.lines() {
        // 注释与 osu! 一致：`//` 之后整体截断（文件名里出现 `//` 的情况不存在）。
        let line = match raw.find("//") {
            Some(index) => &raw[..index],
            None => raw,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].to_string();
            // 区段切换时同样要落盘当前元素，避免元素跨区段悬挂。
            if let Some(element) = current.take() {
                builder.elements.push(element.finish());
            }
            continue;
        }

        // 变量替换先行（只做一轮，每个变量全量替换一次）。
        let line = substitute_variables(line, &builder.variables);

        match section.as_str() {
            "Variables" => {
                if let Some((key, value)) = line.split_once('=') {
                    builder.variables.push((key.trim().to_string(), value.trim().to_string()));
                }
            }
            "General" => {
                if let Some((key, value)) = line.split_once(':') {
                    match key.trim() {
                        "WidescreenStoryboard" => builder.widescreen = value.trim() == "1",
                        "UseSkinSprites" => builder.use_skin_sprites = value.trim() == "1",
                        _ => {}
                    }
                }
            }
            "Events" => {
                parse_event_line(&line, format_version, &mut current, builder);
            }
            _ => {}
        }
    }

    // 流末尾的最后一个元素也要落盘。
    if let Some(element) = current.take() {
        builder.elements.push(element.finish());
    }
}

/// 解析 `[Events]` 里的一行（元素头或命令）。
fn parse_event_line(
    line: &str,
    format_version: i32,
    current: &mut Option<ElementBuilder>,
    builder: &mut StoryboardBuilder,
) {
    let depth = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '_')
        .count();
    let content = &line[depth..];
    if content.is_empty() {
        return;
    }
    let split: Vec<&str> = content.split(',').collect();

    if depth == 0 {
        // 新元素头行出现时先落盘上一个元素。
        if let Some(element) = current.take() {
            builder.elements.push(element.finish());
        }
        *current = parse_element_header(&split, format_version, builder);
        return;
    }

    let Some(element) = current.as_mut() else {
        // osu! 同样静默丢弃「元素之前的命令行」。
        return;
    };
    // 缩进 < 2 时命令回到元素根组；更深的缩进保持在当前 L/T 组内。
    if depth < 2 {
        element.active = ActiveGroup::Root;
    }
    parse_command_line(&split, element);
}

/// 解析元素头行（Sprite / Animation / Sample；其余事件由谱面解析器负责）。
fn parse_element_header(
    split: &[&str],
    format_version: i32,
    builder: &mut StoryboardBuilder,
) -> Option<ElementBuilder> {
    let kind = split.first().copied().unwrap_or_default();
    match kind {
        // Background / Video / Break / Colour 事件不属于故事板元素。
        "Background" | "0" | "Video" | "1" | "Break" | "2" | "Colour" | "3" => None,
        "Sprite" | "4" => {
            let layer = Layer::parse(split.get(1)?)?;
            let origin = Origin::parse(split.get(2)?);
            let path = normalize_path(split.get(3)?);
            let x = parse_coordinate(split.get(4)?)?;
            let y = parse_coordinate(split.get(5)?)?;
            Some(ElementBuilder {
                kind: ElementKind::Sprite,
                layer,
                path,
                origin,
                x,
                y,
                root: Vec::new(),
                loops: Vec::new(),
                triggers: Vec::new(),
                active: ActiveGroup::Root,
            })
        }
        "Animation" | "6" => {
            let layer = Layer::parse(split.get(1)?)?;
            let origin = Origin::parse(split.get(2)?);
            let path = normalize_path(split.get(3)?);
            let x = parse_coordinate(split.get(4)?)?;
            let y = parse_coordinate(split.get(5)?)?;
            let frame_count = parse_f64(split.get(6)?)?.max(0.0) as u32;
            let mut frame_delay_ms = parse_f64(split.get(7)?)?;
            if format_version < 6 {
                // 老格式的帧时长换算（照抄 osu-stable；Math.Round 为四舍六入五成双）。
                frame_delay_ms = round_half_even(0.015 * frame_delay_ms) as f64
                    * 1.186
                    * (1000.0 / 60.0);
            }
            let loop_type = split
                .get(8)
                .map(|value| AnimationLoop::parse(value))
                .unwrap_or(AnimationLoop::LoopForever);
            Some(ElementBuilder {
                kind: ElementKind::Animation {
                    frame_count,
                    frame_delay_ms,
                    loop_type,
                },
                layer,
                path,
                origin,
                x,
                y,
                root: Vec::new(),
                loops: Vec::new(),
                triggers: Vec::new(),
                active: ActiveGroup::Root,
            })
        }
        "Sample" | "5" => {
            let time_ms = parse_f64(split.get(1)?)?;
            let layer = Layer::parse(split.get(2)?)?;
            let path = normalize_path(split.get(3)?);
            let volume = split
                .get(4)
                .and_then(|value| parse_f64(value))
                .map(|value| value as i32)
                .unwrap_or(100);
            builder.samples.push(Sample {
                time_ms,
                layer,
                path,
                volume,
            });
            None
        }
        _ => None,
    }
}

/// 解析命令行（F/M/MX/MY/S/V/R/C/P/L/T）。
fn parse_command_line(split: &[&str], element: &mut ElementBuilder) {
    let kind = split.first().copied().unwrap_or_default();
    match kind {
        "T" => {
            // 触发组：命令时间相对触发时刻；分组号按 stable 怪癖取负。只解析保留。
            let name = split.get(1).copied().unwrap_or_default().to_string();
            let start_ms = split
                .get(2)
                .filter(|value| !value.trim().is_empty())
                .and_then(|value| parse_f64(value))
                .unwrap_or(f64::MIN);
            let end_ms = split
                .get(3)
                .filter(|value| !value.trim().is_empty())
                .and_then(|value| parse_f64(value))
                .unwrap_or(f64::MAX);
            let group_number = split
                .get(4)
                .and_then(|value| parse_f64(value))
                .map(|value| -(value as i32))
                .unwrap_or(0);
            element.triggers.push(TriggerGroup {
                name,
                start_ms,
                end_ms,
                group_number,
                commands: Vec::new(),
            });
            element.active = ActiveGroup::Trigger(element.triggers.len() - 1);
        }
        "L" => {
            // L,startTime,loopCount：总播放次数 = loopCount（≤0 视为 1）。
            let start = split.get(1).and_then(|value| parse_f64(value));
            let count = split.get(2).and_then(|value| parse_f64(value));
            let (Some(start), Some(count)) = (start, count) else {
                return;
            };
            element.loops.push(LoopBuilder {
                start_ms: start,
                iterations: (count as i64).max(1) as u32,
                commands: Vec::new(),
            });
            element.active = ActiveGroup::Loop(element.loops.len() - 1);
        }
        "F" | "S" | "R" | "M" | "MX" | "MY" | "V" | "C" | "P" => {
            if let Some(commands) = parse_value_command(split) {
                match element.active {
                    ActiveGroup::Root => element.root.extend(commands),
                    ActiveGroup::Loop(index) => element.loops[index].commands.extend(commands),
                    ActiveGroup::Trigger(index) => {
                        element.triggers[index].commands.extend(commands)
                    }
                }
            }
        }
        _ => {}
    }
}

/// 解析带数值的变换命令（可产出 1~2 条命令，`M` 会拆成 X/Y）。
fn parse_value_command(split: &[&str]) -> Option<Vec<Command>> {
    let easing = parse_f64(split.get(1)?)?.max(0.0) as u32;
    let start_ms = parse_f64(split.get(2)?)?;
    // 结束时间为空时取开始时间（瞬时命令）。
    let end_ms = match split.get(3).copied().unwrap_or("").trim() {
        "" => start_ms,
        value => parse_f64(value)?,
    };
    // osu! 构造命令时会把 end < start 钳成 start。
    let end_ms = end_ms.max(start_ms);

    let value = |index: usize| split.get(index).and_then(|value| parse_f64(value));
    let one = |start: f64| value(5).unwrap_or(start);
    let mut commands = Vec::new();
    let mut push = |property: Property, start_value: Value, end_value: Value| {
        commands.push(Command {
            property,
            easing,
            start_ms,
            end_ms,
            start_value,
            end_value,
        })
    };

    match split.first().copied()? {
        "F" => {
            let start = value(4)?;
            push(
                Property::Alpha,
                Value::Scalar(start as f32),
                Value::Scalar(one(start) as f32),
            );
        }
        "S" => {
            let start = value(4)?;
            push(
                Property::Scale,
                Value::Scalar(start as f32),
                Value::Scalar(one(start) as f32),
            );
        }
        "R" => {
            // R 命令单位是弧度，内部直接按弧度求值（旋转方向：屏幕上顺时针为正）。
            let start = value(4)?;
            push(
                Property::Rotation,
                Value::Scalar(start as f32),
                Value::Scalar(one(start) as f32),
            );
        }
        "MX" => {
            let start = value(4)?;
            push(
                Property::X,
                Value::Scalar(start as f32),
                Value::Scalar(one(start) as f32),
            );
        }
        "MY" => {
            let start = value(4)?;
            push(
                Property::Y,
                Value::Scalar(start as f32),
                Value::Scalar(one(start) as f32),
            );
        }
        "M" => {
            let start_x = value(4)?;
            let start_y = value(5)?;
            let end_x = value(6).unwrap_or(start_x);
            let end_y = value(7).unwrap_or(start_y);
            push(
                Property::X,
                Value::Scalar(start_x as f32),
                Value::Scalar(end_x as f32),
            );
            push(
                Property::Y,
                Value::Scalar(start_y as f32),
                Value::Scalar(end_y as f32),
            );
        }
        "V" => {
            let start_x = value(4)?;
            let start_y = value(5)?;
            let end_x = value(6).unwrap_or(start_x);
            let end_y = value(7).unwrap_or(start_y);
            push(
                Property::VectorScale,
                Value::Vector(start_x as f32, start_y as f32),
                Value::Vector(end_x as f32, end_y as f32),
            );
        }
        "C" => {
            let (r, g, b) = (value(4)?, value(5)?, value(6)?);
            let (er, eg, eb) = (
                value(7).unwrap_or(r),
                value(8).unwrap_or(g),
                value(9).unwrap_or(b),
            );
            push(
                Property::Colour,
                Value::Rgb((r / 255.0) as f32, (g / 255.0) as f32, (b / 255.0) as f32),
                Value::Rgb((er / 255.0) as f32, (eg / 255.0) as f32, (eb / 255.0) as f32),
            );
        }
        "P" => {
            // P 命令：零时长永久生效，有时长在区间内生效、end 时刻恢复（osu! 语义）。
            let flag = match split.get(4).copied().unwrap_or("").trim() {
                "A" => Property::Additive,
                "H" => Property::FlipH,
                "V" => Property::FlipV,
                _ => return None,
            };
            let end = Value::Flag(start_ms == end_ms);
            push(flag, Value::Flag(true), end);
        }
        _ => return None,
    }
    Some(commands)
}

/// `[Variables]` 的单轮文本替换：每个变量全量替换一次、不递归。
fn substitute_variables(line: &str, variables: &[(String, String)]) -> String {
    if !line.contains('$') {
        return line.to_string();
    }
    let mut result = line.to_string();
    for (key, value) in variables {
        if result.contains(key.as_str()) {
            result = result.replace(key.as_str(), value);
        }
    }
    result
}

/// 从首行解析 `osu file format v{n}`。
fn parse_header_version(text: &str) -> Option<i32> {
    let first = text.lines().next()?.trim();
    first.strip_prefix("osu file format v")?.trim().parse().ok()
}

/// 解析数值：空白或非法返回 `None`（该行按 osu! 行为跳过）。
fn parse_f64(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    parsed.is_finite().then_some(parsed)
}

/// 解析事件坐标：钳制到 osu! 的 ±131072。
fn parse_coordinate(value: &str) -> Option<f32> {
    let parsed = parse_f64(value)? as f32;
    Some(parsed.clamp(-MAX_COORDINATE_VALUE, MAX_COORDINATE_VALUE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件行拼装测试文本（测试辅助，仅测试构建使用）。
    #[cfg(test)]
    fn storyboard_of(events: &str) -> Storyboard {
        parse_storyboard(&format!("[Events]\n{events}"), None)
    }

    /// Sprite 头行解析：层/原点/路径/坐标，路径反斜杠归一化。
    #[test]
    fn sprite_header_parses_layer_origin_path_and_position() {
        let storyboard = storyboard_of(r#"Sprite,Foreground,Centre,"SB\Lyrics\1.png",320,240"#);
        let elements = &storyboard.layers[0].elements;
        assert_eq!(elements.len(), 1);
        let element = &elements[0];
        assert_eq!(element.layer, Layer::Foreground);
        assert_eq!(element.origin, Origin::Centre);
        assert_eq!(element.path, "SB/Lyrics/1.png");
        assert_eq!((element.x, element.y), (320.0, 240.0));
    }

    /// 命令解析：M 拆成 X/Y、空结束时间为瞬时命令、单值命令两端同值。
    #[test]
    fn commands_parse_into_per_property_lists() {
        let storyboard = storyboard_of(
            "Sprite,Foreground,Centre,\"a.png\",320,240\n\
             _M,0,100,200,10,20,30,40\n\
             _F,0,300,,0.5\n\
             _S,1,400,500,2",
        );
        let element = &storyboard.layers[0].elements[0];
        let x = element.commands.list(Property::X);
        assert_eq!(x.len(), 1);
        assert_eq!(x[0].start_ms, 100.0);
        assert_eq!(x[0].end_ms, 200.0);
        assert_eq!(x[0].start_value, Value::Scalar(10.0));
        assert_eq!(x[0].end_value, Value::Scalar(30.0));
        let alpha = element.commands.list(Property::Alpha);
        assert_eq!(alpha[0].end_ms, 300.0, "空结束时间取开始时间");
        assert_eq!(alpha[0].end_value, Value::Scalar(0.5), "单值命令两端同值");
        let scale = element.commands.list(Property::Scale);
        assert_eq!(scale[0].easing, 1);
    }

    /// 循环组展开：周期 = max(end) - min(start)，总次数 = loopCount。
    #[test]
    fn loop_groups_expand_with_lazer_period() {
        let storyboard = storyboard_of(
            "Sprite,Foreground,Centre,\"a.png\",320,240\n\
             _L,1000,3\n\
             __F,0,0,100,0,1\n\
             __F,0,50,150,1,0",
        );
        let alpha = storyboard.layers[0].elements[0].commands.list(Property::Alpha);
        assert_eq!(alpha.len(), 6, "2 条命令 × 3 次播放");
        // 周期 = 150 - 0 = 150；第 2 次播放整体 +150、第 3 次 +300。
        assert_eq!(alpha[2].start_ms, 1150.0);
        assert_eq!(alpha[2].end_ms, 1250.0);
        assert_eq!(alpha[5].end_ms, 1450.0);
    }

    /// 触发组命令不进入求值列表、原样保留。
    #[test]
    fn trigger_groups_are_preserved_but_not_evaluated() {
        let storyboard = storyboard_of(
            "Sprite,Foreground,Centre,\"a.png\",320,240\n\
             _T,HitSoundClap,0,1000,2\n\
             __F,0,0,100,0,1",
        );
        let element = &storyboard.layers[0].elements[0];
        assert!(element.commands.list(Property::Alpha).is_empty());
        assert_eq!(element.commands.triggers.len(), 1);
        let trigger = &element.commands.triggers[0];
        assert_eq!(trigger.name, "HitSoundClap");
        assert_eq!(trigger.group_number, -2, "分组号按 stable 怪癖取负");
        assert_eq!(trigger.commands.len(), 1);
    }

    /// [Variables] 做单轮文本替换。
    #[test]
    fn variables_substitute_once_per_definition() {
        let storyboard = parse_storyboard(
            "[Variables]\n$a=sprite\n[Events]\nSprite,Foreground,Centre,\"$a.png\",320,240",
            None,
        );
        assert_eq!(storyboard.layers[0].elements[0].path, "sprite.png");
    }

    /// Animation 头行解析（含可选 loopType）。
    #[test]
    fn animation_header_parses_frames_and_loop_type() {
        let storyboard = storyboard_of("Animation,Background,TopLeft,\"a.png\",0,0,4,120,LoopOnce");
        match &storyboard.layers[0].elements[0].kind {
            ElementKind::Animation {
                frame_count,
                frame_delay_ms,
                loop_type,
            } => {
                assert_eq!(*frame_count, 4);
                assert_eq!(*frame_delay_ms, 120.0);
                assert_eq!(*loop_type, AnimationLoop::LoopOnce);
            }
            kind => panic!("必须是动画元素，实际 {kind:?}"),
        }
    }

    /// Sample 事件解析保留（不绘制）。
    #[test]
    fn sample_events_are_collected() {
        let storyboard = storyboard_of("Sample,1234,Foreground,\"a.wav\",80");
        assert!(storyboard.layers.iter().all(|layer| layer.elements.is_empty()));
        assert_eq!(storyboard.samples.len(), 1);
        assert_eq!(storyboard.samples[0].time_ms, 1234.0);
        assert_eq!(storyboard.samples[0].volume, 80);
    }

    /// .osb 元素排在 .osu 元素之后（osu! 主/次流顺序）。
    #[test]
    fn osb_elements_come_after_osu_events() {
        let storyboard = parse_storyboard(
            "[Events]\nSprite,Foreground,Centre,\"a.png\",0,0",
            Some("[Events]\nSprite,Foreground,Centre,\"b.png\",0,0"),
        );
        let elements = &storyboard.layers[0].elements;
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].path, "a.png");
        assert_eq!(elements[1].path, "b.png");
    }

    /// [General] 的宽屏与皮肤贴图开关。
    #[test]
    fn general_flags_are_parsed() {
        let storyboard = parse_storyboard(
            "[General]\nWidescreenStoryboard: 1\nUseSkinSprites: 1\n[Events]",
            None,
        );
        assert!(storyboard.widescreen);
        assert!(storyboard.use_skin_sprites);
    }
}

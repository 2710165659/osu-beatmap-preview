//! 故事板命令的时间求值：缓动函数、命令覆盖规则与逐帧状态。
//!
//! 求值是纯函数 `f(absolute_ms) -> ElementState`：MP4/GIF 出帧是并行且可能乱序的，
//! 不允许依赖渲染顺序（与 `render::visibility` 的时间线约定一致）。
//!
//! 命令覆盖规则（对齐 osu.Framework 的 Transform 调度）：
//! - 每个属性的命令序列稳定排序 `(start, end)`，同键保留文件定义顺序；
//! - 任一时刻取「已开始命令中排序最后一条」：`t` 在其区间内则按 easing 插值，
//!   已过区间则保持其 `end_value`；尚无命令开始时取最早命令的 `start_value`
//!   （osu! 的 `ApplyInitialValue` 语义）。
//! - `P,,H/V/A` 这类布尔命令是步进语义：`start` 时刻跳到 `start_value`，
//!   `end` 时刻跳到 `end_value`；且仅瞬时命令会把初值提前生效（lazer 的
//!   `ApplyInitialValue` 只在 `StartTime == EndTime` 时赋值）。

use super::{Command, Element, ElementKind, Property, Value, AnimationLoop};

/// 元素在某一时刻的完整状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementState {
    /// 位置（640×480 虚拟坐标）。
    pub x: f32,
    pub y: f32,
    /// 均匀缩放（`S` 命令）。
    pub scale: f32,
    /// 矢量缩放（`V` 命令）。
    pub vector_scale: [f32; 2],
    /// 旋转弧度，屏幕上顺时针为正（osu! 的 `R` 命令即弧度）。
    pub rotation: f32,
    /// 颜色调制（0..1 的 RGB）。
    pub colour: [f32; 3],
    /// 不透明度（已应用 osu! 的 `alpha > 1` 取模怪癖）。
    pub alpha: f32,
    /// 加色混合（`P,,A`）。
    pub additive: bool,
    /// 水平翻转（`P,,H`）。
    pub flip_h: bool,
    /// 垂直翻转（`P,,V`）。
    pub flip_v: bool,
    /// 动画元素的当前帧序号。
    pub frame_index: u32,
}

impl ElementState {
    /// 无任何命令时的默认状态（osu! 的 Drawable 默认值 + 元素声明位置）。
    fn default_state(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            scale: 1.0,
            vector_scale: [1.0, 1.0],
            rotation: 0.0,
            colour: [1.0, 1.0, 1.0],
            alpha: 1.0,
            additive: false,
            flip_h: false,
            flip_v: false,
            frame_index: 0,
        }
    }
}

impl Element {
    /// 求值 `time_ms`（谱面绝对毫秒）时刻的元素状态。
    pub fn state_at(&self, time_ms: f64) -> ElementState {
        let lists = &self.commands;
        let mut state = ElementState::default_state(self.x, self.y);

        state.x = scalar_at(lists.list(Property::X), time_ms, state.x);
        state.y = scalar_at(lists.list(Property::Y), time_ms, state.y);
        state.scale = scalar_at(lists.list(Property::Scale), time_ms, state.scale);
        state.vector_scale =
            vector_at(lists.list(Property::VectorScale), time_ms, state.vector_scale);
        state.rotation = scalar_at(lists.list(Property::Rotation), time_ms, state.rotation);
        state.colour = rgb_at(lists.list(Property::Colour), time_ms, state.colour);
        state.alpha = scalar_at(lists.list(Property::Alpha), time_ms, state.alpha);
        state.additive = flag_at(lists.list(Property::Additive), time_ms);
        state.flip_h = flag_at(lists.list(Property::FlipH), time_ms);
        state.flip_v = flag_at(lists.list(Property::FlipV), time_ms);

        // osu! 复刻 stable 的闪烁黑科技：alpha 超过 1 会对 1 取模（很多 storyboard
        // 依赖该行为制造闪烁），见 DrawableStoryboardSprite.Update。
        if state.alpha > 1.0 {
            state.alpha %= 1.0;
        }

        state.frame_index = self.frame_index_at(time_ms);
        state
    }

    /// 动画元素在 `time_ms` 的帧序号。
    ///
    /// 帧时间轴锚定元素最早命令开始时间（osu! 的 `EarliestTransformTime`），
    /// 第 k 帧显示于 `[k*frame_delay, (k+1)*frame_delay)`；`LoopForever` 循环，
    /// `LoopOnce` 停在最后一帧。
    fn frame_index_at(&self, time_ms: f64) -> u32 {
        let ElementKind::Animation {
            frame_count,
            frame_delay_ms,
            loop_type,
        } = self.kind
        else {
            return 0;
        };
        if frame_count == 0 || frame_delay_ms <= 0.0 {
            return 0;
        }
        let anchor = self.earliest_start_ms();
        if !anchor.is_finite() {
            return 0;
        }
        let elapsed = (time_ms - anchor).max(0.0);
        let frame = (elapsed / frame_delay_ms).floor();
        if !frame.is_finite() || frame < 0.0 {
            return 0;
        }
        let frame = frame as u64;
        match loop_type {
            AnimationLoop::LoopForever => (frame % u64::from(frame_count)) as u32,
            AnimationLoop::LoopOnce => frame.min(u64::from(frame_count - 1)) as u32,
        }
    }
}

/// 定位 `time_ms` 时刻生效的命令：已开始命令中排序最后一条的下标。
fn active_index(list: &[Command], time_ms: f64) -> Option<usize> {
    let started = list.partition_point(|command| command.start_ms <= time_ms);
    started.checked_sub(1)
}

fn scalar_at(list: &[Command], time_ms: f64, default: f32) -> f32 {
    let Some(first) = list.first() else {
        return default;
    };
    let Some(index) = active_index(list, time_ms) else {
        return scalar_value(first.start_value, default);
    };
    let command = &list[index];
    let start = scalar_value(command.start_value, default);
    let end = scalar_value(command.end_value, default);
    interpolate(start, end, command, time_ms)
}

fn vector_at(list: &[Command], time_ms: f64, default: [f32; 2]) -> [f32; 2] {
    let Some(first) = list.first() else {
        return default;
    };
    let Some(index) = active_index(list, time_ms) else {
        return vector_value(first.start_value, default);
    };
    let command = &list[index];
    let start = vector_value(command.start_value, default);
    let end = vector_value(command.end_value, default);
    let eased = eased_ratio(command, time_ms);
    [
        start[0] + eased * (end[0] - start[0]),
        start[1] + eased * (end[1] - start[1]),
    ]
}

fn rgb_at(list: &[Command], time_ms: f64, default: [f32; 3]) -> [f32; 3] {
    let Some(first) = list.first() else {
        return default;
    };
    let Some(index) = active_index(list, time_ms) else {
        return rgb_value(first.start_value, default);
    };
    let command = &list[index];
    let start = rgb_value(command.start_value, default);
    let end = rgb_value(command.end_value, default);
    // osu! 的颜色插值在 linear RGB 空间进行（gamma 校正），且插值进度截断到 [0,1]，
    // Back/Elastic 等过冲缓动不会让颜色过冲（数值属性则允许过冲）。
    let t = eased_ratio(command, time_ms).clamp(0.0, 1.0);
    let mut result = [0.0; 3];
    for channel in 0..3 {
        let a = srgb_to_linear(start[channel]);
        let b = srgb_to_linear(end[channel]);
        result[channel] = linear_to_srgb(a + t * (b - a));
    }
    result
}

fn flag_at(list: &[Command], time_ms: f64) -> bool {
    let Some(first) = list.first() else {
        return false;
    };
    let Some(index) = active_index(list, time_ms) else {
        // lazer 的布尔命令只有瞬时命令会把初值提前生效（ApplyInitialValue）。
        return if first.is_instant() {
            flag_value(first.start_value)
        } else {
            false
        };
    };
    let command = &list[index];
    if command.is_instant() || time_ms < command.end_ms {
        flag_value(command.start_value)
    } else {
        flag_value(command.end_value)
    }
}

/// 命令内的插值进度：`t` 在 `[start,end]` 内按 easing 映射，区间外不进入本函数。
fn eased_ratio(command: &Command, time_ms: f64) -> f32 {
    let duration = command.end_ms - command.start_ms;
    if duration <= 0.0 {
        return 1.0;
    }
    let ratio = ((time_ms - command.start_ms) / duration) as f32;
    apply_easing(command.easing, ratio)
}

/// 数值属性插值：不截断进度，允许 Back/Elastic 等缓动过冲（与 osu! 一致）。
fn interpolate(start: f32, end: f32, command: &Command, time_ms: f64) -> f32 {
    if command.is_instant() || time_ms >= command.end_ms {
        return end;
    }
    start + eased_ratio(command, time_ms) * (end - start)
}

fn scalar_value(value: Value, default: f32) -> f32 {
    match value {
        Value::Scalar(v) => v,
        _ => default,
    }
}

fn vector_value(value: Value, default: [f32; 2]) -> [f32; 2] {
    match value {
        Value::Vector(x, y) => [x, y],
        _ => default,
    }
}

fn rgb_value(value: Value, default: [f32; 3]) -> [f32; 3] {
    match value {
        Value::Rgb(r, g, b) => [r, g, b],
        _ => default,
    }
}

fn flag_value(value: Value) -> bool {
    match value {
        Value::Flag(flag) => flag,
        _ => false,
    }
}

/// sRGB → linear（osu.Framework `Color4Extensions.ToLinear` 同公式）。
fn srgb_to_linear(color: f32) -> f32 {
    if color == 1.0 {
        return 1.0;
    }
    if color <= 0.04045 {
        color / 12.92
    } else {
        ((color + 0.055) / 1.055).powf(2.4)
    }
}

/// linear → sRGB（osu.Framework `Color4Extensions.ToSRGB` 同公式）。
fn linear_to_srgb(color: f32) -> f32 {
    if color == 1.0 {
        return 1.0;
    }
    if color < 0.0031308 {
        12.92 * color
    } else {
        1.055 * color.powf(1.0 / 2.4) - 0.055
    }
}

/// 应用缓动：`easing` 编号与 osu.Framework `Easing` 枚举序号一致，公式逐条
/// 移植自 `DefaultEasingFunction.ApplyEasing`（含 Expo/Elastic 的端点修正常量）。
pub fn apply_easing(easing: u32, time: f32) -> f32 {
    let t = time as f64;
    let value = apply_easing_f64(easing, t);
    value as f32
}

/// [`apply_easing`] 的 f64 版本（宿主求值时间是 f64）。
pub fn apply_easing_f64(easing: u32, time: f64) -> f64 {
    const ELASTIC_CONST: f64 = 2.0 * std::f64::consts::PI / 0.3;
    const ELASTIC_CONST_2: f64 = 0.3 / 4.0;
    const BACK_CONST: f64 = 1.70158;
    const BACK_CONST_2: f64 = BACK_CONST * 1.525;
    const BOUNCE_CONST: f64 = 1.0 / 2.75;
    // Expo/Elastic 曲线端点修正：保证 0/1 处精确落点（与 osu.Framework 相同）。
    let expo_offset = 2f64.powi(-10);
    let elastic_offset_full = 2f64.powi(-11);
    let elastic_offset_half = 2f64.powi(-10) * ((0.5 - ELASTIC_CONST_2) * ELASTIC_CONST).sin();
    let elastic_offset_quarter = 2f64.powi(-10) * ((0.25 - ELASTIC_CONST_2) * ELASTIC_CONST).sin();
    let in_out_elastic_offset =
        2f64.powi(-10) * ((1.0 - ELASTIC_CONST_2 * 1.5) * ELASTIC_CONST / 1.5).sin();

    let mut t = time;
    match easing {
        // 0 None：线性。
        2 | 3 => t * t,                    // In / InQuad
        1 | 4 => t * (2.0 - t),            // Out / OutQuad
        5 => {
            if t < 0.5 {
                t * t * 2.0
            } else {
                t -= 1.0;
                t * t * -2.0 + 1.0
            }
        }
        6 => t * t * t,                    // InCubic
        7 => {
            t -= 1.0;
            t * t * t + 1.0
        }
        8 => {
            if t < 0.5 {
                t * t * t * 4.0
            } else {
                t -= 1.0;
                t * t * t * 4.0 + 1.0
            }
        }
        9 => t * t * t * t,                // InQuart
        10 => {
            t -= 1.0;
            1.0 - t * t * t * t
        }
        11 => {
            if t < 0.5 {
                t * t * t * t * 8.0
            } else {
                t -= 1.0;
                t * t * t * t * -8.0 + 1.0
            }
        }
        12 => t * t * t * t * t,           // InQuint
        13 => {
            t -= 1.0;
            t * t * t * t * t + 1.0
        }
        14 => {
            if t < 0.5 {
                t * t * t * t * t * 16.0
            } else {
                t -= 1.0;
                t * t * t * t * t * 16.0 + 1.0
            }
        }
        15 => 1.0 - (t * std::f64::consts::PI * 0.5).cos(),  // InSine
        16 => (t * std::f64::consts::PI * 0.5).sin(),        // OutSine
        17 => 0.5 - 0.5 * (std::f64::consts::PI * t).cos(),  // InOutSine
        18 => 2f64.powf(10.0 * (t - 1.0)) + expo_offset * (t - 1.0), // InExpo
        19 => -2f64.powf(-10.0 * t) + 1.0 + expo_offset * t,         // OutExpo
        20 => {
            if t < 0.5 {
                0.5 * (2f64.powf(20.0 * t - 10.0) + expo_offset * (2.0 * t - 1.0))
            } else {
                1.0 - 0.5 * (2f64.powf(-20.0 * t + 10.0) + expo_offset * (-2.0 * t + 1.0))
            }
        }
        21 => 1.0 - (1.0 - t * t).sqrt(),  // InCirc
        22 => {
            t -= 1.0;
            (1.0 - t * t).sqrt()
        }
        23 => {
            t *= 2.0;
            if t < 1.0 {
                0.5 - 0.5 * (1.0 - t * t).sqrt()
            } else {
                t -= 2.0;
                0.5 * (1.0 - t * t).sqrt() + 0.5
            }
        }
        24 => {
            // InElastic
            -2f64.powf(-10.0 + 10.0 * t) * ((1.0 - ELASTIC_CONST_2 - t) * ELASTIC_CONST).sin()
                + elastic_offset_full * (1.0 - t)
        }
        25 => {
            // OutElastic
            2f64.powf(-10.0 * t) * ((t - ELASTIC_CONST_2) * ELASTIC_CONST).sin() + 1.0
                - elastic_offset_full * t
        }
        26 => {
            // OutElasticHalf
            2f64.powf(-10.0 * t) * ((0.5 * t - ELASTIC_CONST_2) * ELASTIC_CONST).sin() + 1.0
                - elastic_offset_half * t
        }
        27 => {
            // OutElasticQuarter
            2f64.powf(-10.0 * t) * ((0.25 * t - ELASTIC_CONST_2) * ELASTIC_CONST).sin() + 1.0
                - elastic_offset_quarter * t
        }
        28 => {
            // InOutElastic
            t *= 2.0;
            if t < 1.0 {
                -0.5 * (2f64.powf(-10.0 + 10.0 * t)
                    * ((1.0 - ELASTIC_CONST_2 * 1.5 - t) * ELASTIC_CONST / 1.5).sin()
                    - in_out_elastic_offset * (1.0 - t))
            } else {
                t -= 1.0;
                0.5 * (2f64.powf(-10.0 * t)
                    * ((t - ELASTIC_CONST_2 * 1.5) * ELASTIC_CONST / 1.5).sin()
                    - in_out_elastic_offset * t)
                    + 1.0
            }
        }
        29 => t * t * ((BACK_CONST + 1.0) * t - BACK_CONST), // InBack
        30 => {
            t -= 1.0;
            t * t * ((BACK_CONST + 1.0) * t + BACK_CONST) + 1.0
        }
        31 => {
            // InOutBack
            t *= 2.0;
            if t < 1.0 {
                0.5 * t * t * ((BACK_CONST_2 + 1.0) * t - BACK_CONST_2)
            } else {
                t -= 2.0;
                0.5 * (t * t * ((BACK_CONST_2 + 1.0) * t + BACK_CONST_2) + 2.0)
            }
        }
        32 => {
            // InBounce
            let u = 1.0 - t;
            1.0 - out_bounce(u, BOUNCE_CONST)
        }
        33 => out_bounce(t, BOUNCE_CONST), // OutBounce
        34 => {
            // InOutBounce
            if t < 0.5 {
                0.5 - 0.5 * out_bounce(1.0 - t * 2.0, BOUNCE_CONST)
            } else {
                out_bounce((t - 0.5) * 2.0, BOUNCE_CONST) * 0.5 + 0.5
            }
        }
        35 => {
            // OutPow10：osu! 实现里 `--time * Math.Pow(time, 10) + 1` 的 time
            // 已被前置自减，等价于 (t-1)^11 + 1。
            t -= 1.0;
            t * t.powi(10) + 1.0
        }
        // 其余（含 0 None）线性。
        _ => t,
    }
}

/// OutBounce 分段曲线（osu.Framework `DefaultEasingFunction` 同公式）。
fn out_bounce(t: f64, bounce_const: f64) -> f64 {
    let mut t = t;
    if t < bounce_const {
        7.5625 * t * t
    } else if t < 2.0 * bounce_const {
        t -= 1.5 * bounce_const;
        7.5625 * t * t + 0.75
    } else if t < 2.5 * bounce_const {
        t -= 2.25 * bounce_const;
        7.5625 * t * t + 0.9375
    } else {
        t -= 2.625 * bounce_const;
        7.5625 * t * t + 0.984375
    }
}

/// 由测试直接构造命令求值入口（生产路径走解析器的 `from_sorted`）。
#[cfg(test)]
pub(crate) fn build_commands(
    lists: Vec<Vec<Command>>,
    triggers: Vec<super::TriggerGroup>,
) -> super::ElementCommands {
    super::ElementCommands::from_sorted(lists, triggers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storyboard::{Layer, Origin, TriggerGroup, PROPERTY_COUNT};

    /// 造一个带指定命令的静态元素：命令按属性分桶（测试辅助，仅测试构建使用）。
    #[cfg(test)]
    fn element_with(command_groups: Vec<Vec<Command>>) -> Element {
        let mut lists = vec![Vec::<Command>::new(); PROPERTY_COUNT];
        for group in command_groups {
            for command in group {
                lists[command.property.index()].push(command);
            }
        }
        Element {
            kind: ElementKind::Sprite,
            layer: Layer::Background,
            path: "sb/x.png".to_string(),
            origin: Origin::Centre,
            x: 320.0,
            y: 240.0,
            commands: build_commands(lists, Vec::<TriggerGroup>::new()),
        }
    }

    /// 造一条标量命令（测试辅助，仅测试构建使用）。
    #[cfg(test)]
    fn scalar(
        property: Property,
        easing: u32,
        start_ms: f64,
        end_ms: f64,
        start: f32,
        end: f32,
    ) -> Command {
        Command {
            property,
            easing,
            start_ms,
            end_ms,
            start_value: Value::Scalar(start),
            end_value: Value::Scalar(end),
        }
    }

    /// 缓动 0/1/2/3 与 osu.Framework 公式一致（线性/Out/In/InQuad）。
    #[test]
    fn easing_matches_osu_framework_formulas() {
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let t64 = f64::from(t);
            assert!((apply_easing(0, t) - t).abs() < 1e-6, "线性 {t}");
            assert!((apply_easing(1, t) - (t64 * (2.0 - t64)) as f32).abs() < 1e-6, "Out {t}");
            assert!((apply_easing(2, t) - (t64 * t64) as f32).abs() < 1e-6, "In {t}");
            assert!((apply_easing(3, t) - (t64 * t64) as f32).abs() < 1e-6, "InQuad {t}");
        }
        // OutPow10 端点精确。
        assert!((apply_easing(35, 1.0) - 1.0).abs() < 1e-6);
        assert!((apply_easing(35, 0.0) - 0.0).abs() < 1e-5);
    }

    /// 命令前取最早命令的 startValue，命令后保持 endValue，中间按 easing 插值。
    #[test]
    fn scalar_command_holds_initial_and_end_values() {
        let element = element_with(vec![vec![scalar(
            Property::X,
            0,
            1000.0,
            2000.0,
            0.0,
            100.0,
        )]]);
        let state = element.state_at(0.0);
        assert_eq!(state.x, 0.0, "命令开始前显示最早命令的 startValue");
        let state = element.state_at(1500.0);
        assert!((state.x - 50.0).abs() < 1e-3, "中点线性插值，实际 {}", state.x);
        let state = element.state_at(3000.0);
        assert_eq!(state.x, 100.0, "命令结束后保持 endValue");
    }

    /// 同属性重叠时，后开始的命令完全覆盖先开始的（osu.Framework 规则）。
    #[test]
    fn later_command_overrides_earlier_overlap() {
        let element = element_with(vec![vec![
            scalar(Property::Alpha, 0, 0.0, 1000.0, 1.0, 0.0),
            scalar(Property::Alpha, 0, 500.0, 1500.0, 0.5, 0.5),
        ]]);
        assert!((element.state_at(750.0).alpha - 0.5).abs() < 1e-3);
        assert!((element.state_at(2000.0).alpha - 0.5).abs() < 1e-3);
    }

    /// alpha 超过 1 对 1 取模（stable 闪烁黑科技）。
    #[test]
    fn alpha_over_one_wraps_by_modulo() {
        let element = element_with(vec![vec![scalar(
            Property::Alpha,
            0,
            0.0,
            1000.0,
            1.5,
            1.5,
        )]]);
        assert!((element.state_at(500.0).alpha - 0.5).abs() < 1e-3);
    }

    /// 布尔命令：瞬时命令提前生效初值，有时长命令在区间内生效、end 恢复。
    #[test]
    fn flag_commands_follow_step_semantics() {
        let instant = Command {
            property: Property::FlipH,
            easing: 0,
            start_ms: 1000.0,
            end_ms: 1000.0,
            start_value: Value::Flag(true),
            end_value: Value::Flag(true),
        };
        let element = element_with(vec![vec![instant]]);
        assert!(element.state_at(0.0).flip_h, "瞬时命令的初值提前生效");
        assert!(element.state_at(5000.0).flip_h);

        let windowed = Command {
            property: Property::FlipV,
            easing: 0,
            start_ms: 1000.0,
            end_ms: 2000.0,
            start_value: Value::Flag(true),
            end_value: Value::Flag(false),
        };
        let element = element_with(vec![vec![windowed]]);
        assert!(!element.state_at(0.0).flip_v, "有时长命令开始前保持默认");
        assert!(element.state_at(1500.0).flip_v);
        assert!(!element.state_at(2500.0).flip_v);
    }

    /// 动画帧锚定元素最早命令时间；LoopOnce 停在最后一帧。
    #[test]
    fn animation_frames_anchor_at_earliest_command() {
        let mut element = element_with(vec![vec![scalar(
            Property::Alpha,
            0,
            500.0,
            1000.0,
            1.0,
            1.0,
        )]]);
        element.kind = ElementKind::Animation {
            frame_count: 3,
            frame_delay_ms: 100.0,
            loop_type: AnimationLoop::LoopOnce,
        };
        assert_eq!(element.state_at(500.0).frame_index, 0);
        assert_eq!(element.state_at(750.0).frame_index, 2);
        assert_eq!(element.state_at(2000.0).frame_index, 2, "LoopOnce 停在最后一帧");

        element.kind = ElementKind::Animation {
            frame_count: 3,
            frame_delay_ms: 100.0,
            loop_type: AnimationLoop::LoopForever,
        };
        assert_eq!(element.state_at(800.0).frame_index, 0, "LoopForever 循环回第一帧");
    }

    /// 颜色插值在 linear 空间（gamma 校正），与 osu! 一致。
    #[test]
    fn colour_interpolates_in_linear_space() {
        let command = Command {
            property: Property::Colour,
            easing: 0,
            start_ms: 0.0,
            end_ms: 1000.0,
            start_value: Value::Rgb(0.0, 0.0, 0.0),
            end_value: Value::Rgb(1.0, 1.0, 1.0),
        };
        let element = element_with(vec![vec![command]]);
        let mid = element.state_at(500.0).colour[0];
        // linear 空间中点再转回 sRGB 约 0.735；sRGB 直插则为 0.5。
        assert!((mid - 0.7354).abs() < 5e-3, "中点应为 gamma 校正值，实际 {mid}");
    }
}

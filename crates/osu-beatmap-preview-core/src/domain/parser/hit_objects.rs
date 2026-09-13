//! 四种游戏模式的音符对象解析。
//!
//! 各模式解析器将原始 CSV 行转换为对应的类型化音符向量。
//! 滑条结束时间计算（包括 timing point 查找）由 standard、taiko、catch 共用。

use crate::domain::models::*;

pub(crate) struct SliderFields {
    pub slider_type: String,
    pub points: Vec<(i32, i32)>,
    pub repeats: i32,
    pub pixel_length: f64,
    pub edge_hitsounds: Vec<i32>,
    pub edge_sets: Vec<String>,
}

/// `.osu` 对象行 `hitSample` 字段的解析结果。
#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedHitSample {
    normal_set: i32,
    addition_set: i32,
    volume: Option<i32>,
    filename: Option<String>,
}

impl ParsedHitSample {
    /// 转换为「普通打击音 + 所有加成音」。
    ///
    /// `.osu` 里 `hitSample` 全为 0（最常见的情况）表示「沿用当前 timing point 的
    /// 音效组、音量与自定义音效」，因此这里返回空切片，由打击音模块按物件所在时间
    /// 查找 timing point 决定实际参数——在解析阶段就写死会用到错误的时间点信息。
    pub(crate) fn samples(&self, hitsound: i32) -> Vec<HitSample> {
        if self.is_default() {
            return Vec::new();
        }

        let normal_bank = if self.normal_set == 0 {
            SampleBank::Auto
        } else {
            SampleBank::from_set_id(self.normal_set)
        };
        let mut samples = vec![self.build(normal_bank, HitAddition::None)];
        for addition in HitAddition::all_from_hitsound(hitsound) {
            if let Some(sample) = self.addition(addition) {
                samples.push(sample);
            }
        }
        samples
    }

    /// 是否完全沿用 timing point 的音效参数。
    pub(crate) fn is_default(&self) -> bool {
        self.normal_set == 0
            && self.addition_set == 0
            && self.volume.is_none()
            && self.filename.is_none()
    }

    /// 构造带当前音量与自定义文件名的样本。
    fn build(&self, bank: SampleBank, addition: HitAddition) -> HitSample {
        HitSample::new(
            bank,
            addition,
            // 0 表示没有单独指定音量，由 timing point 继承实际值。
            self.volume.unwrap_or(0),
            self.filename.clone(),
        )
    }

    /// 生成加成音；没有加成位时返回 `None`。
    pub(crate) fn addition(&self, addition: HitAddition) -> Option<HitSample> {
        if addition == HitAddition::None {
            return None;
        }
        // 加成音效组缺省时沿用普通打击音效组，与 osu! 的 EditorAutoBank 行为一致。
        let set_id = if self.addition_set == 0 {
            self.normal_set
        } else {
            self.addition_set
        };
        let mut sample = self.build(
            if set_id == 0 {
                SampleBank::Auto
            } else {
                SampleBank::from_set_id(set_id)
            },
            addition,
        );
        // 自定义文件名只覆盖普通层；osu! 仍会按标准加成文件名播放附加层。
        sample.filename = None;
        Some(sample)
    }
}

/// 解析 `hitSample` 字段（`normalSet:additionSet:index:volume:filename`）。
pub(crate) fn parse_hit_sample(field: Option<&str>) -> ParsedHitSample {
    let Some(field) = field else {
        return ParsedHitSample::default();
    };
    if field.is_empty() {
        return ParsedHitSample::default();
    }
    let parts: Vec<&str> = field.split(':').collect();
    let normal_set = parts.first().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let addition_set = parts.get(1).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let volume = parts
        .get(3)
        .and_then(|v| v.trim().parse::<i32>().ok())
        // 音量为 0 表示「没有单独指定音量」，与整段 hitSample 全为 0 等价；
        // 归一成 `None` 后，下游就能用「是否缺省」判断要不要回退到 timing point。
        .filter(|v| *v > 0);
    // 文件名可以包含 ':'，因此把剩余部分整体还原。
    let filename = if parts.len() > 4 {
        let joined = parts[4..].join(":");
        let joined = joined.trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    } else {
        None
    };
    ParsedHitSample {
        normal_set,
        addition_set,
        volume,
        filename,
    }
}

/// 解析滑条节点音效字段。`edgeSets` 只包含采样组参数，hitsound 位掩码来自 `edgeSounds`。
pub(crate) fn parse_edge_sample(
    field: &str,
    hitsound: i32,
    fallback: &ParsedHitSample,
) -> Vec<HitSample> {
    // 空字段表示「该节点沿用 timing point 的音效」，返回空切片让打击音模块回退；
    // 这里不能退回物件自带的 hitSample，否则会把物件参数错误地当成节点参数。
    if field.is_empty() {
        return Vec::new();
    }
    let parts: Vec<&str> = field.split(':').collect();
    let normal_set = parts.first().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let addition_set = parts
        .get(1)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let volume = parts
        .get(3)
        .and_then(|v| v.trim().parse::<i32>().ok())
        // 与物件行同理：0 表示没有单独指定音量。
        .filter(|v| *v > 0);
    let filename = if parts.len() > 4 {
        let joined = parts[4..].join(":");
        let joined = joined.trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    } else {
        None
    };
    let sample = ParsedHitSample {
        normal_set: if normal_set == 0 {
            fallback.normal_set
        } else {
            normal_set
        },
        addition_set: if addition_set == 0 {
            fallback.addition_set
        } else {
            addition_set
        },
        volume: volume.or(fallback.volume),
        filename,
    };
    // 节点音效同样可能「全用默认」：此时按空切片处理，由打击音模块回退到 timing point。
    if sample.is_default() {
        return Vec::new();
    }
    sample.samples(hitsound)
}

pub(crate) fn parse_slider_fields(parts: &[&str]) -> Option<SliderFields> {
    let mut slider_parts = parts[5].split('|');
    let slider_type = slider_parts.next()?.to_string();
    let mut points = Vec::new();
    for p in slider_parts {
        let (x, y) = p.split_once(':')?;
        points.push((x.parse().ok()?, y.parse().ok()?));
    }
    let repeats: i32 = parts.get(6)?.parse().ok()?;
    let pixel_length: f64 = parts.get(7)?.parse().ok()?;
    let mut edge_hitsounds = Vec::new();
    if let Some(eh) = parts.get(8) {
        if !eh.is_empty() {
            for v in eh.split('|') {
                if !v.is_empty() {
                    edge_hitsounds.push(v.parse().ok()?);
                }
            }
        }
    }
    let mut edge_sets = Vec::new();
    if let Some(es) = parts.get(9) {
        if !es.is_empty() {
            for v in es.split('|') {
                edge_sets.push(v.to_string());
            }
        }
    }
    Some(SliderFields {
        slider_type,
        points,
        repeats,
        pixel_length,
        edge_hitsounds,
        edge_sets,
    })
}

pub(crate) fn parse_standard(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<StandardHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i32 = parts[0].parse::<f64>().ok()? as i32;
        let y: i32 = parts[1].parse::<f64>().ok()? as i32;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;
        // 滑条的 `hitSample` 在第 11 列（curve/slides/length/edgeSounds/edgeSets 之后），
        // 圆圈与转盘才在第 6 列；取错列会把曲线当成音效参数。
        let is_slider = hit_type & 2 != 0;
        let hit_sample = parse_hit_sample(parts.get(if is_slider { 10 } else { 5 }).copied());

        let mut obj = StandardHitObject {
            x,
            y,
            start_time,
            end_time,
            hit_type,
            hitsound,
            new_combo: hit_type & 4 != 0,
            combo_offset: (hit_type & 112) >> 4,
            samples: hit_sample.samples(hitsound),
            ..Default::default()
        };
        if is_slider {
            let sf = parse_slider_fields(&parts)?;
            // 滑条头的打击音来自 edgeSets[0]，与物件行的 hitSample 字段含义相同。
            if let Some(first) = sf.edge_sets.first() {
                obj.samples = parse_edge_sample(
                    first,
                    sf.edge_hitsounds.first().copied().unwrap_or(hitsound),
                    &hit_sample,
                );
            }
            obj.slider_type = Some(sf.slider_type);
            obj.slider_points = sf.points;
            obj.slider_repeats = sf.repeats;
            obj.slider_pixel_length = sf.pixel_length;
            obj.slider_edge_hitsounds = sf.edge_hitsounds.clone();
            obj.slider_edge_samples = sf
                .edge_sets
                .iter()
                .skip(1)
                .enumerate()
                .map(|(index, edge)| {
                    parse_edge_sample(
                        edge,
                        sf.edge_hitsounds
                            .get(index + 1)
                            .copied()
                            .unwrap_or(hitsound),
                        &hit_sample,
                    )
                })
                .collect();
        }
        objects.push(obj);
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

pub(crate) fn parse_taiko(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<TaikoHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;
        // 与 standard 一致：滑条的 `hitSample` 在第 11 列（原生 taiko 谱面也可能有滑条）。
        let is_slider = hit_type & 2 != 0;
        let hit_sample = parse_hit_sample(parts.get(if is_slider { 10 } else { 5 }).copied());
        objects.push(TaikoHitObject {
            start_time,
            end_time,
            hit_type,
            hitsound,
            samples: hit_sample.samples(hitsound),
        });
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

pub(crate) fn parse_catch(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<CatchHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i32 = parts[0].parse::<f64>().ok()? as i32;
        let y: i32 = parts[1].parse::<f64>().ok()? as i32;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;
        // 与 standard 一致：滑条的 `hitSample` 在第 11 列。
        let is_slider = hit_type & 2 != 0;
        let hit_sample = parse_hit_sample(parts.get(if is_slider { 10 } else { 5 }).copied());

        let mut obj = CatchHitObject {
            x,
            y,
            start_time,
            end_time,
            hit_type,
            hitsound,
            new_combo: hit_type & 4 != 0,
            combo_offset: (hit_type & 112) >> 4,
            samples: hit_sample.samples(hitsound),
            ..Default::default()
        };
        if is_slider {
            let sf = parse_slider_fields(&parts)?;
            // 滑条头沿用 edgeSets[0]，果汁流的小果与节点音效由 core 的打击音模块生成。
            if let Some(first) = sf.edge_sets.first() {
                obj.samples = parse_edge_sample(
                    first,
                    sf.edge_hitsounds.first().copied().unwrap_or(hitsound),
                    &hit_sample,
                );
            }
            obj.slider_type = Some(sf.slider_type);
            obj.slider_points = sf.points;
            obj.slider_repeats = sf.repeats;
            obj.slider_pixel_length = sf.pixel_length;
        }
        objects.push(obj);
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

pub(crate) fn parse_mania(
    lines: &[&str],
    difficulty: &KvSection,
) -> Option<Vec<ManiaHitObject>> {
    let key_count = difficulty.get_f64("CircleSize")? as i64;
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i64 = parts[0].parse::<f64>().ok()? as i64;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let lane = (x * key_count).div_euclid(512).clamp(0, key_count - 1) as i32;
        let is_long_note = hit_type & 128 != 0;
        let mut end_time = start_time;
        let sample_field = parts.get(5).and_then(|field| {
            if is_long_note {
                field.split_once(':').map(|(_, sample)| sample)
            } else {
                Some(*field)
            }
        });
        let samples = parse_hit_sample(sample_field).samples(hitsound);
        if is_long_note {
            let head = parts.get(5)?.split(':').next()?;
            end_time = head.parse().ok()?;
        }
        objects.push(ManiaHitObject {
            lane,
            start_time,
            end_time,
            is_long_note,
            samples,
        });
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

fn parse_end_time(
    parts: &[&str],
    start_time: i64,
    hit_type: i32,
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<i64> {
    if hit_type & 8 != 0 {
        return parts.get(5)?.parse::<f64>().ok().map(|v| v as i64);
    }
    if hit_type & 2 != 0 {
        return parse_slider_end_time(parts, start_time, difficulty, timing_points);
    }
    Some(start_time)
}

fn parse_slider_end_time(
    parts: &[&str],
    start_time: i64,
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<i64> {
    let slides: f64 = parts.get(6)?.parse::<i32>().ok()? as f64;
    let pixel_length: f64 = parts.get(7)?.parse().ok()?;
    let slider_multiplier = difficulty.get_f64("SliderMultiplier")?;
    let (beat_length, slider_velocity) = resolve_slider_timing(start_time, timing_points);
    let duration =
        pixel_length / (slider_multiplier * 100.0 * slider_velocity) * beat_length * slides;
    Some(start_time + round_half_even(duration))
}

/// 扫描给定开始时间之前的 timing points，解析生效的 beat_length 和 slider_velocity。
pub fn resolve_slider_timing(start_time: i64, timing_points: &[TimingPoint]) -> (f64, f64) {
    let mut beat_length = timing_points[0].beat_length;
    let mut slider_velocity = 1.0;
    for point in timing_points {
        if point.time > start_time as f64 {
            break;
        }
        if point.uninherited {
            beat_length = point.beat_length;
            slider_velocity = 1.0;
        } else if point.beat_length < 0.0 {
            slider_velocity = -100.0 / point.beat_length;
        }
    }
    (beat_length, slider_velocity)
}

// Python 的 round() 使用银行家舍入。
pub fn round_half_even(v: f64) -> i64 {
    let floor = v.floor();
    let diff = v - floor;
    if diff > 0.5 {
        floor as i64 + 1
    } else if diff < 0.5 {
        floor as i64
    } else {
        let f = floor as i64;
        if f % 2 == 0 {
            f
        } else {
            f + 1
        }
    }
}

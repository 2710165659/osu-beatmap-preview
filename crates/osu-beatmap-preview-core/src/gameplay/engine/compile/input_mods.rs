//! AL/SG 拦截动作；RX 只为缺少自动按键的 legacy 回放重建输入。
use super::*;
use crate::replay::ReplayFrame;

pub(super) fn apply(prepared: &PreparedStandard, document: &ReplayDocument) -> ReplayTimeline {
    let mods = &prepared.mods.settings;
    let mut frames = document.frames.clone();
    if mods.alternate || mods.single_tap {
        let mut last_accepted = 0;
        let mut previous_raw = 0;
        let mut accepted = 0;
        let mut previous_time = f64::NEG_INFINITY;
        for frame in &mut frames {
            let time = frame.time_ms as f64;
            let in_break = prepared
                .input_reset_periods
                .iter()
                .any(|&(start, end)| time >= start && time <= end);
            if prepared
                .input_reset_periods
                .iter()
                .any(|&(start, end)| time >= start && previous_time <= end)
            {
                last_accepted = 0;
            }
            let raw = standard_actions(frame.keys);
            accepted &= raw;
            for action in [1, 2] {
                if raw & !previous_raw & action == 0 {
                    continue;
                }
                if in_break
                    || last_accepted == 0
                    || (mods.alternate && last_accepted != action)
                    || (mods.single_tap && last_accepted == action)
                {
                    accepted |= action;
                    if !in_break {
                        last_accepted = action;
                    }
                }
            }
            previous_raw = raw;
            previous_time = time;
            frame.keys &=
                16 | if accepted & 1 != 0 { 5 } else { 0 } | if accepted & 2 != 0 { 10 } else { 0 };
        }
    }
    // AP 的回放帧已经记录自动光标；与 HR/MR 一样保留游玩坐标，不再次生成轨迹。
    ReplayTimeline::new(frames)
}

impl Compiler<'_> {
    pub(super) fn relax_update(&mut self, time: f64) {
        let Some(snapshot) = self.input.snapshot_at(time) else {
            return;
        };
        self.update_active(time);
        let cursor = snapshot.cursor.unwrap();
        let mut requires_hit = false;
        let mut requires_hold = false;
        let first = self
            .prepared
            .objects
            .partition_point(|o| (o.source.start_time as f64) < time - 400.0);
        let last = self
            .prepared
            .objects
            .partition_point(|o| o.source.start_time as f64 <= time + 12.0);
        for id in first..last {
            let object = &self.prepared.objects[id];
            if self.objects[id].end.is_some() || (object.path.is_some() && time > object.end) {
                continue;
            }
            if object.source.hit_type & 8 == 0
                && self.objects[id].head.is_none()
                && circle::in_range(cursor, object.position, self.prepared.radius)
                && circle::result(time - object.source.start_time as f64, self.prepared.od)
                    .is_some_and(|r| r.is_hit())
            {
                requires_hit = true;
            }
        }
        // 长物件可早于局部命中窗开始，使用活动集合保留它们，避免扫描整张谱面。
        for &id in &self.active {
            let object = &self.prepared.objects[id];
            if self.objects[id].end.is_some() || time > object.end {
                continue;
            }
            if object.path.is_some() {
                requires_hold |= circle::in_range(
                    cursor,
                    object.position_at(time),
                    self.prepared.radius
                        * if self.objects[id].tracking_at(time) {
                            2.4
                        } else {
                            1.0
                        },
                );
            } else if object.source.hit_type & 8 != 0 {
                requires_hold |= spinner::required(
                    object.end - object.source.start_time as f64,
                    self.prepared.od,
                )
                .0 > 0;
            }
        }
        if requires_hit && self.relax_actions != 0 {
            self.relax_frame(time, cursor, 0);
        }
        if requires_hit || requires_hold {
            if self.relax_actions == 0 {
                let action = self.relax_next;
                self.relax_next = 3 ^ action;
                self.relax_frame(time, cursor, action);
            }
        } else if self.relax_actions != 0 && time - self.relax_changed > 50.0 {
            self.relax_frame(time, cursor, 0);
        }
        // 只保留原帧与按键变化帧；变化点的坐标取原轨迹插值，不增加每毫秒帧缓存。
    }

    fn relax_frame(&mut self, time: f64, cursor: [f32; 2], actions: u32) {
        self.relax_actions = actions;
        self.relax_changed = time;
        let frame = self.record_relax_frame(time, cursor);
        self.process_frame(frame, time);
    }

    pub(super) fn record_relax_frame(&mut self, time: f64, cursor: [f32; 2]) -> ReplayFrame {
        let previous = self
            .relaxed_frames
            .last()
            .map_or(time as i64, |f| f.time_ms);
        let frame = ReplayFrame {
            time_ms: time as i64,
            delta_ms: time as i64 - previous,
            cursor,
            keys: self.relax_actions,
            sequence: self.relaxed_frames.len(),
        };
        self.relaxed_frames.push(frame);
        frame
    }
}

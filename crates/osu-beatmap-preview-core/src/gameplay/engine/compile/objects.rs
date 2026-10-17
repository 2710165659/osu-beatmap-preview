//! 物件状态推进；原帧坐标与按键逐个消费，显示快照只用于插值区间。
use super::*;
impl Compiler<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit(
        &mut self,
        id: usize,
        node: Option<usize>,
        time: f64,
        sample_time: f64,
        result: HitResult,
        maximum: HitResult,
        error: Option<f64>,
    ) {
        self.events.push(GameplayEvent {
            id: 0,
            object_id: id,
            node_id: node,
            time,
            sample_time,
            result,
            maximum,
            kind: EventKind::Judgement,
            error,
            position: None,
        });
    }
    pub(super) fn input_frame(&mut self, frame_index: usize, time: f64) {
        let mut frame = self.input.frames[frame_index];
        if self.legacy_relax {
            frame = self.record_relax_frame(time, frame.cursor);
        }
        self.process_frame(frame, time);
    }
    pub(super) fn process_frame(&mut self, frame: crate::replay::ReplayFrame, time: f64) {
        while self.first_unjudged < self.objects.len()
            && self.objects[self.first_unjudged].end.is_some()
        {
            self.first_unjudged += 1;
        }
        let actions = standard_actions(frame.keys);
        let pressed = actions & !self.previous_actions;
        self.previous_actions = actions;
        for action in [1, 2] {
            if pressed & action == 0 {
                continue;
            }
            let first = self
                .prepared
                .objects
                .partition_point(|o| (o.source.start_time as f64) < time - 400.0);
            let last = self
                .prepared
                .objects
                .partition_point(|o| o.source.start_time as f64 <= time + 400.0);
            for id in first..last {
                let object = &self.prepared.objects[id];
                if object.source.hit_type & 8 != 0 {
                    continue;
                }
                if self.objects[id].head.is_some() {
                    continue;
                }
                let error = time - object.source.start_time as f64;
                if error < -400.0 {
                    break;
                }
                if error > 400.0
                    || !circle::in_range(frame.cursor, object.position, self.prepared.radius)
                {
                    continue;
                }
                if let Some(result) = circle::result(error, self.prepared.od) {
                    if self.policy.classic_note_lock
                        && !self.classic_hittable(id, time, frame.cursor)
                    {
                        break;
                    }
                    let classic_slider = self.policy.classic_sliders && object.path.is_some();
                    // 官方 CL 将滑条头命中窗中的所有成功判定转成 LargeTickHit，
                    // 真正的 300/100/50 留到整条滑条按嵌套节点比例结算。
                    let result = if classic_slider {
                        if result.is_hit() {
                            HitResult::LargeTickHit
                        } else {
                            HitResult::LargeTickMiss
                        }
                    } else {
                        result
                    };
                    self.objects[id].head = Some((time, result));
                    self.head_actions[id] = action;
                    self.emit(
                        id,
                        None,
                        time,
                        object.source.start_time as f64,
                        result,
                        if classic_slider {
                            HitResult::LargeTickHit
                        } else {
                            HitResult::Great
                        },
                        Some(error),
                    );
                    self.events.last_mut().unwrap().position = Some(frame.cursor);
                    if object.path.is_none() {
                        self.objects[id].end = Some((time, result));
                    } else if result.is_hit() && time >= object.source.start_time as f64 {
                        self.postprocess_head(id, time, actions, frame.cursor);
                    }
                    // 同一点击只命中第一个可接收对象；CL 额外受前序物件 note lock 限制。
                    break;
                }
            }
        }
        // 原帧按键边沿必须先影响跟随状态，不能只在 1ms 网格读取最后一个同刻帧。
        self.update_active(time);
        for index in 0..self.active.len() {
            self.continuous_input(self.active[index], time, actions, frame.cursor);
        }
    }
    pub(super) fn update_active(&mut self, time: f64) {
        while self.next_active < self.prepared.objects.len()
            && self.prepared.objects[self.next_active].source.start_time as f64 <= time
        {
            if self.prepared.objects[self.next_active].source.hit_type & (2 | 8) != 0 {
                self.active.push(self.next_active);
            }
            self.next_active += 1;
        }
        self.active
            .retain(|&id| self.prepared.objects[id].end >= time);
    }
    pub(super) fn timeout(&mut self, id: usize, time: f64) {
        let object = &self.prepared.objects[id];
        if object.source.hit_type & 8 != 0 || self.objects[id].head.is_some() {
            return;
        }
        let classic_slider = self.policy.classic_sliders && object.path.is_some();
        let result = if classic_slider {
            HitResult::LargeTickMiss
        } else {
            HitResult::Miss
        };
        self.objects[id].head = Some((time, result));
        self.emit(
            id,
            None,
            time,
            object.source.start_time as f64,
            result,
            if classic_slider {
                HitResult::LargeTickHit
            } else {
                HitResult::Great
            },
            None,
        );
        if object.path.is_none() {
            self.objects[id].end = Some((time, HitResult::Miss));
        }
    }
    pub(super) fn track(&mut self, id: usize, time: f64, actions: u32, cursor: [f32; 2]) {
        let object = &self.prepared.objects[id];
        if time < object.source.start_time as f64 || time > object.end || object.path.is_none() {
            return;
        }
        let previous = self.objects[id].tracking_at(time);
        // 原帧和积分点可落在同刻；重复查询不能把本时刻释放误当作上一时刻已释放。
        if self.last_action_times[id] != Some(time) {
            self.previous_time_actions[id] = self.last_actions[id];
            self.last_action_times[id] = Some(time);
        }
        if self.head_actions[id] != 0
            && self.any_key_after[id].is_none()
            && self.previous_time_actions[id] & (3 ^ self.head_actions[id]) == 0
        {
            self.any_key_after[id] = Some(time);
        }
        self.last_actions[id] = actions;
        // 官方只在观察到另一键释放的下一时刻允许换键，同刻输入仍须使用头判按键。
        let held = if self.head_actions[id] == 0
            || self.any_key_after[id].is_some_and(|after| time > after)
        {
            actions != 0
        } else {
            actions & self.head_actions[id] != 0
        };
        let tracking = slider::tracking(
            cursor,
            object.position_at(time),
            self.prepared.radius,
            held,
            previous,
        );
        if previous != tracking {
            self.objects[id].tracking.push((time, tracking));
        }
    }
    pub(super) fn continuous(&mut self, id: usize, time: f64) {
        let object = &self.prepared.objects[id];
        if time < object.source.start_time as f64 || self.objects[id].end.is_some() {
            return;
        }
        // 插值只用于策略固定步长；不能把官方可选 FrameAccuratePlayback 当成默认规则。
        let Some(input) = self.input.snapshot_at(time) else {
            return;
        };
        self.continuous_input(
            id,
            time,
            if self.legacy_relax {
                self.relax_actions
            } else {
                standard_actions(input.keys)
            },
            input.cursor.unwrap(),
        );
    }
    fn continuous_input(&mut self, id: usize, time: f64, actions: u32, cursor: [f32; 2]) {
        let object = &self.prepared.objects[id];
        if time < object.source.start_time as f64 || self.objects[id].end.is_some() {
            return;
        }
        if object.path.is_some() {
            self.track(id, time, actions, cursor);
            let tracking = self.objects[id].tracking_at(time);
            let judged_head = self.objects[id].head.is_some();
            for (node_id, node) in object.nodes.iter().enumerate() {
                if self.objects[id].nodes[node_id].is_some() || node.time > time || !judged_head {
                    continue;
                }
                if node.kind == NodeKind::Tail && !tracking && time < object.end {
                    continue;
                }
                let (result, maximum) = self.node_result(node.kind, tracking);
                self.objects[id].nodes[node_id] = Some((time, result));
                self.emit(
                    id,
                    Some(node_id),
                    time,
                    node.sample_time,
                    result,
                    maximum,
                    None,
                );
            }
        } else if object.source.hit_type & 8 != 0 {
            let angle = -(cursor[0] - 256.0).atan2(cursor[1] - 192.0).to_degrees();
            let auto_spin = self.prepared.mods.settings.spun_out;
            let delta = if auto_spin {
                let elapsed = self.rotation_times[id].map_or(0.0, |last| time - last);
                let duration = object.end - object.source.start_time as f64;
                let required = spinner::required(duration, self.prepared.od).0;
                // SO 以真实时间驱动，不读取录制光标和按键；重复同刻调用的增量为零。
                (elapsed / self.prepared.mods.settings.speed_multiplier * 1.01 * required as f64
                    / duration.max(1.0)
                    * 360.0) as f32
            } else {
                self.last_angles[id]
                    .map_or(0.0, |last| (angle - last + 180.0).rem_euclid(360.0) - 180.0)
            };
            if auto_spin || (actions != 0 && time < object.end) {
                let before = (self.rotations[id].total() / 360.0) as u32;
                self.rotations[id].add(delta);
                let after = (self.rotations[id].total() / 360.0) as u32;
                let (required, total) = spinner::required(
                    object.end - object.source.start_time as f64,
                    self.prepared.od,
                );
                if after > before && before < total {
                    let result = if before < required + 2 {
                        HitResult::SmallBonus
                    } else {
                        HitResult::LargeBonus
                    };
                    self.emit(id, Some(before as usize), time, time, result, result, None);
                }
            }
            self.last_angles[id] = Some(angle);
            let target = self.rotations[id].angle;
            let spinning = time < object.end && (target - self.displayed_angles[id]).abs() > 10.0;
            if self.objects[id].spinning_at(time) != spinning {
                self.objects[id].spinning.push((time, spinning));
            }
            // 官方 Interpolation.Damp 使用 0.99^elapsed_ms，按固定规则步长缓动。
            let elapsed = self.rotation_times[id].map_or(0.0, |last| time - last);
            self.displayed_angles[id] +=
                (target - self.displayed_angles[id]) * (1.0 - 0.99_f64.powf(elapsed)) as f32;
            self.rotation_times[id] = Some(time);
            self.objects[id].rotation.push((
                time,
                self.rotations[id].angle,
                self.rotations[id].total(),
            ));
        }
    }
    pub(super) fn finish(&mut self, id: usize, time: f64) {
        if self.objects[id].end.is_some() {
            return;
        }
        let object = &self.prepared.objects[id];
        if object.path.is_some() && self.objects[id].head.is_none() {
            return;
        }
        self.continuous(id, time);
        if object.path.is_some() {
            let hits = usize::from(self.objects[id].head.is_some_and(|h| h.1.is_hit()))
                + self.objects[id]
                    .nodes
                    .iter()
                    .flatten()
                    .filter(|n| n.1.is_hit())
                    .count();
            let result = if self.policy.classic_sliders {
                crate::replay::calculate::standard::classic::slider_result(
                    hits,
                    object.nodes.len() + 1,
                )
            } else if hits > 0 {
                HitResult::IgnoreHit
            } else {
                HitResult::IgnoreMiss
            };
            self.objects[id].end = Some((time, result));
            self.emit(
                id,
                None,
                time,
                time,
                result,
                if self.policy.classic_sliders {
                    HitResult::Great
                } else {
                    HitResult::IgnoreHit
                },
                None,
            );
        } else if object.source.hit_type & 8 != 0 {
            let (required, total) = spinner::required(
                object.end - object.source.start_time as f64,
                self.prepared.od,
            );
            let completed = (self.rotations[id].total() / 360.0) as u32;
            for node in completed..total {
                let maximum = if node < required + 2 {
                    HitResult::SmallBonus
                } else {
                    HitResult::LargeBonus
                };
                self.emit(
                    id,
                    Some(node as usize),
                    time,
                    time,
                    HitResult::IgnoreMiss,
                    maximum,
                    None,
                );
            }
            let result = spinner::result(self.rotations[id].total(), required);
            self.objects[id].end = Some((time, result));
            self.emit(id, None, time, time, result, HitResult::Great, None);
        }
    }

    /// 晚打头部时，官方先核对已经过节点是否都在扩大跟随区内，再恢复这些节点。
    fn postprocess_head(&mut self, id: usize, time: f64, actions: u32, cursor: [f32; 2]) {
        let object = &self.prepared.objects[id];
        if !circle::in_range(cursor, object.position_at(time), self.prepared.radius * 2.4) {
            return;
        }
        let all_in_range = object
            .nodes
            .iter()
            .filter(|n| n.sample_time <= time)
            .all(|n| {
                circle::in_range(
                    cursor,
                    object.position_at(n.sample_time),
                    self.prepared.radius * 2.4,
                )
            });
        for (node_id, node) in object.nodes.iter().enumerate() {
            if node.sample_time > time || self.objects[id].nodes[node_id].is_some() {
                continue;
            }
            let (result, maximum) = self.node_result(node.kind, all_in_range);
            self.objects[id].nodes[node_id] = Some((time, result));
            self.emit(
                id,
                Some(node_id),
                time,
                node.sample_time,
                result,
                maximum,
                None,
            );
        }
        let tracking = actions & self.head_actions[id] != 0
            && (all_in_range
                || circle::in_range(cursor, object.position_at(time), self.prepared.radius));
        if self.objects[id].tracking_at(time) != tracking {
            self.objects[id].tracking.push((time, tracking));
        }
    }
}

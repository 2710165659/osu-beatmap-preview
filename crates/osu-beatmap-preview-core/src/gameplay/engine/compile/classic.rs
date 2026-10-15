//! CL 的节点类型与 note lock；与公共输入编译流程分开，便于核对官方策略。
use super::*;

#[cfg(test)]
mod tests;

impl Compiler<'_> {
    pub(super) fn node_result(&self, kind: NodeKind, hit: bool) -> (HitResult, HitResult) {
        if kind != NodeKind::Tail {
            (
                if hit {
                    HitResult::LargeTickHit
                } else {
                    HitResult::LargeTickMiss
                },
                HitResult::LargeTickHit,
            )
        } else if self.policy.classic_sliders {
            (
                if hit {
                    HitResult::SmallTickHit
                } else {
                    HitResult::SmallTickMiss
                },
                HitResult::SmallTickHit,
            )
        } else {
            (
                if hit {
                    HitResult::SliderTailHit
                } else {
                    HitResult::IgnoreMiss
                },
                HitResult::SliderTailHit,
            )
        }
    }

    pub(super) fn classic_hittable(&self, id: usize, time: f64, cursor: [f32; 2]) -> bool {
        if (time - self.prepared.objects[id].source.start_time as f64).abs() >= 400.0 {
            return false;
        }
        if id > 0
            && self.prepared.objects[id - 1].source.stack_height > 0
            && self.objects[id - 1].end.is_none()
        {
            return false;
        }
        // 对照 LegacyHitPolicy：允许重叠的长物件；前序未结算物件的终点与目标头部
        // 相隔超过 3ms 时阻止点击。只在 CL 路径执行，普通预览不增加开销。
        let first = self.first_unjudged.min(id);
        !self.prepared.objects[first..id]
            .iter()
            .zip(&self.objects[first..id])
            .any(|(object, state)| {
                // 已判定的滑条头在父滑条结束前仍可吞掉其下方点击，包括长于 400ms 的滑条。
                state.end.is_none()
                    && (object.end + 3.0 < self.prepared.objects[id].source.start_time as f64
                        || (object.path.is_some()
                            && state.head.is_some()
                            && circle::in_range(cursor, object.position, self.prepared.radius)))
            })
    }
}

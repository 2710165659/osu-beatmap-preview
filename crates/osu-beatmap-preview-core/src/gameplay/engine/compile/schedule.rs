//! 在分配前限制规则步数；同刻输入按原序号执行，之后处理节点、超时和结束。
use super::*;

const MAX_STEPS: usize = 2_000_000;
pub(super) type Step = (f64, u8, usize);

pub(super) fn build(
    prepared: &PreparedStandard,
    document: &ReplayDocument,
    deadline: f64,
) -> Result<Vec<Step>> {
    let limit = || PreviewError::new("ReplayCompilationLimit: too many rule steps");
    let mut count = document.frames.len();
    for object in &prepared.objects {
        let continuous = object.source.hit_type & (2 | 8) != 0;
        let end = object.end.min(deadline).ceil() as i64;
        let grid = if continuous && end >= object.source.start_time {
            usize::try_from(end - object.source.start_time + 1).map_err(|_| limit())?
        } else {
            0
        };
        count = count
            .checked_add(grid)
            .and_then(|n| n.checked_add(object.nodes.len()))
            .and_then(|n| n.checked_add(3))
            .filter(|n| *n <= MAX_STEPS)
            .ok_or_else(limit)?;
    }
    if count > MAX_STEPS {
        return Err(limit());
    }
    let mut points = Vec::new();
    points.try_reserve_exact(count).map_err(|_| limit())?;
    for (i, frame) in document.frames.iter().enumerate() {
        if frame.time_ms as f64 <= deadline {
            points.push((frame.time_ms as f64, 0, i));
        }
    }
    for (id, object) in prepared.objects.iter().enumerate() {
        points.push((
            object.source.start_time as f64 + circle::windows(prepared.od)[2] + 1.0,
            2,
            id,
        ));
        if object.source.hit_type & (2 | 8) != 0 {
            for time in object.source.start_time..=object.end.min(deadline).ceil() as i64 {
                points.push((time as f64, 1, id));
            }
            points.push((object.end, 3, id));
            points.push((
                object
                    .end
                    .max(object.source.start_time as f64 + circle::windows(prepared.od)[2] + 1.0),
                3,
                id,
            ));
            for node in &object.nodes {
                points.push((node.time, 1, id));
            }
        }
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    Ok(points)
}

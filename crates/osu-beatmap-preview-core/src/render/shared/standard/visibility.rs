//! 按稳定时间顺序查询可见物件；区间树跳过已结束的分支，支持乱序 seek。

pub(crate) struct ObjectVisibility {
    starts: Vec<(i64, usize)>,
    max_ends: Vec<i64>,
    leaves: usize,
}

impl ObjectVisibility {
    pub(crate) fn new(intervals: impl IntoIterator<Item = (i64, i64, usize)>) -> Self {
        let mut intervals: Vec<_> = intervals.into_iter().collect();
        intervals.sort_unstable_by_key(|&(start, _, id)| (start, id));
        let leaves = intervals.len().max(1).next_power_of_two();
        let mut max_ends = vec![i64::MIN; leaves * 2];
        for (index, &(_, end, _)) in intervals.iter().enumerate() {
            max_ends[leaves + index] = end;
        }
        for index in (1..leaves).rev() {
            max_ends[index] = max_ends[index * 2].max(max_ends[index * 2 + 1]);
        }
        let starts = intervals
            .into_iter()
            .map(|(start, _, id)| (start, id))
            .collect();
        Self {
            starts,
            max_ends,
            leaves,
        }
    }

    pub(crate) fn at(&self, time: i64) -> VisibleObjects<'_> {
        let limit = self.starts.partition_point(|&(start, _)| start <= time);
        let mut pending = [(0, 0, 0); usize::BITS as usize];
        pending[0] = (1, 0, self.leaves);
        VisibleObjects {
            index: self,
            time,
            limit,
            pending,
            count: 1,
        }
    }
}

pub(crate) struct VisibleObjects<'a> {
    index: &'a ObjectVisibility,
    time: i64,
    limit: usize,
    // 深度优先查询只暂存每层的一个兄弟节点，上限不超过 usize 的位数。
    pending: [(usize, usize, usize); usize::BITS as usize],
    count: usize,
}

impl Iterator for VisibleObjects<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        while self.count > 0 {
            self.count -= 1;
            let (node, left, right) = self.pending[self.count];
            if left >= self.limit || self.index.max_ends[node] < self.time {
                continue;
            }
            if right - left == 1 {
                return Some(self.index.starts[left].1);
            }
            let middle = left + (right - left) / 2;
            // 右子树先出栈，让更晚的物件先画；同一时刻按原谱面索引稳定倒序。
            self.pending[self.count] = (node * 2, left, middle);
            self.pending[self.count + 1] = (node * 2 + 1, middle, right);
            self.count += 2;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_queries_keep_stable_order_boundaries_and_long_objects_after_seek() {
        let intervals = [(10, 5000, 0), (20, 30, 1), (20, 40, 2), (100, 110, 3)];
        let index = ObjectVisibility::new(intervals);
        for time in [0, 20, 40, 30, 100, 5000, 5001, 20] {
            let mut expected: Vec<_> = intervals
                .iter()
                .copied()
                .filter(|&(start, end, _)| start <= time && end >= time)
                .collect();
            expected.sort_unstable_by_key(|&(start, _, id)| std::cmp::Reverse((start, id)));
            assert_eq!(
                index.at(time).collect::<Vec<_>>(),
                expected
                    .into_iter()
                    .map(|(_, _, id)| id)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(ObjectVisibility::new([]).at(0).next(), None);
    }
}

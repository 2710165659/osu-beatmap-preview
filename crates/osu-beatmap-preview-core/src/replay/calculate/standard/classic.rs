//! 对照官方 DrawableSlider 的 ClassicSliderBehaviour，按嵌套物件命中比例结算。
use crate::gameplay::events::HitResult;

pub(crate) fn slider_result(hits: usize, total: usize) -> HitResult {
    if hits == total {
        HitResult::Great
    } else if hits == 0 {
        HitResult::Miss
    } else if hits as f64 / total as f64 >= 0.5 {
        HitResult::Ok
    } else {
        HitResult::Meh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_slider_grades_all_none_half_and_less_than_half() {
        assert_eq!(slider_result(4, 4), HitResult::Great);
        assert_eq!(slider_result(0, 4), HitResult::Miss);
        assert_eq!(slider_result(2, 4), HitResult::Ok);
        assert_eq!(slider_result(1, 4), HitResult::Meh);
    }
}

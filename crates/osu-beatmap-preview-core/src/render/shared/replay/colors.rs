//! osu! 游戏内判定颜色；前景和时间轴背景统一从这里取色。

use crate::gameplay::events::HitResult;

pub(crate) const BLUE_LIGHT: [u8; 4] = [153, 238, 255, 255];
pub(crate) const BLUE: [u8; 4] = [102, 204, 255, 255];
pub(crate) const GREEN_LIGHT: [u8; 4] = [179, 217, 68, 255];
pub(crate) const GREEN: [u8; 4] = [136, 179, 0, 255];
pub(crate) const YELLOW: [u8; 4] = [255, 204, 34, 255];
pub(crate) const RED: [u8; 4] = [237, 17, 33, 255];
pub(crate) const GRAY: [u8; 4] = [128, 128, 128, 255];

/// 与 osu.Game.Graphics.OsuColour.ForHitResult 一致。
pub(crate) const fn hit_result(result: HitResult) -> [u8; 4] {
    match result {
        HitResult::IgnoreMiss | HitResult::SmallTickMiss => GRAY,
        HitResult::Miss | HitResult::LargeTickMiss => RED,
        HitResult::Meh => YELLOW,
        HitResult::Ok => GREEN,
        HitResult::Good => GREEN_LIGHT,
        HitResult::SmallTickHit
        | HitResult::LargeTickHit
        | HitResult::SliderTailHit
        | HitResult::Great => BLUE,
        _ => BLUE_LIGHT,
    }
}

pub(crate) const fn with_alpha(mut color: [u8; 4], alpha: u8) -> [u8; 4] {
    color[3] = alpha;
    color
}

pub(crate) const fn rgb(color: [u8; 4]) -> [u8; 3] {
    [color[0], color[1], color[2]]
}

// 与 OsuColour.ForRank 一致；SS 和失败 F 的显示名称由 HUD 统一处理。
pub(crate) fn rank(rank: &str) -> [u8; 4] {
    match rank {
        "SS" => [222, 49, 174, 255],
        "S" => [2, 181, 195, 255],
        "A" => [136, 218, 32, 255],
        "B" => [227, 177, 48, 255],
        "C" => [255, 142, 93, 255],
        "D" => [255, 90, 90, 255],
        _ => [63, 63, 63, 255],
    }
}

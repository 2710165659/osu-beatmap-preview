//! Standard 逻辑准备：堆叠、难度、滑条路径与嵌套节点均不依赖渲染。
#[cfg(feature = "replay")]
mod objects;
pub mod stacking;
#[cfg(feature = "replay")]
pub use objects::*;

/// HR 先翻转物件和滑条控制点，再计算堆叠；录制的光标已经处于游玩坐标。
pub fn apply_object_mods(
    mut objects: Vec<crate::model::StandardHitObject>,
    mods: Option<&crate::domain::mods::ModSettings>,
) -> Vec<crate::model::StandardHitObject> {
    if mods.is_some_and(|mods| mods.hard_rock) {
        for object in &mut objects {
            object.y = 384 - object.y;
            for point in &mut object.slider_points {
                point.1 = 384 - point.1;
            }
        }
    }
    objects
}

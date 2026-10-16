//! Standard 逻辑准备：堆叠、难度、滑条路径与嵌套节点均不依赖渲染。
#[cfg(feature = "replay")]
mod objects;
pub mod stacking;
#[cfg(feature = "replay")]
pub use objects::*;

/// HR/Mirror 先翻转物件和滑条控制点，再计算堆叠；录制的光标已经处于游玩坐标。
pub fn apply_object_mods(
    mut objects: Vec<crate::model::StandardHitObject>,
    mods: Option<&crate::domain::mods::ModSettings>,
) -> Vec<crate::model::StandardHitObject> {
    if let Some(mods) = mods {
        let horizontal = mods.mirror && matches!(mods.mirror_reflection, 0 | 2);
        let vertical = mods.hard_rock || (mods.mirror && matches!(mods.mirror_reflection, 1 | 2));
        for object in &mut objects {
            if object.hit_type & 8 != 0 {
                continue;
            }
            if horizontal {
                object.x = 512 - object.x;
            }
            if vertical {
                object.y = 384 - object.y;
            }
            for point in &mut object.slider_points {
                if horizontal {
                    point.0 = 512 - point.0;
                }
                if vertical {
                    point.1 = 384 - point.1;
                }
            }
        }
    }
    objects
}

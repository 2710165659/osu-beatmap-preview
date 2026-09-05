//! 与具体后端无关的逐帧场景描述，为 WGPU 与实时 Surface 预留稳定输入边界。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct SceneSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct FrameScene {
    pub size: SceneSize,
    pub time_ms: i64,
}

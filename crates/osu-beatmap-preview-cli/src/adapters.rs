//! 桌面、Android、iOS 的平台接口骨架；完整 UI 由各应用实现。

pub trait NativeSurface {
    fn raw_handle(&self) -> *mut core::ffi::c_void;
}
pub trait NativeAudio {
    fn play(&mut self);
    fn pause(&mut self);
    fn position_ms(&self) -> i64;
}
pub trait NativeLifecycle {
    fn on_resume(&mut self);
    fn on_pause(&mut self);
}

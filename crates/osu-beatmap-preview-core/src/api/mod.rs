//! 面向宿主程序的核心调用模型。
//!
//! `api` 只描述跨平台调用边界，不暴露解析和渲染实现细节。输入资源、输出信息
//! 与会话分别位于独立子模块，便于不同宿主按需引入。

pub mod input;
pub mod output;
pub mod session;

pub use input::{AudioData, ImageData, RealtimeOptions, RenderConfig, ResourceBundle};
pub use output::{RealtimeMode, TimelineInfo};
pub use session::RealtimeSession;

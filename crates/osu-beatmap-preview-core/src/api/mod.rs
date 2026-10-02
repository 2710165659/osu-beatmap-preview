//! 面向宿主程序的核心调用模型。
//!
//! `api` 只描述跨平台调用边界，不暴露解析和渲染实现细节。输入资源、输出信息
//! 与会话分别位于独立子模块，便于不同宿主按需引入。时钟（`clock`）与混音输出流
//! （`stream`）是实时预览的两条公共机制：前者是画面与声音共用的时间权威，后者决定
//! 「下一段混音从哪里、混多少」并跟踪音频线程的消费进度。

pub mod clock;
pub mod input;
pub mod output;
pub mod session;
pub mod stream;

pub use clock::PreviewClock;
pub use input::{
    AudioConfig, ImageData, RealtimeOptions, RenderConfig, ResourceBundle, StoryboardBundle,
};
pub use output::{RealtimeMode, TimelineInfo};
pub use session::RealtimeSession;
pub use stream::AudioStream;

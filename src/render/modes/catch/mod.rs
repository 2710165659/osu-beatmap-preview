//! osu!catch 渲染器：展开水果、果汁流、香蕉雨、HR 偏移和 hyperdash 等渲染对象，
//! 并输出 PNG 网格和 GIF 预览。RNG 调用顺序严格匹配 Python/stable 实现。

mod animation;
pub(crate) mod constants;
mod drawing;
pub(crate) mod objects;
mod png;
mod route;
mod video;

pub(crate) use animation::render_catch_gif;
pub(crate) use png::render_catch_grid;
pub(crate) use video::render_catch_video;

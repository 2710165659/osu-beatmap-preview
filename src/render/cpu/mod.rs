//! CPU 场景光栅化后端。

pub(crate) mod modes;
// 该后端作为像素回归的 CPU 参考实现保留，不进入现有 CPU CLI 调用链。
#[cfg(test)]
#[allow(dead_code)]
mod rasterizer;

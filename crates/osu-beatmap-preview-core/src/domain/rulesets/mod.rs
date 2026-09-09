//! 模式转换规则。渲染器只消费转换结果，不再拥有谱面转换逻辑。

pub mod catch;
pub mod mania;
pub mod taiko;

pub use catch::catch_convert;
pub use mania::mania_convert;
pub use taiko::taiko_convert;

#[cfg(test)]
mod tests;

//! 模式转换规则。渲染器只消费转换结果，不再拥有谱面转换逻辑。

pub(crate) mod catch;
pub(crate) mod mania;
pub(crate) mod taiko;

#[cfg(test)]
mod tests;

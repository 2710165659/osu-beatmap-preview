//! 加载时一次编译输入和判定；输出线程只读结果，不共享可变引擎。
mod compile;
mod validation;
pub use compile::{compile_catch, compile_mania, compile_standard, compile_taiko};

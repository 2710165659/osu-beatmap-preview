//! 启用 WGPU feature 时使用的命名入口；实际 CLI 实现与 CPU 入口共用。

#[path = "main.rs"]
mod shared_main;

fn main() {
    shared_main::main();
}

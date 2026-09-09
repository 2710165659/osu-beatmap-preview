//! WASM/WebGPU 示例播放器的静态资源宿主。
mod server;
fn main() {
    if let Err(error) = server::run("127.0.0.1:8787") {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

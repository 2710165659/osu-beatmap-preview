//! 追加写入：单次 `write_all` 写一整行，依赖操作系统"追加模式单次写原子"
//! 语义（Windows `FILE_APPEND_DATA` / POSIX `O_APPEND`）保证多进程并发安全。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// 行上限：保持单次写可原子完成的经验安全值。
pub const MAX_LINE_BYTES: usize = 4096;

pub fn append_line(path: &Path, line: &str) {
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    if !buf.ends_with(b"\n") {
        buf.push(b'\n');
    }
    let mut file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("[log] failed to open '{}': {error}", path.display());
            return;
        }
    };
    if let Err(error) = file.write_all(&buf) {
        eprintln!("[log] failed to write '{}': {error}", path.display());
    }
}

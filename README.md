# osu! Beatmap Preview

> 一个快速的 osu! 谱面预览工具，支持四种模式（Standard / Taiko / Catch / Mania）的 GIF 动图、PNG 静态图与 MP4 视频渲染。

## 特性

- **单可执行文件**：皮肤资源在编译期嵌入二进制，无运行时依赖。
- **跨平台**：Windows / Linux / macOS。
- **四模式支持**：`standard`、`taiko`、`catch`、`mania`。
- **三种输出格式**：`gif` 动图、`png` 静态长图、`mp4` 视频。
- **GPU 加速视频编码**：自动检测 NVIDIA NVENC / AMD AMF 硬件编码器，无 GPU 时回退 CPU（openh264），保持单文件无运行时依赖。
- **转谱**：Standard 可转为 Taiko / Catch / Mania 并预览。
- **丰富的 Mod**：`EZ` `HR` `HD` `DA` `DT` `HT` `SW` `CS` `1K`–`10K` `DS` `IN` `HO`。
- **高性能**：渲染速度快、内存占用低、输出文件体积小。详见 [批量渲染报告](docs/report.txt)。

> 如果这个项目对你有帮助，欢迎点个 ⭐ Star 支持一下～

## 使用

<img src="./docs/usage.png" width="100%">

## 输出

程序向 stdout 输出 JSON，schema 如下：

```json
{
  "status": "success",
  "msg": "preview generated successfully for bid 738063",
  "preview-img": "/path/to/output.png",
  "beatmap-info": {
    "meta-data": { "title": "...", "artist": "...", ... },
    "difficulty": { ... }
  },
  "build-info": {
    "version": "1.0.3",
    "build_time": "2026-06-22T16:01:06.623636800Z"
  }
}
```

> `preview-img` 字段为输出文件的绝对路径，格式由 `--fmt` 决定（`.gif` / `.png` / `.mp4`）。

| 路径 | 说明 |
| --- | --- |
| 谱面缓存 | `<临时目录>/osu-beatmap-preview/osu-download-cache/<bid>.osu` |
| 输出文件 | `<临时目录>/osu-beatmap-preview/outputs/<mode>_<bid>[_convert][_mods][_t<时间点>][_bpm<BPM值>].<fmt>` |
| 批量脚本 | `batch_render.ps1` — 可批量渲染多个 bid 并生成对比 HTML |

> 缓存文件不会自动删除，占用过大时可手动清理临时目录。

## 效果预览

![总览](docs/total.png)

## 构建

```bash
cargo build --release
# 产物: target/release/osu-beatmap-preview(.exe)
```

> 需要 Rust 1.70+。安装方式：<https://rustup.rs>

## License

[MIT](LICENSE)
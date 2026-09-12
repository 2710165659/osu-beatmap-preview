# osu! Beatmap Preview

[中文](README.md) | [English](docs/README.en.md)

独立的 osu! 谱面预览工具，支持 osu!standard、osu!taiko、osu!catch、osu!mania 四种模式：既可以把谱面导出成 PNG、GIF 和带原曲音频的 MP4，也可以在浏览器里用 WebGPU 实时播放谱面。

![四种模式的渲染效果](docs/total.png)

## 功能亮点

- **Mod 支持**：`EZ` `HR` `HD` `DA` `TC` `SW` `CS` `DS` `IN` `HO` 以及 `1K`–`10K` 键数可自由组合；`DT`、`HT` 支持自定义倍速（`1.01`–`2.00x`、`0.50`–`0.99x`），`DA` 还能按 `cs`/`ar`/`od`/`hp` 逐项改难度。重复的、互相冲突的（如 `EZ` 配 `HR`）或当前模式不支持的 Mod 会直接报错，不会静默忽略。
- **转谱**：Standard 谱面可转到 Taiko、Catch 或 Mania；转 Mania 时 `1K`–`10K`、`DS`、`IN`、`HO` 会参与转谱结果，目标模式与源模式相同时按不转谱处理。
- **四种模式**：osu!standard、osu!taiko、osu!catch、osu!mania 各有独立的布局、皮肤和配色，都能输出 PNG 概览、GIF 分段预览和带原曲音频的 H.264 MP4。
- **浏览器实时预览**：Web 包解压后 `node backend/server.js`（或 `npm start`）就能在浏览器里播放谱面，支持 Mod 热切换、转谱、seek、倍速、分辨率（480P/720P/1080P）与 30/60/120 FPS 切换；也能用 `/?bid=<BID>` 直接进入预览。后端只做跨域下载与缓存并向页面报告下载进度，每一帧都在本地 WebGPU 中完成。
- **高性能、低占用**：帧渲染按配置分块并行，并按单帧字节数动态限制批次，降低大画布的临时内存峰值；GIF 复用帧编码缓冲区，Standard MP4 预计算每帧可见物件索引，滑条 tick、鼓滚 tick 等使用程序化精灵缓存。实测数据见[批量渲染报告](docs/report.md)。
- **独立、无外部依赖的 CLI**：单个可执行文件内嵌皮肤、字体、H.264 和 AAC 编码器，运行时不需要 FFmpeg、额外动态库或资源文件；Windows 会自动尝试 NVENC / AMF，缺失时回退内置 CPU 编码器（`OSU_PREVIEW_NO_GPU=1` 可强制 CPU）。下载缓存、输出缓存和日志都在本机复用，不同配置使用独立输出目录。

各模式的具体 Mod 语法、参数和配置项见 [CLI 使用说明](crates/osu-beatmap-preview-cli/README.md)。

## 文档入口

CLI 是主要用法，完整说明在 CLI 文档里；本文件只做项目介绍和架构概览。

| 文档 | 内容 |
| --- | --- |
| [**CLI 使用说明**](crates/osu-beatmap-preview-cli/README.md) | 下载、快速开始、全部命令行参数、输出格式、Mod、配置项、缓存与日志 |
| [Web 站点说明](crates/osu-beatmap-preview-web/README.md) | Web 包的启动方式、后端接口、下载与缓存策略 |
| [WASM 使用说明](crates/osu-beatmap-preview-wasm/README.md) | 构建 WASM、JavaScript API、宿主职责与使用限制 |
| [架构说明](docs/architecture.md) | crate 划分、API 边界、场景与 WGPU 后端、非目标 |
| [批量渲染报告](docs/report.md) | 性能与资源占用数据 |
| [第三方声明](docs/THIRD_PARTY_NOTICES.md) | 内嵌编解码组件的许可证与源码获取方式 |

## 发布产物

两部分开下载，互不依赖：

| 产物 | 形式 | 说明 |
| --- | --- | --- |
| CLI | 4 个平台的单文件可执行程序 | `osu-beatmap-preview-<平台>-<架构>-cli`，导出 PNG/GIF/MP4 |
| Web 包 | `osu-beatmap-preview-web.zip` | 静态站点（浏览器 WebGPU 实时渲染）+ Node.js 下载后端 |

获取方式见 [Releases](https://github.com/2710165659/osu-beatmap-preview/releases)。CLI 的用法见 [CLI 使用说明](crates/osu-beatmap-preview-cli/README.md)，Web 包解压后 `node backend/server.js` 即可，见 [Web 站点说明](crates/osu-beatmap-preview-web/README.md)。

## 架构概览

代码按“跨平台核心 / GPU 绘制 / 平台适配”三层拆分，CPU 导出与 GPU 实时绘制是两条独立路径：

```text
CLI  -> cli 应用层 -> 参数、配置、下载、缓存、日志、媒体编码、文件输出
                   `-> core：解析、Mod、转谱、时间轴、四模式 CPU 单帧与静态场景 -> PNG/GIF/MP4

Web  -> Node 后端下载并缓存 .osu/.osz -> 浏览器 -> wasm -> core RealtimeSession -> renderer WebGPU Canvas
```

| crate | 职责 |
| --- | --- |
| `osu-beatmap-preview-core` | 谱面模型、解析、Mod、转谱、时间轴，以及四模式的 CPU 单帧与静态场景绘制，产出不依赖窗口、网络和音频设备的 `FrameScene` |
| `osu-beatmap-preview-renderer` | 平台无关的 WGPU 绘制；device、queue、surface 和目标 view 由宿主提供 |
| `osu-beatmap-preview-cli` | 命令行参数、配置、下载、缓存、日志、时间序列与布局组装、媒体编码和文件输出（默认构建目标，Release 产物） |
| `osu-beatmap-preview-wasm` | 把 core 会话和 renderer 接到浏览器 WebGPU Canvas，并导出按 `.osu` 字节汇总谱面信息的 `beatmapInfo`（Release 产物的一部分） |
| `osu-beatmap-preview-web` | Vue 静态站点与 Node.js 下载后端：下载并缓存 `.osu`/`.osz`，把音频和背景交给浏览器（Release 产物，不在 Cargo workspace 内） |
| `osu-beatmap-preview-gui`、`osu-beatmap-preview-mobile` | 桌面与移动端的 surface、输入、播放生命周期和音频时钟适配骨架 |

CLI 始终走 CPU 导出，不依赖 renderer，也不提供 WGPU 参数；实时 API 在 GPU 不可用时明确报错，不会静默回退到 CPU。Web 后端只负责跨域下载与缓存，不含任何绘制逻辑，所有帧都由浏览器内的 wasm 完成。crate 之间的边界、配置来源和非目标见[架构说明](docs/architecture.md)。

## 从源码构建

需要稳定版 Rust 工具链和可用的 C/C++ 编译环境，安装方式见 <https://rustup.rs>。

```bash
git clone https://github.com/2710165659/osu-beatmap-preview.git
cd osu-beatmap-preview
cargo build --release
```

workspace 默认成员只有 CLI，产物为 `target/release/osu-beatmap-preview-cli`（Windows 为 `osu-beatmap-preview-cli.exe`）。运行测试：

```bash
cargo test --workspace --all-features --all-targets
```

Web 站点需要 Node.js 18+ 运行，构建前端需要 Node.js 20.19+，构建 wasm 产物需要 Rust 工具链和与 `Cargo.lock` 一致的 `wasm-bindgen-cli`：

```bash
cd crates/osu-beatmap-preview-web
npm install          # Vue / Vite / Tailwind，仅构建需要
npm run build        # 把 src/ 编译到 dist/
npm run build:wasm   # 生成 public/pkg
npm test             # 后端回归测试
npm start            # 访问 http://127.0.0.1:8787
```

细节见 [Web 站点说明](crates/osu-beatmap-preview-web/README.md) 与 [WASM 使用说明](crates/osu-beatmap-preview-wasm/README.md)。

## 许可证

本项目使用 [MIT License](LICENSE)。内嵌 AAC 编码器使用 Fraunhofer FDK-AAC，其许可证不授予专利权，详见[第三方声明](docs/THIRD_PARTY_NOTICES.md)。

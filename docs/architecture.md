# 架构说明

项目将文件导出与实时绘制分开。核心数据和场景描述不依赖窗口、网络或音频设备；GPU 代码由 renderer crate 提供，平台适配层负责创建 surface。

```text
CLI -> cli 应用层 -> 文件/下载/缓存/配置/媒体适配
                    `-> core 单帧/静态场景计算与绘制 -> export 时间序列、布局组装 -> PNG/GIF/MP4 编码

Web -> Node 后端下载并缓存 .osu/.osz -> 浏览器 -> wasm -> core RealtimeSession -> renderer WebGPU Canvas
```

## 目录职责

- `crates/osu-beatmap-preview-core`：谱面模型、`.osu` 解析、Mod、转谱、时间轴、四模式 CPU 单帧/静态场景绘制、`FrameScene`。
- `crates/osu-beatmap-preview-renderer`：平台无关的 WGPU 场景绘制、surface 和离屏后端。
- `crates/osu-beatmap-preview-cli`：CLI/native 适配、文件/下载/缓存/配置/日志、时间序列与布局组装、媒体编码和 I/O；不再包含模式绘制逻辑。
- `crates/osu-beatmap-preview-wasm`：将 core 会话和 renderer 接到宿主 WebGPU Canvas 的 WASM API，并导出 `beatmapInfo`（按 `.osu` 字节汇总谱面内部信息），没有 Rust 调用方。
- `crates/osu-beatmap-preview-web`：Vue 静态站点与 Node.js 下载后端，负责跨域下载 `.osu`/`.osz`、解出音频与背景并缓存、向页面报告加载进度；不属于 Cargo workspace，也不参与任何绘制。
- `crates/osu-beatmap-preview-gui`：桌面 surface、输入和播放生命周期接口骨架。
- `crates/osu-beatmap-preview-mobile`：Android/iOS surface、输入和音频时钟接口骨架。

workspace 的默认成员是 `osu-beatmap-preview-cli`。普通 `cargo build --release` 构建 CLI。CLI 不依赖 renderer crate，也不提供实时渲染入口；Web 包与 GUI/mobile 骨架都不进入 CLI Release。

## Web 后端

浏览器无法跨域直接下载 osu! 的资源，所以 Release 里的 Web 包由一个 Node.js 进程提供静态站点（`dist/`，由 Vite 从 `src/` 构建）和三类资源接口：`/resource/beatmap`、`/resource/audio`、`/resource/background`，外加只读的加载进度快照 `/resource/progress`。下载策略与 CLI 一致（多镜像竞速、Range 分块、osu.direct 优选 IP、缓存优先），并复用同一份缓存目录布局；解包只用 Node 自带的 zlib，因此后端没有任何第三方依赖（Vue/Vite/Tailwind 都只是构建期依赖）。绘制全部发生在浏览器内的 wasm 中，后端不返回像素数据。

谱面的元信息同样由 wasm 解析：浏览器把 `/resource/beatmap` 拿到的 `.osu` 字节交给 `beatmapInfo`，返回 `BeatmapInfo` 的全量字段（概览、统计、难度与 `[General]`/`[Metadata]`/`[Difficulty]` 区段），页面只展示其中一部分。

## 请求与配置

`RenderRequest::validate` 处理不依赖谱面的语法和范围，`RenderPlan::build` 在目标模式确定后处理格式、Mod、时间点和默认值。CLI 的 `export` 模块只负责组织 PNG、GIF 和 MP4 导出，不作为跨平台实时渲染 API。

CPU CLI 使用进程级只读配置。实时会话通过 `RealtimeOptions` 接收类型化配置，宿主可以为每个会话独立构造配置，不访问 CLI 的配置文件。

模式几何、`SCALE`、背景和 HUD 读取对应的 `render.<mode>.<format>` 配置。配置文件变更参与稳定配置 hash；CLI 的 `--scale`、`--fps`、Mod 和时间选段不参与目录 hash，由产物文件名表达。renderer 的画布尺寸、MSAA 和 readback 并发数由 Web、GUI 或移动端宿主通过类型化 API 提供，不读取 CLI 配置。

## 配置资源

- `assets/shared_config.yml`：core 与 CLI 共享的配置，包含 `render`、`skin`。core 构建时提取并生成 `CoreConfig` 的默认值；CLI 启动时将其与 CLI 专用配置合并为 `RuntimeConfig`。
- `crates/osu-beatmap-preview-cli/assets/cli_config.yml`：CLI 专用配置，包含 `paths`、`download`、`timeout`、`advance`，只由 `osu-beatmap-preview-cli` 使用。
- `crates/osu-beatmap-preview-core/assets/testdata_conversion/`：core 转谱回归测试使用的 fixture，只由 `osu-beatmap-preview-core` 使用。

## 场景与后端

`FrameScene` 对外字段私有，只公开尺寸与绝对时间。内部场景由有序命令和会话资源组成，支持裁剪、精灵、矩形、圆/环、线段、字形和 Standard 滑条厚线网格。`FrameSceneBuilder` 负责合并场景时平移命令并重新编号资源。

WGPU 后端在场景合成阶段按模式把场景坐标映射到固定 RGBA8 画布并居中缩放，不在最终 RGBA 上做整体缩放。Standard/Catch 的背景图使用等比铺满并居中裁剪，避免固定画布出现黑色留边；Taiko/Mania 的补边保留对应模式背景色。场景仍会在无法满足模式布局时返回所需尺寸。渲染使用 premultiplied 中间目标完成 straight-alpha src-over 混合，最终 pass 恢复 straight RGBA；MSAA 不可用时只向下选择。纹理按稳定资源编号和资源实例缓存，相邻同类命令合并批次，裁剪映射为 scissor。

readback buffer 按 `MAX_IN_FLIGHT` 预分配。`render_stream` 在提交任何 GPU 工作前拒绝重复 frame index，按 index 排序，最多保留配置数量的在途映射，并严格按顺序回调。取消、场景错误、设备错误或回调错误都会停止新提交并取消其余映射。

## API 边界

core 提供 `RealtimeSession`、`FrameScene`、资源包、时间线和分类错误；renderer 提供 `SurfaceRenderer`、GPU `OffscreenRenderer` 和 `RgbaFrame`。会话通过资源字节和类型化选项构造，不自行下载或访问文件。异步 WGPU 方法不绑定 Tokio，宿主可以自行选择执行器。

GPU 不可用或设备失败时 WGPU API 明确返回错误，不切换到 CPU 绘制。Windows 的 NVENC/AMF 仅属于 CLI 的 H.264 编码选择，仍可回退 OpenH264，与 renderer 的 WGPU 绘制无关。

## 非目标

当前版本不包含正式 GUI/移动端产品 UI、WGPU PNG/GIF、回放解析、外部 texture 编码 API 或 NVENC/AMF 零拷贝；GUI/mobile crate 仅提供适配接口骨架。Web 站点的控制面包含播放/暂停、seek、方向键跳转、音频、`0.5x..=2.0x` 倍速、30/60 FPS 和 1080P/720P/480P 分辨率切换。

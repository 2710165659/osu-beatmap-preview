# 架构说明

项目同时保留稳定的 CPU 导出链和可选的 WGPU 实时/离屏链。领域模型、转谱、Mod、时间轴、配置和媒体封装不依赖 WGPU；GPU 代码只在 `wgpu-renderer` feature 下编译。

```text
共享 CLI -> RenderRequest -> 加载 / 校验 / 转谱 / RenderPlan
                             |-> PNG/GIF -> render/cpu -> 现有编码器
                             |-> CPU MP4 -> render/cpu -> RGBA -> H.264/AAC/MP4
                             |-> WGPU MP4 -> FrameScene -> render/wgpu -> RGBA -> H.264/AAC/MP4
                             `-> 调试播放器 -> RealtimeSession -> FrameScene -> WGPU 窗口
```

## 目录职责

- `src/application`：请求校验、渲染计划、产物命名和 CLI 用例编排。
- `src/domain`：谱面模型、`.osu` 解析、Mod、转谱和时间选择规则。
- `src/infrastructure`：配置快照、缓存、下载、日志、音频和 H.264/AAC/MP4 封装。
- `src/render`：CPU/WGPU 共用的画布类型、几何、文字、时序和 `FrameScene` 契约。
- `src/render/cpu`：原 PNG/GIF/MP4 软件光栅化代码及 CPU 场景参考后端。
- `src/render/wgpu`：WGPU 模式帧源、场景合成、pipeline、纹理缓存、MSAA 和离屏 readback。
- `src/realtime`：公开 realtime API 的会话和数据类型，只在 `wgpu-renderer` 下存在。
- `crates/osu-beatmap-preview-wgpu`：正式 WGPU exporter；PNG/GIF 调用 CPU，MP4 调用 WGPU。
- `crates/osu-beatmap-preview-player`：不发布的 egui/rodio 调试播放器。

根 workspace 使用 `default-members = ["."]`。因此普通 `cargo build --release` 只产生原 CPU 二进制，不编译或链接 WGPU、winit、egui 或 rodio。正式发布矩阵分别构建四个 CPU 产物和四个 WGPU exporter 产物；播放器不进入 Release。

## 请求与配置

两个 CLI 复用根库中 `#[doc(hidden)]` 的词法适配器，并生成相同的 `RenderRequest`。`RenderRequest::validate` 处理不依赖谱面的语法和范围，`RenderPlan::build` 在目标模式确定后处理格式、Mod、时间点和默认值。

CPU CLI 使用进程级只读配置。`RealtimeSession` 使用私有 `ConfigSnapshot`，通过线程局部动态作用域让既有模式计算读取本会话配置，因此不同会话不会改写进程全局状态。

四种模式共享 `advance.wgpu`，只包含固定画布宽高、MSAA、目标 FPS 和最大在途 readback。模式几何、`SCALE`、背景和 HUD 继续读取对应的 `render.<mode>.mp4`。Standard/Catch 使用中心锚定的等比 contain，空出的边缘使用谱面背景；Taiko 按宽度铺满并上下补边，Mania 按高度铺满并左右补边，另一方向超出画布时返回明确错误。配置文件变更参与稳定配置 hash；CLI 的 `--scale`、`--fps`、Mod 和时间选段不参与目录 hash，由产物文件名表达。

## 场景与后端

`FrameScene` 对外字段私有，只公开尺寸与绝对时间。内部场景由有序命令和会话资源组成，支持裁剪、精灵、矩形、圆/环、线段、字形和 Standard 滑条厚线网格。`FrameSceneBuilder` 负责合并场景时平移命令并重新编号资源。

WGPU 后端在场景合成阶段按模式把场景坐标映射到固定 RGBA8 画布并居中缩放，不在最终 RGBA 上做整体缩放。Standard/Catch 的背景图使用等比铺满并居中裁剪，避免固定画布出现黑色留边；Taiko/Mania 的补边保留对应模式背景色。场景仍会在无法满足模式布局时返回所需尺寸。渲染使用 premultiplied 中间目标完成 straight-alpha src-over 混合，最终 pass 恢复 straight RGBA；MSAA 不可用时只向下选择。纹理按稳定资源编号和资源实例缓存，相邻同类命令合并批次，裁剪映射为 scissor。

readback buffer 按 `MAX_IN_FLIGHT` 预分配。`render_stream` 在提交任何 GPU 工作前拒绝重复 frame index，按 index 排序，最多保留配置数量的在途映射，并严格按顺序回调。取消、场景错误、设备错误或回调错误都会停止新提交并取消其余映射。

## API 边界

公开 `realtime` API 提供 `RealtimeSession`、`FrameScene`、`OffscreenRenderer`、`RgbaFrame`、时间线、媒体策略、帧请求、取消令牌和分类错误。会话通过 `Arc` 安全共享；单个 `OffscreenRenderer` 只允许顺序的 `&mut self` 调用。异步方法不绑定 Tokio，CLI 和播放器使用 `pollster` 驱动。

GPU 选择默认使用 HighPerformance，支持 `WGPU_BACKEND` 与 `WGPU_ADAPTER_NAME`。GPU 不可用或设备失败时 WGPU API 明确返回错误，不切换到 CPU 绘制。Windows 的 NVENC/AMF 是 WGPU readback 之后独立的 H.264 编码选择，仍可回退 OpenH264。

## 非目标

当前版本不包含正式播放器产品 UI、WGPU PNG/GIF、移动端、回放解析、外部 WGPU texture 编码 API 或 NVENC/AMF 零拷贝。播放器只用于本机调试，控制面仅包含播放/暂停、seek 和 `0.5x..=2.0x` 倍速。

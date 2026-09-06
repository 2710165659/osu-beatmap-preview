# WGPU 实时与离屏渲染实施记录

本文记录 `v1.2.0` WGPU 实时与离屏渲染计划的实现边界和验收状态。原始需求文档保持不变；本文件只记录已经落地的工程约束和本轮布局修正。

## 代码边界

- `src/render/cpu` 保留原有 PNG、GIF、CPU MP4 行为，不依赖 WGPU。
- `src/render/wgpu` 提供 WGPU 场景、离屏渲染和 MP4 帧源。
- `src/render` 下的场景、画布、几何和文字类型是两个后端的共用层。
- `src/realtime` 只在 `wgpu-renderer` feature 下公开实时会话和离屏 API。
- `crates/osu-beatmap-preview-wgpu` 是独立 exporter；调试播放器不进入 Release。

## 配置边界

四种模式共享 `advance.wgpu`：

```yaml
advance:
  wgpu:
    WIDTH: 1280
    HEIGHT: 720
    MSAA_SAMPLES: 4
    TARGET_FPS: 30
    MAX_IN_FLIGHT: 3
```

模式几何、MP4 `SCALE`、背景和 HUD 样式仍从对应的 `render.<mode>.mp4` 读取。配置文件变更参与配置 hash；CLI 临时覆盖不参与 hash。

## 画布布局

WGPU 在 `FrameScene` 合成阶段完成缩放、平移和裁剪，最终输出始终是紧凑行优先 RGBA8 的 `WIDTH x HEIGHT` 画布：

- Standard/Catch 使用中心锚定的等比 contain，较短方向在两侧或上下补齐；WGPU 场景不再绘制不透明黑色底层，固定画布中的谱面背景使用等比 cover 居中裁剪并直接显示在谱面区域。
- Taiko 使用宽度比例铺满画布，上下补边；缩放后高度超过画布时返回 `CanvasTooSmall`。
- Mania 使用高度比例铺满画布，左右补边；缩放后宽度超过画布时返回 `CanvasTooSmall`。

缩放会作用于场景中的图层、裁剪区、精灵、几何图元、字形和 Standard 滑条网格，不在最终 RGBA 帧上整体缩放。CPU 的背景适配函数保持原样，WGPU 使用独立的背景准备函数。

## 放行记录

- 默认 CPU 测试、默认 Release 构建和全 feature 测试必须通过。
- WGPU exporter Release 构建必须通过。
- WGPU 适配器不可用、设备错误和 Taiko/Mania 画布不足必须显式失败，不回退 CPU 绘制。
- 网络谱面、完整 MP4、图像相似度和长流性能测试属于显式验收，不作为默认单元测试的网络依赖。

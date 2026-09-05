# 架构说明

本项目正在从按输出格式组织的脚本式结构迁移为分层架构。第一阶段已经建立稳定的请求、领域、基础设施和渲染目录，并保留一个显式的 CPU 渲染迁移边界。

```text
CLI / Library API
       |
       v
application: Request -> ValidatedRequest -> RenderPlan -> ArtifactName
       |                         |
       v                         v
domain: parser / rulesets    render: scene / backend / modes
       ^                         ^              |
       |                         |              v
       +---- application ---- infrastructure: config / cache / download / logging / media
```

## 目录职责

- `src/application`：请求格式校验、业务计划、产物命名和用例编排。
- `src/domain`：谱面模型、`.osu` 解析、Mod 规则、转谱规则和共享时间算法。
- `src/infrastructure`：配置合并、缓存、下载、日志、图片与视频编码。
- `src/render`：后端无关的场景边界、后端接口和四模式 CPU 渲染器。
- `src/main.rs`：CLI 词法适配器，不声明或复制业务模块。
- `src/lib.rs`：公共 Rust API，并保留 `PreviewOptions` 兼容入口。

`application/engine/legacy.rs` 是迁移边界：它将不可变 `RenderPlan` 适配到现有 CPU 渲染函数。新增功能不得继续扩大它的方法签名；应先扩展计划或端口，再在适配器中消费。当前 `render::modes` 仍直接读取进程配置并调用媒体编码器，这是保留现有渲染行为的过渡依赖，不是最终边界。

## 请求与校验

第一阶段由 `RenderRequest::validate` 完成，只处理不依赖谱面的内容：ID、枚举、数字范围、FPS 和 Mod 语法。第二阶段在谱面解析和目标模式确定后由 `RenderPlan::build` 完成，处理输出能力、时间点数量、Mod 冲突和模式限制。

CLI 与库 API 使用同一个 `RenderRequest`。`PreviewOptions` 仅负责旧 API 到新请求模型的转换，不再维护另一套校验逻辑。

## 配置、缩放与缓存

配置的渲染字段按语义分组：

- `structure`：行列数量等拓扑，不缩放。
- `sizing`：逻辑像素和字号，在底层绘制前应用对应模式与格式的 `SCALE`。
- `style`：颜色、开关、时长、FPS、比例和限制，不缩放。

代码常量采用相同思路，模式常量分为 `cache/rules/sizing/style`。`sizing` 常量必须在使用处通过几何缩放函数换算，不能对最终图整体缩放。

配置差异的稳定哈希只决定 `OUTPUT_DIR/<config-hash>`。命令行 `fps/scale/mod/time/convert` 不进入配置哈希，而由 `ArtifactName` 写入输出文件名；`no-cache/no-log` 等执行选项既不进入哈希，也不改变文件名。

## 扩展边界

- WGPU：实现 `render::backend::FrameBackend`，消费 `render::scene::FrameScene`，不要依赖 GIF 或 MP4 编码器。
- 移动端实时渲染：由宿主提供 Surface 后端，持续提交 `FrameScene`；文件下载和日志通过应用端口替换。
- 回放：实现 `application::ports::GameplayTimeline`，把光标和按键状态作为时间线输入；`.osr` 解析和计分属于 `domain`，不进入渲染模式模块。
- 新输出格式：复用场景后端，在 `infrastructure::media` 增加编码器，并扩展应用层输出能力矩阵。

GIF 与 MP4 现在都依赖各模式的中立 `animation` 模块。修改 GIF 封装或调色板不会再改变 MP4；修改共享场景绘制时，两种动画输出才会同时变化。

## 后续迁移路线

1. 配置注入：把 `RuntimeConfig` 从进程级全局读取改为显式 `RenderContext`，模式渲染器只接收本次请求对应的只读配置视图。完成标准是同一进程可并发执行不同 CLI scale 的请求。
2. 场景提取：逐模式把时间采样与绘制描述写入 `FrameScene`，CPU 后端先实现完整契约并保持像素回归测试不变。完成标准是 GIF、MP4 和实时 Surface 消费同一帧场景。
3. 媒体反转：渲染层只产生帧，`infrastructure::media` 负责 GIF/MP4 编码和落盘，消除 `render -> infrastructure` 依赖。完成标准是替换编码器不需要修改模式渲染代码。
4. 宿主适配：为 CLI、桌面实时窗口和移动 Surface 分别实现 source、sink、logging 端口。WGPU 只作为新的 `FrameBackend`，不进入领域和应用请求模型。
5. 回放扩展：在 `domain` 增加 `.osr`、判定与计分，在应用层把 `GameplayTimeline` 组合进场景生成；回放解析不得依赖渲染后端。

每个阶段都应保持 `cargo test`、release 编译和已有产物像素/命名回归测试通过。阶段 1～3 完成前，`legacy.rs` 只允许缩小，不允许承载新功能分支。

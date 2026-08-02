# Changelog

All notable changes to this project will be documented in this file.

---
## [Unreleased]

### Fixed

- OpenH264 CPU 编码器现在定期生成真实 IDR 帧，并按实际 H.264 NAL 类型写入 MP4 同步样本索引，修复 Linux 视频无法跳转以及 QQ 只能显示首帧的问题。
- MP4 封装完成后会内置执行 faststart 重排，将 `moov` 索引移动到文件前部并修正 chunk offset，修复 QQ 等聊天预览器只能播放前几秒或约 10 秒后停止的问题。
- OpenH264 CPU 编码器使用约 500 kbps 的独立目标码率，在画质、文件体积和编码速度之间取得平衡。
- 输出文件（PNG / GIF / MP4）改为原子写入：先写同目录临时文件，全部完成后才替换最终文件，渲染中断（如进程被强制关闭）不再在缓存路径留下损坏的半成品。缓存命中新增格式完整性校验，已损坏的旧缓存会被识别并自动重新渲染。

### Added

- 新增多进程安全日志系统：`render.log`（NDJSON 汇总，每谱面一行，含时间、bid、渲染时长、谱面信息与各阶段耗时）与 `progress.log`（可 `tail -f` 实时查看的阶段事件流），默认写入 `<临时目录>/osu-beatmap-preview/logs`。
- 新增 `--log-dir=<DIR>`（覆盖日志目录）与 `--no-log`（关闭日志）参数，支持 `OSU_PREVIEW_LOG_DIR` 环境变量；stdout JSON 增加可选 `log` 字段，不影响现有解析。
- 视频渲染支持从preview time开始固定30s画面输出。
- gif渲染支持单游戏界面clip，默认10s。

### Changed

- `--time` / `--times` 改为使用 osu! 游戏皮肤时间轴：转谱后目标模式首物件为 `0:00`，支持负时间；所有可见时间标签同步采用该时间轴，MP4 右上角显示“当前皮肤时间 / 全谱可玩总时长”，内部谱面与音频计算仍使用绝对时间。
- 优化项目结构。
- OSZ 下载改为智能镜像竞速，支持低速自动回退、最多 3 个来源并行，并为 osu.direct 自动选择和缓存 Cloudflare 优选 IP。

## [1.0.6] - 2026.07.30

### Added

- MP4 输出加入从 OSZ 提取的谱面原始音频，支持 MP3、OGG 和 WAV。
- 音轨遵循 `.osu` 的 `AudioFilename`、`AudioLeadIn`、`--time` 范围以及 DT/HT 倍速。
- OSZ 下载支持 Nekoha、Sayobot、osu.direct、Catboy 镜像顺序回退。
- OSZ 下载、音频解码与 AAC 编码会和画面渲染并行执行，完成后统一封装 MP4。
- 补充 Symphonia、fdk-aac 与 Fraunhofer FDK AAC 的完整许可证及源码获取信息。

### Performance

- OSZ 下载优先使用 4 路 HTTP Range 分段并行下载，不支持 Range 时自动回退单连接下载。
- MP4 编码调整为速度和体积优先：GPU H.264 目标码率降至 900 kbps，AAC 降至 96 kbps，并启用更快的 GPU/CPU 编码预设。

## [1.0.5] - 2026.07.25

- 多项渲染与运行时性能优化，渲染速度提升一倍以上。

## [1.0.4] - 2026.07.05

### Added

- 增加视频渲染支持（`--fmt=mp4`），四种模式均可输出 H.264 MP4 视频。
- 视频 GPU 硬件加速编码：自动检测 NVIDIA NVENC / AMD AMF，无 GPU 时回退 CPU（openh264），保持单文件无运行时依赖。
- `--time=t1+t2` 支持指定 MP4 视频片段范围。

### Changed

- `--bpm` 参数重命名为 `--gap`。
- 更新 README 与使用说明文档。

## [1.0.3] - 2026.06.23

### Added

- 加入图片缓存功能。
- std模式支持TC mod。

### Changed

- 优化了项目结构

## [1.0.2] - 2026.06.21

### Added
- Mania PNG 渲染支持绘制 BPM 标签。
- Taiko PNG 渲染支持按 BPM 指定间隔绘制节拍线。
- Standard PNG 支持通过 `--time` 指定时间点。
- 转谱模式与目标模式一致时不再报错，视为无操作。
- 增加构建时间，为后续缓存做准备。

### Changed
- 更新输出文件命名方式，路径中包含模式与 mod 信息。

### Fixed
- 修复跳过空白区域时小节线偏移的问题。
- 修复 Mania 小节线节拍计算不准确的问题。
- 修复 Taiko 高 BPM 标签绘制错位的问题。

### Performance
- 多项渲染与运行时性能优化。

---

## [1.0.1] - 2026.06.14

### Changed
- 调整 Taiko / Mania / Catch 静态图渲染样式。
- 优化 Catch 渲染文件体积。
- PNG 太鼓移除鼓面图形，增大顶部留白空间。
- 优化 Standard 和 Catch 的视觉效果。

### Fixed
- 修复 Catch 香蕉位置不一致的问题。
- 修复 Catch 水果串间水滴数量错误的问题。

### Performance
- 优化 Standard、Taiko、Catch 渲染性能。

---

## [1.0.0] - 2026.06.14

### Added
- Rust 重构：从 Python 迁移到纯 Rust，单可执行文件，皮肤资源编译期嵌入。
- 四个模式 (Standard / Taiko / Catch / Mania) 的 GIF 与 PNG 预览。
- Mod 支持：`EZ` `HR` `HD` `DA` `DT` `HT` `SW` `CS` `1K`–`10K` `DS` `IN` `HO`。
- 转谱 (--convert) 支持：Standard → Taiko / Catch / Mania。
- `--time` 自定义 GIF 时间点（最多四个）。
- 自定义倍速 `DT` (1.01–2.00x) 和 `HT` (0.50–0.99x)。
- DA (Difficulty Adjust) 支持：`dacs<CS>` `daar<AR>` 等参数。
- Mania 和 Taiko 的 SV 指示与 BPM 标签。
- 批量渲染脚本 `batch_render.ps1`。
- GitHub Actions CI 工作流与 MIT License。

### Fixed
- 修复 Standard 红线处 Slider Velocity 未重置的问题。
- 修复 Bezier / Perfect Curve 滑条方向计算错误。
- 修复 Taiko 转谱 GIF 中 SV 影响 PNG note 间距的问题。
- 修复 Catch 内存泄露与首次运行缺少 output 目录的问题。
- 修复 Mania 转谱 SV 错误与时间标签重叠问题。
- 修复 Taiko Gimmick 谱面渲染崩溃。
- 修复 GIF 渲染滑条结束后残留的问题。
- 修复 Standard 谱面缺少 AR 时的兼容处理。

### Performance
- 大幅减少渲染内存占用。
- 大幅提升 GIF 渲染速度及各模式渲染速度。

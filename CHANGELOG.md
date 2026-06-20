# Changelog

All notable changes to this project will be documented in this file.

---

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

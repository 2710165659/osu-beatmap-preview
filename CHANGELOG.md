# Changelog

All notable changes to this project will be documented in this file.

---

## [1.1.1] - 2026.09.05

### Added

- 新增 `--output-dir=<DIR>`，可为本次渲染指定输出目录根路径；命令行参数不会参与配置哈希。
- Mania MP4 新增 `render.mania.mp4.style.LANE_DARKEN_ALPHA`，可配置黑色轨道暗化层的透明度，取值范围为 `0..=1`。

### Changed

- Mania 列背景不再按每个 `KEYS_N` 重复配置 `COLUMN_COLORS`，统一使用内置列背景色常量。
- 重整项目目录结构，按 `application`、`domain`、`infrastructure` 和 `render/modes` 划分应用、领域、基础设施与模式渲染代码。

### Fixed

- 修复不同谱面时长下，轨道宽高上限没有正确应用当前输出 `SCALE` 的问题。

## [1.1.0] - 2026.09.04

### Added

- 新增 `--fps=<1-60>`，可为 GIF 和 MP4 单次渲染覆盖配置中的帧率；显式帧率会参与输出缓存区分。
- GIF 新增 `--duration-time` 支持，可让每个选定时间点分别渲染指定时长；未指定时仍使用对应模式的默认片段时长。
- 新增四种模式、三种输出格式分别独立的 `SCALE` 配置，以及 `--scale` 单次输出倍率覆盖；倍率会在绘制前作用于文字、图形、间距和布局尺寸。
- 扩展库接口 `PreviewOptions`，支持传入外部配置、输出倍率和 GIF/MP4 帧率。
- Catch PNG 新增香蕉雨推荐接盘路线、edge 跳跃引导线和边缘 combo 数字，并可通过 `layout.catch.png.SHOW_BANANA_ROUTE` 开关路线计算与绘制。

### Changed

- 重整四种模式的布局配置，PNG、GIF 和 MP4 可分别配置画布边距、信息区、间距、标签、背景和视频样式；MP4 背景图与暗化程度不再由所有模式共用一组配置。
- Mania 的各输出格式可独立开关 SV 标签，PNG/GIF/MP4 的键道宽度和边界线宽度按对应皮肤配置参与布局。
- Catch 渲染物件更改为新样式：水果和水滴使用带白色边框的实心圆，香蕉使用空心圆环，Hyper Dash 使用外环提示。
- Catch PNG 的多列高度会按主要小节间隔对齐，时间标签旁新增 BPM 信息；edge 引导线会在列边界处连续衔接，减少跨列阅读歧义。
- DA 参数改为在确定目标模式后按各规则集的 Extended Limits 校验；Standard 支持 AR `-10` 至 `11`、CS/OD/HP `0` 至 `11`。
- 标准化字体、标签、边距和游玩区域的缩放计算，非 `1` 倍输出会使用倍率后缀区分文件名。

### Contributors

- `yaowan233`：贡献 Catch 预览改进，包括新样式、香蕉雨推荐路线、edge 引导线、边缘 combo 数字、列高对齐和时间/BPM 标签优化。

### Performance

- Catch 物件更改为新样式后，减少了光斑精灵生成与缓存开销，提升 Catch 渲染性能。


## [1.0.8] - 2026.08.30

### Added

- 新增统一的 YAML 配置系统。默认配置编译进可执行文件，启动时依次合并可执行文件同目录的 `config.yml` 与 `--config` 覆盖；`--config` 支持文件路径和内联 JSON/YAML，并对未知字段、类型和取值范围进行校验。
- 新增完整的可配置项，覆盖输出、缓存、配置和日志目录，以及四模式布局、颜色、皮肤、网络下载、GIF/MP4 并行参数、音视频码率与渲染行为；原独立 `skin.ini` 内容已并入默认配置。
- 新增配置隔离的输出缓存。非默认配置按规范化差异生成稳定的 6 位哈希目录，并保存只包含非默认字段的 `config.yml`；等价配置复用相同输出目录，差异配置文件采用原子写入。
- 新增 PNG、GIF 和 MP4 独立的整次请求超时，默认均为 300 秒，覆盖下载、解析、转谱、缓存检查、渲染、音频处理、编码和落盘；超时会取消后台任务、清理临时文件，并保留已有有效输出。
- MP4 新增谱面背景图支持：从 OSZ 的 `[Events]` 读取背景，兼容带引号、逗号、Windows 路径分隔符和大小写差异的文件名；背景会等比适应视频画布并保留黑边，可配置开关和暗化程度。
- 新增 Rust 库接口 `PreviewOptions`、`TimePoint` 与 `generate_preview`，可在不启动 CLI 进程的情况下调用渲染流程。
- Standard 新增 osu! stable 风格的 Stack Leniency 堆叠，以及 Slider 中间 tick、重复段 tick、Hidden/Traceable 生命周期和透明度处理。
- Taiko GIF/MP4 新增随谱面滚动的小节线、鼓滚 tick 及命中动画；小节线遵循红线节拍相位与 `OmitFirstBarLine`，并支持分别配置 GIF 与 MP4 的显示开关。

### Changed

- 重整命令行接口：Mod 使用可重复的 `--mod`，时间点使用可重复的 `--time-points=<秒数|preview>`，MP4 时长使用 `--duration-time`，外部配置使用 `--config`。旧的参数别名和专用预览参数不再接受；相关布局、间距和日志目录改由配置控制。
- GIF 与 Standard PNG 的时间点改为按布局容量处理：显式时间点未填满时优先补入 `PreviewTime`，其余片段以确定性方式选择并避免重叠。
- MP4 默认从游戏时间 `0` 请求 600 秒；短谱面输出完整可播放范围，超出谱面尾部的区间整体前移。只有显式传入时间参数时，输出文件名才追加时间后缀。
- 配置、下载、解析、校验和渲染结果统一通过 stdout JSON 返回，诊断信息保持在 stderr；命令行参数错误仅输出到 stderr。stdout 结果不再附加构建信息和日志路径字段。
- 日志路径统一由 `paths.LOG_DIR` 控制，日志文件名由配置指定；日志写入失败仍只降级为 stderr 提示，不中断渲染。
- 渲染器、下载器、日志和编码参数改为从编译期生成的强类型配置读取，减少分散常量并保持单文件发布。
- 批量 PNG/GIF 与 MP4 基准脚本已适配新的命令行参数和默认视频文件名。

### Fixed

- 修复 Taiko 转盘和鼓滚尾部出现黑边的问题。

### Performance

- GIF 与 MP4 帧渲染按配置分块并行，并根据单帧字节数动态限制并行批次，降低大画布渲染的临时内存峰值。
- 增加 Standard Slider tick、Taiko 鼓滚 tick 等程序化精灵缓存，减少重复生成和缩放开销。

## [1.0.7] - 2026.08.02

### Fixed

- OpenH264 CPU 编码器现在定期生成真实 IDR 帧，并按实际 H.264 NAL 类型写入 MP4 同步样本索引，修复 Linux 视频无法跳转以及 QQ 只能显示首帧的问题。
- 修复 OpenH264 码率控制跳过视频帧后仍向 MP4 写入零长度 sample 的问题；现保证每个输入帧都有有效 H.264 数据，并在封装前拒绝空 sample，修复 QQ Windows 播放失败或中途停止。
- MP4 封装完成后会内置执行 faststart 重排，将 `moov` 索引移动到文件前部并修正 chunk offset，修复 QQ 等聊天预览器只能播放前几秒或约 10 秒后停止的问题。
- 输出文件（PNG / GIF / MP4）改为原子写入：先写同目录临时文件，全部完成后才替换最终文件，渲染中断（如进程被强制关闭）不再在缓存路径留下损坏的半成品。缓存命中新增格式完整性校验，已损坏的旧缓存会被识别并自动重新渲染。
- OSZ 下载进度日志现在会记录请求使用的 bid，便于区分同一谱面集内不同难度的渲染请求。
- MP4 遇到 `.osu` 中缺失或无效的 `BeatmapSetID` 时，现在会根据 osu! 官方谱面页的重定向地址解析真实谱面集 ID，避免因无法下载 OSZ 音频而渲染失败。

### Added

- 新增多进程安全日志系统：`render.log`（NDJSON 汇总，每谱面一行，含时间、bid、渲染时长、谱面信息与各阶段耗时）与 `progress.log`（可 `tail -f` 实时查看的阶段事件流），默认写入 `<临时目录>/osu-beatmap-preview/logs`。
- 新增 `--log-dir=<DIR>`（覆盖日志目录）与 `--no-log`（关闭日志）参数，支持 `OSU_PREVIEW_LOG_DIR` 环境变量；stdout JSON 增加可选 `log` 字段，不影响现有解析。
- 新增 `--preview-30s`，支持在 `.osu` 的 `PreviewTime` 附近输出约 30 秒 MP4 预览视频。
- 新增 `--gif-clip` 与 `--gif-clip-label`，支持输出单屏连续 GIF；未指定时间区间时默认时长为 10 秒。
- 渲染汇总日志新增 OSZ 下载耗时、缓存命中状态与视频处理耗时。

### Changed

- `--time` / `--times` 改为使用 osu! 游戏皮肤时间轴：转谱后目标模式首物件为 `0:00`，支持负时间；所有可见时间标签同步采用该时间轴，MP4 右上角显示“当前皮肤时间 / 全谱可玩总时长”，内部谱面与音频计算仍使用绝对时间。
- OpenH264 CPU 编码器改用约 500 kbps 的独立目标码率，在画质、文件体积和编码速度之间取得平衡。
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

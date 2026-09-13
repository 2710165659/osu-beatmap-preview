# Changelog

All notable changes to this project will be documented in this file.

---

## [Unreleased]

### Added

- Web 播放页新增音量调节（0–100%，默认 50%），位于齿轮抽屉的「声音」分组，拖动即时生效且换谱面后沿用。
- 新增打击音（hit sound）支持，覆盖 CLI 的 MP4 导出与 Web 实时预览：Standard / Catch / Mania 使用 argon pro (2022) 音效，Taiko 使用 osu! "classic" (2013)；谱面音效组、音量、滑条 tick/滑行音、转盘音、果汁流小果都按 osu! 规则还原。音效资源内嵌进可执行文件与 wasm，读取失败按静音处理。
- `assets/shared_config.yml` 各模式的 `mp4` 小节新增 `ENABLE_HITSOUND`（布尔，默认开启）与 `HITSOUND_VOLUME`（百分比，默认 50，与游戏内默认音量等效）。
- WASM 新增打击音接口：`hitsoundNames`、`hitsoundAsset`、`hitsoundAssetCount`、`hitsoundDefaults`，以及会话上的 `enableHitsound` / `setHitsoundSample` / `rebuildHitsoundTimeline` / `setHitsoundVolume` / `positionHitsound` / `seekHitsound` / `renderHitsound` / `takeHitsoundBuffer` / `resetHitsoundSamples`；画面与声音的时间轴统一由 wasm 维护，宿主只负责解码样本与输出 PCM。
- Web 播放页新增「打击音」开关与音量滑杆（默认开启、50%），与音乐音量独立调节。
- core 新增游玩/回放接口骨架（`gameplay::{GameplayMode, InputSnapshot, InputSource, JudgementEngine, ScoreSnapshot, GameplayOverlay, OverlayStyle, GameplayOptions}`）、打击音显式触发 API（`HitsoundMixer::{trigger, start_loop, stop_loop}`、`LoopHandle`）、`SampleLibrary::id_of` 与 `hitsound::has_embedded_asset`，为「谱面自带音效 / OSR 回放联动 / Web 游玩」预留接口（默认全部不生效，接口说明见 `docs/architecture.md` 的「后续功能接口」）。
- `docs/architecture.md` 新增「媒体资源与 `.osu`/`.osz` 传输路径」与「音频-画面-打击音时钟模型」两节；`crates/osu-beatmap-preview-wasm/README.md` 补充音频时间轴契约（音频文件 0 点 == 谱面 0 点，`AudioLeadIn` 只影响起点）。

### Changed

- 预览与 CLI 的 MP4 现在共用同一个「完整区间起点」函数（`preview_start_ms`）：首个物件前 2000ms，谱面 `AudioLeadIn` 更大时按它提前。此前 Web 预览固定用 2000ms、CLI 才尊重 `AudioLeadIn`，同一张谱面在两边的起点可能不同；受影响的是 `AudioLeadIn > 2000` 的谱面（其 Web 预卷会变长）。
- 媒体条目策略统一到 core 的 `processing::media`（路径归一化、音频/背景/自带音效条目，以及 `preview_start_ms`）；CLI 删除自己那份重复实现，Node 后端保持自写实现并由新的 `test/media-contract.test.js` 契约测试钉住。
- 压缩包条目「归一化后什么都不剩」时，Node 后端由返回空串改为返回 `null`，与 core 完全一致。
- 移除 core 里没有生产调用方的 `AudioData` / `RealtimeSession::set_audio` / `ResourceBundle::audio`，`cli::ResourceLoader::bundle_from_files` 随之去掉音频参数：音乐字节本来就不进 core（Web 用 `<audio>` 播放，导出由宿主解码后与打击音混音）。

### Fixed

-  修复部分浏览器下因时序问题导致谱面背景异常的bug
- 修复打击音在混音窗口边界被跳过的问题：正好落在窗口末尾（例如整秒）的打击音会被永久丢弃，导致 CLI 与 Web 的打击音大部分或全部静音。
- 修复 Web 端打击音与画面完全对不上的问题，两处根因：
  - 共享环形缓冲的控制字发布的是**绝对**帧号，而音频线程用**环形帧下标**比较，播放绕回一圈后数据被判定为「未就绪」，声音时有时无且位置错乱；现在两端统一使用相对帧数。
  - 混音锚点每帧都被当前时间覆盖，导致已混好的数据对应的时间被改写、写入位置一路领先播放位置；现在锚点随播放推进结算，写入位置始终只领先预读窗口。
  - 向后 seek 时未把音频线程的读取指针拨回，它会读到新写入区之外的垃圾数据。
- 修复 Web 端打击音「自动播放时没有声音、拖动进度条不恢复、必须在设置里手动开关一次才出声，开关之后 seek 或暂停再播放又会消失」的问题：预读窗口此前只按画面时钟计算，而音频线程由音频硬件时钟驱动、稳定地领先画面时钟几十毫秒，于是它一路追到写入前沿、读到的全是静音。现在填充目标同时看住音频线程回报的已读位置，并保证写入前沿领先它一个预读窗口（约 170ms）；音频线程回报位置也从每约 30ms 加密到每约 11ms。同时给音频线程加了欠载计数，真的跟不上混音时会在播放日志里提示一次。
- 修复混音器里已播完的声音不会回收的问题：时长 0 的打击音事件结束时间是无限，声音列表因此随播放无限增长，每个输出帧都要遍历整张列表，长谱面会把混音拖慢到实时以下（Web 端表现为打击音整体消失）。
- 修复混音器窗口边界的事件被播放两次的问题：正好落在窗口末尾的事件一度被前后两个窗口各收一次，音量凭空翻倍；现在窗口是 `[start, end)`，边界事件只归下一个窗口。
- 修复滑条音效参数取错列的问题：`.osu` 中滑条的 `hitSample` 位于第 11 列，此前按第 6 列解析会把曲线数据当成音效参数，导致音效组与音量全错。
- 修复 `hitSample` 全为 0（最常见情况）时未回退到 timing point 的问题：音效组与音量应随时间点变化，此前会被写成固定的 normal 组与 100 音量。
- 修复缺少背景声明的谱面（或背景条目不在 OSZ 里）在 Web 端整张加载失败的问题：背景改为可选，缺失时前端退化成纯色背景，音频与画面照常工作（与 CLI 的行为一致）。

## [1.2.2] - 2026.09.13

### Added

- Web 后端新增 `--tls-host=<HOST>`（可重复，也可用 `OSU_PREVIEW_TLS_HOSTS` 指定），用于把容器/服务器实际访问的域名或 IP 加进自签证书的 SAN。
- 新增 `Docker/Dockerfile-web` 与 `.gitattributes`，可构建包含 wasm 与前端产物的可部署镜像。
- Web 站点支持 `/?bid=<BID>` 深链直接进入预览，可选 `&convert=<模式>`。
- WASM 新增 `beatmapInfo(bytes)`，返回谱面概览、统计与难度等内部信息；core 新增对应的 `BeatmapInfo`。
- Web 播放页新增谱面信息条（名称与难度名），帧率新增 120 FPS 选项。
- Web 加载进度改为按阶段展示：`/resource/progress` 上报连接镜像、下载谱面包、解包、传输、就绪等阶段与字节数，前端显示实时速率与预计剩余时间；音频与背景后台下载期间在播放页顶部显示细进度条。
- Web 播放页日志新增各阶段耗时汇总，便于判断云端部署时慢在哪一段。

### Changed

- Web 包前端改为 Vue 3 单文件组件 + Tailwind CSS v4，由 Vite 构建到 `dist/`，后端模块移到 `backend/`；启动方式仍是 `node backend/server.js`，最终用户仍只需要 Node.js。
- Web 播放页重排：画面参数、Mod 与日志收进齿轮抽屉，移动端改用 `100dvh` 动态视口高度。
- Web 播放页进度条改为活动时显示、静止约 1 秒后淡出；左右方向键改为松开时跳转 ±5 秒，并支持长按快进与倒带。
- DA 参数面板改为勾选 DA 后才出现；`navigator.gpu` 为空时区分“非安全上下文”与“浏览器不支持 WebGPU”。
- Web 后端同时支持 `--flag=value` 与 `--flag value` 两种参数写法。

### Fixed

- 修复 Web 播放页播到结尾后再次播放报 `play() request was interrupted by a call to pause()` 的问题。
- 修复 Web 播放页“静音播放中”角标不消失的问题：改为按音频实际是否出声判定，并在首次点击/触摸/按键时于用户手势里恢复声音。
- 修复 Web 播放页音频重试策略：仅在缓冲与 seek 之后续播，按 500 ms 节流，不再每帧调用 `play()`。
- 修复移动端音画不同步的问题：`currentTime` 的赋值是异步 seek，此前设完就当完成；现在改为等待 `seeked` 事件、期间冻结画面时钟，并在暂停/恢复与切换帧率、分辨率时对齐音频。
- 修复暂停后再次播放时画面前进几帧即冻结、只剩音频的问题：去掉会与音频位置走散的“已落实进度”快照，改以音频自身的 `currentTime` 为唯一时钟基准。
- 修复 Web 加载页阶段文案不准确的问题：解包时不再显示“下载谱面包”，未拿到首字节时显示“服务端连接镜像”。

### Performance

- Web 加载的关键路径解耦：会话就绪即可进入播放页，音频与背景改为后台并行下载，不再等音频元数据。
- Web 播放页减少无谓的音频 seek：仅在确实需要挪动时 seek，且同一时刻只允许一个 seek 在飞。

---

## [1.2.1] - 2026.09.11

### Changed

- 版本号集中到根 `Cargo.toml` 的 `[workspace.package].version`，各 crate 改用 `version.workspace = true` / `edition.workspace = true` 继承；Web 包的 `package.json` 版本改为由 `scripts/version.mjs` 从根 `Cargo.toml` 单向同步（`npm run sync:version`，`npm run build:wasm` 与发布流程会自动执行），不再手工维护。

### Fixed

- 修复 Standard 转 mania 在 1K 以及部分 4K 谱面上报 `not enough columns to complete mania conversion` 的问题：1K 滑条现在按上游直接产出单根长条，4K 镜像路径不再错误清零第三个音符的概率，随机列查找在受限范围内无解时回退到全列确定性扫描（对应上游会抛 `NotEnoughColumnsException` 的路径）。
- 修复 `slider_gen_holds` 在上一模式占满全部可用列时生成音符数量与上游不一致的问题。

### Tests

- 新增 `3451313` 转谱 fixture 及 1K/4K golden 快照，并新增 1K–10K（含 DS）全键数转谱冒烟测试。

---

## [1.2.0] - 2026.09.11

### Added

- 新增平台无关的 `osu-beatmap-preview-renderer` crate，提供基于 WGPU 的 `SurfaceRenderer`、`OffscreenRenderer`、流式 readback、取消令牌和分类渲染错误。
- 新增 `osu-beatmap-preview-wasm` crate，提供 `WebGpuSession`：宿主把 `.osu` 字节和 WebGPU Canvas 交给 WASM，每帧在本地 GPU 绘制，不返回 RGBA 缓冲、不通过 HTTP 轮询帧。
- 新增 Web 站点与下载后端 `crates/osu-beatmap-preview-web`（Node.js，无第三方依赖）：静态站点在浏览器里用 WebGPU 播放谱面，后端只负责跨域下载并缓存 `.osu` 与 `.osz`、从 OSZ 解出音频和背景；支持 Mod 热切换、转谱、seek、`0.5x..=2.0x` 倍速和 1080P/720P/480P 切换。
- core 的公开入口固定为 `api`、`model`、`render`、`processing`，新增 `RealtimeSession`、`ResourceBundle`、`RealtimeOptions`、`TimelineInfo`；`FrameScene` 成为跨平台单帧场景描述边界。
- 新增 GUI 与移动端平台适配骨架 crate，只定义 surface、输入、播放生命周期和音频时钟边界。
- 新增 CLI 使用说明 `crates/osu-beatmap-preview-cli/README.md`、Web 站点说明 `crates/osu-beatmap-preview-web/README.md` 和 WASM 使用说明 `crates/osu-beatmap-preview-wasm/README.md`；根目录 README 精简为架构概览与文档入口。

### Changed

- 项目改为 Cargo workspace，不再保留根包，代码迁入 `osu-beatmap-preview-core`、`osu-beatmap-preview-renderer`、`osu-beatmap-preview-cli`、`osu-beatmap-preview-wasm`、`osu-beatmap-preview-gui`、`osu-beatmap-preview-mobile`；Web 站点是 Node.js 项目，不参与 Cargo 构建。
- workspace 默认成员是 `osu-beatmap-preview-cli`，普通 `cargo build --release` 只构建 CLI；CLI 二进制名从 `osu-beatmap-preview` 改为 `osu-beatmap-preview-cli`，`--help` 与 `--version` 的输出同步更名。
- core/CLI 职责重新划分：四模式的 CPU 单帧与静态场景绘制位于 core，CLI 只负责参数解析、配置、下载、缓存、日志、时间序列与布局组装、媒体编码和文件输出，且不依赖 renderer。
- 默认配置拆分为 core/CLI 共享的 `assets/shared_config.yml` 和 CLI 专用的 `crates/osu-beatmap-preview-cli/assets/cli_config.yml`；CLI 启动时把两者合并为内嵌默认配置，core 在构建期生成 `CoreConfig` 默认值。
- 发布产物固定为 CLI 与 Web 包两类：CLI 的 4 个平台资产文件名以 `-cli` 结尾，Web 包以 `osu-beatmap-preview-web.zip` 发布，内含静态站点（含 wasm 产物）、Node.js 下载后端、说明与许可证。
- Web 后端的下载与缓存策略与 CLI 对齐：多镜像竞速、Range 分块并行、osu.direct 优选 IP 与超时回退，并复用同一份缓存目录布局。
- 实时预览右上角时间标签改为跟随实际 Canvas/合成尺寸布局，不再受固定初始分辨率影响。

### Fixed

- 修复实时预览右上角时间标签随分辨率变化而错位的问题。
- 修复 Standard 转 mania 时 1K 与部分 4K/6K 谱面报 "not enough columns to complete mania conversion" 的问题：1K 滑条现在直接产出单根长条，4K 镜像概率不再被错误清零，随机列查找在受限范围内无解时回退到全列扫描。

### Performance

- 复用 GIF 帧编码缓冲区，减少逐帧分配。
- Standard MP4 预先计算每帧可见物件索引，减少逐帧筛选开销。

### Compatibility

- CLI 的 PNG/GIF/MP4 行为、配置项和 `--bid`、`--mod`、`--time-points` 等参数继续沿用 1.1.1；主要破坏性变化是二进制名、发布资产名和 workspace 内部 crate 路径。
- WGPU 绘制与实时播放只由 renderer、WASM 和 Web 站点使用；CLI 保持 CPU 导出路径。

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

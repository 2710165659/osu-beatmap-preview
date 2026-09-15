# 架构说明

项目将文件导出与实时绘制分开。核心数据和场景描述不依赖窗口、网络或音频设备；GPU 代码由 renderer crate 提供，平台适配层负责创建 surface。

```text
CLI -> cli 应用层 -> 文件/下载/缓存/配置/媒体适配
                    `-> core 单帧/静态场景计算与绘制 -> export 时间序列、布局组装 -> PNG/GIF/MP4 编码

Web -> Node 后端下载并缓存 .osu/.osz -> 浏览器 -> wasm -> core RealtimeSession -> renderer WebGPU Canvas
```

## 目录职责

- `crates/osu-beatmap-preview-core`：谱面模型、`.osu` 解析、Mod、转谱、时间轴、四模式 CPU 单帧/静态场景绘制、`FrameScene`、打击音事件时间轴与混音。不接触文件、网络、音频设备，也不解压 OSZ、不解码图像/音频：这些都由宿主完成后把数据传进来。
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

## 媒体资源与 `.osu` / `.osz` 传输路径

外部文件只有 `.osu` 与 `.osz` 两种，都由宿主负责获取；core 只认识「字节」和「已解码的数据」。

```text
CLI（bid 驱动）
  bid → https://osu.ppy.sh/osu/<bid>
      → <CACHE>/osu-download-cache/<bid>.osu          （缓存命中即复用）
      → core::parse_beatmap_bytes                     （全项目唯一的 .osu 全量解析）
      → MP4 需要音频时才下载 .osz：
          <CACHE>/osz-download-cache/<set_id>.osz     （多镜像竞速 + Range 分块 + 优选 IP）
          zip crate 抽音频条目 → <CACHE>/osz-download-cache/<set_id>/<fnv1a64(条目名)>.<ext>
          image crate 解背景 → RGBA（内存）            （缺背景/条目缺失 → 纯色背景）
          symphonia 解音频 → fdk-aac 编码

Web（Node 后端 + 浏览器）
  GET /resource/beatmap    → <CACHE>/osu-download-cache/<bid>.osu → 原样发给浏览器
  GET /resource/audio      ┐
  GET /resource/background ┘→ ResourceProvider.media（单飞：两个请求只下一次 .osz）
      → 自己的最小 ZIP 读取（backend/zip.js）+ Node 自带 zlib
      → <CACHE>/media/<bid>/{audio,background}.<ext>   （浏览器专属的媒体缓存）
  浏览器：
      .osu  字节 → wasm.beatmapInfo(bytes)（信息面板）
                → WebGpuSession.create(bytes, canvas, options)（渲染会话）
      背景  字节 → createImageBitmap → 画布 → getImageData → set_background_rgba（整帧拷贝进 wasm）
      音频  字节 → Blob → <audio> 元素（**不进 wasm**，音乐由浏览器直接播放）
      打击音 → 不走网络：wasm 内嵌 ogg → hitsoundAsset 取字节 → Web Audio 解码 → setHitsoundSample(PCM)
```

要点：

- **条目策略只有一份**：`.osu` 里声明的文件名怎么归一化、需要压缩包里的哪些条目，由 core 的 [`processing::media`](../crates/osu-beatmap-preview-core/src/domain/media.rs)（`normalize_entry_path` / `BeatmapMedia`）定义；CLI 直接调用，Node 后端保持自写实现但被 `test/media-contract.test.js` 用同一张用例表钉住。
- **音频必需、背景可选**：缺音频（或条目不在压缩包里）是致命错误；缺背景（未声明、路径非法、条目缺失）只退化成纯色背景，Web 端由 `loadBackground` 的失败分支处理。
- **缓存契约**：`.osu`（按 bid）、`.osz`（按 set id）、优选 IP JSON 与锁（缓存根）两项前端共享；解出来的媒体不共享——CLI 放在 `osz-download-cache/<set_id>/`，Web 放在 `media/<bid>/`。
- **整包不进浏览器**：后端解出音频与背景再发（通常几 MiB），而不是让浏览器下载几十 MiB 的 `.osz`。

## 音频-画面-打击音的时钟模型

实时预览只有**一个时钟**：`state.position` 是游戏时间，`absoluteTime() = absoluteStart + position` 是谱面绝对时间（0 = 音频文件 0 点），音频元素的 `currentTime * 1000` 就是这个绝对时间。三套坐标的换算只有这一处，宿主不得再叠加 `AudioLeadIn`（它只体现在 `absoluteStart` 里）。

- **起点**：`absoluteStart = preview_start_ms(首个物件, AudioLeadIn)`，即首个物件前 2000ms、`AudioLeadIn` 更大时按它提前；首物件很早时该值为负，此时绝对时间 `< 0` 的前置段没有音频可播（前端停住音频、只让画面时钟走）。CLI 的 MP4 完整区间用同一个函数，因此导出与预览的时间轴一致（MP4 的尾部留白是格式差异，仍由 CLI 配置决定）。
- **真源**：播放中 `audio.currentTime` 是唯一事实（`syncClockFromAudio` 每帧回写，偏差超过 1500ms 才补一次 seek）；音频还没出声（下载/缓冲/自动播放被拦）时画面按墙钟推进并周期性重试播放。
- **seek**：赋值 `currentTime` 前先 `pause` 并冻结画面（`audioSeekPending`），等 `seeked` 或超时；拖动期间只保留最后一个目标。
- **暂停**：先把进度对齐到音频当前位置（80ms 容差），再 `pause`，恢复时才不会跳帧。
- **倍速**：`audio.playbackRate = 倍速 × 谱面倍速`，画面按同一倍率推进，打击音内核的 `rate` 同步。
- **打击音**：事件时间轴也用谱面绝对时间；宿主把换算好的位置交给混音器，混音结果写进与 AudioWorklet 共享的环形缓冲，写入前沿始终领先音频线程约 170ms（见 `src/hitsound-stream.js`）。
- **MP4 导出不走这条实时链路**：用「输出帧号 ↔ 谱面时间」的纯函数在同一 48kHz 下标上混合音乐与打击音。

## 后续功能接口

以下三项功能**尚未实现**，但接口、配置位与数据流已经留好（见 `crates/osu-beatmap-preview-core/src/gameplay.rs`、`src/domain/media.rs` 与 `src/hitsound/mixer.rs`）：

1. **谱面自带音效**：`HitSample::filename` 与 `referenced_names` 已经把自定义文件名收集出来；`BeatmapMedia::from_beatmap` 的 `samples` 给出「内嵌皮肤没有、需要去 OSZ 找」的条目，`has_embedded_asset` 用来区分两者，`SampleLibrary::insert` 负责填充。契约是**样本源优先级：OSZ 内同名条目 > 内嵌皮肤 > 静音**（后写入覆盖先写入即可）。保留的配置位是各模式 `render.<mode>.mp4.style.ENABLE_BEATMAP_HITSOUND`（默认 true）——注意 `crates/osu-beatmap-preview-core/src/generated_config.rs` 是随仓库提交的生成文件（仓库内没有生成脚本），接入时要同时改 `assets/shared_config.yml` 与它。
2. **OSR 回放与画面联动**：`gameplay::{InputSnapshot, InputSource, JudgementEngine, ScoreSnapshot, GameplayOverlay, OverlayStyle}` 定义输入、判定与 HUD 的契约；OSR 的 LZMA 解码计划放在 core 的可选特性里（默认不启用，CLI 与 wasm 共享一份实现），CLI 侧用 `RenderRequest::gameplay`、Web 侧用 `RealtimeOptions::gameplay` 接入。HUD 画在画布内（实时用场景命令、CLI 用 `Img` 绘制），DOM 只做控制面板。
3. **Web 游玩**：`GameplayMode::Play` 下打击音改由输入触发；为此 `HitsoundMixer` 已经提供 `trigger` / `start_loop` / `stop_loop` 显式触发 API（预览路径继续走时间轴）。输入偏移（`GameplayOptions::input_offset_ms`）留给延迟补偿。

三项功能共用同一条链：**输入快照 → 判定引擎 → 状态快照 → 画面叠加 + 打击音触发**，因此只接入一次渲染分支。

## 打击音（hit sound）

打击音的「什么时候播放哪个样本、多大声」集中在 core 的 `hitsound` 模块里：它按四模式的 osu! 规则（Standard 的滑条 tick/滑行音、转盘的旋转音按 autoplay 转速（477 RPM）换算进度做音高调制并在每转满一圈时发奖励音、Taiko 走 legacy（classic 皮肤）路径——音效组取自物件 `hitSample` 与所在 timing point、鼓边按 `clap|whistle` 判定、strong 追加 `finish`/`whistle`、连打逐 tick、大连打按 autoplay 节奏交替敲击，Catch 的果汁流小果、Mania 的长条）把谱面展开成事件时间轴，并提供一个与音频设备无关的离线混音器。样本 PCM 由宿主提供，core 不接触文件、网络或音频设备。

模块按职责拆分：`hitsound/mod.rs` 只放公开入口（`build_timeline` / `referenced_names` / `volume_gain` / `SAMPLE_RATE`）与再导出，`sample.rs` 管样本与名字解析，`timeline.rs` 管事件类型与构建器（含候选名的栈上拼接），`common.rs` 放各模式共用的取样与 timing point 辅助，`standard.rs` / `taiko.rs` / `catch.rs` / `mania.rs` 各自按 osu! 规则展开物件（测试与被测模块同文件），`mixer.rs` 把时间轴混成 PCM，`assets.rs` 是内嵌样本表。

- CLI：`build.rs` 把 `assets/hitsound/*.ogg` 内嵌进可执行文件，导出 MP4 时用 symphonia 解码被引用到的样本，再按视频输出时间轴整段混音后交给 AAC 编码器；音乐与打击音共用同一个 48kHz 输出下标（`chart_start + i * 1000 * speed / sample_rate`），倍速通过把混音器的内部采样率取 `sample_rate / speed` 实现，因此时间与音高都和音乐、Web 端一致；视频区间起点可能为负（首个物件前的预卷），混音位置同样允许为负。混音按 1 秒窗口分块渲染，避免把整段事件压在声音列表里。任何样本读取失败都退化为静音，不影响导出。
- Web：core 构建时把同一批 ogg 内嵌进 wasm（`hitsound::asset_bytes`），页面按名字取字节、用 Web Audio 解码成 PCM 再交给 wasm；wasm 在音频线程的时钟下推进时间轴并混音，画面与声音使用同一条时间轴（见 [WASM 使用说明](../crates/osu-beatmap-preview-wasm/README.md)）。宿主只负责解码与输出，不需要下载音效文件。

开关与音量来自各模式 `render.<mode>.mp4.style` 的 `ENABLE_HITSOUND` 与 `HITSOUND_VOLUME`（0～100）：与 osu! 一样按 `v / 100` 换算为线性增益（`SkinnableSound` 的映射），地图里每条 timing point / 物件的音量再叠乘其上。

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

当前版本不包含正式 GUI/移动端产品 UI、WGPU PNG/GIF、外部 texture 编码 API 或 NVENC/AMF 零拷贝；GUI/mobile crate 仅提供适配接口骨架。Web 站点的控制面包含播放/暂停、seek、方向键跳转、音量、`0.5x..=2.0x` 倍速、30/60 FPS 和 1080P/720P/480P 分辨率切换。

回放（OSR）解析与 Web 游玩**只预留接口，不含实现**（见「后续功能接口」）：core 不引入解压/图像解码/音频解码依赖，`.osz` 始终只在宿主侧解开，浏览器也不下载整包。

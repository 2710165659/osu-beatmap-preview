# WASM 使用说明（osu-beatmap-preview-wasm）

[返回项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [架构说明](../../docs/architecture.md)

WASM 产物把 core 的实时会话和 renderer 的 WGPU 绘制接到浏览器，让宿主网页在本地 GPU 上逐帧绘制谱面。它不下载文件、不返回像素数据，也不通过 HTTP 轮询渲染结果：`.osu` 字节、Canvas、音频和播放控制都由 JavaScript 宿主负责。

## 职责边界

| WASM 负责 | 宿主（JavaScript）负责 |
| --- | --- |
| 解析 `.osu` 字节、Mod、转谱和时间轴 | 获取 `.osu` / OSZ / 背景 / 音频并解码 |
| 按绝对时间生成单帧场景（`FrameScene`） | 维护播放时钟、暂停、seek、倍速 |
| 在浏览器 WebGPU Canvas 上绘制并呈现 | 提供 `<canvas>`、控制 UI 和音频输出 |
| 打击音的事件时间轴、倍速换算与混音（PCM） | 下载并解码打击音样本、按音频硬件时钟输出 PCM |
| 会话内的 Mod 热切换与画布尺寸变更 | 处理按键、指针等输入事件 |
| 从 `.osu` 字节汇总谱面内部信息（`beatmapInfo`） | 按 bid 取回 `.osu` 字节并展示信息 |

WASM 不返回 RGBA 缓冲，因此宿主拿不到像素结果；需要图片或视频文件时请使用 [CLI](../osu-beatmap-preview-cli/README.md)。在官方 [Web 包](../osu-beatmap-preview-web/README.md) 里，表右侧的下载职责由 Node.js 后端完成，音频播放、背景解码和播放控制由页面脚本完成。

## 获取

本 crate 没有独立的发布包：它的产物位于 Release 里 [Web 包](../osu-beatmap-preview-web/README.md) 的 `public/pkg/` 目录下。

| 文件 | 说明 |
| --- | --- |
| `osu_beatmap_preview_wasm.js` | `wasm-bindgen --target web` 生成的 ES 模块与默认初始化函数 |
| `osu_beatmap_preview_wasm_bg.wasm` | WebAssembly 二进制 |
| `osu_beatmap_preview_wasm.d.ts`、`osu_beatmap_preview_wasm_bg.wasm.d.ts` | TypeScript 类型声明 |

## 构建

需要稳定版 Rust 工具链、`wasm32-unknown-unknown` 目标和与 `Cargo.lock` 中版本一致的 `wasm-bindgen-cli`。

最省事的方式是直接用 Web 包里的脚本，它会把产物写进 `public/pkg`：

```bash
cd crates/osu-beatmap-preview-web
npm run build:wasm
```

等价的手工步骤：

```bash
# 1. 安装目标和胶水工具；wasm-bindgen 的版本必须与 Cargo.lock 中的一致
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked

# 2. 构建 WASM
cargo build --release --package osu-beatmap-preview-wasm --target wasm32-unknown-unknown

# 3. 生成 JavaScript 胶水代码
wasm-bindgen --target web --out-dir pkg \
  target/wasm32-unknown-unknown/release/osu_beatmap_preview_wasm.wasm
```

本地验证时启动 Web 站点即可（它同时负责下载 `.osu`/`.osz`）：

```bash
cd crates/osu-beatmap-preview-web
npm install && npm run build   # 首次需要，生成 dist/
npm start
# 然后访问 http://127.0.0.1:8787
```

## 最小示例

用一个静态服务器托管 `pkg/` 与下面的页面，浏览器需要支持 WebGPU（Chrome / Edge 113+ 等）。`.osu` 字节由宿主自己获取；只想快速跑通时直接用 [Web 包](../osu-beatmap-preview-web/README.md)，它已经带好了静态站点和下载后端。

```html
<canvas id="stage"></canvas>
<script type="module">
  import init, { WebGpuSession } from "./pkg/osu_beatmap_preview_wasm.js";

  await init();
  const canvas = document.getElementById("stage");
  // .osu 需要同源可访问：Web 包的 /resource/beatmap?bid=738063 就是为此准备的。
  const bytes = new Uint8Array(await (await fetch("./738063.osu")).arrayBuffer());

  // 第 3 个参数是可选选项：convert、mods、width、height
  const session = await WebGpuSession.create(bytes, canvas, {
    convert: "mania",
    mods: ["4K", "DT"],
    width: 1280,
    height: 720,
  });

  // 游戏时间轴（转谱后首个可玩物件为 0:00）→ 绝对时间
  const absoluteStart = session.absolute_start_ms_number();
  const duration = session.duration_ms_number();

  const startedAt = performance.now();
  function frame(now) {
    const gameTime = (now - startedAt) % duration;
    session.render_number(absoluteStart + gameTime);
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
</script>
```

## JavaScript API

导出一个类型与三个函数，所有时间都用毫秒。

`WebGpuSession`：

| 成员 | 说明 |
| --- | --- |
| `WebGpuSession.create(bytes, canvas, options?)` | 异步创建会话。`bytes` 为 `.osu` 文件字节，`canvas` 为目标 `HTMLCanvasElement`，会话创建时会把 Canvas 宽高设为选项中的输出尺寸 |
| `render_number(absoluteTimeMs)` | 把该绝对时间的一帧绘制到 Canvas；时间必须为有限数字 |
| `duration_ms_number()` | 预览时长（含最后一个物件后的 2s 余韵），用于进度条和循环边界 |
| `absolute_start_ms_number()` | 游戏时间 `0:00` 对应的绝对时间；游戏时间到绝对时间即 `absoluteStart + gameTime` |
| `beatmap_speed_number()` | 当前谱面倍速（DT/HT 等），用于同步音频播放速率 |
| `width()` / `height()` | 当前输出尺寸 |
| `mode()` | 解析和转谱后的目标模式，小写字符串，如 `standard`、`mania` |
| `set_mods(mods)` | 热切换 Mod，数组每项是一个独立 token；切换后 `absolute_start_ms_number()` 可能变化，需要重新读取 |
| `set_background_rgba(width, height, rgba)` | 传入已解码的背景图 RGBA 数据；不调用时使用模式默认背景 |
| `resize(width, height)` | 同时更新 surface 与 core 的合成尺寸；只改 Canvas 不会改变渲染尺寸 |

### 音频时间轴与起点

- **音频文件的 0 点就是谱面时间轴的 0 点**：`audio.currentTime * 1000` 直接就是谱面
  绝对时间，宿主不需要、也不应该再叠加 `AudioLeadIn`——它只决定「从多早开始播放」，
  不改变音频与物件时间的对应关系。
- `absolute_start_ms_number()` 是预览起点：首个物件前 2000ms，谱面 `AudioLeadIn` 更大
  时按它提前；首物件很早时该值会是负数。游戏时间与绝对时间的关系是
  `绝对时间 = absoluteStart + 游戏时间`，起点变化后必须重新读取（`set_mods` 之后同理）。
- 绝对时间 `< 0` 的前置段没有音频可播（音频文件从 0 开始）：宿主应停住音频、只让画面
  时钟继续走，等绝对时间 `>= 0` 再开始播放。
- 打击音的时间轴同样使用谱面绝对时间，因此它和音乐天然共用这一套换算。

### 打击音（hit sound）

音频时间轴（什么时候播放哪个样本、多大声、倍速与 seek 怎么换算）全部在 WASM 内，
宿主只做两件事：把解码好的 PCM 送进来，把混音结果按音频硬件时钟送出去。因此画面与
声音始终使用同一份位置数据，不需要两处时钟互相对齐。

| 成员 | 说明 |
| --- | --- |
| `enableHitsound(volumePercent, sampleRate)` | 打开打击音。`sampleRate` 必须是宿主音频设备的采样率（`AudioContext.sampleRate`），否则输出会被按错误速率消费 |
| `disableHitsound()` | 关闭打击音，之后所有混音接口返回静音 |
| `hitsoundEnabled()` | 是否已打开 |
| `hitsoundRequiredNames()` | 当前谱面需要宿主提供 PCM 的样本名（按优先级，含裸名回退） |
| `hitsoundHasSamples()` | 是否已经放入过可用样本 |
| `hitsoundSampleRate()` | 当前混音采样率 |
| `setHitsoundSample(name, channels, sampleRate, loopLength, samples)` | 放入一段已解码的 PCM。`channels` 为 1 或 2，`loopLength` 是循环长度（采样帧，0 表示不循环），滑条滑行音与转盘旋转音需要传样本总帧数 |
| `rebuildHitsoundTimeline()` | 样本**全部放完后调用一次**；逐个样本调用会反复遍历整张谱面 |
| `setHitsoundVolume(volumePercent)` | 更新音量（0–100），按 osu! 的 `10^((v - 100) / 25)` 曲线换算 |
| `positionHitsound(chartTimeMs)` | 把混音位置对齐到谱面绝对时间，不清空正在播放的声音 |
| `seekHitsound(chartTimeMs)` | 跳到指定位置并丢弃正在播放的声音 |
| `hitsoundPositionMs()` | 当前混音位置（谱面毫秒） |
| `renderHitsound(frames)` | 从当前位置渲染 `frames` 个立体声采样帧，返回可读取的帧数（未启用时为 0） |
| `takeHitsoundBuffer()` | 取回上一批混音结果，返回交错立体声 `Float32Array`（长度 = 帧数 × 2） |
| `resetHitsoundSamples()` | 清空样本并重建时间轴（切 Mod/转谱后重新加载时使用），保留音量与采样率 |

推荐的使用顺序（官方 Web 页面的做法），音效字节优先取谱面自带的同名条目，取不到再用 WASM 内嵌资源：

```js
const session = await WebGpuSession.create(bytes, canvas, options);
const names = session.hitsoundRequiredNames();        // 需要哪些音效
session.enableHitsound(50, audioContext.sampleRate);   // 音频设备采样率
for (const name of names) {
  // 谱面自带的自定义音效要先从后端取（OSZ 里的同名条目），内嵌资源只是兜底。
  const bytes = (await fetchBeatmapSample(name)) ?? hitsoundAsset(name);
  if (!bytes.length) continue;
  const buffer = await decodeOgg(bytes);               // 宿主只用 Web Audio 解码
  session.setHitsoundSample(name, channels, buffer.sampleRate, loopFrames, pcm);
}
session.rebuildHitsoundTimeline();                     // 全部放完后重建一次
session.setPlaying(true);                              // 由宿主驱动
session.positionHitsound(chartTimeMs);                 // 每帧对齐位置
const frames = session.renderHitsound(4096);           // 取一段混音结果
const pcm = session.takeHitsoundBuffer();              // 交错立体声 Float32Array
```

`hitsoundNames(bytes)`：按 `.osu` 字节返回打击音需要的样本名，可在会话创建前调用。

`hitsoundAsset(name)`：按样本名返回内嵌的 ogg 字节（空数组表示没有对应资源）。

`hitsoundAssetCount()`：内嵌资源数量（36），便于宿主自检。

`hitsoundDefaults(mode)`：返回该模式在 `assets/shared_config.yml` 里的打击音默认值
（`{ enabled, volume, beatmapEnabled }`；`beatmapEnabled` 对应 `ENABLE_BEATMAP_HITSOUND`，
表示是否使用谱面自带的自定义音效）。CLI 读同一份配置，网页端用它保证默认值一致。

`beatmapInfo(bytes)`：按传入的 `.osu` 字节返回谱面内部信息对象（全量字段）。它不下载文件，也不依赖 WebGPU，可以在创建会话之前调用：

```js
import init, { beatmapInfo, WebGpuSession } from "./pkg/osu_beatmap_preview_wasm.js";

await init();
const bytes = new Uint8Array(await (await fetch("/resource/beatmap?bid=738063")).arrayBuffer());

const info = beatmapInfo(bytes);
info.title;    // 'No title'
info.version;  // "Lust's Insane"（难度名）
info.modeName; // 'standard'
info.bpm;      // 200
info.ar;       // 9.3
info.metadata; // [Metadata] 全量键值
```

| 分组 | 字段 |
| --- | --- |
| 概览 | `title`、`titleUnicode`、`artist`、`artistUnicode`、`creator`、`version`、`source`、`tags`、`beatmapId`、`beatmapSetId` |
| 格式与 `[General]` | `mode`、`modeName`、`formatVersion`、`audioFilename`、`audioLeadInMs`、`stackLeniency`、`backgroundFilename`、`beatDivisor` |
| 统计 | `hitObjectCount`、`firstObjectMs`、`lastObjectEndMs`、`chartDurationMs`、`bpm`、`timingPointCount`、`breakPeriodCount`、`comboColors` |
| 难度 | `ar`、`cs`、`hp`、`od` |
| 全量区段 | `general`、`metadata`、`difficulty`（`.osu` 里对应区段的每个键值） |

缺失的字段是 `null`（不是空字符串），`chartDurationMs` 等派生值在谱面没有音符时同样为 `null`。

`options` 的字段都是可选的：`convert`（`mania`/`ctb`/`taiko`/`standard`/`std`）、`mods`（字符串数组）、`width`、`height`（输出尺寸）。Mod 语法与 CLI 一致，见 [CLI 的 Mod 支持](../osu-beatmap-preview-cli/README.md#mod-支持)。

## 使用限制

- 只支持 WebGPU 后端；浏览器或设备不支持时 `create` 直接返回错误，不会回退到 WebGL 或 CPU 绘制。
- 本 crate 只在 `wasm32` 目标下导出上述类型；为其他目标编译时是空库，请在宿主页面使用 wasm 产物。
- 不解析回放、不切分 MP4；倍速与 seek 由宿主把「当前游戏时间」告诉 WASM（`positionHitsound` / `seekHitsound`），音频事件时间轴与混音都在 WASM 内。
- 打击音资源内嵌在 wasm 里（36 个 ogg，约 240 KiB，压缩后 wasm 约 1.2 MiB），宿主只需用 Web Audio 解码；还需要 `SharedArrayBuffer`（跨源隔离）才能把混音结果交给音频线程，环境不具备时页面应退化成「只播音乐」，而不是报错。
- 每帧都直接在 GPU 上绘制，宿主应按目标帧率调用 `render_number`，不要在同一帧重复提交。
- 背景图需要宿主自行解码成 RGBA 后通过 `set_background_rgba` 传入。
- `beatmapInfo` / `hitsoundNames` 只解析传入的字节，不认识 `bid`：`.osu` 的下载由宿主负责（Web 包里是 Node 后端的 `/resource/beatmap?bid=`）。

## 相关文档

- [Web 站点说明](../osu-beatmap-preview-web/README.md)：发布包里静态站点与 Node.js 下载后端的用法，本地调试也从这里启动。
- [CLI 使用说明](../osu-beatmap-preview-cli/README.md)：导出 PNG/GIF/MP4 的完整参数说明。
- [架构说明](../../docs/architecture.md)：core、renderer、wasm 之间的 API 边界与场景合成细节。

# WASM 使用说明（osu-beatmap-preview-wasm）

[返回项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [架构说明](../../docs/architecture.md)

WASM 产物把 core 的实时会话和 renderer 的 WGPU 绘制接到浏览器，让宿主网页在本地 GPU 上逐帧绘制谱面。它的输入只有**一份文件**——`.osu` 字节（只有谱面与内嵌音效）或 `.osz` 整包字节（音乐、背景、自带音效都在包里）；解包、解码、混音与时钟全部在 WASM 内完成。它不下载文件、不返回像素数据，也不通过 HTTP 轮询渲染结果。

## 职责边界

| WASM 负责 | 宿主（JavaScript）负责 |
| --- | --- |
| `.osz` 解包（zip crate）与难度选择 | 获取一份文件的字节（本地文件或后端下载） |
| 音乐 / 背景 / 自带音效解码（symphonia、image） | 提供 `<canvas>` 与控制 UI |
| 解析 `.osu`、Mod、转谱和时间轴 | 建立 `AudioContext` + AudioWorklet 音频输出 |
| **时钟**：播放、暂停、seek、倍速、当前位置 | 把 `pullAudio` 的混音结果写进音频环形缓冲 |
| 音乐与打击音的统一混音（PCM） | 把音频线程的消费位置回报给 `onAudioClock` |
| 按内部时钟生成并绘制单帧（`renderFrame`） | 处理按键、指针等输入事件 |
| 会话内的 Mod 热切换与画布尺寸变更 | |
| 各模式可选的 Mod 清单（`supportedMods`） | 渲染 Mod 按钮、收集勾选项 |
| 谱面信息与 `.osz` 难度清单（`beatmapInfo`） | 展示信息、渲染难度下拉 |

WASM 不返回 RGBA 缓冲，因此宿主拿不到像素结果；需要图片或视频文件时请使用 [CLI](../osu-beatmap-preview-cli/README.md)。在官方 [Web 包](../osu-beatmap-preview-web/README.md) 里，表右侧的下载职责由 Node.js 后端完成，音频输出与控制 UI 由页面脚本完成。

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

用一个静态服务器托管 `pkg/` 与下面的页面，浏览器需要支持 WebGPU（Chrome / Edge 113+ 等）。文件字节由宿主自己获取；只想快速跑通时直接用 [Web 包](../osu-beatmap-preview-web/README.md)，它已经带好了静态站点和下载后端。

```html
<canvas id="stage"></canvas>
<script type="module">
  import init, { WebGpuSession } from "./pkg/osu_beatmap_preview_wasm.js";

  await init();
  const canvas = document.getElementById("stage");
  // 一份文件的字节：.osu 或 .osz 都行（.osz 里的音乐/背景/音效由 WASM 解出）。
  const bytes = new Uint8Array(await (await fetch("./738063.osz")).arrayBuffer());

  // 第 3 个参数是可选选项：difficulty/bid、convert、mods、width、height、sampleRate……
  const session = await WebGpuSession.create(bytes, canvas, {
    width: 1280,
    height: 720,
    sampleRate: audioContext.sampleRate,
  });

  session.play();
  function frame() {
    session.renderFrame();          // 按 WASM 内部时钟绘制当前帧
    const pcm = session.pullAudio(4096); // 音乐 + 打击音统一混音，写进音频环形缓冲
    writeRingBuffer(pcm);
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
</script>
```

## JavaScript API

导出两个类型与两个函数，所有时间都用毫秒（谱面绝对时间，0 = 音频文件 0 点）。

### `WebGpuSession`

| 成员 | 说明 |
| --- | --- |
| `WebGpuSession.create(bytes, canvas, options?)` | 异步创建会话。`bytes` 为 `.osu` 或 `.osz` 的文件字节，`canvas` 为目标 `HTMLCanvasElement`（宽高设为输出尺寸）。`.osz` 的解包、音乐/背景/音效的解码都在这一步完成 |
| `play()` / `pause()` | 开始 / 暂停推进时钟；幂等 |
| `seek(chartTimeMs)` | 跳到谱面绝对时间：换时钟锚、丢弃正在播放的声音、重置输出流（见 `audioEpoch()`） |
| `setRate(userRate)` | 设置用户倍速（不含 DT/HT）；总倍速 = 用户倍速 × 谱面变速，由会话换算 |
| `rate()` | 当前总倍速，即音频线程的消费速率 |
| `clockMs()` | 当前谱面绝对时间；画面按它渲染，进度条的「已播放时长」= `clockMs() - absoluteStartMs()` |
| `playing()` | 时钟是否在推进 |
| `renderFrame()` | 按内部时钟把当前帧绘制到 Canvas |
| `pullAudio(maxFrames)` | 补一段「音乐 + 打击音」统一混音，返回交错立体声 `Float32Array`（帧数 × 2，可能为空）；补多少由 WASM 的预读窗口决定 |
| `onAudioClock(consumedFrames)` | 转发音频线程回报的消费位置（相对帧号）：有音频输出时它是画面时钟的锚点 |
| `audioEpoch()` | 输出流的重置纪元；变化表示整条流已重置，宿主必须把环形缓冲读写指针一起归零并通知音频线程从头重读 |
| `setMusicVolume(percent)` / `setHitsoundVolume(percent)` | 音乐 / 打击音音量（0–100），即时生效 |
| `setHitsoundEnabled(bool)` | 打开/关闭打击音；关闭时音乐照常输出 |
| `set_mods(mods)` | 热切换 Mod，数组每项是一个独立 token；转谱后所需样本由 WASM 重新装载，`absoluteStartMs()` 可能变化，需要重新读取 |
| `resize(width, height)` | 同时更新 surface 与 core 的合成尺寸；只改 Canvas 不会改变渲染尺寸 |
| `durationMs()` | 预览时长（含最后一个物件后的 2s 余韵），用于进度条和结束判定 |
| `absoluteStartMs()` | 预览起点（进度条 `0:00`）对应的谱面绝对时间；与游戏时间轴（首个可玩物件为 `0:00`）不同 |
| `beatmapSpeed()` | 当前谱面倍速（DT/HT 等） |
| `audioSampleRate()` | 混音输出采样率（创建时给定的 `sampleRate`） |
| `width()` / `height()` | 当前输出尺寸 |
| `mode()` | 解析和转谱后的目标模式，小写字符串，如 `standard`、`mania` |

`options` 的字段都是可选的：

| 字段 | 说明 |
| --- | --- |
| `difficulty` | `.osz` 里要预览的难度条目名（如 `Hard.osu`）；优先级最高 |
| `bid` | 按 `[Metadata] BeatmapID` 在 `.osz` 里选难度（数字或数字字符串）；都没有时取第一个顶层 `.osu` |
| `convert` | 转谱模式（`mania`/`ctb`/`taiko`/`standard`/`std`） |
| `mods` | Mod token 数组，语法与 CLI 一致，见 [CLI 的 Mod 支持](../osu-beatmap-preview-cli/README.md#mod-支持) |
| `width` / `height` | 输出尺寸 |
| `sampleRate` | **音频设备采样率**（`AudioContext.sampleRate`）：混音输出按它生成，必须一致 |
| `hitsoundEnabled` / `hitsoundVolume` / `musicVolume` | 打击音开关（默认 true）、打击音音量（默认 100）、音乐音量（默认 50） |
| `beatmapHitsound` | 是否采用谱面自带的自定义音效（`ENABLE_BEATMAP_HITSOUND`，默认 true）；**创建时生效**，决定装载哪些样本 |

### 时钟与时间轴

- **时钟在 WASM 里**：`play` / `pause` / `seek` / `setRate` / `clockMs` 全部由会话维护，宿主不再计算时间，也没有「静音 WAV + `<audio>` 元素」这类假时钟。
- **音频文件的 0 点就是谱面时间轴的 0 点**：宿主不需要、也不应该再叠加 `AudioLeadIn`——它只决定「从多早开始播放」（体现在 `absoluteStartMs()` 里）。
- `absoluteStartMs()` 是预览起点：首个物件前 2000ms，谱面 `AudioLeadIn` 更大时按它提前；首物件很早时该值会是负数。绝对时间 `< 0` 的前置段音乐还没开始（混音位置允许为负，输出静音），画面与音效照常。
- **有音频输出时画面贴着「此刻听到的位置」走**：音频线程每约 11ms 报告一次消费位置（`onAudioClock`），时钟向它平滑锚定；没有音频输出（或被自动播放策略拦下）时时钟按墙钟推进，画面不冻结。音频恢复后输出流自动重置到画面位置继续。
- 倍速**音乐与音效一起变速变调**（与 CLI MP4 导出一致）。

### 音频输出（音乐 + 打击音统一混音）

WASM 把**音乐与打击音混成一条 PCM 流**，宿主只做搬运：

```js
const session = await WebGpuSession.create(bytes, canvas, {
  sampleRate: audioContext.sampleRate,   // 设备采样率，必须一致
});

let written = 0;                 // 环形写入位置（相对帧号）
let epoch = session.audioEpoch();
session.play();
audioOutput.setPlaying(true);

function pump() {
  const pcm = session.pullAudio(4096);          // 交错立体声 Float32Array
  if (session.audioEpoch() !== epoch) {          // 输出流整体重置（seek / 走散重对齐）
    epoch = session.audioEpoch();
    written = 0;
    workletNode.port.postMessage({ command: 'readFrame', value: 0 });
  }
  writeRingBuffer(pcm, written);                 // 接在环形写入前沿之后
  written += pcm.length / 2;
  Atomics.store(control, 0, written);
}
// 音频线程的回报（readPosition）也要转发回来：
workletNode.port.onmessage = (event) => session.onAudioClock(event.data.readPosition);
```

音乐与音效的装载完全不需要宿主参与：`.osz` 里的音乐、背景、自带音效在 `create` 时就解好了，自带音效按「同名条目 > 内嵌皮肤」取用；单独的 `.osu` 没有音乐与背景，内嵌皮肤照常发声。

### 自由函数

`beatmapInfo(bytes, options?)`：按传入的文件字节（`.osu` 或 `.osz`）返回谱面内部信息对象（全量字段）与难度清单。它不下载文件，也不依赖 WebGPU，可以在创建会话之前调用：

```js
import init, { beatmapInfo, WebGpuSession } from "./pkg/osu_beatmap_preview_wasm.js";

await init();
const bytes = new Uint8Array(await (await fetch("/resource/file?bid=738063")).arrayBuffer());

const info = beatmapInfo(bytes);  // options 与 create 相同（bid / difficulty 选难度）
info.title;         // 'No title'
info.version;       // "Lust's Insane"（难度名）
info.modeName;      // 'standard'
info.bpm;           // 200
info.difficulties;  // .osz 的难度清单 [{ entry, label, beatmapId }]；单文件 .osu 为空数组
```

| 分组 | 字段 |
| --- | --- |
| 概览 | `title`、`titleUnicode`、`artist`、`artistUnicode`、`creator`、`version`、`source`、`tags`、`beatmapId`、`beatmapSetId` |
| 格式与 `[General]` | `mode`、`modeName`、`formatVersion`、`audioFilename`、`audioLeadInMs`、`stackLeniency`、`backgroundFilename`、`beatDivisor` |
| 统计 | `hitObjectCount`、`firstObjectMs`、`lastObjectEndMs`、`chartDurationMs`、`bpm`、`timingPointCount`、`breakPeriodCount`、`comboColors` |
| 难度 | `ar`、`cs`、`hp`、`od` |
| 全量区段 | `general`、`metadata`、`difficulty`（`.osu` 里对应区段的每个键值） |
| 难度清单 | `difficulties`（`.osz` 里每个顶层 `.osu` 的 `{ entry, label, beatmapId }`；单独的 `.osu` 为空数组） |

缺失的字段是 `null`（不是空字符串），`chartDurationMs` 等派生值在谱面没有音符时同样为 `null`。

`hitsoundDefaults(mode)`：返回该模式在 `assets/shared_config.yml` 里的打击音默认值
（`{ enabled, volume, beatmapEnabled }`；`beatmapEnabled` 对应 `ENABLE_BEATMAP_HITSOUND`，
表示是否使用谱面自带的自定义音效）。CLI 读同一份配置，网页端用它保证默认值一致；
它应在 `create` 之前调用（`beatmapHitsound` 决定装载哪些样本）。

`supportedMods(mode)`：返回该模式在实时预览里可选的 Mod token 数组，顺序即界面展示顺序。
列表来自 core 的支持矩阵（与 GIF/MP4 校验同一套规则），网页端直接拿它渲染 Mod 按钮，
不必自己维护一份支持表——两边各写一份很容易在新增 Mod 后走偏。`mode` 接受
`standard` / `taiko` / `catch` / `mania`（也接受 `std` 与 `ctb`），未知模式抛错：

```js
const mods = supportedMods('mania');
// ['HD','FL','CS','DT','HT','1K','2K','3K','4K','5K','6K','7K','8K','9K','10K','DS','IN','HO']
// HD/FL 在四种模式下都可用；DA 需要补参数后提交（如 DAAR9CS4）。
session.set_mods(mods.filter((token) => token === 'HD' || token === 'FL'));
```

## 使用限制

- 只支持 WebGPU 后端；浏览器或设备不支持时 `create` 直接返回错误，不会回退到 WebGL 或 CPU 绘制。
- 本 crate 的 wasm 导出只在 `wasm32` 目标下生成；解包/解码模块（`archive.rs`、`decode.rs`）不依赖 wasm 运行时，`cargo test` 可直接在宿主上跑它们的用例。
- 不解析回放、不切分 MP4；时钟由 WASM 维护，宿主只在音频线程回报时把消费位置转发回来（`onAudioClock`）。
- 音乐、音效与背景的解码（symphonia / image）在 `create` 时一次性完成：大谱面包（长图、大音乐）会多花一些内存与几百毫秒加载时间。打击音皮肤内嵌在 wasm 里（36 个 ogg，约 240 KiB）；解包/解码依赖（zip / symphonia / image）加上皮肤后 wasm 约 2.2 MiB（gzip 后约 0.9 MiB）。
- 音频输出还需要 `SharedArrayBuffer`（跨源隔离）与 `AudioWorklet`，环境不具备时页面应退化成「只有画面」，而不是报错。
- 每帧都直接在 GPU 上绘制，宿主应按目标帧率调用 `renderFrame`，不要在同一帧重复提交。
- `beatmapInfo` / `create` 只解析传入的字节，不认识 `bid` 的下载：文件获取由宿主负责（Web 包里是 Node 后端的 `/resource/file?bid=`）。

## 相关文档

- [Web 站点说明](../osu-beatmap-preview-web/README.md)：发布包里静态站点与 Node.js 下载后端的用法，本地调试也从这里启动。
- [CLI 使用说明](../osu-beatmap-preview-cli/README.md)：导出 PNG/GIF/MP4 的完整参数说明。
- [架构说明](../../docs/architecture.md)：core、renderer、wasm 之间的 API 边界与场景合成细节。

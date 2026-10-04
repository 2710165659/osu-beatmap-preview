# WASM 使用说明（osu-beatmap-preview-wasm）

[返回项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [架构说明](../../docs/architecture.md)

输入**一份文件的字节**——`.osu`（谱面与内嵌音效）或 `.osz` 整包（含音乐、背景、自带音效）——即可在浏览器里用 WebGPU 实时渲染并播放谱面。解包、解码、混音、时钟都在 WASM 内完成。WASM 不下载文件、也不返回像素；导出 PNG/GIF/MP4 请用 [CLI](../osu-beatmap-preview-cli/README.md)。

| WASM 负责 | 宿主负责 |
| --- | --- |
| `.osz` 解包、难度选择、音乐/背景/打击音解码 | 获取文件字节（本地文件或后端下载） |
| 解析、Mod、转谱、时间轴、逐帧绘制到 Canvas | 提供 `<canvas>`（宽高设为输出尺寸） |
| 时钟（播放/暂停/seek/倍速）与「音乐 + 打击音」混音 | 音频输出：消费 `pullAudio` 的 PCM 并回报 `onAudioClock` |
| Mod 热切换、背景视频与故事板合成 | 控制 UI（播放按钮、进度条、难度下拉等） |

## 获取与构建

产物没有独立发布包，位于 [Web 包](../osu-beatmap-preview-web/README.md) 的 `public/pkg/`：

| 文件 | 说明 |
| --- | --- |
| `osu_beatmap_preview_wasm.js` | ES 模块胶水代码（`wasm-bindgen --target web`） |
| `osu_beatmap_preview_wasm_bg.wasm` | WebAssembly 二进制 |
| `*.d.ts` | TypeScript 类型声明 |

自己构建需要稳定版 Rust、`wasm32-unknown-unknown` 目标，以及与 `Cargo.lock` 版本一致的 `wasm-bindgen-cli`：

```bash
cd crates/osu-beatmap-preview-web
npm run build:wasm   # 构建 wasm 并生成 public/pkg
```

等价的手工步骤：

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked   # 版本与 Cargo.lock 一致
cargo build --release -p osu-beatmap-preview-wasm --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/osu_beatmap_preview_wasm.wasm \
  --target web --out-dir pkg
```

## 最小示例

用静态服务器托管 `pkg/` 与页面，浏览器需要 WebGPU（Chrome / Edge 113+）。文件字节由宿主自行获取；想直接跑通就用 [Web 包](../osu-beatmap-preview-web/README.md)，站点和下载后端都是现成的。

```html
<canvas id="stage"></canvas>
<script type="module">
  import init, { WebGpuSession } from "./pkg/osu_beatmap_preview_wasm.js";

  await init();
  const bytes = new Uint8Array(await (await fetch("./738063.osz")).arrayBuffer());
  const session = await WebGpuSession.create(bytes, document.getElementById("stage"), {
    width: 1280,
    height: 720,
    sampleRate: audioContext.sampleRate,   // 必须是音频设备的实际采样率
  });

  session.play();
  function frame() {
    session.renderFrame();                  // 按 WASM 内部时钟绘制当前帧
    const pcm = session.pullAudio(4096);    // 音乐 + 打击音混音（交错立体声）
    writeRingBuffer(pcm);                   // 写进音频环形缓冲，见下节
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
</script>
```

## 音频输出与时钟

时钟在 WASM 里维护，宿主只搬运 PCM 并把消费位置回报回来：

```js
let written = 0;                      // 环形写入位置（相对帧号）
let epoch = session.audioEpoch();
session.play();

function pump() {
  const pcm = session.pullAudio(4096);
  if (session.audioEpoch() !== epoch) {  // seek / 重对齐后整条输出流重置
    epoch = session.audioEpoch();
    written = 0;
    workletNode.port.postMessage({ command: 'readFrame', value: 0 });
  }
  writeRingBuffer(pcm, written);
  written += pcm.length / 2;
}
// 音频线程的消费位置回报给 WASM，作为画面时钟的锚点：
workletNode.port.onmessage = (e) => session.onAudioClock(e.data.readPosition);
```

- 时间轴 0 点 = 音频文件 0 点，**不要再叠加 `AudioLeadIn`**：它只决定预览起点，体现在 `absoluteStartMs()`。
- 有音频输出时画面贴着「此刻听到的位置」走（约每 11ms 由 `onAudioClock` 锚定）；没有音频时按墙钟推进，画面不冻结，音频恢复后自动重对齐。
- `audioEpoch()` 变化表示输出流整体重置：环形缓冲的读写指针必须一起归零，让音频线程从头重读。
- 倍速下音乐与打击音一起变速变调（与 CLI 的 MP4 导出一致）。
- 音乐、背景、自带音效在 `create` 时解好，不需要宿主参与；自带音效按「同名条目 > 内嵌皮肤」取用，单独的 `.osu` 没有音乐与背景。

## API 参考

导出 `WebGpuSession`、`beatmapInfo`、`supportedMods`、`hitsoundDefaults` 与默认初始化函数 `init`。所有时间均为毫秒的谱面绝对时间（0 = 音频文件 0 点）。

### `WebGpuSession.create(bytes, canvas, options?)`

异步创建会话，`.osz` 解包与全部解码都在这一步完成。难度选择优先级：`difficulty`（压缩包条目名）> `bid`（`BeatmapID`）> 第一个顶层 `.osu`。`options` 字段全部可选：

| 字段 | 说明 |
| --- | --- |
| `difficulty` / `bid` | 选择 `.osz` 里的难度（见上） |
| `convert` | 转谱目标：`mania` / `ctb` / `taiko` / `standard` |
| `mods` | Mod token 数组，语法同 [CLI 的 Mod 支持](../osu-beatmap-preview-cli/README.md#mod-支持) |
| `width` / `height` | 输出尺寸 |
| `sampleRate` | 音频设备采样率（`AudioContext.sampleRate`），必须一致 |
| `musicVolume` / `hitsoundVolume` / `hitsoundEnabled` | 音乐音量（默认 50）、打击音音量（默认 100）、打击音开关（默认 true） |
| `beatmapHitsound` | 优先使用谱面自带音效（默认 true）；**创建时生效**，决定装载哪些样本 |
| `storyboard` | 绘制故事板（默认 false），可随时 `setStoryboard` 切换 |

### 播放与画面

| 成员 | 说明 |
| --- | --- |
| `play()` / `pause()` / `playing()` | 推进 / 暂停时钟（幂等）/ 查询是否在推进 |
| `seek(chartTimeMs)` | 跳到绝对时间：丢弃在播声音并重置输出流（`audioEpoch()` 变化） |
| `setRate(userRate)` | 用户倍速（不含 DT/HT）；同游戏 `UserPlaybackRate`，音乐与打击音一起变调 |
| `rate()` | 当前总倍速 = 用户倍速 × 谱面变速 |
| `clockMs()` | 当前绝对时间；进度条已播放时长 = `clockMs() - absoluteStartMs()` |
| `renderFrame()` | 把当前帧绘制到 Canvas；返回 `false` 表示本帧没画（画布不可见或 GPU 取不到帧），连续 1 秒拿不到帧才抛错 |
| `resize(width, height)` | 改输出尺寸（surface 与合成尺寸一起改；只改 Canvas 不生效） |
| `width()` / `height()` / `mode()` | 输出尺寸 / 目标模式（`standard`、`mania` 等） |
| `durationMs()` | 预览时长（最后一个物件后 +2s），用于进度条与结束判定 |
| `absoluteStartMs()` | 预览起点（进度条 `0:00`）的绝对时间：首个物件前 2s，`AudioLeadIn` 更大时提前，可能为负 |

### 音频

| 成员 | 说明 |
| --- | --- |
| `pullAudio(maxFrames)` | 补一段「音乐 + 打击音」混音，返回交错立体声 `Float32Array`（帧数 × 2，可能为空） |
| `onAudioClock(consumedFrames)` | 转发音频线程的消费位置（相对帧号）；有音频输出时它是画面时钟的锚点 |
| `audioEpoch()` | 输出流纪元；变化表示整条流已重置 |
| `setMusicVolume(pct)` / `setHitsoundVolume(pct)` | 音量 0–100，即时生效 |
| `setHitsoundEnabled(bool)` | 开关打击音；关闭时音乐照常 |
| `audioSampleRate()` | 混音输出采样率（创建时给定的 `sampleRate`） |

### Mod、背景与故事板

| 成员 | 说明 |
| --- | --- |
| `set_mods(mods)` | 热切换 Mod（token 数组）；转谱后重新装载样本，`absoluteStartMs()` 可能变化 |
| `setBackgroundDim(pct)` | 背景暗化 0–100（默认 70），背景图 / 背景视频 / 故事板共用 |
| `hasBackgroundVideo()` / `setBackgroundVideo(bool)` | 谱面是否有可用背景视频 / 开关（默认关闭） |
| `hasStoryboard()` / `setStoryboard(bool)` | 是否有可绘制的故事板 / 开关（默认关闭）；层序同 osu!：underlay 在物件之下，Overlay 压在物件之上 |
| `setBeatmapHitsound(bool)` | 切换「谱面打击音」：是否优先使用谱面自带样本；切换后重新装载样本并重建时间轴 |

### `beatmapInfo(bytes, options?)`

返回谱面信息与 `.osz` 难度清单；不依赖 WebGPU，可在 `create` 之前调用。难度选择同 `create`（`difficulty` / `bid`）：

```js
import init, { beatmapInfo } from "./pkg/osu_beatmap_preview_wasm.js";
await init();
const info = beatmapInfo(bytes);
info.title;        // 'No title'
info.version;      // "Lust's Insane"（难度名）
info.bpm;          // 200
info.difficulties; // .osz 难度清单 [{ entry, label, beatmapId }]；单文件 .osu 为 []
```

| 分组 | 字段 |
| --- | --- |
| 概览 | `title`、`titleUnicode`、`artist`、`artistUnicode`、`creator`、`version`、`source`、`tags`、`beatmapId`、`beatmapSetId` |
| 格式与 `[General]` | `mode`、`modeName`、`formatVersion`、`audioFilename`、`audioLeadInMs`、`stackLeniency`、`backgroundFilename`、`beatDivisor` |
| 统计 | `hitObjectCount`、`firstObjectMs`、`lastObjectEndMs`、`chartDurationMs`、`bpm`、`timingPointCount`、`breakPeriodCount`、`comboColors` |
| 难度 | `ar`、`cs`、`hp`、`od` |
| 全量区段 | `general`、`metadata`、`difficulty`（`.osu` 对应区段的每个键值） |
| 难度清单 | `difficulties`（每个顶层 `.osu` 的 `{ entry, label, beatmapId }`） |

缺失字段为 `null`（不是空字符串）；`chartDurationMs` 等派生值在没有音符时同为 `null`。

### `supportedMods(mode)` / `hitsoundDefaults(mode)`

- `supportedMods(mode)`：该模式实时预览可选的 Mod token 数组，顺序即界面展示顺序（与 GIF/MP4 共用 core 的支持矩阵）。`mode` 取 `standard` / `taiko` / `catch` / `mania`（也接受 `std` / `ctb`），未知模式抛错：

  ```js
  const mods = supportedMods('mania');
  // ['HD','FL','CS','DT','HT','NC','DC','1K',…,'10K','DS','IN','HO']
  session.set_mods(mods.filter((token) => token === 'HD' || token === 'FL'));
  ```

- `hitsoundDefaults(mode)`：该模式的打击音默认值 `{ enabled, volume, beatmapEnabled }`（与 CLI 读同一份配置，保证默认值一致）。应在 `create` 之前调用（`beatmapHitsound` 决定装载哪些样本）。

## 使用限制

- 只支持 WebGPU 后端：浏览器或设备不支持时 `create` 直接报错，不回退 WebGL 或 CPU 绘制。
- 不解析回放、不导出视频；文件获取由宿主负责（Web 包里是后端的 `/resource/file?bid=`）。
- 解码在 `create` 一次性完成，大谱面包会多占内存、多花几百毫秒加载；wasm 本体约 2.2 MiB（gzip 后约 0.9 MiB），内嵌打击音约 280 KiB。
- 音频输出还需要 `SharedArrayBuffer`（跨源隔离）与 `AudioWorklet`；环境不具备时页面应退化成「只有画面」，而不是报错。
- 每帧调用一次 `renderFrame` 即可，不要在同一帧重复提交；返回 `false` 不是错误，时钟与音频照常推进。

## 相关文档

- [Web 站点说明](../osu-beatmap-preview-web/README.md)：静态站点与 Node.js 下载后端，本地调试从这里启动。
- [CLI 使用说明](../osu-beatmap-preview-cli/README.md)：导出 PNG/GIF/MP4 的完整参数说明。
- [架构说明](../../docs/architecture.md)：core、renderer、wasm 之间的 API 边界与场景合成细节。

# WASM 使用说明

[项目首页](../../README.md) | [Web 站点说明](../osu-beatmap-preview-web/README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md)

`osu-beatmap-preview-wasm` 将谱面与回放接入浏览器 WebGPU Canvas，负责解包、媒体解码、判定、绘制、混音与播放时钟。宿主负责获取文件、提供 Canvas、输出 PCM 和实现界面；PNG/GIF/MP4 导出使用 CLI。

## 构建

发布的 [Web 包](../osu-beatmap-preview-web/README.md) 在 `dist/pkg/` 中包含 JS、WASM 与类型声明；源码构建先生成到 `public/pkg/`，再由前端构建复制到 `dist/pkg/`。自行构建需要稳定版 Rust、`wasm32-unknown-unknown` 目标，以及与 `Cargo.lock` 一致的 `wasm-bindgen-cli`：

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
cd crates/osu-beatmap-preview-web
npm run build:wasm
```

安装前应核对当前 `Cargo.lock` 的版本。脚本在仓库根目录执行 Rust 构建，再以 `wasm-bindgen --target web` 生成以下产物：

| 文件 | 用途 |
| --- | --- |
| `osu_beatmap_preview_wasm.js` | ES 模块入口 |
| `osu_beatmap_preview_wasm_bg.wasm` | WebAssembly 二进制 |
| `*.d.ts` | TypeScript 类型声明 |

Web 页面构建必须在 WASM 生成后执行，才能将 `public/pkg/` 复制到 `dist/`。

## 最小画面示例

通过静态服务器托管页面、`pkg/` 和谱面包。浏览器需要支持 WebGPU，并处于 HTTPS 或 localhost 安全上下文。以下示例仅播放画面；音频输出见下一节。

```html
<canvas id="stage"></canvas>
<script type="module">
  import init, { WebGpuSession } from "./pkg/osu_beatmap_preview_wasm.js";

  await init();
  const response = await fetch("./map.osz");
  if (!response.ok) throw new Error("谱面包加载失败");
  const bytes = new Uint8Array(await response.arrayBuffer());
  const session = await WebGpuSession.create(bytes, document.getElementById("stage"), {
    width: 1280,
    height: 720,
  });

  session.play();
  function frame() {
    session.renderFrame();
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
</script>
```

会话使用 WASM 内部时钟推进；`renderFrame()` 返回 `false` 时继续调度即可。销毁页面或替换会话时应停止动画循环并调用 wasm-bindgen 生成的 `session.free()`。

## 输入与创建选项

`await WebGpuSession.create(bytes, canvas, options?)` 创建会话。输入必须为 `Uint8Array`，支持 `.osu` 或 `.osz`；单独 `.osu` 没有音乐与背景。回放使用匹配的完整 OSZ 作为主输入，并在 `replayBytes` 中附带 OSR。

| 选项 | 说明 |
| --- | --- |
| `checksum` | 按原始 OSU 字节 MD5 选择难度，优先于下列选择项 |
| `difficulty` | 包内顶层 OSU 条目名 |
| `bid` | 按 `BeatmapID` 选择，优先级低于 `difficulty`；无选择项时取首个顶层 OSU |
| `osuBytes` | 外部 OSU 字节，主输入须为 OSZ；普通预览按有效 BeatmapID 绑定包内难度 |
| `replayBytes` | OSR 字节，按回放 MD5 选谱，覆盖普通难度选择；拒绝转谱与用户 Mod |
| `convert` | `standard`、`taiko`、`ctb`、`mania` 等目标模式 |
| `mods` | Mod token 数组，语法与 [CLI Mod](../osu-beatmap-preview-cli/README.md#mod-支持) 相同，支持实时预览列 |
| `width`、`height` | 输出像素尺寸 |
| `sampleRate` | 音频输出采样率，应传 `AudioContext.sampleRate` |
| `musicVolume` | 音乐音量百分比，默认 50 |
| `hitsoundEnabled`、`hitsoundVolume` | 打击音开关与音量，默认 true、100 |
| `beatmapHitsound` | 优先使用谱面自带音效，默认 true，影响创建时的样本装载 |
| `storyboard` | 初始故事板开关，默认 false |

WASM 会话的背景视频和故事板默认关闭，Web 页面自行启用。音乐、背景和样本在创建阶段解码，背景视频按需解码；大包会增加初始化时间和内存占用。

## 音频接入

WASM 输出音乐与打击音的混音 PCM，宿主只负责搬运到音频设备，不能再用独立播放器推进第二条音轨。完整实现可参考 [AudioOutput](../osu-beatmap-preview-web/src/hitsound.js) 和 [AudioWorklet](../osu-beatmap-preview-web/public/hitsound-worklet.js)。

1. 用户交互后创建或恢复 `AudioContext`，以实际设备采样率创建会话。
2. 建立 AudioWorklet 与交错立体声环形缓冲，定期调用 `pullAudio(maxFrames)` 填充。
3. 将音频线程消费的相对采样帧号传给 `onAudioClock(consumedFrames)`，作为画面时钟锚点。
4. 检查 `audioEpoch()`；发生变化时清空旧缓冲，同时重置读写指针和相对帧号。
5. 暂停、跳转和倍速统一通过会话控制，设备输出跟随会话状态。

音频示例中的帧号是自上次输出流重置后的采样帧数，不是环形缓冲下标，也不是标量样本数；立体声 PCM 的帧数为 `pcm.length / 2`。seek 或重对齐会重置输出流，消费位置必须与新纪元一致。

所有时间以音频文件起点为 0，单位为毫秒。`absoluteStartMs()` 可为负值，它已包含预览起点与 `AudioLeadIn` 的处理，宿主无需额外叠加；进度条时间为 `clockMs() - absoluteStartMs()`。

有设备输出时画面跟随消费时钟，无音频输出时按墙钟推进。

使用现有音频实现需要 `SharedArrayBuffer`、AudioWorklet 与跨源隔离。静态服务器和反向代理应保留：

```http
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

这些音频条件不满足时，宿主可只播放画面。

## 回放

```js
import init, {
  WebGpuSession, replayInfo, beatmapInfo,
} from "./pkg/osu_beatmap_preview_wasm.js";

await init();
// osrBytes、oszBytes 和 canvas 由宿主提供，均须对应同一次回放。
const summary = replayInfo(osrBytes);
const info = beatmapInfo(oszBytes, { checksum: summary.checksum });
const session = await WebGpuSession.create(oszBytes, canvas, {
  replayBytes: osrBytes,
  width: 1280,
  height: 720,
  sampleRate: audioContext.sampleRate,
});
session.play();
const state = session.replayState();
```

此片段展示回放接入顺序，`osrBytes`、`oszBytes` 为 `Uint8Array`，`audioContext` 为宿主创建的音频上下文。`replayInfo()` 解析并检查回放兼容性，返回 `checksum` 与 `version`，不依赖 WebGPU。资源获取仍由宿主完成；WASM 不自动下载。

支持 stable、lazer、tachyon 的回放，按内容检查兼容性。Mod 支持范围见 [CLI 回放说明](../osu-beatmap-preview-cli/README.md#输出与兼容性)。回放要求包含音乐的完整 OSZ；若同时提供 `osuBytes`，外部和包内谱面原始 MD5 都必须匹配。用户不能通过 `mods`、`convert` 或 `set_mods()` 改写回放规则。

`replayState()` 返回当前时刻的只读判定快照，普通谱面会话返回 `null`：

| 字段 | 含义 |
| --- | --- |
| `score`、`accuracy` | 标准化分数与准确率 |
| `combo`、`max_combo` | 当前与最高连击 |
| `statistics` | 各判定计数 |
| `rank`、`hit_error_ms`、`health` | 等级、击打误差与生命值 |
| `status` | `Playing`、`Completed` 或 `Failed` |

可空字段无值时为 `null`。界面可将 `X/XH` 显示为 SS，失败显示 F。成绩使用当前官方规则重算，历史记录差异仅作诊断；查询不推进判定，seek 可查看任意播放时刻。

输入提前结束后成绩与光标冻结、按键清零、玩家音效停止；背景、谱面和音乐继续，seek 回失败前可恢复当时状态。回放字体与提示音已内嵌，见 [第三方声明](../../docs/THIRD_PARTY_NOTICES.md)。

## 会话 API

### 播放与画面

| 方法 | 说明 |
| --- | --- |
| `play()`、`pause()`、`playing()` | 播放、暂停、查询播放状态 |
| `seek(chartTimeMs)` | 跳到绝对谱面时间，丢弃旧声音并重置音频输出流 |
| `setRate(userRate)` | 设置有限正数的用户播放倍速，不含 Mod 倍速 |
| `rate()`、`beatmapSpeed()` | 当前总倍速、Mod 倍速 |
| `clockMs()`、`absoluteStartMs()` | 当前绝对时间、预览起点 |
| `durationMs()` | 预览时长，保留最后一个物件后的 2 秒 |
| `renderFrame()` | 按内部时钟绘制；返回 false 表示未取得可用帧，连续约 1 秒不可用才报错 |
| `resize(width, height)` | 同步调整 surface 和合成尺寸，仅修改 Canvas 不够 |
| `width()`、`height()`、`mode()` | 输出尺寸与目标模式 |
| `replayState()` | 回放快照，普通预览为 null |

### 音频

| 方法 | 说明 |
| --- | --- |
| `pullAudio(maxFrames)` | 返回交错立体声 `Float32Array`，可能为空 |
| `onAudioClock(consumedFrames)` | 回报设备消费的相对帧号 |
| `audioEpoch()` | 输出流纪元，变化时同步清空宿主缓冲 |
| `audioSampleRate()` | 会话输出采样率 |
| `setMusicVolume(pct)`、`setHitsoundVolume(pct)` | 0–100 音量，即时生效 |
| `setHitsoundEnabled(bool)` | 开关打击音 |
| `setBeatmapHitsound(bool)` | 切换谱面自带音效，重新装载样本与时间轴 |

### Mod 与背景

| 方法 | 说明 |
| --- | --- |
| `set_mods(mods)` | 普通预览热切换 Mod，重建相关资源；回放拒绝调用 |
| `setBackgroundDim(pct)` | 背景图、视频和故事板共用的暗化，0–100，默认 70 |
| `hasBackgroundVideo()`、`setBackgroundVideo(bool)` | 查询可用视频与切换播放 |
| `hasStoryboard()`、`setStoryboard(bool)` | 查询可绘制故事板与切换显示，Overlay 层可覆盖物件 |

## 信息查询 API

所有导出函数使用前均需 `await init()`。

### `beatmapInfo(bytes, options?)`

不依赖 WebGPU，返回谱面元数据与难度清单。选择项支持 `checksum`、`difficulty`、`bid`、`osuBytes`，与创建会话规则相同。

| 分组 | 字段 |
| --- | --- |
| 元数据 | `title`、`titleUnicode`、`artist`、`artistUnicode`、`creator`、`version`、`source`、`tags`、`beatmapId`、`beatmapSetId` |
| 格式与通用设置 | `mode`、`modeName`、`formatVersion`、`audioFilename`、`audioLeadInMs`、`stackLeniency`、`backgroundFilename`、`beatDivisor` |
| 统计 | `hitObjectCount`、`firstObjectMs`、`lastObjectEndMs`、`chartDurationMs`、`bpm`、`timingPointCount`、`breakPeriodCount`、`comboColors` |
| 难度 | `ar`、`cs`、`hp`、`od` |
| 全量区段 | `general`、`metadata`、`difficulty` |
| 难度清单 | `difficulties`，各项为 `{ entry, label, beatmapId }`；单独 OSU 为 `[]` |

缺失字段与无法计算的派生值为 `null`。

### `supportedMods(mode)` / `hitsoundDefaults(mode)`

`mode` 使用 `standard`、`taiko`、`catch`、`mania`，也接受 `std`、`ctb`。未知模式报错。

- `supportedMods(mode, level?)`：返回指定能力级别的 Mod token 数组，顺序可直接用于界面。`level` 可选 `png`、`animation`、`realtime`，默认 `realtime`；Web 普通谱面预览使用 `animation`，隐藏实时专属选项。
- `hitsoundDefaults(mode)`：返回 `{ enabled, volume, beatmapEnabled }`，与 CLI 共用默认配置。

## 使用边界

- 仅使用 WebGPU；设备或浏览器不可用时创建会话报错。
- 文件获取与在线成绩认证由宿主或后端完成，WASM 不持有下载凭据。
- 解码后的媒体驻留内存，宿主应限制输入大小并释放旧会话。
- 每个动画帧调用一次 `renderFrame()`，音频填充按缓冲余量调度。

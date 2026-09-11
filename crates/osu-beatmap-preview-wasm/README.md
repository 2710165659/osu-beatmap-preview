# WASM 使用说明（osu-beatmap-preview-wasm）

[返回项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [架构说明](../../docs/architecture.md)

WASM 产物把 core 的实时会话和 renderer 的 WGPU 绘制接到浏览器，让宿主网页在本地 GPU 上逐帧绘制谱面。它不下载文件、不返回像素数据，也不通过 HTTP 轮询渲染结果：`.osu` 字节、Canvas、音频和播放控制都由 JavaScript 宿主负责。

## 职责边界

| WASM 负责 | 宿主（JavaScript）负责 |
| --- | --- |
| 解析 `.osu` 字节、Mod、转谱和时间轴 | 获取 `.osu` / OSZ / 背景 / 音频并解码 |
| 按绝对时间生成单帧场景（`FrameScene`） | 维护播放时钟、暂停、seek、倍速 |
| 在浏览器 WebGPU Canvas 上绘制并呈现 | 提供 `<canvas>`、控制 UI 和音频播放 |
| 会话内的 Mod 热切换与画布尺寸变更 | 处理按键、指针等输入事件 |

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

`WebGpuSession` 是唯一的导出类型，所有时间都用毫秒。

| 成员 | 说明 |
| --- | --- |
| `WebGpuSession.create(bytes, canvas, options?)` | 异步创建会话。`bytes` 为 `.osu` 文件字节，`canvas` 为目标 `HTMLCanvasElement`，会话创建时会把 Canvas 宽高设为选项中的输出尺寸 |
| `render_number(absoluteTimeMs)` | 把该绝对时间的一帧绘制到 Canvas；时间必须为有限数字 |
| `duration_ms_number()` | 可玩时长，用于进度条和循环边界 |
| `absolute_start_ms_number()` | 游戏时间 `0:00` 对应的绝对时间；游戏时间到绝对时间即 `absoluteStart + gameTime` |
| `beatmap_speed_number()` | 当前谱面倍速（DT/HT 等），用于同步音频播放速率 |
| `width()` / `height()` | 当前输出尺寸 |
| `mode()` | 解析和转谱后的目标模式，小写字符串，如 `standard`、`mania` |
| `set_mods(mods)` | 热切换 Mod，数组每项是一个独立 token；切换后 `absolute_start_ms_number()` 可能变化，需要重新读取 |
| `set_background_rgba(width, height, rgba)` | 传入已解码的背景图 RGBA 数据；不调用时使用模式默认背景 |
| `resize(width, height)` | 同时更新 surface 与 core 的合成尺寸；只改 Canvas 不会改变渲染尺寸 |

`options` 的字段都是可选的：`convert`（`mania`/`ctb`/`taiko`/`standard`/`std`）、`mods`（字符串数组）、`width`、`height`（输出尺寸）。Mod 语法与 CLI 一致，见 [CLI 的 Mod 支持](../osu-beatmap-preview-cli/README.md#mod-支持)。

## 使用限制

- 只支持 WebGPU 后端；浏览器或设备不支持时 `create` 直接返回错误，不会回退到 WebGL 或 CPU 绘制。
- 本 crate 只在 `wasm32` 目标下导出上述类型；为其他目标编译时是空库，请在宿主页面使用 wasm 产物。
- 不解析回放、不切分 MP4，也不处理音频；倍速与 seek 需要宿主同步音频播放位置。
- 每帧都直接在 GPU 上绘制，宿主应按目标帧率调用 `render_number`，不要在同一帧重复提交。
- 背景图需要宿主自行解码成 RGBA 后通过 `set_background_rgba` 传入。

## 相关文档

- [Web 站点说明](../osu-beatmap-preview-web/README.md)：发布包里静态站点与 Node.js 下载后端的用法，本地调试也从这里启动。
- [CLI 使用说明](../osu-beatmap-preview-cli/README.md)：导出 PNG/GIF/MP4 的完整参数说明。
- [架构说明](../../docs/architecture.md)：core、renderer、wasm 之间的 API 边界与场景合成细节。

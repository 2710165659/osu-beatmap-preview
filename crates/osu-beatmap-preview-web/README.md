# Web 站点与下载后端（osu-beatmap-preview-web）

[返回项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [WASM 使用说明](../osu-beatmap-preview-wasm/README.md) | [架构说明](../../docs/architecture.md)

这是 Release 里的 **Web 包**：一个 Vue 静态站点加一个很薄的 Node.js 后端。

- **静态站点**：Vue 3 单文件组件 + Tailwind CSS v4，浏览器里跑 [osu-beatmap-preview-wasm](../osu-beatmap-preview-wasm/README.md)，谱面解析、Mod、转谱和每一帧绘制都在本地 WebGPU 中完成。
- **Node.js 后端**：浏览器因为跨域限制无法直接下载 osu! 的资源，后端负责下载并缓存 `.osu` 与 `.osz`，再把音频、背景图交给前端。后端不参与渲染，也不上传任何数据。

## 运行

运行只需要 Node.js 18 或更高版本，以及支持 WebGPU 的浏览器（Chrome / Edge 113+ 等）。发布包里已经带好 `dist/`，解压后直接启动：

```bash
node backend/server.js
# 然后访问 http://127.0.0.1:8787
```

也可以 `npm start`（等价于上面这条命令）。从源码构建前端需要 Node.js 20.19+，见「开发」。

可选参数（`--flag=value` 与 `--flag value` 两种写法都支持）：

| 参数 | 说明 |
| --- | --- |
| `--host=<ADDR>` | 监听地址，默认 `127.0.0.1`（只允许本机访问）；手机同网测试用 `--host=0.0.0.0` |
| `--port=<PORT>` | 监听端口，默认 `8787` |
| `--cache-dir=<DIR>` | 缓存目录，默认 `<安装目录>/.cache`，也可用环境变量 `OSU_PREVIEW_CACHE_DIR` |
| `--https` | 用自签证书启用 HTTPS；证书在首次启动时生成到 `<缓存目录>/.tls/` |
| `--tls-cert`、`--tls-key` | 指定已有证书与私钥（PEM），同时提供时不再自动生成 |
| `--no-cache` | 忽略已缓存的 `.osu` 与 `.osz`，强制重新下载 |
| `--quiet` | 不打印下载日志 |
| `--help` | 打印用法 |

默认只监听本机地址且没有鉴权，请勿直接暴露到公网。

## 手机同网测试

WebGPU 只在**安全上下文**中可用：`http://localhost` 算安全上下文，但 `http://192.168.x.x` 不算，此时手机浏览器里 `navigator.gpu` 是 `undefined`，页面无法渲染。所以局域网访问要开 HTTPS：

```bash
node backend/server.js --host 0.0.0.0 --port 8443 --https
```

启动时会打印本机可用的地址，手机用同一个 Wi-Fi 打开 `https://<局域网IP>:8443`。自签证书不受信任，首次访问需要在警告页选择继续访问（Chrome 的“高级 → 继续前往”，Safari 的“显示详细信息 → 访问此网站”）。

还需要注意两点：

- Windows 防火墙默认会拦截入站连接。首次从手机访问时如果弹出 Node.js 的防火墙提示，勾选专用/公用网络并允许；已经拦截过的话，用管理员权限执行
  `netsh advfirewall firewall add rule name="osu-beatmap-preview-web" dir=in action=allow protocol=TCP localport=8443 profile=any`。
- 浏览器本身要支持 WebGPU（Android Chrome 121+、iOS Safari 26+ 默认开启，iOS 18.2 起可在“高级 → 功能开关”里打开）。确实没有 WebGPU 时，只能用桌面浏览器打开。

如果页面提示「No suitable graphics adapter found」或一直停在“等待渲染”，在手机上打开 **`/gpu-check.html`**：这个页面不加载 wasm，只列出 `isSecureContext`、`navigator.gpu`、硬件适配器与 CPU 回退适配器、`getContext("webgpu")` 的结果，可以直接定位是浏览器不支持、设备 GPU 被浏览器屏蔽，还是应用内浏览器。

wasm 侧会先请求硬件适配器，失败后再请求一次 CPU 回退适配器（`force_fallback_adapter`），因此 GPU 被屏蔽的设备仍能以软件光栅化运行，只是帧率较低。

## 目录结构

```text
crates/osu-beatmap-preview-web/
├─ backend/           # 后端：HTTP 入口、下载、缓存、镜像、ZIP 读取、资源准备
│  └─ server.js       # 服务入口：路由、Range、静态文件、加载进度
├─ src/               # Vue 前端源码（单文件组件 + Tailwind）
├─ dist/              # Vue 构建结果（构建生成，发布包里已包含）
├─ public/            # 构建时原样拷进 dist/ 的静态资源
│  ├─ pkg/            # wasm 产物（构建生成，发布包里已包含）
│  └─ gpu-check.html  # 不加载 wasm 的 WebGPU 自检页
├─ scripts/           # npm run build:wasm 使用的构建脚本
└─ test/              # node --test 回归测试与 OSZ fixture
```

后端与前端是两套独立的构建：`backend/` 是普通的 ESM，直接 `node` 运行；`src/` 由 Vite 编译成 `dist/`，`public/` 里的文件原样拷贝进 `dist/`。`dist/` 是后端唯一托管的目录。

## 前端

- **框架**：Vue 3 单文件组件，没有路由、没有状态管理库；`src/preview.js` 用一份模块级单例状态承载会话、音频、播放时钟与 Mod，组件只画界面。
- **样式**：Tailwind CSS v4（`@tailwindcss/vite`），全部是构建期生成的静态 CSS，运行时不请求任何 CDN。
- **深链**：`/?bid=<BID>` 直接进入预览，可选 `&convert=taiko`；加载成功后地址栏会写回这两个参数，刷新或分享都能回到同一个预览，点「返回加载」时清除。
- **默认视图**：只保留一行谱面信息、视频和进度条，视频区域最大。进度条在鼠标移动或触摸时显示，停下约 1 秒后淡出；淡出只改透明度，进度条原来的位置始终占着，所以画面不会上下位移。画面参数（30/60/120 FPS、480P/720P/1080P、0.5x–2x 倍速）、Mod 与运行日志都收在右上角齿轮打开的抽屉里。
- **谱面信息**：名称与难度取自 WASM 的 `beatmapInfo`（见下）。
- **操作**：点击画面播放/暂停，空格同样；`Esc` 关闭抽屉。左右方向键短按在**松开时**跳转 ±5 秒（按下不跳），长按右键进入 3 倍速播放、长按左键持续向前倒带，两种情况都会在画面上显示角标。

## 后端接口

前端只会用到下面这些路由；后端也不提供其他写操作。

| 路由 | 说明 |
| --- | --- |
| `GET /`、`/assets/<哈希>` | `dist/` 下的静态站点页面、脚本与样式 |
| `GET /pkg/<文件>` | wasm 产物（`.js`、`.wasm`、`.d.ts`） |
| `GET /gpu-check.html` | WebGPU 自检页 |
| `GET /resource/beatmap?bid=<BID>` | 该难度的 `.osu` 文本，命中缓存时直接返回本地文件 |
| `GET /resource/audio?bid=<BID>` | 从 OSZ 中取出的音频，支持 `Range` 请求（进度条 seek 需要） |
| `GET /resource/background?bid=<BID>` | 从 OSZ 中取出的背景图 |
| `GET /resource/progress?bid=<BID>` | 加载进度快照（JSON）：`phase`、`received`、`total` |

`bid` 必须是纯数字，否则返回 `400`；下载或解析失败返回 `502` 并附带原因文本，前端会把它显示在日志里。

`/resource/progress` 的阶段依次是 `osu`（取谱面）→ `osz`（下载谱面包，带字节进度）→ `extract`（解包音频与背景）→ `ready`，失败时为 `error`。前端在加载期间每 400 ms 轮询一次，`total` 未知时显示不确定态进度条。进度快照按 bid 保留 10 分钟。

## 下载与缓存

下载策略与 CLI 一致，两边共用同一套缓存目录，已经用 CLI 下载过的谱面在 Web 端会直接命中：

- `.osu` 从 `https://osu.ppy.sh/osu/<bid>` 获取；`.osu` 里缺少 `BeatmapSetID` 时，跟随 `https://osu.ppy.sh/beatmaps/<bid>` 的重定向解析真实谱面集 ID。
- `.osz` 在 sayobot、osu.direct、nekoha、catboy 之间竞速，最多 3 个尝试同时进行；尝试失败立即补位，出现「3 秒没有首字节」或「5 秒窗口内低于 128 KiB/s」时触发回退，必要时取消最慢的尝试。
- 单个尝试先用 `Range: bytes=0-0` 探测分块支持，支持则按 4 块并行下载，失败回退单流；超过 `Content-Length`、超过 50 MiB 或不是有效 ZIP 的结果都会被拒绝。
- osu.direct 会从 Cloudflare 网段采样候选 IP，先测 TCP 再测 HTTPS，胜者写入缓存并在 24 小时内复用。
- 同一 `bid` 的并发请求（音频 + 背景）会合并成一次下载，不会重复拉包。

缓存布局：

| 内容 | 位置 |
| --- | --- |
| `.osu` | `<缓存目录>/osu-download-cache/<bid>.osu` |
| OSZ | `<缓存目录>/osz-download-cache/<setId>.osz` |
| osu.direct 优选 IP | `<缓存目录>/osu-direct-preferred-ip.json` |
| 解包后的音频与背景 | `<缓存目录>/media/<bid>/audio.<ext>`、`background.<ext>` |

缓存不会自动清理，空间占用过大时可以直接删除整个缓存目录。所有写入都先落临时文件再改名，中断不会留下半个文件。

## WASM 接口：谱面内部信息

`beatmapInfo(bytes)` 按传入的 `.osu` 字节返回谱面内部信息（WASM 没有网络能力，`bid` → 字节由宿主的 `/resource/beatmap` 完成）：

```js
const module = await import('/pkg/osu_beatmap_preview_wasm.js');
await module.default();
const bytes = new Uint8Array(await (await fetch('/resource/beatmap?bid=738063')).arrayBuffer());
const info = module.beatmapInfo(bytes);
info.title;          // 'No title'
info.version;        // "Lust's Insane"（难度名）
info.metadata;       // [Metadata] 全量键值
```

返回值覆盖概览字段（`title`、`titleUnicode`、`artist`、`artistUnicode`、`creator`、`version`、`source`、`tags`、`beatmapId`、`beatmapSetId`、`mode`、`modeName`、`formatVersion`、`audioFilename`、`audioLeadInMs`、`stackLeniency`、`backgroundFilename`、`beatDivisor`）、统计（`hitObjectCount`、`firstObjectMs`、`lastObjectEndMs`、`chartDurationMs`、`bpm`、`timingPointCount`、`breakPeriodCount`、`comboColors`）、难度（`ar`、`cs`、`hp`、`od`）以及 `general`、`metadata`、`difficulty` 三个区段的全量键值。当前界面只用到名称与难度，其余字段保留给后续展示。

## 开发

需要 Node.js 20.19+（Vite 7 的要求）：

```bash
npm install          # 安装 Vue / Vite / Tailwind，仅开发与构建需要
npm test             # ZIP 读取、Range、路径归一化、`.osu` 字段解析等回归测试
npm run build        # 把 src/ 编译到 dist/
npm run build:wasm   # 构建 wasm 产物到 public/pkg
npm start            # node backend/server.js
```

改前端时可以用 Vite 开发服务器（前端热更新，后端要另开一个终端）：

```bash
npm run dev:server   # 终端 1：后端，默认 127.0.0.1:8787
npm run dev          # 终端 2：Vite，默认 5173，/resource 会代理到后端
```

`npm run build:wasm` 需要 Rust 工具链、`wasm32-unknown-unknown` 目标和与 `Cargo.lock` 中 `wasm-bindgen` 版本一致的 `wasm-bindgen-cli`：

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
```

发布包里已经带好 `dist/` 与 `public/pkg`，最终用户只需要 Node.js，不需要 Rust，也不需要 `npm install`。

## 相关文档

- [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)：浏览器侧的 `WebGpuSession` 与 `beatmapInfo` API 与职责边界。
- [CLI 使用说明](../osu-beatmap-preview-cli/README.md)：导出 PNG/GIF/MP4 的完整参数说明。
- [架构说明](../../docs/architecture.md)：各 crate 的划分与 API 边界。

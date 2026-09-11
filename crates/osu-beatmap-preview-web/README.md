# Web 站点与下载后端（osu-beatmap-preview-web）

[返回项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [WASM 使用说明](../osu-beatmap-preview-wasm/README.md) | [架构说明](../../docs/architecture.md)

这是 Release 里的 **Web 包**：一个静态站点加一个很薄的 Node.js 后端。

- **静态站点**：浏览器里跑 [osu-beatmap-preview-wasm](../osu-beatmap-preview-wasm/README.md)，谱面解析、Mod、转谱和每一帧绘制都在本地 WebGPU 中完成。
- **Node.js 后端**：浏览器因为跨域限制无法直接下载 osu! 的资源，后端负责下载并缓存 `.osu` 与 `.osz`，再把音频、背景图交给前端。后端不参与渲染，也不上传任何数据。

## 运行

需要 Node.js 18 或更高版本，以及支持 WebGPU 的浏览器（Chrome / Edge 113+ 等）。

```bash
node server.js
# 然后访问 http://127.0.0.1:8787
```

可选参数：

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
node server.js --host 0.0.0.0 --port 8443 --https
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
├─ server.js          # HTTP 服务入口：路由、Range、静态文件
├─ src/               # 后端模块（下载、缓存、镜像、ZIP 读取、资源准备）
├─ public/            # 静态站点：index.html、app.js、style.css
│  └─ pkg/            # wasm 产物（构建生成，发布包里已包含）
├─ scripts/           # npm run build:wasm 使用的构建脚本
└─ test/              # node --test 回归测试与 OSZ fixture
```

## 后端接口

前端只会用到下面这些路由；后端也不提供其他写操作。

| 路由 | 说明 |
| --- | --- |
| `GET /`、`/app.js`、`/style.css` | 静态站点页面与脚本 |
| `GET /pkg/<文件>` | wasm 产物（`.js`、`.wasm`、`.d.ts`） |
| `GET /resource/beatmap?bid=<BID>` | 该难度的 `.osu` 文本，命中缓存时直接返回本地文件 |
| `GET /resource/audio?bid=<BID>` | 从 OSZ 中取出的音频，支持 `Range` 请求（进度条 seek 需要） |
| `GET /resource/background?bid=<BID>` | 从 OSZ 中取出的背景图 |

`bid` 必须是纯数字，否则返回 `400`；下载或解析失败返回 `502` 并附带原因文本，前端会把它显示在日志里。

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

## 开发

```bash
npm test            # ZIP 读取、Range、路径归一化、`.osu` 字段解析等回归测试
npm run build:wasm  # 构建 wasm 产物到 public/pkg
```

`npm run build:wasm` 需要 Rust 工具链、`wasm32-unknown-unknown` 目标和与 `Cargo.lock` 中 `wasm-bindgen` 版本一致的 `wasm-bindgen-cli`：

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
```

发布包里已经带好 `public/pkg`，最终用户只需要 Node.js，不需要 Rust。

## 相关文档

- [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)：浏览器侧的 `WebGpuSession` API 与职责边界。
- [CLI 使用说明](../osu-beatmap-preview-cli/README.md)：导出 PNG/GIF/MP4 的完整参数说明。
- [架构说明](../../docs/architecture.md)：各 crate 的划分与 API 边界。

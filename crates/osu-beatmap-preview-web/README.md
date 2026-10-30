# Web 站点说明

[项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)

Web 包提供四模式谱面实时预览和回放预览。Vue 页面通过 WASM 完成解包、解码、判定、混音与 WebGPU 绘制；Node.js 后端负责资源下载、官方成绩查询和缓存。

## 启动

运行发布包需要 Node.js 18+ 和支持 WebGPU 的浏览器。包内包含构建好的前端与 WASM，解压后运行：

```bash
node backend/server.js
# 或 npm start
```

访问 [http://127.0.0.1:8787](http://127.0.0.1:8787)。WebGPU 需要安全上下文：本机可用 localhost 或回环地址，其他设备访问时需要 HTTPS 和浏览器可用的 WebGPU。

| 启动参数 | 说明 |
| --- | --- |
| `--host=<地址>` | 默认 `127.0.0.1`；局域网或容器内监听全部地址可用 `0.0.0.0` |
| `--port=<端口>` | 默认 `8787` |
| `--cache-dir=<目录>` | 默认包目录下的 `.cache`，也可设 `OSU_PREVIEW_CACHE_DIR` |
| `--https` | 启用 HTTPS，自动生成的自签证书存放在缓存目录的 `.tls/` |
| `--tls-cert=<路径>`、`--tls-key=<路径>` | 同时指定已有 PEM 证书与私钥 |
| `--tls-host=<域名或IP>` | 补充自签证书 SAN，可重复；环境变量 `OSU_PREVIEW_TLS_HOSTS` 使用逗号分隔 |
| `--no-cache` | 跳过已下载资源缓存 |
| `--quiet` | 关闭下载日志 |
| `--help` | 查看用法 |

带值参数支持 `--参数=值` 和 `--参数 值`。自动生成证书需要可用的 OpenSSL。

## 谱面预览

加载页默认选择「谱面预览」，可填写单张难度的谱面 ID，或分别选择本地 `.osu` 与 `.osz`：

| 本地输入 | 行为 |
| --- | --- |
| `.osu` | 使用该谱面，无音乐和背景媒体 |
| `.osz` | 使用包内难度，可在难度列表中切换；填写谱面 ID 时按 ID 匹配 |
| `.osu` + `.osz` | 使用外部谱面，媒体取自包内；两者必须通过有效 `BeatmapID` 绑定 |

支持四模式原生谱面，以及 Standard 转 Taiko、Catch、Mania。播放页可切换 Mod、跳转进度，调整倍速（0.1–3 倍）、音乐与打击音音量、480P/720P/1080P 分辨率和 30/60/120 FPS。

音乐音量默认 50%，打击音默认开启、音量 100%，背景默认暗化 70%；背景视频、故事板、谱面打击音和帧率显示在页面中默认开启。「画面 → 其他」可切换 Standard 的光标和 Catch 的自动接盘，均默认关闭。普通谱面预览的 Mod 选项采用 [CLI 文档](../osu-beatmap-preview-cli/README.md#mod-支持)中 PNG 与 GIF/MP4 的累积支持范围。

## 回放预览

切换到「回放预览」后，选择以下一种来源：

| 来源 | 输入与选择规则 |
| --- | --- |
| 本地 OSR | 选择 `.osr`，本地回放字节只在浏览器处理 |
| 成绩 ID / 官方成绩链接 | 统一成绩 ID 或 `https://osu.ppy.sh/scores/<ID>` |
| 玩家 + 谱面 ID | 用户名与谱面 ID；默认官方个人最高分，可选最近一次 |
| 玩家最近游玩 | 用户名；无需谱面 ID，取官方最近一条成绩 |

玩家查询接受用户名，纯数字也按用户名处理。查询模式可选 `osu`、`taiko`、`fruits`、`mania`；指定谱面时省略模式使用原始谱面模式，未指定谱面时由官方使用玩家默认模式。最高分按分数选择；指定谱面的最近一次最多扫描近期 1000 条成绩，按结束时间选择。最近成绩默认排除失败，可用「包含失败」开启。

查到不支持的模式、无可用回放或不兼容成绩时直接报错，不替换成绩。

### 谱面匹配与播放

各来源均可补充本地 `.osu` 与 `.osz`。回放需要包含音乐的完整谱面包，缺包时自动下载；仅提供本地 OSR 和匹配的完整 OSZ 即可离线播放。外部 `.osu` 和包内选中谱面都必须以原始字节 MD5 匹配回放，不能用修改过的同 ID 谱面替代。

支持 stable、lazer、tachyon 回放，Mod 支持范围见 [CLI 文档](../osu-beatmap-preview-cli/README.md#mod-支持)。

回放按模式显示光标、按键、判定和成绩 HUD（Mania 不显示右侧按键面板），使用当前官方标准化计分规则重算，历史记录的分数差异作为诊断。回放期间不能修改 Mod 或转谱，但可调整播放倍速与画面。输入提前结束时显示失败，成绩与光标冻结、按键清零、玩家音效停止；背景、谱面与音乐继续，跳回失败前可查看当时状态。

### 官方 API 认证

在线成绩下载与按 MD5 自动查谱需要部署者在后端进程环境配置以下任一方式：

- `OSU_API_TOKEN`：可用的官方 API 访问令牌。
- `OSU_CLIENT_ID` 与 `OSU_CLIENT_SECRET`：OAuth 客户端凭据，由后端获取令牌。

凭据只在服务端使用，页面访问者无需填写。本地匹配资源齐全时不需要认证；有匹配 `.osu` 且有效谱面 ID 时可直接据此下载包，否则需要 API 查谱。查询、下载和 MD5 校验失败都会显示错误。

## 分享链接

- 谱面：`/bids/<谱面ID>`；转谱可附 `?convert=taiko`、`ctb` 或 `mania`。
- 回放：`/scores/<成绩ID>`；玩家查询成功后使用最终选中成绩的 ID。

在线加载成功后地址栏同步为对应链接，刷新可重新加载。本地资源加载后地址回到 `/`，链接不能携带本地文件。回放链接中附带的转谱请求同样受回放兼容性检查约束。

## 后端接口与缓存

| GET 接口 | 用途 |
| --- | --- |
| `/resource/file?bid=<ID>` | 返回完整 OSZ 字节，支持 Range |
| `/resource/progress?bid=<ID>` | 查询下载与传输进度 |
| `/resource/replay-lookup?checksum=<MD5>` | 官方查谱，返回谱面 ID、谱面集 ID 和 checksum |
| `/resource/replay?scoreId=<ID>` | 下载指定成绩的 OSR |
| `/resource/replay?user=<用户名>` | 查询玩家成绩并下载 OSR，可附加下列参数 |

玩家查询可附带 `bid`、`selection=best|recent`、`mode=osu|taiko|fruits|mania`、`includeFails=0|1`。有 `bid` 时默认 `best`，无 `bid` 时默认 `recent`；`best` 必须有 `bid`，`includeFails` 仅用于最近成绩。`scoreId` 与玩家查询参数互斥。

在线资源缓存在配置的缓存目录，OSR 位于 `osr-download-cache/<成绩ID>.osr`；最高分与近期成绩的定位每次重新查询。Web 下载完整带视频谱面包，默认大小上限 256 MiB。缓存不自动清理，可停止服务后按需删除。

## Docker 部署

在仓库根目录构建；镜像包含 WASM、前端、Node.js 和 OpenSSL：

```bash
docker build -f Docker/Dockerfile-web -t osu-beatmap-preview-web .
docker run -d --name osu-preview --restart unless-stopped \
  -p 127.0.0.1:8787:8787 -v osu-preview-cache:/data \
  -e OSU_CLIENT_ID="" -e OSU_CLIENT_SECRET="" \
  osu-beatmap-preview-web
```

使用宿主机上的反向代理提供 HTTPS，例如 Caddy：

```caddyfile
preview.example.com {
    reverse_proxy 127.0.0.1:8787
}
```

也可由容器直接提供自签 HTTPS；浏览器需信任证书，且证书 SAN 应覆盖实际访问地址：

```bash
docker run -d --name osu-preview --restart unless-stopped \
  -p 443:8443 -v osu-preview-cache:/data \
  -e OSU_CLIENT_ID="" -e OSU_CLIENT_SECRET="" \
  osu-beatmap-preview-web \
  node backend/server.js --host=0.0.0.0 --port=8443 --https --tls-host=服务器IP
```

需要在线回放时，在上述空值中填入官方 API 凭据；若宿主机已导出同名环境变量，可改为 `-e OSU_CLIENT_ID -e OSU_CLIENT_SECRET`，由 Docker 直接传入。服务本身没有用户鉴权，访问控制由部署环境提供。

## 开发

前端构建需要 Node.js 20.19+；先完成 [WASM 构建准备](../osu-beatmap-preview-wasm/README.md#构建)，再在本目录运行：

```bash
npm install
npm run build:wasm
npm run build
npm test
npm start
```

顺序必须是先构建 WASM，再构建前端，保证 `public/pkg/` 被复制进 `dist/`。修改 Rust 后重新执行两步构建，重启服务并刷新页面；仅修改前端时执行 `npm run build`。

Windows 可用 `run_web.ps1` 完成依赖安装、构建与启动：

```powershell
.\run_web.ps1
.\run_web.ps1 --port=8443 --https
.\run_web.ps1 -NoWasm
.\run_web.ps1 -NoServe
.\run_web.ps1 -Help
```

`backend/` 为后端，`src/` 为 Vue 页面，`public/` 为静态资源，`dist/` 为发布页面，`test/` 为 Node.js 测试。播放与音频集成 API 见 [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)。

# Web 站点与下载后端（osu-beatmap-preview-web）

[返回项目首页](../../README.md) | [CLI 使用说明](../osu-beatmap-preview-cli/README.md) | [WASM 使用说明](../osu-beatmap-preview-wasm/README.md) | [架构说明](../../docs/architecture.md)

这是 Release 里的 **Web 包**：一个 Vue 静态站点加一个很薄的 Node.js 后端。

- **静态站点**：Vue 3 单文件组件 + Tailwind CSS v4，浏览器里跑 [osu-beatmap-preview-wasm](../osu-beatmap-preview-wasm/README.md)，谱面解析、Mod、转谱和每一帧绘制都在本地 WebGPU 中完成；也支持原生 Standard `.osr`，回放文件只留在浏览器本地。
- **Node.js 后端**：浏览器因为跨域限制无法直接下载 osu! 的资源，后端负责下载并缓存 `.osu` 与 `.osz`，再把音频、背景图交给前端；OSR 缺少谱面时，后端用服务端 OAuth/API 凭据按 checksum 查询谱面并下载完整谱面包。OSR 文件本身不上传。

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
| `--tls-host=<HOST>` | 自签证书要覆盖的域名或 IP，可重复；也可用 `OSU_PREVIEW_TLS_HOSTS`（逗号分隔）。容器里探测到的是容器自己的地址，必须用它补上浏览器实际访问的地址 |
| `--no-cache` | 忽略已缓存的 `.osu` 与 `.osz`，强制重新下载 |
| `--quiet` | 不打印下载日志 |
| `--help` | 打印用法 |

默认只监听本机地址且没有鉴权，请勿直接暴露到公网。

## OSR 回放来源

加载页「回放来源」提供三个互斥入口：

- **本地 OSR**：选择 `.osr`；不选择则沿用普通谱面预览。本地回放字节不上传服务器。
- **成绩 ID / 官方成绩链接**：填写 `6086051920` 或 `https://osu.ppy.sh/scores/6086051920`；旧式 `/scores/osu/<ID>` 链接暂不接受。
- **玩家 + 谱面 ID**：明确选择玩家名或玩家 ID，填写单张难度的谱面 ID；默认取官方个人最高分，可切换「最近一次」。最高分按分数而非 PP；最近一次只扫描官方近期最多 1000 条成绩（含失败），按结束时间选择，查不到不回退。

三种来源都可在下方附带 `.osu/.osz`，匹配难度由回放 MD5 自动决定。本地 OSR 搭配完整匹配 OSZ 可离线播放；在线成绩则由服务端下载回放，再由浏览器校验并播放。缺少谱面包时自动下载完整包；无回放、不兼容、MD5 不符或下载失败均明确报错，不换其它成绩。切换来源会清理旧本地回放，其余来源的输入不会一起发送。

支持原生 Standard，不以客户端版本或 OSR 分数版本限制播放；支持 tachyon、历史 lazer 和 stable。回放与普通 Standard 动画预览支持相同的 Mod：EZ/HR/HD/FL/AT/DA/TC/CL/DT/HT/NC/DC，按录制设置应用几何、可见性、速度和当前官方计分倍率。stable 自动启用 CL，lazer 显式 CL 使用同一规则。历史帧按官方方式归一化，原记录与重算成绩的差异只作诊断。AL 和其它未实现 Mod 仍明确报错；回放模式不可转谱或修改 Mod。支持范围和样本验收见 [OSR 说明](../../docs/osr-preview.md)。

在线成绩下载或仅上传 OSR 自动查谱时，服务器必须配置 `OSU_API_TOKEN`，或 `OSU_CLIENT_ID` 与 `OSU_CLIENT_SECRET`；OAuth 凭据只在服务端使用。上传 `.osu` 时先核验 MD5，再按谱面 ID 下载 OSZ；没有有效谱面 ID 时也需要官方 API 查谱。查询和下载失败会直接显示错误。在线回放缓存于 `osr-download-cache/<成绩ID>.osr`，`--no-cache` 跳过文件缓存；最高分和近期成绩的定位每次重新查询。凭据由部署者配置一次，页面访问者无需填写。

修改 core/WASM 后依次运行 `npm run build:wasm`、`npm run build`，最后重启后端并刷新浏览器。

## Docker 部署

`Docker/Dockerfile-web` 会把整个 Web 包从源码构建成镜像（Rust → wasm、Vite → `dist/`、Node 运行时），**构建上下文必须是仓库根目录**（wasm 需要整个 Cargo workspace），所以在根目录执行：

```bash
docker build -f Docker/Dockerfile-web -t osu-beatmap-preview-web .
docker run -d --name osu-preview --restart unless-stopped \
  -p 8787:8787 -v osu-preview-cache:/data \
  osu-beatmap-preview-web
# 打开 http://<服务器地址>:8787
```

### 服务器上必须用 HTTPS，否则页面提示没有 WebGPU

WebGPU 只在**安全上下文**里可用：

**① 有域名：前面放反向代理拿真证书（推荐）**

```caddyfile
# Caddyfile
preview.example.com {
    reverse_proxy 127.0.0.1:8787
}
```

```bash
docker run -d --name osu-preview --restart unless-stopped \
  -p 127.0.0.1:8787:8787 -v osu-preview-cache:/data osu-beatmap-preview-web
docker run -d --name caddy --restart unless-stopped \
  -p 80:80 -p 443:443 -v $PWD/Caddyfile:/etc/caddy/Caddyfile \
  caddy:2
```

Nginx 同理：`proxy_pass http://127.0.0.1:8787;` 再配 `certbot` 签证书。手机浏览器要信任该证书，所以不要用自签。

**② 只有 IP：用容器自带的 `--https` + `--tls-host`**

自签证书会被浏览器标记为不受信任，点“高级 → 继续访问”后仍然是 https，WebGPU 就能用：

```bash
docker run -d --name osu-preview --restart unless-stopped \
  -p 443:8443 -v osu-preview-cache:/data \
  osu-beatmap-preview-web \
  node backend/server.js --host=0.0.0.0 --port=8443 --https \
    --tls-host=你的服务器IP
```

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

## 开发

需要 Node.js 20.19+（Vite 7 的要求）：

```bash
npm install          # 安装 Vue / Vite / Tailwind，仅开发与构建需要
npm test             # ZIP 读取、Range、路径归一化、`.osu` 字段解析等回归测试
npm run build        # 把 src/ 编译到 dist/
npm run build:wasm   # 构建 wasm 产物到 public/pkg
npm start            # node backend/server.js
```

Windows 上也可以直接运行 `run_web.ps1`：

```powershell
.\run_web.ps1                         # 全部构建后启动（http://127.0.0.1:8787）
.\run_web.ps1 --port=8443 --https     # 其余参数原样转发给后端
.\run_web.ps1 -NoWasm                 # 只改了前端时跳过 wasm 构建（快很多）
.\run_web.ps1 -NoBuild                # 只改了 wasm 时跳过前端构建
.\run_web.ps1 -NoServe                # 只构建，不启动
.\run_web.ps1 -Help                   # 选项说明
```

## 相关文档

- [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)：浏览器侧的 `WebGpuSession` 与 `beatmapInfo` API 与职责边界。
- [CLI 使用说明](../osu-beatmap-preview-cli/README.md)：导出 PNG/GIF/MP4 的完整参数说明。
- [架构说明](../../docs/architecture.md)：各 crate 的划分与 API 边界。

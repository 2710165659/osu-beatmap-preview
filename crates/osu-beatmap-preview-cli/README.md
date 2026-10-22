# CLI 使用说明

[项目首页](../../README.md) | [Web 站点说明](../osu-beatmap-preview-web/README.md) | [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)

CLI 将四模式谱面导出为 PNG、GIF 或带音频的 MP4，并支持回放的 PNG/GIF/MP4 导出。运行时无需 FFmpeg、额外动态库或外部皮肤。

## 获取与快速开始

从 [Releases](https://github.com/2710165659/osu-beatmap-preview/releases) 下载对应平台的产物：

| 平台 | 文件 |
| --- | --- |
| Windows x64 | `osu-beatmap-preview-windows-amd64-cli.exe` |
| Linux x64 | `osu-beatmap-preview-linux-amd64-cli` |
| macOS Intel | `osu-beatmap-preview-macos-amd64-cli` |
| macOS Apple Silicon | `osu-beatmap-preview-macos-arm64-cli` |

Linux/macOS 首次使用需要 `chmod +x ./osu-beatmap-preview-*-cli`。下文假定程序已命名为 `osu-beatmap-preview-cli` 并加入 `PATH`；PowerShell 调用当前目录程序时加 `.\` 前缀。

```bash
# 在线谱面：Standard 默认 GIF，其他模式默认 PNG
osu-beatmap-preview-cli --bid=738063

# 指定格式、缩放与输出目录
osu-beatmap-preview-cli --bid=738063 --fmt=png --scale=2 --output-dir=outputs

# Standard 转 Mania，使用 4K 和 1.25 倍 DT
osu-beatmap-preview-cli --bid=738063 --convert=mania --mod=4k --mod=dt1.25 --fmt=gif

# 从 PreviewTime 开始导出 30 秒视频
osu-beatmap-preview-cli --bid=738063 --fmt=mp4 --time-points=preview --duration-time=30

# 本地谱面包，使用 --bid 选择难度
osu-beatmap-preview-cli --osz-file=map.osz --bid=738063 --fmt=mp4

# 本地回放，默认输出 MP4
osu-beatmap-preview-cli --type=osr --osr-file=play.osr --osz-file=map.osz
```

## 命令行参数

参数支持 `--参数=值` 或 `--参数 值`；Mod 组合和多个时间点需要分别重复传入。

| 参数 | 说明 |
| --- | --- |
| `--type=osu\|osr` | 默认 `osu` 预览谱面；`osr` 预览回放 |
| `--bid=<ID>` | 在线谱面 ID、本地难度选择，或回放玩家查询的谱面 ID |
| `--osu-file=<路径>` | 本地 `.osu`，可独立使用或搭配媒体包 |
| `--osz-file=<路径>` | 本地 `.osz` 谱面包 |
| `--osr-file=<路径>` | 本地回放，需要 `--type=osr` |
| `--score-id=<ID>` | 下载统一成绩 ID 的回放，需要 `--type=osr` |
| `--user=<用户名>` | 查询玩家回放，需要 `--type=osr`；纯数字也按用户名查询 |
| `--score-select=best\|recent` | 仅玩家查询：有 `--bid` 时默认 `best`，无 `--bid` 时默认 `recent` |
| `--mode=osu\|taiko\|fruits\|mania` | 仅玩家查询；指定谱面时省略默认 `osu`，无谱面时由官方使用玩家默认模式 |
| `--include-fails` | 最近成绩查询包含失败成绩；默认排除，指定谱面时需搭配 `--score-select=recent` |
| `--convert=<模式>` | `standard` / `std`、`taiko`、`ctb`、`mania`；只有 Standard 可转其他模式，同模式视为无操作 |
| `--fmt=png\|gif\|mp4` | 谱面默认 Standard 为 GIF、其他为 PNG；回放默认 MP4，支持 PNG/GIF/MP4 |
| `--mod=<Mod>` | 缩写或全名，不区分大小写；全名空格写成短横杠，如 `hard-rock` |
| `--time-points=<秒数或preview>` | 片段起点；GIF、Standard PNG 可重复，MP4 与其他模式 PNG 最多一个 |
| `--duration-time=<秒数>` | GIF 每段时长、MP4 时长或区间段 PNG 时长；必须为有限正数 |
| `--fps=<1–60>` | GIF/MP4 的整数帧率，默认取对应模式和格式配置 |
| `--scale=<倍率>` | 本次输出缩放倍率，必须为有限正数 |
| `--output-dir=<目录>` | 本次输出根目录 |
| `--config=<路径或对象>` | JSON/YAML 文件或内联对象，只能传入一次 |
| `--no-cache` | 跳过下载和输出缓存 |
| `--no-log` | 关闭文件日志 |
| `--version` | 打印版本号 |
| `--help`、`-h` | 打印用法 |

## 谱面来源

未指定本地文件时，必须提供 `--bid`，程序会下载谱面和所需媒体。提供本地文件时使用下表规则，不另行联网补充媒体：

| 输入 | 难度选择与绑定 | 输出 |
| --- | --- | --- |
| 仅 `--osu-file` | 可省略 `--bid`；提供时必须匹配有效的 `BeatmapID` | PNG、GIF，无媒体 |
| 仅 `--osz-file` | 只有一个顶层 `.osu` 时可自动选择；多个时必须用 `--bid` 匹配 | PNG、GIF、MP4 |
| 两个文件 | 使用外部 `.osu`，从 `.osz` 读取媒体；外部有效正数 `BeatmapID` 必须存在于包内，提供的 `--bid` 也必须一致 | PNG、GIF、MP4 |

扩展名不区分大小写，包内子目录中的 `.osu` 不参与难度选择。没有顶层谱面、无法解析或指定 ID 不匹配时会报错。单文件或唯一难度没有有效 ID 时，产物与日志以输入文件名标识。

```bash
# 唯一顶层难度无需 --bid
osu-beatmap-preview-cli --osz-file=single-map.osz --fmt=mp4

# 外部编辑谱面搭配同 ID 媒体包
osu-beatmap-preview-cli --osu-file=edited.osu --osz-file=map.osz --fmt=mp4
```

## 回放

所有回放命令均需 `--type=osr`，以下三种来源必须且只能选择一种：

| 来源 | 用法 |
| --- | --- |
| 本地 OSR | `--osr-file=play.osr` |
| 统一成绩 ID | `--score-id=6086051920` |
| 玩家成绩 | `--user=玩家名`，可搭配 `--bid`、`--mode`、`--score-select` |

`--bid` 只用于玩家来源，不能附在本地 OSR 或成绩 ID 来源上。玩家最高分必须指定谱面，按官方分数选择；指定谱面的最近一次最多扫描近期 1000 条成绩，按结束时间选择；不指定谱面时取官方最近一条。无回放、不支持的模式、MD5 不符或下载失败均直接报错，不换其他成绩。

```bash
# 离线 GIF，只需要原始谱面和回放
osu-beatmap-preview-cli --type=osr --osr-file=play.osr --osu-file=map.osu --fmt=gif

# 离线 MP4，需要匹配的完整谱面包
osu-beatmap-preview-cli --type=osr --osr-file=play.osr --osz-file=map.osz

# 在线成绩或指定谱面的个人最高分
osu-beatmap-preview-cli --type=osr --score-id=6086051920
osu-beatmap-preview-cli --type=osr --user="玩家名" --bid=4903207

# 指定谱面最近一次，包含失败成绩
osu-beatmap-preview-cli --type=osr --user="玩家名" --bid=4903207 --score-select=recent --include-fails

# 玩家最近游玩，省略 --mode 时使用玩家默认模式
osu-beatmap-preview-cli --type=osr --user="玩家名"
osu-beatmap-preview-cli --type=osr --user="玩家名" --mode=osu --include-fails
```

### 输出与兼容性

- **PNG/GIF**：只使用 `.osu` 与 `.osr`，不读取或下载 OSZ，不加载音频、背景、视频或故事板。PNG 使用 `render.standard.png` 的快照网格、选段、标签和缩放配置，每格显示对应时刻的回放与 HUD；GIF 默认 2×2 同时播放不同片段，使用 `render.standard.gif` 配置。
- **MP4**：使用完整 OSZ 加载音乐、背景与音效，画面与音频开关使用 `render.standard.mp4` 配置。缺少资源时自动查谱下载。
- **谱面匹配**：按回放记录的原始 `.osu` 字节 MD5 校验；MP4 同时提供外部谱面与媒体包时，两份谱面都必须匹配。
- **Mod**：使用回放文件中的 Mod，不接受 `--convert` 或 `--mod`。
- **兼容性**：支持 stable、lazer、tachyon 回放；成绩按当前游戏规则重算，可能与历史记录不同。
- **提前结束**：输入提前结束时显示失败，冻结成绩与光标，清空按键并停止玩家音效；MP4 中背景、谱面和音乐继续播放。

### 官方 API 认证

在线回放下载与按 MD5 查谱优先使用环境变量 `OSU_API_TOKEN`；没有令牌时，使用 `OSU_CLIENT_ID` 与 `OSU_CLIENT_SECRET`，分别优先读取同名非空环境变量，再读取 `download.auth` 配置：

```yaml
download:
  auth:
    OSU_CLIENT_ID: "你的 Client ID"
    OSU_CLIENT_SECRET: "你的 Client Secret"
```

本地 PNG/GIF 的 OSU + OSR、或本地 MP4 的完整 OSZ + OSR 匹配齐全时无需认证。认证配置不参与输出目录 hash，也不会写入输出目录的配置差异文件。回放字体与提示音已内嵌，许可见 [第三方声明](../../docs/THIRD_PARTY_NOTICES.md)。

## 输出格式与选段

| 格式 | 默认形态 |
| --- | --- |
| Standard PNG | 5 行、每行 8 帧的快照，每行可指定起点 |
| Taiko PNG | 按游玩顺序排成多行，显示节拍线、BPM 与 SV |
| Catch PNG | 按谱面进度排成多列 |
| Mania PNG | 按键道绘制，长谱自动分列，显示 BPM 与 SV |
| GIF | Standard/Catch 为 2×2 网格，Taiko 为 4 行，Mania 为 4 列 |
| MP4 | 四模式单画面视频，H.264 + AAC，支持 MP3、OGG、WAV 音源 |

数值时间点使用游戏时间：转谱后目标模式的首个可玩物件为 `0:00`，不是音轨绝对时间。`preview` 使用谱面 `PreviewTime`，无效时回退到首个物件。

GIF 与 Standard PNG 的每个时间点分别对应一段或一行。起点不能超过布局容量；未指定满时优先补入 PreviewTime，再随机选取其余片段，避开休息段、长空白和已选片段。短谱空间不足时仅输出可选片段；随机补点每次重新抽取。

MP4 默认从游戏时间 0 开始请求 600 秒；短谱只输出可播放范围，不补空白。请求超过谱尾时整体前移，以尽量保留请求时长。

Taiko、Catch、Mania PNG 不带时间参数时输出整谱；区间渲染必须同时提供一个 `--time-points` 和 `--duration-time`：

```bash
osu-beatmap-preview-cli --bid=738063 --convert=mania --fmt=png --time-points=60 --duration-time=30
```

### MP4 音画

背景图默认开启，暗化 70%；背景视频和故事板默认关闭。打击音默认开启、音量 100%，谱面自带音效优先；缺少样本时回退内嵌皮肤，无法读取的音效按静音处理。Standard/Catch/Mania 使用 Argon Pro (2022)，Taiko 使用 Classic (2013)。

Windows 自动尝试 NVENC 或 AMF，失败时回退 CPU OpenH264；其他平台使用 CPU 编码。Windows 可设 `OSU_PREVIEW_NO_GPU=1` 强制 CPU 编码。该选择影响视频编码，CLI 的画面始终使用 CPU 绘制。

## Mod 支持

下表按 **PNG → GIF/MP4 → 实时预览** 累积支持，后两列只列新增 Mod。

| 模式 | PNG | GIF / MP4（新增） | 实时预览（新增） |
| --- | --- | --- | --- |
| Standard | Easy、Hard-Rock、Hidden、Flashlight、Autoplay、Difficulty-Adjust、Traceable、Mirror | Double-Time、Half-Time、Nightcore、Daycore | Classic、Relax、Spun-Out、Autopilot、Alternate、Single-Tap |
| Taiko | Easy、Hard-Rock、Swap | Hidden、Flashlight、Autoplay、Constant-Speed、Double-Time、Half-Time、Nightcore、Daycore | 无新增 |
| Catch | Easy、Hard-Rock | Hidden、Flashlight、Autoplay、Double-Time、Half-Time、Nightcore、Daycore | 无新增 |
| Mania | One-Key 至 Ten-Keys、Dual-Stages、Invert、Hold-Off、Mirror | Hidden、Flashlight、Constant-Speed、Double-Time、Half-Time、Nightcore、Daycore | 无新增 |

CLI 普通导出不支持“实时预览新增”列的 Mod；回放导出使用回放文件中的 Mod。

### 全名与缩写对照

全名和缩写不区分大小写，使用相同参数语法；全名中的空格写成短横杠，例如 `--mod=hard-rock` 等同于 `--mod=hr`。

| 全名（传入形式） | 缩写 |
| --- | --- |
| Easy | EZ |
| Hard-Rock | HR |
| Hidden | HD |
| Flashlight | FL |
| Autoplay | AT |
| Difficulty-Adjust | DA（需附加难度参数） |
| Traceable | TC |
| Double-Time | DT |
| Half-Time | HT |
| Nightcore | NC |
| Daycore | DC |
| Classic | CL |
| Relax | RX |
| Spun-Out | SO |
| Autopilot | AP |
| Alternate | AL |
| Single-Tap | SG |
| Mirror | MR |
| Swap | SW |
| Constant-Speed | CS |
| Dual-Stages | DS |
| Invert | IN |
| Hold-Off | HO |
| One-Key | 1K |
| Two-Keys | 2K |
| Three-Keys | 3K |
| Four-Keys | 4K |
| Five-Keys | 5K |
| Six-Keys | 6K |
| Seven-Keys | 7K |
| Eight-Keys | 8K |
| Nine-Keys | 9K |
| Ten-Keys | 10K |

Mod 的效果与冲突规则遵循游戏。

- `DA` 目前仅实现 Standard，参数支持 `cs`、`ar`、`od`、`hp`，例如 `--mod=dacs5ar9.5`。
- `AT`（Autoplay）用于显示 Standard 的自动光标、Taiko 的自动击打按键与击飞 Note，或 Catch 自动移动的接盘，普通谱面预览默认隐藏；本项目仅提供预览，不涉及实际游玩。
- 变速 Mod 可指定倍率，例如 `--mod=dt1.25`、`--mod=nc1.4`；加速范围为 1.01–2.00 倍，减速范围为 0.50–0.99 倍。

## 配置

默认值来自 [共享配置](../../assets/shared_config.yml)（`render`、`skin`）与 [CLI 配置](assets/cli_config.yml)（`paths`、`download`、`timeout`、`advance`）。通常只需填写要覆盖的字段：

```text
内嵌默认值 < 可执行文件同目录的 config.yml < --config
```

对象递归合并，数组与标量整体替换。未知字段、非对象顶层或无法转换的值会报错；不存在的默认 `config.yml` 会跳过，文件存在但无效则报错。

```bash
osu-beatmap-preview-cli --bid=738063 --config=config.yml
osu-beatmap-preview-cli --bid=738063 --config='{"render":{"standard":{"gif":{"structure":{"ROW_COUNT":1}}}}}'
```

例如开启 Standard MP4 的背景视频与故事板、调整打击音量，并延长 MP4 超时：

```yaml
render:
  standard:
    mp4:
      style:
        ENABLE_BACKGROUND_IMAGE: true
        ENABLE_BACKGROUND_VIDEO: true
        ENABLE_STORYBOARD: true
        BACKGROUND_DIM: 0.7
        ENABLE_HITSOUND: true
        ENABLE_BEATMAP_HITSOUND: true
        HITSOUND_VOLUME: 50
timeout:
  PNG_TIMEOUT: 300
  GIF_TIMEOUT: 300
  MP4_TIMEOUT: 900
```

各模式、各格式分别配置缩放、输出参数及可用的拼图密度。`--scale` 覆盖当前模式和输出格式的 `SCALE`。超时单位为秒，必须为正整数，覆盖整次请求的下载、解析、缓存检查、渲染、编码与落盘。

普通在线谱面按当前模式的 `ENABLE_BACKGROUND_VIDEO` 选择完整带视频包或 novideo 包，两者分别缓存为 `<set_id>-video.osz` / `<set_id>-novideo.osz`；回放 MP4 使用同一背景视频开关选择包。下载大小上限 `download.osz.MAX_OSZ_BYTES` 默认为 256 MiB。

### 路径与缓存

| 内容 | 默认位置 |
| --- | --- |
| 输出 | `<临时目录>/osu-beatmap-preview/outputs/` |
| OSU 下载缓存 | `<临时目录>/osu-beatmap-preview/osu-download-cache/<bid>.osu` |
| OSZ 与媒体缓存 | `<临时目录>/osu-beatmap-preview/osz-download-cache/` |
| 在线 OSR 缓存 | `<临时目录>/osu-beatmap-preview/osr-download-cache/<成绩ID>.osr` |
| 日志 | `<临时目录>/osu-beatmap-preview/logs/` |
| 自动读取的配置 | 可执行文件同目录的 `config.yml` |

路径由 `paths` 配置，`%TEMP%` 在各平台均展开为系统临时目录；相对 `CONFIG_DIR` 以可执行文件目录为基准。

默认配置直接输出到 `OUTPUT_DIR`；非默认有效配置使用 `OUTPUT_DIR/<6位配置hash>/`，并保存排除认证信息的差异 `config.yml`。只有配置项影响 hash，命令行参数不参与；`--output-dir` 指定本次根目录。

产物名包含模式、谱面标识、Mod、转谱和时间参数。GIF（含回放）与 Standard PNG 将最终选段的绝对毫秒写入文件名，只有选段相同才命中对应缓存；指定全部起点可稳定复用，随机补点可能生成新文件。本地双文件输入任一更新都会使输出缓存失效；输出缓存还校验编译时注入的构建时间。

下载与输出缓存不自动清理，可按需手动删除。输出采用原子写入，完整成功后才替换目标文件。

## 结果与日志

参数解析成功后的请求结果以 JSON 写入 stdout，诊断写入 stderr：

```json
{
  "status": "success",
  "msg": "preview generated successfully for bid 738063",
  "preview-img": "/absolute/path/to/standard_738063.gif",
  "beatmap-info": {
    "meta-data": { "title": "...", "artist": "..." },
    "difficulty": { "circle-size": "...", "approach-rate": "..." }
  }
}
```

`preview-img` 始终为绝对路径，扩展名与实际格式一致。

| 退出码 | 含义 |
| --- | --- |
| `0` | 渲染成功，或执行帮助/版本命令 |
| `1` | 请求失败，如配置、下载、解析、校验、渲染或编码错误 |
| `2` | 命令行参数错误，仅向 stderr 输出错误与用法 |

日志默认开启：`progress.log` 记录阶段事件与最终选段，`render.log` 以 NDJSON 记录请求状态、缓存与耗时，`selected_times_ms` 对应产物名中的选段。日志写入失败不影响渲染，可用 `--no-log` 关闭。在线成绩定位每次重新查询，`--no-cache` 同时跳过 OSU、OSZ、OSR 与产物缓存。

## 从源码构建

需要稳定版 Rust 和 C/C++ 编译环境，在仓库根目录运行：

```bash
cargo build --release -p osu-beatmap-preview-cli
cargo test -p osu-beatmap-preview-cli
```

产物为 `target/release/osu-beatmap-preview-cli`（Windows 加 `.exe`）。性能样本见 [批量渲染报告](../../docs/report.md)。

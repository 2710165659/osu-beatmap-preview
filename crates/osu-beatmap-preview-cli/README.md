# CLI 使用说明（osu-beatmap-preview-cli）

[返回项目首页](../../README.md) | [WASM 使用说明](../osu-beatmap-preview-wasm/README.md) | [架构说明](../../docs/architecture.md)

命令行为单文件可执行程序，把一个 Beatmap ID 渲染成 PNG 静态图、GIF 动图或带原曲音频的 H.264 MP4，支持 osu!standard、osu!taiko、osu!catch、osu!mania 四种模式。CLI 始终使用 CPU 导出路径，不包含 WGPU 绘制和实时预览。

## 职责边界

| 本 crate 负责 | 本 crate 不负责 |
| --- | --- |
| 命令行参数解析、校验和 `--config` 合并 | 谱面解析、Mod 解析、转谱和时间轴计算（由 `osu-beatmap-preview-core` 提供） |
| 下载 `.osu` 与 OSZ、管理下载缓存和输出缓存 | 四模式单帧与静态场景绘制（由 core 的 CPU 渲染完成） |
| 日志（`progress.log`、`render.log`）与 stdout JSON 结果 | 窗口、输入、音频播放和实时预览（见 [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)） |
| GIF/MP4 的时间序列选段、布局组装、帧并行调度 | WGPU/GPU 绘制与 `SurfaceRenderer`、`OffscreenRenderer`（由 `osu-beatmap-preview-renderer` 提供） |
| 媒体编码（GIF、H.264、AAC）和原子文件输出 | 正式 GUI 或移动端应用；CLI 只做一次性导出 |

因此 CLI 不依赖 renderer crate，下载和编码都在本 crate 内完成，输出结果只通过文件路径和 stdout JSON 交给调用者。

## 获取与运行

从 [Releases](https://github.com/2710165659/osu-beatmap-preview/releases) 下载对应平台的 CLI 产物，文件名统一以 `-cli` 结尾：

| 平台 | CLI 产物 |
| --- | --- |
| Windows x64 | `osu-beatmap-preview-windows-amd64-cli.exe` |
| Linux x64 | `osu-beatmap-preview-linux-amd64-cli` |
| macOS Intel | `osu-beatmap-preview-macos-amd64-cli` |
| macOS Apple Silicon | `osu-beatmap-preview-macos-arm64-cli` |

Linux 和 macOS 首次运行前需要添加执行权限：

```bash
chmod +x ./osu-beatmap-preview-*-cli
```

随后使用下载文件的实际名称运行，例如：

```bash
# Linux x64
./osu-beatmap-preview-linux-amd64-cli --bid=738063

# macOS Apple Silicon
./osu-beatmap-preview-macos-arm64-cli --bid=738063
```

也可以按[从源码构建](#从源码构建)自行编译。

## 快速开始

最少只需提供数字格式的 Beatmap ID：

```bash
osu-beatmap-preview-cli --bid=738063
```

以下示例假定已将程序重命名为 `osu-beatmap-preview-cli` 并加入 `PATH`。未指定 `--fmt` 时，Standard 默认输出 GIF，Taiko、Catch 和 Mania 默认输出 PNG。成功后，程序会向 stdout 输出 JSON，其中 `preview-img` 是生成文件的绝对路径。

### 常用示例

```bash
# 明确输出格式
osu-beatmap-preview-cli --bid=738063 --fmt=png

# 使用命令行覆盖输出倍率（0.5 倍、2 倍等）
osu-beatmap-preview-cli --bid=738063 --scale=2

# 将本次结果输出到指定目录
osu-beatmap-preview-cli --bid=738063 --output-dir=C:/path/to/outputs

# 覆盖 GIF/MP4 输出帧率
osu-beatmap-preview-cli --bid=738063 --fmt=gif --fps=30

# 将 Standard 谱面转为 Mania 并输出 GIF
osu-beatmap-preview-cli --bid=738063 --convert=mania --fmt=gif

# 转谱后使用 4K 和 1.25 倍 DT
osu-beatmap-preview-cli --bid=738063 --convert=mania --mod=4k --mod=dt1.25 --fmt=gif

# 组合多个 Mod；每个 Mod 都要单独传入
osu-beatmap-preview-cli --bid=738063 --mod=hd --mod=hr

# 从谱面的 PreviewTime 开始输出 30 秒 MP4
osu-beatmap-preview-cli --bid=738063 --fmt=mp4 --time-points=preview --duration-time=30

# 为 GIF 指定四个片段起点，每个时间点渲染 6 秒
osu-beatmap-preview-cli --bid=738063 --fmt=gif --time-points=5 --time-points=10 --time-points=15 --time-points=20 --duration-time=6

# 只渲染 Mania 谱面 1:00 起 30 秒的区间段 PNG
osu-beatmap-preview-cli --bid=738063 --convert=mania --fmt=png --time-points=60 --duration-time=30

# 跳过下载缓存和输出缓存，同时关闭日志
osu-beatmap-preview-cli --bid=738063 --no-cache --no-log

# 预览本地 .osu 文件（只支持 PNG / GIF，不需要 --bid）
osu-beatmap-preview-cli --osu-file=C:/maps/local-test.osu --fmt=png

# 预览本地 .osz 谱面包：--bid 指定压缩包内的难度（按 .osu 的 BeatmapID 匹配），支持 MP4
osu-beatmap-preview-cli --osz-file=C:/maps/local-pack.osz --bid=738063 --fmt=mp4
```

Windows PowerShell 中，如果程序位于当前目录，需要使用 `.\osu-beatmap-preview-windows-amd64-cli.exe` 或重命名后的实际文件名调用。

## 命令行参数

```text
osu-beatmap-preview-cli [--type=osu|osr] [--osr-file=<PATH> | --score-id=<ID> | --user=<NAME>|--user-id=<ID> --bid=<BID> [--score-select=best|recent]] [--bid=<BID>] [--osu-file=<PATH>] [--osz-file=<PATH>] [--convert=mania|ctb|taiko|standard] [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] [--fps=<1-60>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>] [--scale=<POSITIVE_NUMBER>] [--output-dir=<DIR>] [--version] [--help]
```

| 参数 | 说明 |
| --- | --- |
| `--type` | `osu`（默认）预览谱面，`osr` 预览原生 Standard 回放；回放支持范围见下文。 |
| `--osr-file` | 本地 OSR 来源，与 `--score-id`、玩家查询互斥；需要 `--type=osr`。 |
| `--score-id` | 下载指定统一成绩 ID 的 OSR；无需额外提供谱面 ID。 |
| `--user`、`--user-id` | 玩家名或玩家数字 ID（二选一），搭配 `--type=osr --bid=<单张谱面ID>`。数字用户名使用 `--user`。 |
| `--score-select` | 仅用于玩家查询：`best`（默认，官方个人最高分）或 `recent`（官方近期记录内的最新一次，包含失败记录）；查不到不回退。 |
| `--bid` | 纯数字的 Beatmap ID。|
| `--osu-file` | 本地 `.osu` 谱面路径；`--bid` 可省略，提供时必须与谱面的 `BeatmapID` 相同。 |
| `--osz-file` | 本地 `.osz` 谱面包路径；可独立使用，也可为外部 `.osu` 提供媒体资源。 |
| `--convert` | 目标模式：`mania`、`ctb`、`taiko`、`standard` 或 `std`。只有 Standard 谱面能转换到其他模式；目标与原模式相同时按不转谱处理。 |
| `--fmt` | 输出格式：`png`、`gif` 或 `mp4`。省略时，Standard 使用 GIF，其他模式使用 PNG。 |
| `--mod` | 单个 Mod，接受缩写或全名，全名中的空格写成短横杠，如 `hr` / `hard-rock`。组合时重复传入；参数不区分大小写，仅允许当前模式 PNG 或 GIF/MP4 支持的 Mod。 |
| `--time-points` | 游戏时间点，单位为秒，也可传 `preview`。GIF 和 Standard PNG 可重复传入，MP4 与 Taiko/Catch/Mania PNG 最多传入一次。 |
| `--duration-time` | GIF 每个时间点、MP4 的输出时长，或 Taiko/Catch/Mania PNG 的区间段时长，单位为秒；MP4 默认 `600`；区间段 PNG 必须与 `--time-points` 成对给出。 |
| `--fps` | GIF 或 MP4 输出帧率，必须为 `1` 至 `60` 的整数。省略时使用对应模式和格式配置中的帧率。 |
| `--no-cache` | 跳过 `.osu`、OSZ、在线 OSR 和输出文件缓存，强制重新下载和渲染。 |
| `--no-log` | 关闭文件日志。 |
| `--config` | 配置文件路径，或内联 JSON/YAML 对象。只能传入一次。 |
| `--scale` | 本次输出缩放倍率。 |
| `--output-dir` | 指定本次请求的输出根目录。 |
| `--version` | 向 stdout 打印版本号后退出，退出码为 `0`。 |
| `--help`、`-h` | 向 stdout 打印用法后退出，退出码为 `0`。 |

### OSR 回放

```powershell
# 三种来源互斥，可附带匹配的 .osu/.osz。
osu-beatmap-preview-cli --type=osr --osr-file=C:/maps/play.osr --osz-file=C:/maps/map.osz
osu-beatmap-preview-cli --type=osr --score-id=6086051920
osu-beatmap-preview-cli --type=osr --user="玩家名" --bid=4903207
osu-beatmap-preview-cli --type=osr --user-id=123 --bid=4903207 --score-select=recent
```

OSR 默认输出 MP4，只允许 GIF/MP4，拒绝 `--convert`、用户 `--mod`（包括 NM）和 PNG；`--bid` 仅用于玩家查询，不能附带在本地回放或成绩 ID 来源上。可选 `.osu/.osz` 按回放谱面原始 MD5 绑定；缺少资源时查询官方 API 并下载，失败报错。官方查询与在线回放下载需要 `OSU_API_TOKEN` 或 `OSU_CLIENT_ID` 与 `OSU_CLIENT_SECRET` 环境变量；本地 OSR 搭配匹配的完整 OSZ 时无需认证。在线来源直接使用成绩附带的谱面位置下载包，仍以回放 MD5 校验。

`best` 是官方按分数选出的个人最高成绩，并非最高 PP；`recent` 最多扫描近期 1000 条成绩，按结束时间选取指定谱面的最新一次。选中成绩无回放、规则不兼容、谱面版本不匹配或下载失败时明确报错，不换成其它成绩。回放文件缓存于 `osr-download-cache/<成绩ID>.osr`，`--no-cache` 可强制重下；成绩查询不使用该文件缓存。

原生 Standard 不限制客户端或分数版本，支持 tachyon、历史 lazer 与 stable；stable 自动启用 Classic，lazer 显式 Classic 使用同一策略，保留录制设置。历史成绩按当前官方标准化规则重播，原记录差异仅作诊断。回放读取录制的 Mod，支持下表 Standard 实时预览列的累计集合；未实现 Mod 和未知 settings 仍明确拒绝。回放提前结束显示失败并冻结成绩，谱面背景与音乐继续。完整用法、成绩差异处理、内嵌字体和验收记录见 [OSR 回放预览](../../docs/osr-preview.md)。

### 普通预览的本地谱面文件（`--osu-file` / `--osz-file`）

提供本地文件时不再联网。两个路径分别要求 `.osu` / `.osz` 扩展名，大小写不敏感；旧参数 `--input-file` 已移除。

- **仅 `--osu-file`**：`--bid` 可省略；提供时必须与 `[Metadata] BeatmapID` 数值相同，否则报错。缺少、为 0 或无效的 `BeatmapID` 也不能搭配 `--bid`。没有媒体包，因此只支持 PNG / GIF，不支持 MP4。
- **仅 `--osz-file`**：包内只有一个顶层 `.osu` 时可以省略 `--bid`，自动读取并解析；多个顶层 `.osu` 时必须用 `--bid` 选择难度。提供 `--bid` 后，即使只有一个谱面也必须匹配其 `BeatmapID`；找不到仍报错。支持 PNG / GIF / MP4。
- **两个文件同时提供**：始终以外部 `.osu` 渲染，音频、背景和打击音等资源取自 `.osz`。外部 `.osu` 必须有有效正数 `BeatmapID`，且包内必须存在该 ID 的谱面。`--bid` 可省略，提供时必须与外部谱面的 ID 相同。支持 PNG / GIF / MP4。

压缩包只统计顶层 `.osu` 文件，后缀大小写不敏感；子目录里的谱面不参与选择。没有顶层谱面、唯一谱面无法解析，或指定 ID 找不到时均报错。多个条目即使包含损坏谱面，也必须提供 `--bid`。

省略 `--bid` 时，产物与日志使用谱面的有效 `BeatmapID`；单独 `.osu` 或唯一谱面的 `.osz` 没有有效 ID 时，回退到输入文件名。双文件模式中，任一输入更新都会使输出缓存失效；这些参数不参与输出目录的配置 hash。

```sh
# 只有一个顶层 .osu 的谱面包可以自动选择难度
osu-beatmap-preview-cli --osz-file=C:/maps/single-map.osz --fmt=mp4

# 使用外部编辑过的谱面，并从对应媒体包读取资源
osu-beatmap-preview-cli --osu-file=C:/maps/edited.osu --osz-file=C:/maps/local-pack.osz --fmt=mp4
```

### 时间轴与选段

数值时间点使用游戏时间轴：转谱后目标模式的首个可玩物件是 `0:00`，并非编辑器左下角显示的绝对音轨时间。

- GIF 和 Standard PNG 会把每个 `--time-points` 作为一个分段起点。指定点未占满布局容量时，程序优先补入谱面的 `PreviewTime`，再以确定性方式补齐其他不重叠片段；相同谱面和配置会得到相同选段。
- 时间点数量不能超过当前布局的分段容量。GIF 默认共 4 段（Standard/Catch 为 2 × 2 网格，Taiko 为 4 行，Mania 为 4 列）；Standard PNG 默认有 5 行，因此最多指定 5 个行起点。
- MP4 默认从游戏时间 `0` 开始，请求 600 秒。谱面较短时输出完整可播放范围，不填充到 600 秒；请求区间超过谱面尾部时会整体前移以保留时长。
- `--time-points=preview` 使用 `.osu` 文件中的 `PreviewTime`；缺失或无效时回退到首个物件。
- Taiko、Catch 和 Mania 的 PNG 支持区间段生成：`--time-points` 与 `--duration-time` 必须同时给出，且各最多一个，输出只覆盖 `[起点, 起点 + 时长]` 这一段。两者都缺时保持整谱渲染。
- `--time-points` 适用于 GIF、Standard PNG、MP4 以及 Taiko/Catch/Mania 的区间段 PNG。

## 输出格式

### PNG 静态图

- **Standard**：默认输出 5 行、每行 8 帧的游戏画面快照；每行起点可由 `--time-points` 指定。
- **Taiko**：按游玩顺序排成多行，并绘制节拍线、BPM 与 SV 信息。
- **Catch**：按谱面进度排成多列。
- **Mania**：按键道绘制谱面，长谱面自动拆分为多列，并显示 BPM 与 SV 信息。

Taiko、Catch 和 Mania 可用 `--time-points` 与 `--duration-time` 只渲染谱面的一个区间段（见「时间轴与选段」）；不带时间参数时仍渲染整谱。

布局、颜色、间距和标签等均可通过配置调整。

### GIF 动图

四种模式都会把多个谱面片段组合到同一张动图中。默认布局为：Standard 和 Catch 使用 `2 x 2` 网格，Taiko 使用 4 行，Mania 使用 4 列。每种模式可以独立配置片段数量、片段时长、帧率和时间标签。

### MP4 视频

四种模式均可输出带谱面原始音频的 MP4，支持 MP3、OGG 和 WAV 音源。视频默认读取背景图，并应用 70% 暗化。

谱面带背景视频时可打开 `ENABLE_BACKGROUND_VIDEO`（四模式的 `mp4` 小节各一份，**默认关闭**）把视频合成进背景。

谱面带故事板时可打开 `ENABLE_STORYBOARD`（四模式的 `mp4` 小节各一份，**默认关闭**）把故事板合成进视频。

MP4 默认还会把打击音（hit sound）混入音轨，音量 100%：

- Standard / Catch / Mania 使用 argon pro (2022) 音效，Taiko 使用 osu! "classic" (2013) 音效；
- 转盘的旋转音会按 autoplay 转速播放；
- **谱面自带的自定义打击音优先**；
- 某个音效文件无法读取时按静音处理，不会中断导出；
- 各模式可分别用 `ENABLE_HITSOUND`、`ENABLE_BEATMAP_HITSOUND` 与 `HITSOUND_VOLUME` 控制（见下方配置示例）。

Windows 会自动选择可用的 NVENC 或 AMF 硬件编码器，失败时回退到 CPU OpenH264。设置环境变量 `OSU_PREVIEW_NO_GPU=1` 可以强制使用 CPU 编码，便于兼容性检查或性能对比。

## Mod 支持

支持按同一行从左到右累积：**PNG → GIF/MP4 → 实时预览**。后两列只列新增 Mod，包含前面所有列的能力；「无新增」仍继承前面的支持。

| 模式 | PNG | GIF / MP4（新增） | 实时预览（新增） |
| --- | --- | --- | --- |
| Standard | Easy、Hard-Rock、Hidden、Flashlight、Autoplay、Difficulty-Adjust、Traceable | Double-Time、Half-Time、Nightcore、Daycore | Classic、Relax、Spun-Out、Autopilot、Alternate、Single-Tap、Mirror |
| Taiko | Easy、Hard-Rock、Swap | Hidden、Flashlight、Constant-Speed、Double-Time、Half-Time、Nightcore、Daycore | 无新增 |
| Catch | Easy、Hard-Rock | Hidden、Flashlight、Autoplay、Double-Time、Half-Time、Nightcore、Daycore | 无新增 |
| Mania | One-Key 至 Ten-Keys、Dual-Stages、Invert、Hold-Off | Hidden、Flashlight、Constant-Speed、Double-Time、Half-Time、Nightcore、Daycore | 无新增 |

CLI 只有 PNG、GIF 和 MP4 导出，不能用 `--mod` 传入实时列新增的 Mod（含 Classic），也没有 realtime 输出格式。Classic 只支持 Standard 实时预览，其他模式拒绝。OSR 内录制的 Mod 由回放解码器读取，因此带 Classic 等 Mod 的回放仍可导出 GIF/MP4；用户不能通过 `--mod` 修改回放。

### 全名与缩写对照

全名和缩写不区分大小写，使用相同参数语法。全名中的空格用短横杠：`--mod=hard-rock` 等同于 `--mod=hr`；`--mod=double-time1.25` 等同于 `--mod=dt1.25`；`--mod=difficulty-adjustCS5AR9.5` 等同于 `--mod=dacs5ar9.5`。缩写和全名混用仍按同一个 Mod 检测重复。

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

主要规则如下：

- `DT`、`HT`、`NC`、`DC` 四者互斥。加速类默认 `1.5x`、可设为 `1.01` 至 `2.00`；减速类默认 `0.75x`、可设为 `0.50` 至 `0.99`，例如 `--mod=dt1.25`、`--mod=nc1.4`。
- `EZ` 与 `HR`、`TC` 与 `HD`、`IN` 与 `HO` 分别互斥。
- Standard 实时预览接受 Classic、Relax、Spun-Out、Autopilot、Alternate、Single-Tap、Mirror。普通预览自动演示物件，没有玩家输入，因此 Relax、Alternate、Single-Tap 和 Classic 不额外改变自动命中画面；Autopilot 显示自动光标，Spun-Out 自动旋转转盘，Mirror 默认水平翻转物件与滑条。
- 输入类 Mod 沿用官方互斥关系：Relax 与 Autopilot、Alternate、Single-Tap、Autoplay 互斥；Spun-Out 与 Autopilot、Autoplay 互斥；Alternate 与 Single-Tap、Autoplay 互斥；Single-Tap 与 Autoplay 互斥；Autopilot 与 Autoplay 互斥；Mirror 与 Hard-Rock 互斥。
- Standard 回放按录制设置应用 Mod，保留游玩光标坐标。旧版 Relax 回放重建自动按键，现代 Relax 使用录制输入；Alternate / Single-Tap 限制输入动作，休息段重置限制；Mirror 支持录制的水平、垂直及双轴设置。分数按当前官方 `OsuScoreMultiplierCalculatorV2` 重算，其中 Relax / Autopilot 为 `0.1`，Spun-Out 为 `0.95`，Alternate / Single-Tap / Mirror 为 `1.0`，再与其他 Mod 倍率相乘。
- Mania 的 `HD` 与 `FL` 互斥，其余三种模式允许 `HD+FL`。
- 除 Catch HD 外，`HD`、`FL` 与游戏内 Autoplay 效果一致。
- Catch HD 在 Kiai 区段持续保留淡淡的物件轮廓，与游戏内效果不一致。
- `DA` 仅适用于 Standard，不能与 `EZ` 或 `HR` 同时使用。格式为 `da<参数><值>`，参数支持 `cs`、`ar`、`od`、`hp`，例如 `--mod=dacs5ar9.5`。
- `1K` 至 `10K` 互斥；`DS` 和键数 Mod 只会在 Standard 转 Mania 时改变转谱结果。
- 重复的 Mod 或不受当前模式、格式支持的 Mod 会直接报错，不会静默忽略。
- `AT`（Autoplay）支持 Standard 与 Catch；Standard 用 `--mod AT` 显示自动光标，轨迹参考 lazer 的物件间缓动、滑条跟随与转盘旋转。实时预览、PNG、GIF 和 MP4 共用轨迹缓存，不增加判定或计分功能。
- Catch `AT` 支持实时预览、GIF 和 MP4：简洁盘子按 lazer 自动回放规则移动，Dash 留下短拖尾，hyperdash 盘子和拖尾变红；香蕉雨使用现有推荐路线。Catch `FL` 的光圈跟随同一盘子路径，仅 FL 时不显示盘子，开启 FL 不改变盘子运动。
- Standard 光标采用 Argon Pro 的粉红渐变环、白色中心与点击缩放；`FL` 消费同一份光标轨迹，按游戏默认 120ms 延迟跟随。只开启 `FL` 时计算轨迹但不绘制光标，也不分配光标精灵。

### 变速与音高

四种倍速 Mod 的音乐处理与游戏一致（`ModDoubleTime` / `ModHalfTime` / `ModNightcore` / `ModDaycore`）：

- `DT` / `HT` 是**保调**变速（游戏里 `AdjustPitch` 默认关，等价 `AdjustableProperty.Tempo`）：音乐按倍率变快/变慢，音高不变；
- `NC` / `DC` 在变速的同时把音乐音高**固定**为加速类的 `1.5x` / 减速类的 `0.75x`（游戏里 `Frequency` 取 `SpeedChange.Default`），与自定义倍速无关：`--mod=nc2` 是 2 倍速 + 1.5 倍音高；
- 打击音（含 NC 鼓点）与游戏 `ModRateAdjust.ApplyToSample` 一致，按倍速重采样：既变快也变调；
- `NC` 还会叠加节拍鼓点：每半拍触发一次，4/4 为 kick(1、3 拍) / clap(2、4 拍) / hat(反拍)，3/4 为 kick(每 3 拍) / clap(第 2 拍反拍) / hat(反拍)，每 4 小节在段首加一声 finish，`OmitFirstBarLine` 会让整条网格后移半拍；`SliderTickRate` 不是偶数时不放 hat（与游戏一致）。`DC` 没有鼓点；
- 鼓点属于 Mod，不受 `ENABLE_HITSOUND` 影响（关闭打击音后仍会播放，只受 `HITSOUND_VOLUME` 控制）；`ENABLE_BEATMAP_HITSOUND` 仍决定是否采用谱面包里的同名 `nightcore-*.ogg`。

## 配置

默认配置由两份源文件合并而成：

- [assets/shared_config.yml](../../assets/shared_config.yml)：共享配置，包含 `render`、`skin`，被 `osu-beatmap-preview-core` 和 `osu-beatmap-preview-cli` 使用。
- [assets/cli_config.yml](assets/cli_config.yml)：CLI 专用配置，包含 `paths`、`download`、`timeout`、`advance`，仅由 `osu-beatmap-preview-cli` 使用。

CLI 启动时会把两者合并为内嵌默认配置；自定义配置通常只需写出要覆盖的字段。

配置按以下优先级递归合并：

```text
内置默认值 < 可执行文件同目录的 config.yml < --config
```

映射会递归合并，数组和标量会整体替换。未知字段、非对象的顶层值或无法转换为目标类型的值会导致启动失败；数字和布尔值也接受可安全转换的字符串形式。`config.yml` 不存在时继续使用默认值，但文件存在且内容无效时会报错。

`--config` 可以指向 JSON/YAML 文件，也可以直接接收内联对象：

```bash
# 配置文件
osu-beatmap-preview-cli --bid=738063 --config=C:/path/to/config.yml

# 内联 JSON
osu-beatmap-preview-cli --bid=738063 --config='{"render":{"standard":{"gif":{"structure":{"ROW_COUNT":1}}}}}'

# 内联 YAML
osu-beatmap-preview-cli --bid=738063 --config='{render: {standard: {gif: {structure: {ROW_COUNT: 1}}}}}'
```

以下示例关闭 Standard MP4 背景图、调整暗化程度、为 Standard 打开背景视频与故事板、让 Taiko 忽略谱面自带音效，关闭 Mania 打击音，同时分别设置三种格式的整次请求超时：

```yaml
render:
  standard:
    mp4:
      style:
        ENABLE_BACKGROUND_IMAGE: false
        ENABLE_BACKGROUND_VIDEO: true
        ENABLE_STORYBOARD: true
        BACKGROUND_DIM: 0.5
        ENABLE_HITSOUND: true
        ENABLE_BEATMAP_HITSOUND: true
        HITSOUND_VOLUME: 50
  taiko:
    mp4:
      style:
        ENABLE_BEATMAP_HITSOUND: false
  mania:
    mp4:
      style:
        ENABLE_HITSOUND: false
timeout:
  PNG_TIMEOUT: 300
  GIF_TIMEOUT: 300
  MP4_TIMEOUT: 900
```

在线下载的谱面包跟随**当前模式的 `ENABLE_BACKGROUND_VIDEO`**：开启背景视频时下载带视频的完整包（背景视频功能需要素材），关闭时下载 novideo 去视频包（明显更省流量）。两种包在缓存里分开存放（`<set_id>-video.osz` / `<set_id>-novideo.osz`），切换开关会自动改用对应缓存。包大小上限 `download.osz.MAX_OSZ_BYTES` 默认 256MB。

超时单位为秒且必须是正整数。计时从请求入口开始，覆盖下载、解析、转谱、缓存检查、渲染、音频处理、编码和落盘。

### 默认路径

| 内容 | 默认位置 |
| --- | --- |
| 输出文件 | `<临时目录>/osu-beatmap-preview/outputs/` |
| `.osu` 缓存 | `<临时目录>/osu-beatmap-preview/osu-download-cache/<bid>.osu` |
| OSZ 与音频缓存 | `<临时目录>/osu-beatmap-preview/osz-download-cache/` |
| osu.direct 优选 IP 缓存 | `<临时目录>/osu-beatmap-preview/osz-download-cache/osu-direct-preferred-ip.json` |
| 自动配置文件 | `<可执行文件所在目录>/config.yml` |
| 日志 | `<临时目录>/osu-beatmap-preview/logs/` |

路径由默认配置中的 `paths` 控制。`%TEMP%` 在所有平台都展开为系统临时目录。程序会单独解析 `CONFIG_DIR`：相对路径始终以可执行文件所在目录为基准，不受启动命令时所在目录影响。

默认配置的输出文件直接写入 `OUTPUT_DIR`。只要最终有效配置与默认值不同，程序就会根据差异配置计算稳定的 6 位哈希，并改用 `OUTPUT_DIR/<config-hash>/`；该目录内会写入只包含非默认字段的 `config.yml`。因此等价的配置内容会复用同一输出缓存。所有 CLI 参数均不影响 output 目录下的配置哈希。

## 程序输出、缓存与日志

### stdout JSON

命令行参数解析成功后，配置、下载、谱面解析、校验或渲染阶段的成功与失败结果都会以 JSON 输出到 stdout，便于脚本调用：

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

`preview-img` 始终是绝对路径，扩展名与实际输出格式一致。诊断信息写入 stderr，不会混入 JSON。退出码含义如下：

| 退出码 | 含义 |
| --- | --- |
| `0` | 渲染成功，或执行了 `--version` / `--help` |
| `1` | 参数解析成功后的请求错误（配置、下载、解析、校验、渲染或编码失败） |
| `2` | 命令行参数错误；只向 stderr 输出错误与用法，不输出 JSON |

### 缓存与日志

- 输出文件名包含目标模式、Beatmap ID、转谱标记、Mod 和显式时间参数，例如 `mania_738063_convert_4k_fps30.gif`；显式倍率会以 `@2x` 形式追加在扩展名之前。
- 日志默认开启。`progress.log` 记录可实时跟踪的阶段事件，`render.log` 以 NDJSON 记录每次请求的状态、谱面信息、缓存命中和耗时。
- 日志写入失败只会在 stderr 给出提示，不影响渲染结果。使用 `--no-log` 可以关闭日志。
- 下载与输出缓存不会自动清理；空间占用过大时可以手动删除对应临时目录。
- 输出采用原子写入，只有完整渲染成功后才替换目标文件；中断渲染不会留下可被误判为有效缓存的半成品。

## 从源码构建

需要稳定版 Rust 工具链和可用的 C/C++ 编译环境。安装 Rust：<https://rustup.rs>

```bash
git clone https://github.com/2710165659/osu-beatmap-preview.git
cd osu-beatmap-preview

# 默认成员就是 CLI
cargo build --release

# 也可以显式指定包
cargo build --release --package osu-beatmap-preview-cli
```

构建产物位于：

```text
target/release/osu-beatmap-preview-cli      # Linux / macOS
target/release/osu-beatmap-preview-cli.exe  # Windows
```

运行 CLI 的单元测试：

```bash
cargo test --package osu-beatmap-preview-cli
```

## 相关文档

- [WASM 使用说明](../osu-beatmap-preview-wasm/README.md)：浏览器 WebGPU 实时渲染的构建方式与 JavaScript API。
- [架构说明](../../docs/architecture.md)：core 与 CLI 的职责边界、配置来源、场景与后端。
- [批量渲染报告](../../docs/report.md)：性能与资源占用数据。

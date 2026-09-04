# osu! Beatmap Preview

[中文](README.md) | [English](docs/README.en.md)

一个快速、可独立运行的 osu! 谱面预览渲染器，支持 Standard、Taiko、Catch 和 Mania 四种模式，可输出 PNG 静态图、GIF 动图与带谱面原始音频的 MP4 视频。

![四种模式的渲染效果](docs/total.png)

## 功能亮点

- **四模式渲染**：支持 osu!standard、osu!taiko、osu!catch 与 osu!mania。
- **三种输出格式**：生成 PNG 谱面概览、GIF 分段预览或带音频的 H.264 MP4 视频。
- **转谱与 Mod**：支持从 Standard 转换到 Taiko、Catch、Mania，并可组合常用 Mod 和自定义倍速。
- **单文件运行**：皮肤、字体和编解码组件随程序构建，常规使用无需额外资源文件或 FFmpeg。
- **跨平台**：支持 Windows、Linux 与 macOS；Windows 会依次尝试 NVIDIA NVENC、AMD AMF，均不可用时自动回退到 CPU OpenH264，其他平台使用 CPU 编码。
- **缓存与配置隔离**：缓存下载内容和渲染结果；不同有效配置使用独立输出目录，避免误用旧结果。

性能与资源占用数据见[批量渲染报告](docs/report.md)。

## 获取与运行

可以从 [Releases](https://github.com/2710165659/osu-beatmap-preview/releases) 下载对应平台的可执行文件：

| 平台 | 发布文件 |
| --- | --- |
| Windows x64 | `osu-beatmap-preview-windows-amd64.exe` |
| Linux x64 | `osu-beatmap-preview-linux-amd64` |
| macOS Intel | `osu-beatmap-preview-macos-amd64` |
| macOS Apple Silicon | `osu-beatmap-preview-macos-arm64` |

Linux 和 macOS 首次运行前需要添加执行权限：

```bash
chmod +x ./osu-beatmap-preview-*
```

随后使用下载文件的实际名称运行，例如：

```bash
# Linux x64
./osu-beatmap-preview-linux-amd64 --bid=738063

# macOS Apple Silicon
./osu-beatmap-preview-macos-arm64 --bid=738063
```

macOS 发布文件未经过 Apple 签名或公证。若系统阻止首次启动，可在“系统设置 > 隐私与安全性”中选择仍要打开，或在 Finder 中右键程序并选择“打开”。

也可以按下文的[从源码构建](#从源码构建)自行编译。

## 快速开始

最少只需提供数字格式的 Beatmap ID：

```bash
osu-beatmap-preview --bid=738063
```

以下示例假定已将程序重命名为 `osu-beatmap-preview` 并加入 `PATH`。未指定 `--fmt` 时，Standard 默认输出 GIF，Taiko、Catch 和 Mania 默认输出 PNG。成功后，程序会向 stdout 输出 JSON，其中 `preview-img` 是生成文件的绝对路径。

### 常用示例

```bash
# 明确输出格式
osu-beatmap-preview --bid=738063 --fmt=png

# 使用命令行覆盖输出倍率（0.5 倍、2 倍等）
osu-beatmap-preview --bid=738063 --scale=2

# 将 Standard 谱面转为 Mania 并输出 GIF
osu-beatmap-preview --bid=738063 --convert=mania --fmt=gif

# 转谱后使用 4K 和 1.25 倍 DT
osu-beatmap-preview --bid=738063 --convert=mania --mod=4k --mod=dt1.25 --fmt=gif

# 组合多个 Mod；每个 Mod 都要单独传入
osu-beatmap-preview --bid=738063 --mod=hd --mod=hr

# 从谱面的 PreviewTime 开始输出 30 秒 MP4
osu-beatmap-preview --bid=738063 --fmt=mp4 --time-points=preview --duration-time=30

# 为 GIF 指定四个片段起点，每个时间点渲染 6 秒
osu-beatmap-preview --bid=738063 --fmt=gif --time-points=5 --time-points=10 --time-points=15 --time-points=20 --duration-time=6

# 跳过下载缓存和输出缓存，同时关闭日志
osu-beatmap-preview --bid=738063 --no-cache --no-log
```

Windows PowerShell 中，如果程序位于当前目录，需要使用 `.\osu-beatmap-preview-windows-amd64.exe` 或重命名后的实际文件名调用。

## 命令行参数

```text
osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard] [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>] [--scale=<POSITIVE_NUMBER>]
```

| 参数 | 说明 |
| --- | --- |
| `--bid` | 必填。纯数字的 Beatmap ID。 |
| `--convert` | 目标模式：`mania`、`ctb`、`taiko`、`standard` 或 `std`。只有 Standard 谱面能转换到其他模式；目标与原模式相同时按不转谱处理。 |
| `--fmt` | 输出格式：`png`、`gif` 或 `mp4`。省略时，Standard 使用 GIF，其他模式使用 PNG。 |
| `--mod` | 单个 Mod。组合时重复传入；参数不区分大小写。 |
| `--time-points` | 游戏时间点，单位为秒，也可传 `preview`。GIF 和 Standard PNG 可重复传入，MP4 最多传入一次。 |
| `--duration-time` | GIF 每个时间点或 MP4 的输出时长，单位为秒，必须为有限正数。GIF 未指定时使用对应模式配置的片段时长；MP4 默认 `600`。 |
| `--no-cache` | 跳过 `.osu`、OSZ 和输出文件缓存，强制重新下载和渲染。 |
| `--no-log` | 关闭文件日志。 |
| `--config` | 配置文件路径，或内联 JSON/YAML 对象。只能传入一次。 |
| `--scale` | 本次输出倍率，必须为有限正数。 |
| `--version` | 打印版本号和构建时间后退出。 |
| `--help`、`-h` | 打印用法后退出。 |

### 时间轴与选段

数值时间点使用游戏时间轴：转谱后目标模式的首个可玩物件是 `0:00`，并非编辑器左下角显示的绝对音轨时间。

- GIF 和 Standard PNG 会把每个 `--time-points` 作为一个分段起点。指定点未占满布局容量时，程序优先补入谱面的 `PreviewTime`，再以确定性方式补齐其他不重叠片段；相同谱面和配置会得到相同选段。
- 时间点数量不能超过当前布局的分段容量。默认 GIF 容量为 4；Standard PNG 默认有 5 行，因此最多指定 5 个行起点。
- MP4 默认从游戏时间 `0` 开始，请求 600 秒。谱面较短时输出完整可播放范围，不填充到 600 秒；请求区间超过谱面尾部时会整体前移以保留时长。
- MP4 支持负数起点，早于音频起点的部分输出静音。`--time-points=preview` 使用 `.osu` 文件中的 `PreviewTime`；缺失或无效时回退到首个物件。
- `--duration-time` 适用于 GIF 和 MP4。GIF 会从每个 `--time-points` 时间点分别渲染指定时长；`--time-points` 仅适用于 GIF、Standard PNG 和 MP4。

## 输出格式

### PNG 静态图

- **Standard**：默认输出 5 行、每行 8 帧的游戏画面快照；每行起点可由 `--time-points` 指定。
- **Taiko**：按游玩顺序排成多行，并绘制节拍线、BPM 与 SV 信息。
- **Catch**：按谱面进度排成多列。
- **Mania**：按键道绘制谱面，长谱面自动拆分为多列，并显示 BPM 与 SV 信息。

布局、颜色、间距和标签等均可通过配置调整。

### GIF 动图

四种模式都会把多个谱面片段组合到同一张动图中。默认布局为：Standard 和 Catch 使用 `2 x 2` 网格，Taiko 使用 4 行，Mania 使用 4 列。每种模式可以独立配置片段数量、片段时长、帧率和时间标签。

### MP4 视频

四种模式均可输出带谱面原始音频的 MP4，支持 MP3、OGG 和 WAV 音源。视频默认读取 OSZ 中 `[Events]` 声明的背景图，并按 `BACKGROUND_DIM=0.7` 暗化；可通过配置关闭背景图。

Windows 会自动选择可用的 NVENC 或 AMF 硬件编码器，失败时回退到 CPU OpenH264。设置环境变量 `OSU_PREVIEW_NO_GPU=1` 可以强制使用 CPU 编码，便于兼容性检查或性能对比。

## Mod 支持

| 模式 | GIF / MP4 | PNG |
| --- | --- | --- |
| Standard | `EZ` `HR` `HD` `DA` `TC` `DT` `HT` | `EZ` `HR` `HD` `DA` `TC` |
| Taiko | `EZ` `HR` `SW` `CS` `DT` `HT` | `EZ` `HR` `SW` |
| Catch | `EZ` `HR` `DT` `HT` | `EZ` `HR` |
| Mania | `CS` `DT` `HT` `1K`-`10K` `DS` `IN` `HO` | `1K`-`10K` `DS` `IN` `HO` |

主要规则如下：

- `DT` 与 `HT` 互斥。`DT` 默认 `1.5x`，可设为 `1.01` 至 `2.00`；`HT` 默认 `0.75x`，可设为 `0.50` 至 `0.99`，例如 `--mod=dt1.25`。
- `EZ` 与 `HR`、`TC` 与 `HD`、`IN` 与 `HO` 分别互斥。
- `DA` 仅适用于 Standard，不能与 `EZ` 或 `HR` 同时使用。格式为 `da<参数><值>`，参数支持 `cs`、`ar`、`od`、`hp`，例如 `--mod=dacs5ar9.5`。
- `1K` 至 `10K` 互斥；`DS` 和键数 Mod 只会在 Standard 转 Mania 时改变转谱结果。
- `DT` 和 `HT` 不适用于 PNG；MP4 使用与 GIF 相同的 Mod 支持规则。
- 重复的 Mod 或不受当前模式、格式支持的 Mod 会直接报错，不会静默忽略。

## 配置

完整默认配置及字段说明见 [assets/default_config.yml](assets/default_config.yml)。通常只需在自定义配置中写出要覆盖的字段。

配置按以下优先级递归合并：

```text
内置默认值 < 可执行文件同目录的 config.yml < --config
```

映射会递归合并，数组和标量会整体替换。未知字段、非对象的顶层值或无法转换为目标类型的值会导致启动失败；数字和布尔值也接受可安全转换的字符串形式。`config.yml` 不存在时继续使用默认值，但文件存在且内容无效时会报错。

`--config` 可以指向 JSON/YAML 文件，也可以直接接收内联对象：

```bash
# 配置文件
osu-beatmap-preview --bid=738063 --config=C:/path/to/config.yml

# 内联 JSON
osu-beatmap-preview --bid=738063 --config='{"layout":{"standard":{"gif":{"ROW_COUNT":1}}}}'

# 内联 YAML
osu-beatmap-preview --bid=738063 --config='{layout: {standard: {gif: {ROW_COUNT: 1}}}}'
```

以下示例关闭 Standard MP4 背景图、调整暗化程度，并分别设置三种格式的整次请求超时：

```yaml
layout:
  standard:
    mp4:
      ENABLE_BACKGROUND_IMAGE: false
      BACKGROUND_DIM: 0.5
timeouts:
  render:
    PNG_TIMEOUT: 300
    GIF_TIMEOUT: 300
    MP4_TIMEOUT: 900
```

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

默认配置的输出文件直接写入 `OUTPUT_DIR`。只要最终有效配置与默认值不同，程序就会根据差异配置计算稳定的 6 位哈希，并改用 `OUTPUT_DIR/<config-hash>/`；该目录内会写入只包含非默认字段的 `config.yml`。因此等价的配置内容会复用同一输出缓存。

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

`preview-img` 始终是绝对路径，扩展名与实际输出格式一致。诊断信息写入 stderr，不会混入 JSON。成功退出码为 `0`；参数解析后的请求错误为 `1`；命令行参数错误只写入 stderr、不输出 JSON，退出码为 `2`。`--version` 向 stdout 输出普通文本，`--help` 向 stderr 输出用法，二者均以 `0` 退出。

### 缓存与日志

- 输出文件名包含目标模式、Beatmap ID、转谱标记、Mod 和显式时间参数。未显式传入 MP4 时间参数时，文件名不追加默认时间范围。
- 日志默认开启。`progress.log` 记录可实时跟踪的阶段事件，`render.log` 以 NDJSON 记录每次请求的状态、谱面信息、缓存命中和耗时。
- 日志写入失败只会在 stderr 给出提示，不影响渲染结果。使用 `--no-log` 可以关闭日志。
- 下载与输出缓存不会自动清理；空间占用过大时可以手动删除对应临时目录。
- 输出采用原子写入，只有完整渲染成功后才替换目标文件；中断渲染不会留下可被误判为有效缓存的半成品。

## 从源码构建

需要稳定版 Rust 工具链和可用的 C/C++ 编译环境。安装 Rust：<https://rustup.rs>

```bash
git clone https://github.com/2710165659/osu-beatmap-preview.git
cd osu-beatmap-preview
cargo build --release
```

构建产物位于：

```text
target/release/osu-beatmap-preview       # Linux / macOS
target/release/osu-beatmap-preview.exe   # Windows
```

运行测试：

```bash
cargo test
```

## 许可证

本项目使用 [MIT License](LICENSE)。内嵌 AAC 编码器使用 Fraunhofer FDK-AAC，其许可证不授予专利权，详见[第三方声明](docs/THIRD_PARTY_NOTICES.md)。

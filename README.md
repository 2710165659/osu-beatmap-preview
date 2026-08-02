# osu! Beatmap Preview

[中文](README.md) | [English](docs/README.en.md)

> 一个快速的 osu! 谱面预览工具，支持四种模式（Standard / Taiko / Catch / Mania）的 GIF 动图、PNG 静态图与 MP4 视频渲染。

## 特性

- **单可执行文件**：所有资源在编译时嵌入二进制，运行时无任何外部依赖，即开即用。  
- **跨平台**：原生支持 Windows、Linux 与 macOS。  
- **功能完备**：支持四种游戏模式、MOD、转谱及 SV（变速）功能。  
- **三种输出格式**：GIF 动图、PNG 静态长图，以及包含原始谱面音频的 MP4 视频。  
- **高性能**：视频编码采用 GPU 加速，整体处理流程速度快、内存占用低、输出文件体积小。详见[批量渲染报告](docs/report.txt)。

> 如果这个项目对你有帮助，欢迎点个 ⭐ Star 支持一下～

## 使用

```bash
osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard|std] [--mods=<MODS>] [--fmt=png|gif|mp4] [--time=<T1+T2+...>] [--gif-clip] [--gif-clip-label] [--preview-30s] [--gap=<BPM>] [--log-dir=<DIR>] [--no-log] [--no-cache]
```

参数别名：`--mod` = `--mods`，`--format` = `--fmt`，`--times` = `--time`。

### 参数

| 参数 | 说明 |
| --- | --- |
| `--bid` | 必填，纯数字的 Beatmap ID。 |
| `--convert` | 转谱模式，支持 `mania` / `ctb` / `taiko` / `standard` / `std`。仅 Standard 可用。 |
| `--mods` | Mod 组合，用 `+` 连接，如 `hd+hr`；倍速类可带数值，如 `dt1.25`。 |
| `--fmt` | 输出格式：`gif` / `png` / `mp4`。不填时按模式取默认值。 |
| `--time` | 游戏皮肤时间轴上的时间点或范围，单位秒；目标模式首个可玩物件为 `0:00`，支持负数。|
| `--gif-clip` | 仅 GIF 可用，输出单屏连续 GIF，不显示时间标签。 |
| `--gif-clip-label` | 仅 GIF 可用，和 `--gif-clip` 同类，但会显示时间标签。 |
| `--preview-30s` | 仅 MP4 可用，按 `PreviewTime` 附近渲染约 30 秒实际播放时长。 |
| `--gap` | 仅 Taiko PNG 可用，指定排列间距 BPM。 |
| `--log-dir` | 指定日志目录。 |
| `--no-log` | 关闭日志。 |
| `--no-cache` | 跳过下载缓存与输出缓存，强制重新渲染。 |
| `--version` | 打印版本号与构建时间后退出。 |

> `--time` 与 osu! 游戏内歌曲进度皮肤组件使用同一时间轴：转谱后的目标模式首个可玩物件为 `0:00`，不是编辑器左下角的绝对音轨时间。普通 GIF 和 Standard PNG 可传入 1–4 个时间点；MP4、`--gif-clip` 和 `--gif-clip-label` 必须传入两个时间点表示区间。负数表示首物件之前，建议使用等号形式，例如 `--time=-2+10`；早于音频起点的 MP4 部分输出静音。

### 示例

```bash
# 使用默认参数渲染
osu-beatmap-preview --bid=123456

# 转谱渲染
osu-beatmap-preview --bid=123456 --convert=mania

# 转谱后应用 Mod，并输出 GIF
osu-beatmap-preview --bid=123456 --convert=mania --mods=4k+dt1.25 --fmt=gif

# 应用多个 Mod 渲染
osu-beatmap-preview --bid=123456 --mods=hd+hr

# 指定多个 GIF 预览时间点
osu-beatmap-preview --bid=123456 --fmt=gif --time=10+25+60

# 从首物件前 2 秒开始渲染连续 GIF
osu-beatmap-preview --bid=123456 --fmt=gif --gif-clip-label --time=-2+10

# 指定区间，输出无时间标签的连续 GIF
osu-beatmap-preview --bid=123456 --fmt=gif --gif-clip --time=30+42

# 指定区间和 Mod，输出 MP4
osu-beatmap-preview --bid=123456 --fmt=mp4 --time=30+60 --mods=hd+dt1.25

# 按谱面预览时间输出约 30 秒 MP4
osu-beatmap-preview --bid=123456 --fmt=mp4 --preview-30s

# 组合使用转谱、PNG 和排列间距参数
osu-beatmap-preview --bid=123456 --convert=taiko --fmt=png --gap=180 --mods=sw

# 强制重新渲染并关闭日志
osu-beatmap-preview --bid=123456 --no-cache --no-log
```

### Mod 支持情况

| 模式 | GIF / MP4 | PNG |
| --- | --- | --- |
| Standard | `EZ` `HR` `HD` `DA` `TC` `DT` `HT` | `EZ` `HR` `HD` `DA` `TC` |
| Taiko | `EZ` `HR` `SW` `CS` `DT` `HT` | `EZ` `HR` `SW` |
| Catch | `EZ` `HR` `DT` `HT` | `EZ` `HR` |
| Mania | `CS` `DT` `HT` `1K`-`10K` `DS` `IN` `HO` | `1K`-`10K` `DS` `IN` `HO` |

### Mod 冲突规则

| 组合 | 说明 |
| --- | --- |
| `DT` / `HT` | 互斥。`DT` 默认 `1.5x`，范围 `1.01-2.00`；`HT` 默认 `0.75x`，范围 `0.50-0.99`。 |
| `EZ` / `HR` | 互斥。 |
| `TC` / `HD` | 互斥。 |
| `1K`-`10K` | 互斥，仅 `--convert=mania` 时生效。 |
| `IN` / `HO` | 互斥。 |
| `DA` / `EZ` / `HR` | `DA` 不能与 `EZ` 或 `HR` 同时使用，仅 Standard 可用。 |
| `DA` 参数 | 格式为 `da<参数><值>`，如 `dacs5`、`daar9.5`，也可叠加成 `dacs5ar9.5`。 |

## 输出

| 路径 | 说明 |
| --- | --- |
| 谱面缓存 | `<临时目录>/osu-beatmap-preview/osu-download-cache/<bid>.osu` |
| OSZ 缓存 | `<临时目录>/osu-beatmap-preview/osz-download-cache/`（谱包为 `<set-id>.osz`，提取音频按 `<set-id>/<文件名哈希>.<扩展名>` 隔离） |
| 优选 IP 缓存 | `<临时目录>/osu-beatmap-preview/osz-download-cache/osu-direct-preferred-ip.json` |
| 输出文件 | `<临时目录>/osu-beatmap-preview/outputs/<mode>_<bid>[_convert][_mods][_gifclip][_t<时间点或区间>][_preview30s][_bpm<BPM值>].<fmt>` |
| 日志文件 | `<临时目录>/osu-beatmap-preview/logs/` — `progress.log`（实时进度，`tail -f`）与 `render.log`（NDJSON 汇总） |

### 图片（PNG）

使用 `--fmt=png` 输出静态 PNG 图片。

- Standard 输出 `5×8` 的预览图，共 5 行、每行 8 个连续画面；`--time` 最多可指定 4 个时间点，每个时间点作为一行预览的起点。
- Taiko 输出按谱面滚动排列的长图，可使用 `--gap=<BPM>` 调整排列间距。
- Catch 输出按谱面排列的长图。
- Mania 输出按键道排列的长图，谱面较长时会自动分成多列。

### GIF 动图

使用 `--fmt=gif` 输出 GIF 动图。默认输出多段预览，可用 `--time=t1+t2+...` 指定游戏皮肤时间轴上的预览点。`--gif-clip` 输出单屏连续 GIF，不显示时间标签；`--gif-clip-label` 同样输出单屏连续 GIF，但会显示时间标签。未指定区间时，单屏模式仍按 `.osu` 的绝对 `PreviewTime` 选段，标签会换算为首物件相对时间；尾段不足时会向前补足。

### MP4 视频

使用 `--fmt=mp4` 输出带谱面音频的视频，支持四种模式。默认渲染全谱面，也可用 `--time=t1+t2` 指定游戏皮肤时间轴上的视频区间；负时间或全谱默认 padding 会显示为负标签，落在音频文件前的部分为静音。`--preview-30s` 会从 `.osu` 的绝对 `PreviewTime` 附近渲染约 30 秒实际播放时长，不能与 `--time` 同时使用。预览时间落在谱面尾段时，会向前补足播放时长。

### 命令行输出

程序向 stdout 输出 JSON，schema 如下：

```json
{
  "status": "success",
  "msg": "preview generated successfully for bid 738063",
  "preview-img": "/path/to/output.png",
  "beatmap-info": {
    "meta-data": { "title": "...", "artist": "...", ... },
    "difficulty": { ... }
  },
  "build-info": {
    "version": "1.0.3",
    "build_time": "2026-06-22T16:01:06.623636800Z"
  },
  "log": {
    "progress": "C:/Users/.../AppData/Local/Temp/osu-beatmap-preview/logs/progress.log",
    "render": "C:/Users/.../AppData/Local/Temp/osu-beatmap-preview/logs/render.log"
  }
}
```

> `preview-img` 字段为输出文件的绝对路径，格式由 `--fmt` 决定（`.gif` / `.png` / `.mp4`）。
> `log` 字段为可选字段，只有日志启用时才存在，不影响现有解析。

## 效果预览

![总览](docs/total.png)

## 其他说明

### 参数限制

- `--gif-clip` 和 `--gif-clip-label` 互斥，且只能用于 GIF 输出。
- `--preview-30s` 只能用于 MP4 输出，且不能同时传 `--time`。
- MP4 使用 `--time` 时必须传入两个时间点，格式为 `--time=t1+t2`。
- `--gif-clip` 或 `--gif-clip-label` 使用 `--time` 时必须传入两个时间点，格式为 `--time=t1+t2`。
- `--gap` 只对 Taiko PNG 生效。

### 缓存与日志

日志默认开启，可用 `--log-dir=<DIR>` 指定目录、`--no-log` 关闭；也可用环境变量 `OSU_PREVIEW_LOG_DIR` 覆盖目录。日志写入失败只降级到 stderr 提示，不影响渲染与 stdout 结果。

缓存文件不会自动删除，占用过大时可手动清理临时目录。输出文件采用原子写入（先写临时文件、完成后才替换），渲染中断不会产生可被当作有效缓存的损坏文件。

## 构建

### 环境要求

需要 Rust 1.73+ 和 C++ 编译器。安装 Rust：<https://rustup.rs>

### 构建命令

```bash
cargo build --release
# 产物: target/release/osu-beatmap-preview(.exe)
```

## License

[MIT](LICENSE)。内嵌 AAC 编码器使用 Fraunhofer FDK-AAC，许可证不授予专利权，详见 [第三方声明](docs/THIRD_PARTY_NOTICES.md)。

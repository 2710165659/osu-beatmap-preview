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
osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard] [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>]
```

### 参数

| 参数 | 说明 |
| --- | --- |
| `--bid` | 必填，纯数字的 Beatmap ID。 |
| `--convert` | 转谱模式，支持 `mania` / `ctb` / `taiko` / `standard` / `std`。仅 Standard 可用。 |
| `--mod` | 单个 Mod，组合时重复传入，例如 `--mod=hd --mod=hr`；倍速类可带数值，如 `--mod=dt1.25`。 |
| `--fmt` | 输出格式：`gif` / `png` / `mp4`。不填时按模式取默认值。 |
| `--time-points` | 时间点列表。GIF 和 Standard PNG 可重复传入多个点；MP4 最多传入一个点。每个点为游戏时间秒数或 `preview`。未传时自动选择（MP4 默认从 `0` 开始）。 |
| `--duration-time` | 仅 MP4 可用，输出时长（秒）。默认 `600`。 |
| `--no-log` | 关闭日志。 |
| `--no-cache` | 跳过下载缓存与输出缓存，强制重新渲染。 |
| `--config` | 配置文件路径，或 JSON/YAML 格式的配置对象。嵌套映射递归合并；数组和标量整体替换，未传入字段保留默认值。 |
| `--version` | 打印版本号与构建时间后退出。 |

> MP4 数值起始时间使用游戏时间轴：转谱后的目标模式首个可玩物件为 `0:00`，不是编辑器左下角的绝对音轨时间。支持负数，早于音频起点的部分输出静音。

### 示例

```bash
# 使用默认参数渲染
osu-beatmap-preview --bid=123456

# 转谱渲染
osu-beatmap-preview --bid=123456 --convert=mania

# 转谱后应用 Mod，并输出 GIF
osu-beatmap-preview --bid=123456 --convert=mania --mod=4k --mod=dt1.25 --fmt=gif

# 应用多个 Mod 渲染
osu-beatmap-preview --bid=123456 --mod=hd --mod=hr

# 按谱面预览时间输出 30 秒 MP4
osu-beatmap-preview --bid=123456 --fmt=mp4 --time-points=preview --duration-time=30

# GIF 指定四个渲染时间点（列表参数必须重复传入）
osu-beatmap-preview --bid=123456 --fmt=gif --time-points=5 --time-points=10 --time-points=15 --time-points=20

# 组合使用转谱和 PNG；Taiko 间距由配置项控制
osu-beatmap-preview --bid=123456 --convert=taiko --fmt=png --mod=sw

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
| 输出文件 | 默认配置写入 `<OUTPUT_DIR>/<mode>_<bid>...<fmt>`；非默认配置写入 `<OUTPUT_DIR>/<config-hash>/<mode>_<bid>...<fmt>` |
| 配置文件夹 | 二进制文件同级目录 |
| 日志文件 | `<临时目录>/osu-beatmap-preview/logs/` — `progress.log`（实时进度，`tail -f`）与 `render.log`（NDJSON 汇总） |

上述目录由 `assets/default_config.yml` 顶层 `paths` 配置项定义；默认 `CONFIG_DIR: "./"` 表示二进制文件所在目录。程序会尝试读取二进制文件同级的 `config.yml`，其后再应用 `--config` 覆盖。配置文件不存在时继续使用默认值；存在但格式错误会导致启动失败。

配置来源按“内置默认值 < `CONFIG_DIR/config.yml` < `--config`”合并。`--config` 可以是文件路径，也可以直接传入 JSON/YAML 对象，例如：

配置字段必须来自内置配置；未知字段、顶层非对象和无法转换的类型会导致启动失败。数字和布尔值也接受可安全转换的字符串形式。
配置会先与内置默认值合并并归一化；若最终生效值与默认配置相同，仍使用 `OUTPUT_DIR`。否则程序对差异配置计算稳定的 SHA-256（取末 6 位）并使用 `OUTPUT_DIR/<config-hash>/`，同时在该目录写入只包含非默认字段的规范 `config.yml`。因此等价的 JSON、YAML、配置文件及显式填写默认值不会拆分输出缓存；以后新增且仍采用默认值的配置项也不会改变已有 hash。

```bash
osu-beatmap-preview --bid=123456 --config='{"layout":{"standard":{"gif":{"ROW_COUNT":1}}}}'
osu-beatmap-preview --bid=123456 --config='{layout: {standard: {gif: {ROW_COUNT: 1}}}}'
osu-beatmap-preview --bid=123456 --config=C:/path/to/config.yml
```

### 图片（PNG）

使用 `--fmt=png` 输出静态 PNG 图片。

- Standard GIF 网格可在 `assets/default_config.yml` 中配置行数和列数。
- Taiko 输出按谱面滚动排列的长图，间距由配置项 `SPACING_PER_BPM` 控制，`0` 表示自动计算。
- Catch 输出按谱面排列的长图。
- Mania 输出按键道排列的长图，谱面较长时会自动分成多列。

### GIF 动图

使用 `--fmt=gif` 输出 GIF 动图。四种模式分别在 `assets/default_config.yml` 中配置网格、时长和 `SHOW_TIME_LABEL`。将网格设为 `1×1` 并启用标签即可得到单画面带时间标签的预览。自动选段结果是确定性的，并优先使用谱面的 `PreviewTime`。

### MP4 视频

使用 `--fmt=mp4` 输出带谱面音频的视频，支持四种模式。默认从游戏时间 `0` 开始输出 600 秒；使用单个 `--time-points` 和 `--duration-time` 指定区间。`--time-points=preview` 使用谱面的 `PreviewTime`，接近谱面尾部时会向前调整。GIF 和 Standard PNG 可以重复传入 `--time-points` 指定多个渲染点；旧的 `5+10+15` 拼接格式不再支持。

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

- `--mod` 每次只能传入一个 Mod，组合时必须重复参数；使用 `+` 拼接会报错。
- `--time-points` 和 `--duration-time` 只能用于 GIF、Standard PNG 或 MP4（`--duration-time` 仅 MP4）。

### 缓存与日志

日志默认开启，目录始终由配置项 `LOG_DIR` 决定；可用 `--no-log` 关闭。日志写入失败只降级到 stderr 提示，不影响渲染与 stdout 结果。

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

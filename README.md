# osu! Beatmap Preview

> 一个快速的 osu! 谱面预览工具，支持四种模式（Standard / Taiko / Catch / Mania）的 GIF 动图与 PNG 静态图渲染。

## 特性

- **单可执行文件**：皮肤资源在编译期嵌入二进制，无运行时依赖。
- **跨平台**：Windows / Linux / macOS。
- **四模式支持**：`standard`、`taiko`、`catch`、`mania`。
- **转谱**：Standard 可转为 Taiko / Catch / Mania 并预览。
- **丰富的 Mod**：`EZ` `HR` `HD` `DA` `DT` `HT` `SW` `CS` `1K`–`10K` `DS` `IN` `HO`。
- **GIF + PNG**：GIF 支持自定义时间点，PNG 输出全谱面长图。
- **高性能**：渲染速度快、内存占用低、输出文件体积小。详见 [批量渲染报告](docs/report.txt)。

## 构建

```bash
cargo build --release
# 产物: target/release/osu-beatmap-preview(.exe)
```

> 需要 Rust 1.70+。安装方式：<https://rustup.rs>

## 使用

### 基本命令

```bash
osu-beatmap-preview --bid=<BID> [选项]
```

### 参数

| 参数 | 说明 |
| --- | --- |
| `--bid` | Beatmap ID，**必填** |
| `--convert` | 转谱模式：`mania`、`ctb`、`taiko`（仅 standard 可用） |
| `--mods` | Mod 组合，用 `+` 连接，如 `hd+hr` |
| `--fmt` | 输出格式：`gif` 或 `png` |
| `--time` | GIF 时间点，最多四个，单位秒，用 `+` 连接，如 `10+25+60` |
| `--bpm` | 指定 BPM 值，用于渲染节拍线 |
| `--version` | 打印版本号与构建时间 |

### 默认行为

| 模式 | 默认格式 | 说明 |
| --- | --- | --- |
| `standard` | GIF | 自动选 4 个时间点，每段 10 秒 |
| `taiko` | PNG | 全谱面长图 |
| `catch` | PNG | 全谱面长图 |
| `mania` | PNG | 全谱面长图 |

### 示例

```bash
# Standard，默认 GIF
osu-beatmap-preview --bid=123456

# Standard 转 Mania，默认 PNG
osu-beatmap-preview --bid=123456 --convert=mania

# 强制输出 GIF
osu-beatmap-preview --bid=123456 --convert=mania --fmt=gif

# 添加 mod
osu-beatmap-preview --bid=123456 --mods=hd+hr

# 自定义倍速
osu-beatmap-preview --bid=123456 --mods=dt1.25

# 自定义时间点
osu-beatmap-preview --bid=123456 --time=10+25+60

# Standard 的 DA（Difficulty Adjust）
osu-beatmap-preview --bid=123456 --mods=dacs5ar9.5

# 复杂组合
osu-beatmap-preview --bid=123456 --convert=mania --fmt=gif --mods=4k+ds+in+dt1.2 --time=10+30+50
```

## Mod 支持

### GIF

| Mod | Standard | Taiko | Catch | Mania |
| --- | :---: | :---: | :---: | :---: |
| `EZ` | ✅ | ✅ | ✅ | — |
| `HR` | ✅ | ✅ | ✅ | — |
| `HD` | ✅ | — | — | — |
| `DA` | ✅ | — | — | — |
| `SW` | — | ✅ | — | — |
| `CS` | — | ✅ | — | ✅ |
| `DT` | ✅ | ✅ | ✅ | ✅ |
| `HT` | ✅ | ✅ | ✅ | ✅ |
| `1K`–`10K` | — | — | — | ✅ |
| `DS` | — | — | — | ✅ |
| `IN` | — | — | — | ✅ |
| `HO` | — | — | — | ✅ |

### PNG

| Mod | Standard | Taiko | Catch | Mania |
| --- | :---: | :---: | :---: | :---: |
| `EZ` | ✅ | ✅ | ✅ | — |
| `HR` | ✅ | ✅ | ✅ | — |
| `HD` | ✅ | — | — | — |
| `DA` | ✅ | — | — | — |
| `SW` | — | ✅ | — | — |
| `1K`–`10K` | — | — | — | ✅ |
| `DS` | — | — | — | ✅ |
| `IN` | — | — | — | ✅ |
| `HO` | — | — | — | ✅ |

> `DT` / `HT` / `CS` 仅 GIF 支持。

### Mod 规则

| 规则 | 说明 |
| --- | --- |
| `DT` / `HT` | 互斥。`DT` 默认 1.5x，范围 1.01–2.00；`HT` 默认 0.75x，范围 0.50–0.99 |
| `EZ` / `HR` | 互斥 |
| `1K`–`10K` | 互斥，仅 `--convert=mania` 时生效 |
| `IN` / `HO` | 互斥 |
| `DA` / `EZ` / `HR` | `DA` 不能与 `EZ` 或 `HR` 同时使用，仅 Standard 可用 |
| `DA` 参数 | 格式 `da<参数><值>`，如 `dacs5` `daar9.5`，可叠加如 `dacs5ar9.5` |

> 冲突组合会直接报错。

## 输出

程序向 stdout 输出 JSON，schema 如下：

```json
{
  "status": "ok",
  "msg": "...",
  "preview-img": ["/path/to/output.gif"],
  "beatmap-info": {
    "title": "...",
    "artist": "...",
    "creator": "...",
    "version": "...",
    "mode": "..."
  },
  "build-info": {
    "version": "1.0.2",
    "build_time": "2026-06-21T00:00:00.000000000+08:00"
  }
}
```

| 路径 | 说明 |
| --- | --- |
| 谱面缓存 | `<临时目录>/osu-beatmap-preview/osu-download-cache/<bid>.osu` |
| 输出文件 | `<临时目录>/osu-beatmap-preview/outputs/<mode>_<bid>[_convert][_mods][_t<时间点>][_bpm<BPM值>].<fmt>` |
| 批量脚本 | `batch_render.ps1` — 可批量渲染多个 bid 并生成对比 HTML |

> 缓存文件不会自动删除，占用过大时可手动清理临时目录。

## 效果预览

![总览](docs/total.png)

> 更多示例见 [docs/preview](docs/preview)。

---

> 如果这个项目对你有帮助，欢迎点个 ⭐ Star 支持一下～

## License

[MIT](LICENSE)
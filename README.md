# osu! 谱面预览工具（Rust 版）

`osu-beatmap-preview` 的 Rust 重构版，生成 osu! 四模式的预览图或预览动图。

- 单可执行文件：皮肤资源在编译期嵌入二进制，无运行时依赖。
- 跨平台：Windows / Linux / macOS（动态链接系统库即可）。

## 默认行为

- `standard`：默认输出 GIF，自动选 4 个时间点，每段 10 秒。
- `taiko` / `catch` / `mania`：默认输出全谱面 PNG。
- 任意模式都可用 `--fmt gif` 或 `--fmt png` 强制指定格式。
- GIF 时长：四个模式每段均为 10 秒。

> `standard` 以外模式的默认 PNG 是全谱面长图。

## 构建

```bash
cargo build --release
# 产物: target/release/osu-beatmap-preview(.exe)
```

## 命令

```bash
osu-beatmap-preview --bid=<BID> [选项]
```

| 参数 | 说明 |
| --- | --- |
| `--bid` | Beatmap ID，必填 |
| `--convert` | 仅 `standard` 可用：`mania`、`ctb`、`taiko` |
| `--mods` | 多个 mod 用 `+` 连接，如 `dt1.2+hr` |
| `--fmt` | `gif` 或 `png` |
| `--time` | GIF 时间点，最多四个，单位秒，如 `10+25+60` |

## 示例

```bash
# standard，默认 GIF
osu-beatmap-preview --bid=123456

# standard 转 mania，默认 PNG
osu-beatmap-preview --bid=123456 --convert=mania

# 强制输出 GIF
osu-beatmap-preview --bid=123456 --convert=mania --fmt=gif

# 添加 mod
osu-beatmap-preview --bid=123456 --mods=hd+hr

# 自定义倍速
osu-beatmap-preview --bid=123456 --mods=dt1.25

# 自定义时间点
osu-beatmap-preview --bid=123456 --time=10+25+60

# standard 的 DA
osu-beatmap-preview --bid=123456 --mods=dacs5ar9.5

# 较复杂的组合示例
osu-beatmap-preview --bid=123456 --convert=mania --fmt=gif --mods=4k+ds+in+dt1.2 --time=10+30+50
```

## Mod 支持

### GIF

| 模式 | 支持 |
| --- | --- |
| `standard` | `EZ` `HR` `HD` `DA` `DT` `HT` |
| `taiko` | `EZ` `HR` `SW` `CS` `DT` `HT` |
| `catch` | `EZ` `HR` `DT` `HT` |
| `mania` | `1K`-`10K` `DS` `CS` `IN` `HO` `DT` `HT` |

### PNG

| 模式 | 支持 |
| --- | --- |
| `standard` | `EZ` `HR` `HD` `DA` |
| `taiko` | `EZ` `HR` `SW` |
| `catch` | `EZ` `HR` |
| `mania` | `1K`-`10K` `DS` `IN` `HO` |

> `taiko` 的 `CS` 仅 GIF 支持，`mania` 的 `CS` 也仅 GIF 支持。

## 规则

- `DT` 默认 `1.5x`，范围 `1.01–2.00`；`HT` 默认 `0.75x`，范围 `0.50–0.99`。
- `1K`-`10K` 和 `DS` 仅在 `--convert=mania` 时真正生效。
- `DA` 仅 `standard` 可用，参数格式：`da<参数><值>`，如 `dacs5ar9.5`。
- `DA` 不能和 `EZ` 或 `HR` 同时使用。
- `DT` 和 `HT` 互斥，`EZ` 和 `HR` 互斥，`1K`-`10K` 互斥，`IN` 和 `HO` 互斥。

> 冲突组合会直接报错。

## 输出

- stdout 输出 JSON（与 Python 版 schema 一致）：`status` / `msg` / `preview-img` / `beatmap-info`。
- 谱面缓存：`<系统临时目录>/osu-beatmap-preview/osu-download-cache/<bid>.osu`。
- 输出文件：`<系统临时目录>/osu-beatmap-preview/outputs/<bid>[_convert][_mod-...].<fmt>`。
- 缓存文件不会自动删除，占用过大时可手动清理。
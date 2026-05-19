# osu! 谱面预览工具

## 简介

本工具可为 osu! 的四个模式生成预览图或预览动图。

- **默认行为**
  - `standard` 模式：生成 GIF，从谱面中截取 4 个时间点（默认包含预览点，其余随机，避开休息段），每个点截取 5 秒。
  - `taiko`、`catch`、`mania` 模式：生成 全谱面 PNG，单张图片大小一般不超过 2 MB，不支持超过 10 分钟的谱面。
- **主动指定 `--fmt`**
  - 无论哪种模式，都可以通过 `--fmt gif` 强制输出 GIF。
  - 若为 GIF，standard 每个时间点 5 秒，其他三种模式每个时间点 **10 秒**。
  - 也可以通过 `--fmt png` 强制输出静态 PNG。
- **性能表现**
  - 渲染速度：#TODO 待项目完成后测试
  - 内存占用：#TODO 待项目完成后测试
  - 生成图片大小：#TODO 待项目完成后测试
  - 其他：缓存文件夹存放的文件资源不会自动删除，如文件占用过大可手动删除

> **注意**：`standard` 以外模式的默认 PNG 是全谱面长图；GIF 则按上述时间点生成动画。

## 命令格式

```bash
python scripts/run.py --bid=<BID> [选项]
```

| 参数        | 必填 | 说明                                                                                                |
| ----------- | ---- | --------------------------------------------------------------------------------------------------- |
| `--bid`     | 是   | 谱面的 Beatmap ID（数字）                                                                           |
| `--convert` | 否   | 仅对 `osu!standard` 谱面生效，转换为其他模式：`mania`、`ctb`（catch）、`taiko`                      |
| `--mod`     | 否   | 启用的 mod，大小写不敏感，多个 mod 用 `+` 连接（例如 `hd+hr`）                                      |
| `--fmt`     | 否   | 输出格式：`gif` 或 `png`。不填时按默认行为                                                          |
| `--times`   | 否   | 自定义 GIF 时间点（单位：秒），用 `+` 连接多个时间点。仅 `--fmt gif` 时有效，会覆盖默认的四个时间点 |


## 使用示例

### 基础用法

```bash
# 查看标准谱面（默认生成 GIF，4个自动时间点）
python scripts/run.py --bid=123456

# 将标准谱面转换为 mania 并生成全谱面 PNG
python scripts/run.py --bid=123456 --convert=mania

# 转换到 mania 并强制生成 GIF（4个自动时间点，每个10秒）
python scripts/run.py --bid=123456 --convert=mania --fmt=gif
```

### 添加 Mod

```bash
# 标准 + Hidden + HardRock（默认 GIF）
python scripts/run.py --bid=123456 --mod=hd+hr

# 转换到 catch，开启 Easy（默认 PNG）
python scripts/run.py --bid=123456 --convert=ctb --mod=ez

# mania 模式，指定 4K 且开启 Hidden + Fade In（默认 PNG）
python scripts/run.py --bid=123456 --convert=mania --mod=4k+hd+in
```

### 自定义速率 Mod

速率 mod 可以带小数点：

```bash
# DT 1.8 倍速
python scripts/run.py --bid=123456 --mod=dt1.8

# HT 0.65 倍速
python scripts/run.py --bid=123456 --mod=ht0.65

# 标准 1.5 倍速（等同于 --mod=dt1.5）
python scripts/run.py --bid=123456 --mod=dt
```

### 自定义 GIF 时间点

```bash
# 在 10、25、60、90 秒处截取 GIF（每个点5秒）
python scripts/run.py --bid=123456 --times=10+25+60+90

# 只截取一个点（GIF 会包含该点开始的5秒片段和其他片段）
python scripts/run.py --bid=123456 --times=10

# 结合 mod 与自定义时间
python scripts/run.py --bid=123456 --mod=hd --times=10+30+50
```

### DA (Difficulty Adjust) 参数

仅 `standard` 模式可用，通过 `--mod` 指定，格式：`da<参数><值>`，多项连续书写。

```bash
# 调整 CS=5，AR=9.5
python scripts/run.py --bid=123456 --mod=dacs5ar9.5

# 调整 OD=8，HP=3
python scripts/run.py --bid=123456 --mod=daod8hp3

# 同时调整多项：CS=4.2, AR=9, OD=7.5, HP=5
python scripts/run.py --bid=123456 --mod=dacs4.2ar9od7.5hp5
```

> **取值范围**：CS、OD、HP 0.0 – 11.0；AR -10.0 – 11.0。


## 各模式 Mod 支持一览

### GIF 格式下支持的 Mod

| 模式     | 可用 Mod                                   |
| -------- | ------------------------------------------ |
| standard | `EZ` `HR` `HD` `DA` `DT` `HT`              |
| catch    | `EZ` `HR` `DT` `HT`                        |
| taiko    | `EZ` `HR` `SW` `CS` `DT` `HT`              |
| mania    | `1K` – `10K` `DS` `CS` `IN` `HO` `DT` `HT` |

### PNG 格式下支持的 Mod

| 模式     | 可用 Mod                              |
| -------- | ------------------------------------- |
| standard | `EZ` `HR` `HD` `DA` `DT` `HT`         |
| catch    | `EZ` `HR` `DT` `HT`                   |
| taiko    | `EZ` `HR` `SW` `DT` `HT`              |
| mania    | `1K` – `10K` `DS` `IN` `HO` `DT` `HT` |

> **差异提醒**：taiko 的 `CS` 仅在 GIF 下生效；mania 的 `CS` 也仅 GIF 下生效，PNG 不支持。


## 特殊 Mod 规则

### 速率 mod：DT / HT

- `DT` 默认 **1.5x**，自定义范围：**1.01 – 2.00**（例如 `dt1.2`、`dt2`）
- `HT` 默认 **0.75x**，自定义范围：**0.50 – 0.99**（例如 `ht0.8`、`ht0.5`）
- 写法示例：`dt1.35`、`ht0.6`

### mania 的 1K–10K 与 DS

- 仅在 **转谱**（`--convert=mania`）时真正生效，直接用于原生 mania 谱面不会报错但没有实际效果。
- `1K` 到 `10K` 之间互斥，只能同时启用其中一个。

### DA 冲突说明

- `DA` 与 `EZ` 冲突，不能同时使用。
- `DA` 与 `HR` 冲突，不能同时使用。
- 这是 standard 模式的特有规则。


## 全局 Mod 冲突规则

| Mod A  | Mod B  | 冲突说明       |
| ------ | ------ | -------------- |
| DT     | HT     | 速率互斥       |
| EZ     | HR     | 四维结果互斥       |
| 1K–10K | 同系列 | mania 键数互斥 |
| IN     | HO     | mania 长条行为互斥 |

> 冲突会直接报错提示
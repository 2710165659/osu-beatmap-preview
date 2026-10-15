# OSR 回放预览架构设计：模块划分与依赖规则

返回入口：[架构设计总览](../osr-architecture.md)。

## 3. 目标目录与职责

下面是预期的模块级目录；文件名是边界建议，实施时可以按现有文件大小拆分。

```text
crates/osu-beatmap-preview-cli/src/
├─ cli.rs                    # 保留语法解析入口，收集参数出现情况
├─ application/
│  ├─ request.rs             # 请求组合与公开应用入口
│  ├─ request/               # common.rs、osu.rs、osr.rs：通用/特化请求
│  ├─ validation_common.rs   # 不依赖文件内容的通用校验
│  ├─ validation_osu.rs      # osu 来源、convert、mod、fmt 校验
│  ├─ validation_osr.rs      # OSR 来源、模式、回放格式、fmt 校验
│  ├─ source_osu.rs          # osu/osz 下载、解包、缓存
│  ├─ source_osr.rs          # OSR 文件读取与 checksum 触发的谱面获取
│  ├─ plan/                  # osu.rs、osr.rs：默认值、能力校验、选段计划
│  └─ execute.rs             # 只编排端口，不实现规则判定
├─ download/official/        # OAuth、checksum 查询；复用现有 osu/osz 下载
├─ media/                    # 现有解码/混音/编码，增加接收回放声音时间轴
└─ export/
   ├─ standard/osr.rs       # Standard GIF/MP4 回放输出适配
   ├─ taiko/osr.rs
   ├─ catch/osr.rs
   └─ mania/osr.rs

crates/osu-beatmap-preview-core/src/
├─ domain/parser/
│  ├─ osu/                   # 移入现有 hit_objects、sections、timing
│  └─ osr/                   # header、LZMA、帧、扩展 JSON；纯 Rust 可选依赖
├─ preparation/              # 共享准备：难度、Mod、堆叠、逻辑路径、嵌套物件
├─ replay/
│  ├─ model.rs               # ReplayDocument、ReplayHeader、ReplayFrame、ReplayMetadata
│  ├─ timeline.rs            # 输入索引、显示插值、按顺序遍历
│  ├─ normalize.rs           # stable/lazer 帧归一化和坐标边界
│  ├─ compatibility.rs       # mode、版本、Mod、规则变体选择
│  └─ failure.rs             # 回放结束、提前终止和失败状态
├─ calculate/
│  ├─ standard/              # 判定、滑条追踪、转盘积分、score、rank、规则变体
│  ├─ taiko/                 # 后续：鼓面/鼓边、连打、大连打
│  ├─ catch/                 # 后续：水果/小果/Dash/香蕉
│  └─ mania/                 # 后续：键数、按键和长条判定
├─ gameplay/
│  ├─ input.rs               # InputSnapshot 与 ReplayFrameEvent
│  ├─ engine.rs              # 能力分派、顺序编译、checkpoint
│  ├─ timeline.rs            # 只读结果查询，输出消费者分别持有查询游标
│  ├─ state.rs               # GameplayState、ObjectState、ScoreSnapshot、RankSnapshot
│  ├─ events.rs              # Hit、Miss、SliderTick、ComboBreak、Fail
│  └─ overlay.rs             # HUD、光标、按键、判定弹字的纯数据
├─ render/cpu/modes/<mode>/
│  └─ osr.rs                 # 回放叠加和打击反馈；不计算分数
├─ render/wgpu/modes/<mode>/
│  └─ osr.rs                 # 实时场景中的同一回放叠加契约
└─ hitsound/
   └─ replay.rs              # GameplayEvent 到样本事件的映射

crates/osu-beatmap-preview-wasm/src/
├─ replay.rs                 # 回放字节、头部摘要、双资源会话创建
├─ archive.rs                # 复用解包；难度选择增加 MD5 条件
└─ lib.rs                    # 保留旧 WebGpuSession 入口，增加回放构造入口

crates/osu-beatmap-preview-web/
├─ src/replay/               # 本地 OSR、可选 osu/osz、加载状态及回放控制
├─ src/preview.js            # 原预览控制；选择回放分支并关闭改 Mod/转谱
├─ src/components/           # 回放入口和控制 UI；分数等 HUD 仍画在 Canvas
└─ backend/official/         # 服务端 OAuth 和 checksum 查询；不解析 OSR
```

建议将需求中的 `caculate` 命名为 `calculate`。四模式目录按阶段创建，首版只落地 Standard；公共门面保持 `processing::parse`、`model`、`gameplay`，现有 `parse_beatmap_bytes` 用再导出过渡。不为占位创建空实现，也不立即拆出新 crate。`renderer` 继续只接收 FrameScene；GUI/mobile 暂不增加产品功能。

### 文件体量与目录整洁

新增生产逻辑文件建议 200～400 行，超过约 500 行必须检查是否存在多个变化原因；这是一条设计审查线，不是强制为了行数切文件。`mod.rs`/门面原则上控制在约 100 行，放声明、再导出和小型组装；复杂判定、I/O 和算法不放入口。测试较大时按现有规范放同目录 `tests.rs` 或 `tests/`，文件顶部 `#![cfg(test)]`，仅通过文件末尾的 `#[cfg(test)] mod tests;` 引入；不能为行数把正常逻辑和测试交错。

首版复杂模块建议一开始按以下职责组织，短模块先保留单文件，增长后保持同名目录升级，不改变公共入口：

| 模块 | 内部文件划分 | 避免的混杂 |
| --- | --- | --- |
| `parser/osr/` | `header.rs`、`frames.rs`、`extension.rs`、`decode.rs` | 二进制边界、帧文本兼容、JSON 和解压分别维护 |
| `calculate/standard/` | `circle.rs`、`slider.rs`、`spinner.rs`、`score.rs`、`rank.rs`、`policy/` | 不把三类物件、计分、版本策略装进一个 standard.rs |
| `preparation/standard/` | `difficulty.rs`、`stacking.rs`、`objects.rs`；路径算法继续复用共享实现 | 规则坐标与像素缓存分开，禁止复制 slider.rs |
| `render/.../standard/osr/` | 起步 `osr.rs`；增长后拆 `objects.rs`、`cursor.rs`、`hud.rs`、`feedback.rs` | 入口保留 `osr`，不增加 `osr2`、`extra` 或新的通用大文件 |
| `gameplay/` | 输入、事件、状态契约与编译、查询各自独立；复杂内部放 `compile/` | 避免 calculate ↔ engine 循环，以及 state 与渲染配置混杂 |
| `web/src/replay/` | `load.js`、`session.js`、`controls.js`；共用资源与音频桥接 | 不继续把回放上传、校验和生命周期加入 preview.js |

当前文件总行数（含测试/空行）已有：`legacy.rs` 985、`api/session.rs` 1236、Standard `context.rs` 1024、`frame.rs` 1049、`slider.rs` 1181、WASM `lib.rs` 907、Web `preview.js` 1646。回放接入前应先抽取会用到的职责：session 拆 `preview.rs`/`replay.rs` 与共享时钟/资源控制；Standard 抽准备层，基础 circle/slider/spinner 逐项拆；WASM lib 只保留绑定门面。已有大文件不要求本轮全量清理，但新增功能不得继续堆入其中。

目录规则：沿用 `standard/taiko/catch/mania` 命名，每个概念只有一个实现归属；一般保持 2～3 层职责目录，只有复杂模式策略允许再多一层。共享代码只有至少两个真实消费者时才提取；不建立 `manager/utils/common` 万能容器，不为一个短函数建文件，也不同时维护新旧两份算法。


## 4. 模块依赖规则

依赖方向必须保持为：

```text
宿主 → application/api 编排 → parser、preparation、gameplay、render、hitsound
parser → model + support
preparation → model + 共享逻辑路径/时间工具
replay 输入归一化 → model + gameplay 契约
gameplay 编译器 → calculate 规则实现 + preparation + replay 输入
calculate 规则实现 → gameplay 契约 + preparation 数据（禁止调用编译器）
render/osr、hitsound/replay → gameplay 只读结果 + model
renderer → FrameScene；宿主媒体输出 → Img/PCM/编码器
```

这里的箭头表示源码依赖，第二节的箭头表示数据流。共享类型放 `gameplay::state/events/input`，其不得引用模式引擎；`gameplay::engine` 可以调用 calculate，calculate 不得反向调用 engine，避免模块循环。core 不引用任何宿主 crate；CLI→core，WASM→core+renderer，renderer→core 场景契约，Web→WASM 公共 API；CLI 原 CPU 输出不引入 renderer。

具体规则：

1. `domain/parser/osr` 只接收字节并返回 `ReplayDocument`，不得访问路径、网络、缓存、日志或音频设备。
2. `replay` 可以依赖谱面模型和规则集接口，但不能依赖 CPU/WGPU 绘制、GIF/MP4 编码或 CLI 配置。
3. `calculate` 只消费 `PreparedBeatmap`、带时间的模式输入和规则变体，输出状态与事件；由 gameplay 编译器顺序驱动，不得直接绘图或播放声音。
4. `render/.../osr.rs` 与基础物件 renderer 只读消费 `PreparedBeatmap` 和 `GameplayTimeline`：HUD 读 GameplayState/事件，基础物件读 ObjectState/轨迹。OverlayStyle 只控制展示；判定、ACC、评级不能从渲染器反向计算。
5. `hitsound/replay.rs` 只把事件映射成统一音频事件；样本查找仍复用现有 `SampleResolver`，不能在四个渲染器中各自读文件。
6. `osu` 与 `osr` 解析器只依赖共享谱面模型；OSR 不调用 `.osu` 解析器，二者由应用层在 `BeatmapResolver` 中组合。
7. 现有 Preview 路径保持当前绘制/可选 autoplay 和谱面 hitsound 时间轴。只有 Replay 才解析回放并编译回放结果，Preview 不构造空引擎。
8. CLI、WASM 只实现资源和输出端口。core 不依赖 `reqwest`、`zip`、`symphonia`、窗口或具体编码器。OSR 必需的 LZMA 属于文件格式解析，用 `replay` 可选特性启用纯 Rust 实现，CLI/WASM 显式启用；不重复写两份解码器。


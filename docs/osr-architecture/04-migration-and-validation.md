# OSR 回放预览架构设计：迁移、测试与验收

返回入口：[架构设计总览](../osr-architecture.md)。

## 10. 测试与迁移顺序

保留当前 crate 分工，以共享准备和结果契约为必要重构。另建完整回放渲染/混音管线会复制路径和样本规则；现在整体重写 workspace 也没有业务依据。按以下顺序推进，每一步都可单独评审，迁移完成后旧实现只留再导出，不能双份维护。

| 顺序 | 当前文件 → 目标边界 | 完成条件 |
| --- | --- | --- |
| 1 | `gameplay.rs` → 契约目录；`application/request.rs` → 通用/特化类型和两个校验文件；`domain/parser` → osu/osr | 普通请求兼容；新参数错误在加载前返回；占位 trait 不再两份 |
| 2 | `application/legacy.rs`、`local.rs` → 来源与计划；CLI/Node official 适配 | 实际调用 checksum；本地 MD5 与下载错误路径明确；已有来源规则不回退 |
| 3 | Standard `context.rs`、`stacking.rs`、`slider.rs` → preparation 逻辑准备 | 同一 ObjectId/逻辑几何被判定与绘制共享；原 Preview 图像和缓存行为对照 |
| 4 | 新建 `calculate/standard/` 和 gameplay 编译器/结果索引 | 无输出 FPS 依赖；支持版本表与已有 Mod 对照；stable 自动 CL，AL 拒绝 |
| 5 | Standard CPU/WGPU mode、`hitsound`、`api/session.rs` → 结果消费 | 并行帧、音频预读、seek、失败后预览使用同一结果；基础物件也反映真实命中 |
| 6 | CLI export、WASM `lib.rs/archive.rs`、Web `preview.js` → 宿主接入 | GIF/MP4/Web 完整加载与取消流程；分数不在 JS 重算；新逻辑未堆入大文件 |

测试代码必须放在对应模块末尾或受 cfg 保护的同目录测试模块，不能进入生产构建。验收包括：

1. **格式层**：用 `.local/osr-samples` 手工验证五份 OSR 的头部、LZMA、扩展 JSON、负首帧、模式和 MD5；CI 使用最小合成 fixture，不能依赖忽略目录或实时网络。加入损坏压缩流、空回放、反向时间和截断 fixture。
2. **来源层**：验证本地 osu/osz 优先级、checksum 查询、MD5 不匹配、OSZ 音频缺失和下载失败。
3. **输入层**：首版验证 Standard 快速按键、同刻输入和截止点、负时间、seek 与输出 FPS 无关；其他模式先验证原始解析，归一化验收随模式实现。
4. **Standard 规则层**：成功样本做终态比对；小型确定性 fixture 单独覆盖命中窗、晚打滑条、断连、spinner、ACC/评级。补充已有 Mod 与计分版本样本，stable/显式 CL 对照，AL 明确 Unsupported。
5. **输出层**：同一回放在 GIF、MP4、Web 的状态快照和音效事件时间一致；普通 osu 预览像素和耗时路径不受回放模块影响。
6. **失败层**：构造回放提前结束，断言失败提示、状态冻结、无后续玩家音效但谱面和音乐继续。


## 11. 样本与外部依赖记录

架构分析样本保存在 `.local/osr-samples/`，该目录已被 `.gitignore` 排除。当前包含 `xuan_yuan` 的 Standard、Taiko、Catch、Mania 回放及对应 `.osu`、`manifest.json` 和 `inspection.json`。样本通过官方 OAuth API 下载；认证密钥只用于下载，不写入仓库、文档、日志或测试 fixture。

样本事实：四模式样本均为 lazer 导出；Standard 另有一个 AL 样本，且 AL 仅出现在扩展 JSON、传统 Mod 位掩码为 0；样本均为成功成绩，不能证明 stable、CL、失败回放或其它 Mod 已兼容。样本的作用是固定二进制解析和数据契约，不代表首版必须支持其中所有规则。


## 12. 架构验收标准

架构改造完成后，应能回答以下问题而无需查看渲染实现：回放从哪里来、谱面如何按 MD5 绑定、哪个规则变体负责计分、判定事件如何到达画面和音频、提前失败后哪些状态冻结、stable/CL 由哪些规则模块处理、普通 osu 预览为何不增加开销。任何新增模式或 Mod 如果需要同时修改 CLI、OSR 解析器、计分器、四个渲染器和音频解码器，说明依赖方向被破坏，应先调整边界再实现功能。

| 需求变化 | 主要修改位置 | 可维护性边界 |
| --- | --- | --- |
| 新 OSR 文件字段 | parser/osr + replay/model | 已知字段消费者不变；未知字段保留或按版本拒绝 |
| stable/CL/历史分数 | calculate/standard/classic + gameplay/engine/compile/classic + replay/compatibility | 复用准备、结果、绘制和混音；先证明坐标/事件契约足够 |
| AL 或已有 Mod 新 settings | Mod 归一化、对应模式规则策略、必要视觉策略 | 不增加 CLI --mod 覆盖回放，不在所有消费者散布字符串判断 |
| 新回放模式 | calculate/<mode>、输入归一化、render 两后端 mode/osr、hitsound 模式策略 | 新增该模式业务，容器解码/下载/编码不重写 |
| HUD 样式 | mode/osr + 当前格式配置 | 不改变判定和时间线 |
| 新下载源/鉴权 | CLI/Node 资源适配器 | core 无网络依赖，产物与判定契约不变 |
| 新编码器/输出平台 | 宿主媒体层/适配 crate | 不改回放规则 |

设计限制：当前没有 stable、失败、HR/DT 或转谱回放样本，不能承诺完全复刻所有历史客户端。后续转谱回放由回放 mode 自动选用既有转换器和记录 seed，不开放 --convert 覆盖；需要 fixture 固定转换版本，再注册对应模式能力。需求新增若超出契约，应先更新契约及验收，再考虑更大范围重构。


# OSR 回放预览架构设计：运行时契约、边界与性能

返回入口：[架构设计总览](../osr-architecture.md)。

## 5. 关键数据契约

### 5.1 回放文档

`ReplayDocument` 至少包含：

- `ruleset_id`、文件版本、客户端版本、谱面 MD5、玩家名、日期；
- stable 头部字段：六类计数、总分、最大连击、旧 Mod 位掩码、生命曲线原文；
- lazer 扩展 JSON：`mods`、细分 statistics、maximum statistics、rank、pauses、online id；
- 原始输入帧、随机种子及兼容性元信息；输入归一化由 replay 模块另行完成；
- 不可解码字段、时间溢出、反向时间及截断数据的诊断信息，合法负首帧不视为错误。

解析器必须保留原始头字段和扩展字段，不能只转换成当前 `ScoreSnapshot`，否则后续无法区分 stable/lazer、CL、ScoreV1/V2 或新 Mod。

### 5.2 回放帧与判定输入

现有 `InputSource::snapshot_at(time)` 只能取得某一时刻的快照，不能保证两次输出帧之间的快速按键被处理。回放引擎应增加 `events_between(start, end)` 或等价的有序事件接口：判定引擎按回放时间推进，输出 FPS 只决定画面采样，不决定判定次数。

每个模式将 OSR 原始帧归一化为带时间和原序号的契约。Mania 的列按键编码来自原帧 `x` 的低 20 位，不是通用 `keys` 字段；Taiko 保留四种鼓动作；Catch 保留位置和 Dash；Standard 保留原鼠标/键盘来源位，但判定左右动作按游戏逻辑合并，避免一个键被算作两次点击。边沿事件不得由渲染帧差分临时推导。

同刻帧保持原始顺序，不去重点击；`-12345` 是 seed 标记，不能当作 delta；早期占位帧/负 delta 按版本策略处理，不一律排序后掩盖错误。光标在相邻合法帧间插值，按键阶跃变化；越界光标不裁剪回 playfield，避免把本来打空的输入变成命中。

统一时间域为谱面绝对毫秒，0 对应音频文件 0。回放 delta 累计进入该域，DT/HT 只改变播放速度，不重复缩放回放时间；古老谱面 offset 由兼容策略统一应用一次。用户倍速只改变输出时钟。物件几何使用已应用 Mod/堆叠的游戏逻辑坐标，HR 的谱面变换不能再次作用到已记录的回放光标；实际坐标规则须用 HR fixture 验证。

### 5.3 判定状态与事件

`GameplayState` 包含分数、ACC、当前/最大连击、四模式统计、评级、最近命中误差、回放状态和玩家名。分数使用足够宽的整数；ACC 在 core 为 0..1，百分比和舍入归展示层；没有已判定物件时返回明确的未判定状态，避免除零和提前展示终局评级。

`ObjectState` 包含稳定 `ObjectId`/嵌套节点 ID、实际命中/结束时刻、滑条追踪区间、转盘角度/进度和音效所需状态。`GameplayEvent` 至少包含事件 ID、时间、对象/节点 ID、判定种类、命中误差、断连和失败；对象终判、滑条 tick、ComboBreak 与奖励分开表达，现有五档 `Judgement` 不能覆盖所有模式。

公共契约只固定对象/节点 ID、时间域、状态索引和事件窗口；模式专用结果以 `ModeObjectState`/`ModeEvent` 的枚举分支承载。Standard 的 slider/spinner 数据不能成为其他模式的必填字段；增加模式允许扩展该枚举及该模式 renderer，而不能迫使其他模式解释陌生字段。

`calculate/<mode>/` 输出状态与事件；`gameplay::engine` 在加载阶段单向编译整段回放。内部按回放输入、物件节点、超时点及连续积分所需步长推进；滑条和转盘的积分策略版本化并用 fixture 验证，不能假设只采按键边沿足够，也不能假设固定 1ms 就完全复刻游戏。

编译结果 `GameplayTimeline` 是不可变共享数据，提供 `state_at(t)`、`object_state_at(id,t)`、`events_between(a,b)` 和追踪/转盘轨迹查询。稀疏状态索引和事件轨迹支持二分查询，不保存每个输出像素帧。CLI workers、Web 画面和音频各有独立查询游标；音频预读不能推动 HUD 提前显示。未来实时游玩可复用同一规则引擎，但把编译器换成增量驱动，不要求实时游玩预计算未来。

对外事件窗口统一为 `(a,b]`，状态查询包含 `t` 时刻已完成的事件。内部同刻次序由规则策略定义为明确的 `(time, phase, input_sequence, object_id)`，截止点等号归属用命中窗边界测试固定，不能依赖 HashMap 遍历顺序；音频和渲染不用自行决定判定次序。

### 5.4 规则变体与 Mod

兼容性按实际规则内容选择，不以客户端名称、平台或分数版本建立白名单。原生 Standard 接受 tachyon、历史 lazer 和 stable；按官方 `LegacyScoreDecoder`，stable 自动附加 CL，lazer 的显式 CL 使用同一策略。AL、其它尚未实现 Mod 和未知 settings 仍返回具体的 `UnsupportedReplayVariant`，不静默过滤。

当前实现使用官方当前重播的标准化计分。原始客户端、文件版本、Mod、统计与总分独立保留，完整回放终态比较只返回结构化差异，不覆盖重算状态，也不因差异阻止播放。历史帧使用官方的启动帧修复、占位帧移除和倒退帧丢弃；早期 lazer 的小数 delta 逐帧舍入，同刻帧仍保留原顺序。暂停记录是墙钟统计，不重复修改帧的谱面时间。

| 回放组合 | 当前入口结果 | 实施验收要求 |
| --- | --- | --- |
| 原生 Standard，无 Mod，任意客户端/分数版本 | 播放 | 当前规则重播，保留原记录与重算差异 |
| stable，无额外 Mod | 自动 CL | 与显式 CL 使用相同判定与计分 |
| lazer 显式 CL | 播放 | 支持 Classic 滑条比例判定、note lock、提前淡出和尾音设置 |
| Standard EZ/HR/HD/FL/AT/DA/TC/DT/HT/NC/DC，及兼容的 CL 组合 | 播放 | 与普通动画预览共用 Mod 范围，应用难度、几何、可见性、时钟和当前官方计分倍率 |
| AL、其它未实现 Mod 或未知 settings | Unsupported | 返回实际未实现规则，不过滤后播放 |
| Taiko/Catch/Mania | 允许格式解析，播放 UnsupportedMode | 后续各模式增加计算和渲染 |
| 非法长度/压缩/帧 token | InvalidReplay | 加载阶段停止，不当作游玩失败 |

CL 策略由 `replay/compatibility` 选择并保存在不可变 `GameplayTimeline.policy`；`calculate/standard/classic` 定义滑条比例判定，`gameplay/engine/compile/classic` 处理嵌套结果和 note lock，绘制和混音消费同一策略。`classic_health` 保留，但预览仅按回放输入提前结束产生失败，不另外模拟 HP 失败。扩展其它 Mod 时在规则层实现，复用显示与音频契约。


## 6. 渲染、缩放与性能

现有四模式谱面绘制的几何、精灵、布局和背景逻辑复用。各模式 `osr.rs` 负责回放对象状态映射、光标/按键、命中闪光、判定弹字、分数/ACC/连击/评级、玩家名和失败提示；基础物件绘制必须接收真实命中/消退时刻、滑条 tracking、转盘旋转和 FL 的回放光标。只在已有 autoplay 画面上叠 HUD 会让提前/晚打/Miss 的画面不正确，必须修改对应绘制边界。CPU/WGPU 共用逻辑状态与布局契约，各自生成 Img/FrameScene；不复制整套绘制实现。

所有回放 HUD 尺寸、位置和字体大小在绘制前使用当前模式、当前输出格式的 `SCALE`；不得把 Standard 的缩放配置借给 Mania/Taiko/Catch，也不得在整张图完成后再统一缩放。

普通预览路径不创建回放输入、引擎或结果列表，保留现有纯预览入口与每线程缓存。回放路径加载时由 `preparation` 一次准备难度、HR、堆叠、逻辑路径和嵌套节点，calculate 和 render 共享同一对象 ID 和几何；像素路径、皮肤缓存和 SCALE 属于绘制层。共享准备策略必须显式区分 Preview/Replay 兼容语义，不能为迁移改变原预览画面。

GIF/MP4 的每个 worker 只读取准备结果和 `GameplayTimeline`，禁止共享可变引擎、互斥锁串行产帧或按输出 FPS 重判。回放加载有一次顺序编译成本，结果内存随输入/事件/轨迹增长，必须设置解压、帧数和结果大小限制；Web 大回放的解析与编译放 Worker，完成后再启动播放。普通预览的复杂度、分配与吞吐量应保持原有量级，具体无回退由基线对比验收，设计本身不能代替测量。

音频事件按时间轴统一混音。回放命中音不能由绘制触发；MP4 从事件时间轴离线混音，Web 保留 PreviewClock 和 audioEpoch，声音与画面查询同一时间域。seek 只换时钟/查询位置并重置声音流，不重新编译回放；在滑条/转盘中途 seek 应恢复当前循环及播放相位，不补播过去的一次性音效。


## 7. Hitsound 规则

预览模式继续使用现有 `hitsound::build_timeline`：它按照谱面和 autoplay 展开音乐、滑条、tick、spinner 等事件。回放模式增加 `hitsound::replay`，以 `GameplayEvent` 为输入：

- 成功命中使用物件/节点样本和 timing point 音量；
- 普通 Miss 不播放成功命中音；断连音按 `ComboBreakPolicy` 处理，而不是每次 Miss 播放 failsound。参考 `ComboEffects`：连击归零且旧连击 >20，或启用首次断连提示时播放 `Gameplay/combobreak`；失败状态单独产生 `Gameplay/failsound`；
- 滑条头、重复点、tick、尾部和转盘奖励由判定事件驱动，不能把整条滑条当作必命中；
- seek、暂停、倒放和重复渲染必须通过事件纪元或事件 ID 去重，避免重复播放；
- 样本来源仍为“谱面包同名条目 > 内嵌皮肤”，读取失败按现有静音降级。

样本选择时刻与实际播放时刻分开：游戏可使用物件/节点及 control point leniency 解析样本，但早打/晚打的起音以判定事件为准。Classic 的“尾部未命中仍可播放尾音”由将来的 CL 策略声明，不用全局 `IsHit` 开关覆盖。滑行/旋转音的开始、结束、音高轨迹来自真实 tracking/rotation，复用 mixer 并补足分段变调/相位契约；现有 autoplay 转盘音高斜坡不能直接用作回放。

回放提前失败后，停止新的玩家命中/ComboBreak 音效；已经开始的样本是否自然播放完由统一混音器决定。谱面音乐继续按预览时间轴播放。


## 8. 失败、截断和边界语义

游玩状态分为 `Playing`、`Completed`、`Failed`；格式/来源/兼容性错误经 Result 返回，不和游玩状态混合。先验证压缩流、长度和帧结构完整，再比较最后有效输入时刻 `L` 与最后物件的真实结束时刻 `E`：`L < E` 按用户约定视为提前失败（退出也归入该展示策略）；`L >= E` 可继续结算已经到达的物件宽限窗，再进入 Completed。终止 seed 不参与 L；合法的起始负帧不改变 L。不能只因最后 Miss 窗晚于 L 就误判完整回放失败。

宽限结算的终点 `D` 是已开始对象/节点的规则最终截止最大值；当 `L >= E` 时继续无输入结算到 `max(L,D)`，不生成点击、只处理既有状态和超时。输出末尾至少包含该结算时刻及配置的余韵，GIF 选段/MP4 完整范围/Web 时钟都从同一计划取得；失败分支在 L 即冻结，不走 D 后补判。

提前失败在 L 时刻处理同刻输入/节点后，产生一次状态转移：

1. 在失败时刻产生一次 `Fail` 事件并显示失败状态；
2. 冻结分数、ACC、连击和评级；最后已知光标可保留展示，但按键清零并停止追踪/旋转循环，不能无限按住末帧；
3. 不再产生玩家命中音、Miss 音和 ComboBreak 音；
4. 基础物件显示从失败时刻起走 Preview 生命周期，继续背景/故事板/音乐；保留失败时刻以前的短暂判定反馈和失败 HUD，不继续计算剩余 Miss；
5. seek 回到失败前查询相应历史状态，重置音频消费纪元；再次正向越过失败点时重新出现提示和一次失败音，不重新编译；
6. 空帧、非法 LZMA、残缺 token 为 InvalidReplay，无匹配 MD5 为 BeatmapMismatch，未支持模式/Mod 为 Unsupported；这些加载错误不进入失败预览。

不能用生命曲线是否存在判断失败：官方样本中生命曲线为空；也不能把第一帧负时间裁剪为 0，负时间是实际 OSR 数据的一部分。


## 9. 参数与输出规则

参数模型建议拆成：

```text
CommonOptions  = type, osu_file, osz_file, fmt, time, duration,
                 fps, scale, output_dir, cache, logging, config
OsuOptions     = bid, convert, repeated mod
OsrOptions     = File(osr_file) | Score(score_id) | Player(user | user_id, bid, score_select=best|recent)
```

`type` 允许空值并默认 `osu`；文档和实现统一使用 `osr`，不使用需求中的 `orz` 拼写。`--bid` 用于 osu 下载/选择流程，OSR 分支仅在玩家查询来源接受；本地回放与成绩 ID 来源由回放及成绩确定谱面。

OSR 的 `fmt` 只允许 `gif`/`mp4`，缺省为 `mp4`；`png`、`convert`、`mod` 在 OSR 分支的参数校验阶段拒绝。输出目录配置 hash 只包含配置项；输入 OSR 的身份在产物名称或记录中表达，命令行参数本身不改变配置 hash，遵守现有缓存规则。

请求规范化为 `PreviewRequest { common, kind: OsuRequest | OsrRequest }`，不要用一个结构保存互相冲突的 Option。raw CLI 参数保留“是否出现”，所以 `--convert=standard`、`--mod=NM` 在 OSR 下也要拒绝；库 API 和 WASM 构造使用同一内容校验。`fmt` 属于导出端口，Web 使用实时端口，不把 `web` 塞进 CLI 的 `fmt`。

| 参数组合 | 规则 |
| --- | --- |
| 省略 type 或 `--type=` | osu；裸 `--type` 缺值按参数错误处理 |
| osu + osr-file | 拒绝，不能自动猜 type |
| osr + osr-file + 可选 osu/osz | 接受，谱面按 MD5 绑定；缺 OSZ 必须下载 |
| osr + score-id | 下载指定统一成绩 ID 的回放并复用成绩谱面定位 |
| osr + user/user-id + bid | 默认官方个人最高分，recent 仅查近期最多 1000 条；查不到不回退 |
| osr 来源缺失或重复 | 拒绝，三个来源互斥；user 与 user-id 也互斥 |
| osr + convert/mod/png | 拒绝；本地或成绩 ID 来源附带 bid 也拒绝 |
| osr + GIF | 必须获得 OSZ（用户指定的资源规则），GIF 只输出画面、无音轨 |
| osr + Web 会话 | 双资源构造，服务端只凭 checksum 获取谱面；播放/暂停/seek/用户倍速可用，set_mods/convert 在 API 层也拒绝 |

回放产物名称加入基于 OSR 内容的稳定身份（如 SHA-256 短码）、格式和选段信息，避免同一谱面的不同回放覆盖；原始用户名不能直接拼路径。解析/判定结果若后续持久缓存，键必须包含 OSR 摘要、谱面 MD5 和规则实现版本；本轮不新增持久结果缓存。


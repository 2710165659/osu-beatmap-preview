# 第一阶段 OSR 回放预览

2026-10-15。当前实现提供 CLI 的原生 osu!standard 回放 GIF/MP4 导出，以及 Web 的本地/在线 OSR 播放。架构提案见 [osr-architecture.md](osr-architecture.md)，本文件记录已经实现的范围与验收证据。

## 使用

```powershell
# 同时提供回放和谱面包，按回放内的 MD5 自动选中难度；默认输出完整 MP4。
.\target\debug\osu-beatmap-preview-cli.exe --type=osr --osr-file=C:/maps/play.osr --osz-file=C:/maps/map.osz

# 可选外部原始 .osu，必须与回放和包内谱面 MD5 都一致。
.\target\debug\osu-beatmap-preview-cli.exe --type=osr --osr-file=C:/maps/play.osr --osu-file=C:/maps/map.osu --osz-file=C:/maps/map.osz --time-points=29 --duration-time=5 --fps=30

# GIF 可按给定顺序拼接多个片段，包括倒序片段；GIF 没有音轨。
.\target\debug\osu-beatmap-preview-cli.exe --type=osr --osr-file=C:/maps/play.osr --osz-file=C:/maps/map.osz --fmt=gif --time-points=29 --time-points=10 --duration-time=1
```

省略 `--type` 仍使用普通谱面预览。OSR 来源严格三选一：

| 来源 | 参数 |
| --- | --- |
| 本地回放 | `--type=osr --osr-file=play.osr` |
| 指定成绩 | `--type=osr --score-id=6086051920` |
| 玩家 + 谱面 | `--type=osr --user="玩家名" --bid=4903207 [--score-select=best|recent]`，或用 `--user-id` 替代 `--user` |

玩家查询默认 `best`，通过官方个人谱面成绩接口取最高分（不是最高 PP）；`recent` 分页扫描官方近期最多 1000 条成绩，含失败记录，按结束时间取该谱面最新一次。查不到、无回放或不兼容时明确报错，不跳过所选成绩或回退到另一条。指定成绩通过 `GET /api/v2/scores/<ID>` 取得谱面信息，再经 `GET /api/v2/scores/<ID>/download` 下载 OSR；显式请求 `x-api-version: 20220705` 使用统一成绩 ID。在线回放缓存与谱面包缓存分开，`--no-cache` 跳过文件缓存，成绩定位每次重新查询。

OSR 拒绝 `--convert`、所有用户 `--mod`（包括 `NM`）和 PNG；`--bid` 仅用于玩家来源。数值时间点沿用首个物件为零的游戏时间轴，`preview` 使用谱面 `PreviewTime`。MP4 只允许一个起点；省略时长时包括完整回放结算与配置余韵。

## 谱面来源

按 `.osu` 原始字节计算 MD5，换行、编码或任意编辑都可能改变摘要；不以谱面 ID 替代摘要。显式外部 `.osu` 不匹配时直接报错，不自动替换。

- 有本地 OSZ：选取包内 MD5 匹配的顶层 `.osu`，多个难度无需额外指定 ID；媒体取自同包。
- 本地回放缺少 OSZ：通过官方 `GET /api/v2/beatmaps/lookup?checksum=...` 查询谱面与谱面集 ID，再调用现有谱面包下载器取得完整包。也缺少 `.osu` 时，先下载官方原文并核验 MD5，再核验包内谱面。
- 在线成绩已提供谱面和谱面集 ID 时直接复用，不重复查询 checksum；本地资源仍先核验 MD5。
- 查询与回放下载使用进程环境变量 `OSU_API_TOKEN`，或 `OSU_CLIENT_ID` 与 `OSU_CLIENT_SECRET` 换取 public OAuth token。认证数据不会输出到错误、日志或产物。
- 查询、下载、MD5 不匹配、歌曲缺失/空文件/超限和音频无法解码都返回错误；缓存命中也检查当前包内歌曲。

## 已验证规则

原生 Standard 不再限制客户端名称、平台或 OSR 分数版本；包括 tachyon、旧 lazer 和 stable。stable 文件（版本小于 `30000000`）按官方 `LegacyScoreDecoder` 自动启用 CL；lazer 显式 CL 使用同一策略，并保留 `no_slider_head_accuracy`、`classic_note_lock`、`always_play_tail_sample`、`fade_hit_circle_early` 设置。`classic_health` 保留但预览不模拟 HP 失败。CL 滑条头使用 LargeTick 判定，尾使用不改变连击的 SmallTick，父滑条按嵌套物件命中比例判为 300/100/50/Miss；支持 Classic note lock、提前淡出及默认漏尾仍播放尾音。AL 和其它未实现 Mod/未知 settings 仍明确报错；其它模式只解析容器。用户不能通过 `--mod` 覆盖录制规则。

OSR 头部、限长 LZMA、负首帧、seed、输入帧、扩展 JSON 与未知扩展字段分别解析。旧版 32/64 位在线成绩 ID、历史小数帧间隔及可选扩展字段均按格式读取；引导占位帧和反向时间按官方解码器归一化，保留同刻帧原始顺序。暂停统计不再次加进已经处于谱面时间域的帧。共享准备层负责逻辑路径、堆叠、精确结束时刻和滑条嵌套节点；玩法编译器按原帧顺序处理按键边沿，同刻多帧保留各自点击位置。连续追踪/旋转使用策略内固定 1ms 网格，分数不依赖输出 FPS、音频预读、倒序查询或 worker 顺序。

分数、ACC、连击、评级从事件独立重算。与官方当前重播一样，历史成绩统一使用当前标准化计分；原记录与重播结果可以不同。完整回放比较终态，所有差异通过 stdout JSON 的 `replay-info.recorded-result-differences` 返回，不再以 `ReplayResultMismatch` 阻止播放，也不在最后一帧覆盖重算状态。版本号不再要求登记白名单；新增 Mod 仍须实现其规则和验证样本。

## 画面与音效

HUD 覆盖在原有 Gameplay 画布上，不挤占游戏区域；所有尺寸使用当前 Standard、当前 GIF/MP4 的 SCALE。右上分数、评级、ACC，左下连击，右下游戏时间；没有玩家名或上下粉边。HUD 是调整前的 1.5 倍、时间标签为 2 倍，右侧按键按钮保持尺寸。评级显示 `SS/S/A/B/C/D/F`，将 lazer `X/XH` 映射为 `SS`，`SH` 映射为 `S`。

光标复用 Autoplay 的精灵、拖尾和点击弹性。反馈使用 Torus Bold 原字体、Argon 颜色、环形粒子、加法合成及对应缓动，在实际点击位置显示主判定；滑条嵌套漏判使用 Argon 的实心圆。按用户要求保留 GREAT（300）反馈；官方 Argon Pro 默认会隐藏该反馈。随机粒子使用稳定事件 ID，避免 seek、并行或 FPS 改变随机结果；CPU 光栅化与官方 GPU 输出仍可能有细微差异，不宣称逐像素相同。

完整内嵌 `Torus-Bold.bin` 与 `Torus-Bold_0.png`，共 323018 字节（315.45 KiB），没有裁剪字符。`advance.replay.ARGON_FONT_DIRECTORY` 可覆盖字体目录；显式损坏资源报错。字体与回放提示音的原许可证保留在 `assets/fonts/argon`、`assets/hitsound/replay`，详见 [第三方声明](THIRD_PARTY_NOTICES.md)。

回放 hitsound 由成功事件与实际追踪/旋转状态触发，CL 默认在漏尾时也播放尾音且父滑条不重复起音；滑条循环和转盘音高查询相同时间线。转盘静止时停止发声，再次旋转重启样本；连续分段保留相位，选段起点恢复在响循环，不补播过去的一次性音效。断连与失败使用 Argon 提示音，谱面自带同名样本优先。

回放输入提前结束时产生一次失败：冻结成绩与最后光标、清空按键、停止玩家一次性音效及追踪/旋转循环，显示 F 和失败提示；基础物件改走普通预览生命周期，背景、故事板和音乐继续。格式损坏或来源不匹配属于加载错误，不展示为游戏失败。

## 验收记录

本地样本与临时官方对照程序位于忽略目录 `.local/osr-samples`，没有进入生产构建或普通测试。官方源码参考 `E:/MyCodes/c#/PROJECTS/osu`，相关滑条/回放输入源码与 `2026.119.0` tag 对照无差异。

真实样本：`xuan_yuan-osu-6086051920.osr`，谱面 `4903207.osu`，谱面包 `2293442.osz`，MD5 `0e1620f38ca941ea5d1c4c3e829abe55`，9265 帧。

| 指标 | OSR 记录 | 官方 Headless ReplayPlayer 重播 | 当前实现 |
| --- | ---: | ---: | ---: |
| 分数 | 988187 | 991262 | 991262 |
| 最大连击 | 728 | 729 | 729 |
| ACC | 99.6295198577% | 99.7036158862% | 99.7036158862% |
| slider_tail_hit | 102 | 103 | 103 |
| ignore_miss | 3 | 2 | 2 |
| 评级 | S | S | S |

圆圈 519、滑条头 103、尾 103、tick 3，共 728 项逐事件比较：结果和谱面节点时刻一致；点击时刻一致，连续节点与官方 60Hz Headless 轮询相差不超过 17ms。转盘终态统计一致；合成测试补充旋转发声启停、方向反转、积分、最大 bonus 音效和失败清理。当前官方真样本仍只有一份无 Mod Standard，不能证明所有边界或历史规则已兼容。

- core：418 项普通测试通过，3 项本地样本测试显式忽略；五份官方样本容器解析、实际 Standard 规则对照与 seek 后画面重复性另行运行通过。
- CLI：188 项测试通过，包括来源优先级、假下载失败、缺失歌曲、格式互斥、字体内嵌和媒体选段；自动测试不访问实时 API。
- workspace 编译、无 replay 特性的 core、`wasm32-unknown-unknown` 编译通过；core/CLI clippy 全目标 `-D warnings` 通过。workspace clippy 被未修改的 WASM `remux.rs` 原有两条 lint 阻止（`needless_question_mark`、`manual_div_ceil`）。
- 默认完整 MP4：826×464、15 FPS、2258 帧、150.533333 秒，H.264 视频与 AAC 音频时长一致；另验证 30 FPS 五秒片段。
- CPU 编码回退：设置 `OSU_PREVIEW_NO_GPU=1`、`--scale=0.5`，实际输出 414×232、30 FPS、1 秒，音视频长度一致；规则终态与默认 SCALE/FPS 相同。
- GIF：29 秒和 10 秒两个倒序片段，15 FPS、30 帧、2 秒，背景正常。
- 在 30.992 秒截短帧并保留完整合法容器：失败前 149671 分、151 连击；失败后仍显示该成绩和 F，按键清空，后续谱面可见；31 秒以后两秒音轨 RMS 为 0.357，歌曲继续。玩家音效停止由 PCM 隔离测试验证。
- 普通 `.osu` 对照：基线 `126f0e4` 与当前同参数一秒 GIF 的 SHA-256 都为 `50cec5f8c84a51f8261b8f23d27938b80977dc4819a8113ef49e25a245d40181`。同机 debug 构建、一次预热后交替各测五次，中位耗时 3.222 秒与 3.146 秒；当前/基线比值 0.976。单样本测量只证明该场景未见退化。

重复规则样本验证：先用本地官方 Headless 项目生成 `oracle-results.json`，再设置 `OSR_SAMPLE_DIR` 运行对应 ignored 测试。截图测试还需设置 `OSR_SCREENSHOT_DIR`，只在测试构建中写入验收图像。

```powershell
$env:OSR_SAMPLE_DIR='E:/MyCodes/rust/osu-beatmap-preview/.local/osr-samples'
cargo test -p osu-beatmap-preview-core --features replay official_standard_sample_matches_official_replay_results -- --ignored --nocapture
```


## Web 回放

加载页选择本地 OSR、成绩 ID/新式官方链接或玩家 + 谱面 ID，三种来源互斥，再按需要选择匹配 `.osu/.osz`。本地 OSZ 按回放 MD5 自动选中难度；本地 `.osu` 校验通过后下载完整包以提供音乐和背景。没有本地谱面时，后端调用官方 checksum 查询定位谱面，再下载完整 OSZ。所有查询、下载和 MD5 错误都显示给用户，不回退到错谱或无音乐播放。

自动查谱需要在 Node 后端进程设置 `OSU_API_TOKEN`，或 `OSU_CLIENT_ID` + `OSU_CLIENT_SECRET`；本地匹配 OSZ 不需要 API 凭据。浏览器不上传 OSR 内容，查询请求只有谱面 checksum。回放模式隐藏转谱、难度选择和 Mod 控件；播放、暂停、进度拖动、倍速、音量、背景、清晰度和 FPS 控件继续可用。

WASM 新增 `replayInfo(bytes)`、`WebGpuSession.create(..., { replayBytes })` 和 `session.replayState()`。解析、兼容性检查、LZMA、MD5 选谱、判定、音乐/音效混音均在 WASM；绘制使用原生 WebGPU 物件场景及共享 CPU/WebGPU Argon 小精灵参数，不逐帧上传完整画面位图。分辨率切换按当前 Standard MP4 SCALE 重新准备几何，原回放判定保持不变。

- Node：49 项测试通过；隔离测试覆盖官方 OAuth 单飞、token 缓存、checksum 不符、无凭据、资源选择、下载错误与回放选择限制；不访问实时 API。
- core：420 项测试通过、3 项样本测试显式忽略；WASM：26 项通过、1 项外部媒体测试忽略。新增 Rust 测试覆盖精确原始字节 MD5 选谱、包内回放提示音、字体完整字符、回放 seek/音频预读/分辨率往返不改成绩、失败后继续音乐与物件预览。
- Windows Edge 无头浏览器用真实官方 Standard 样本验证：最终 `991262 / 729 / 99.7036% / S`；混音峰值 `0.7863`、`crossOriginIsolated=true`，页面异常为零。验证了背景、判定反馈、失败冻结、进度拖回失败前恢复、480P/720P 切换、AL 拒绝、错谱和缺凭据报错、注入网络响应的 OSR 单文件加载，以及恢复普通谱面预览和 Mod 控件。
- 已使用真实 OAuth 凭据验证官方成绩查询、玩家最高分和回放下载；真实近期记录缺少目标谱面时按预期报错。Web 经真实后端下载 OSR 后播放，谱面包使用本地真实样本；最终 991262 / 729 / 99.7036% / S。凭据未写入项目。
- 本地验收脚本 `.local/web-osr/browser-check.py` 与 `web-osr-preview.png`、`web-osr-failed.png` 截图只作开发验收，不参与生产包。

## 在线来源增量验收

CLI 的三种来源由 `ReplaySource` 枚举表达，参数互斥与输出限制在文件/网络读取前校验。认证、成绩选择和回放下载拆分为独立模块，core 判定与渲染未改动。近期查询没有无限历史语义，固定最多扫描 1000 条；选中的成绩无回放或不兼容时不改选。

CLI 新增本地 HTTP 契约测试，验证统一成绩格式、数字用户名、最高分、近期绝对时间、无回放、MD5、OSR 缓存及 no-cache；不读取真实凭据。Web 新增来源组合、官方链接解析、近期分页与扫描上限、错误成绩身份、并发下载与坏缓存恢复测试；不访问实时 API。

本轮 CLI 共 195 项测试通过，Web 共 49 项通过，CLI all-target Clippy（-D warnings）与前端构建通过。Edge 无头浏览器验证三种来源、最高分/最近、无回放、AL 拒绝、切换本地与普通预览，页面异常为零。真实凭据下 CLI 按成绩 ID 下载并导出一秒 GIF 成功；官方 API 验收请求量低于每分钟 60 次。

## 版本限制移除与 CL 增量验收

- 客户端名称、平台和 OSR 分数版本不再作为播放白名单；覆盖稳定版日期格式、早期无扩展 lazer、30000016/30000019 和未知未来版本。限长解压、MD5 绑定、非法输入和未实现 Mod 检查仍保留。
- Standard 普通测试 430 项通过、5 项外部样本测试显式忽略；CLI 195 项、WASM 26 项通过，Web Node 50 项通过。core/CLI all-target Clippy `-D warnings`、无 replay 特性构建及 Web WASM/前端构建通过。
- mrekk 的真实 tachyon 回放：成绩 `7346217987`、谱面 `5774158`、客户端 `2026.821.0-tachyon-windows`，已在原生计算和 Edge WebGPU 实际播放中通过；终态 `988924 / 4094 / 99.6285059749% / S`。
- 基于原有真实 Standard 输入构造显式 CL 与 stable 容器，测试两者走同一规则。官方 Headless 的判定统计、ACC 和最大连击与本实现一致：620 Great、3 Ok、106 LargeTickHit、103 SmallTickHit、最大连击 729、ACC 99.6860446863%。这两份是合成规则样本，不宣称为玩家实际录制的 stable 成绩。
- 本地官方二进制采用旧 CL 0.96 倍率，得到 951110 分；当前参考源码 `OsuScoreMultiplierCalculatorV2` 对开启 classic note lock 的 CL 使用 0.985，关闭时使用 0.96。本实现按当前源码先取整无 Mod 分数，再应用倍率；默认 CL 重播为 975879 分。与旧二进制按倍率换算后的结果相差不超过 1 分。
- 显式 CL、隐式 CL 与真实 tachyon 三份回放在 Edge WebGPU 中都能结算且无页面异常；stable 格式通过 CLI GIF 导出。原有无 Mod 官方样本仍为 `991262 / 729 / 99.7036158862% / S`。
- 合成普通测试覆盖 Classic 滑条比例判定、note lock 开关、滑条头精度开关、漏尾默认播放尾音及关闭该设置、尾音不重复、历史小数 delta、占位帧/倒退帧、缺省扩展和成绩差异只作诊断。外部规则验收位于 `gameplay/engine/compile/classic/tests.rs`，依赖环境变量显式运行；辅助项目、原始回放和浏览器脚本都在忽略目录 `.local/`。

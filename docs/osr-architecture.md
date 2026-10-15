# OSR 回放预览架构设计

日期：2026-10-13。本文保留原架构设计与后续阶段目标；第一阶段 CLI 原生 Standard OSR 实施范围、使用方法与验收见 [OSR 回放预览](osr-preview.md)。流程：需求确认 → 当前架构与游戏源码核对 → 实际样本验证 → 边界与契约设计 → 分阶段验收。

本文将 OSR 回放从文件读取、谱面绑定、规则计算、画面反馈、音效和媒体输出拆成可独立演进的模块。正文按职责拆分，避免把后续需求继续堆入单个大文件。

## 阅读顺序

1. [概览与实际业务流程](osr-architecture/01-overview.md)：范围、输入来源、数据流和现有代码依据。
2. [模块划分与依赖规则](osr-architecture/02-modules-and-dependencies.md)：预期目录、源码依赖 DAG、文件体量规则。
3. [运行时契约、边界与性能](osr-architecture/03-runtime-contracts.md)：OSR 数据、判定、渲染、音效、失败和参数契约。
4. [迁移、测试与验收](osr-architecture/04-migration-and-validation.md)：现有文件迁移、测试策略、样本和需求变化映射。

## 关键决策

- `--type` 只使用 `osu`/`osr`，默认 `osu`；需求中的 `orz` 视为笔误。
- OSR 只接受手动 `--osr-file`，允许同时提供 `.osu`/`.osz`；缺少谱面按 OSR MD5 查询，缺少 OSZ 则联网下载，下载失败报错。
- OSR 允许 `gif`/`mp4`，默认 `mp4`；OSR 拒绝 `png`、`convert` 和用户 `mod`。
- 当前只做原生 Standard 回放判定；客户端与分数版本不设白名单，stable 自动 CL、显式 CL 共用策略；AL、其它未实现 Mod 与未知 settings 明确拒绝并保留扩展点。
- 回放结果先编译成不可变时间线；画面、音频和并行导出独立查询，避免 FPS、音频预读或线程顺序改变判定。
- 回放提前结束统一展示失败：冻结回放状态和玩家音效，谱面、背景、故事板与音乐继续。
- 普通 osu 预览不创建回放引擎，不改变原有路径的复杂度和输出。

## 外部样本

架构分析样本位于 `.local/osr-samples/`，目录被 `.gitignore` 排除。样本说明、下载链、解析统计和限制见 [.local/osr-samples/README.md](../.local/osr-samples/README.md)。认证信息只用于下载，不写入仓库、文档、日志或测试 fixture。

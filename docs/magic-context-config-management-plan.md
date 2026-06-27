# Magic Context 配置管理技术方案

## 背景

本方案用于在 AI Toolbox 中为 Magic Context 增加配置管理能力。目标是让 OpenCode 和 Pi 在检测到对应插件或扩展已安装时，展示类似 Oh-My-OpenAgent 配置管理的配置卡片，并提供语义化表单和 JSONC 高级编辑能力。

本方案只描述技术设计和实现边界，不直接等同于最终实现承诺。后续应按本文拆分任务逐步落地。

## 研究来源

### 上游仓库

已下载并研究 `https://github.com/cortexkit/magic-context`，本次研究基于：

- 本地路径：`/tmp/magic-context`
- 当前提交：`4119f89`
- 包版本：
  - `@cortexkit/opencode-magic-context`: `0.29.1`
  - `@cortexkit/pi-magic-context`: `0.29.1`
  - `@cortexkit/magic-context`: `0.29.1`

重点阅读文件：

- `/tmp/magic-context/CONFIGURATION.md`
- `/tmp/magic-context/assets/magic-context.schema.json`
- `/tmp/magic-context/packages/plugin/src/config/schema/magic-context.ts`
- `/tmp/magic-context/packages/plugin/src/config/schema/agent-overrides.ts`
- `/tmp/magic-context/packages/plugin/src/config/index.ts`
- `/tmp/magic-context/packages/pi-plugin/src/config/index.ts`
- `/tmp/magic-context/packages/plugin/src/config/migrate-config-location.ts`
- `/tmp/magic-context/packages/plugin/src/config/project-security.ts`
- `/tmp/magic-context/packages/plugin/src/config/variable.ts`
- `/tmp/magic-context/packages/plugin/src/index.ts`
- `/tmp/magic-context/packages/pi-plugin/src/index.ts`
- `/tmp/magic-context/packages/dashboard/src/components/ConfigEditor/ConfigEditor.tsx`

### 当前项目

已核对当前 AI Toolbox 相关模块：

- `web/features/coding/opencode/AGENTS.md`
- `web/features/coding/shared/AGENTS.md`
- `tauri/src/coding/open_code/AGENTS.md`
- `tauri/src/coding/pi/AGENTS.md`
- `tauri/src/coding/wsl/AGENTS.md`
- `web/features/coding/opencode/components/OhMyOpenAgentSettings.tsx`
- `web/features/coding/opencode/components/OhMyOpenAgentConfigCard.tsx`
- `web/features/coding/opencode/components/PluginSettings.tsx`
- `web/features/coding/pi/components/PiExtensionsSection.tsx`
- `web/components/common/JsonEditor/index.tsx`

## 关键事实

### Source of Truth

Magic Context 的配置文件不是 OpenCode 自身配置，也不是 Pi 扩展目录下的文件。OpenCode plugin 和 Pi extension 共享同一套 CortexKit 配置位置。

| 范围 | 官方路径 | AI Toolbox 语义 |
|---|---|---|
| 用户级配置 | Unix/WSL: `$XDG_CONFIG_HOME/cortexkit/magic-context.jsonc` 或 `~/.config/cortexkit/magic-context.jsonc`；Windows: `%USERPROFILE%\.config\cortexkit\magic-context.jsonc` | 用户全局默认配置 |
| 项目级配置 | `<project>/.cortexkit/magic-context.jsonc` | 项目覆盖配置，覆盖用户级配置 |
| 共享数据库 | `~/.local/share/cortexkit/magic-context/context.db` | 运行时数据，不作为配置编辑目标 |
| Schema | `https://raw.githubusercontent.com/cortexkit/magic-context/master/assets/magic-context.schema.json` | 可写入 `$schema`，用于编辑器提示和校验 |

加载顺序：

1. 读取用户级配置。
2. 读取项目级配置。
3. 深度合并，项目级覆盖用户级。
4. 对项目级配置先剥离不安全字段。
5. 用 Zod schema 解析，解析失败时尽量恢复有效字段，非法字段回退默认值。

AI Toolbox 不能把“合并后的 effective config”直接写回任一配置文件。用户级文件和项目级文件必须分别编辑、分别保存。

### 历史路径迁移

上游源码会在启动时把旧路径迁移到 CortexKit 共享路径。

旧路径包括：

- OpenCode 用户级：`~/.config/opencode/magic-context.jsonc`
- Pi 用户级：`~/.pi/agent/magic-context.jsonc`
- 项目根：`<project>/magic-context.jsonc`
- OpenCode 项目级：`<project>/.opencode/magic-context.jsonc`
- Pi 项目级：`<project>/.pi/magic-context.jsonc`

目标路径统一为：

- 用户级：Unix/WSL 使用 `$XDG_CONFIG_HOME/cortexkit/magic-context.jsonc` 或 `~/.config/cortexkit/magic-context.jsonc`，Windows 使用 `%USERPROFILE%\.config\cortexkit\magic-context.jsonc`
- 项目级：`<project>/.cortexkit/magic-context.jsonc`

迁移是 move-and-marker 模式，旧文件会被改名为 `<old-name>.MOVED_READPLEASE`。如果旧 OpenCode/Pi 配置存在差异，上游不会擅自合并，会留下文件并警告用户手工整理。

AI Toolbox 第一版不应自行迁移旧路径。更稳妥的做法是：

- 展示当前 CortexKit 路径。
- 如果发现旧路径存在，提示用户运行 `doctor` 或 `setup`。
- 提供“运行 doctor”的操作，而不是自己复制或合并旧文件。

### 文档与源码漂移

本次研究发现 `CONFIGURATION.md` 与当前源码存在少量漂移，方案以源码为准。

重要差异：

| 配置 | `CONFIGURATION.md` 表述 | 当前源码事实 | 设计处理 |
|---|---|---|---|
| `compressor` | 文档仍有 compressor 配置段 | schema 注释明确 v2 已移除 compressor knobs，旧 `compressor` 会被 schema 作为 unknown key 忽略 | 不做结构化表单，只在“旧配置/其他配置”里提示已废弃 |
| `sidekick.enabled` | 文档提到 `enabled` | 当前 schema 没有 `sidekick.enabled`，loader 会把历史 `sidekick.enabled=false` 迁移为 `sidekick.disable=true` | 表单使用 `sidekick.disable` 的反向开关，不再写 `enabled` |
| Pi 旧 README 路径 | 部分 README 仍写 `~/.pi/agent/magic-context.jsonc` | 当前 Pi loader 读取 CortexKit 共享路径，并只把旧路径作为迁移/回退场景 | AI Toolbox 按 CortexKit 路径做 UI |
| Embedding 字段 | Markdown 主文档只列基础字段 | 源码还支持 `input_type`、`query_input_type`、`truncate`、`max_input_tokens` | 放入 embedding 高级表单或 JSONC 高级区 |

## 产品目标

当前实现范围调整：第一版只提供用户级配置管理。Magic Context 上游仍支持 `<project>/.cortexkit/magic-context.jsonc` 项目级覆盖，但 AI Toolbox 当前不展示项目路径入口，也不提供项目级读写 API。

第一版应完成：

1. OpenCode 页面检测到 `@cortexkit/opencode-magic-context` 后展示 Magic Context 配置管理卡片。
2. Pi 页面检测到 `@cortexkit/pi-magic-context` 后展示同一套配置管理卡片。
3. 用户可以管理用户级配置。
4. 常用字段用语义化表单编辑。
5. 高级字段、未知字段、未来字段用 JSONC 编辑器展示和编辑。
6. 保存时保留未知字段，不丢用户手写配置。
7. 支持运行 `doctor --harness opencode` / `doctor --harness pi`。
8. 支持打开配置文件所在目录、复制路径、创建默认配置。

第一版不做：

- 不编辑 `context.db`。
- 不把 Magic Context 配置写入 OpenCode plugin tuple options。
- 不把 Magic Context 配置写入 Pi extension 目录。
- 不提供项目级配置卡片、项目路径选择或项目级配置读写 API。
- 不自动迁移旧配置路径。
- 不默认同步 SQLite 数据库。
- 不从项目级配置读取或展开 `{env:}`、`{file:}` 这类敏感变量。
- 不实现完整上游 dashboard 的所有数据浏览功能。

## 安装检测

### OpenCode

OpenCode 插件配置类型已有：

```ts
type OpenCodePluginEntry = string | [string, Record<string, unknown>]
```

Magic Context 安装检测只需要判断当前 OpenCode `plugins` 中是否存在等价插件：

- `@cortexkit/opencode-magic-context`
- 带 npm source 或版本号的等价形式

需要复用现有工具函数：

- `getOpenCodePluginName`
- `getOpenCodePluginPackageName`
- `isOpenCodePluginEquivalent`
- `normalizeOpenCodePluginName`

配置卡片展示位置：

- OpenCode 插件管理区域下方。
- 只在插件已安装时显示。
- 如果插件未安装，不显示配置卡片，避免把配置入口误解为内置能力。

### Pi

Pi 扩展事实源是 Pi CLI 输出和当前 runtime root 派生的本地扩展扫描，不是数据库。

安装检测应复用 `PiExtensionsSection` 里的等价判断语义：

- `@cortexkit/pi-magic-context`
- `npm:@cortexkit/pi-magic-context`

配置卡片展示位置：

- Pi 扩展管理区域下方。
- 只在扩展已安装时显示。
- 不从 `~/.pi/agent/extensions` 查找配置，因为 Magic Context 配置不在这里。

## 配置文件与作用域

### 用户级配置

路径：

```text
~/.config/cortexkit/magic-context.jsonc
```

适合管理：

- 全局开关。
- 输出语言。
- 插件自动更新。
- SQLite 调优。
- Embedding endpoint 和 API key。
- 默认 historian/dreamer/sidekick 模型。
- 默认 dreamer schedule。

用户级配置是可信配置。上游允许在用户级配置中展开：

- `{env:VAR}`
- `{file:path}`

AI Toolbox 编辑时应保留这些 token，不能在前端提前替换成真实 secret。

### 项目级配置

路径：

```text
<project>/.cortexkit/magic-context.jsonc
```

适合管理：

- 项目自己的上下文阈值。
- 项目自己的 memory 开关和预算。
- 项目自己的 dreamer schedule。
- 项目自己的模型选择，但不能让项目配置重定向 embedding endpoint。

项目级配置是不可信配置。上游会剥离以下字段：

| 字段 | 原因 |
|---|---|
| `auto_update` | 仓库不能禁止插件安全更新 |
| `language` | 仓库不能通过用户语言偏好注入 prompt 行为 |
| `sqlite` | SQLite PRAGMA 影响进程全局 DB 连接，不能由仓库控制 |
| `embedding.provider` | 仓库不能选择私有文本发往哪个 embedding provider |
| `embedding.endpoint` | 仓库不能重定向私有文本到攻击者 endpoint |
| `historian.prompt` / `dreamer.prompt` / `sidekick.prompt` | 仓库不能重写 hidden agent 指令 |
| `historian.permission` / `dreamer.permission` / `sidekick.permission` | 仓库不能扩大 hidden agent 权限 |
| `historian.tools` / `dreamer.tools` / `sidekick.tools` | 仓库不能重新开启高权限工具 |
| `sidekick.system_prompt` | 仓库不能重写 sidekick system prompt |

AI Toolbox UI 应在项目级配置中隐藏或禁用这些字段的语义化编辑。若 JSONC 高级区里已有这些字段，应保留原文但给出警告：运行时会忽略。

## JSONC 编辑器要求

当前项目的 `web/components/common/JsonEditor/index.tsx` 使用 `JSON.parse` 校验，Monaco 语言也是 `json`。它不能直接胜任 Magic Context 的完整配置编辑，因为 Magic Context 文件是 JSONC。

需要新增或扩展：

```text
web/components/common/JsoncEditor/
```

要求：

- 使用 Monaco，语言优先设为 `json` 或注册 `jsonc`。
- 解析层必须支持注释和 trailing comma。
- 保存时保留原始 JSONC 文本。
- 表单保存可以输出格式化 JSON，但完整编辑模式不能强制删除注释。
- 编辑 `$schema` 时不报错。
- 支持 raw text change 和 parsed object change 两套回调。
- 支持 inline parse error。

后端也需要 JSONC 解析和序列化能力，不能只用 `serde_json::from_str`。

推荐 Rust 侧依赖或实现方向：

- 若已有 JSONC parser，复用现有实现。
- 若没有，后端可先做 comment/trailing comma tolerant parser，用于读写 `serde_json::Value`。
- Raw save 只需语法校验后原文写回。
- Structured save 需要 patch 对象并重新格式化，这一模式可以接受注释丢失，但必须在 UI 明示。

## 配置管理 UI 总体设计

### 展示形态

新增共享组件：

```text
web/features/coding/shared/magicContext/
  MagicContextSettings.tsx
  MagicContextConfigCard.tsx
  MagicContextConfigModal.tsx
  MagicContextJsoncEditor.tsx
  magicContextConfigFields.ts
  magicContextConfigMerge.ts
```

OpenCode 和 Pi 页面只负责：

- 传入 `harness: 'opencode' | 'pi'`。
- 传入插件/扩展是否已安装。
- 传入当前项目路径，若当前页面能解析。
- 刷新当前页面扩展/插件状态。

共享组件负责：

- 读取配置状态。
- 渲染用户级卡片和项目级卡片。
- 打开编辑 modal。
- 调用后端保存。
- 调用 doctor/setup。

### 卡片列表

配置卡片建议固定两张：

| 卡片 | 条件 | 行为 |
|---|---|---|
| 用户配置 | 插件/扩展已安装 | 读写 `~/.config/cortexkit/magic-context.jsonc` |
| 项目覆盖 | 插件/扩展已安装且有项目路径，或用户手动选择项目路径 | 读写 `<project>/.cortexkit/magic-context.jsonc` |

如果文件不存在：

- 卡片仍展示。
- 状态显示“未创建”。
- 主按钮为“创建配置”。
- 可选按钮为“运行 setup”。

如果文件存在但解析失败：

- 卡片显示错误状态。
- 禁用结构化表单保存。
- 允许打开完整 JSONC 编辑器修复。
- 允许打开目录。

### 卡片摘要

每张卡片应显示语义化摘要，而不是直接展示 raw JSON。

建议摘要字段：

- 启用状态：`enabled`
- 语言：`language`
- Historian 模型：`historian.model`
- Dreamer 状态：`dreamer.disable` 和已启用任务数
- Sidekick 状态：`sidekick.model` / `sidekick.disable`
- Embedding provider：`embedding.provider`
- Memory 状态：`memory.enabled`
- 阈值：`execute_threshold_percentage` 或 `execute_threshold_tokens`
- 配置来源：用户级 / 项目级
- 路径状态：存在 / 未创建 / 解析错误 / 有 ignored fields

### Modal 布局

编辑 modal 使用 Tabs 或左侧分组导航：

1. 基础
2. 上下文与清理
3. 记忆与检索
4. Embedding
5. Historian
6. Dreamer
7. Sidekick
8. 高级 JSONC

表单设计原则：

- 常用配置用语义化控件，不让用户直接面对 key。
- 每个字段旁边显示真实 key，用 `Text code` 或辅助说明展示。
- 数值范围使用 `InputNumber`、`Slider`、`Segmented`、`Switch`。
- 模型字段用现有模型选择器或普通输入，第一版可先用输入框。
- `fallback_models` 用可增删列表。
- per-model map 用可增删 key-value 表格。
- 高级 JSONC 保留完整原文。

## 字段语义与 UI 分层

### 基础卡片

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 作用域 |
|---|---|---:|---|---|---|
| `$schema` | string | 无 | 编辑器 schema 提示，不参与运行时配置 | 只读/自动补齐 | 用户级、项目级 |
| `enabled` | boolean | `true` | Magic Context 总开关。关闭后插件大部分 runtime 能力不工作 | Switch | 用户级、项目级 |
| `auto_update` | boolean | `true` | OpenCode plugin wrapper 自更新检查。项目级会被忽略 | Switch | 用户级 |
| `language` | ISO 639-1 string | unset | 控制 historian/dreamer/sidekick/主提示的自然语言输出，结构 token 仍保持英文 | Input 或 Select | 用户级 |
| `toast_duration_ms` | number 0-60000 | `5000` | Magic Context 通知显示时长，0 表示关闭 toast | InputNumber + Slider | 用户级、项目级 |
| `keep_subagents` | boolean | `false` | 调试开关，保留 historian/dreamer/sidekick 等子会话，便于排查但会累积数据 | Switch，放高级区 | 用户级、项目级 |

### 上下文与清理卡片

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 结构化 |
|---|---|---:|---|---|---|
| `ctx_reduce_enabled` | boolean | `true` | 是否向 agent 暴露 `ctx_reduce` 工具和相关提示。关闭后仍有自动清理、historian、memory、search | Switch | 是 |
| `cache_ttl` | string 或 per-model map | `"5m"` | 等待 provider prompt cache 过期后再执行会破坏缓存的变更 | 常用 `default` 输入，高级 per-model 表格 | 是 |
| `execute_threshold_percentage` | number 20-80 或 per-model map | `65` | 上下文使用率达到阈值后强制执行 pending ops。80 上限是 cache safety | Slider + per-model 表格 | 是 |
| `execute_threshold_tokens` | per-model map | 无 | 用绝对 token 阈值替代百分比阈值，适合 provider 实际 prompt cap 小于宣传窗口 | Key-value 表格 | 是 |
| `protected_tags` | number 1-100 | `20` | 最近 N 个 active tags 不会被立即 drop | InputNumber + Slider | 是 |
| `clear_reasoning_age` | number >=10 | `50` | 清理早于 N 个 tags 的 reasoning/thinking blocks | InputNumber | 是 |
| `history_budget_percentage` | number 0.05-0.5 | `0.15` | `<session-history>` 可使用的上下文预算比例 | Slider | 是 |
| `commit_cluster_trigger.enabled` | boolean | `true` | 是否在连续 commit cluster 累积后触发 historian，即使上下文压力还不高 | Switch | 是 |
| `commit_cluster_trigger.min_clusters` | number >=1 | `3` | unsummarized tail 中至少多少个 commit cluster 才触发 historian | InputNumber | 是 |
| `smart_drops` | boolean | `false` | 内容感知清理。删除过期 todo、旧 ctx_reduce、零价值 meta，压缩被新编辑覆盖的旧编辑 | Switch，标实验 | 是 |
| `caveman_text_compression.enabled` | boolean | `false` | 当 `ctx_reduce_enabled=false` 时，对较旧长文本做确定性 caveman 压缩 | Switch，标有损 | 是 |
| `caveman_text_compression.min_chars` | number 100-10000 | `500` | 短于该字符数的文本不压缩 | InputNumber | 是 |
| `temporal_awareness` | boolean | `true` | 注入用户消息间隔标记和 compartment 日期范围，让 agent 感知时间流逝 | Switch | 是 |

### Historian 卡片

Historian 是压缩会话历史并发布 compartments 的后台 agent。它是 Magic Context 的核心能力。

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 结构化 |
|---|---|---:|---|---|---|
| `historian.model` | string | unset | Historian 主模型。建议强提示必须配置，否则部分迁移/recomp 不会运行 | Model Select/Input | 是 |
| `historian.fallback_models` | string 或 string[] | unset | 主模型失败时的 fallback chain | 可排序列表 | 是 |
| `historian.temperature` | number 0-2 | unset | 采样温度 | InputNumber | 是 |
| `historian.top_p` | number 0-1 | unset | nucleus sampling | InputNumber，高级 | 其他配置或高级表单 |
| `historian.two_pass` | boolean | `false` | 成功后再运行 editor pass 清理低价值 `U:` lines 和重复信息，成本更高 | Switch | 是 |
| `historian_timeout_ms` | number >=60000 | `300000` | 单次 historian 调用超时 | InputNumber | 是 |
| `historian.variant` | string | unset | OpenCode only。选择 OpenCode reasoning variant | OpenCode 视图显示 | 是 |
| `historian.thinking_level` | enum | unset | Pi only。传给 Pi subagent `--thinking`，支持 `off/minimal/low/medium/high/xhigh` | Pi 视图显示 | 是 |
| `historian.disallowed_tools` | string[] | `[]` | OpenCode only。从 historian 默认 allow-list 移除工具，支持 `*` | Checkbox 多选，高级 | 是 |
| `historian.prompt` | string | unset | 覆盖或补充 hidden agent prompt。项目级会被剥离 | TextArea，高级 | 是但项目级禁用 |
| `historian.tools` | object | unset | 工具启停 override。项目级会被剥离 | JSONC 高级 | 否 |
| `historian.permission` | object | unset | 权限 override。项目级会被剥离 | JSONC 高级 | 否 |
| `historian.disable` | boolean | unset | 禁用 historian。注意 historian 默认 runnable，只有显式 `disable=true` 才关 | Switch，高级 | 是 |
| `historian.description/mode/color/maxSteps/maxTokens` | mixed | unset | 透传给 hidden agent config，影响展示、模式或调用限制 | JSONC 高级 | 否 |

### Dreamer 卡片

Dreamer 是后台维护任务系统，不是单一 nightly job。每个任务有自己的 cron schedule。

Dreamer agent 基础字段：

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 结构化 |
|---|---|---:|---|---|---|
| `dreamer.model` | string | unset | Dreamer 默认模型，task 可覆盖 | Model Select/Input | 是 |
| `dreamer.fallback_models` | string 或 string[] | unset | Dreamer 默认 fallback chain | 可排序列表 | 是 |
| `dreamer.disable` | boolean | unset | 禁用 dreamer。禁用后包括手动 `/ctx-dream` | Switch | 是 |
| `dreamer.inject_docs` | boolean | `true` | 向 dreamer 注入 `ARCHITECTURE.md` 和 `STRUCTURE.md` | Switch | 是 |
| `dreamer.temperature` | number 0-2 | unset | 采样温度 | InputNumber | 是 |
| `dreamer.thinking_level` | enum | unset | Pi only。默认 task thinking level | Pi 视图显示 | 是 |
| `dreamer.variant` | string | unset | OpenCode only。reasoning variant | OpenCode 视图显示 | 是 |
| `dreamer.prompt/tools/permission` | mixed | unset | Hidden agent prompt 和权限。项目级 prompt/tools/permission 会被剥离 | JSONC 高级 | 否 |

Dreamer tasks：

| Task | 默认 schedule | 作用 | UI 表达 |
|---|---|---|---|
| `map-memories` | `0 2 * * *` | 为 memories 建立对应文件映射，给 verify 提供 gate | 任务行 |
| `verify` | `0 3 * * *` | 只验证映射文件发生变化的 memories | 任务行 |
| `verify-broad` | `0 4 * * 0` | 周期性广泛验证 memory pool | 任务行 |
| `curate` | `0 4 * * 0` | 去重、收紧、归档低价值 memories | 任务行 |
| `classify-memories` | `0 6 * * *` | 给 memories 评分 importance/scope/shareable | 任务行 |
| `retrospective` | `0 5 * * *` | 从新的用户消息中提取纠正/重复解释信号 | 任务行 |
| `maintain-docs` | `""` | 维护 `ARCHITECTURE.md` 和 `STRUCTURE.md`，默认关闭 | 任务行，默认 off |
| `evaluate-smart-notes` | `0 3 * * *` | 评估 smart notes 条件是否满足 | 任务行 |
| `review-user-memories` | `0 3 * * *` | 隐私敏感。把重复用户行为观察提升到 user profile | 任务行，带隐私提示 |
| `promote-primers` | `0 3 * * *` | 把反复出现的问题提升为 primer | 任务行 |
| `refresh-primers` | `0 3 * * *` | 重新调查 stale primers | 任务行 |

每个 task 支持：

| 字段 | 类型 | 默认 | 作用 | UI 控件 |
|---|---|---:|---|---|
| `schedule` | cron string 或 `""` | 按 task 默认 | 5 字段 cron，空字符串禁用 | Cron preset + raw input |
| `model` | string | 继承 `dreamer.model` | task 级模型覆盖 | Model Select/Input |
| `fallback_models` | string 或 string[] | 继承 `dreamer.fallback_models` | task 级 fallback 覆盖 | 高级列表 |
| `thinking_level` | enum | 继承 `dreamer.thinking_level` | Pi task 级 thinking 覆盖 | Pi 高级 |
| `timeout_minutes` | number >=5 | `20` | task 超时分钟数 | InputNumber |
| `promotion_threshold` | number 2-20 | task-specific | 只对 `review-user-memories` 和 `promote-primers` 有意义 | InputNumber |

UI 建议：

- Dreamer 卡片顶部显示“已启用 X/11 个任务”。
- 常用模式提供 preset：
  - 推荐默认。
  - 手动-only，所有 schedule 置空。
  - 关闭 docs 维护。
  - 关闭 user memory review。
- 展开后展示任务表格，每行一个任务。
- 每行显示 schedule、状态、模型继承关系和高级配置入口。

### Memory 与检索卡片

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 结构化 |
|---|---|---:|---|---|---|
| `memory.enabled` | boolean | `true` | 是否启用跨会话项目 memory。关闭后 `ctx_memory` 隐藏，memory 注入关闭，`ctx_search` 仍可返回非 memory 来源 | Switch | 是 |
| `memory.injection_budget_tokens` | number 500-20000 | `4000` | session start memory 注入预算 | InputNumber + Slider | 是 |
| `memory.auto_promote` | boolean | `true` | historian/recomp 后自动把合适事实提升为 project memory | Switch | 是 |
| `memory.retrieval_count_promotion_threshold` | number >=1 | `3` | memory 被检索多少次后可提升为 permanent | InputNumber | 是 |
| `memory.auto_search.enabled` | boolean | `true` | 新用户消息触发后台搜索，命中强相关结果时附加 `<ctx-search-hint>` | Switch | 是 |
| `memory.auto_search.score_threshold` | number 0.3-0.95 | `0.6` | auto search 触发最低相似度 | Slider | 是 |
| `memory.auto_search.min_prompt_chars` | number 5-500 | `20` | 太短的用户消息不触发 auto search | InputNumber | 是 |
| `memory.git_commit_indexing.enabled` | boolean | `false` | 索引 HEAD git commit message 到 `ctx_search` | Switch | 是 |
| `memory.git_commit_indexing.since_days` | number 7-3650 | `365` | 索引多少天内的 commit | InputNumber | 是 |
| `memory.git_commit_indexing.max_commits` | number 100-20000 | `2000` | 每项目最多保留多少 commit | InputNumber | 是 |

### Embedding 卡片

Embedding 控制 memory、commit、conversation semantic search 的向量检索。

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 结构化 |
|---|---|---:|---|---|---|
| `embedding.provider` | `local` / `openai-compatible` / `off` | `local` | `local` 使用内置模型，`openai-compatible` 调远程 `/embeddings`，`off` 禁用语义向量 | Segmented | 是 |
| `embedding.model` | string | local 时 `Xenova/all-MiniLM-L6-v2` | embedding 模型名 | Input | 是 |
| `embedding.endpoint` | string | unset | OpenAI-compatible endpoint，源码会请求 `${endpoint}/embeddings` | Input | 是，项目级禁用 |
| `embedding.api_key` | string | unset | 可用 `{env:}` 或 `{file:}` token，不能明文展示。项目级不会展开 token，不建议做结构化编辑 | 用户级 Password/Input，项目级仅高级 JSONC | 是 |
| `embedding.input_type` | string | unset | passage/stored embedding 请求中的 provider-specific `input_type`，例如 NVIDIA NIM | Input，高级 | 是 |
| `embedding.query_input_type` | string | unset | query/search embedding 的 `input_type`，不设置则用 `input_type` | Input，高级 | 是 |
| `embedding.truncate` | string | unset | 发送给 embedding provider 的 truncate 模式，例如 `NONE/START/END` | Input/Select，高级 | 是 |
| `embedding.max_input_tokens` | positive integer | unset | chunk embedding 最大输入 token，上游用于窗口切分和 provider identity | InputNumber，高级 | 是 |

项目级安全规则：

- `embedding.provider` 和 `embedding.endpoint` 会被上游剥离。
- 项目级配置可以设置 `embedding.model`，但如果用户级配置里有 `api_key`，项目级 endpoint 重定向不会继承 key。
- 项目级配置中的 `{env:}` 和 `{file:}` 不会展开。AI Toolbox 的项目级表单应隐藏 `provider`/`endpoint`，`api_key` 不做常规结构化入口，只允许高级 JSONC 保留，避免诱导用户把 secret 提交到仓库。

可选能力：

- 用户级配置提供“测试 embedding endpoint”。
- 项目级配置不提供 endpoint 测试，因为项目级 endpoint 不可信且运行时会忽略。

### Sidekick 卡片

Sidekick 是 `/ctx-aug` 的按需上下文增强 agent。

当前源码事实：

- `sidekick` 是 optional object。
- 没有有效的 `sidekick.enabled` 字段。
- 历史 `sidekick.enabled=false` 会被 loader 迁移为 `sidekick.disable=true`。
- Pi 侧如果没有 `sidekick.model`，`/ctx-aug` 会提示未配置。

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 结构化 |
|---|---|---:|---|---|---|
| `sidekick.model` | string | unset | Sidekick 主模型。配置后 `/ctx-aug` 可用 | Model Select/Input | 是 |
| `sidekick.fallback_models` | string 或 string[] | unset | Sidekick fallback chain | 可排序列表 | 是 |
| `sidekick.disable` | boolean | unset | 禁用 sidekick。UI 显示为“启用 Sidekick”的反向 Switch | Switch | 是 |
| `sidekick.timeout_ms` | number | `30000` | `/ctx-aug` 每次运行超时 | InputNumber | 是 |
| `sidekick.temperature` | number 0-2 | unset | 采样温度 | InputNumber | 是 |
| `sidekick.prompt` | string | unset | 持久 agent prompt | TextArea，高级 | 是但项目级禁用 |
| `sidekick.system_prompt` | string | unset | 单次 `/ctx-aug` 附加 system prompt。项目级会被剥离 | TextArea，高级 | 是但项目级禁用 |
| `sidekick.variant` | string | unset | OpenCode only | OpenCode 视图显示 | 是 |
| `sidekick.thinking_level` | enum | unset | Pi only | Pi 视图显示 | 是 |
| `sidekick.tools/permission/description/mode/color/maxSteps/maxTokens` | mixed | unset | 透传或高级 agent 配置 | JSONC 高级 | 否 |

### 系统提示注入卡片

| 字段 | 类型 | 默认 | 作用 | UI 控件 |
|---|---|---:|---|---|
| `system_prompt_injection.enabled` | boolean | `true` | 全局关闭 Magic Context 对 agent system prompt 的注入，包括指导块、project docs、user profile 等 | Switch |
| `system_prompt_injection.skip_signatures` | string[] | `["<!-- magic-context: skip -->"]` | 如果 agent system prompt 包含任一字符串，则跳过 Magic Context 注入 | 可增删字符串列表 |

该配置适合放在高级卡片。误关会让 Magic Context 看起来“安装了但不注入上下文”，需要显著提示。

### SQLite 存储调优卡片

| 字段 | 类型 | 默认 | 作用 | UI 控件 | 作用域 |
|---|---|---:|---|---|---|
| `sqlite.cache_size_mb` | number 2-2048 | `64` | 每连接 SQLite page cache 大小 | InputNumber，高级 | 用户级 |
| `sqlite.mmap_size_mb` | number 0-8192 | `0` | SQLite mmap size，0 禁用 | InputNumber，高级 | 用户级 |

项目级 `sqlite` 会被上游剥离，AI Toolbox 不应在项目级结构化表单里展示。

### OpenCode pass-through 高级字段

上游 OpenCode loader 还从 raw config 中读取：

| 字段 | 作用 | 设计处理 |
|---|---|---|
| `disabled_hooks` | OpenCode plugin hooks 禁用列表。加载时 user/project union merge | 只放完整 JSONC，高级用户使用 |
| `command` | OpenCode command 配置，和 Magic Context builtin commands 合并 | 只放完整 JSONC，高级用户使用 |

这两个不是 `MagicContextConfigSchema` 的核心字段，Pi 不消费。第一版不做语义化表单。

### 废弃或迁移字段

| 字段 | 当前处理 | UI 策略 |
|---|---|---|
| `compressor` | 当前 schema 没有该字段，v2 使用 deterministic decay-tier rendering，旧 block 会被忽略 | 完整 JSONC 中保留；结构化表单不展示；提示已废弃 |
| `experimental.temporal_awareness` | loader 会迁移到 `temporal_awareness` | 完整 JSONC 中提示运行 doctor |
| `experimental.auto_search` | loader 会迁移到 `memory.auto_search` | 完整 JSONC 中提示运行 doctor |
| `experimental.git_commit_indexing` | loader 会迁移到 `memory.git_commit_indexing` | 完整 JSONC 中提示运行 doctor |
| `dreamer.user_memories` | doctor/loader 迁移到 `dreamer.tasks.review-user-memories` | 结构化表单只写新字段 |
| `dreamer.pin_key_files` | 上游注释说明该功能已移出 Magic Context | 不做表单 |
| `dreamer.enabled` / `sidekick.enabled` | loader 迁移为 `disable` 语义 | 表单只写 `disable` |
| `historian.enabled` | loader 会移除 invalid `historian.enabled` | 不做表单 |

## 后端设计

### 模块位置

建议新增后端模块：

```text
tauri/src/coding/magic_context/
  mod.rs
  commands.rs
  config.rs
  jsonc.rs
  types.rs
```

如果根 `AGENTS.md` 索引需要，后续实现时新增模块级 `AGENTS.md` 并登记到根文档。

### API

建议 Tauri commands：

```rust
read_magic_context_config(harness: MagicContextHarness) -> MagicContextConfigFile
save_magic_context_config(harness: MagicContextHarness, content: String) -> MagicContextConfigFile
create_magic_context_config(harness: MagicContextHarness) -> MagicContextConfigFile
run_magic_context_doctor(harness: MagicContextHarness) -> MagicContextCommandResult
```

类型草案：

```rust
enum MagicContextHarness {
    OpenCode,
    Pi,
}

struct MagicContextConfigFile {
    harness: MagicContextHarness,
    path: String,
    directory: String,
    exists: bool,
    content: String,
    parsed: Option<serde_json::Value>,
    parse_error: Option<String>,
    warnings: Vec<String>,
}
```

### 路径解析

用户级路径：

- Windows: `%USERPROFILE%\.config\cortexkit\magic-context.jsonc`
- Unix/WSL: `$XDG_CONFIG_HOME/cortexkit/magic-context.jsonc` 或 `~/.config/cortexkit/magic-context.jsonc`

项目级路径：

- `<project>/.cortexkit/magic-context.jsonc`

项目路径来源：

- OpenCode：优先用当前配置或 session/root 语义能得到的 project path；如果没有，让用户手动选择目录。
- Pi：优先用当前 runtime view 或页面根目录；如果没有，让用户手动选择目录。

注意：项目级配置是项目根路径，不是 OpenCode config 目录，也不是 Pi runtime root。

### 保存策略

保存分两类：

1. Raw JSONC 保存：
   - 用户在完整 JSONC Tab 中编辑。
   - 后端校验 JSONC 语法。
   - 原文写回。
   - 保留注释和格式。

2. Structured patch 保存：
   - 用户在语义表单里编辑。
   - 前端提交 patch。
   - 后端读取当前 JSONC。
   - 解析为 JSON value。
   - 合并 patch。
   - 写回格式化 JSONC。
   - 注释可能丢失，UI 需要提示。

为了 KISS，第一版可以：

- Raw Tab 保留注释。
- 表单保存写格式化 JSON，注释丢失可接受，但必须在 modal 里提示。

### 安全处理

后端不应在保存时主动展开 `{env:}` 和 `{file:}`。这些 token 是 Magic Context runtime 的职责。

项目级配置保存限制：

- 结构化表单不允许写入项目级不安全字段。
- Raw JSONC 允许用户写，但读取状态应警告“运行时会忽略”。
- 后端不应替用户删除这些字段，避免破坏用户文件。

### Doctor/Setup

命令：

```bash
npx @cortexkit/magic-context@latest doctor --harness opencode
npx @cortexkit/magic-context@latest doctor --harness pi
npx @cortexkit/magic-context@latest doctor --harness opencode --force
npx @cortexkit/magic-context@latest doctor --harness pi --force
npx @cortexkit/magic-context@latest setup --harness opencode
npx @cortexkit/magic-context@latest setup --harness pi
```

实现建议：

- 后端使用当前项目已有命令执行封装。
- 捕获 stdout、stderr、exit code。
- UI 用 modal 展示运行结果。
- `--force` 需要确认弹窗。
- `--issue` 会输出 sanitized issue report，可以作为复制文本功能。

## 前端设计

### 共享 service

新增：

```text
web/services/magicContextApi.ts
web/types/magicContext.ts
```

接口：

```ts
export type MagicContextHarness = 'opencode' | 'pi';

export interface MagicContextConfigRequest {
  harness: MagicContextHarness;
}

export interface MagicContextConfigFile {
  harness: MagicContextHarness;
  path: string;
  directory: string;
  exists: boolean;
  content: string;
  parsed?: Record<string, unknown>;
  parseError?: string;
  warnings: string[];
}
```

### OpenCode 页面接入

改动点：

- `web/features/coding/opencode/pages/OpenCodePage.tsx`
- `web/features/coding/opencode/components/PluginSettings.tsx`
- 可能新增 `MagicContextSettings` 调用位置。

展示逻辑：

```ts
const hasMagicContextPlugin = plugins.some((plugin) =>
  isOpenCodePluginEquivalent(getOpenCodePluginName(plugin), '@cortexkit/opencode-magic-context')
);
```

然后：

```tsx
{hasMagicContextPlugin && (
  <MagicContextSettings harness="opencode" />
)}
```

### Pi 页面接入

改动点：

- `web/features/coding/pi/components/PiExtensionsSection.tsx`
- 或 Pi 页面在 extensions section 下方挂载共享组件。

展示逻辑：

```ts
const hasMagicContextExtension = extensions.some((extension) => {
  const source = normalizeSource(extension.source);
  return source === '@cortexkit/pi-magic-context'
    || source === 'npm:@cortexkit/pi-magic-context';
});
```

然后：

```tsx
{hasMagicContextExtension && (
  <MagicContextSettings harness="pi" />
)}
```

### i18n

实现时所有新增文案必须使用 `scripts/i18n-keys.mjs`，不要手动编辑完整 locale JSON。

建议 namespace：

```text
magicContext.*
```

或按页面归属：

```text
coding.magicContext.*
```

需要覆盖：

- 卡片标题。
- scope 标签。
- doctor/setup 按钮。
- 字段 label/helper。
- 安全警告。
- JSONC 解析错误。
- 保存成功/失败。

## WSL/SSH/备份同步

### WSL

推荐新增默认 file mapping：

| id | source | target | 模块 |
|---|---|---|---|
| `magic-context-user-config` | Windows 用户级 `~/.config/cortexkit/magic-context.jsonc` | WSL 用户级 `~/.config/cortexkit/magic-context.jsonc` | Magic Context |

不建议第一版默认同步：

- `~/.local/share/cortexkit/magic-context/context.db`

原因：

- SQLite 数据库可能被 OpenCode/Pi 运行中占用。
- 数据体积可能很大。
- 跨环境同步容易产生锁、schema 版本、路径 identity 不一致问题。
- Magic Context 文档明确 DB 是运行时数据，不是配置文件。

项目级 `<project>/.cortexkit/magic-context.jsonc` 一般应由项目本身同步或 Git 管理，不应作为全局 WSL mapping 默认项。

### SSH

同 WSL。第一版只考虑用户级配置文件映射，不默认同步 DB。

### 备份恢复

用户级 config 可以纳入普通配置备份。DB 暂不纳入默认备份，后续如要支持，应作为独立大文件/运行时数据备份策略设计。

## 兼容和边界

### Unknown fields

Magic Context 当前 Zod schema 默认会 strip unknown keys，但用户可能提前写了未来版本字段。AI Toolbox 不应在读写时主动丢这些字段。

策略：

- Raw JSONC 保存保留所有内容。
- Structured patch 保存从当前 parsed object 出发合并 patch，保留未知字段。
- 对源码明确废弃的字段只提示，不删除。

### Project unsafe fields

Raw JSONC 中出现不安全字段时：

- 保留。
- 显示警告。
- 不在结构化表单里提供编辑。
- 不做自动删除。

### Secrets

`embedding.api_key` 可能是：

- 明文 key。
- `{env:OPENAI_API_KEY}`
- `{file:~/path/to/key}`

UI 要：

- 默认以 password 形式隐藏。
- 不调用后端展开 token。
- 提供“显示/隐藏”但不默认显示。
- 不把 key 写入日志。

### JSONC comments

表单保存可能丢注释，Raw 保存不丢注释。UI 必须显式区分：

- “表单保存会格式化配置并可能移除注释。”
- “完整 JSONC 保存会保留当前文本。”

## 实施步骤

### 阶段 1：方案确认

产物：

- 本文档。
- 评审后确认第一版字段范围。

### 阶段 2：后端只读能力

实现：

- `magic_context` 后端模块。
- 用户级和项目级路径解析。
- JSONC 读取和 parse error 返回。
- status API。
- OpenCode/Pi 安装检测可以先由前端传入，也可以后端辅助。

验证：

- 无配置文件时返回 `exists=false`。
- JSONC 有注释能解析。
- JSONC 语法错误返回 parse error，不 panic。

### 阶段 3：前端卡片只读展示

实现：

- `MagicContextSettings`。
- `MagicContextConfigCard`。
- OpenCode/Pi 页面接入。
- 卡片摘要和路径操作。

验证：

- 插件/扩展不存在时不展示。
- 用户级配置存在时显示摘要。
- 配置解析失败时卡片显示错误。

### 阶段 4：JSONC 编辑

实现：

- `JsoncEditor`。
- Raw JSONC modal。
- 保存文件。

验证：

- 注释和 trailing comma 可编辑。
- 解析错误时禁用保存或明确提示。
- 保存后重新读取路径一致。

### 阶段 5：语义化表单

实现：

- 基础、上下文、memory、embedding、historian、dreamer、sidekick cards。
- per-model map editor。
- fallback models editor。
- Dreamer task table。
- OpenCode/Pi 专属字段条件显示。

验证：

- 表单保存保留 unknown fields。
- 项目级禁用 unsafe fields。
- `sidekick.enabled` 不被写入。
- `compressor` 不被结构化写入。

### 阶段 6：doctor/setup

实现：

- 运行 doctor/setup。
- 输出 modal。
- force 确认。

验证：

- OpenCode harness 命令正确。
- Pi harness 命令正确。
- 失败时展示 stderr。

### 阶段 7：同步和备份

实现：

- WSL/SSH 默认映射仅新增用户级 config。
- 注意 bump 默认 mapping version 时只 backfill 新 mapping id，不恢复用户删除的旧默认项。
- 备份恢复纳入用户级 config。

验证：

- WSL 自动同步事件是否需要新事件，或复用 OpenCode/Pi 保存后的事件。
- WSL Direct 场景跳过本机到 WSL 同步。

## 最小测试计划

### 前端单元测试

建议新增：

- 配置摘要提取函数测试。
- unknown fields 合并测试。
- project unsafe fields gating 测试。
- `sidekick.enabled` 不写入测试。
- `compressor` 不进入 structured fields 测试。

### 后端单元测试

建议新增：

- 用户级路径解析。
- 项目级路径解析。
- JSONC 读取。
- JSONC parse error。
- structured patch 保留 unknown fields。
- raw save 保留注释。

### 集成验证

手动验证：

1. 安装 OpenCode Magic Context 后 OpenCode 页面显示配置卡片。
2. 未安装插件时不显示。
3. 安装 Pi Magic Context 后 Pi 页面显示配置卡片。
4. 用户级配置可创建、编辑、保存。
5. 项目级配置可创建、编辑、保存。
6. `doctor --harness opencode` 能运行并展示输出。
7. `doctor --harness pi` 能运行并展示输出。
8. 设置 `embedding.api_key="{env:XXX}"` 后 UI 不展开 secret。
9. JSONC 中有注释时 Raw 保存不丢注释。
10. 表单保存后 unknown top-level field 保留。

## 需要评审确认的问题

1. 第一版已确认先只做用户级配置，不做“项目覆盖”卡片。
2. 是否接受表单保存会格式化 JSONC 并丢注释，Raw 保存保留注释。
3. 是否把 `embedding.input_type/query_input_type/truncate/max_input_tokens` 放入高级表单，还是只放 JSONC。
4. 是否第一版就实现 endpoint 测试。
5. 是否第一版就新增 WSL/SSH 默认 mapping。
6. Dreamer tasks 是否需要完整 task 表格，还是先只做统一模型和推荐 schedule preset。

## 推荐第一版范围

推荐第一版做得完整但不过度扩张：

- 做用户级配置卡片。
- 做 Raw JSONC 编辑。
- 做基础、上下文、memory、embedding、historian、dreamer、sidekick 的结构化表单。
- Dreamer tasks 做表格，但高级 task model override 可以先折叠。
- endpoint 测试可以后置。
- WSL/SSH mapping 可以后置到配置保存稳定后。

这样既满足“语义化配置卡片”的目标，也保留完整 JSONC escape hatch，不会因为上游字段快速变化而频繁追 UI。

# Gateway 本地 CLI 用量接入研究

研究日期：2026-09-10；实施与验证更新：2026-09-11。原生采集、统计展示与跨源去重修复已实现。文中的源码版本与初始样本保留调查时点，实施后的核对结果见末尾。

## 结论

Pi、Oh My Pi、DSH、Grok、Hermes、OpenClaw、Kimi Code 和 Python Kimi CLI 的原生用量已接入统一采集器。OpenCode、Gemini CLI 继续复用已有入口；Claude Desktop 改为 transcript 优先、audit 回合汇总互斥回退，并修复可严格证明的重复历史贡献。

OpenClaw、Hermes、Kimi Code 和开源 Python 版 Kimi CLI 在本机调查目录中没有可核验的有效原生用量样本，验证依据是更新后的源码与合成 JSONL/SQLite/WAL 回归；不声称已实测真实账号会话。Hermes 保留会话/模型累计差额粒度，Grok 保留回合粒度，不把汇总伪装成逐 HTTP 请求。

issue #340 的根因是跨源身份匹配遗漏：原实现匹配成功后已经优先网关，但只比较网关结束时间前后 10 秒，漏掉较长请求和延迟落盘。修复后使用实际执行区间与 30 秒结束后宽限，仍要求同工具、模型、非零且相同的四类 Token、唯一完整成功候选和一对一占用；已知不同响应 ID 不会退回启发式匹配。

“CLI 已安装”“本地存在会话”“会话包含真实 usage”“已被 AI Toolbox 导入”是四个独立事实。所有接入只能统计 CLI 实际落盘的用量，不能从正文长度、上下文占用或模型最大上下文推算真实消耗。

## 源码更新与调查边界

已有仓库更新前均检查了分支与工作区，最终通过 `git pull --ff-only --no-stat --no-tags origin <branch>` 快进成功。首次 OMP 拉取超时、OpenCode SSH 传输停滞，终止本次任务创建的拉取进程后，以临时 HTTPS URL 映射重试成功；未更改仓库持久化 remote 配置。

| 工具 | 相对仓库根目录的源码位置 | 操作 | 调查版本/commit |
|---|---|---|---|
| Pi | `../pi` | 补充下载官方主仓库 | `400d690`，源码 0.85.1；本机安装包 0.84.2 |
| Oh My Pi | `../oh-my-pi` | pull main | `d3b22a0db6` → `2e6b5b79a0`，源码 18.1.16；本机安装包 17.4.0 |
| DSH | `../deepseek-harness` | pull master | `4e84901e64` → `b2e3b2a012`，源码 0.1.5-alpha.2 |
| OpenCode | `../opencode` | pull dev | `fdeb2748e` → `b6914b39d`；本机安装包 1.18.19 |
| Grok | `../grok-build` | pull main | `47348d1` → `37949780` |
| Gemini CLI | `../gemini-cli` | pull main | `1a894c18e` → `ed2ac40df`；本机安装包 0.56.0 |
| Codex | `../codex` | pull main | `c9fac4dd5a` → `0447e4a1fd`；本机安装包 0.153.4 |
| Hermes | `../hermes-agent` | 补充下载官方主仓库 | `fad7cfc` |
| Kimi Code，Node 版 | `../kimi-code` | 补充下载官方主仓库 | `e348cf5`，0.42.0 |
| Kimi CLI，Python 版 | `../kimi-cli` | 补充下载官方主仓库 | `86f1364`，1.50.0 |
| OpenClaw | `../openclaw` | 补充下载官方主仓库 | `c9b30878` |

五个新仓库采用 shallow clone。检查完成时以上十一个源码工作区均干净。`PiDeck` 与 `openclaw-manager` 是外围项目，本次未将它们当作对应 CLI 核心源码。

Claude Code / Claude Desktop 的结论来自本机原始文件和 AI Toolbox 当前代码，不声称其完整产品源码已公开或已经拉取。CCS 的补充比较使用现有只读 checkout `6243e20a`，未修改其工作区中的已有文件。

## 实施前 AI Toolbox 的覆盖快照

实施前导入选择枚举包含 Claude、Claude Desktop、Codex、Grok、Kimi、Gemini、OpenCode，未包含 Pi、OMP、DSH、Hermes、OpenClaw。原 Grok / Kimi 为通用 usage 兼容，不等于覆盖对应 CLI 原生文件格式。

实现继续复用同一导入 mutex、60 秒调度、文件状态账本、SQLite 写入、跨源去重和历史 rollup，没有另建一套后台导入服务。

源码入口：

- [导入调度、来源目录、持久化](../tauri/src/coding/proxy_gateway/session_import.rs)
- [现有原生解析器](../tauri/src/coding/proxy_gateway/session_import/parsers.rs)
- [OpenCode SQLite 采集](../tauri/src/coding/proxy_gateway/session_import/open_code.rs)
- [统计枚举与 DTO](../tauri/src/coding/proxy_gateway/types.rs)
- [会话运行时目录解析](../tauri/src/coding/session_manager/mod.rs)

本地 `log_retention_days = 7`。调查时请求明细中没有 OpenCode / Gemini / Desktop 的近期 Session 行，但以下历史 Session 用量已在 `usage_daily_rollups`：

| 工具 | Session 汇总中的记录数 | 已记录 Token 总量 | 解释 |
|---|---:|---:|---|
| OpenCode | 6,641 | 459,369,042 | 已接入，历史明细已归档 |
| Gemini CLI | 17 | 330,820 | 已接入，历史明细已归档 |
| Claude Desktop | 8 | 214,641 | 已导入，但存在下述重复统计，不能当成正确实际消耗 |

表格只报告初始调查时数据库内容，不等价于供应商账单或修复后结果。Desktop 的其它 provider 汇总还包含旧代理记录，不能直接与 Session 行相加后宣称独立真实调用量。

## Pi

**可接入，已有本机真实样本。**

- 默认来源：`~/.pi/agent/sessions/**/*.jsonl`。必须复用应用现有 Pi 路径解析，同时支持 custom root、`PI_CODING_AGENT_DIR`、`PI_CODING_AGENT_SESSION_DIR`、`settings.json.sessionDir` 与 WSL runtime location。
- 本机扫描到 2 个会话文件，34 条 assistant 用量记录；四项之和与原生 `totalTokens` 一致，共 2,938,766 Token。
- 主记录形态：`type=message`、`message.role=assistant`，用量在 `message.usage`；模型和 provider 在同一 message，部分记录具有真实 `responseId`。
- `input` 是新增输入；`cacheRead` / `cacheWrite` 独立计数；`output` 已含思考 Token，`reasoning` 仅是其子集，不能再次加到输出。
- 最新源码还会在 `compaction`、`branch_summary` 上持久化可选 `usage`。仅扫描 assistant 消息会漏掉这部分辅助调用。缺少原始模型归属时不能用当前默认模型补猜价格。
- 最新官方 session 统计还会消费工具结果自身的可选 `usage`；接入时应区分工具的独立消耗和子代理的转述汇总，已有子代理原始记录时不能重复累加。
- fork 会复制历史 entry，并在 header 记录 `parentSession`。应按可证实的继承身份排重，不能只用“文件路径 + 行号”作为调用身份，也不能只统计当前可见分支而漏掉已发生的调用。

证据：[Usage 类型](../../pi/packages/ai/src/types.ts)、[Session entry 与 fork](../../pi/packages/coding-agent/src/core/session-manager.ts)、[摘要 usage 的保存与统计](../../pi/packages/coding-agent/src/core/agent-session.ts)。

## Oh My Pi（OMP）

**可接入，已有本机真实样本。**

- 默认来源：`~/.omp/agent/sessions/**/*.jsonl`，与 Pi 使用不同根目录和工具身份。
- 本机 2 个会话文件包含 2 条 assistant usage：1 条失败全零，1 条有效调用。有效调用为 `input=19,819`、`output=44`、总量 `19,863`，并包含 `usage.cost.total=0.0279402`。
- `agent.db` 中的 `client_usage`、`usage_history`、`usage_cost_history` 本机均为空，不能因为存在这些表就用它们替代真实会话来源；同库还包含认证数据，采集无需读取这些认证表。
- 常规 message 的四项字段与 Pi 接近，但应分别维护版本适配。新版另有 `model_usage` entry，记录不进入普通对话的辅助模型调用。
- `output` 已含思考；新版 `reasoningTokens` 是输出子集。新版 `orchestration` 是额外服务端用量，可能使原生 `totalTokens` 大于常规四项之和，不能静默丢掉差额或强行塞入普通输入。
- task 工具结果可能带子代理用量汇总；同时扫描子代理会话时不能再把父工具汇总算一遍。fork 可能只清费用而保留 Token，不能用费用为零判定它是一次新的免费调用。
- 首期可覆盖本机文件存储。最新版还支持可配置的 SQL/Redis session storage，这类非默认存储不能仅靠扫描 JSONL 宣称已覆盖。

证据：[Usage 口径](../../oh-my-pi/packages/catalog/src/types.ts)、[model_usage entry](../../oh-my-pi/packages/coding-agent/src/session/session-entries.ts)、[统计及继承费用处理](../../oh-my-pi/packages/coding-agent/src/session/session-manager.ts)、[SQL storage 扩展](../../oh-my-pi/packages/coding-agent/src/session/sql-session-storage.ts)。

## DSH

**可接入，本机有大量真实数据；实现需要独立的版本化事件解析器。**

- 本机 `~/.dsh/sessions/` 下有 31 个 `session.jsonl.zstd`，header 全部是格式 v0，均成功只读解压解析。
- 找到 3,702 条 `assistant/message` 用量与 147 条 `compaction/summary` 用量。本次 3,702 条 assistant message ID 均不同，全部是 append 事件；存在 4 个子代理会话。上述数量是原始用量记录数，不将它们预先写入应用或当成完整供应商账单。
- v0 主用量位于 `data.usage`，模型归属在 `data.message.source.provider/model`；部分上游 ID 在 `source.replayState.responseId`。
- `inputTokens` 已是新增输入，`cacheReadTokens` / `cacheWriteTokens` 独立；`outputTokens` 已包括思考。`shadowedTokenCount`、上下文压力和 `maxTokens` 不是消耗。
- 同一调用同时出现 `assistant/chunk` usage 和最终 `assistant/message`，不能相加。压缩摘要有独立实际 usage，应计入一次，不能因它不在普通聊天展示中而遗漏。
- 本机是 v0，最新源码已经是 v3，支持 `session.vN.jsonl[.zstd]`、generation 选择，以及 `assistant/attempt` / `data.stream` 中的最终 usage。不能只实现旧文件名或把同一会话的多代文件全部重复扫描。
- 按已提交的有效 generation 读取；结合 `session/end-seed` / parent 信息排除继承前缀，保留重试中的实际用量。当前 Session Manager 的正文提取不是完整用量采集器，例如会跳过只携带 usage 的空正文 assistant。

证据：[文件版本命名](../../deepseek-harness/packages/session/session-format/src/filename.ts)、[压缩与 generation](../../deepseek-harness/packages/session/session-persistence-jsonl/src/format.ts)、[原生 Token 归一化](../../deepseek-harness/packages/llm/llm-pi-ai/src/stream.ts)、[用量归并](../../deepseek-harness/packages/llm/token-meter/src/usage-projection.ts)、[回合用量与重试](../../deepseek-harness/packages/llm/token-meter/src/turn-usage.ts)。

## OpenCode

**已经接入，应扩充真实版本回归，无需重新做一个来源。**

- 支持 legacy `storage/message` JSON 与原生 `opencode.db` SQLite/WAL，当前本机数据库也确实有 assistant `tokens`。
- `tokens.input` 已是新增输入；`tokens.output` 不含 `tokens.reasoning`，导入输出应相加；`tokens.cache.read/write` 独立计数。
- 本机一条记录：输入 363、普通输出 1,135、思考 25、缓存命中 30,208，总计 31,731，与原生 `tokens.total` 一致。当前解析器已有正确的输出加思考逻辑。
- 同一消息的 legacy JSON 与 SQLite 副本应使用同一身份；SQLite 读取应覆盖 WAL 更新，不能以主数据库文件 mtime 为唯一水位。
- 当前保留期之外的历史用量进入汇总，不能用“请求列表看不到近期行”判断尚未接入。

证据：[上游 Token 归一化](../../opencode/packages/opencode/src/session/session.ts)、[本应用 JSON/SQLite 共用 parser](../tauri/src/coding/proxy_gateway/session_import/parsers.rs)、[只读 SQLite/WAL 采集](../tauri/src/coding/proxy_gateway/session_import/open_code.rs)。

## Claude Desktop

**Code/Cowork 本地会话可以接入，现有入口已导入，但需要先修正统计来源。**

- 本机真实来源位于 `%LOCALAPPDATA%/Claude-3p/local-agent-mode-sessions/...`：包含 `audit.jsonl` 和 `.claude/projects/.../*.jsonl` 标准转录。
- 同一次调用的 transcript assistant 最终 usage 与 audit `result.usage` 是相同消耗；audit assistant 还可能停在输入 1、输出 1 的初始快照。
- 当前通用 parser 接收所有正 usage，`result` 即使没有模型也会成为 `unknown` 记录。本机账本中已确认 4 个 audit result ID；Session 的 `unknown` 日汇总正好是它们合计的输入 75,046、输出 86、缓存读取 47,360，即 122,492 Token。相同调用还在消息/代理路径中出现。
- `source_identity` 对不同目录的 `audit.jsonl` 都得到 `claude_desktop:audit`，不同物理文件还共享了文件状态键。这也是不能直接扩大递归扫描范围的原因。
- 建议以标准 transcript 的真实 assistant 消息为主要来源；如仅有 audit，则明确使用一种可核验的降级来源，不能把消息与汇总同时算入。若读取 `result.modelUsage`，也必须保持与消息来源互斥。
- 应支持实际存在的官方/3P本地 Code/Cowork 来源，避免把数据目录固定为一种部署方式；本次没有证明普通云端 Chat 全部消息都具有可离线读取的用量。
- 新增过滤规则不能自动修复已归档汇总。历史重复贡献的修复应另做可验证回填，先快照、再按来源贡献核对，不能清空全部统计重新猜测。

证据：[当前目录选择与递归发现](../tauri/src/coding/proxy_gateway/session_import.rs)、[当前 parser admission](../tauri/src/coding/proxy_gateway/session_import/parsers.rs)、[Desktop 路径与会话约定](../tauri/src/coding/claude_desktop/AGENTS.md)。

## Grok

**应新增原生解析；本机已经有可统计但未导入的数据。**

- 本机 `chat_history.jsonl` 没有可用 usage；真正来源是 `updates.jsonl`，事件为 `_x.ai/session/update` → `params.update.sessionUpdate=turn_completed` → `usage`。
- 找到 50 个不同 prompt 的回合结束用量事件，原生 `modelCalls` 合计 349；当前 Grok 采集账本导入记录数为 0。
- 优先按 `usage.modelUsage` 拆模型；顶层 totals 是相同回合的总和，不能再加一次。`params._meta.totalTokens` 是另一个状态读数，不能逐条累加当账单。
- 输入是含缓存总输入；应扣除缓存部分得到网关统一的新增输入。`reasoningTokens` 已包含在 output，不能重复加。新版还支持缓存创建、费用 ticks、完整性标记。
- 本地 50 个事件中有 1 个 `usageIsIncomplete`；不能展示为已完整覆盖该回合。
- 这是回合/模型汇总，不能把 50 个事件当成 50 个 HTTP 请求，也不能生成 349 条不存在的请求明细。应保存原生调用数与统计粒度。
- 最新上游父回合会折入子代理用量，扫描子会话时必须选择一致的归属规则，避免父子同时重复计算。

证据：[上游 prompt/session 用量账本](../../grok-build/crates/codegen/xai-chat-state/src/usage.rs)、[CCS 原生 Grok 适配参考](../../cc-switch/src-tauri/src/services/session_usage_grokbuild.rs)。CCS checkout 仅作补充参考，不覆盖最新 Grok 全部字段。

## Hermes、OpenClaw 与 Kimi

| 工具 | 已核实的原生来源 | 接入注意点 | 本机验证程度 |
|---|---|---|---|
| Hermes | `state.db` 的 `session_model_usage`，旧版 `sessions` 累计列 | 优先模型/provider/task 维度；累计快照只记录变更贡献，不能每次全量相加；不能同时累加模型行与 session 总行。没有逐调用时间的旧历史不能精确还原每天/每次请求 | 默认目录有配置和 skills，本次未发现有效 `state.db` / transcript 用量 |
| OpenClaw | 历史 agent JSONL、归档文件；最新版也有 agent SQLite transcript | 用既有 `normalizeUsage` 语义；SQLite 与同源 JSONL 优先级、归档身份、分支重放要一致。不能只套 Pi JSONL parser 就宣称覆盖最新版 | 本机默认根只有配置/skills，未发现 agent 会话用量 |
| Kimi Code，Node 版 | `~/.kimi-code` 的版本化 agent wire / session store；持久化 `usage.record` | 事件携带 `agentId/model/usage/usageScope`；四项为 `inputOther/output/inputCacheRead/inputCacheCreation`。单次事件与 byModel/total/currentTurn 汇总不能同时相加；兼容 wire 协议和新 session store 时避免重复读迁移副本 | 源码已核实，本次未找到有效本机会话 |
| 开源 Kimi CLI，Python 版 | `KIMI_SHARE_DIR` 或 `~/.kimi/sessions/.../wire.jsonl` 的 `StatusUpdate.token_usage` | `input_other/input_cache_read/input_cache_creation/output` 可映射；usage 属当前 step，`message_id` 可用于身份；嵌套 `SubagentEvent` 需要去重。无法从事件确认模型时保留未知 | 本次未找到可核对的有效本机会话 |

Kimi 必须区分产品/格式：本应用当前 `kimi/` 适配的是 `@moonshot-ai/kimi-code`、`KIMI_CODE_HOME`、`~/.kimi-code`，其正确源码仓库 `MoonshotAI/kimi-code` 也已补充下载。另一个 `MoonshotAI/kimi-cli` 1.50.0 是 Python 项目、默认 `~/.kimi`。不能用 Python 版的路径推翻现有 Kimi Code 配置语义，也不能把支持其中一个宣称为支持两者。

Kimi Code 的 `UsageRecord` 明确标记 `durable=true`，`UsageAgentModel` 通过单次事件累加模型用量；相比只解析通用顶层 `usage`，读取专用 `usage.record` 能保留模型归属。该结论已由源码确认，尚未用用户真实 Node 版会话做导入回归。

Hermes 的 `messages.token_count` 是消息侧字段，并非四类计费用量的替代源；`session_model_usage` 的 `first_seen/last_seen` 只描述累计窗口，不能推导窗口内每次调用发生时刻。费用要区分原生 `actual_cost_usd` 与 `estimated_cost_usd`。

证据：[Hermes schema](../../hermes-agent/hermes_state_common.py)、[Hermes 累计写入与模型归属](../../hermes-agent/hermes_state_usage.py)、[Hermes 每调用归一化](../../hermes-agent/agent/turn_usage.py)、[OpenClaw 多存储来源](../../openclaw/src/infra/session-cost-usage-collection.ts)、[OpenClaw 用量归一化](../../openclaw/src/agents/usage.ts)、[Kimi Code 持久化用量事件](../../kimi-code/packages/agent-core-v2/src/agent/usage/usageOps.ts)、[Kimi Code 模型用量归并](../../kimi-code/packages/agent-core-v2/src/session/usage/usageAgentModel.ts)、[Python Kimi 原生统计](../../kimi-cli/src/kimi_cli/vis/api/statistics.py)、[Python Kimi step usage](../../kimi-cli/src/kimi_cli/wire/types.py)、[现有 Kimi Code 约定](../tauri/src/coding/kimi/AGENTS.md)。

## 其它已检查的 CLI

- Claude Code / Codex 已有本地导入，继续复用既有最终快照、归档和父会话去重机制。本轮不是重新审查整个代理协议转换。
- Gemini CLI 已有 JSON/JSONL 导入。原生 `input` 含缓存，`output` 不含 `thoughts`；当前导入逻辑的新增输入扣缓存、输出加 thoughts 与源码一致。`total` / `tool` 的特殊组合需以实际 provider 记录验证，不把上下文用量当额外输入。
- iFlow 本机有 4 条 assistant usage，但均为输入 0、输出 0，不能从这些记录补出真实消耗。
- Qwen 默认 tmp 目录本次只有日志，没有可核验的生成用量。未据此断言 Qwen CLI 不支持用量；本轮未做其源码适配评估。

证据：[Gemini usage 持久化](../../gemini-cli/packages/core/src/services/chatRecordingService.ts)。

## 已落实的接入方案

### 复用现有采集架构

保留 `session_import` 的后台调度、blocking worker、导入账本、事务写入、静默刷新事件和归档；各工具增加自己的来源发现与解析适配，产出统一用量记录。JSONL 按行处理，DSH 流式解压，外部 SQLite 只读并覆盖 WAL。

“统计可识别的工具集合”与“网关可接管的工具集合”已分开，使用 `GatewayUsageTool` 承载 13 种用量工具；`GatewayCliKey` 继续只表示可接管工具。调整已贯通 Rust DTO、查询过滤、SQLite 详情回退、前端筛选与统计概览。

统一保留：新增输入、含思考的输出、缓存读取、缓存创建，以及来源可提供的原始总量、费用来源、真实消息/响应身份、完整性和统计粒度。只有会改变实际结果的元数据才进入持久化方案；不为未知未来字段预建一套框架。

有明确原生 provider/model 的记录可以保留其历史标签；未知来源继续显示 Session，不能从当前配置反推历史渠道。模型原生费用是 CLI 报告/估算值，并不自动等于供应商实际扣费。

### 实施范围

1. **修正已有来源**：修 Claude Desktop audit/message 重复与状态键冲突；补 Grok 原生 `updates.jsonl`；确认 OpenCode/Gemini 的历史归档及筛选语义。Desktop 历史汇总修复单独验证来源贡献。
2. **接入有本机样本的工具**：Pi、OMP、DSH。先用现有真实文件的脱敏结构做回归，覆盖最新源码明确存在的辅助调用、版本 generation、重试和继承规则。
3. **接入其它已证实格式**：OpenClaw JSONL + SQLite/归档、Kimi Code 的原生用量事件及 Python Kimi 的独立兼容；Hermes按会话/模型累计粒度提供统计。对缺少真实样本或无法还原逐请求时间的来源，验收结果应保留这些限制。

以上三个阶段均已实现。SQLite v20 保存原生粒度、调用数与费用来源，并独立保留四类 Token 之外的原生差额；旧明细、日汇总和导入账本保持兼容。请求分页显示记录条数，统计累加原生调用数，未知调用数不推算为 1。

### 验证要求

- 覆盖“原生文件/SQLite → 导入 → 请求/汇总查询”的往返，不能只测字段 helper。
- 同一来源重复同步、应用重启、原文件追加最终 usage、归档后重扫均不得重复计数。
- Pi/OMP 的 reasoning 不能二次加输出；OpenCode/Gemini 的独立 reasoning/thoughts 需要加进统一输出。
- Pi 摘要、OMP `model_usage`、DSH 压缩摘要单独统计；父子代理汇总、fork 复制、同一响应的多快照只能计算一次。
- DSH 同时验证本机 v0 与最新 v3 格式、多个 generation 共存、压缩文件尾部仍在写入及 retry attempt。
- Grok 验证回合/调用数的不同口径、modelUsage 与顶层 totals 互斥、incomplete 标记及子代理折入。
- Kimi Code 验证持久化事件与 byModel/currentTurn 状态汇总互斥、父子 agent 和迁移存储去重；Python Kimi 单独验证 StatusUpdate、message_id 与嵌套 SubagentEvent。
- Desktop 验证 transcript 与 audit 同时存在、仅一个存在、audit 汇总与最终消息等量，以及多个 audit 文件的状态隔离。
- 外部 SQLite 验证 WAL-only 更新；旧 schema/数据源缺失不能妨碍其它工具同步。
- 实施属于跨层持久化迭代，交付前运行 `pnpm test`、`cargo test`、`pnpm exec tsc --noEmit`；若涉及共享 UI/i18n，补 `pnpm build`。

## 实施后本机核对

所有原生 CLI 文件只读；真实文件导入验证写入临时数据库，历史修复验证使用通过只读连接备份的数据库副本。没有启动付费模型请求。运行中的本机开发版也已自动加载实现并将应用数据库升级到 v20；下述“原库”结果来自只读查询，而非测试直接修改原库。

| 来源 | 新建临时库中的调用数 | Token 总量 | 已核对行为 |
|---|---:|---:|---|
| Pi | 34 | 2,938,766 | 原生四类之和；重复同步不新增 |
| Oh My Pi | 2 | 19,863 | 包含 1 条明确记录了零用量的失败调用 |
| DSH | 3,849 | 723,958,157 | 3,702 条 assistant 和 147 条压缩摘要；正常 resume 不丢历史 |
| Grok | 349 | 47,115,450 | 保留回合/模型汇总记录，349 是原生模型调用数 |
| Claude Desktop | 4 | 122,492 | transcript 与 audit 不叠加 |

新建临时库共扫描 52 个文件、解析并写入 3,939 条记录；第二次同步新增 0 条，查询汇总不变。记录条数与模型调用数并不相同。

DSH 的旧开发版曾导入并归档 1,414 条、269,612,327 Token。自查发现普通空 `session/end-seed.data={}` 也是恢复标记，不能删除其前面的真实用量；旧的 turn/step 身份还会随恢复重用。新版以消息 ID 为主，parser revision 4 重扫旧文件，只有时间、模型、Token 等指纹及响应身份相容时才认领旧 alias，并且一对一退休。原库已自动补齐为 **3,849 条、723,958,157 Token**；副本同步结果与全新导入一致，再次同步新增 0 条。

Desktop 原库中可证明的 audit 重复贡献已移除，历史 Session 汇总由 8 条、214,641 Token 收敛为 **4 条、92,149 Token**。这与全新导入 transcript 的 122,492 Token 不同：旧 canonical 消息已经有部分早期快照归档，缺少精确贡献证据时不能凭新的全文值改写旧汇总。此实现不声称已完整重建这部分历史。

### Claude Code / GLM-5.2 零用量

本机 `.claude/projects` 的只读核对发现 272 个相关文件、11,221 个不同 assistant 消息 ID；其中 5,933 个 ID 的所有已落盘 usage 快照四类 Token 均为零，另 5,288 个有非零快照。这些是本地日志事实，不代表供应商真实账单或独立付费调用数。

采集器对有模型、assistant 身份、消息 ID 且明确记录 usage 的零计数响应保留一条记录，并支持后续最终 usage 更新；空 usage、user 记录与零用量 synthetic 消息不作为调用。前端显示已有的输入/输出/缓存与总 Token，不能用正文长度补造缺失用量。CCS 的本地采集同样依赖 CLI 落盘字段，且当前 checkout 以任一计费维度大于零作为导入条件，既不保留这类全零行，也无法还原未保存的真实消耗。

### issue #340 与网关优先

截图中的网关时间为 17:51:54，Session 为 17:52:15，相差 21 秒，模型、输入 1,559、输出 90、缓存约 21.2 万及费用一致，符合旧 ±10 秒规则漏匹配的条件。截图缺少完整响应身份，因此不能仅凭截图独立证明两行一定是同一次请求。

本机另核对到两对 Codex 记录：Session 比网关结束分别早 109 秒和 119 秒，而网关耗时分别为 151,653 ms 和 239,252 ms；均在实际请求执行区间内。新版运行后，两条 Session 明细已删除、对应网关行保留、账本保存了匹配关系。修复后数据库副本再复核新增合并为 0，并保持 707 条网关记录不变。

修复仍然保留身份边界：全零用量、已知不同响应 ID、多个候选、回合/累计汇总不做逐请求启发式合并。历史明细已归档且缺少身份时，无法保证事后完全去重；不能按当前接管状态或宽时间窗删除所有 Session 用量。界面新增“全部来源 / 网关 / Session”仅用于查询显示。

## 自查与验证状态

自查覆盖全部本任务变更，包括来源发现、原生单位、身份迁移、事务/账本、归档、DTO、筛选和统计展示。除 DSH 恢复及旧身份问题外，还修正 Hermes 不同模型相同累计值的身份冲突、同一观察时间重复费用更正覆盖旧记录的问题；按工具汇总也保留只有费用变化、没有新增 Token 或调用数的时间窗口。

- 前端：衔接远端最新 main 后再次运行 `pnpm test`，530 项通过；`pnpm exec tsc --noEmit` 与 `pnpm build` 通过。构建保留原有大 chunk 提示。
- Rust：衔接远端最新 main 后再次运行全量 `cargo test --jobs 2`，使用独立 target 通过，包含 1,927 项单元测试、136 项集成测试和 10 项文档测试，失败为 0；5 项默认忽略。原 target 的尝试因运行中的开发版锁住 `ai-toolbox.exe` 而退出，未计为通过。
- 真实文件与数据库副本：3 项开发者本地核对测试另以 `--ignored --nocapture --test-threads 1` 显式运行，全部通过，覆盖新建临时库导入/二次幂等、DSH 原库副本与完整原始数据对账、跨源收敛后复核。DSH 旧账本升级与归档兼容回归也在全量单元测试中通过。
- 界面：合成数据页已核对亮色、暗色、system theme、英文、1000px 窄窗口、长名称、空态/加载态、选中/禁用状态、来源过滤、全部工具选项、明确的零 Token、额外 Token、累计说明、费用调整及本地工具 RPM 空值。窄窗口日期输入挤压已改为自然换行并复核，最后的样式更新再次通过生产构建。临时页面在验收后清理。
- 未覆盖真实环境：Hermes/OpenClaw/两种 Kimi 的真实账号会话、OMP 非默认 SQL/Redis session storage、Claude Desktop 普通云端 Chat 的完整离线用量。

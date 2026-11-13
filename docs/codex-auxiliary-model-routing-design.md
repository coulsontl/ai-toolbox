# Codex 辅助模型定义与路由设计

> 状态：Implemented（2026-11-07）
> 关联 issue：https://github.com/coulsontl/ai-toolbox/issues/455
> 参考实现：[Magpie](https://github.com/yetone/magpie) 的 `internal/gateway/codex_titles.go`、`codex_memory.go`、`codex_follow.go`（仅参考行为与术语，不复制实现）
> 相关文档：[gateway-model-routing.md](gateway-model-routing.md)、[gateway-protocol-conversion.md](gateway-protocol-conversion.md)、[gateway-provider-compatibility.md](gateway-provider-compatibility.md)
>
> **2026-11-07 决策**：不做全局任务表，辅助模型只按渠道（provider）定义；辅助请求始终跟随当前候选渠道，不跨渠道固定 provider。
>
> **2026-11-07 确认**：接受“不跨渠道路由”；记忆首期只提供一条 `memory` 路由；WebSocket 首期只标记未应用，后续补齐。
>
> **2026-11-07 决策**：渠道辅助模型采用三态：缺键 = 自动；`{ "mode": "main" }` = 强制渠道默认模型；`{ "mode": "model", "model": "..." }` = 指定模型。不把当前默认模型名写死进配置。

## 1. 背景与目标

Codex 除了主编码回合，还会发出若干“隐藏调用”：会话标题、描述、单行摘要、composer 预测、记忆提取/整理、自动审批等。它们目前都会落到当前主会话模型，或者落到 Codex 自己的默认模型，导致：

- 轻量任务消耗昂贵模型；
- 无法把标题/预测/记忆路由到本地或低成本模型；
- 用户不知道某条请求是什么、用了哪个模型、走了哪个渠道；
- 把 Codex 指向网关后，Codex 自己的默认记忆模型（如 `gpt-6-luna`）可能不被上游支持，记忆功能静默失败。

本设计的目标：

1. 每类支持的 Codex 辅助任务都可以在**当前渠道内**独立指定模型；
2. 未配置时有明确、稳定的回退逻辑；
3. 请求/用量详情尽可能显示辅助任务类型、实际模型、provider 和目标协议；
4. 不改变主用户回合的模型选择；
5. 协议/响应格式不支持时明确报错，不静默换模型；
6. 升级后保留现有配置，不覆盖用户已定义的映射。

## 2. 上游事实（Source of Truth）

### 2.1 Codex 原生只支持记忆模型

Codex 的 `config.toml` 有：

```toml
[memories]
extract_model = "gpt-6-luna"
consolidation_model = "gpt-6.1-terra"
# 其余字段：version / dual_write / generate_memories / use_memories / ...
```

来源：Codex `codex-rs/config/src/types.rs` 的 `MemoriesToml`（`#[schemars(deny_unknown_fields)]`）。

另外还有 `review_model`，但它是 `/review` 命令的模型，不是审批自动审查（auto-review）。自动审批的模型由模型目录条目的 `auto_review_model_override` 控制，现有仓库已经实现这条链路。

**标题/描述/摘要/预测没有原生配置字段。** Codex TUI 的标题生成在 provider 不是 `openai` 时直接使用当前会话模型；Codex App 的标题/描述/摘要使用它自己的模型（Magpie 观察到是 `gpt-6-luna`），预测使用会话模型。因此这些任务只能由网关在请求层拦截和改写。

### 2.2 请求标记

Codex 通过以下信息标记辅助调用：

| 位置 | 值 | 含义 |
|---|---|---|
| `x-openai-subagent` header | `guardian` / `auto_review` | 自动审批 |
| `x-openai-subagent` header | `review` / `compact` / `memory_consolidation` | 其他隐藏调用 |
| `x-openai-subagent` header | `collab_spawn` | 用户可见的子代理回合，**不是**辅助调用 |
| `x-openai-memgen-request` header | `true` | 记忆请求 |
| `x-codex-turn-metadata` header/body | `thread_source` | 线程来源 |
| 同上 | `request_kind` | `turn` / `memory` / `compaction` 等 |

`thread_source` 中**表示辅助调用**的值（分类白名单）：

- `thread_title`
- `thread_title_reconsideration`
- `title_generation`
- `thread_description`
- `thread_summary`
- `composer_predictions`
- `guardian_review`
- `memory_consolidation`

其余值是普通来源，不参与辅助分类，例如：

- `user`：Codex App 的普通用户会话
- `subagent`：子代理会话
- `automation` / `voice_chat` / `realtime_voice` 等客户端 Feature 标签
- 未识别的值

`x-codex-turn-metadata` 是 header 投影；规范传输是 Responses body 的 `client_metadata["x-codex-turn-metadata"]`（值可能是 JSON 字符串或对象）。网关必须同时支持两处。

### 2.3 记忆提取与整理无法在线上区分

Codex 的提取（phase 1）和整理（phase 2）都使用：

- `thread_source = memory_consolidation`
- `request_kind = memory`
- `x-openai-memgen-request: true`

两者唯一的区别是请求 body 里的 `model`（分别来自 `extract_model` / `consolidation_model`）。因此：

- 原生 `[memories]` 的两个字段可以分别配置；
- 渠道级辅助模型首期只提供一条 `memory` 路由；如果要分别路由，需要根据请求 model 与 provider 里保存的原生记忆模型匹配，见 §6.3。

### 2.4 结构化输出与协议矩阵

标题/描述/摘要/预测依赖 Responses 的 `text.format` JSON Schema。记忆提取也使用严格 output schema（`output_schema_strict = true`）。

当前 transformer 的行为：

| 目标协议 | `text.format` 处理 | 是否可直接路由 |
|---|---|---|
| OpenAI Responses | 原样保留 | ✅ |
| OpenAI Chat | 映射为 `response_format` | ✅（依赖上游遵守） |
| Gemini Native | 映射为 `responseMimeType` / `responseSchema` | ✅（依赖上游遵守） |
| Anthropic Messages | **丢弃** | ❌ 必须做响应整形或明确报错 |

即使目标协议支持 schema，第三方模型也可能忽略它并返回纯文本。Magpie 的做法是把纯文本包成 Codex 期望的对象：

- 标题：`{"title": "..."}`（必要时补齐 schema 要求的其他字符串字段）
- 描述：`{"description": "..."}`
- 摘要：`{"summary": "..."}`（必要时补齐 `compactSummary` 等字段）
- 预测：只接受 `{"suggestion": string|null}`；纯文本绝不能当成建议，否则会把模型的话塞进用户输入框

因此“只改模型名”不足以让廉价/本地模型可用；响应整形是完整方案的一部分。

## 3. 非目标

- **不做全局辅助任务表，不做跨渠道辅助路由。** 辅助模型只属于某个 provider，请求跟随当前候选渠道。
- 不改变主用户回合的模型、reasoning effort、工具或 provider 选择。
- 不实现 `/review` 的 `review_model` UI（可后续复用同一机制）。
- 不把其他 CLI 的辅助调用纳入本期。
- 不重做现有 auto-review 的审批语义；只在其前面增加可选的渠道级显式模型。
- 不把辅助任务单独计入新的统计口径；首期只做请求级可见性。

## 4. 总体设计

### 4.1 按渠道配置

```
provider.settingsConfig.auxiliaryModels
    title / description / summary / prediction / memory
        缺键（自动） / { mode: "main" } / { mode: "model", model: "..." }
        ↓ 只在当前候选 provider 内生效
provider.settingsConfig.config 的 [memories]
    extract_model / consolidation_model（原生，直连也生效）
        ↓
自动回退
    保留请求原值 / provider 默认模型
```

辅助请求不改变候选列表，只改变**当前候选**转发时使用的模型。三种 Gateway 模式的行为：

| 模式 | 行为 |
|---|---|
| single | 只有 P0；使用 P0 的渠道级辅助模型。 |
| failover | 每个候选使用自己的渠道级辅助模型；候选 A 没配就用 A 的自动回退，切到候选 B 时按 B 的配置解析。 |
| aggregate | 先按现有 slug/前缀/声明目录解析出目标站点，再使用该站点的渠道级辅助模型。 |

`mode: "model"` 的值是**该渠道的上游模型 ID**，不是网关别名或聚合 slug；`mode: "main"` 引用该渠道的默认模型；缺键是自动回退。三者都在选渠道之后才生效，没有跨渠道解析的需求。如果用户希望标题走另一个渠道，就把那个渠道设为主渠道/候选渠道，或改用该渠道的模型。

### 4.2 优先级

对单个辅助请求，按以下顺序解析（先命中者胜）：

1. 当前候选 provider 的 `auxiliaryModels.<task>`：
   - `{ "mode": "model", "model": "..." }`：直接使用该上游模型 ID；
   - `{ "mode": "main" }`：使用该 provider 的 `default_model`；provider 没有 `default_model` 时返回配置错误 `aux_task_main_model_missing`，**不回退到请求模型**；
   - 缺键：进入第 3 步。
2. 记忆任务：请求里已有的原生 `[memories]` 模型（Codex 已经按 `extract_model` / `consolidation_model` 发出），保持；
3. 自动回退（只在该任务缺键时）：
   - 如果请求模型不被当前 provider 的 `declared_models` 声明，且 provider 有 `default_model`，改用 `default_model`；
   - provider 没有模型目录、无法判断是否支持时，保持请求原值（避免误改）；
   - provider 没有 `default_model` 时，保持请求原值。
4. 未配置任何东西：保持现状（single 直通；failover 按现有 default/auto-review 规则）。

> “默认模型/主模型”在这里指 provider `config` 顶层的 `model`（Codex 读 `[chat].model` 优先，`codex_model_from_config` 已统一），也就是供应商表单里的默认模型，不是模型目录的第一项。它也**不是**“当前 Codex 线程实际使用的模型”；后者需要按 thread id 跟踪主回合，本期不做（参见 Magpie 的 `codexLeads`）。用户切换会话模型但没改渠道默认模型时，`main` 仍走渠道默认模型。

自动审批是例外：它继续使用现有的 `settingsConfig.autoReviewModelOverride`（provider 级单值）和现有的 failover 解析，不放进 `auxiliaryModels`，避免两套同义字段。

### 4.3 任务标识

后端统一使用枚举键，持久化用 snake_case：

```
title                 # thread_title / thread_title_reconsideration / title_generation
description           # thread_description
summary               # thread_summary
prediction            # composer_predictions
memory                # memory_consolidation / memgen
auto_review           # guardian / auto_review / guardian_review（只观测；模型走 autoReviewModelOverride）
```

其他隐藏调用（`review`、`compact`）只做观测分类，不进入首期路由表；保留 `other` 分类用于详情展示。`collab_spawn` 是用户可见的子代理回合，不参与辅助分类。

### 4.4 生效范围（重要）

不是所有设置都只在网关模式生效。按配置类型区分：

| 配置 | 直连（Codex 不经过网关） | Gateway 接管 |
|---|---|---|
| `auxiliaryModels.title/description/summary/prediction` | ❌ 不生效：Codex 没有原生字段，请求也不经过网关 | ✅ 网关改写 |
| `auxiliaryModels.memory` | ❌ 不生效：请求不经过网关 | ✅ 覆盖原生记忆模型 |
| `[memories] extract_model/consolidation_model` | ✅ Codex 直接读取 | ✅ Codex 读取；网关可覆盖 |
| `autoReviewModelOverride`（模型目录字段） | ✅ Codex 读取 catalog | ✅ |
| `aux_task` 请求/用量观测 | ❌ 没有网关请求记录 | ✅ |

因此：

- **渠道级辅助模型是网关模式专属**，UI 必须明确标注“仅网关接管时生效”，不能把它描述成直连也生效。
- **原生 `[memories]` 两个字段不是网关专属**，直连时由 Codex 自己读取；网关模式只是多了一层可覆盖的 `auxiliaryModels.memory`。
- 现有 `autoReviewModelOverride` 也不是网关专属：它写进模型目录，直连时 Codex 同样会读。
- 观测只覆盖经过网关的请求；直连请求不会出现在 AI Toolbox 的请求详情里。

## 5. 数据模型

### 5.1 渠道级辅助模型

放在 provider `settingsConfig` 顶层，和 `autoReviewModelOverride` 同级（`web/types/codex.ts` 的 `CodexSettingsConfig`）：

```json
{
  "config": "...",
  "auxiliaryModels": {
    "title": { "mode": "model", "model": "gpt-5-mini" },
    "description": { "mode": "model", "model": "gpt-5-mini" },
    "summary": { "mode": "model", "model": "gpt-5-mini" },
    "prediction": { "mode": "model", "model": "deepseek-flash" },
    "memory": { "mode": "main" }
  }
}
```

三态语义：

| 状态 | 存储 | 运行时 |
|---|---|---|
| 自动（默认） | **缺键**（`auxiliaryModels` 里没有该任务） | 请求模型被渠道声明就保留，否则回退 `default_model` |
| 跟随主模型 | `{ "mode": "main" }` | 无条件使用渠道 `default_model` |
| 指定模型 | `{ "mode": "model", "model": "<上游模型 ID>" }` | 使用该模型 |

类型：

```ts
type CodexAuxiliaryModelRoute =
  | { mode: 'main' }
  | { mode: 'model'; model: string };
```

- 只保存当前 provider 能服务的目标；官方 provider 不保存/不投影这些字段（与 `autoReviewModelOverride` 一致）。
- **不要**用 `"mode": "auto"`、空串或 `null` 表示自动；自动的唯一表示是缺键。这样切换回自动时是删除键，不会留下旧模型名。
- 切换模式时整体替换对象：从“指定模型”切到“跟随主模型”或“自动”时，不能保留旧的 `model` 字段，否则会形成陈旧映射。
- `mode: "model"` 的 `model` trim 后不能为空；不做存在性硬校验，允许用户手填目录尚未刷新的模型。空串按缺键（自动）处理并记录一条 warning。
- 读取旧值/手改值时只接受上述两种对象；其他形状按缺键（自动）处理，不阻断表单。
- `mode: "main"` 要求 provider 有 `default_model`；UI 在保存时校验，运行时缺失则返回明确错误，不静默改用请求模型。
- 模型目录刷新、catalog 重新生成不碰这个字段；它是用户意图，不是派生数据。
- 运行时在 `runtime/providers.rs` 的 Codex provider 加载路径解析成 `UpstreamModelMapping.auxiliary_models: BTreeMap<String, CodexAuxiliaryModelRoute>`；`CodexAuxiliaryModelRoute` 需派生 `PartialEq, Eq`，因为 `UpstreamModelMapping` 已经派生它们。

### 5.2 原生 `[memories]`

原生字段仍以 provider 的 `settingsConfig.config` TOML 为唯一来源：

```toml
[memories]
extract_model = "gpt-5-mini"
consolidation_model = "gpt-5-mini"
```

不在 `settingsConfig` 里再建一套 `memories` 对象，避免双源。UI 只做结构化编辑：

- 解析当前 `configToml`；
- 只改 `[memories].extract_model` / `consolidation_model`；
- 保留 `[memories]` 的其他键（`version`、`generate_memories`、`use_memories` 等）和文件其他内容；
- 清空输入框时删除对应键，回到 **Codex 默认记忆模型**；这不是“跟随主模型”。原生 Codex 没有“主模型”引用，直连时若要用渠道默认模型，必须显式填写该模型名；网关模式下可改用 `auxiliaryModels.memory = { "mode": "main" }`；
- TOML 解析失败时沿用现有表单校验，不写入。

这样直连、WSL、备份恢复、切换 provider 都沿用现有 `config.toml` 链路，不需要新的投影代码。

### 5.3 写入路径与配置透传

`auxiliaryModels` 是 provider 存储的一部分，必须补齐所有透传点，漏一处就会在保存模型目录/切换卡片时静默丢失：

- 后端 `adapter::to_db_value_provider` 写入 JSONB；
- `extract_provider_settings_for_storage` 从 provider TOML 提取存储字段时保留它；
- 前端 `CodexSettingsConfig` 增加 `auxiliaryModels?: Partial<Record<'title' | 'description' | 'summary' | 'prediction' | 'memory', CodexAuxiliaryModelRoute>>`；`buildCodexSettingsConfig`、`useCodexConfigState` 的 `parseInitialCodexState` / `CodexSettingsConfigSnapshot` / `resetFromSettingsConfig` / `getFinalSettingsConfig` / `resetCodexConfig` 原样保留这个对象；
- `CodexPage.persistProviderCatalog` 和两条 legacy 重建分支；
- 供应商复制/导入/CC Switch 导入如果重建 settingsConfig，也要带上它（或按“未知字段保留”原则原样透传）。

它不进入 `modelCatalog`，也不进入生成的 `ai-toolbox-codex-model-catalog.json`。

### 5.4 为什么不做进模型目录

`auto_review_model_override` 必须写进 catalog，因为 Codex 只从当前会话模型的 catalog 条目读取它。标题/预测/记忆没有这种机制，写进 catalog 不会生效；把用户映射写进 catalog 还会在每次目录刷新时面临被覆盖的风险。因此辅助模型只存 provider 配置，不写 catalog。

## 6. 运行时

### 6.1 分类

新增 `tauri/src/coding/proxy_gateway/runtime/compat/codex_aux_task.rs`（纯函数，便于单测）：

```rust
pub(crate) enum CodexAuxTask {
    Title,
    Description,
    Summary,
    Prediction,
    Memory,
    AutoReview,
    Review,
    Compact,
}

impl CodexAuxTask {
    /// 只有可路由的任务有持久化键；AutoReview/Review/Compact 返回 None。
    pub(crate) fn route_key(&self) -> Option<&'static str> {
        match self {
            Self::Title => Some("title"),
            Self::Description => Some("description"),
            Self::Summary => Some("summary"),
            Self::Prediction => Some("prediction"),
            Self::Memory => Some("memory"),
            _ => None,
        }
    }
}

pub(crate) fn classify_codex_aux_task(request: &DebugHttpRequest) -> Option<CodexAuxTask>;
```

解析顺序：

1. `x-openai-subagent`（trim、小写）；
2. `x-openai-memgen-request` 非空 → `Memory`；
3. `x-codex-turn-metadata` header；
4. body `client_metadata["x-codex-turn-metadata"]`（字符串或对象）；
5. 都取不到 → `None`（视为普通回合）。

分类采用**白名单**：只有上面列出的已知辅助值才产生任务类型。

- `x-openai-subagent: collab_spawn` 是用户可见的子代理回合 → `None`；
- `thread_source` 为 `user` / `subagent` / 客户端 Feature 标签 / 未识别值 → 不分类；如果同一请求的 `request_kind == "memory"`，仍按兜底归为 `Memory`；
- 未识别的 `x-openai-subagent` 标签 → `None`，不猜测。

> 2026-11-11 修正（issue #460）：旧实现把所有未识别的 `thread_source` / `x-openai-subagent` 都归为 `Other` 并在列表显示“辅助”，导致 Codex App 的普通会话（`thread_source = user`）和子代理（`subagent` / `collab_spawn`）被误标。现在只有白名单信号才打标。

安全边界：

- 只对 `GatewayCliKey::Codex` 路由分类；
- 解析失败按 `None` 处理，不影响主回合。

### 6.2 候选与模型解析

候选列表完全沿用现有逻辑（single/failover/aggregate），辅助路由**不增加、不删除、不重排候选**：

- single：只有 P0，使用 P0 的渠道配置；
- failover：每个候选在进入 attempt 前解析自己的渠道配置；
- aggregate：站点解析完成后，使用目标站点的渠道配置。

每个候选的辅助模型解析：

```text
resolve_aux_model(task, requested_model, provider) -> Result<Option<String>, AuxRouteError>
  1. match task.route_key().and_then(|key| provider.auxiliary_models.get(key)):
       Some({ mode: "model", model })  -> 使用 model（上游模型 ID）
       Some({ mode: "main" })          -> provider.default_model
                                          或 Err(aux_task_main_model_missing)
       None                            -> 进入第 2 步
  2. task == Memory 且 requested_model 是原生 [memories] 模型 -> 保留
  3. requested_model 未被 provider.declared_models 声明
     且 provider.default_model 存在 -> default_model
  4. 其它 -> 保留 requested_model
```

规则细节：

- `mode: "model"` 的值作为**上游模型 ID**直接使用；剥 `[1M]` 后转发，不再走 provider 的 default/family 映射。仍应用 provider 的 `modelRewrites`（provider 级 wire 适配），匹配键用改写前的辅助模型名。
- `mode: "main"` 只使用 `default_model`；没有 `default_model` 是配置错误，不退回请求模型。single 直接返回 400；failover 可以按现有规则尝试下一个候选，由下一个候选解析自己的配置。
- 自动回退只依据 `declared_models`，不依据模型名猜测；`declared_models` 为空时保持请求原值。
- 记忆任务的原生模型判断：从 provider 的 `settingsConfig.config` 解析 `[memories].extract_model` / `consolidation_model`，请求模型等于其中任一值时视为“用户已配置”，不做自动回退。`auxiliaryModels.memory` 显式配置时优先于原生字段。
- 自动审批不经过这条解析；它继续用 `autoReviewModelOverride` / `default_model` 的现有 failover 规则。
- 连通性测试（`provider_override_id` 非空）完全跳过辅助解析，保持被测模型不变。
- `modelRewrites`、`modelServiceTiers`、协议转换、auth、图片策略等仍按当前候选 provider 正常执行。

如果渠道级辅助模型被上游拒绝（404/400），按普通请求错误处理：single 直接失败；failover 可以按现有规则尝试下一个候选，而下个候选使用它自己的辅助模型。错误信息要能看出是辅助请求、哪个任务、哪个模型。

### 6.3 记忆提取/整理

- 首期只提供一条 `memory` 渠道路由，覆盖提取和整理。
- 原生 `[memories]` 仍然可以分别配置两个字段，直连时按 Codex 语义生效。
- 如果后续需要网关分别路由：用请求 model 与 provider 原生 `extract_model` / `consolidation_model` 比较；等于 `extract_model` → 视为提取，等于 `consolidation_model` → 视为整理；两者都不等或相等时按单一 `memory` 处理。存在误判风险，本期不做。

### 6.4 自动审批

保持现有语义：

- Codex 先按模型目录的 `auto_review_model_override` 选择请求模型；
- failover 下网关再按候选渠道的 `autoReviewModelOverride` 解析；
- 本期不为 auto-review 增加渠道级 `auxiliaryModels` 字段；如果后续要统一，再把 `autoReviewModelOverride` 迁进 `auxiliaryModels.autoReview`，但必须保留旧字段读取兼容。

审批的安全约束不变：不允许因为辅助路由失败而自动批准或降级审批；请求失败就返回失败，由 Codex 自己处理。

### 6.5 WebSocket

Codex 的 Responses WebSocket（`codex_websocket_enabled`）也会承载辅助调用。当前实现：

- 在 `response.create` 帧上复用同一分类器（`client_metadata["x-codex-turn-metadata"]` 在帧 body 里）和 `resolve_codex_aux_model`；
- 对有 `route_key` 的任务（title / description / summary / prediction / memory），上游请求改走现有 **HTTP 非流式辅助通道**：
  - 从原始 `response.create` 去掉 `type` / `stream_id` / `event_id` / `generate` / `background` / `stream_options`；
  - 设置 `stream: false` 和解析出的辅助模型；
  - 复用 HTTP 侧的 SSE 聚合、`shape_aux_response`、usage、pipeline 与隐私还原。
- relay 再把 HTTP 返回的 JSON 合成为裸 JSON WebSocket 帧（与 `build_aux_sse` 共用同一套事件生命周期）；
- 真实 Codex 的 `response.create` 通常不带 `stream_id`；每个无 `stream_id` 的辅助 HTTP 轮次使用内部唯一 lane（`aux-http-<request_id>`），避免 HTTP 完成乱序时被当成过期结果丢弃，也避免后续主回合的上游事件关联到辅助轮次；
- HTTP 失败必须合成 `response.created`（`in_progress`）+ `response.failed`，两者共享 `resp_aux_ws_<request_id>`：真实 Codex 会把没有创建事件的裸 `response.failed` 当成流断开并触发重连；整形失败 code 使用 `aux_task_empty_response` / `aux_task_unexpected_response`，上游错误保留原始 body；
- HTTP 2xx 但 body 是协议错误信封（`response.failed` / 顶层 `error`）时不得送入整形，必须识别信封并按上游错误返回 `response.failed`（`aux_task_upstream_bad_request`），保留上游 `error.message`；
- `auto_review` / `review` / `compact` 没有 `route_key`，继续走 WS 直通，只做观测分类；
- 主回合仍走原来的 pinned WebSocket，不因辅助调用切换模型。

**为什么辅助轮次不走同一条上游 WebSocket**：New API 系中转会把 Responses WebSocket 锁定到首个成功模型。辅助模型通常与主模型不同，同 socket 改写会被上游以 model-lock 拒绝，网关必须断开连接让 Codex 重连；如果辅助调用与主回合并发，还会影响同一连接上的其他 lane。改走一次性 HTTP 辅助请求可以彻底避开这个约束，并直接复用 HTTP 侧已经验证过的整形链路。

## 7. 响应整形

### 7.1 何时需要

- 目标协议是 Anthropic Messages：必须整形（schema 被丢弃）。
- 目标协议是 Chat/Gemini/Responses，但上游返回的 assistant 文本不是任务期望的 JSON：整形可以把纯文本包成期望结构。
- 预测任务例外：纯文本永远不能当 suggestion；只能接受 JSON 里的 `suggestion`，否则返回 `{"suggestion":null}` 并记一条失败原因。

### 7.2 算法

对需要整形的请求：

1. 改写上游 body 为 `stream: false`（去掉 `stream_options`），让上游一次性返回；
2. 走现有协议转换，得到面向客户端的 Responses 形态 body；
3. 从 `output[]` 的 assistant message 提取文本；
4. 按任务包装：

   | 任务 | 结果 |
   |---|---|
   | Title | `{"title": firstLine(text), ...schema required fields}`，按 `maxLength` 截断 |
   | Description | `{"description": nonEmpty(text)}` |
   | Summary | `{"summary": oneLine(text), ...required fields}` |
   | Prediction | 只接受 `{"suggestion": string|null}`；纯文本 → `{"suggestion":null}` |

5. 生成 Responses 响应：
   - 客户端非流式：返回 JSON，`output` 里一条 message，`usage` 用上游 usage；
   - 客户端流式（Codex 常态）：合成 `response.created` → `response.output_item.added` → `response.output_item.done` → `response.completed`，并带 usage；
   - WebSocket 客户端：HTTP 辅助通道先返回完整 JSON，relay 再用同一套生命周期合成裸 JSON 文本帧（`build_aux_frames`）；
6. 原样保留上游错误；2xx 但整形失败（空文本、无法解析）返回明确 502 `aux_task_empty_response` / `aux_task_unexpected_response`，并在详情里保存原始上游 body；
7. `upstream_response_body` 保留真实上游 body，`response_body` 保存整形后的 body，便于排障；
8. token/cost 使用上游 usage，不能因为整形丢失计费。

### 7.3 与现有 runtime 的衔接

- 复用 `runtime/compat/codex_remote_compaction.rs` 已经验证过的“强制非流 → 聚合 → 按客户端 stream 重渲染”模式；
- 整形只作用于 Codex 路由且 `aux_task` 命中的请求，不影响普通回合；
- 不要把整形塞进 `transformer`：它是 Codex 任务语义，不是协议转换；
- 如果上游是 Responses 且已经返回合法 JSON，整形应是无操作（保留原 body，避免不必要的改写和日志噪音）；
- 失败重试/故障转移：整形发生在成功响应之后时，建议直接返回整形错误，不再重试（换渠道通常同样返回纯文本），并记录原因。换渠道的模型改写仍按各候选自己的配置执行。

## 8. 原生记忆与渠道辅助模型 UI

### 8.1 位置

Codex 供应商表单（`CodexProviderFormModal`）的“模型”区域下方新增一个可折叠的“辅助模型”小节：

- **渠道级辅助模型**（仅网关接管时生效），每个任务一行：
  - 会话标题
  - 会话描述
  - 会话摘要
  - 输入框预测
  - 记忆（提取与整理共用）
  - 每行左侧是模式选择（自动 / 跟随主模型 / 指定模型），右侧是模型输入框（仅“指定模型”时显示），与顶部“模型名称”共用 `modelOptions` AutoComplete：可下拉选择当前渠道已知模型，也保留手输任意上游模型 ID 的能力。
- **原生记忆模型**（直连也生效）：
  - 记忆提取模型（`extract_model`）
  - 记忆整理模型（`consolidation_model`）
  - 两个输入框同样复用 `modelOptions` AutoComplete，可下拉选择或手输；留空 placeholder 写 **“Codex 默认”**，不能写“跟随主模型”。
- 自动审批模型继续使用现有的 `autoReviewModelOverride` 控件，不重复。

官方 provider 不显示/不保存这些字段。

每行说明文案要写清楚：

- 渠道级辅助模型只在 Gateway 接管时生效；
- 自动 = 请求模型可用时保留，否则回退渠道默认模型；
- 跟随主模型 = 始终使用当前渠道的默认模型（不是当前 Codex 会话实际切换到的模型）；
- 指定模型 = 使用该上游模型 ID，网关不会跨渠道解析；
- “跟随主模型”在渠道没有默认模型时禁用，并提示“请先设置默认模型”；
- 模式选择旁显示当前默认模型，例如“跟随主模型（gpt-5.2）”，让用户知道会落到哪个模型。

### 8.2 表单提交与 TOML 合并

渠道级辅助模型：

- 作为 `settingsConfig.auxiliaryModels` 对象提交，走现有 `buildCodexSettingsConfig`；
- 序列化规则：
  - 自动 → 删除该任务的键；
  - 跟随主模型 → `{ "mode": "main" }`；
  - 指定模型 → `{ "mode": "model", "model": trimmed }`；
  - 所有任务都是自动时，删除整个 `auxiliaryModels` 对象；
  - 切换模式时整体替换，不保留旧的 `model` 字段；
- 表单内部可以用 `mode: 'auto' | 'main' | 'model'` 的三态值，提交时把 `auto` 转成“缺键”，不要把 `auto` 写进存储；
- 保存时校验：`main` 要求 `configToml` 能解析出默认模型；`model` 要求非空；
- 默认模型的读取与原生记忆一致，解析当前 `configToml`：顶层 `model` 优先，其次 `[chat].model`；建议抽成纯函数 `readCodexDefaultModel(configToml)` 便于单测；
- 如果用户手动改了 TOML 导致默认模型变空，已有 `main` 任务在保存时显示字段错误，不静默改成自动；
- 模型目录编辑（`persistProviderCatalog`）不能丢失它，见 §5.3。

原生记忆模型：

新增纯函数（放 `web/features/coding/codex/utils/`，便于 node 单测）：

```ts
interface CodexMemoriesModels {
  extractModel?: string;
  consolidationModel?: string;
}

readCodexMemoriesModels(configToml: string): CodexMemoriesModels

patchCodexMemoriesModels(
  configToml: string,
  next: CodexMemoriesModels,
): {
  nextToml: string;
  changed: boolean;
  /** `inline_table` / `dotted_keys` / `invalid_toml`；非空时结构化编辑禁用 */
  unsupportedShape?: 'inline_table' | 'dotted_keys' | 'invalid_toml';
}
```

### 8.3 已有 `[memories]` 区块的处理（重点）

表单里的两个原生记忆输入框**不是独立的 Form 字段**，而是从当前供应商存储的 `settingsConfig.config`（弹窗里的 `configToml` 文本）派生。打开弹窗时用 `smol-toml` 解析，读取 `memories.extract_model` / `consolidation_model` 回填；缺失则留空，placeholder 显示“Codex 默认”。

用户修改任一字段时，只定向修改 `[memories]` 对应的键，**不重建整份 TOML**：

| 现有形态 | 处理 |
|---|---|
| 有 `[memories]` 表且含目标 key | 只替换该行的值；保留缩进、行内注释和键的引号写法 |
| 有 `[memories]` 表但缺目标 key | 在表内最后一个有效行之后插入一行；缩进沿用该表现有 key |
| 有 `[memories]` 表，目标 key 的值为空 | 按“有 key”处理；用户清空输入框时删除该行 |
| 没有 `[memories]` 表 | 在文件末尾追加 `[memories]` 表，只写用户设置的键；不改变现有内容 |
| `memories = { ... }` 内联表 | **不自动改**；输入框禁用，提示“请在下方 TOML 编辑器中修改” |
| `memories.extract_model = ...` 点号键 | 同上，不自动改 |
| `[memories]` 重复、TOML 无效 | 沿用现有 `validateTomlRule` 错误；输入框禁用 |
| 目标 key 的值不是字符串 | 不回填；用户显式修改后替换为字符串，help 里给出提示 |
| 值是多行字符串 | 不自动改；按不支持的形态处理，提示在 TOML 编辑器里修改 |

必须保持：

- `[memories]` 的其他键（`version`、`dual_write`、`generate_memories`、`use_memories`、`dedicated_tools`、`max_raw_memories_for_consolidation`、`max_unused_days`、`max_rollout_age_days`、`max_rollouts_per_startup`、`min_rollout_idle_hours`、`min_rate_limit_remaining_percent`、`consolidation_service_tier` 等）逐字保留；
- 注释、空行、表顺序、键顺序、CRLF/LF 行尾、文件末尾换行；
- 只改 `[memories]` 内的两行，不碰 `base_url`、`model`、`[model_providers.*]` 和其他表。

清空一个字段只删除对应的键行，**不删除整个 `[memories]` 表**；表因此变空也保留（Codex 接受空表），避免误删用户手写的其他键或注释。

写入时的状态同步：

- 同时调用 `setCodexConfig(nextToml)` 和 `form.setFieldValue('configToml', nextToml)`，避免用户改完立刻点保存时 `getFieldsValue(true)` 读到旧的 `configToml`；
- 现有 `codexConfig → configToml` 的同步 effect 继续作为兜底；
- 原始 TOML 编辑器改动后，结构化输入框从 `codexConfig` 重新派生，保持双向一致。

### 8.4 存储片段 vs 运行文件

这里编辑的是**该供应商保存的 `config.toml` 片段**，不是当前 `~/.codex/config.toml` 运行文件。应用时后端按现有 merge 语义写入：

- 上一次 AI Toolbox 写过的键会被更新或移除；
- 用户手工加在 live 文件里、从未进入 provider 存储的 `[memories]` 键会被保留；
- 如果 live 文件里已有 `[memories]` 而 provider 存储里没有，弹窗不会显示它；应用该 provider 时 live 的其余键保持不变。

UI 用 12px 辅助文字说明这一点，避免用户误以为表单会镜像当前运行文件。

> 切换官方/自定义类别时，现有 `normalizeCodexConfigForOfficialMode` / `ensureCodexCustomProviderConfig` 会走 `parseToml + stringifyToml`，可能重排整份 TOML。这是既有行为；它会保留 `[memories]` 的键值，但可能丢失注释和格式。结构化记忆编辑本身不触发这条路径。

### 8.5 Gateway 接管与恢复

- 接管 `patch_codex_config` 只改 active provider 表，不改 `[memories]`；
- 切换 provider 的 Gateway-aware 编排（restore → apply → re-engage）会把新 provider 的 `[memories]` 和 `auxiliaryModels` 生效；
- `auxiliaryModels` 只存在 DB，不写 runtime 文件；接管期间切换 provider 后，网关按新的已应用 provider 读取；
- 恢复直连按 manifest 原始备份回填；如果用户在接管期间改了 provider 的 `[memories]`，恢复直连后可能回到旧值，这是现有“接管期间 runtime 由网关托管”的一致语义，UI 提示按现有规则处理。

## 9. 可观测性

### 9.1 后端摘要

`GatewayRequestLogSummary` 新增：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub aux_task: Option<CodexAuxTaskKind>,   // title/description/summary/prediction/memory/auto_review/other
#[serde(default, skip_serializing_if = "Option::is_none")]
pub aux_route_source: Option<String>,     // explicit/main/auto_kept/auto_fallback/none
```

`CodexAuxTaskKind` 的 `as_str` 与 DB 列一致；未知类型存 `other`，原始值放详情，不进 compact 摘要（避免不可控长度）。

SQLite：`TARGET_SCHEMA_VERSION` 从 36 升到 37，新增：

```sql
ALTER TABLE proxy_request_logs ADD COLUMN aux_task TEXT;
ALTER TABLE proxy_request_logs ADD COLUMN aux_route_source TEXT;
```

规则：

- 旧行 NULL，查询回退为“普通请求”；
- 不写 `usage_daily_rollups`（首期不做辅助任务统计维度）；
- 请求详情 JSONL 新增 `auxiliary` 对象，detail-only：

```json
{
  "task": "title",
  "raw_kind": "thread_title",
  "configured_mode": "model",
  "route_source": "explicit",
  "requested_model": "gpt-6-luna",
  "routed_model": "deepseek-flash",
  "provider_id": "site-a",
  "target_protocol": "openai_chat",
  "shaped": true,
  "route_applied": true
}
```

- `task`/`route_source` 是 compact 摘要；`configured_mode`、`raw_kind`、`target_protocol`、`shaped`、`route_applied` 只在详情；
- `route_source` 取值：`explicit`（指定模型）、`main`（强制主模型）、`auto_kept`（自动且保留请求模型）、`auto_fallback`（自动且回退默认模型）、`none`（未应用/无法分类）；
- 路由解析失败（如 `aux_task_main_model_missing`）仍记录 `aux_task`，`aux_route_source = none`，详情记录 `route_error`；请求失败但观测不丢；
- 详情导出继续走现有脱敏逻辑；任务类型和模型名不是敏感信息，但 header/body 的脱敏规则不变。

### 9.2 前端

- 请求列表：模型列旁增加小徽标（标题/描述/摘要/预测/记忆/审批/辅助），hover 显示原始 kind；普通请求不显示。
- 请求详情：新增“用途”行，显示任务、路由来源、实际模型、provider、目标协议、是否整形；未知类型显示“辅助（未知）”，不隐藏。
- 筛选：首期不加“按用途筛选”，避免列表工具栏过密；后续可加。
- i18n：`gateway.page.requests.auxTask.*`；文案遵循 `DESIGN.md` 的 10px/12px 辅助信息层级和现有 badge 样式。

## 10. 错误与回退矩阵

| 场景 | 行为 |
|---|---|
| 未配置任何渠道辅助模型 | 现状不变 |
| 渠道辅助模型合法 | 使用该模型；候选列表不变 |
| `mode: "main"` 但 provider 没有 `default_model` | 本地 400 `aux_task_main_model_missing`；不回退请求模型；UI 保存时已阻止 |
| `mode: "model"` 但模型名为空 | 按缺键（自动）处理并记录 warning；UI 保存时已阻止 |
| 渠道辅助模型被上游拒绝（404/400） | 按普通请求错误处理；failover 可尝试下一个候选，下个候选用自己的渠道配置 |
| 目标协议 Anthropic 且任务需要结构化输出（首期未整形） | 本地 400 `aux_task_protocol_unsupported`，message 说明需要整形或换 Responses/Chat/Gemini 渠道 |
| 聚合模式模型解析不到 | 404 `aux_task_model_unavailable`（现有聚合语义） |
| 上游支持 schema 但返回纯文本（整形未实现时） | 502 `aux_task_unexpected_response`，详情保存原始 body |
| 整形后仍无有效内容 | 502 `aux_task_empty_response` |
| 预测返回纯文本 | `{"suggestion":null}`，记录一条失败原因；绝不把文本塞进 composer |
| 辅助请求失败 | 只影响该请求；不改变主回合、不改变审批结果 |
| 连通性测试 | 完全跳过辅助模型解析（`provider_override_id` 非空） |
| WebSocket 且配置了渠道辅助模型 | 有 `route_key` 的任务走 HTTP 辅助通道并改写模型；详情记录 `route_applied=true` 与实际 `shaped`；辅助请求失败只结束该轮 |
| provider 未配置辅助模型 | 自动回退；不报错 |

## 11. 测试矩阵

### 11.1 分类器

- `x-openai-subagent: guardian / auto_review / review / compact`；
- `x-openai-memgen-request: true`；
- header `x-codex-turn-metadata` 为 JSON 字符串；
- body `client_metadata` 为字符串或对象；
- `thread_title` / `thread_title_reconsideration` / `title_generation` 归为 title；
- `thread_description` / `thread_summary` / `composer_predictions`；
- 未知 kind → `other`，raw 截断；
- 无标记 → `None`；损坏 JSON → `None`；
- 非 Codex CLI 不分类。

### 11.2 路由

- 渠道辅助模型命中：single / failover / aggregate 三种模式；
- failover 下每个候选使用自己的渠道配置；
- `mode: "model"` 使用指定模型；
- `mode: "main"` 使用当前候选的 `default_model`；
- `mode: "main"` 且无 `default_model` → 明确错误；failover 可尝试下一个候选；
- 候选未配置 → 自动回退；其他候选配置不影响它；
- 自动模式保留请求模型 vs 回退 `default_model` 两种分支；
- 自动回退只在 provider 有 `declared_models` 且请求模型未声明时生效；
- provider 无目录时保持请求模型；
- 原生 `[memories]` 模型命中时不做自动回退；
- 主回合模型不受辅助路由影响；
- 连通性测试不应用辅助路由；
- 聚合模式下渠道辅助模型按目标站点解析，不按 slug 再解析；
- 官方 provider 不参与。

### 11.3 响应整形

- 纯文本 → title/description/summary JSON；
- JSON 带 code fence、带 reasoning 前缀；
- `maxLength` 截断；
- 预测只接受 JSON suggestion；纯文本 → null；
- 流式合成事件的顺序、幂等和 usage；
- 上游错误原样透传；
- 空文本/不可解析 → 明确 502；
- `upstream_response_body` 与 `response_body` 分别保存；
- 计费使用上游 usage。

### 11.4 原生记忆与表单

- 表单读取/写回保留 `[memories]` 其他键和注释；
- 清空字段删除键；
- 切换 provider 后 `[memories]` 跟随；
- Gateway 接管/恢复、WSL Direct、备份恢复；
- TOML 无效时不写入；
- 渠道辅助模型三态序列化：自动 → 缺键、main → `{mode:"main"}`、model → `{mode:"model",model}`；
- `model → main → auto` 切换后不保留旧 `model` 字段；空 `auxiliaryModels` 对象被丢弃；
- `main` 任务在默认模型为空时表单阻止保存；
- 渠道辅助模型在模型目录编辑、复制、导入后不丢失。

### 11.5 可观测性

- SQLite v37 迁移幂等；旧行 NULL 回退；
- summary/详情/JSONL/导出字段一致；
- 请求列表徽标、详情行、未知类型；
- 日志关闭/截断时统计不变；
- `pnpm test`、`pnpm exec tsc --noEmit`、`pnpm i18n:check`、`cargo test`。

## 12. 一次性交付清单

不分期，以下内容在同一次改动中完成：

1. **观测**：辅助任务分类器 + 单测；`GatewayRequestLogSummary` / SQLite v37 / JSONL detail；请求列表徽标与详情“用途”行。
2. **渠道辅助模型**：provider `settingsConfig.auxiliaryModels` 三态数据模型、所有透传点、运行时解析；Codex 供应商表单的“辅助模型”折叠区（自动 / 跟随主模型 / 指定模型）。
3. **原生记忆**：`[memories]` 结构化字段 + 定向 TOML 补丁（含已有区块、缺键、无表、非法形态的处理）；原始 TOML 编辑器双向同步。
4. **运行时路由**：single / failover / aggregate 按当前候选渠道应用辅助模型；自动回退；`main` 强制 `default_model`；缺少 `default_model` 时明确报错；连通性测试跳过；主回合不受影响。
5. **响应整形**：title/description/summary/prediction 的纯文本包装与合成 Responses 响应；Anthropic 目标解禁；整形失败明确报错。
6. **WebSocket**：在 `response.create` 帧上复用分类与模型改写；有 `route_key` 的任务通过 HTTP 辅助通道发送并合成裸 JSON 帧；`auto_review` / `review` / `compact` 保持 WS 直通。
7. **验证**：`cargo test`、`pnpm test`、`pnpm exec tsc --noEmit`、`pnpm i18n:check`；涉及 UI/样式时按 `DESIGN.md` 检查亮色、暗色、窄窗口和长文本。

不包含（后续单独做）：`off`（本地回答空结果）、记忆 extract/consolidation 分离路由、把 auto-review 迁入 `auxiliaryModels`、`review_model`。

## 13. 风险与待决事项

1. **无法跨渠道路由（已确认接受）**：这是按渠道定义的直接结果。如果用户希望标题走另一个渠道，只能把那个渠道设为主渠道/候选渠道。
2. **记忆提取/整理不可区分（已确认单路由）**：线上标记相同。只提供一条 `memory` 路由；若要分开，必须用请求 model 与 provider 原生 `[memories]` 模型匹配，存在误判风险。
3. **响应整形的工作量**：是完整方案中最大的一块；Anthropic 目标必须整形或明确报错，不能静默失败。
4. **WebSocket**：有 `route_key` 的辅助轮次改走一次性 HTTP 上游请求，避免 New API 的模型锁定与主/辅并发的重连风暴；`auto_review` / `review` / `compact` 仍走 pinned WS，若它们的模型与主模型不同，沿用既有 model-lock 重连逻辑。
5. **模型有效性**：Codex 不校验 `memories.extract_model` / `auxiliaryModels` 是否存在。网关只能在请求时按 provider 目录判断；最终 404/400 要给出可操作的错误文案。
6. **官方订阅**：`category=official` 的 Codex provider 不参与网关候选，不能作为辅助路由目标；UI 要排除并说明原因。
7. **`review_model`**：本期不做，但错误矩阵和任务枚举预留 `review` 分类，避免后续改表。
8. **“主模型”是渠道默认模型，不是当前会话模型**：`main` 使用 provider `default_model`，不跟踪 Codex 线程实际切换到的模型。若用户切换了会话模型但没改渠道默认模型，辅助请求仍走渠道默认模型。需要“跟随当前会话”时要按 thread id 跟踪主回合（Magpie 的 `codexLeads`），本期不做；UI 文案必须写“跟随主模型（渠道默认模型）”，避免误解。
9. **三态存储的透传**：`auxiliaryModels` 是新字段，没有旧数据迁移；但所有 `settingsConfig` 透传/重建点必须原样保留对象，否则会在保存模型目录、复制、导入时丢失。测试要覆盖 `model → main → auto` 的序列化。

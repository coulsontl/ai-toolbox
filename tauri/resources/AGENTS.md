# 模型资源说明

## 一句话职责

- `tauri/resources/` 存放会被后端在编译期嵌入的模型默认数据资源；这里的 JSON 不是运行时缓存副本，而是本仓随版本发布的 bundled defaults。

## Source of Truth

- `preset_models.json` 是预设模型默认数据的源码来源。`tauri/src/coding/preset_models.rs` 通过 `include_str!` 在编译期嵌入它；应用启动后前端先读 app data 里的缓存，再回退到这里的 bundled defaults，随后后台拉远端最新版本更新内存和缓存。
- `models.dev.json` 是 OpenCode 免费/官方模型默认数据的源码来源。`tauri/src/coding/open_code/free_models.rs` 同样通过 `include_str!` 在编译期嵌入它；运行时 app data 里的 `models.dev.json` 只是缓存，不是本仓编辑入口。
- `model_pricing.json` 是 Gateway 官方模型定价默认数据的源码来源。`tauri/src/db/model_pricing_seed.rs` 通过 `include_str!` 编译期嵌入它，并在 SQLite migration 后用 `INSERT OR IGNORE` 增量补齐 `model_pricing` 表。
- `gateway_provider_profiles.json` 是 Gateway 内置供应商 endpoint 默认数据的源码来源。`tauri/src/coding/proxy_gateway/provider_profiles.rs` 通过 `include_str!` 编译期嵌入它；运行时 app data 里的同名文件是远端刷新缓存，用于前端启动后动态更新供应商列表、API 格式和 Base URL。
- 备份/恢复里读写的 `preset_models.json`、`models.dev.json`、`model_pricing.json`、`gateway_provider_profiles.json` 都是 app data 缓存文件；不要把这些缓存链路误认为仓库内 `tauri/resources/*.json` 会被运行时直接原地改写。

## 核心设计决策（Why）

- 这些 JSON 放在 `tauri/resources/`，是为了让应用在无网络、缓存缺失或远端拉取失败时仍有稳定的默认模型数据可用。
- `preset_models.json` 的数组顺序是用户可见语义，不只是排版。前端预设模型选择 UI 直接按分组数组顺序渲染，不会再做二次排序。
- `gateway_provider_profiles.json` 的 endpoint 是内置供应商默认 URL、API 格式、providerType 和 provider 兼容能力的事实源。前端保存内置供应商时只应持久化 `meta.gatewayProfile={tool,profileId,endpointId}` 引用和用户覆盖项，Base URL 继续使用表单中的用户可编辑值；不要把 `meta.apiFormat` / `meta.providerType` / `apiKeyField` / `reasoningField` / `defaultMaxTokens` / 图片策略 / `codexChatReasoning` 等 profile 派生快照写进 provider meta。
- Gateway provider 专属兼容能力必须通过 `gateway_provider_profiles.json` 的明确 profile/endpoint 暴露给用户选择，或通过用户显式 provider meta 配置启用；不要让 runtime 仅凭模型名猜测 provider。比如自定义渠道里 `model=deepseek-*`、`qwen*`、`MiniMax-*` 不应触发 DeepSeek/Qwen/MiniMax 专属请求改写。
- `gateway_provider_profiles.json` 的 `compat` 是 catalog 描述，不是 runtime 执行入口；但后端 validator 会拒绝未知 compat 名称，防止 profile 声明和 runtime 实现漂移。新增 compat 名称时必须同步更新 `tauri/src/coding/proxy_gateway/provider_profiles.rs` 的白名单、runtime adapter/测试和架构文档。

## 关键流程

```mermaid
sequenceDiagram
  participant Repo as tauri/resources/*.json
  participant Rust as Rust include_str! defaults
  participant Cache as App Data Cache
  participant UI as Frontend
  participant Remote as Remote JSON

  Repo->>Rust: compile-time embed
  UI->>Rust: load cached models
  Rust-->>UI: cache if valid, else bundled defaults
  UI->>Remote: background fetch latest preset models
  Remote-->>UI: latest JSON
  UI->>Cache: update cache
  UI-->>UI: refresh in-memory model lists
```

## 易错点与历史坑（Gotchas）

- 改 `preset_models.json` 时，不要只看“有没有这个模型”，还要看它在同组数组里的位置；顺序会直接影响预设标签展示顺序。
- 新增某个模型家族的新版本时，默认应放在对应旧版本前面，而不是机械地塞到整栏最前。比如 `qwen3.6-plus` 放在 `qwen3.5-plus` 前，`kimi-k2.6` 放在 `kimi-k2.5` 前。
- 给新模型补预设时，若需求是“参数和旧模型一样”，优先复制对应旧模型条目，只做 `id` / `name` 和必要顺序调整，不要顺手改能力字段。
- GPT-5.6 的三个正式 OpenAI 模型 ID 是 `gpt-5.6-sol`、`gpt-5.6-terra`、`gpt-5.6-luna`；`gpt-5.6` 只是路由到 Sol 的 alias，不要在预设里与 `gpt-5.6-sol` 重复展示。三者的 API reasoning effort 包含 `max`，但 `ultra` 是 Codex 将 `max` 推理与主动多智能体委派组合起来的客户端档位，不是普通 OpenAI/OpenCode `reasoningEffort` 值。
- xAI 官方模型预设放在 `@ai-sdk/xai`，不要因为 Grok 也兼容 OpenAI API 就塞进 `@ai-sdk/openai-compatible`。`grok-4.5` 是用户可见 canonical ID；`grok-4.5-latest` 和 `grok-build-latest` 可用于价格匹配，但不要作为重复预设展示。Grok 4.5 reasoning 不能关闭，只提供 `low`、`medium`、`high` 三档；在 xAI 官方未公开最大输出 token 前，不要猜测 `outputLimit`。
- `model_pricing.json` 保存标准请求的基础单价，不能表达长上下文分段计费。GPT-5.6 输入超过 272K token 后的输入/输出倍率不应通过伪造第二套模型 ID 表达；需要精确支持时应扩展计费规则结构。OpenAI 官方单独公布的 cache write 单价仍写入 `cache_creation_cost_per_million`，实际是否产生该费用取决于 usage 是否提供 cache creation token。
- OpenCode 的 `@ai-sdk/anthropic` 预设必须按模型代际维护 reasoning variants，不能把所有 Claude 模型套成同一结构：Sonnet/Opus 4.6 及之后使用同级的 `thinking: { type: "adaptive" }` 与 `effort`；Opus 4.7+、Sonnet 5、Fable 5 还要提供 `xhigh`，并设置 `display: "summarized"` 避免默认省略思考文本；Opus 4.5 使用 `effort` 但不启用 adaptive；Sonnet 4.5、Haiku 4.5、Opus 4.1、Sonnet 4 和 Sonnet 3.7 继续使用 `thinking: { type: "enabled", budgetTokens }`。`effort` 是 `thinking` 的同级字段，不能放进 `thinking` 对象。
- 更新 Claude 预设时，同时核对 models.dev 的 `reasoning_options`、context/output limit 与当前 OpenCode provider variant 生成逻辑；只看 Anthropic API 支持范围不够，因为 OpenCode 会按 provider SDK 和具体模型代际生成不同参数形状。
- 不要在这里记录“远端缓存刷新后也许会覆盖本地顺序”之类推测；判断最终线上效果时，要先区分当前看到的是 bundled defaults 还是 app data / 远端缓存数据。
- 新增 Gateway provider compat 后要同步更新 `gateway_provider_profiles.json`，否则用户只能走自定义渠道，runtime 也不应靠模型名 fallback 补偿这个缺口。模型名可以作为已识别 provider 内部的能力细分条件，但不能作为 provider 身份识别条件。
- `tools.gemini` 可以从 `tools.codex` 的同 target endpoint 派生基础 URL/API 格式/providerType/model 信息，但不能复制 Codex-only `codexChatReasoning`；Gemini CLI provider 只保存 `gatewayProfile` 引用和用户覆盖项，runtime 从 `tools.gemini` 动态解析自身会用到的 effective meta。

## 跨模块依赖

- `tauri/src/coding/preset_models.rs` 依赖 `preset_models.json` 作为编译期默认数据，并向前端暴露加载缓存与远端刷新命令。
- `tauri/src/coding/open_code/free_models.rs` 依赖 `models.dev.json` 作为 OpenCode 默认模型数据。
- `tauri/src/db/model_pricing_seed.rs` 依赖 `model_pricing.json` 作为 Gateway 官方模型定价默认数据；运行时远端同步只增量插入缺失行，不覆盖用户已有价格。
- `web/app/providers.tsx` 在启动时先加载 preset models / Gateway provider profiles 本地缓存，再异步拉远端并更新前端内存态。
- 备份恢复模块会单独备份和恢复 app data 下的 `preset_models.json`、`models.dev.json`、`model_pricing.json` 与 `gateway_provider_profiles.json` 缓存文件。

## 典型变更场景（按需）

- 改 `preset_models.json` 时，至少检查：
  - 是否仍是合法 JSON。
  - 模型放在了正确分组。
  - 相对顺序是否符合用户可见展示语义。
- 改 `models.dev.json` 时，至少检查：
  - 变更是否真的是 OpenCode 默认模型数据，而不是应该改运行时缓存或远端源。
  - `tauri/src/coding/open_code/free_models.rs` 的默认读取路径和筛选语义是否仍成立。
- 改 `model_pricing.json` 时，至少检查：
  - 是否仍是合法 JSON 数组。
  - 每个对象字段是否与 SQLite `model_pricing` 表一致，成本字段是否仍是非负数字字符串。
  - 新增默认价格不会覆盖用户已有行；修正已存在模型的官方价格不会自动改写老用户数据库。

## 最小验证

- 修改任一 JSON 后，至少做一次 JSON 合法性校验。
- 修改 `preset_models.json` 后，至少复核消费端是否仍按数组顺序直出，没有额外排序。
- 修改 GPT-5.6 预设或共享思考等级时，至少确认三个 canonical ID 的顺序与 `none/low/medium/high/xhigh/max` variants，并确认 Pi 前后端仍能保留 `max`、拒绝把 `ultra` 当成普通 thinking level。
- 修改 Grok 4.5 预设时，至少确认它位于 `@ai-sdk/xai`，只展示 canonical ID，保留 500K context 与 `low/medium/high` 三档 reasoning，并且不凭推测填写 `outputLimit`。
- 修改 `@ai-sdk/anthropic` Claude 预设后，至少运行 `cargo test coding::preset_models::tests`，确认 adaptive、effort-only 与固定 budget 三类模型没有互相串用参数。
- 修改 `model_pricing.json` 后，至少跑一次 `cargo test model_pricing_seed` 或等价测试，确认 bundled JSON 可解析且 seed 仍是 `INSERT OR IGNORE` 语义。
- 如果本轮同时改了缓存/远端刷新链路，还要额外区分 bundled defaults、app data cache 和 remote fetch 三条路径分别验证。

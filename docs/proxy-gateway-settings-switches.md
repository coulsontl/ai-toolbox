# Proxy Gateway 设置开关说明

本文梳理本机代理网关设置页和 `ProxyGatewaySettings` 中各项开关/参数的作用。字段名以 Rust/前端 DTO 中的 snake_case 为准，页面文案以中文 UI 为准。

## 总体边界

- 网关设置的事实源是 SQLite JSONB 中的 `proxy_gateway_settings`，前端设置页只读写后端命令返回的 DTO。
- 协议转换模块 `transformer` 不读取设置、不依赖数据库或 Tauri context。开关控制发生在 Gateway runtime 层：runtime 先根据入站 CLI route 和 provider `apiFormat` 判断目标协议，再决定转换前或转换后应用哪些整流能力。
- 协议相同的请求走直通链路，不进入结构转换器；但仍可以执行模型名改写、`[1M]` 标记剥离等 runtime 级处理。Thinking 整流属于上游 4xx 后的反应式重试，不是正常发送前的结构改写。

## 监听

| UI | 字段 | 默认值 | 作用 |
|---|---|---:|---|
| 监听地址 | `listen_host` | `127.0.0.1` | 网关 HTTP listener 绑定地址。当前 MVP 只允许 `127.0.0.1` / `localhost`。 |
| 端口 | `listen_port` | `37123` | 网关 HTTP listener 绑定端口，要求不低于 1024。 |
| 端口冲突自动选择 | `port_auto_select` | `false` | 启动时如果配置端口不可用，自动选择可用端口。关闭时端口冲突会启动失败。 |
| WSL 访问地址 | `wsl_host` | 空字符串 | WSL Direct CLI 访问 Windows 本机网关时，用此 host 替换 `127.0.0.1`。仅在 runtime location 为 WSL Direct 且非空时影响 CLI 接管写入和 drift 检测。 |

## 请求整流

| UI | 字段 | 默认值 | 作用 |
|---|---|---:|---|
| Thinking 整流 | `thinking_rectifier_enabled` | `true` | 只对 Claude/Anthropic 入站请求生效。开启后，正常请求不会预先删除 `thinking` / `output_config.effort`；仅当非流式上游 HTTP 4xx 错误命中 thinking/signature 兼容问题时，runtime 才删除顶层 `thinking` 参数、`messages[].content[]` 中的 `thinking` / `redacted_thinking` 内容块，以及内容块直接携带的 `signature` 字段，并在同一渠道重试一次。不会递归扫描 metadata、tool input 或其他业务 payload。 |
| Thinking budget 修正 | `thinking_budget_rectifier_enabled` | `true` | 目标协议为 Anthropic Messages、非流式响应、上游返回 HTTP 4xx 且错误内容命中 budget_tokens 问题时，自动调整请求体里的 thinking budget 后，在同一渠道重试一次。现在按 `provider.target_protocol == AnthropicMessages` 判断，不再只等同于 Claude CLI 入站。 |
| Cache 注入 | `cache_injection_enabled` | `false` | 对最终发往 Anthropic Messages 上游的请求体注入 `cache_control`，用于降低重复上下文成本。runtime 会先完成模型改写和必要的协议转换，再对目标 Anthropic body 注入；因此 Codex/Responses 转 Anthropic 的请求也受此开关控制。 |

### Thinking 整流与协议转换的关系

`Thinking 整流` 保留“遇到 thinking/signature 兼容错误后清理私有 thinking 历史并重试”的语义，默认开启。开启后：

- Claude 直通到 Anthropic 或转 OpenAI Chat / OpenAI Responses / Gemini Native 时，首次请求都不会因为模型映射或协议转换而预先删除 thinking。
- Anthropic `output_config.effort` 继续由协议转换正常映射到 OpenAI Chat `reasoning_effort` 或 Responses `reasoning.effort`。
- 上游返回非流式 4xx 且错误内容命中 thinking/signature 兼容问题时，才对同一请求体执行 thinking/signature 清理并同渠道重试一次。
- 非 Claude 入站请求不触发该整流；OpenAI/Gemini/Responses 协议内部的 reasoning 映射仍由 `transformer` 的正常转换语义处理。

这样可以避免正常跨协议 reasoning effort 被半截删除，同时仍能在上游明确拒绝 thinking/signature 历史时自动修复一次。

## 超时

| UI | 字段 | 默认值 | 作用 |
|---|---|---:|---|
| 首包超时秒 | `streaming_first_byte_timeout_secs` | `60` | 流式请求上游返回成功状态后，等待第一个非空 chunk 的最长时间。首包前失败或超时可按 failover/retry 策略继续尝试。 |
| 流式空闲秒 | `streaming_idle_timeout_secs` | `120` | 流式响应过程中两次 chunk 之间允许的最长空闲时间，避免半开连接无限挂起。 |
| 非流式超时秒 | `non_streaming_timeout_secs` | `600` | 非流式请求等待完整上游响应的最长时间。 |

## 重试

| UI | 字段 | 默认值 | 作用 |
|---|---|---:|---|
| 单渠道重试次数 | `per_provider_retry_count` | `0` | 当前 provider 失败后，最多在同一 provider 上额外重试多少次。耗尽后，failover 模式会切到下一个 provider；single 模式直接返回错误。 |
| 最大重试次数 | `max_retry_count` | `8` | 单次请求跨所有 provider 累计允许的额外重试总上限，用来限制整体耗时。 |
| 单渠道重试间隔秒 | `retry_interval_secs` | `1` | 同一 provider 内两次重试之间的等待时间。设为 `0` 表示立即重试。跨 provider 故障转移不等待该间隔。 |

## 模型健康

这些字段只在故障转移模式下影响 provider/model 路由。single 模式只有一个候选 provider 时，即使模型处于 cooling/down，也仍会尝试转发，避免单渠道被冷却后完全不可用。

| UI | 字段 | 默认值 | 作用 |
|---|---|---:|---|
| 失败分阈值 | `model_failure_score_threshold` | `5` | 同一 provider/model 在失败窗口内累计到该分数后进入冷却。不同失败类型会有不同计分。 |
| 失败窗口秒 | `model_failure_window_seconds` | `300` | 只统计最近这段时间内的失败，窗口外的失败逐步失效。 |
| 基础冷却秒 | `model_base_cooldown_seconds` | `120` | 模型首次熔断后的最短冷却时间，冷却结束后进入探测。 |
| 最大冷却秒 | `model_max_cooldown_seconds` | `1800` | 连续熔断时冷却时间的上限，避免退避无限增长。 |
| 探测成功次数 | `half_open_success_required` | `2` | 冷却结束进入 half-open/probing 后，需要连续成功多少次才恢复 healthy。 |

## 每 CLI 覆盖

字段：`app_configs`

`app_configs` 是按 CLI key 保存的覆盖配置，支持 `claude`、`codex`、`gemini` 等 key。设置页当前展示超时和重试覆盖项；定价管理弹窗还会复用同一结构保存每 CLI 默认计费配置。

| 子字段 | 作用 |
|---|---|
| `streaming_first_byte_timeout_secs` | 覆盖该 CLI 的首包超时；空值继承全局 `streaming_first_byte_timeout_secs`。 |
| `streaming_idle_timeout_secs` | 覆盖该 CLI 的流式空闲超时；空值继承全局 `streaming_idle_timeout_secs`。 |
| `non_streaming_timeout_secs` | 覆盖该 CLI 的非流式超时；空值继承全局 `non_streaming_timeout_secs`。 |
| `per_provider_retry_count` | 覆盖该 CLI 的单 provider 重试次数；空值继承全局 `per_provider_retry_count`。 |
| `max_retry_count` | 覆盖该 CLI 的全局最大重试次数；空值继承全局 `max_retry_count`。 |
| `retry_interval_secs` | 覆盖该 CLI 的同 provider 重试间隔；空值继承全局 `retry_interval_secs`。 |
| `cost_multiplier` | provider 未显式设置计费倍率时使用的该 CLI 默认倍率。 |
| `pricing_model_source` | provider 未显式设置定价模型来源时使用的该 CLI 默认值，支持 `upstream` / `requested`。 |

## 日志与统计

| UI | 字段 | 默认值 | 作用 |
|---|---|---:|---|
| 请求记录 | `request_log_enabled` | `true` | 开启 JSONL 请求详情文件。只有开启时才会写 headers/body/response/attempt 等详情文件。 |
| 统计事件 | `metrics_enabled` | `true` | 开启 SQLite compact 请求摘要，用于统计页、请求列表和 usage 聚合。 |
| 保存 Headers | `store_headers` | `false` | 请求记录开启时，控制是否在 JSONL 详情里保存 headers。关闭请求记录时该选项在 UI 中禁用且不会落详情。 |
| 保存请求体 | `store_request_body` | `false` | 请求记录开启时，控制是否保存网关收到的原始请求体和实际发往上游的请求体快照。 |
| 保存响应 | `store_response_body` | `false` | 请求记录开启时，控制是否保存响应 body 快照。 |
| 保留天数 | `log_retention_days` | `7` | JSONL 详情文件保留天数。 |
| 日志目录上限 MB | `log_max_dir_size_mb` | `512` | JSONL 详情目录体积上限，超过后按清理策略裁剪。 |
| 单体明细上限 KB | `log_max_body_size_kb` | `256` | 单个 body/header/response 明细保存上限，避免敏感或超大内容无限落盘。 |

补充语义：

- 只要 `request_log_enabled` 或 `metrics_enabled` 任一开启，runtime 都会写 SQLite compact 请求摘要；两者都关闭时，请求 Tab 和统计页不会记录当前请求。
- 只有 `request_log_enabled=true` 时才写 JSONL 详情文件。`metrics_enabled=true` 且 `request_log_enabled=false` 时，请求详情只能从 SQLite 摘要降级展示 provider/model/token/status/latency 等基础字段。
- `store_request_body` 同时控制 `request_body` 和 `upstream_request_body`。如果请求体发生整流或协议转换，详情页可用这两个快照对比“收到的原始请求体”和“实际发出的请求体”。

## 隐藏与兼容字段

| 字段 | 默认值 | 作用 |
|---|---:|---|
| `enabled_on_startup` | `false` | 上次应用退出前的网关运行态，不是用户可见开关。启动成功后置 `true`，用户手动停止成功前置 `false`，应用启动时按它自动恢复网关。运行中保存设置时必须保留该标记。 |
| `enabled_cli_keys` | `["claude","codex","gemini"]` | 旧设置兼容字段，不表示当前已接管 CLI。真实接管状态以 `proxy-gateway/cli-proxy/<cli>/manifest.json` 为准。 |
| `request_log_level` | `summary` | UI 由 `request_log_enabled`、`store_request_body`、`store_headers`、`store_response_body` 派生展示/保存的兼容字段。请求详情是否保存仍以具体布尔开关为准。 |

## 当前一致性结论

- 直通链路和协议转换链路现在都由同一组 Gateway runtime 开关控制，不要求 `transformer` 模块感知设置。
- `Thinking 整流` 的默认值是开启；开启后，Claude 入站请求只在上游非流式 4xx 明确命中 thinking/signature 兼容问题时删除 thinking/signature 并重试一次，正常协议转换会保留 `thinking` / `output_config.effort` 的语义映射。
- `Cache 注入` 与 `Thinking budget 修正` 按最终目标协议判断。只要目标是 Anthropic Messages，就可以覆盖 Codex/Responses -> Anthropic、Gemini -> Anthropic 等转换路由；目标不是 Anthropic Messages 时不应用。
- Codex 新增/编辑自定义渠道时，如果 API 格式不是默认 `openai_responses`，前端会自动展开模型映射区域；切回 `openai_responses` 不会自动收起，避免覆盖用户手动展开状态。

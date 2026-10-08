# Gateway Provider 兼容细节

本文记录 Proxy Gateway 逐 provider/channel 的 wire 兼容事实，覆盖当前源码可证实的入参兼容、出参兼容、触发条件、默认行为、开关和回归测试。

本文不是架构主文档。协议转换架构、统一 IR、SSE 生命周期、runtime/transformer/pipeline/side store 边界、参考项目同步流程和 baseline commit 仍以 [`docs/gateway-protocol-conversion.md`](gateway-protocol-conversion.md) 为准。后续同步 `../cc-switch` 或 `../axonhub` 时，先按架构主文档读取 baseline 和参考项目查询入口；如果吸收结果改变 provider/channel 兼容事实，再同步更新本文。

本文只写当前实现能证明的行为。参考项目已有但 AI Toolbox 还没有实现的能力，不能在本文写成“已支持”。

## 1. 事实源与触发模型

### 1.1 方向术语

- 请求侧：客户端 CLI -> Gateway -> 上游 provider。
- 响应侧：上游 provider -> Gateway -> 客户端 CLI。
- source protocol：Gateway 从入站 route 推导出的客户端协议。
- target protocol：Gateway 从 provider effective meta/settings 推导出的上游协议。
- 同协议直通：source protocol 与 target protocol 相同，不创建 `ConversionRoute`，不调用结构转换器；runtime 仍会执行 URL/header/auth、模型名、provider body、stream filter、rectifier 等兼容。
- 跨协议转换：source protocol 与 target protocol 不同，先由 `transformer` 做公共协议结构转换，再由 runtime 对最终上游 body 做 provider 兼容。

### 1.2 生产触发不是 `compat` 字段

`tauri/resources/gateway_provider_profiles.json` 里的 `compat` 字段是 catalog 描述和 schema 校验材料。`tauri/src/coding/proxy_gateway/provider_profiles.rs::SUPPORTED_COMPAT_RULES` 使用 `CompatRuleRegistration` 为每个 tag 静态登记 `runtime_owner + test_name`：catalog 校验要求登记证据完整，测试会检查 tag 唯一且 owner/test 符号实际存在。生产请求仍不会因为某个 profile 声明了 `compat` 字符串就直接执行兼容逻辑，runtime 也不能把这些 tag 当作动态规则引擎。

真正触发来自 runtime 解析后的 effective meta：

- `providerType` / `provider_type`
- `apiFormat` / `api_format`
- `apiKeyField` / `api_key_field`
- `reasoningField` / `reasoning_field`
- `defaultMaxTokens` / `default_max_tokens`
- `codexChatReasoning` / `codex_chat_reasoning`
- `imageInputPolicy` / `image_input_policy`
- `textOnlyModels` / `text_only_models`
- `imageCapableModels` / `image_capable_models`
- `allowTextOnlyModelHeuristic` / `allow_text_only_model_heuristic`
- provider record `category` legacy fallback

源码入口：`tauri/src/coding/proxy_gateway/runtime/providers.rs::provider_meta_from_record()`。

### 1.3 `gatewayProfile` 动态解析

内置渠道保存的是 `data.meta.gatewayProfile={tool,profileId,endpointId}` 引用。runtime 每次读取 provider 时从当前 `gateway_provider_profiles.json` 动态解析 profile/endpoint：

- `providerType` 来自 profile。
- `apiFormat` 来自 endpoint，决定 `UpstreamProvider.target_protocol`。
- `apiKeyField`、`reasoningField`、`defaultMaxTokens`、图片策略字段优先 endpoint，再 fallback profile。
- profile 中的 `codexChatReasoning` 只在 `gatewayProfile.tool == "codex"` 时解析；Claude/Grok/Gemini 不从 profile 解析这个 Codex-only 字段，但后续 fallback inference 仍可由明确 effective `providerType/apiFormat` 触发。
- `gatewayProfile.tool` 必须匹配当前 CLI；不匹配时忽略引用，继续使用 legacy meta。
- profile/endpoint 缺失或解析失败时保留 legacy meta；如果最终 `providerType` 仍为空，fallback 到 provider record 的 `category`。
- `profileId`、`tool`、`endpointId` 是 reference-only provider 的持久化稳定 ID。远端 catalog 激活前必须保留上一份有效 catalog 中已有的 profile、受支持 tool 和 endpoint ID；允许新增 ID 和修改非 ID 元数据，删除/rename 会拒绝激活并保留上一份有效 catalog。不兼容 cache 回退 bundled defaults；breaking rename 必须先提供 alias 或 migration。

源码入口：`runtime/providers.rs::apply_gateway_profile_reference()`、`provider_profiles.rs::validate_gateway_provider_profile_compatibility()`。

### 1.4 target protocol 推导

供应商分享导入保留实际上游协议和 API-key/Bearer 认证语义；有匹配目标 endpoint 时保存 `gatewayProfile` 引用，不复制 profile 派生兼容快照。没有匹配 endpoint 时，只有通用协议可表达的连接才能降为 custom；依赖特殊 adapter 的 native 目标在预览阶段拒绝。OpenCode 与原生 Anthropic/Google SDK 的版本路径适配归分享配置层，不改变 runtime URL/IR/SSE 职责。入口及回归见 `coding/deeplink/importer.rs`、`tauri/tests/coding/deeplink/provider_transfer.rs` 与 [`deep-link-import.md`](deep-link-import.md)。

- Claude Code / Claude Desktop：effective `apiFormat` -> settings `api_format/apiFormat` -> `openrouter_compat_mode=true` -> 默认 `AnthropicMessages`。
- Codex：effective `apiFormat` -> settings `api_format/apiFormat` -> `config.toml` 的 `wire_api/api_format` -> base URL 是否 `/chat/completions` -> 默认 `OpenAiResponses`。
- Grok：effective `apiFormat` -> selected model `api_backend` -> 默认 `OpenAiChat`。
- Kimi：effective meta `apiFormat` -> 默认 `OpenAiChat`；不从接管后的客户端协议推断真实上游。
- Gemini：effective `apiFormat` -> settings `api_format/apiFormat` -> 默认 `GeminiNative`。
- Copilot：请求级动态特例，模型名 `gpt-<major>` 且 major >= 5 但不是 `gpt-5-mini` 时，本次请求切到 `OpenAiResponses`；其它走 `OpenAiChat`。这只改变本次 effective provider，不改 provider 记录。

源码入口：`runtime/providers.rs`、`runtime/upstream.rs::effective_upstream_provider_for_request()`。

### 1.5 source protocol 和 conversion route

`runtime/upstream.rs::source_protocol_from_route()` 当前规则：

| CLI/route | 条件 | source protocol |
|---|---|---|
| Claude Code / Claude Desktop | `/v1/messages` 或 `/messages` | `AnthropicMessages` |
| Codex | `/v1/chat/completions` | `OpenAiChat` |
| Codex | `/v1/responses`、`/v1/responses/compact` | `OpenAiResponses` |
| Grok | `/v1/responses` | `OpenAiResponses` |
| Kimi | `/v1/chat/completions` | `OpenAiChat` |
| Gemini | path 包含 `:generateContent` 或 `:streamGenerateContent` | `GeminiNative` |

这里列出经过 `runtime/routes.rs` 实际可达的 forwarded path；source helper 中不带 `/v1` 的内部别名不扩大公开入口，例如 `/openai/responses` 当前不可达。`runtime/upstream.rs::conversion_route()` 只在 `source_protocol != provider.target_protocol` 时创建 `ConversionRoute`。同协议路径不进入 transformer。

Grok 还有 `/grok/v1` 的本地探测路由；正式模型请求当前只接受 `/grok/v1/responses`。`/grok/v1/chat/completions` 和 `/grok/v1/responses/compact` 会在 `runtime/routes.rs::match_gateway_route()` 被拒绝，不能按 Codex 的 source path 推导规则理解成 Grok 可达接口。

Claude Desktop 使用独立 `/claude-desktop` 前缀和自己的 provider 表，共用 Anthropic 入站语义。Kimi 的正式入口为 `/kimi/v1/chat/completions`；六类受支持 CLI 与所有有效模型入口的隐私覆盖见 §7.2。旧版 `/openai/v1/completions` 继续走 runtime 专用兼容，其 JSON/SSE 有效内容包含 `choices[].text`。

### 1.6 Responses streamed compaction 边界

- Responses -> Responses：同协议 identity route 不创建 `ConversionRoute`，原始 SSE 字节直接透传，因此 `compaction` / `compaction_summary` 保真，但不经过 `StreamKernel`。
- Responses -> OpenAI Chat / Anthropic Messages / Gemini Native：目标协议没有 Responses compaction 原生表示，普通 stream kernel 忽略该 item；同一流中的 text、tool、usage 和 terminal 事件仍需继续转换。
- OpenAI Chat / Anthropic / Gemini -> Responses：source 协议没有 compaction 语义，不能凭空恢复。
- `/responses/compact` 是 runtime 专项 facade；非流 JSON 中显式可达的 compaction helper 继续保留。两者都不能用来宣称普通 SSE kernel 支持跨协议 compaction roundtrip。

测试：`responses_identity_stream_preserves_compaction_bytes_without_kernel`、`responses_stream_drops_compaction_for_chat_without_losing_text`。

#### 1.6.1 Codex remote compaction v2

v2 与 legacy `/responses/compact` 是两代不同协议，识别边界刻意互斥（`runtime/compat/codex_remote_compaction.rs`）：v2 认普通 `/responses`（WS 为普通 `response.create` 帧）里 `input[]` 的 `compaction_trigger`，legacy 端点仍归 `codex_responses_compact.rs`。

| 环节 | 行为 |
|---|---|
| 触发信号 | `input[]` 含 `{"type":"compaction_trigger"}`。**不是** `x-codex-turn-metadata.request_kind == "compaction"`——本地 checkpoint 压缩也带该元数据，其响应必须保持普通 assistant 摘要 |
| 摘要请求 | 剥离 trigger 与 `tools`/`tool_choice`/`parallel_tool_calls`/`response_format`/`stop`/`previous_response_id`，替换 `instructions` 为专用摘要指令，末尾追加产出指令，HTTP 侧再置 `stream:false` |
| 摘要抽取 | HTTP 按结构识别四种目标协议（Responses / Chat / Anthropic / Gemini），不依赖声明的 target protocol；WS 从上游 delta 事件重组 |
| 失败处理 | 摘要为空、`finish_reason == length`、上游 error 或封装失败时一律失败该轮，**不伪造 item**：Codex 把返回的 item 当作整段对话历史安装，占位文本等于静默丢弃上下文。HTTP 侧走既有错误路径（400 `gateway_request_schema_rejected`）；WS 侧转发上游自己的终态，让 Codex 按 error code 分类重试。该规则同样覆盖「上游**成功**但摘要不可用」——`response.completed` 不带文本、`response.incomplete` 截断——此时转发上游终态会让 Codex 报自己的 `expected exactly one compaction output item`，这是刻意的：比塞一个伪造上下文好 |
| HTTP 响应形态 | 合成好的 item 跟随客户端声明成形：流式客户端拿到 5 事件 SSE，非流式客户端拿到同一 response 的 JSON。Codex 恒为流式，非流分支实际不可达，但重渲染必须按 `request_declares_streaming` 门控，否则非流式调用方收到 `text/event-stream` 外壳却解析不出 body |
| identity 路由 | v2 走 Responses 同协议直通时**同样被改写**。`build_upstream_body_for_provider` 对无 `conversion_route` 的请求本就会解析再重发 JSON，所以改写就地做，不需要为它造转换路由。这是最易回归的一点：改动 body 构造就可能让 `compaction_trigger` 原样透传给第三方上游，且模块单测看不出来 |
| `encrypted_content` | AES-256-GCM（自识别前缀 `ai-toolbox-compact-v1:`），每次封装新 nonce。解开失败按“非本 Gateway 生成”原样保留，绝不报错。**不能**退回 base64：该字段随 Codex 历史落 rollout 文件与日志 |
| 后续轮次 | 回传的本地 capsule 解回带标题的可读上下文；其它工具/官方写的 compaction item 原样保留 |
| WS 传输 | 只覆盖 Responses 直通（需协议转换的 provider 握手前已 `426`）。帧是**裸 JSON**（无 `event:`/`data:` 外壳）。终态前**不转发任何上游帧**（摘要未封装完成，且透传 delta 会把摘要明文送上线路）；终态后合成 5 帧，上游自己的 `response.completed` 丢弃。仅当摘要不可用时改为转发上游终态 |

Codex 侧门控：只有 `name == "OpenAI"`（或 Azure）的 provider 默认启用 v2（`codex-rs/model-provider/src/capabilities.rs`），纯字符串比较，与 `base_url` 无关；`[features] remote_compaction_v2 = false` 是 no-op。接管时写 `capabilities.remote_compaction = "v2"` 可为第三方 provider 打开。

测试：纯函数层 `the_legacy_compact_endpoint_is_not_claimed`、`local_checkpoint_metadata_is_not_remote_compaction`、`the_summary_comes_out_of_every_target_protocol`、`the_compaction_stream_has_exactly_one_done_item`、`the_websocket_frames_are_bare_json_with_one_compaction_item`、`usage_survives_from_every_upstream_shape`；WS relay 层 `websocket_compaction_turn_is_summarized_and_replaced`、`websocket_compaction_turn_keeps_an_incremental_delta`、`websocket_compaction_failure_is_forwarded_not_synthesized`、`websocket_empty_summary_forwards_the_upstream_terminal`、`websocket_truncated_summary_forwards_the_upstream_terminal`；HTTP 挂载层（真网关 + stub 上游）`tauri/tests/coding/proxy_gateway/codex_remote_compaction_http.rs` 的 `identity_responses_compaction_is_rewritten_and_sealed`、`a_non_streaming_client_still_gets_one_compaction_item`、`a_replayed_capsule_becomes_readable_context_upstream`、`an_empty_summary_fails_instead_of_faking_an_item`。挂载层必须有集成测试：纯函数全对但接线错（identity 路由没改写、非流客户端被塞 SSE、回放没展开 capsule）单测看不出来。

### 1.7 跨协议终态 envelope 索引

详细状态机见架构主文档 §10 / §18。本表只索引 source terminal 到四个 target 的默认 wire，便于 provider/channel 审查检索：

| Source terminal | Anthropic target | Chat target | Responses target | Gemini target | Runtime health / failover |
|---|---|---|---|---|---|
| Responses `failed` / `status=failed` | `event:error` / JSON `{type:"error",error:{...}}`；保留 message/type/code；**禁止** `message_stop` / `end_turn` | SSE/JSON `{error:{message,type,code}}`；**禁止** `finish_reason=error` + `[DONE]` | `response.failed` | Gemini error envelope；**禁止** `finishReason=STOP` | 视为失败，可 retry/failover |
| Responses `incomplete` / `length` | 正常 finish（`max_tokens` 等） | `finish_reason=length` | **有意 bridge**：`response.completed` + `status=incomplete`（不是官方 `response.incomplete` 事件名）；`incomplete_details` 仅可选增强 | `finishReason=MAX_TOKENS` | 合法终态，不扣 health |
| Responses `cancelled` / `canceled` | 当前跨协议仍可能 best-effort 正常 finish（无 1:1 cancellation terminal）；**runtime 侧保持 health-neutral，不 retry/failover** | 非官方 `finish_reason=cancelled` bridge 仍存在 | `response.cancelled` + `status=canceled` | 可能 `STOP` best-effort | 合法终态，不扣 health |

测试锚点：`responses_failed_stream_to_anthropic_emits_error_not_message_stop`、`responses_failed_stream_to_chat_emits_error_envelope`、`responses_failed_stream_to_gemini_emits_error_not_stop`、`responses_failed_json_to_*`。

## 2. 通用请求侧兼容

请求 body 的事实源是 `runtime/upstream.rs::build_upstream_body_for_provider()` 和 `runtime/middleware.rs`。当前顺序：

1. 解析入站 JSON。
2. 构造 request-scoped pipeline：`OutboundAdapterCompatMiddleware`、`BillingHeaderCchMiddleware`、必要时 `EnsureMaxTokensMiddleware`。
3. 运行 inbound middleware。`BillingHeaderCchMiddleware` 会从 Claude Code system 文本开头剥离动态 `x-anthropic-billing-header:` / `cch=...` 并保存到 `PipelineContext.billing_cch`。
4. 如果是 Codex Responses 转 Chat/Anthropic，转换前用 `CodexHistoryStore` 补回上一轮缺失的 call item。
5. 只在存在 conversion route 时执行有损检测；默认放过并写 `X-Transformer-Lossy`，用户开启 `lossy_rejection_enabled` 后才拒绝，且 `X-Allow-Lossy: true` 可绕过。
6. 写入或改写最终上游 `model`：先查 provider 精确模型改写规则（2.6），未命中再走各 CLI family/default 映射，最终剥离 `[1M]` / `[1m]`。
7. Gemini source 转非 Gemini target 且 route streaming 时写 `stream=true`。
8. thinking rectifier 重试路径才执行 `strip_thinking_blocks`；该 rectifier 按**入站 CLI=Claude** 门控，不是按 target protocol 判断；正常请求不会预先删除 thinking。
9. `/responses/compact` 走 compact 专项 compat；Codex remote compaction v2 的 `compaction_trigger` 轮次在此前已改写为普通摘要请求（见 §1.6.1），不走 compact facade；普通跨协议请求调用 `convert_request_body_with_context()`；同协议请求直通当前 body。
10. target Gemini 时可由 `GeminiShadowStore` 回放上一轮带 `thoughtSignature` 的 model functionCall。
11. target OpenAI Responses 时执行 `prompt_cache_key` fallback。
12. target OpenAI Chat 时先缓存被 strip 前的 `prompt_cache_key`，再跑 provider pipeline。
13. target OpenAI Chat 后置 `prompt_cache_key` allowlist reinject。
14. xAI native Responses passthrough gate 命中时执行 namespace flatten 和 sanitize。
15. target Anthropic 且 `cache_injection_enabled=true` 时注入 cache_control（`runtime/cache_injector.rs`）。注入必须先做 `normalize_message_contents` + `sanitize_unsupported_cache_controls`，再按 Anthropic 的 4 断点上限裁剪（先丢最早的 message 断点，再丢最早的 tools/system 断点）；结构断点只在缺失时补，且按剩余预算补（tools 优先于 system，总数绝不越过 4 个上限），message anchor 只在客户端一个断点都没给时才规划。客户端已自规划的断点位置跨轮稳定，重新规划会让前缀 hash 每轮变化、每次请求都付满额 cache write（AxonHub `7444f537`）。回归：`preserves_client_structural_breakpoints_and_fills_message_anchors`、`keeps_client_message_breakpoints_without_replanning`、`trims_excess_client_breakpoints_deterministically`、`keeps_structural_anchors_within_breakpoint_limit`、`keeps_string_system_untouched_when_budget_is_spent`。

### 2.1 outbound adapter 顺序

`runtime/upstream.rs::apply_outbound_adapter_compat_value()` 当前顺序：

1. `filter_private_outbound_fields()` 递归移除 `_` 开头内部字段，但在 JSON Schema `properties`、`patternProperties`、`definitions`、`$defs` 下保留属性名。
2. 用 `ProviderBodyCompat::from_provider_meta()` 识别 provider 方言。
3. 用 `ReasoningFieldPolicy::from_provider_meta()` 计算 OpenAI Chat assistant reasoning 字段策略。
4. 读取 explicit/legacy/inferred `codexChatReasoning`。
5. provider body compat before generic。
6. target OpenAI Chat 时执行 Codex Chat reasoning 配置。
7. target OpenAI Chat 时执行通用第三方 Chat 兼容清理。
8. 有 conversion route 且非 Gemini source 时，无 tools 清理 `tool_choice/parallel_tool_calls` 或 Anthropic `tool_choice`；Anthropic target 且 tool_choice 强制 tool_use 时移除顶层 `thinking`。
9. provider body compat after generic。
10. target OpenAI Chat 时执行 reasoning field policy，再执行 DeepSeek final reasoning gate。
11. 执行预测式图片/多模态兼容策略。
12. Ollama target 最后投影到 Ollama `/api/chat` wire format。

### 2.2 OpenAI Chat 通用兼容

`normalize_openai_chat_for_provider_compat()` 是发往 OpenAI Chat-compatible provider 前的通用清理：

- 删除顶层 `verbosity`、`prompt_cache_key`。
- 非 DeepSeek 且没有显式或 inferred `codexChatReasoning` 要保留 effort 时删除 `reasoning_effort`。自定义（未识别 vendor）Chat provider 可由用户显式 `meta.preserveReasoningEffort=true` 保留顶层 effort（Codex 供应商表单的「透传推理强度」开关，默认开、保存后生效；issue #412）。显式 `false` 强制删除，覆盖 vendor 推断；provider 记录带 `gatewayProfile` 引用时该字段被清空，由 profile 的 `codexChatReasoning` 决定。
- 过滤 tools，只保留 `type=function` 且有 `function.name` 的工具，移除 `response_custom_tool`。
- `developer` role 改成 `system`。
- system content parts 压成 string。
- 多个 system 合并到首条；Anthropic Messages 来源（Claude Code）例外：只合并前导连续块，块之后的 system/developer 保持原 index、role 降级为 `user`（`InstructionPlacement`，见 `docs/gateway-protocol-conversion.md` §19.7 F-15）。
- tool call arguments 空值补 `"{}"`。
- 删除 Google 私有 `thought_signature/thoughtSignature`，以及 `google`、`extra_content/extra_fields` 中包含 signature 的容器。
- 删除不支持 tool call 及对应 tool result。
- 删除空 assistant message。

这些是 runtime provider compat，不属于 transformer roundtrip 语义。

跨协议 `tool_choice` 的通用保真规则属于 transformer，不在本层：OpenAI `allowed_tools`（Chat 嵌套 `allowed_tools.mode/tools`、Responses 顶层 `mode`/`tools`）按 `ToolChoice::AllowedTools` 保留 mode 与工具子集，Chat/Responses writer 各写回本协议形态，非 function selector（如 MCP `server_label`）原样转发；Anthropic/Gemini target 没有白名单形态，只保留 mode（AxonHub `d5237439`，细节与回归见 `docs/gateway-protocol-conversion.md` §19.4）。本层的 `remove_tool_controls_without_tools`（无 tools 时清空 `tool_choice`）与 §5.5 的 Z.ai `force_tool_choice_auto` 仍按各自既有触发条件优先，不被该保真规则改写。

Responses source 转 Chat 时有两条有意区分的 transformer 输入形态：custom-only 请求保留 `responses_custom_tool` Chat 兼容扩展，到本层再按第三方 Chat wire 能力过滤；请求包含 `tool_search`、namespace 或历史 `tool_search_output` 时，transformer 使用 request-scoped Codex context，提前把这些扩展和同请求 custom tool 投影成普通 Chat function，本层不会再把它们当成 `response_custom_tool` 删除。两条路径都必须保留现有回归，不能无条件构造完整 Codex context 把 custom-only roundtrip 静默降级成普通 function。

Responses source 转 Anthropic Messages / Gemini Native 时，namespace child 声明使用与 Chat 相同的 `flatten_namespace_tool_name()` 投影为普通 function，并用 namespace-only `ConversionContext` 保持同一次请求/响应的身份映射。历史 `function_call` 与具名 `tool_choice` 必须同步改写 flat name，namespace 类型 choice 降为 `auto`；Anthropic/Gemini 的 JSON 和 SSE 工具调用转回 Responses 时恢复原 `namespace` 与子工具名。最终 flat name 与顶层 function/custom 或其它 namespace child 重名时在本地 fail closed，不向上游发送重复工具声明。该行为是所有 Responses→Anthropic/Gemini 转换的通用 wire 兼容，不受 providerType 开关控制。

Responses source 的并行工具批次在 transformer 入站统一归并（issue #352，架构文档 §8.4），不依赖 `providerType`：连续 function/custom 调用与调用前、中、后的 assistant commentary/reasoning 输出同一条 assistant，随后逐条输出所有结果。非 assistant 消息、工具结果、raw item 与 compaction 边界不跨越；原始 call ID、参数和结果正文保留，不补造缺失结果。namespace/tool search 的 context 路径和 custom-only 的原有扩展路径共用该规则，custom-only 最终 wire 过滤仍沿用上面的既有边界。

这保证 Kimi/Moonshot、GLM/Z.ai 和普通 custom Chat 上游收到合法工具消息序列，包括上游 SSE 在工具调用之后才发出文本的情况；Bailian 的 provider-local 合并 helper 继续保留原触发条件，不代替公共 Responses 转换。`parallel_tool_calls=false` 不能删除或拆散已存在的历史并行批次。回归见 `tauri/tests/coding/proxy_gateway/parallel_tools.rs`、`parallel_tools_matrix.rs`、`parallel_tools_http.rs`：完整协议矩阵和真实 HTTP 后续请求核验最终 wire、完整/部分历史、JSON/SSE/强制聚合、历史缓存和正文日志关闭/截断。

Gemini 的通用工具语义见架构文档 §8.5：同批结果归为一个 user content；入站显式 ID 优先，无 ID 的同名结果按待完成调用逐个匹配；只为缺失调用 ID 生成唯一的本地身份。合成 ID 不回传 Gemini，移除前按调用顺序排列结果并保持媒体归属；普通 Gemini 原生 ID 及对应结果提交顺序保留。Gemini source SSE 和 forced SSE 聚合共用按原生 ID 合并快照的 helper，不能按函数名或 chunk 内位置覆盖并行调用。

### 2.3 prompt cache

- OpenAI Responses target：最终 body 没有 `prompt_cache_key` 时，从稳定 session 线索 fallback；显式值不覆盖。
- OpenAI Chat target：默认 strip `prompt_cache_key`；只有 allowlist providerType 才 reinject，当前包括 `openai`、`openai-chat`、`kimi`、`kimi-coding`、`moonshot`、`moonshot-v1`、`moonshot-coding`。优先使用 explicit pre-strip key，其次 session hint；没有线索不写默认值或随机值。

测试：`responses_prompt_cache_key_falls_back_to_session_header`、`responses_prompt_cache_key_keeps_explicit_request_value`、`chat_prompt_cache_key_strips_by_default_without_allowlist`、`chat_prompt_cache_key_reinjects_explicit_for_allowlisted_provider`、`chat_prompt_cache_key_reinjects_session_when_allowlisted_and_no_explicit`。

### 2.4 图片/多模态兼容

发送前预测式替换由 provider meta 或 model catalog 显式能力驱动：

- `imageInputPolicy=strip/replace/text_only/unsupported` -> 替换图片块为 `[Unsupported Image]`。
- `preserve/keep/vision/multimodal/image/images` -> 保留。
- `imageCapableModels` 优先保留。
- `textOnlyModels` -> 替换。
- model catalog 里的 `supportsImage=false`、`vision=false`、`attachment=false`、`modalities.input` 不含 `image` 等会触发替换。
- `allowTextOnlyModelHeuristic=true` 才启用模型名启发式；默认不猜。
- 启发式 exact tails 包含 `glm-5.1`、`glm-5.2`（以及 `GLM-5.2[1M]` / `vendor/GLM-5.2` 归一化后的 tail）；不能用 `glm-5.2` 前缀，避免误伤多模态 `glm-5.2v`。

上游错误后的反应式 rectifier：

- 只在 HTTP 400/415/422/501 时尝试同 provider 重试。
- 触发条件二选一：
  1. 错误文本明确 image/media/vision/attachment unsupported；
  2. 自证性 text-only 短语 `only support text` / `only supports text` / `text only` / `text-only`（无需提到 image；覆盖火山 `Model only support text input`）。
- 替换 OpenAI/Anthropic image/image_url、Responses `input_image`、Gemini image `inlineData/fileData` 为文本占位。
- 保留 `cache_control`。

测试：`unsupported_media_rectifier_*`、`predictive_media_policy_*`、`known_text_only_model_matches_glm_5_2_exact_tail_not_multimodal_variant`、`predictive_media_policy_replaces_images_for_glm_5_2_when_heuristic_enabled`。

### 2.5 middleware

- `BillingHeaderCchMiddleware`：request inbound 剥离 Claude Code 动态 billing CCH；target Anthropic 时在 outbound body 和 client-facing JSON/SSE reverse 阶段回填，非 Anthropic target 不泄漏。
- `EnsureMaxTokensMiddleware`：只有 provider meta 显式 `defaultMaxTokens > 0` 时加入。按 target 协议写或截断：
  - Anthropic：`max_tokens`
  - OpenAI Responses：`max_output_tokens`
  - Gemini：`generationConfig.maxOutputTokens`
  - OpenAI Chat：`max_completion_tokens` 优先，否则 `max_tokens`

测试：`provider_pipeline_caps_default_max_tokens_in_upstream_body`、`provider_pipeline_strips_billing_cch_for_non_anthropic_target`、`provider_pipeline_restores_billing_cch_for_anthropic_target`。

### 2.6 用户自定义精确模型改写（model rewrites，issue #321）

触发条件：

- provider `data.meta.modelRewrites`（snake/camel 双兼容）是非空 `ModelRewriteRule` 数组，每项 `{ from, to }`；读取侧过滤 `from`/`to` 空白的规则，空数组视为未配置。
- 不是指定 `provider_override_id` 的连通性测试（连通性测试保留用户点选模型）。
- 与 family/default 映射不同，该规则**在所有代理模式生效**：single 模式 CLI 透传模型命中规则时同样改写。
- 聚合模式（`mode = "aggregate"`）按模型名是否自带站点身份区分：命中**已发布 slug 表**（`AggregateSlugEntry.slug`）或解析出 `<site><sep><model>` / `<model><sep><site>` 前缀的请求已经指定了精确上游模型，**跳过本规则**；其余裸模型名（用户手填的模型 id）与其他模式一样命中本规则。判据是「是否命中 slug 表/前缀」而不是「有没有分隔符」——`model_only` 模式下已发布的 slug 本身就没有分隔符（如 `m#2`），它同样算 explicit 并跳过本规则，只有既不是 slug 也不是前缀的模型名（`AggregateRoute.explicit = false`）才走本规则。聚合模式始终不进入 per-CLI family/default/auto-review 映射。

匹配与改写语义：

- 精确匹配：请求模型先剥 `[1M]`/`[1m]` 上下文标记，再按 trim + 大小写不敏感与 `from` 相等比较；不支持通配/前缀/正则。
- 命中后上游模型为 `to` trim 并剥 `[1M]` 后的值，且不再进入各 CLI 的 family/default/auto-review 映射。
- 未命中进入既有 per-CLI 逻辑：Claude family（failover）、Codex default_model/auto_review（failover）、Grok default、ClaudeDesktop family、Kimi default（failover）、Gemini/OpenCode 透传。
- 只改写请求方向，不改回上游响应模型。请求摘要同时记录 `requested_model`（改写前）与 `upstream_model_id`（改写后），成本按改写后真实模型计价。
- Gemini 双路径覆盖：跨协议转 Gemini target 时 path 由 `gemini_native_forwarded_path(upstream_model_id)` 重建；Gemini CLI 入站直通 Gemini target 时 `gemini_forwarded_path_for_provider` 用 resolved 模型替换 `models/<model>` 段（Gemini 的模型在 URL path 而非 body），`models/` 无模型段的 path（模型列表等）原样保留。
- 已知边界：Copilot provider 的 warmup 请求降级（`effective_upstream_model_id_for_request`）发生在本规则之后，命中 warmup 检测时会把改写结果覆盖为 `warmup_model`；这是 Copilot 专项适配的既有层次，不由本规则改变。

各 CLI 优先级：exact 规则 > per-CLI family/default/auto-review 映射 > single 透传。用户配置的规则在 single 和 failover 下都生效——这是与“仅 failover 生效”门控的有意差异：issue #321 的场景是 Codex 单渠道代理下仍会请求被中转站禁用的内置小模型（对话标题模型），只有 all-mode 规则才能覆盖。

源码：

- `types.rs::ModelRewriteRule` / `ProviderGatewayMeta.model_rewrites`
- `runtime/providers.rs::model_rewrites_from_meta()` / `model_rewrite_rules_from_meta()`
- `runtime/upstream.rs::resolve_upstream_model_id()` / `resolve_model_rewrite_rule()`（聚合模式的裸模型名分支复用后者）

测试：

- `provider_meta_reads_model_rewrites_and_filters_blank_rules`
- `provider_meta_reads_snake_case_model_rewrites_and_defaults_to_none`
- `codex_single_exact_rewrite_rule_applies`
- `aggregate_bare_model_still_honours_rewrite_rules_but_explicit_slug_does_not`
- `codex_failover_exact_rewrite_rule_wins_over_default_model`
- `codex_failover_exact_rewrite_rule_wins_over_auto_review_model`
- `model_rewrite_rule_matches_case_insensitively_after_trim`
- `model_rewrite_rule_strips_one_m_marker_from_match_and_target`
- `model_rewrite_rule_skipped_for_connectivity_test`
- `claude_single_exact_rewrite_rule_applies_before_passthrough`
- `claude_failover_unmatched_rule_still_uses_family_mapping`
- `gemini_exact_rewrite_rule_applies`
- `gemini_forwarded_path_applies_resolved_model_segment`
- `gemini_forwarded_path_strips_one_m_marker_from_resolved_model`
- `gemini_forwarded_path_without_model_segment_unchanged`
- `resolved_rewrite_model_reaches_upstream_body_and_gemini_path`

前端：

- 共享编辑器 `web/features/coding/shared/providerModelRewrites/`（`ModelRewritesCollapse` + `modelRewritesUtils`），挂载在 Codex/Claude/Grok/Kimi/Gemini/ClaudeDesktop 的 provider 表单；official 渠道保存时强制清空。切换 gateway profile 时 `modelRewrites` 保留（`mergeGatewayProfileReferenceIntoMeta` 的 delete 白名单不含该 key）。
- 前端测试：`web/test/features/coding/shared/providerModelRewrites/modelRewritesUtils.test.ts`。

### 2.7 Codex failover auto-review 模型映射

触发条件：

- CLI 必须是 Codex。
- Gateway manifest 必须处于 `failover` 模式，并且不是指定 `provider_override_id` 的连通性测试。
- 请求 header `x-openai-subagent` 去空白后等于 `guardian` 或 `auto_review`（大小写不敏感）。

模型选择顺序：

1. auto-review/guardian 请求按当前候选 provider 解析模型，优先使用该 provider 的 `settingsConfig.autoReviewModelOverride` / `auto_review_model_override`。
2. 顶层 override 缺失或为空白时，兼容旧数据里的 `settingsConfig.modelCatalog.models[].autoReviewModelOverride` / `auto_review_model_override`，取首个非空值。
3. 仍未命中时回退该候选 provider 的 `default_model`。
4. 再没有时透传客户端请求里的 `model`。

主会话请求不读取 auto-review override；Codex failover 主会话仍只使用当前候选 provider 的 `default_model`。Codex `single` 模式下 auto-review/guardian 与主会话一样透传请求模型。所有分支最终都会剥离模型名中的 `[1M]` / `[1m]` 上下文标记。

默认模型来源仍是 provider `settings_config.config` 中 Codex `config.toml` 的 `[chat].model`，没有时回退根级 `model`。`modelCatalog` 只用于 legacy auto-review override 兼容，不能当作 Codex 主会话默认模型来源。

源码：

- `runtime/providers.rs::provider_from_record()`
- `runtime/providers.rs::codex_auto_review_model_from_settings()`
- `runtime/providers.rs::codex_auto_review_model_from_catalog()`
- `runtime/upstream.rs::is_codex_auto_review_request()`
- `runtime/upstream.rs::resolve_upstream_model_id()`

测试：

- `codex_provider_loads_default_model_from_config_toml`
- `codex_provider_loads_auto_review_model_from_model_catalog_legacy`
- `codex_provider_top_level_auto_review_overrides_model_catalog_legacy`
- `codex_provider_blank_auto_review_override_falls_back_to_catalog_then_none`
- `codex_failover_auto_review_uses_channel_auto_review_model`
- `codex_failover_auto_review_falls_back_to_channel_default_model`
- `codex_failover_auto_review_both_none_passthroughs_requested_model`
- `codex_failover_auto_review_header_value_auto_review_is_recognized`
- `codex_failover_main_session_ignores_auto_review_model`
- `codex_single_auto_review_preserves_requested_model`

### 2.8 最终思考强度的日志观测（issue #332）

`runtime/observability.rs::final_upstream_reasoning_effort()` 读取最终 attempt 的请求体快照；不改写出站 body，也不反向使用客户端原始值。根据响应携带的实际 `target_protocol` 选择对应字段，再对非空字符串 trim/lowercase；不能将其它协议遗留字段当作实际出站 effort：

| 目标协议 | 出站字段 |
|---|---|
| OpenAI Chat | 优先 `reasoning_effort`，回退该协议的 OpenRouter 方言 `reasoning.effort` |
| OpenAI Responses | 仅 `reasoning.effort` |
| Anthropic Messages | 仅 `output_config.effort` |
| Gemini Native | `generationConfig.thinkingConfig.thinkingLevel`，或对应 snake_case 表示 |
| 未知 | 不推断，返回空值 |

只记录明确发送的 effort；原生 `none` 等字符串保留原意，boolean thinking 开关和 `budget_tokens` 不猜成 low/high。若 provider 兼容或 rectifier 删除了 effort，则该次记录为空，不能回退到客户端原字段或模型名后缀。无上游 URL/响应快照的本地 schema 拒绝也不记录该字段；实际上游错误响应不因失败而丢失 effort。此信息独立于正文保存开关，SQLite 摘要和 JSONL summary 使用同一结果。

Copilot 的 warmup 模型和 Chat/Responses 动态 target 在每次 attempt 的发送入口前统一解析，连接失败和本地拒绝继续使用同一份 effective provider。首包失败/空响应包装继承原 response 的 target，不能用供应商默认 API 格式覆盖。

回归：`runtime/observability.rs::tests::final_effort_reads_explicit_upstream_dialects_without_inference`、`final_effort_ignores_fields_from_other_protocols`、`final_effort_round_trips_with_body_storage_disabled_and_metrics_only`，以及 `runtime.rs` 的同协议 Messages（含冲突协议字段）、Messages → Responses 真实转发和 `copilot_failed_requests_keep_effective_protocol_and_effort`（空响应、流式首包失败、连接失败）测试。更完整的指标范围见 [架构文档](gateway-protocol-conversion.md) §11.4 和 [Gateway 模块约束](../tauri/src/coding/proxy_gateway/AGENTS.md)。

## 3. 通用响应侧兼容

响应事实源是 `runtime/upstream.rs::build_gateway_response()`。

### 3.1 streaming 判定

`should_stream_response()` 只对 2xx/3xx 生效：

1. `Content-Type: text/event-stream` 优先进入 SSE 路径。
2. 明确非 SSE Content-Type，例如 `application/json`，不走 SSE wrapper，即使 request body 写了 `stream:true`。
3. 缺少 Content-Type 时，才 fallback 到 request `stream:true` 或 Gemini route streaming。
4. Ollama `application/x-ndjson` / `application/x-json-stream` 是专用例外，只对 Ollama provider_kind 转换。

### 3.2 SSE wrapper 顺序

当前流式路径顺序：

1. xAI native Responses namespace restore，且只在 HTTP 2xx restore。
2. Gemini shadow record。
3. Bailian OpenAI Chat SSE filter。
4. xAI/Grok OpenAI Chat SSE filter。
5. Ollama NDJSON -> OpenAI Chat SSE。
6. protocol SSE conversion。
7. Codex Responses SSE record。
8. request-scoped reverse pipeline middleware。

provider raw stream filter 必须发生在 protocol SSE conversion 之前。

Codex history 与 Gemini shadow 的旁路 SSE 记录器也遵循同一 SSE 分帧不变量：当缓冲区同时出现 `\n\n` 和 `\r\n\r\n` 时，必须消费物理位置更早的 delimiter，不能因固定换行优先级把多个 provider event 合并。回归测试为两个 side store 模块中的 `takes_physically_earliest_sse_delimiter`。

### 3.3 非流 JSON 顺序

非流 JSON 路径：

1. Ollama 2xx/3xx JSON 先转 OpenAI Chat JSON。
2. compact success/error 走 compact 专项转换。
3. protocol response/error conversion。
4. xAI native Responses 2xx restore。
5. reverse pipeline response middleware。
6. 用最终 body 和原始 upstream body 共同做 failure/empty response 分类。

Responses 非流 response output 中的多个 `reasoning` item 由 transformer 按出现顺序合并 summary 文本，最后一个有效 `encrypted_content` 作为 provider-local signature；runtime 只负责后续 provider wire 兼容，不覆盖该公共 IR 语义。回归测试：`responses_response_accumulates_multiple_reasoning_items`。

### 3.4 rectifier 默认行为

当前 `ProxyGatewaySettings` 默认：

| 设置 | 默认 | 行为 |
|---|---:|---|
| `thinking_rectifier_enabled` | `true` | Claude/Anthropic target 非流 4xx thinking/signature 兼容错误后，清理 thinking/signature 并同 provider 重试一次 |
| `responses_encrypted_content_rectifier_enabled` | `true` | OpenAI Responses target 非 compact、非流 4xx 且明确 encrypted_content 无法验证/解密时，删除失效 reasoning item 并重试一次 |
| `thinking_budget_rectifier_enabled` | `true` | Anthropic target 非流 4xx thinking budget 类问题时走预算修正重试 |
| `cache_injection_enabled` | `false` | target Anthropic 时才注入 cache_control |
| `lossy_rejection_enabled` | `false` | 有损转换默认放过并写 warning；显式开启后才硬拒绝 |

xAI native Responses passthrough 不是用户开关控制，而是严格自动门控，详见 5.2。

## 4. 当前 profile 概览

以下由当前 `tauri/resources/gateway_provider_profiles.json` 抽取。`*` 表示该 tool 的默认 endpoint。

| profile | providerType | compat 声明 | 当前 endpoint 摘要 |
|---|---|---|---|
| `deepseek` | `deepseek` | DeepSeek Chat/Anthropic | Claude `anthropic*`/`openai_chat`；Codex/Gemini/Grok `openai_responses*`/`openai_chat`/`anthropic_messages` |
| `zai_cn` / `zai_en` | `zai` | Z.ai Chat | Claude `anthropic*`/`openai_chat`；Codex `openai_responses*`/`openai_chat`/`anthropic_messages`；Gemini/Grok `openai_chat*`/`anthropic_messages` |
| `doubao` | `doubao` | Doubao metadata | Claude `anthropic*`/`openai_responses`；Codex/Gemini/Grok `openai_responses*`/`anthropic_messages` |
| `bailian` / `bailian_coding` | `bailian` | Bailian tool merge/SSE filter | Claude `anthropic*`/`openai_responses`；Codex/Gemini/Grok `openai_responses*`/`anthropic_messages` |
| `moonshot` / `kimi_coding` | `moonshot` | Moonshot Chat/Anthropic | Claude `anthropic*`/`openai_chat`；Codex/Gemini/Grok `openai_chat*`/`anthropic_messages` |
| `modelscope` | `modelscope` | remove metadata | Claude `anthropic*`/`openai_chat`；Codex/Gemini/Grok `openai_chat*`/`anthropic_messages` |
| `longcat` | `longcat` | Chat content array | Claude `anthropic*`/`openai_responses`；Codex/Gemini/Grok `openai_responses*`/`anthropic_messages` |
| `mimo` / `mimo_token_plan` | `mimo` | Anthropic tool thinking | Claude `anthropic*`/`openai_responses`；Codex/Gemini/Grok `openai_responses*`/`anthropic_messages` |
| `openrouter` | `openrouter` | reasoning object/field | Claude/Codex/Gemini/Grok 都是 `openai_chat*` |
| `siliconflow_cn` / `siliconflow_en` | `siliconflow` | Codex Chat `enable_thinking` | Codex/Gemini/Grok `openai_chat*` |
| `stepfun_cn` / `stepfun_ai` | `stepfun` | Codex Chat low/high effort | Codex/Gemini/Grok `openai_chat*` |
| `minimax_cn` / `minimax_global` | `minimax` | Codex Chat `reasoning_split` | Claude `anthropic*`/`openai_chat`；Codex/Gemini/Grok `openai_chat*`/`anthropic_messages` |
| `ollama` | `ollama` | Ollama `/api/chat` | Claude/Codex/Gemini/Grok 都是 `openai_chat*` |
| `github_copilot` | `github_copilot` | Copilot headers/token/dynamic route | Codex/Gemini/Grok `openai_chat*`，runtime 可请求级切 Responses |
| `xai` | `xai` | xAI Chat/Responses | Codex `openai_chat*` + `openai_responses`；Gemini/Grok `openai_chat*` |

注意：

- 有些 profile 当前默认 endpoint 不是 OpenAI Chat，但 runtime 仍存在 Chat 兼容分支；只有 provider target protocol 实际为 Chat 时才会触发。
- SiliconFlow、StepFun、MiniMax 没有独立 `ProviderBodyCompat` 分支；它们的当前兼容主要通过 Codex Chat reasoning meta/inference 和通用 Chat transformer/parser 覆盖。

## 5. 逐 provider/channel 兼容

### 5.1 OpenAI-like 与 Codex official

触发条件：

- 普通 OpenAI-like Chat/Responses 走通用 target protocol 兼容。
- Codex official adapter 触发于 `providerType=codex|openai-codex|chatgpt-codex|codex-official` 且 target protocol 为 `OpenAiResponses`。
- `category=official` provider 仍不参与 Gateway 候选；Codex official adapter 不改变这个安全边界。

请求侧：

- Codex official Responses body 强制 `stream=true`、`store=false`、`parallel_tool_calls=true`。
- 移除 `max_tokens`、`max_completion_tokens`、`metadata`、`user`（Chat 侧客户端可能带顶层 `user`，ChatGPT Codex 后端对该字段返回 400；AxonHub `147e6791`）。
- 确保 `include` 包含 `reasoning.encrypted_content`。
- 确保 `reasoning.summary="auto"`。
- headers 补 `Accept: text/event-stream`。
- 缺 `Originator` 时写 `Originator: ai-toolbox`。
- 缺 `Session_id` 时从 header `session_id` 或 header `x-codex-turn-metadata` JSON 中的 `session_id` 推导（不读 body `session_id`）。
- 保留客户端已有 `Originator`、`Session_id`、`Chatgpt-Account-Id`。
- 缺 `Chatgpt-Account-Id` 时尽力从 bearer JWT payload 的 OpenAI/ChatGPT account claim 解析。

响应侧：

- 官方 Codex 上游可能被强制流式。客户端非流时 runtime 聚合 Responses SSE 为 Responses JSON，再按需要做 response conversion。
- 聚合必须等待 terminal event；缺 terminal event 按连接错误进入 retry/failover。
- issue #318 的部分 Codex 镜像中转站导出体包含无 `\n\n` 的单行空格分隔 SSE，以及 `codex.rate_limits` / `codex.response.metadata` / `codex.event.balance` 自定义事件；不能凭 HTTP 200、非零 usage 或上游扣费推断成功。`response.reasoning_summary_part.done` 等中间事件里的 item `status=incomplete` 不能提前结束流，但真实 failed/canceled/error 必须保留。
- 无实际 reverse 改写时，出站 pipeline 直接按原 chunk 透传，不等待 SSE 分隔符或 EOF，避免把持续上游数据误计为空闲。需要 CCH 回填的 Anthropic reverse 路径仍按帧改写，遇到 transport error 要先冲刷已读尾部。真正上游停流的 `stream_idle_timeout` 不属于这项兼容。
- 终态/usage collector 按完整事件跨任意网络分片拼接，约 256 KiB 扫描并释放完成事件，单个未完成事件的硬上限为 16 MiB；未闭合 JSON 中的伪 `event:` / `data:` 不得被当成新事件。终态只从实际送达的字节确认，关闭正文日志或截断日志不能改变判定；详见架构主文档的 #318 专项边界。标准分隔符包住的退化扁平事件也必须走 usage fallback，不能只补 terminal 而丢计费数据。
- 回归：`usage_parser.rs::large_terminal_event_survives_every_network_chunk_size` / `partial_flattened_json_does_not_classify_quoted_field_tokens`，`runtime/http_io.rs::large_flattened_terminal_reaches_client_independently_of_body_logging` / `charged_200_stream_keeps_real_non_success_terminal_outcomes`，以及 `runtime/upstream.rs` 的 `reverse_sse_*`。补充回归：`flattened_events_with_a_final_blank_line_keep_usage`、`terminal_in_failed_write_is_not_counted_as_delivered`。
- 同一批中转站的 chunked framing 也可能不规范（终止 chunk 缺失、chunk size 行异常、连接提前断开），触发 hyper body 解码层在流中途报错（reqwest 路径统一显示 `error decoding response body`；header-preserving/hyper-util 路径显示 `error reading a body from connection` / `connection closed before message completed`）。`runtime/http_io.rs::is_demotable_stream_body_error()` 会把这类错误 demote 成干净流 EOF，不再注入合成 error event 破坏客户端已收到的流；成败仍按终态事件是否送达判定。这不是 provider 专属规则，对所有上游生效；见架构主文档「流中途 body 解码错误 demote 为干净 EOF」。

源码：

- `runtime/upstream.rs::apply_codex_official_responses_body_compat()`
- `runtime/upstream.rs::inject_codex_official_headers()`
- `runtime/upstream.rs::aggregate_sse_stream_for_non_streaming_client()`

测试：

- `provider_body_compat_codex_official_responses_forces_required_fields`
- `codex_official_headers_set_originator_accept_and_session`
- `codex_official_headers_preserve_client_originator_and_session`
- `codex_official_headers_derive_account_id_from_jwt_when_missing`
- `codex_official_sse_aggregate_*`

### 5.2 xAI / Grok

触发条件：

- Chat 兼容：`providerType=xai|x-ai|grok`，通常 target OpenAI Chat。
- native Responses passthrough：必须同时满足：
  - source protocol 是 `OpenAiResponses`
  - conversion route 为 `None`
  - target protocol 是 `OpenAiResponses`
  - provider kind 是 Xai，即 effective `providerType=xai|x-ai|grok`
- `compat` 里的 `xai_responses_passthrough` 只是 catalog 登记，不是生产开关。

请求侧，OpenAI Chat：

- 对 `grok-4.5` / `grok-4` 删除 `reasoning_effort`、`presence_penalty`、`frequency_penalty`、`stop`。
- 对 `grok-3` / `grok-3-mini` 删除 `presence_penalty`、`frequency_penalty`、`stop`。
- 支持 `xai/grok-*` 前缀归一后判断。

请求侧，native Responses：

- 从原始 namespace tools 建 request-local restore map。
- namespace children function 提升为顶层 function，名称使用 `flatten_namespace_tool_name()`。
- 同步改写 input history 的 `function_call.name/namespace`。
- namespace `tool_choice` 降为 `"auto"`；其它指向 namespace child 的 choice 改写为 flat name。
- flat name 与顶层 function/custom 或其它 namespace child 冲突时 fail closed，返回本地 RequestSchema。
- 删除顶层 `prompt_cache_retention`、`safety_identifier`。
- 对 `grok-4.5` 清理 presence/frequency penalty、stop。
- 递归删除 `external_web_access`。
- `input[].type=additional_tools` 提升到顶层 `tools` 并去重，carrier item 从 input 删除。
- reasoning item 的 `content:null` 删除 content。
- tools 只保留 allowlist：`function`、`web_search`、`x_search`、`image_generation`、`collections_search`、`file_search`、`code_execution`、`code_interpreter`、`mcp`、`shell`。
- `tool_choice` 指向被删除/unsupported tool 时删除。

响应侧：

- xAI Chat SSE filter 丢弃 choices 中全是空 delta、无 finish_reason、无 usage 的 chunk。
- native Responses JSON/SSE 只在 HTTP 2xx 恢复 function_call namespace；3xx/4xx/5xx 不恢复。
- SSE restore 对 `[DONE]` 原样透传。

开关/默认：

- native Responses 兼容是默认自动严格门控，不是用户开关。

源码：

- `runtime/compat/xai_responses.rs`
- `runtime/upstream.rs::should_apply_xai_responses_passthrough()`
- `runtime/upstream.rs::maybe_filter_xai_openai_chat_sse_stream()`

测试：

- `xai_responses_passthrough_gate_accepts_xai_provider_aliases`
- `xai_responses_passthrough_gate_requires_explicit_responses_source`
- `xai_responses_passthrough_scrubs_native_responses_body`
- `xai_responses_passthrough_skips_non_xai_provider`
- `xai_stream_filter_drops_empty_delta_chunks`
- `provider_body_compat_xai_chat_strips_model_specific_unsupported_fields`
- `provider_body_compat_xai_chat_strips_grok_45_and_prefixed_model_ids`
- `runtime/compat/xai_responses.rs` 内 namespace flatten/restore/sanitize 单测

### 5.3 DeepSeek

触发条件：

- `providerType=deepseek`。
- Chat、Anthropic target 和 legacy Completion path 各有不同 runtime 分支。
- profile `deepseek` 的 Codex/Gemini/Grok 默认 endpoint 自 2026-08-01 切到 `openai_responses`（官方新增 Responses API 支持），保留 `openai_chat` 与 `anthropic_messages` 作可选 fallback。Responses target 走通用 OpenAI Responses 直通，无 DeepSeek 专用 body compat（compat 规则 `deepseek_json_schema`/`deepseek_thinking`/`deepseek_disabled_strip_effort` 仅作用于 `openaiChat`/`anthropicMessages`）。
- `openai_responses` endpoint 的 `baseUrl` 用官方值 `https://api.deepseek.com`（不带 `/v1`，与 DeepSeek 官方 Codex 接入脚本一致）。DeepSeek 服务端对 `/responses` 与 `/v1/responses` 双路径兼容，Codex/Grok CLI 直连按 `base_url + /responses` 可直接命中；走网关时 `build_target_url`（`runtime/routes.rs`）对 OpenAiResponses 固定拼 `/v1/responses` 也能命中。内置官方渠道的 baseUrl 由 profile 提供且已验证可用，因此 Codex/Grok 表单的 `/v1` 软确认对内置 endpoint 跳过（仅对自定义手填地址保留）。
- Codex provider 生成 `ai-toolbox-codex-model-catalog.json` 时，`wire_api="responses"`（native Responses）且 `base_url` 命中 `deepseek.com` 的渠道镜像内置的 DeepSeek 官方 models.json（`tauri/resources/codex_deepseek_catalog_template.json`：freeform `apply_patch`、GPT-5 harness base_instructions、low/high/max reasoning、1m context），不套 neutral 模板的 image/text_and_image 声明；用户显式 `displayName` / `contextWindow` 仍优先，未知模型克隆官方旗舰条目。非 `deepseek.com` host 或非 Responses target（chat/anthropic）仍用 neutral 模板。实现在 `codex/commands.rs`，不进入 runtime/transformer。
- Codex catalog：`deepseek.com` native `/responses` 渠道的 vendor mirror 把 `supports_search_tool` 设为 `false`（DeepSeek Responses API 无 `tool_search`，Codex 会据此把 MCP 工具藏在 tool_search 后面导致调不到；参考 cc-switch `5a040348`）。neutral 模板保留 `true`，因为跨协议目标由网关展平 `tool_search`/namespace 后下发。

请求侧，OpenAI Chat：

- `response_format.type=json_schema` 降为 `json_object`，移除 `json_schema`。
- 按 `reasoning_effort` 写 `thinking.type=enabled|disabled`。
- disabled 时移除 `reasoning_effort` 并清理 assistant reasoning 字段。
- enabled 时 effort 映射：`max/xhigh -> max`，其它 -> `high`。
- 有 tool_calls 的 assistant 历史保留/回填 `reasoning_content`；无 tool_calls 的 assistant 历史移除 `reasoning_content` 和 `reasoning`。

请求侧，Anthropic target：

- 规范化 assistant tool_use 历史 thinking：删除 signature，空 thinking 补 `"tool call"`，`redacted_thinking` 转普通占位 thinking，无 thinking 时插入 `thinking:"tool call"`。
- `thinking.type=disabled` 时删除 `output_config.effort`，必要时删除空 `output_config`，并删除 `reasoning_effort`。

请求侧，legacy Completion：

- Codex/OpenAI `/v1/completions` 或 `/completions` + DeepSeek provider 改写 URL 到 `/beta/completions`。
- 该路径跳过 Chat body adapter，不进入 transformer 聊天矩阵。

响应侧：

- 没有 DeepSeek 专用 response wrapper；走通用 response conversion、failure classification、usage parser。

源码：

- `runtime/upstream.rs::apply_openai_chat_provider_body_compat_before_generic()`
- `runtime/upstream.rs::apply_anthropic_provider_body_compat()`
- `runtime/upstream.rs::is_deepseek_legacy_completion_forward()`

测试：

- `provider_body_compat_deepseek_chat_rewrites_json_schema_thinking_and_custom_tools`
- `provider_body_compat_deepseek_chat_preserves_reasoning_with_tool_calls_and_strips_without`
- `provider_body_compat_deepseek_anthropic_disabled_thinking_strips_effort_fields`
- `deepseek_legacy_completion_route_uses_beta_path`
- `deepseek_legacy_completion_body_skips_chat_adapter`
- `deepseek_host_native_catalog_mirrors_official_entries`
- `non_deepseek_or_non_native_provider_keeps_neutral_template`

### 5.4 Moonshot / Kimi

触发条件：

- `providerType=moonshot|kimi`。

请求侧：

- OpenAI Chat target：`response_format.type=json_schema` 降为 `json_object`；assistant 有 tool_calls 且无非空 `reasoning_content` 时补 `"tool call"`。
- Anthropic target：规范化 assistant tool_use 历史 thinking。
- Codex -> Chat reasoning 矩阵可用 `thinking` + `reasoning_content`。
- OpenAI Chat `prompt_cache_key` 对 `kimi` / `moonshot` allowlist provider 可 reinject。

响应侧：

- Moonshot/Kimi Anthropic-compatible usage 解析按 provider-aware 规则处理 `cached_tokens` 和可能的负 input token 折扣；该逻辑属于 usage/cost 兼容，不在 transformer。
- Kimi CLI 接管的 OpenAI Chat 兼容 usage 解析除 `prompt_tokens_details.cached_tokens` 外，还会防御性读取 Moonshot 风格顶层 `usage.cached_tokens` 作为 `cache_read_tokens`，并按仓库统一语义把 `prompt_tokens` 当 cache-inclusive 总数扣减 fresh input。该路径仅对 `GatewayCliKey::Kimi` 生效；Codex/Grok/OpenCode 不读顶层 `cached_tokens`，保持共享 `openai_usage` 行为不变（`usage_parser.rs` 的 `openai_usage_with_extra_cache_read_paths`）。

测试：

- `provider_body_compat_anthropic_reasoning_vendor_normalizes_tool_thinking_history`
- `provider_compat_moonshot_rewrites_schema_and_backfills_tool_reasoning`
- `chat_prompt_cache_key_reinjects_explicit_for_allowlisted_provider`
- `parses_kimi_moonshot_style_top_level_cached_tokens`
- `non_kimi_openai_usage_ignores_top_level_cached_tokens`

### 5.5 Z.ai / GLM / 智谱

触发条件：

- `providerType=zai|zhipu|glm|chatglm|bigmodel|big-model`。

端点（官方按 CLI 拆成三条互不通用的路径，内置 profile `zai_cn` / `zai_en` 与之对齐）：

- Claude Code：`/api/anthropic`（Anthropic Messages，`claude` 默认 endpoint `anthropic`）。
- 通用 OpenAI Chat：`/api/coding/paas/v4`（`codex`/`gemini`/`grok` 的历史 endpoint `openai_chat`）。
- Codex：`/api/v1`（OpenAI Responses，CN `https://open.bigmodel.cn/api/v1`、EN `https://api.z.ai/api/v1`），是 `codex` 的默认 endpoint `openai_responses`。官方 Codex 接入文档（`docs.bigmodel.cn/cn/coding-plan/tool/codex.md`）要求 `wire_api = "responses"`，并用 Codex catalog 形状的 `model_catalog_json`。
- 内置 `openai_responses` endpoint 的模型目录是 `glm-5.3`（1M 上下文，默认模型）和 `glm-5-turbo`（200k）。用户已保存的旧 provider 仍持有 `endpointId=openai_chat` 引用，runtime 按引用解析，不会自动迁移到新默认 endpoint。
- 同一个 `/api/v1` 前缀还提供 Codex catalog 形状的模型列表 `GET /models`（`{"models":[{"slug":...}]}`）；模型发现必须识别该 schema（`tauri/src/coding/open_code/models_api.rs::parse_generic_models_response`，模块规则见 `tauri/src/coding/open_code/AGENTS.md`）。
- issue #381 的两处现象都来自这里的错配：Codex 用 Chat endpoint 让网关做 Responses -> Chat 转换时「模型测试」失败；改成 `/api/v1` 后「获取模型」又因 catalog schema 不被 `fetch_provider_models` 识别而报 `Failed to parse models response`。

请求侧，OpenAI Chat：

- JSON Schema response_format 降为 `json_object`。
- `metadata.user_id/request_id` 提升为顶层字段。
- 无 request_id 时生成 `req_<timestamp>`。
- 有 `tool_choice` 时强制为 `auto`。
- 按 `reasoning_effort` 写 `thinking.type`。
- Codex -> Chat reasoning 矩阵可写 `thinking`。
- **待处理（需要跨层方案）**：AxonHub `834eea2b` 的 GLM-5.2+ `reasoning_effort` 与 GLM-5.3 恒开思考（其 #2314）。本仓 Z.ai Chat 的 `thinking.type` 有两个写入点：`runtime/upstream.rs::apply_zai_openai_chat_thinking_compat`（body compat 层）与 Codex -> Chat reasoning 矩阵 `apply_codex_chat_reasoning_config`（有 provider meta 时后者会在 `reasoning_enabled == false` 时清 `reasoning_effort`、写 `reasoning.effort`/`thinking_param`，可能覆盖前者）。只改 body compat 层会让“有 provider meta”与“无 provider meta”两条路径分叉，本轮完整回退、不落地半成品；吸收必须在 reasoning 矩阵内统一决定 `thinking.type` 的最终值，并补 GLM-5.2+/5.3 与 GLM-4.x 的档位回归。结论记录见 `docs/gateway-protocol-conversion.md` 的 2026-09-22 AxonHub 条目。

响应侧：

- 没有专用 response wrapper；走通用 response conversion。

测试：

- `provider_body_compat_zai_chat_moves_metadata_and_forces_auto_tool_choice`

### 5.6 Doubao / Volces

触发条件：

- `providerType=doubao|doubaoseed|doubao-seed|volces`。

请求侧：

- OpenAI Chat target：`metadata.user_id/request_id` 提升，缺 request_id 时生成；按 `reasoning_effort` 写 `thinking.type`。通用 Chat 清理之后顶层 `reasoning_effort` 不直传。
- OpenAI Responses target：删除 `metadata`。

响应侧：

- 没有 Doubao 专用 response wrapper。

注意：

- 当前 profile 默认多为 Anthropic 或 Responses endpoint；Chat branch 只有实际 target 为 OpenAI Chat 时触发。

测试：

- `provider_body_compat_doubao_chat_extracts_metadata_and_generates_request_id`

### 5.7 Bailian / DashScope / Qwen / Aliyun

触发条件：

- `providerType=bailian|dashscope|aliyun`。

请求侧，OpenAI Chat：

- 合并连续 assistant tool-call-only message。
- 有 side-effect 字段的 message 不合并，避免丢失 provider 附加语义。
- Codex -> Chat reasoning 矩阵可写 `enable_thinking`。

响应侧，OpenAI Chat SSE：

- 只在 target OpenAI Chat 且 provider kind Bailian 时启用 raw stream filter。
- 见到 `tool_calls` 后，后续文本 delta 先缓冲，在 finish 前作为独立 text delta 重发，避免 tool call 后文本顺序问题。
- 如果某个 tool call 已累计非空 arguments，上游再发 `{}` 参数片段时改为空字符串，避免重复空 args 污染已累计参数。

注意：

- 当前 profile 默认多为 Anthropic 或 Responses endpoint；Chat branch 只有实际 target 为 OpenAI Chat 时触发。
- SSE filter 必须保持在 runtime raw upstream SSE adapter，不能下沉到 transformer。

测试：

- `provider_body_compat_bailian_chat_merges_consecutive_tool_call_messages`
- `provider_body_compat_bailian_keeps_tool_call_messages_with_side_effect_fields`
- `bailian_stream_filter_buffers_text_after_tool_calls_until_finish`
- `bailian_stream_filter_drops_duplicate_empty_tool_arguments`

### 5.8 OpenRouter

触发条件：

- `providerType=openrouter|open-router`，通常 target OpenAI Chat。

请求侧：

- 顶层 `reasoning_effort` 移到 `reasoning.effort`。
- effort 映射：`max/xhigh -> xhigh`；`high/medium/low/minimal` 保留；`none/off/disabled -> none`。
- 默认 reasoning field policy 为 `reasoning`，除非 meta 显式覆盖。
- Codex -> Chat reasoning 矩阵使用 `reasoning.effort`。
- OpenRouter 在 `reasoningField=none` / disabled thinking 时仍会剥离 assistant reasoning 字段；“其它 none”仅对显式 disabled 成立，不是默认对所有非 OpenRouter 路径都 none。

响应侧：

- OpenAI Chat assistant 历史 reasoning 字段按 `reasoning` 策略输出/保留。

测试：

- `provider_body_compat_openrouter_moves_reasoning_effort_to_reasoning_object`
- `provider_body_compat_openai_chat_applies_reasoning_field_policy`
- `codex_chat_reasoning_config_maps_openrouter_effort_object`

### 5.9 SiliconFlow

触发条件：

- 当前没有 `ProviderBodyCompat::SiliconFlow`。
- 兼容由 `codexChatReasoning` explicit meta 或 `infer_codex_chat_reasoning_config()` 在明确 effective `providerType=siliconflow` 且 target OpenAI Chat 时触发。

请求侧：

- Codex -> Chat reasoning 写 `enable_thinking`。
- 不传 effort。
- output reasoning 期望为 `reasoning_content`。

响应侧：

- 由通用 OpenAI Chat transformer/parser 提取 `reasoning_content`。

注意：

- Gemini/Grok/Claude endpoint 不会从 profile 解析 Codex-only `codexChatReasoning` 字段；但如果最终 target 是 OpenAI Chat，且 effective `providerType` 明确为 `siliconflow`，runtime fallback inference 仍会触发 `enable_thinking` 兼容。区别是：显式 profile 配置只来自 Codex endpoint，fallback inference 来自明确 providerType/apiFormat，不来自模型名猜测。

测试：

- `codex_chat_reasoning_config_strips_effort_for_thinking_only_provider`
- `codex_chat_reasoning_custom_qwen_model_does_not_infer_provider_compat`
- `provider_compat_siliconflow_uses_enable_thinking_without_reasoning_effort`

### 5.10 StepFun

触发条件：

- 当前没有 `ProviderBodyCompat::StepFun`。
- 兼容由 `codexChatReasoning` 或明确 effective `providerType=stepfun` 的 inference 触发。

请求侧：

- `thinkingParam=none`。
- `effortParam=reasoning_effort`。
- `effortValueMode` 按模型分档：`step-3.5-flash-2603` 用 `low_high`（`minimal/low -> low`，其它 -> `high`）；`step-3.7-flash` 用 `passthrough`（low/medium/high 原样下发，套 `low_high` 会把 medium 塌成 high）；无后缀 `step-3.5-flash` 与其它 step 模型不下发 effort。
- `supportsEffort` 仅在模型含 `2603` 或 `step-3.7-flash` 时为真；模型名只在已识别 providerType 为 StepFun 后用于能力细分；自定义 provider 不会因为模型名包含 stepfun/2603 自动套规则（参考项目 `haystack.contains("step-3.5-flash-2603")` 的聚合渠道分支明确不吸收）。

响应侧：

- 走通用 Chat。

测试：

- `codex_chat_reasoning_custom_provider_model_names_do_not_infer_provider_compat`
- `provider_compat_stepfun_uses_per_model_effort_tiers`
- `codex_chat_reasoning_explicit_meta_overrides_inference`

### 5.11 MiniMax

触发条件：

- 当前没有 `ProviderBodyCompat::MiniMax`。
- 兼容由 `codexChatReasoning` 或明确 effective `providerType=minimax` 的 inference 触发。

请求侧：

- Codex -> Chat reasoning 写 `reasoning_split`。
- output format 声明为 `reasoning_details`。

响应侧：

- OpenAI Chat transformer/parser 已能提取 `reasoning_details`。

注意：

- text-only 图片预测启发式名单包含 MiniMax 相关模型前缀，但只有 `allowTextOnlyModelHeuristic=true` 时启用。
- 如 MiniMax endpoint 未来需要额外 body 字段清理，应新增 runtime adapter 和测试，不能只改 profile compat 描述。

测试：

- `codex_chat_reasoning_custom_provider_model_names_do_not_infer_provider_compat`
- `provider_compat_minimax_uses_reasoning_split_and_reasoning_details_output`

### 5.12 MiMo

触发条件：

- `providerType=mimo|xiaomimimo|xiaomi-mimo`。

请求侧：

- OpenAI Chat target：assistant tool_call 缺 `reasoning_content` 时补 `"tool call"`。
- Anthropic target：规范化 assistant tool_use 历史 thinking。
- Codex -> Chat reasoning 矩阵可写 `thinking` + `reasoning_content`。

响应侧：

- 没有专用 response wrapper。

测试：

- `provider_body_compat_anthropic_reasoning_vendor_normalizes_tool_thinking_history` 覆盖同类 Anthropic tool thinking 历史规范化，但当前样例使用 Moonshot provider。
- `provider_compat_mimo_alias_backfills_missing_tool_reasoning_content`

### 5.13 LongCat

触发条件：

- body compat 的 canonical `providerType=longcat`；当前内置 profile 只生成该值。
- `providerType` 归一化会把 `long_cat` / `long-cat` 统一为 `longcat`；OpenAI Chat 的 LongCat content-array body compat 与 Anthropic platform/auth 都识别这些别名。

请求侧：

- OpenAI Chat target after generic：所有 message `content` 归一为 array。
- string -> text part。
- null/none -> empty text。
- object -> array[object]。
- 其它类型 -> text。
- Anthropic target 作为 `AnthropicPlatform::LongCat`，使用 Bearer auth，并按非 Direct/非 Bedrock 平台过滤 native web_search。

响应侧：

- 没有专用 response wrapper。

测试：

- `provider_body_compat_longcat_chat_forces_message_content_arrays`

### 5.14 ModelScope

触发条件：

- `providerType=modelscope|model-scope`。

请求侧：

- OpenAI Chat target：删除 `metadata`。
- OpenAI Responses target：删除 `metadata`。
- profile 可声明 Codex -> Chat reasoning 配置。

响应侧：

- 没有专用 response wrapper。

测试：

- `provider_compat_modelscope_removes_metadata_for_chat_and_responses`
- `provider_body_compat_detects_canonical_provider_type_aliases`

### 5.15 Anthropic Direct / Bedrock / Vertex

触发条件：

- target protocol 必须是 `AnthropicMessages`。
- `providerType=bedrock|anthropic-bedrock|aws-bedrock` -> Bedrock。
- `providerType=vertex|anthropic-vertex|claude-vertex` -> Anthropic Vertex。
- `providerType=anthropic|claude|direct|claude-code` -> Direct Anthropic。
- `providerType=longcat|long-cat` -> LongCat platform。

请求侧，header/path/auth：

- Bedrock URL 使用 `/model/{model}/invoke` 或 `/model/{model}/invoke-with-response-stream`。
- Vertex URL 使用 base URL 中 project/location 前缀拼 `publishers/anthropic/models/{model}:rawPredict` 或 `:streamRawPredict`。
- Bedrock header/body version 为 `bedrock-2023-05-31`。
- Vertex header/body version 为 `vertex-2023-10-16`。
- Direct Anthropic 默认 `anthropic-version: 2023-06-01`。
- Bedrock/LongCat 使用 Bearer auth；Direct Anthropic 默认 `x-api-key`。

请求侧，body：

- Bedrock body 写 `anthropic_version=bedrock-2023-05-31`，移除 `model` 和 `stream`。
- Vertex body 写 `anthropic_version=vertex-2023-10-16`。
- Direct Anthropic body 含 native `web_search` tool 时注入 `anthropic-beta: web-search-2025-03-05` header。
- Bedrock native `web_search` 通过 body `anthropic_beta=["web-search-2025-03-05"]`。
- Vertex/LongCat/普通非 Direct、非 Bedrock 平台过滤 native web_search tool。

响应侧：

- 没有平台专用 response wrapper；走 Anthropic response/SSE conversion 和通用 failure classification。

测试：

- `anthropic_bedrock_provider_uses_model_invoke_path_and_version_header`
- `anthropic_bedrock_body_clears_model_and_stream`
- `anthropic_direct_web_search_adds_beta_header`
- `anthropic_vertex_filters_native_web_search_tool`
- `anthropic_vertex_provider_uses_raw_predict_path_and_version_header`

### 5.16 Gemini Direct / Vertex

触发条件：

- Gemini Native target 由 provider target protocol 决定。
- Gemini Vertex body compat 来自 `providerType=vertex|googlevertex|google-vertex|geminivertex|gemini-vertex`，并且 `target_protocol` 必须是 `GeminiNative`；相同 `vertex` 字符串在 `AnthropicMessages` target 下归类为 Anthropic Vertex，`google-vertex` 在 Chat/Anthropic target 下不归类为 Gemini Vertex。

请求侧：

- target Gemini + streaming 自动补 `alt=sse`。
- Gemini API version 从 provider base URL 推断，支持 `v1` / `v1beta` / `v1alpha`。
- Gemini source 转非 Gemini target 时过滤 `alt=sse` 和 `key=` query。
- Gemini Vertex target 删除 `contents[].parts[].functionCall.id` 和 `functionResponse.id`；删除前依据同轮原生调用 ID 排列 functionResponse，匿名结果再匹配尚未被显式 ID 占用的同名调用位置，不能把已移除合成 ID 的结果统一排到末尾。Gemini 2.x 后随 marker/inline media 与 Gemini 3.x nested media 必须随各自结果一起移动。同名工具逆序完成时，直接删 ID 会交换结果归属。该规则仅由 `GeminiVertex` provider compat 触发，普通 Gemini 保留原生 ID 与结果提交顺序。

响应侧：

- target Gemini 的原始 SSE 可被 `GeminiShadowStore` 旁路记录，用于后续 reliable session 的 thoughtSignature shadow 回放。
- Gemini response/SSE 的协议结构转换由 transformer 处理。工具快照只在非空原生 ID 相同时合并；无 ID 的独立调用逐次保留，不能按函数名 deduplicate。正常 SSE 暂存工具快照、持续输出文本/reasoning，在终态输出完整工具参数；非流客户端的 forced SSE 由 runtime 聚合，复用同一纯协议 helper。回归：`gemini_sse_aggregate_*`、`outbound_adapter_*vertex*` 及 `tauri/tests/coding/proxy_gateway/`。

测试：

- `outbound_adapter_strips_gemini_function_ids_for_vertex`
- `gemini_vertex_provider_kind_requires_gemini_native_target`
- `conversion_route_rewrites_claude_to_gemini_native_generate_content_path`
- `conversion_route_rewrites_claude_to_gemini_native_streaming_path_and_query`

### 5.17 GitHub Copilot

触发条件：

- `providerType=copilot|github-copilot|githubcopilot`。

请求侧，target protocol：

- 请求级模型 `gpt-<major>` 且 major >= 5、并且不是 `gpt-5-mini` -> OpenAI Responses。
- 其它模型 -> OpenAI Chat。
- warmup 降级到 `gpt-5-mini` 只在 provider 是 Copilot、请求头包含 `anthropic-beta`、body 不是 compact、initiator 为 user、无 tools 时执行。

请求侧，token/header：

- API key field 是 GitHub-token-like，或 token 前缀为 `ghp_`、`github_pat_`、`gho_`、`ghu_`、`ghs_`、`ghr_` 时，先 exchange 到 Copilot bearer token。
- raw Copilot token 直接 Bearer。
- exchange endpoint 是 `https://api.github.com/copilot_internal/v2/token`。
- 结果按 token hash 缓存到过期前 5 分钟。
- 注入/覆盖 fingerprint headers：
  - `Editor-Version`
  - `Editor-Plugin-Version`
  - `User-Agent`
  - `Copilot-Integration-Id`
  - `Openai-Intent`
  - `X-Github-Api-Version`
  - `X-Vscode-User-Agent-Library-Version`
- body 有图片时写 `Copilot-Vision-Request: true`。
- 按 body/header 计算 `X-Initiator`。
- subagent 写 `X-Interaction-Type: conversation-subagent`。
- 基于 session/body 生成确定性 interaction/request/task ids。

请求侧，body：

- Claude 4.x Copilot model id 归一化，覆盖 date/dash/dot 和 `[1M]` 形态。
- Chat target 删除 assistant content 中 `thinking` / `redacted_thinking`。
- Chat orphan tool message 降级成 user 文本 `[Tool result for ...]`。
- Responses orphan `function_call_output` 降级成 user message。
- Responses `function_call` item `id=call_id`；缺 name 时 name=`function`。

响应侧：

- 没有专用 response wrapper；动态 route 后按 target protocol 的通用 response conversion。

注意：

- 不包含 GitHub device-code 登录 UI、账号存储或 live model list fallback。
- Copilot profile 必须保存 origin base URL，不能 fixed full URL 到 `/chat/completions`，否则会绕过动态 `/responses` path。

测试：

- `copilot_model_uses_responses_api_matches_axonhub_rule`
- `copilot_effective_provider_switches_chat_and_responses_by_model`
- `copilot_warmup_downgrades_model_before_route_selection`
- `copilot_warmup_does_not_downgrade_tool_or_agent_requests`
- `copilot_token_exchange_detection_is_explicit_or_github_token_shaped`
- `copilot_token_exchange_sends_github_token_and_caches_response`
- `copilot_openai_chat_body_normalizes_model_and_sanitizes_orphan_tool_message`
- `copilot_responses_body_normalizes_function_item_ids_and_orphan_outputs`
- `copilot_headers_override_forwarded_fingerprint_and_infer_agent_turn`
- `copilot_headers_detect_compact_subagent_and_vision`

### 5.18 Ollama

触发条件：

- `providerType=ollama|ollama-chat|ollamachat` 且 target OpenAI Chat。
- 或 `data.meta.apiFormat=ollama/chat` 且 target OpenAI Chat。

请求侧：

- Gateway target protocol 仍视作 OpenAI Chat，不新增第五种 transformer 协议。
- 上游 URL 使用 `/api/chat`，并剥离 base URL 尾部 `/v1`。
- 最后一步把 OpenAI Chat body 投影为 Ollama wire format：
  - `model`
  - `messages[].role/content`
  - `tools` 及 assistant 历史 `tool_calls`；合法 function arguments JSON 字符串解码为对象，不用空对象静默覆盖无法解析的原参数
  - 已有消息 `name/tool_name` 保留为 Ollama `tool_name`
  - `image_url` data URL 去前缀放 `images[]`
  - 普通 URL 原样放 `images[]`
  - reasoning field -> `thinking`
  - `temperature/top_p/top_k/max_tokens/max_completion_tokens` -> `options`
  - `max_*` -> `options.num_predict`
  - `stop` -> `options.stop` array
  - `response_format=json_object` -> `"json"`
  - `response_format=json_schema` -> schema object
  - `stream` 缺省 false

响应侧：

- 非流 Ollama JSON 先转 OpenAI Chat response。
- Ollama NDJSON / x-json-stream 先转 OpenAI Chat SSE，再进入 protocol SSE conversion。
- usage、done_reason、tool_calls、thinking 都在 Ollama adapter 中归一到 Chat 形态。工具参数对象序列化为 Chat JSON 字符串；缺少调用 ID 时生成当前响应内稳定的 `call_ollama_<index>`，流式多个调用依次分配 index。工具轮正常结束写 `finish_reason:tool_calls`，达到长度上限仍优先保留 `length`。
- 最终用量使用 Chat `choices:[]` usage-only 事件；转换到 Gemini 时必须等 usage 或正常 EOF 后再发唯一 finish，不能把真实 token 用量丢掉。仅真实 `done` 才发送正常终态；非空 error 或缺 done 的 EOF 进入流错误，不自动补成功 `[DONE]`。

测试：

- `ollama_chat_url_uses_api_chat_and_strips_v1_base_suffix`
- `ollama_body_compat_converts_openai_chat_request_shape`
- `ollama_json_response_converts_to_openai_chat_response`
- `ollama_ndjson_stream_converts_to_openai_chat_sse`
- `runtime/websocket/privacy_matrix_tests.rs::privacy_all_cli_entries_restore_ollama_json_and_ndjson`：七入口、JSON/NDJSON、开关两态，校验出站工具定义/历史、客户端文本/工具参数与精确 token 用量
- `privacy_ollama_stream_failures_do_not_deliver_a_success_terminal`：显式 error 与截断 EOF
- `transformer/kernel_tests.rs::chat_stream_to_gemini_waits_for_usage_only_chunk_before_finish`：晚到用量与唯一终态

### 5.19 Custom / legacy provider

触发条件：

- 没有 `gatewayProfile`，或 profile 解析失败/不匹配，runtime 使用 legacy meta。
- 如果 legacy meta 也没有 `providerType`，fallback 到 provider record `category`。

请求侧：

- 仍执行通用 source/target protocol 转换、header/auth、URL/path、Chat 通用清理、tool controls 清理、prompt cache 默认策略、lossy 策略、predictive media policy 等。
- 不会仅凭模型名或 Base URL 猜 DeepSeek/Qwen/GLM/MiniMax/MiMo/StepFun 等供应商方言。

响应侧：

- 走通用 response conversion、streaming 判断、failure/empty response classification。

注意：

- 如果用户希望 custom provider 获得内置供应商方言，应通过内置 profile endpoint 或显式 legacy meta 选择该兼容行为，而不是依赖模型名。

测试：

- `codex_chat_reasoning_custom_deepseek_model_does_not_infer_provider_compat`
- `codex_chat_reasoning_custom_qwen_model_does_not_infer_provider_compat`
- `codex_chat_reasoning_custom_provider_model_names_do_not_infer_provider_compat`

## 6. Codex Chat reasoning 矩阵

该矩阵只对 target OpenAI Chat 生效，入口是 `runtime/upstream.rs::apply_codex_chat_reasoning_config()` 和 `infer_codex_chat_reasoning_config()`。

优先级：

1. explicit `meta.codexChatReasoning/codex_chat_reasoning`。
2. 缺 explicit 时，根据明确 effective `providerType/apiFormat` fallback。
3. 自定义 provider 不能因为 body `model` 字符串像某供应商而触发 fallback。
4. 模型名只允许在已识别 provider 内做能力细分，例如 StepFun 的 `2603`（low_high 收敛）与 `step-3.7-flash`（passthrough）。

当前支持：

| 模式 | 出站字段 |
|---|---|
| `thinkingParam=thinking` | `thinking:{type:enabled|disabled}` |
| `enable_thinking` | `enable_thinking: true/false` |
| `reasoning_split` | `reasoning_split: true/false` |
| `none` | 不写 thinking 参数 |
| `effortParam=reasoning_effort` | 写顶层 `reasoning_effort` |
| `effortParam=reasoning.effort` | 写 `reasoning.effort` |

disabled 行为：

- disabled 时删除顶层 `reasoning_effort`。
- 如果 effortParam 是 `reasoning.effort`，写 `reasoning:{effort:none}`。
- `supportsEffort=false` 时删除 `reasoning_effort`。

effort 映射：

- `deepseek`：`max/xhigh -> max`，其它 -> `high`。
- `low_high`：`minimal/low -> low`，其它 -> `high`。
- `openrouter`：`max/xhigh -> xhigh`，`high/medium/low/minimal` 保留，其它 none。
- `passthrough`：支持 `minimal/low/medium/high/xhigh/max`。

inferred provider：

- OpenRouter -> `reasoning.effort`，openrouter mode。
- SiliconFlow -> `enable_thinking`，不支持 effort，output `reasoning_content`。
- DeepSeek -> `thinking`，支持 effort，deepseek mode。
- StepFun -> 只有 provider 已识别后按模型分档：模型含 `2603` 用 low_high 收敛，`step-3.7-flash` 用 passthrough，其余不发 effort。
- Kimi/Moonshot -> `thinking`，不支持 effort，output `reasoning_content`。
- GLM/Z.ai/Zhipu -> `thinking`，不支持 effort。
- Qwen/DashScope/Bailian -> `enable_thinking`。
- MiniMax -> `reasoning_split`，output `reasoning_details`。
- MiMo -> `thinking`。

默认 reasoning field 额外漏记行为：

- 未显式配置时默认 `reasoning_content`；`providerType=openrouter|nanogpt`（及常见别名）默认 `reasoning`。
- `long_cat` / `long-cat` 归一化后等效 `longcat`。
- 未知 `providerType` 的 Anthropic target 默认按 Direct 语义处理（注入 Direct 风格 `anthropic-version` / web_search beta 规则），而不是 silently no-op。

测试：

- `codex_chat_reasoning_config_maps_deepseek_effort_and_thinking`
- `codex_chat_reasoning_config_maps_openrouter_effort_object`
- `codex_chat_reasoning_config_strips_effort_for_thinking_only_provider`
- `codex_chat_reasoning_infers_deepseek_without_explicit_meta`
- `codex_chat_reasoning_custom_deepseek_model_does_not_infer_provider_compat`
- `codex_chat_reasoning_infers_openrouter_platform_before_model`
- `codex_chat_reasoning_custom_qwen_model_does_not_infer_provider_compat`
- `codex_chat_reasoning_custom_provider_model_names_do_not_infer_provider_compat`
- `codex_chat_reasoning_explicit_meta_overrides_inference`
- `provider_compat_siliconflow_uses_enable_thinking_without_reasoning_effort`
- `provider_compat_stepfun_uses_per_model_effort_tiers`
- `provider_compat_minimax_uses_reasoning_split_and_reasoning_details_output`

## 7. Header、path、auth 兼容

通用规则：

- 入站 `authorization`、`x-api-key`、`x-goog-api-key`、`x-goog-api-client` 等不会直接透传为上游 auth；runtime 重新按 provider auth strategy 注入。
- `ProviderAuthStrategy`：
  - Anthropic API key -> `x-api-key`
  - Bearer -> `Authorization: Bearer ...`
  - Google API key -> `x-goog-api-key`
  - Google OAuth -> Bearer + `x-goog-api-client: GeminiCLI/1.0`
- converted route query 过滤 `beta=...`。
- Gemini source 转非 Gemini target 时过滤 `alt=sse` 和 `key=...`。
- target Gemini + streaming 自动补 `alt=sse`。
- provider full URL：`is_full_url=true` 或 base URL suffix `##` 时只 merge query，不追加默认 path。
- provider 级自定义请求头覆盖（升级自 issue #294 的自定义 User-Agent，对齐 axonhub 的 header override operations）：
  - provider `data.meta.customHeaders`（snake/camel 双兼容）是一个 `CustomHeaderOverride` 数组，每项 `{ op, name, value, from, to }`，`op` 取 `set / delete / rename / copy` 四种之一（其余 op 静默跳过）：
    - `set`：大小写不敏感删除转发自客户端的 `name`，再注入唯一的 `value`（即原自定义 UA 的语义，UA 预设现在以 `set User-Agent` 行插入）。
    - `delete`：大小写不敏感删除 `name`。
    - `rename`：取 `from` 的全部值，删除 `from`，再逐个追加到 `to`。
    - `copy`：取 `from` 的全部值，逐个追加到 `to`（保留 `from`）。
  - `build_upstream_headers` 在 inject 链最末尾（`inject_copilot_headers` 之后）调用 `inject_custom_headers` 逐项应用，故覆盖优先级最高；未配置时保留客户端原请求头。
  - 校验用 `parse_header_override_name`（`HeaderName::from_bytes`）与 `parse_header_override_value`（`http::HeaderValue::from_str` 字节规则：可见 ASCII / 非 ASCII / `\t` 合法，其余控制字符非法），非法 name/value 运行时静默跳过不阻断请求；前端 `headerValidation`（`isValidHeaderName` RFC 7230 token 规则 + `isValidHeaderValue` 字节规则）与之对齐并给非阻断红字提示。
  - Copilot provider 避让：`ProviderBodyCompat::Copilot` 判定命中时，凡 `name`/`from`/`to` 落入 `COPILOT_MANAGED_HEADERS`（含 `user-agent` 等指纹头）的操作整条跳过，指纹 UA（`GitHubCopilotChat/0.38.2`）由 `inject_copilot_headers` 独占管理，不可被覆盖。
  - `HeaderMap` 与 preserved vec 必须同时移除旧值，保证 header-preserving 裸客户端只写出预期条数；rename/copy 的多值用 `append_preserved_header_value`（append 语义）追加，避免 `insert` 丢值。
  - 连通性测试（`connectivity_test.rs`）走 `route_request_with_options` -> `build_upstream_headers`，自动继承 custom headers，可在表单内预验上游请求头白名单。它也覆盖 target protocol 与 CLI 原生协议一致的直通 provider，并在结果里回报 client `statusCode`/`statusText` 与真实 `upstreamStatusCode`/`upstreamUrl`（网关合成 502 时保留真实上游状态）。
  - 切换网关 profile 时 `customHeaders` 保留（`mergeGatewayProfileReferenceIntoMeta` 的 delete 白名单不含该 key），与 billing 字段同等待遇。
  - 旧 `data.meta.customUserAgent`（字符串）已下线；读取侧在 `provider_meta_from_record` 做只读回退——若 `customHeaders` 缺失而 `customUserAgent` 存在，合成一行 `set User-Agent`。前端 `getCustomHeadersFromMeta` 同样回退旧字段以便用户查看/清理。写入侧只写 `customHeaders`，且 `mergeCustomHeadersIntoMeta` 同时删除旧 `customUserAgent`/`custom_user_agent` 与 `custom_headers`，故保存后旧 key 不再落盘。

特殊 path：

- DeepSeek legacy completion -> `/beta/completions`。
- Ollama -> `/api/chat`，剥离 base URL 尾部 `/v1`。
- Anthropic Bedrock -> `/model/{model}/invoke` 或 `/invoke-with-response-stream`。
- Anthropic Vertex -> `/publishers/anthropic/models/{model}:rawPredict` 或 `:streamRawPredict`。
- Copilot -> 按本次 dynamic target 选择 `/chat/completions`、`/responses` 或 `/responses/compact`。

### 7.1 Codex 同协议 Responses WebSocket

传输与逐轮统计边界见架构主文档 §16.1；本节只约束实际上游 wire 行为。

| 条件 | 握手行为 |
|---|---|
| Gateway 聚合模式 | Codex 接管配置投影 `supports_websockets=false`，客户端在建连前选择 HTTP Responses/SSE；若仍发起 WS，Gateway 本地返回 `426`，不联系上游。聚合 WS 暂不支持，这是明确技术债。 |
| Gateway single/failover 模式且 `codex_websocket_enabled=false`（默认值，旧配置缺字段同样关闭） | 加载 provider 前本地 `426`，不发起上游 WS；在途上游握手返回后再次检查，仍关闭则不升级下游 |
| Gateway single/failover 模式且 effective target 是 Chat / Anthropic / Gemini，或 Copilot 按模型动态选协议 | 本地 `426`，不尝试上游 WS；由 Codex 发 HTTP 请求进入原转换链路 |
| Gateway single/failover 模式且原始 Codex provider 表的 `supports_websockets=false` | 本地 `426`；接管配置中的本机能力声明不覆盖此上游判断 |
| Gateway single/failover 模式且 Responses provider 的能力为 true 或未配置 | 使用实际 URL/认证发起上游握手，有效 `101` 后才升级下游 |
| Gateway single/failover 模式且上游返回非升级的 2xx/3xx，或 `404/405/426/501` | 返回 `426`，详情保留实际上游状态；重定向在 HTTP 路径处理 |
| Gateway single/failover 模式且上游返回 `401/403/429` 等真实错误 | 保留实际错误，不伪装成“不支持 WS”；沿用 retryable status / provider retry / total retry 设置，预算耗尽保留最后实际失败 |
| Gateway single/failover 模式且上游 `101` 的 Accept key、Upgrade/Connection、extensions/subprotocol 不合法 | `502`，不向下游写出 `101` |

- 下列上游握手与逐轮兼容规则仅适用于 single/failover；聚合模式在本地拒绝 WS，不进入这些上游握手步骤。路径和 query 复用既有 `build_provider_target_url`；普通 Responses Base URL、`##` RawURL 和 `is_full_url` 语义一致。HTTP(S) 用于升级请求，详情以 WS(S) URL 标识传输。
- 认证和 provider 自定义 Headers 先走 `build_upstream_headers`。随后由 WS 层重新生成 Connection/Upgrade、Sec-WebSocket-Key/Version，移除客户端 Sec-WebSocket-*、body framing、Host/Accept 等冲突字段；不请求压缩扩展或 subprotocol。客户端/provider 已给出的 OpenAI-Beta 保留，缺省才注入 `responses_websockets=2026-02-06`。
- 使用专门的全局 HTTP client builder，显式 rustls、HTTP/1.1、无重定向、无总响应超时，保留用户的 direct/system/custom proxy。连接首包、逐轮 idle、写入/flush 和服务停止分别控制生命周期，不能套用普通 HTTP 的 30 秒整次请求超时。
- 每个 `response.create` 的 model 改写复用 single/failover 规则、`[1M]` 清理及同协议 provider pipeline；去掉 HTTP 专属 `stream/background`，保留 `type/generate/stream_id/previous_response_id/event_id`。xAI native Responses namespace 恢复表按轮隔离。首版不做 WS 内协议转换、provider 切换或生成重放。
- Codex remote compaction v2 的轮次（`input[]` 含 `compaction_trigger`）同样走 WS，但上游事件被消费而非转发：按 delta 重组摘要后合成恰好一个 `compaction` item，以 5 个裸 JSON 帧发出，上游自己的 `response.completed` 丢弃。`previous_response_id` 原样保留，不替 Codex 决定 full 还是 delta。细节见 §1.6.1。
- 握手时没有模型，只能过滤 provider 级冷却；不能用猜测模型跳过渠道或更新模型健康。每轮生成的健康判定基于实际上游模型和已送达终态；合法 Incomplete/Canceled、客户端取消和预热不当作上游模型故障。
- `response.failed` / error event 仍以 WS 事件送给客户端，业务行的 HTTP status 保持空值，详情单独保留事件 error status。握手尝试只记录在连接 metadata；业务请求的尝试数不被它放大。

single/failover 模式下，网关总开关与 provider 的原始 `supports_websockets` 能力判断同时生效。关闭总开关后已有连接空闲时关闭，在途轮次继续按既有 usage/终态规则结算；开启不会解除 Codex 当前会话已经记住的 HTTP fallback，需要新会话或重启客户端再试。聚合模式独立投影 `supports_websockets=false`，不因该总开关开启而启用 WS。切换开关不影响 HTTP/SSE 的协议转换和数据脱敏设置。

关键实现：`runtime/websocket.rs`、`runtime/upstream.rs::prepare_websocket_request`、`runtime/compat/codex_remote_compaction.rs`、`provider_protocol.rs::codex_supports_websockets_from_config`、`cli_proxy/mod.rs::patch_codex_config`、`http_client.rs::client_websocket_handshake`。回归：`runtime/websocket/tests.rs`、`runtime/websocket/lifecycle_tests.rs`、`runtime/websocket/settings_tests.rs`、`settings.rs::websocket_setting_defaults_to_off_and_round_trips_without_resetting_other_settings`、`provider_protocol.rs::websocket_capability_uses_the_selected_provider_table`、`cli_proxy/mod.rs::codex_takeover_writes_mode_specific_websocket_capability_and_restores_original`、`websocket_compaction_turn_is_summarized_and_replaced`、`websocket_compaction_turn_keeps_an_incremental_delta`。

### 7.2 数据脱敏与渠道兼容（issue #347）

数据脱敏默认关闭，独立于 provider profile；没有渠道隐式默认启用。入口是网关设置页的“数据脱敏”，详细边界见 [`gateway-data-redaction.md`](gateway-data-redaction.md)。

- Claude Code、Claude Desktop、Codex、Grok、Kimi、Gemini 六类当前受支持 CLI 共用 HTTP 保护入口；single/failover 的 Codex Responses WS 也按轮接入保护。聚合模式使用 HTTP/SSE。其他工具手动调用有效模型路由时同样执行。不依赖 User-Agent，也不能把 OpenCode 等 Session 统计支持误写成自动接管支持。
- 出站请求先做 provider 的 body/header/path/auth 兼容，再在消息、工具参数/结果和说明文本里替换敏感值；认证 Header、模型/工具身份、关联 ID、媒体和不透明密文不按普通业务文本改写。业务对象里的 `name`/`url` 不能按裸字段名豁免。
- SSE、JSON 和 single/failover 同协议 Responses WS 在既有响应兼容/namespace 回转之后还原占位符。原始历史记录仍在还原之前，不将客户端还原副本写回 provider side store。
- 各协议的 JSON 文本工具结果统一解码并扫描业务字段；Responses namespace 内嵌工具的描述/schema 也参与处理，名称、namespace、关联 ID 与 Ollama `tool_name` 不变。Gemini response schema 和 Ollama `format` 中的描述/示例参与扫描，`type/format/required` 等控制信息不改。流式工具参数支持多层 JSON 和 Unicode 转义。
- Responses 工具结果 `output[]`、Gemini `functionResponse.parts` 和 Anthropic content 数组按 block 处理，图片/inlineData、Ollama `images[]`、redacted thinking 保持不透明；Anthropic 纯文本文档 data、title/context 仍扫描。
- Anthropic/Gemini 真实签名绑定对象不能静默修改；需要替换或还原时明确拒绝。Gemini functionCall 的默认兼容占位签名复用 transformer 的 `DEFAULT_GEMINI_THOUGHT_SIGNATURE` 识别，不代表真实绑定，也不豁免 thinking 文本或其他签名。Gemini thought/text/function 分通道，连续相同 delta 不能被累计前缀兼容逻辑吞掉。此约束不改现有 thinking/encrypted-content 整流开关。
- 启用隐私不改变 WS 的协议能力判定，也不会将已建立连接上的处理错误伪装成握手 426。未知正文/二进制事件和不完整占位符产生本地失败，provider 健康不扣分，已收到 usage 仍保留。
- WS 保护状态按轮次固定；关掉开关仍需拦截受保护旧轮次的重复事件。开启时，没有待处理请求的正文事件同样不能绕过关联检查。
- 本地 HTTP 隐私错误按入站协议格式化：OpenAI `error.message/type/code`、Anthropic 顶层 `type:error`、Gemini 数字 code/status；非法 JSON、请求签名拒绝和响应还原失败都保持客户端可解析的 envelope。
- 完全关闭后的新请求沿用既有兼容链路；旧上游历史不会被改写。引用过期映射或其它 provider 的 `previous_response_id` 需要新建会话。
- 回归包括 `privacy/tests.rs`、`runtime/websocket/privacy_tests.rs`、`runtime/websocket/privacy_matrix_tests.rs`：七入口 × 四种目标协议 × 开关两态 × JSON/SSE/HTTP 强制 SSE 聚合，以及本节 Ollama、legacy completion、协议错误和开启保护后的 single/failover WS 426→HTTP 往返。

## 8. 维护流程

新增或调整 provider/channel 兼容时，按以下步骤：

1. 先读 [`docs/gateway-protocol-conversion.md`](gateway-protocol-conversion.md)，确认架构边界、参考项目 baseline、查询入口和同步流程。
2. 再读本文，确认当前 provider 的触发条件、入参/出参兼容、默认行为、开关和测试。
3. 读取最近的模块级 `AGENTS.md`：
   - `tauri/src/coding/proxy_gateway/AGENTS.md`
   - `tauri/src/coding/proxy_gateway/transformer/AGENTS.md`
4. 如果改的是 profile/channel 身份，更新 `tauri/resources/gateway_provider_profiles.json`、`provider_profiles.rs` 白名单和 profile resolver 测试。
5. 如果改的是 body/header/path/auth/stream filter/rectifier，放在 runtime provider compat、middleware、side store 或 upstream 编排；不要把 provider type/base URL/API key/model catalog 传进 transformer。
6. 如果改的是 provider-agnostic 公共协议结构，才改 transformer。
7. 补最贴近用户路径的回归测试，尤其包含 custom provider 不误触发的负例。
8. 代码或测试改完后：
   - provider/channel 细节变了，必须更新本文。
   - 架构、IR、SSE 生命周期、pipeline/side store、参考项目 baseline 或同步结论变了，必须更新架构主文档。
   - 跨边界改动同时更新两份文档。
9. 只改本文时，至少运行陈旧表述搜索和 `git diff --check`，并人工核对源码路径和测试名称仍存在。

参考项目同步时，本文不单独维护 baseline commit；baseline、远端、目标 ref、查询入口和吸收日志归架构主文档。吸收后的 provider/channel wire 事实、开关和测试索引必须落到本文。

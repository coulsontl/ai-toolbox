# Transformer Module Notes

## 一句话职责

- 在 Proxy Gateway 请求路径中提供独立、可复用的 AI 协议载荷转换：Anthropic Messages、OpenAI Chat Completions、OpenAI Responses、Gemini Native 的 JSON 与 SSE 聊天协议互转。

## Source of Truth

- 转换模块的 Source of Truth 是统一中间模型新内核：`llm::Request` / `llm::Response`、Inbound/Outbound transformer、`StreamKernel`，以及 `AiProtocol`、`ConversionRoute`、`convert_request_body`、`convert_response_body`、`convert_error_response_body` 和 `convert_sse_stream` 的行为与测试。
- Runtime 只负责判断入站 route、读取 provider 的 target protocol、拼上游 path/header/auth、保存 `request_body` 与 `upstream_request_body` 快照；协议结构转换必须留在本目录。
- `ProviderGatewayMeta.apiFormat` 表示上游真实目标协议，不表示入站 CLI 协议。入站协议由 Gateway route 推导，二者组成 `ConversionRoute`。
- `apiFormat` 字符串别名的唯一解析入口是 `AiProtocol::from_api_format`。Runtime provider 读取、前端/后端是否需要 Gateway 接管的判断，以及后续新增协议都必须复用它；不要在 `provider_protocol` 或 runtime 文件里复制第二套 parser。别名要同时覆盖 snake_case、slash 和 dash 形式，例如 `anthropic/messages`、`openai/responses`、`openai-chat`。
- `source == target` 时 Gateway 必须直通，不调用结构转换；直通路径仍可做已有模型名改写、`[1M]` 标记剥离等 runtime 级处理，但不能重写协议结构。
- 本模块不能依赖数据库、Tauri app handle、provider 表、Gateway runtime context、请求日志或模型健康状态。
- SSE 转换必须边读边写，不允许为了格式转换、日志或统计先 full-buffer 整个上游流。
- `json.rs` / `streaming.rs` 是旧实现遗留文件；新开发和测试以 `kernel.rs`、`stream.rs`、各协议 transformer、`fixtures/reference/` 和 `fixtures/live_provider/` 为准，不要把旧实现重新作为 fallback。
- 不要新增全局递归 `omitempty` / `omit_empty` 清理函数替代 AxonHub Go struct tag。Rust 中间模型可以继续用 `#[serde(skip_serializing_if = ...)]` 表达可省略字段；但最终 provider 出站 JSON 多数是手工 `serde_json::Value` 构造，必须按目标协议逐字段 `if let Some(...)` / `if !items.is_empty()` 插入。原因是部分协议字段允许或需要显式 `null`，全局清理会误删合法语义。

## 支持矩阵

当前 JSON request/response/error 和 SSE stream 支持：

| source | target | 状态 |
|---|---|---|
| `AnthropicMessages` | `OpenAiChat` | 支持 |
| `OpenAiChat` | `AnthropicMessages` | 支持 |
| `AnthropicMessages` | `OpenAiResponses` | 支持 |
| `OpenAiResponses` | `AnthropicMessages` | 支持 |
| `OpenAiChat` | `OpenAiResponses` | 支持 |
| `OpenAiResponses` | `OpenAiChat` | 支持 |
| `AnthropicMessages` | `GeminiNative` | 支持 |
| `GeminiNative` | `AnthropicMessages` | 支持 |
| `GeminiNative` | `OpenAiChat` / `OpenAiResponses` | 支持 |
| `OpenAiChat` / `OpenAiResponses` | `GeminiNative` | 支持 |

任何新增协议或新增聊天载荷能力都必须补全 request/response/SSE/error 的矩阵测试；不能只支持单向或只支持 JSON。

## 参考实现对照结论

- 参考实现使用统一 `llm.Request` / `llm.Response` 中间模型，覆盖 chat、responses、compact、embedding、image、video、rerank 等更大范围；本模块只处理 Gateway CLI 代理需要的聊天协议转换。
- 参考实现有 provider signature marker/footprint 机制，用于跨渠道同会话切换时保留 Anthropic thinking signature、Gemini thoughtSignature、OpenAI Responses encrypted_content。AI Toolbox 当前实现支持本模块内 provider-local signature 生命周期：同 provider JSON/SSE roundtrip 保真，跨 provider 默认不泄漏、不伪造。
- 本模块没有会话级 footprint/runtime 存储，不能跨请求保存或迁移 provider 私有签名；`shared/signature.rs` 的 marker 只用于标记来源和防止错还原。未标记历史值只在 heuristic 能明确识别来源时恢复；`Unknown` 不恢复到任何 provider。
- 当前实现映射可公开互通的 reasoning 文本：OpenAI `reasoning_content` / Responses reasoning summary / Anthropic `thinking` / Gemini `thought:true` text；同时 provider-local 保留 Anthropic `thinking.signature`、Anthropic `redacted_thinking`、OpenAI Responses `encrypted_content`、Gemini `thoughtSignature`。
- 参考实现的 stream transformer 对 tool call、reasoning、finish reason、usage、error event 都有状态机；本模块已对当前支持协议补齐对应的轻量状态机，但保持无 DB、无会话存储、无全局影子状态。
- 参考 fixture 已复制到 `fixtures/reference/{anthropic,openai_chat,openai_responses,gemini}/`。自动化测试必须对复制进来的 118 个 fixture 全部分类；当前聊天内核 supported 子集为 35 个 request、34 个 response、43 个 stream fixture，并全部转到所有非 identity target。`*.aggregator.json`、`*.stream.response.json`、`gemini/gemini-thought.jsonl` 属辅助语料；Responses compact、image generation、embedding、video 等非聊天能力作为明确 out-of-scope 或后续长期项保留。Responses encrypted-only reasoning 已按聊天 signature 生命周期覆盖。
- 全量参考 fixture 矩阵主要防止 panic、解析漂移和 shape 回退；关键协议语义还必须补精确断言。目前已锁定 system/instructions/systemInstruction、base64 image、stop sequences、tool_choice、tool schema strict、多 Anthropic `tool_result` 与同消息后续文本、tool_result cache/is_error、tool_result -> Gemini functionResponse name/id、OpenAI reasoning/reasoning_content、Responses custom tool JSON/SSE、Gemini request-level `thinkingConfig`、Gemini native Google tools、Gemini schema type 归一化、Gemini thoughts usage、Gemini thought text、Responses function_call arguments.done 完整参数、finish 幂等、Chat finish `delta:{}` 和 Gemini stream 不输出 `[DONE]`。
- 参考实现支持 Responses compact 与 custom tool；本模块不扩展 compact 协议。Responses custom tool 在聊天 request/response/stream 内通过 Chat 兼容扩展 `responses_custom_tool` 与 `response_custom_tool_call` 保留 call_id/name/input/output；转换到没有 custom tool 原生形态的目标协议时只能 best-effort 表达为普通 tool call。

## 8 个转换节点对照索引

这些节点来自 `docs/transform/01..08` 的 AxonHub 对照结果。后续排查协议转换时先查本节确认当前实现语义，再按需打开对应 review 文档看原始证据。若本节与旧 review 文档冲突，以当前代码、测试和本节为准。

### 01 OpenAI Chat -> LLM

- 入口：`openai/chat.rs::chat_request_to_llm`、`chat_response_to_llm`，stream source 为 `OpenAiChat`。
- Request 已承载 `model`、`messages`、`max_tokens` / `max_completion_tokens`、采样参数、penalty、`seed`、`service_tier`、`logprobs` / `top_logprobs`、`user`、`logit_bias`、`verbosity`、`stream` / `stream_options`、`stop`、`tool_choice`、`tools`、`parallel_tool_calls`、`response_format`、`prompt_cache_key`、`metadata`、`extra_body`。
- 入站 request 必须设置 `request_type=chat` 与 `api_format=openai_chat_completions`。
- Message 支持 string/array content、text、`image_url`、`name`、`refusal`、`annotations`、tool message 的 `tool_call_id`。
- `reasoning_content` 与 `reasoning` 入站要双向同步；只有其中一个字段时也要进入统一 reasoning。
- `tool_calls` 与 legacy `function_call` 都要转统一 tool call；Responses custom tool 可通过 Chat 兼容扩展 `responses_custom_tool` 保留 roundtrip 语义。
- Response 必须保留所有 choices，不再只取首个 choice；usage 支持 prompt/completion/total、cached tokens、reasoning tokens。
- 不承载 `store`、`safety_identifier`、`modalities`、audio/video、`system_fingerprint`、response `service_tier`、citations、logprobs 明细。OpenAI Chat JSON 没有本模块 owns 的 provider-private signature 字段；Chat SSE 中若出现 `reasoning_signature`，只进入统一 signature event，由目标 provider 按 marker/heuristic 判定是否恢复。
- HTTP content type/body/model/messages 校验不在本模块；由 Gateway runtime/route 层处理。

### 02 OpenAI Responses -> LLM

- 入口：`openai/responses/mod.rs::responses_request_to_llm`、`responses_response_to_llm`，stream source 为 `OpenAiResponses`。
- Request 已承载 `model`、`max_output_tokens`、`reasoning.effort`、采样参数、penalty、`service_tier`、`top_logprobs`、`user`、text verbosity、`stream`、`previous_response_id`、`stop`、`tool_choice`、`parallel_tool_calls`、`text.format`、`prompt_cache_key`、`metadata`、`extra_body`。
- 入站 request 必须设置 `request_type=chat` 与 `api_format=openai_responses`。
- `instructions` 转 system message；`input` string/object/array 都要进入 messages。
- `message.content` 支持 `input_text`、`output_text`、`input_image`、`refusal`，annotations 从 content 中提取；standalone `input_image` item 要转 user image message。
- `function_call` / `function_call_output`、`custom_tool_call` / `custom_tool_call_output` 都要保留，custom tool 用 `ResponseCustomToolCall` 保留 call_id/name/input。
- `reasoning` item 要转 assistant reasoning message，并尝试和紧随其后的 function/custom tool/message 合并到同一 assistant message。
- `encrypted_content` 必须通过 `encode_signature(OpenAiResponses, ...)` 保存到 `reasoning_signature`；转回 Responses 时通过同 provider marker/heuristic 还原，转 Anthropic/Gemini 时不能泄漏。
- Response 已承载 output message text/refusal/annotations、function/custom tool call、reasoning、usage、`created_at` / `created`、`previous_response_id`、status finish；`failed -> error`、`incomplete -> length`、tool call completed -> `tool_calls`。
- `include` 不做语义扩展，但必须保留显式入站的 top-level `include` 或 `extra_body.include`，尤其是 `reasoning.encrypted_content`；不主动新增 include。
- 不承载 `store`、`safety_identifier`、background/conversation、`max_tool_calls`、`prompt_cache_retention`、`truncation`、`include_obfuscation`、compact、image generation。

### 03 Anthropic Claude Messages -> LLM

- 入口：`anthropic/inbound.rs::anthropic_request_to_llm`、`anthropic_response_to_llm`，stream source 为 `AnthropicMessages`。
- Request 已承载 `model`、`max_tokens`、`temperature`、`top_p`、`stream`、`stop_sequences`、`tool_choice`、`thinking` / `output_config.effort` 到 `reasoning_effort`。
- 入站 request 必须设置 `request_type=chat` 与 `api_format=anthropic_messages`；`metadata.user_id` 要进入统一 `metadata["user_id"]`。
- `system` 转 system message；string/array content 都支持。
- Content 支持 text、base64 image、URL image；缺失 `media_type` 时按 AxonHub 使用 `application/octet-stream`，不默认 `image/png`。
- `thinking` block 同步写入 `reasoning_content` 与 `reasoning`，`signature` 必须通过 `encode_signature(Anthropic, ...)` 保存到 `reasoning_signature`；转回 Anthropic 时还原，转其他 provider 时丢弃。
- `redacted_thinking` 在 Anthropic provider-local roundtrip 中保留到 `redacted_reasoning_content` 并转回 `redacted_thinking` block；不能转给 OpenAI/Gemini。
- Anthropic native web_search、顶层/system `cache_control` 完整生命周期不在当前聊天转换范围；content part/tool result `cache_control` 已覆盖。
- `tool_use` 转统一 tool call；`tool_result` 转 tool message，支持 string/array content、`is_error`、`cache_control`。
- 同一 Anthropic user message 中多个 `tool_result` 与后续 text/image 必须通过 message index metadata 支持转回 Anthropic 时重新合并。
- `BatchTool` 转非 Anthropic 目标时过滤；`tool_choice:any` 转 required，named tool choice 支持。
- Response 支持 text、thinking、tool_use、stop reason、usage cache read/cache creation；SSE 覆盖 text/thinking/tool use/usage/finish 幂等。
- HTTP content type、body、`max_tokens > 0`、枚举合法性、Bedrock/Vertex/LongCat 等平台差异不在本模块。

### 04 Gemini Native -> LLM

- 入口：`gemini/mod.rs::gemini_request_to_llm`、`gemini_response_to_llm`，stream source 为 `GeminiNative`。
- Request 已承载 body `model`、`stream`、`generationConfig.maxOutputTokens`、temperature、`topP`、presence/frequency penalty、`seed`、`stopSequences`。
- 入站 request 必须设置 `request_type=chat` 与 `api_format=gemini_contents`；Gemini path model/action 提取属于 runtime，不在本模块复刻。
- `generationConfig.thinkingConfig` 转 `reasoning_effort`：`budget<=0 -> none`、`<=1024 -> low`、`<=8192 -> medium`、更高为 high；`includeThoughts=false` 和 `thinkingLevel` 也要处理。
- `responseMimeType` / `responseSchema` / `responseJsonSchema` 转 `response_format`。
- `systemInstruction.parts` 只取非 `thought:true` 文本，并用 `\n` 连接；role `model` 转 assistant，缺省 user。
- Text part 支持；`thought:true` text 同步写入 `reasoning_content` 与 `reasoning`，`thoughtSignature` 必须通过 `encode_signature(Gemini, ...)` 保存到 `reasoning_signature`。
- `inlineData` / `fileData` 图片进入 image URL；document 的 inline/file data 进入 `DocumentUrl` 基础映射。video/audio、`responseModalities`、`topK`、logprobs、`safetySettings`、`cachedContent`、`imageConfig` 不承载。
- `functionCall` 转 tool call，缺 id 时生成 `gemini_synth_<index>`；`functionCall.thoughtSignature` 或 part-level `thoughtSignature` 存入 tool call `transformer_metadata["gemini_thought_signature"]`，转回 Gemini 时优先恢复到同一个 functionCall part，不能移动到错误 tool。
- `functionResponse` 转 tool message，缺 id 时可从当前请求历史 function call name 回填。
- Function declarations 支持，schema type 要递归小写化；Google native tools `googleSearch`、`codeExecution`、`urlContext` 要保留。
- `toolConfig.functionCallingConfig.allowedFunctionNames` 只有在 `mode:"ANY"` 下生效：单个 allowed 转 named，多个 allowed 转 required；`AUTO` 即 auto，`NONE` 即 none。
- Response 支持 prompt block refusal、所有 candidates -> choices、text、thought text、functionCall、finish reason、usage thought tokens。
- Gemini stream 累计文本按前缀差值输出；thought part 和 functionCall part 的 `thoughtSignature` 按 provider-local marker 生命周期保留，转非 Gemini target 时不泄漏。

### 05 LLM -> OpenAI Chat

- 入口：`openai/chat.rs::llm_request_to_chat`、`llm_response_to_chat`，stream target 为 `OpenAiChat`。
- Request 输出 `model`、`messages`、token 字段、采样参数、penalty、`seed`、`service_tier`、logprobs、`user`、`logit_bias`、`verbosity`、`reasoning_effort`、`stream` / `stream_options`、`stop`、`tool_choice`、`tools`、`parallel_tool_calls`、`response_format`、`prompt_cache_key`、`extra_body`；`metadata` 只在来源也是 OpenAI Chat/Responses 时作为 OpenAI 原生字段透传，Anthropic `metadata.user_id` 只用于转回 Anthropic，不能泄漏到 OpenAI 目标。
- Token 字段按当前兼容策略处理：o-series/GPT-5 类模型输出 `max_completion_tokens`，其他模型输出 `max_tokens`。
- system/developer/user/assistant/tool roles 都可输出；content 支持 text 和 `image_url`，不承载 video/audio。
- `reasoning_content` 当前固定输出用于 reasoning 互通；没有 AxonHub 的 channel-level `ReasoningField` 配置，也不会默认剥离 reasoning。
- Function tools 支持；Responses custom tool 兼容扩展会为 roundtrip 保留，但严格 OpenAI Chat provider 可能不接受该扩展。
- 无最终 tools 时清理 `tool_choice` / `parallel_tool_calls` 属于 Gateway runtime outbound adapter 兼容，不属于纯协议结构转换。
- Response 输出所有 choices、message、finish_reason、usage。
- Chat SSE finish chunk 必须包含 `delta:{}`，并用 `[DONE]` 结束；纯 signature chunk 必须跳过，不能输出空 content/tool chunk 或泄漏 provider-private signature。
- 不承载 `store`、`safety_identifier`、modalities、audio/video。

### 06 LLM -> OpenAI Responses

- 入口：`openai/responses/mod.rs::llm_request_to_responses`、`llm_response_to_responses`，stream target 为 `OpenAiResponses`。
- Request 输出 `model`、`input`、`instructions`、`max_output_tokens`、temperature、`top_p`、penalty、`service_tier`、`top_logprobs`、`user`、`reasoning.effort`、`stream`、`stop`、`tool_choice`、`tools`、`parallel_tool_calls`、`text.format` / verbosity、`prompt_cache_key`、`extra_body`；`metadata` 只在来源也是 OpenAI Chat/Responses 时作为 OpenAI 原生字段透传，Anthropic `metadata.user_id` 只用于转回 Anthropic，不能泄漏到 Responses target。
- `input` 当前统一输出 array，不保留 AxonHub 的 single string input optimization。
- system/developer 合并为 `instructions`；user/assistant text/image/refusal/annotations 输出为 message content。
- Assistant reasoning 输出为 reasoning item；function/custom tool call 与 output 都支持，custom output 通过当前 request 内 call id 判断 item type。
- 无最终 tools 时清理 `tool_choice` / `parallel_tool_calls` 属于 Gateway runtime outbound adapter 兼容，不属于纯协议结构转换。
- Tool call item 必须输出 `status:"completed"`。Responses `function_call.id` 是 item id，必须使用 `fc*` 形态；custom tool item id 必须使用 `ctc*` 形态；原始工具调用 id 保留在 `call_id`，不要把 Anthropic/Chat 的 `call_*` 直接写进 Responses item `id`。
- Response 输出 reasoning、message、refusal、tool calls、usage、status、`created_at`、`previous_response_id`。
- `response_format` json_schema wrapper 与 Responses `text.format` 双向转换。
- Strict schema normalize 不在当前实现中自动补 `additionalProperties:false` / required；只透传 schema。
- Responses SSE 覆盖 text、reasoning、function tool、custom tool、finish，事件序列比 AxonHub 简化但要保证客户端关键事件和完成幂等。
- `encrypted_content` 从 `reasoning_signature` 中仅按 OpenAI Responses provider marker/heuristic 还原；允许 encrypted-only reasoning item，summary 为空时仍输出 reasoning item。
- `include` 只保留显式入站/extra_body 的 top-level include，不由转换器主动添加。
- 不承载 `store`、`safety_identifier`、`max_tool_calls`、`prompt_cache_retention`、`truncation`、`stream_options.include_obfuscation`、compact、image generation。

### 07 LLM -> Anthropic Claude Messages

- 入口：`anthropic/outbound.rs::llm_request_to_anthropic`、`llm_response_to_anthropic`，stream target 为 `AnthropicMessages`。
- Request 输出 `model`、`messages`、`system`、`max_tokens`、`thinking`、temperature、`top_p`、`stream`、`stop_sequences`、`tool_choice`、`tools`。
- `max_tokens` 缺失时默认输出 `8192`，避免 Anthropic target 缺必填字段。
- `metadata["user_id"]` 要输出到 Anthropic `metadata.user_id`。
- 无最终 tools 时清理 `tool_choice` 属于 Gateway runtime outbound adapter 兼容，不属于纯协议结构转换。
- URL/header/auth/Bedrock/Vertex/LongCat 平台差异不在本模块，由 Gateway runtime target protocol/header/auth 决策负责。
- `ReasoningEffort` 可转 Anthropic thinking budget，`none` 转 disabled；不承载 `ReasoningBudget`、thinking display/adaptive、output_config metadata。
- user/assistant text、image data URL -> base64 source、普通 image URL -> URL source 都支持。
- Tool call 转 `tool_use`；tool result messages 聚合为 user `tool_result`，同一 Anthropic 原始 user message 的 tool_result + 后续文本可根据 message index 合并。
- `is_error` 和 tool_result `cache_control` 支持；tool arguments 解析失败 fallback `{}`，没有 AxonHub 的 safe JSON repair。
- `redacted_thinking` 仅在来源为 Anthropic provider-local 数据时输出；Anthropic native web_search 不输出。
- `thinking.signature` 仅从 Anthropic marker/heuristic 还原，不生成 fake signature，也不能把 Gemini/OpenAI 私有签名写入 Anthropic signature。
- Response 输出 thinking/text/image/tool_use、stop reason、usage。
- Anthropic target SSE 必须保证 `content_block_start`、delta、`content_block_stop` 顺序完整，并保证 finish 幂等。

### 08 LLM -> Gemini Native

- 入口：`gemini/mod.rs::llm_request_to_gemini`、`llm_response_to_gemini`，stream target 为 `GeminiNative`。
- Request 输出 `contents`、`systemInstruction`、`generationConfig`、`toolConfig`、`tools`。
- `max_tokens` / `max_completion_tokens`、temperature、`top_p`、presence/frequency penalty、`seed`、`stopSequences` 支持。
- `reasoning_effort` 输出 `thinkingConfig`，支持 none/low/medium/high/xhigh。
- `thoughtSignature` 仅从 Gemini marker/heuristic 或 per-tool metadata 还原；当 Gemini target 存在 reasoning thought 或 functionCall 但没有有效 Gemini signature 时，按 AxonHub 兼容策略补 `DEFAULT_GEMINI_THOUGHT_SIGNATURE` 到第一条适用 thought/functionCall part。不能把 Anthropic/OpenAI 私有签名写入 Gemini `thoughtSignature`。
- `response_format` json_schema/json_object 输出 `responseMimeType` / `responseSchema`。
- system/developer 输出 `systemInstruction`；user/assistant/tool role 映射。
- Text 和 reasoning thought text 支持；image data URL -> `inlineData`，普通 image URL -> `fileData.fileUri`，document data URL / regular URL -> `inlineData` / `fileData`。
- video/audio、modalities、`imageConfig`、`safetySettings`、`topK`、logprobs、`responseLogprobs` 不承载。
- Tool call 输出 `functionCall`；tool result 输出 `functionResponse`，并可根据前序 tool call id 找 name。
- Function declarations、`parametersJsonSchema`、Google native tools 支持；tool choice `NONE` / `ANY` / `allowedFunctionNames` 支持。
- Response 输出所有 choices -> candidates，支持 text/thought/tool call/finish/usage。
- Gemini target stream 不输出 OpenAI `[DONE]`，通过 Gemini finish chunk 完成。

## JSON 请求转换细节

- Anthropic `system` 转 OpenAI Chat `system` message，转 Responses `instructions`，转 Gemini `systemInstruction.parts[].text`。
- OpenAI Chat `system` 和 `developer` 都汇总到 Anthropic `system` 或 Responses `instructions`，顺序保留，用空行连接。
- Anthropic `messages[].content` 支持 string 和 block array；OpenAI/Gemini 转入时统一输出 Anthropic block array。
- 文本映射：
  - Anthropic `text` <-> Chat text / Responses `input_text`、`output_text` / Gemini `parts[].text`。
- 图片/文档映射：
  - Anthropic base64 `image` 转 Chat `image_url` data URL、Responses `input_image`、Gemini `inlineData`。
  - Chat/Responses data URL image 转 Anthropic `image.source`。
  - Gemini `inlineData` 转统一 data URL 媒体内容；当前目标协议出站只保证图片类 data URL 的互通，不实现完整 document/audio/video 生命周期。
- 工具定义映射：
  - Anthropic `tools[].input_schema` <-> Chat `tools[].function.parameters` <-> Responses `tools[].parameters` <-> Gemini `functionDeclarations[].parameters`。
  - Anthropic `BatchTool` 在转 OpenAI/Gemini 时过滤。
  - Responses `custom` tool <-> Chat 兼容扩展 `responses_custom_tool`；不要把它因为带有空 `function` 占位就退化成普通 function tool。
  - Gemini native `googleSearch` / `codeExecution` / `urlContext` 保留为中间模型 Google tools；转回 Gemini 时继续输出对应 native tool object。
  - Gemini SDK 可能给 schema `type` 返回 `OBJECT` / `STRING` 等大写值；入站必须递归归一为小写，避免转 OpenAI/Anthropic 后 schema 不合法。
- 工具选择映射：
  - Anthropic `any` <-> OpenAI/Responses `required`。
  - Anthropic `{type:"tool", name}` <-> Chat `{type:"function", function:{name}}` <-> Responses `{type:"function", name}` <-> Gemini `allowedFunctionNames`。
- 工具调用与工具结果：
  - Anthropic `tool_use` <-> Chat `tool_calls` / legacy `function_call` <-> Responses `function_call` <-> Gemini `functionCall`。
  - Anthropic `tool_result` <-> Chat `role:"tool"` <-> Responses `function_call_output` <-> Gemini `functionResponse`。
  - Anthropic 单条 user message 内允许多个 `tool_result` 和后续普通 text/image；入站不能在第一个 tool_result 处提前返回，出站应把连续 tool results 合并回同一个 Anthropic user content，保留 `cache_control` / `is_error`。
  - Responses `custom_tool_call` / `custom_tool_call_output` 必须和 Chat 兼容扩展 `responses_custom_tool` 双向保真；同一 request 内用前序 custom call id 判断后续 tool output 类型，不做跨请求影子状态。
  - Gemini 缺少 functionCall id 时生成 `gemini_synth_<index>`；转回 Gemini 时不会把这个 synthetic id 作为真实 id 发上游。
  - Gemini `functionResponse.name` 和缺失的 id 通过同一请求里的历史 functionCall 做 best-effort 补全；没有历史时用 id/name fallback。不做跨请求影子状态。
- Reasoning 映射：
  - Chat `reasoning` / `reasoning_content`、Responses `reasoning.summary[].text`、Anthropic `thinking`、Gemini `thought: true` 文本互转。
  - Anthropic 顶层 `thinking` / `output_config.effort` 转 OpenAI Chat `reasoning_effort` 或 Responses `reasoning.effort`；反向转 Anthropic 时用 `reasoning_effort` 生成 `thinking` 配置。
  - Runtime 的 thinking rectifier 不应在正常转换前删除 `thinking` 或 `output_config.effort`；它只在上游 4xx thinking/signature 兼容错误后重建并重试一次请求。协议转换层继续按 source payload 显式字段做 reasoning 映射。
  - Gemini `generationConfig.thinkingConfig` 转 `reasoning_effort`；反向转 Gemini 时用 `reasoning_effort` 生成 `thinkingConfig`，并保持 `includeThoughts` 与 `thinkingLevel`/budget 语义。
  - Anthropic `thinking.signature`、Anthropic `redacted_thinking`、Responses `encrypted_content`、Gemini `thoughtSignature` 支持 provider-local 生命周期：同 provider roundtrip 保真，跨 provider 不泄漏、不伪造。
  - Gemini `thoughtSignature` 可存在于 thought text part 或 functionCall part；functionCall signature 必须通过 per-tool metadata 还原到原 tool call，不得挪到第一条 tool call。
  - OpenAI Responses encrypted-only reasoning item 必须保留；没有 summary 文本时也要输出带 `encrypted_content` 的 reasoning item。
- 参数映射：
  - Anthropic `max_tokens` -> Chat `max_tokens` 或 o/GPT-5 系列 `max_completion_tokens`，-> Responses `max_output_tokens`，-> Gemini `generationConfig.maxOutputTokens`。
  - Chat `max_completion_tokens` / `max_tokens` -> Anthropic `max_tokens` / Responses `max_output_tokens`。
  - Responses `max_output_tokens` -> Anthropic `max_tokens` / Chat `max_tokens`。
  - `temperature`、`top_p`、`stream` 按目标协议保留；stop 在 Anthropic 使用 `stop_sequences`，OpenAI/Responses 使用 `stop`，Gemini 使用 `stopSequences`。
  - OpenAI Chat `response_format` 与 Responses `text.format`、Gemini `generationConfig.responseMimeType/responseSchema` 互转；Anthropic 目标没有等价 JSON schema 字段，不伪造约束。
  - OpenAI Chat / Responses 支持的请求级 pass-through 字段必须显式接线，至少包括 `parallel_tool_calls`、`prompt_cache_key`、`metadata`、`service_tier`、`frequency_penalty`、`presence_penalty`、`top_logprobs`、`user`、`verbosity`、`logprobs`、`logit_bias`、`seed`。其中 `metadata` 只在 OpenAI source 之间透传；Anthropic `metadata.user_id` 不等同于 OpenAI `metadata`，不能发往 OpenAI target。
  - `extra_body` 只作为显式字段读写；不要把未知顶层字段自动合并到目标协议 body，避免把 source-only 参数误发给不支持的 provider。
- OpenAI stream request 转 Chat target 时必须补 `stream_options.include_usage=true`，避免流式 usage 丢失。

## JSON 响应转换细节

- Anthropic response 转 Chat：
  - `text` 合并为 assistant `message.content`。
  - `thinking` 写入 `reasoning_content`。
  - `tool_use` 写入 `tool_calls`。
  - `stop_reason` 映射为 `finish_reason`：`end_turn/stop_sequence -> stop`，`max_tokens -> length`，`tool_use -> tool_calls`。
- Chat response 转 Anthropic：
  - `reasoning_content` 转 `thinking`。
  - `content` 转 `text`。
  - `tool_calls` / `function_call` 转 `tool_use`。
  - 有 tool call 时 `finish_reason` / missing finish 可推导为 `tool_use`。
- Responses response 转 Anthropic/Chat：
  - `output[].message.content[].output_text` 转文本。
  - `refusal` 作为文本块保留，stop reason 不强行改写，除非 Responses status/finish 信息明确。
  - `annotations` / URL citations 作为 OpenAI message metadata 保留到支持该字段的 Chat target。
  - `function_call` / `custom_tool_call` 转 Anthropic `tool_use` 或 Chat `tool_calls`。
  - `reasoning.summary[].text` 转 Anthropic `thinking` 或 Chat `reasoning_content`。
  - `status=completed` 且有 tool call 时映射为 Anthropic `tool_use` / Chat `tool_calls`；`status=incomplete` 映射为 Anthropic `max_tokens` / Chat `length`。
- Gemini response 转 Anthropic：
  - `promptFeedback.blockReason` 生成 refusal 文本并设置 `stop_reason=refusal`。
  - `candidates[0].content.parts[].text` 转 Anthropic text。
  - `functionCall` 转 Anthropic `tool_use`。
  - `finishReason`：`MAX_TOKENS -> max_tokens`，`SAFETY/RECITATION/SPII/BLOCKLIST/PROHIBITED_CONTENT -> refusal`，有 tool call 时 `tool_use`，其他默认 `end_turn`。
- Anthropic response 转 Gemini：
  - text/tool_use 映射到 Gemini `parts[].text` / `functionCall`。
  - usage 映射到 `usageMetadata`，finish 映射到 Gemini `STOP` / `MAX_TOKENS` / `SAFETY`。
- Usage 映射：
  - OpenAI prompt/input tokens 转 Anthropic `input_tokens`；cached tokens 转 `cache_read_input_tokens`。
  - Anthropic `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` 转 Gemini/OpenAI prompt。
  - Responses `input_tokens_details.cached_tokens` 会从 Anthropic `input_tokens` 中扣出，避免缓存 token 被重复计入非缓存输入。
  - Gemini `promptTokenCount` 扣除 `cachedContentTokenCount` 后写 Anthropic `input_tokens`；`thoughtsTokenCount` 计入统一 `completion_tokens` 和 `reasoning_tokens`，转回 Gemini 时 `candidatesTokenCount = completion_tokens - reasoning_tokens`；`totalTokenCount` 优先按上游值保留。

## SSE 转换细节

- SSE parser 支持 `\n\n` 和 `\r\n\r\n`，支持 UTF-8 chunk 边界跨包，忽略空 data 和无效 JSON data。
- OpenAI Chat -> Anthropic：
  - `delta.content` -> `content_block_start(text)` + `text_delta`。
  - `delta.reasoning_content` / `delta.reasoning` -> `content_block_start(thinking)` + `thinking_delta`。
  - `delta.reasoning_signature` 只有识别为 Anthropic signature 时才转 `signature_delta`；`signature_delta` 必须在 thinking block `content_block_stop` 前输出。signature-only 流必须生成 synthetic thinking block：start -> signature_delta -> stop。
  - `delta.tool_calls[].function.arguments` -> `content_block_start(tool_use)` + `input_json_delta`。
  - `[DONE]`、finish chunk 重复出现时只输出一组 `message_delta` + `message_stop`。
- Anthropic -> Chat：
  - `message_start` -> Chat role delta。
  - `text_delta` -> `delta.content`。
  - `thinking_delta` -> `delta.reasoning_content`。
  - `signature_delta` 进入统一 signature event；Chat target 必须忽略，不能输出空 chunk 或原始 signature。
  - `tool_use` start/delta -> Chat `tool_calls` name/id/arguments 增量。
  - `message_delta.stop_reason` -> Chat `finish_reason`。
  - `message_stop` 走统一 finish，避免重复 `[DONE]`。
- OpenAI Chat -> Responses：
  - `delta.content` -> `response.output_text.delta`。
  - `delta.reasoning_content` -> `response.reasoning_summary_text.delta`。
  - `delta.tool_calls` -> `response.output_item.added(function_call)` + `response.function_call_arguments.delta`。
  - Chat 兼容扩展 `responses_custom_tool` -> `response.output_item.added(custom_tool_call)` + `response.custom_tool_call_input.delta/done`。
  - finish -> `response.completed`。
- Responses -> Chat：
  - `response.created` -> Chat role delta。
  - `response.output_text.delta` -> Chat content delta。
  - `response.reasoning_summary_text.delta` -> Chat `reasoning_content` delta。
  - `response.output_item.added/done` 中 reasoning 的 `encrypted_content` 进入统一 signature event；Chat target 必须忽略。
  - `response.output_item.added(function_call)` + `response.function_call_arguments.delta` -> Chat `tool_calls` delta。
  - `response.output_item.added(custom_tool_call)` + `response.custom_tool_call_input.delta/done` -> Chat 兼容扩展 `responses_custom_tool` delta。
  - `response.completed` -> Chat finish + `[DONE]`，有 tool call 时 finish reason 为 `tool_calls`。
- Anthropic -> Responses：
  - `text_delta` -> `response.output_text.delta`。
  - `thinking_delta` -> `response.reasoning_summary_text.delta`。
  - `signature_delta` 来自 Anthropic 私有 signature，不能转成 Responses `encrypted_content`。
  - `tool_use` start/delta/stop -> Responses function_call item added / arguments delta / arguments done / output item done。
  - `message_stop` -> `response.completed`。
- Responses -> Anthropic：
  - `response.output_text.delta` -> Anthropic text block。
  - `response.reasoning_summary_text.delta` -> Anthropic thinking block。
  - reasoning `encrypted_content` 来自 OpenAI Responses 私有 signature，不能转成 Anthropic `signature_delta`。
  - function_call item/delta -> Anthropic tool_use block + input_json_delta。
  - `response.completed` 有 tool call 时 stop reason 为 `tool_use`，否则 `end_turn`。
- Gemini -> Anthropic：
  - Gemini stream chunks 可能发送累计文本，本模块按前缀差值输出 Anthropic `text_delta`。
  - Gemini `thoughtSignature` 不能转成 Anthropic `signature_delta`。
  - `functionCall` 在 finish 时输出 Anthropic tool_use block；缺 id 时使用 synthetic id。
  - blocked prompt 在 finish 时输出 refusal 文本。
- Anthropic -> Gemini：
  - `text_delta` 直接输出 Gemini SSE chunk。
  - Anthropic `signature_delta` 不能原样转成 Gemini `thoughtSignature`；Gemini target 只接受 Gemini marker/heuristic，必要时补默认 Gemini thought signature。
  - `tool_use` start/delta/stop 累计 JSON 参数后输出 Gemini `functionCall`。
  - `message_delta.usage` 转 Gemini `usageMetadata`；`message_stop` 输出 finish chunk。
- Responses -> Gemini：
  - Responses reasoning `encrypted_content` 不能转成 Gemini `thoughtSignature`。
  - Gemini target 存在 reasoning/tool call 但无有效 Gemini signature 时，按默认 signature 策略补第一条适用 part。
  - Gemini target 只有 signature、没有 reasoning text 或 tool call 时不能生成空 thought part；finish-only signature 要丢弃。
- Gemini -> Responses：
  - Gemini `thoughtSignature` 不能转成 Responses `encrypted_content`。

## Error 转换细节

- `convert_error_response_body` 只在 body 是 JSON 且能提取 message 时转换；非 JSON 或无法识别 error shape 时原样返回。
- OpenAI/Responses target 使用 `{error:{message,type,param,code}}`。
- Anthropic target 使用 `{type:"error", error:{type,message}}`。
- Gemini target 使用 `{error:{code,message,status}}`，并按常见 error type 映射 HTTP-like code/status。

## 非目标范围

- 不处理 embedding、image generation、video、rerank、OpenAI Responses compact。
- 不做跨请求工具名影子存储。Gemini functionResponse 的 name 只从当前请求已有 tool_use/tool_result 关系 best-effort 推导。
- 不实现跨请求/会话级 signature footprint 存储；本模块只实现 provider-local marker 生命周期。未来如果需要跨请求 footprint，应由 runtime/session 层通过 `transformer_metadata` 显式注入，本模块不能主动依赖 DB、provider 表或 Gateway runtime context。
- 不在本模块处理上游 URL、query、header、auth、model mapping、`[1M]` URL 段剥离、request logging、usage cost、provider failover。

## 测试覆盖矩阵

- `cargo test transformer::kernel`
  - `request_conversion_covers_all_non_identity_protocol_routes`：4 个协议两两非 identity 的 12 条 request route。
  - `response_conversion_covers_all_non_identity_protocol_routes`：4 个协议两两非 identity 的 12 条 response route。
  - `sse_conversion_covers_all_non_identity_protocol_routes`：4 个协议两两非 identity 的 12 条 SSE route，覆盖 text、reasoning、tool_call、finish、UTF-8/SSE 分块。
  - `reference_request_fixtures_convert_to_all_targets`：参考 simple/tool/thinking request fixture 转所有非 identity target。
  - `reference_response_fixtures_convert_to_all_targets`：参考 stop/tool/thinking response fixture 转所有非 identity target。
  - `reference_stream_fixtures_convert_to_all_targets`：参考 `{Type,Data}` JSONL stream fixture 转标准 SSE 后再转所有非 identity target。
  - `reference_all_copied_fixtures_are_classified`：复制进来的 118 个参考 fixture 必须全部进入 supported/auxiliary/out-of-scope 分类，防止新增语料被静默忽略。
  - `reference_all_supported_request_fixtures_convert_to_all_targets`：35 个 supported request fixture 全部转所有非 identity target。
  - `reference_all_supported_response_fixtures_convert_to_all_targets`：34 个 supported response fixture 全部转所有非 identity target。
  - `reference_all_supported_stream_fixtures_convert_to_all_targets`：43 个 supported stream fixture 全部转标准 SSE 后再转所有非 identity target。
  - `reference_*_semantics_*` 与精确回归测试：从参考实现测试翻译来的关键断言，覆盖 stop/tool_choice、图片、工具结果、reasoning、Anthropic 混合 tool_result、Responses custom tool、Gemini thinkingConfig/native tools/schema/usage、stream tool argument accumulation 与 finish 幂等。
  - `live_provider_response_fixtures_convert_to_all_targets`：真实 provider HTTP 200 响应样本转所有非 identity target，覆盖 OpenAI Chat、OpenAI Responses、Anthropic Messages、Gemini Native 的真实响应 shape、reasoning、finish/status 和 usage 边界。
- `cargo test transformer` 是本模块当前推荐的局部验证命令；它包含 `kernel` 测试和编译边界。

## 回归测试规则

- 以后任何协议转换问题，无论来自开发自测、review、真实 provider 验证还是用户反馈，都必须在同一任务内补一个最贴近失败模式的回归测试；没有测试不能宣称修复完成。
- 外部 provider 返回 shape 导致的问题优先沉淀为 `fixtures/live_provider/` 或更小的脱敏 fixture；转换器逻辑问题优先补精确单元断言；SSE 状态机问题必须补 stream fixture 或逐事件断言。
- 真实 provider fixture 不得包含 API key、Authorization header、query key 或用户敏感输入；动态 id/timestamp 可以稳定化，但必须保留能触发问题的协议结构、finish/status、usage 和 content/reasoning 字段。
- 修改后至少跑 `cd tauri && cargo test transformer --no-default-features`；大范围协议转换改动还必须按根规则补跑全量测试集合。

## 最小验证

- 修改 JSON 转换、SSE parser、stream state、统一模型或 transformer 后至少跑 `cd tauri && cargo test transformer --no-default-features`。
- 修改 route/path/header/auth 编排后额外跑 `cd tauri && cargo test proxy_gateway::runtime::upstream` 和 `cd tauri && cargo test proxy_gateway::runtime::providers`。
- 大范围协议转换改动交付前按根规则跑 `cd tauri && cargo test`；若同时改前端 provider 表单/i18n，再跑 `pnpm test`、`pnpm exec tsc --noEmit` 和 i18n check。

## Gotchas

- 新增协议时先扩展 `AiProtocol` 和 `ConversionRoute`，再同时补 JSON、SSE、error、runtime target path/header/auth、provider `apiFormat` 解析和测试。
- 不要在 `runtime/upstream.rs` 里临时写协议字段转换 helper；只允许 runtime 计算 route、调用本模块、保存转换后上游 body。
- 流式转换中完成事件必须幂等。OpenAI `[DONE]`、Responses `response.completed`、Anthropic `message_stop` 和 finish chunk 可能组合出现，只能输出一组目标协议完成事件。
- 对目标 Anthropic 的流式 tool_use 必须保证 `content_block_start`、若干 `input_json_delta`、`content_block_stop` 顺序完整。
- 对目标 Chat 的流式 finish chunk 必须包含 `delta:{}`，兼容 OpenAI 客户端 streaming parser。
- 对目标 Gemini 的 stream 不输出 OpenAI `[DONE]`；Gemini 结束由最后一个带 `finishReason` 的 chunk 表达。
- 无效 JSON SSE event 直接忽略，不得 panic；source 结束时仍按当前状态尝试 finish。

# Protocol Conversion AxonHub Repair Plan

## 目标

把 `docs/glm-review/` 中其他模型发现的问题、`docs/transform/` 中人工复核出的差异、以及当前实现代码里的可确认缺口合并成一份可执行修复计划。执行原则：

- AxonHub 行为作为对照基准。
- 不把 AxonHub 的平台级能力误塞进 AI Toolbox 当前无状态 `transformer` 模块。
- 能在当前中间模型和当前模块边界内修的，必须补代码和回归测试。
- 需要新中间模型字段、会话级 footprint、provider/platform config 或 runtime header/auth 的，先作为架构任务记录清楚，不能伪实现。
- 每完成一个阶段后必须自审 diff，再跑测试；自审发现问题必须继续修。

## 当前模块边界

代码位置：

- `tauri/src/coding/proxy_gateway/transformer/llm/model.rs`
- `tauri/src/coding/proxy_gateway/transformer/openai/chat.rs`
- `tauri/src/coding/proxy_gateway/transformer/openai/responses/mod.rs`
- `tauri/src/coding/proxy_gateway/transformer/anthropic/inbound.rs`
- `tauri/src/coding/proxy_gateway/transformer/anthropic/outbound.rs`
- `tauri/src/coding/proxy_gateway/transformer/gemini/mod.rs`
- `tauri/src/coding/proxy_gateway/transformer/stream.rs`
- `tauri/src/coding/proxy_gateway/transformer/kernel.rs`

必须遵守：

- 本模块不依赖 DB、Tauri app handle、provider 表、Gateway runtime context。
- source == target 时由 Gateway runtime 直通，不走本模块结构转换。
- SSE 继续边读边写；不能为了补 AxonHub 聚合能力而把整个 stream full-buffer。
- 所有修复必须有最贴近失败模式的 `kernel.rs` 回归测试。

## GLM Review 问题去重结论

`docs/glm-review/README.md` 统计了 150 个问题、155 个测试缺口。逐项对照后可归为 6 类：

1. 当前代码已经修复或 GLM 文档已过时：
   - Anthropic URL image source 双向互通。
   - Gemini `fileData.fileUri` 与普通 image URL 双向互通。
   - Gemini inline/file document 基础映射。
   - LLM model 已经有 `metadata`、`previous_response_id`、`request_type`、`api_format`、`transformer_metadata`、`reasoning_signature`、`redacted_reasoning_content` 等字段，不能再按“模型无字段”直接下结论。

2. 当前中间模型已有字段，能直接补的局部行为：
   - 入站 request 标记 `request_type` / `api_format`。
   - Chat `reasoning` / `reasoning_content` 入站双向同步。
   - Responses reasoning item 向后合并后续 function/custom tool/message。
   - Responses standalone `input_image` item 解析。
   - Responses response `previous_response_id` / `created_at` 提取和输出。
   - Responses finish/status 的 `failed` / `error` 双向映射。
   - Responses tool call item `status:"completed"`。
   - Chat 多 choice 保留。
   - Gemini 多 candidate 保留。
   - Anthropic base64 image 缺省 MIME 与 AxonHub 对齐为 `application/octet-stream`。
   - Anthropic/Gemini reasoning text 入站同步到 `reasoning`。
   - Gemini `systemInstruction.parts` 过滤 `thought:true`，用 `\n` 连接。
   - Gemini `toolConfig.allowedFunctionNames` 只在 `mode:"ANY"` 下生效；多个 allowed 映射为 required。
   - Gemini thinking budget 阈值改成 AxonHub 的 `<=1024 low`、`<=8192 medium`、`>8192 high`。

3. 当前中间模型字段存在但语义仍需谨慎补充：
   - `metadata.user_id` 到 Anthropic `metadata.user_id`。
   - `redacted_thinking` 到 `redacted_reasoning_content`。
   - `reasoning_signature` 在 Responses `encrypted_content` / Anthropic `signature` / Gemini `thoughtSignature` 的 provider-private 生命周期。
   - 这类字段可以保存，但不能跨 provider 伪造 marker；完整实现要进入架构阶段。

4. 需要扩展中间模型或 transformer metadata 的平台级字段：
   - `store`、`safety_identifier`、`modalities`、`reasoning_budget`、`reasoning_summary`。
   - Responses `include`、`max_tool_calls`、`prompt_cache_retention`、`truncation`、`stream_options.include_obfuscation`。
   - Gemini `safetySettings`、`imageConfig`、`responseModalities`、`topK`、logprobs details。
   - Anthropic top-level/system cache_control、thinking display/adaptive/disabled marker、output_config metadata。

5. 当前模块明确不承载的 AxonHub 大能力：
   - `signature marker / footprint` 跨 provider 会话生命周期。
   - OpenAI Responses compact。
   - image generation / embedding / video / audio / rerank。
   - Bedrock / Vertex / LongCat / Direct platform-specific URL/header/auth。
   - stream 聚合 `AggregateStreamChunks` 的非流式回放能力。

6. 需要 runtime 层处理的能力：
   - HTTP Content-Type、path、URL、header、auth、status code 的完整平台校验。
   - Gemini path model/action 提取。
   - Anthropic web-search beta header。
   - Vertex function id 清理。

## 本轮实施阶段 A：局部可落地修复

### A1. 公开 RequestType / ApiFormat 并写入入站 request

文件：`tauri/src/coding/proxy_gateway/transformer/llm/mod.rs`

修改：

1. 把 `ApiFormat` 和 `RequestType` 从 `llm::constants` re-export：
   - 现有：
     - `pub use constants::{ TOOL_TYPE_... }`
   - 修改为：
     - `pub use constants::{ ApiFormat, RequestType, TOOL_TYPE_... }`

文件：`openai/chat.rs`

修改：

1. import 增加 `ApiFormat`、`RequestType`。
2. 在 `chat_request_to_llm` 构造 `Request` 时设置：
   - `request_type: Some(RequestType::Chat)`
   - `api_format: Some(ApiFormat::OpenAiChatCompletions)`

文件：`openai/responses/mod.rs`

修改：

1. import 增加 `ApiFormat`、`RequestType`。
2. 在 `responses_request_to_llm` 构造 `Request` 时设置：
   - `request_type: Some(RequestType::Chat)`
   - `api_format: Some(ApiFormat::OpenAiResponses)`

文件：`anthropic/inbound.rs`

修改：

1. import 增加 `ApiFormat`、`RequestType`。
2. 在 `anthropic_request_to_llm` 构造 `Request` 时设置：
   - `request_type: Some(RequestType::Chat)`
   - `api_format: Some(ApiFormat::AnthropicMessages)`

文件：`gemini/mod.rs`

修改：

1. import 增加 `ApiFormat`、`RequestType`。
2. 在 `gemini_request_to_llm` 构造 `Request` 时设置：
   - `request_type: Some(RequestType::Chat)`
   - `api_format: Some(ApiFormat::GeminiContents)`

测试：

- 在 `kernel.rs` 新增 `inbound_requests_mark_request_type_and_api_format`。
- 分别调用 Chat / Responses / Anthropic / Gemini -> LLM 的 request conversion。
- 断言序列化结果里含：
  - `"request_type":"chat"`
  - 对应 `"api_format"`。

### A2. Chat reasoning 入站双向同步

文件：`openai/chat.rs`

修改函数：`chat_message_to_llm`

步骤：

1. 在构造 `Message` 前计算：
   - `let reasoning = message.get("reasoning_content").or_else(|| message.get("reasoning")).and_then(Value::as_str).map(ToString::to_string);`
2. `Message` 中同时设置：
   - `reasoning_content: reasoning.clone()`
   - `reasoning`

禁止：

- 本阶段不要修改 Chat 出站默认策略为 `none`，因为当前模块没有 provider/channel-level `ReasoningField` 配置；贸然剥离会破坏 DeepSeek/Gemini 兼容 Chat target。

测试：

- 新增 `chat_reasoning_field_syncs_into_llm_message`。
- 输入 assistant message 只有 `reasoning`，无 `reasoning_content`。
- 转 Anthropic 时应出现 thinking 文本。
- 也断言中间 LLM JSON 同时有 `reasoning_content` 与 `reasoning`。

### A3. Responses reasoning item 向后合并

文件：`openai/responses/mod.rs`

新增 helper：

1. `fn responses_reasoning_message(item: &Value) -> Message`
   - role = assistant
   - `reasoning_content` = `responses_reasoning_text(item)`
   - `reasoning` = 同上 clone
   - `reasoning_signature` = `item.encrypted_content`

2. `fn responses_message_item_to_llm(item: &Value) -> Message`
   - 把原 `append_responses_item_to_messages` default 分支里的 message 构造逻辑抽出来。

3. `fn merge_responses_following_item_into_reasoning_message(reasoning_message: &mut Message, following: &Value) -> bool`
   - 如果 following type 是 `function_call` 或 `custom_tool_call`：
     - `reasoning_message.tool_calls.push(responses_call_to_tool_call(following, 0))`
     - return true
   - 如果 following type 是 `message` 或无 type 但含 `content`：
     - 用 `responses_message_item_to_llm(following)` 得到 message。
     - 把 `content`、`refusal`、`annotations` 合并到 `reasoning_message`。
     - return true
   - 其他 return false。

修改函数：`append_responses_input_to_messages`

1. `Value::Array(items)` 分支改成 `while index < items.len()`。
2. 遇到 `type == "reasoning"`：
   - 创建 `reasoning_message`。
   - 看 `items.get(index + 1)`。
   - 如果 helper 返回 true，`index += 2`；否则 `index += 1`。
   - push 合并后的 message。
3. 非 reasoning 继续调用 `append_responses_item_to_messages`。

修改函数：`append_responses_item_to_messages`

1. `Some("reasoning")` 分支改为 push `responses_reasoning_message(item)`。
2. default 分支改为 push `responses_message_item_to_llm(item)`。

测试：

- 新增 `responses_reasoning_item_merges_following_function_call`。
- 输入 `reasoning` item 后紧跟 `function_call` item。
- 转 Anthropic 或 Chat 后，assistant 同一条 message 里同时有 thinking/reasoning 和 tool call。

### A4. Responses standalone input_image item

文件：`openai/responses/mod.rs`

修改函数：`append_responses_item_to_messages`

1. 在 match 中新增 `Some("input_image")` 分支：
   - push user message。
   - content = `MessageContent::Parts(vec![responses_input_image_part(item)])`。

新增 helper：

1. `fn responses_input_image_part(item: &Value) -> Option<MessageContentPart>`
   - 从 `image_url` 读字符串。
   - `detail` 从 `detail` 读字符串。
   - 返回 `part_type:"image_url"`。
2. content array 里的 `input_image` 分支也复用这个 helper。

测试：

- 新增 `responses_standalone_input_image_item_converts_to_image_url`。
- 输入 Responses `input` 数组里只有 `{type:"input_image", image_url:"https://..."}`。
- 转 Chat 后应得到 user content image_url。

### A5. Responses response 字段和 status 映射补齐

文件：`openai/responses/mod.rs`

修改函数：`responses_response_to_llm`

1. Response 增加：
   - `previous_response_id: body.get("previous_response_id").and_then(Value::as_str).map(ToString::to_string)`
   - `created: body.get("created_at").or_else(|| body.get("created")).and_then(Value::as_i64).unwrap_or_default()`

修改函数：`llm_response_to_responses`

1. 构造 body 后，如果 `response.previous_response_id.is_some()`：
   - `body["previous_response_id"] = json!(previous_response_id)`
2. 不要丢 `created_at`。

修改函数：`responses_status_to_finish`

1. 映射：
   - `failed` -> `error`
   - `incomplete` -> `length`
   - `completed && has_tool` -> `tool_calls`
   - `completed` -> `stop`
   - 其他 -> `stop`

修改函数：`finish_to_responses_status`

1. 映射：
   - `error` -> `failed`
   - `length|max_tokens` -> `incomplete`
   - 其他 -> `completed`

测试：

- 新增 `responses_status_and_previous_response_metadata_roundtrip`。
- 输入 Responses response 带 `previous_response_id`、`created_at`、`status:"failed"`。
- 转 LLM 后断言 `previous_response_id`、`created`、finish_reason=`error`。
- 再转 Responses 断言 `previous_response_id` 存在且 `status:"failed"`。

### A6. Responses tool call item status completed

文件：`openai/responses/mod.rs`

修改函数：`tool_call_to_responses_item`

1. function tool call 输出增加：
   - `"status": "completed"`
2. custom tool call 输出增加：
   - `"status": "completed"`

测试：

- 新增 `responses_tool_call_items_include_completed_status`。
- 构造 LLM assistant tool call 转 Responses。
- 断言 output/input item 里 `status == "completed"`。

### A7. Chat 多 choice 双向保留

文件：`openai/chat.rs`

修改函数：`chat_response_to_llm`

1. 替换 `choices.first()` 单 choice 逻辑。
2. 遍历 `body["choices"]` array。
3. 每个 choice 转一个 `Choice`。
4. 如果 choices 缺失或为空，保留一个 default choice，避免当前 fallback 行为突然变成空。

修改函数：`llm_response_to_chat`

1. 替换单 `choice` 输出。
2. 遍历 `response.choices`。
3. 每个 choice 输出：
   - `index`
   - `message`
   - `finish_reason`
4. 如果 choices 为空，输出一个 default choice。

测试：

- 新增 `chat_response_preserves_multiple_choices`。
- 输入两个 choices，转 LLM 后 choices 长度为 2。
- 再转 Chat 后 choices 长度为 2，index 和 content 保留。

### A8. Gemini 多 candidate 双向保留

文件：`gemini/mod.rs`

修改函数：`gemini_response_to_llm`

1. 非 block_reason 分支遍历 `candidates`。
2. 每个 candidate 调 `gemini_content_to_llm`。
3. 每个 candidate 转一个 `Choice`：
   - `index` 用 array index。
   - `finish_reason` 按该 candidate finishReason 和 has_tool 映射。
4. candidates 缺失或为空时保留一个 default choice。

修改函数：`llm_response_to_gemini`

1. 遍历 `response.choices` 生成 candidates。
2. 每个 candidate 包含 content 和 finishReason。
3. choices 为空时保留一个 default candidate。

测试：

- 新增 `gemini_response_preserves_multiple_candidates`。
- 输入两个 candidates，转 LLM 后 choices 长度为 2。
- 再转 Gemini 后 candidates 长度为 2。

### A9. Gemini system thought 过滤与 tool_choice mode 修正

文件：`gemini/mod.rs`

修改函数：`gemini_parts_text`

1. 过滤 `part.thought == true`。
2. join 分隔符从 `"\n\n"` 改成 `"\n"`。

文件：`shared/messages.rs`

修改函数：`tool_choice_from_gemini`

1. 先读取 `mode`。
2. 只有 `mode == "ANY"` 时检查 `allowedFunctionNames`。
3. `allowedFunctionNames.len() == 1` -> `ToolChoice::Named`。
4. `allowedFunctionNames.len() > 1` -> `ToolChoice::String("required")`。
5. `mode == "NONE"` -> none。
6. `mode == "AUTO"` -> auto。

测试：

- 新增 `gemini_system_instruction_filters_thought_parts`。
- 新增 `gemini_tool_choice_allowed_names_respects_mode`。

### A10. Anthropic MIME default 和 reasoning sync

文件：`anthropic/inbound.rs`

修改：

1. 两处 `unwrap_or("image/png")` 改为 `unwrap_or("application/octet-stream")`：
   - `append_anthropic_message_to_llm`
   - `anthropic_content_part_to_llm`
2. `thinking` block 入站时：
   - `reasoning_content = Some(text.clone())`
   - `reasoning = Some(text)`，需要在函数中新增 `let mut reasoning = None;` 并写入 Message。
3. response `anthropic_response_to_llm` 中 thinking block 同步设置 `message.reasoning`。

测试：

- 新增 `anthropic_image_missing_media_type_uses_octet_stream`。
- 新增 `anthropic_thinking_syncs_reasoning_fields`。

### A11. Gemini reasoning sync 和 budget 阈值

文件：`gemini/mod.rs`

修改：

1. `reasoning_effort_from_gemini_thinking_config` threshold：
   - `budget <= 0` -> none
   - `budget <= 1024` -> low
   - `budget <= 8192` -> medium
   - else -> high
2. `gemini_content_to_llm` 返回 `Message` 时，reasoning_chunks 非空：
   - 同时设置 `reasoning_content` 和 `reasoning`。

测试：

- 新增 `gemini_thinking_budget_threshold_matches_axonhub`。
- 新增 `gemini_thought_text_syncs_reasoning_fields`。

## 本轮实施阶段 B：自审与验证

执行顺序：

1. `cargo fmt`
2. `cd tauri && cargo test transformer --no-default-features`
3. `git diff --check -- tauri/src/coding/proxy_gateway/transformer`
4. 自审 diff，重点看：
   - 是否把 runtime/header/auth/path 能力错误写进 transformer。
   - 是否新增了 DB/Tauri app handle 依赖。
   - 是否让 source == target 也走转换。
   - 是否破坏现有 reference fixture 矩阵。
   - 是否把 provider-private signature 伪造给其他 provider。
5. 如果测试失败或自审发现问题，按失败点继续修，再回到第 1 步。

## 后续架构阶段 C：需要单独设计后再修

这些项不能由低成本 patch 直接完成，否则会产生假对齐：

### C1. Signature marker / footprint

来源问题：

- GLM P-01-02 / P-01-03 / P-02-01 / P-03-07 / P-04-13 / P-05-11 / P-06-10 / P-07-05 / P-08-06。

AxonHub 行为：

- Anthropic thinking signature、Gemini thoughtSignature、Responses encrypted_content 通过 provider marker/footprint 跨同会话保存、识别、还原、丢弃。

AI Toolbox 所需设计：

1. 在 `llm::Message` / `ToolCall` / `MessageContentPart` 的 `transformer_metadata` 中定义统一 key：
   - `provider_signature_format`
   - `anthropic_signature`
   - `gemini_thought_signature`
   - `openai_encrypted_content`
   - `signature_source_protocol`
2. 定义只在同协议或明确兼容协议内还原的规则。
3. 明确跨 provider mismatch 行为：默认丢弃，不生成假 signature。
4. 补 JSON request/response/SSE 三类测试。

### C2. TransformerMetadata / ExtraBody platform options

来源问题：

- Responses include/truncation/max_tool_calls/prompt_cache_retention/include_obfuscation。
- Gemini safetySettings/imageConfig/responseModalities/topK/logprobs。
- Anthropic top-level/system cache_control、thinking display/adaptive、output_config。

所需步骤：

1. 定义 metadata key 常量，不允许散落字符串。
2. 入站把 source-only 字段写入 `Request.transformer_metadata`。
3. 出站只在目标协议支持时读出。
4. 不把 source-only 字段自动塞进 `extra_body`。
5. 每个字段补 roundtrip 测试。

### C3. Compact / image_generation / audio / video

来源问题：

- Responses compact/image_generation。
- Chat audio/video/input_audio。
- Gemini video/audio。

所需步骤：

1. 扩展 `MessageContentPart`，新增 `audio`、`video_url`、`input_audio`、`image_generation` 或等价结构。
2. 扩展 request type，明确 Gateway CLI 是否真的会触达这些 API。
3. 先补模型序列化测试，再补 8 向转换测试。

### C4. Stream 聚合

来源问题：

- GLM 多处 `AggregateStreamChunks` 缺失。

当前判断：

- Gateway 运行态要求 SSE 边读边写，当前模块主路径不应该 full-buffer。

后续设计：

1. 如果需要“上游流式 -> 客户端非流式”或测试 fixture 聚合能力，应新增独立 `stream_aggregate.rs`。
2. 聚合器只用于显式调用，不进入默认 streaming proxy path。
3. 聚合器覆盖 text/reasoning/tool_call/usage/annotations/citations/finish。

### C5. Runtime/path/header/auth/status code

来源问题：

- HTTP Content-Type 校验、Gemini path model/action、Anthropic web_search beta header、Vertex function id、platform URL/header/auth。

处理位置：

- `runtime/routes.rs`
- `runtime/upstream.rs`
- `runtime/providers.rs`
- 不放进 `transformer`。

## 执行状态

- [x] A1 RequestType / ApiFormat
- [x] A2 Chat reasoning 入站同步
- [x] A3 Responses reasoning 向后合并
- [x] A4 Responses standalone input_image
- [x] A5 Responses response metadata/status
- [x] A6 Responses tool call status
- [x] A7 Chat 多 choice
- [x] A8 Gemini 多 candidate
- [x] A9 Gemini system/tool_choice
- [x] A10 Anthropic MIME/reasoning
- [x] A11 Gemini reasoning/budget
- [x] B1 cargo fmt
- [x] B2 transformer 测试
- [x] B3 diff check
- [x] B4 自审与二次修复

## 本轮执行结果

已落地代码级修复：

- `llm/mod.rs` 公开 `RequestType` / `ApiFormat`，四个入站 transformer 写入对应协议标记。
- `openai/chat.rs` 同步 `reasoning` / `reasoning_content`，并保留多 choice 响应。
- `openai/responses/mod.rs` 补齐 reasoning item 合并、standalone `input_image`、`previous_response_id` / `created_at`、`failed` / `error` status 映射、tool call `status:"completed"`。
- `anthropic/inbound.rs` / `anthropic/outbound.rs` 补齐 URL image、缺省 MIME、reasoning 同步、`metadata.user_id`、Anthropic 目标缺省 `max_tokens=8192`。
- `gemini/mod.rs` 补齐 `fileData.fileUri`、document/fileData、thinking budget 阈值、thought 文本过滤/同步、多 candidate 保留。
- `shared/messages.rs` 修正 Gemini `allowedFunctionNames` 只在 `mode:"ANY"` 下生效。
- `kernel.rs` 增加每个修复点的精确回归测试，避免只靠 fixture 矩阵隐式覆盖。

验证结果：

- 已执行 `cargo fmt`，通过。
- 已执行 `cd tauri && cargo test transformer --no-default-features`，通过：57 passed，0 failed。
- 已执行 `cd tauri && cargo test`，通过：lib 603 passed / 1 ignored，integration 112 passed，doc-tests 10 passed，0 failed。
- 已执行 `pnpm test`，通过：164 passed，0 failed。
- 已执行 `pnpm exec tsc --noEmit`，通过。
- 已执行 `git diff --check -- tauri/src/coding/proxy_gateway/transformer docs/protocol-conversion-repair-plan.md`，通过。
- 已执行协议转换目录边界扫描；本轮修改没有新增 DB、Tauri app handle、provider 表、Gateway runtime、HTTP header/auth/path 依赖。唯一命中是 `kernel.rs` 既有测试 helper 的 `tauri::async_runtime::block_on`，不属于本轮新增。

自审结论：

- 本轮阶段 A 已完成；没有发现需要二次修复的新增问题。
- 阶段 C 仍是未实施的架构任务，不应由后续模型用局部 patch 伪实现。

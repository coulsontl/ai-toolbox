# Codex Remote Compaction V2 支持设计

> 状态：**设计待审**（2026-10-07）。本文是 issue #416 的设计文档，经用户确认后进入实现。
> 关联文档：[`gateway-protocol-conversion.md`](gateway-protocol-conversion.md)（架构主文档）、[`gateway-provider-compatibility.md`](gateway-provider-compatibility.md)（渠道兼容细节）。实现完成后，架构结论必须回写这两份文档。

## 1. 问题

### 1.1 现象

Codex 使用第三方中转（issue 报的是 DeepSeek）时报错：

```
Error running remote compact task: Fatal error: remote compaction v2 expected exactly
one compaction output item, got 0 from 1 output items
```

该错误串由 Codex 上游硬编码（`codex-rs/core/src/compact_remote_v2.rs:487`）。

### 1.2 触发条件

Codex 判定 provider 支持 remote compaction v2 的唯一条件是 provider 的 `name` 字段：

```rust
// codex-rs/model-provider/src/capabilities.rs:31-41
remote_compaction: if info.is_openai()
    || is_azure_responses_provider(&info.name, info.base_url.as_deref())
{ RemoteCompactionSupport::V2 } else { Unsupported }
```

```rust
// codex-rs/model-provider-info/src/lib.rs:616-618
pub fn is_openai(&self) -> bool { self.name == OPENAI_PROVIDER_NAME }  // "OpenAI"，大小写敏感
```

即 `[model_providers.<id>].name = "OpenAI"` 是唯一开关，与 `base_url` 指向谁无关。

AI Toolbox 的默认模板正是这么写的（`web/features/coding/codex/hooks/useCodexConfigState.ts:52-57`），兜底模板（`web/utils/codexConfigUtils.ts:591`）与 All API Hub 导入模板（`CodexPage.tsx:1826`）同样写死 `name = "OpenAI"`。因此**任何通过 AI Toolbox 新建的第三方 provider 都会触发 v2**。

### 1.3 为什么现在会失败

Codex 2026-09-09（commit `3dc1e2a584`）起把手动与自动压缩全部改走 v2 流式路径，废弃 `/responses/compact` 端点。最新源码中非测试代码对 `/responses/compact` 零引用。

v2 协议形态：

1. 请求：普通 `POST /responses`（stream），`input[]` 末尾追加 `{"type":"compaction_trigger"}`（`compact_remote_v2_attempt.rs:95`）。
2. 响应：SSE 流中**恰好一个** `{"type":"compaction","encrypted_content":"..."}` output item，随后 `response.completed`。
3. 校验：`compaction_count != 1` 即 Fatal（`compact_remote_v2.rs:485-488`）。
4. 后续：该 compaction item 被保留为历史载体（`compact_remote_v2.rs:519` `retained.push(ResponseItemEnvelope::new(compaction_output))`），下次请求原样回传 `encrypted_content`。

第三方上游（DeepSeek / 中转站）不认识 `compaction_trigger`，返回普通 assistant 文本 → 0 个 compaction item → 报错。

### 1.4 AI Toolbox 当前行为

- `compaction_trigger` 全仓库**零匹配**，落到 `responses_item_to_message` 的 `_ => None`（`transformer/openai/responses/shared.rs:508`）。
- 不在 `is_structurally_represented_responses_input_item` 白名单（`shared.rs:231-249`），被存为 raw fragment sidecar。
- 该 sidecar **仅 Responses 目标会回写**（`request.rs:150-153`）；Chat/Anthropic/Gemini 目标静默丢弃，且不触发 lossy 警告（`lossy.rs:240-258` 无 default 分支）。
- 出站 `compaction` item 同理：`response.rs:77-79` 能解析成 IR part，但 Chat/Anthropic/Gemini writer 全在 `_ => None` 丢弃。
- `/responses/compact` facade（`codex_responses_compact.rs:308-316`）是纯路径精确匹配，与 v2 无关，且该端点已被 Codex 废弃。

## 2. 目标与非目标

### 2.1 目标

用户选择"网关支持 v2 压缩"：Gateway 正确应答 compaction v2 请求，而不是回退本地压缩。

- 识别 v2 compaction 请求，按压缩语义转发上游。
- 合成符合 Codex 契约的 compaction item 与 SSE 序列。
- 覆盖四种目标协议：OpenAI Responses 直通、OpenAI Chat、Anthropic Messages、Gemini Native。
- 摘要用**无状态**自识别 envelope 承载（见 §3.5），不引入跨请求状态。

### 2.2 非目标

- 不修改 `name = "OpenAI"` 默认值（用户决定"先只记录不修改"）。
- 不实现 `/responses/compact` 端点（Codex 已废弃）。
- 不伪造 OpenAI 服务端密文语义（不声称能解开真 OpenAI 密文）。

## 3. 核心设计

### 3.1 密文所有权问题

`encrypted_content` 由 Gateway 生成，也必须由 Gateway 解开。Codex 视其为不透明字符串，会在后续请求原样回传（`protocol/src/models.rs:1229-1237`，`Compaction { encrypted_content: String, .. }`）。

关键观察：**Codex 自己会把摘要存进历史**。Gateway 只要在 `encrypted_content` 里自包含地承载摘要，后续请求原样回传时就能就地解出，**不需要任何跨请求状态**。这正是 cc-switch PR #5536 的做法（`transform_codex_chat.rs:1637-1655`），本设计采纳。

因此不引入 side store、不绑定 provider 指纹、不设容量上限——摘要随 Codex 的历史一起流转。

### 3.2 数据流

```
Codex ──POST /responses (input 末尾有 compaction_trigger)──► Gateway
                                                              │
    ┌─────────────────────────────────────────────────────────┘
    ▼
 1. 识别 compaction 请求（input 含 compaction_trigger）
 2. 剥离 compaction_trigger；剥离 tools（压缩请求不需要工具）
 3. 转成「无工具摘要请求」发往上游
 4. 上游返回摘要文本
 5. 摘要 → 存本地 envelope；生成 opaque token 作 encrypted_content
 6. 合成 SSE：一个 compaction item + response.completed
    │
Codex ◄──┘

后续请求：Codex 回传 compaction item（带 opaque token）
         → Gateway 识别 token → 还原摘要 → 作为 assistant 历史注入上游请求
```

### 3.3 请求改写规则

识别在 **runtime 层**完成（路由决策），改写才进入转换层。这与仓库架构边界一致：`transformer/` 是纯载荷转换层，不做路由判定。

| 步骤 | 规则 | 理由 |
|---|---|---|
| 识别 | 路径匹配 `/responses/compact`（legacy）**或** `input[]` 含 `type == "compaction_trigger"`（v2） | 两代协议都要认。**不得**用 `x-codex-turn-metadata.request_kind == "compaction"`：Codex 对本地 checkpoint 压缩也发该元数据，那种请求的响应必须是普通 assistant 摘要（cc-switch review P1） |
| 剥离 | 移除 `compaction_trigger` item，不进 IR | 它只是信号，不是要保留的内容 |
| 去工具 | 移除 `tools` / `tool_choice` / `parallel_tool_calls` / `response_format` / `stop` | 压缩请求不需要工具；cc-switch 移除这五项并设 `n = 1` |
| 指令 | 替换 Codex 的 `instructions` 为专用摘要指令 | Codex 的 base instructions 是"继续实现任务"，会让模型继续干活而不是总结。cc-switch 直接 `body.remove("instructions")` |
| 降级 | 历史 `system` / `developer` item 抽出来，以带标签文本降级为 assistant 消息（插在 index 1），专用指令作为唯一 system | 历史指令是 transcript 数据，不能与压缩指令同级。cc-switch 用 `COMPACTION_HISTORICAL_SYSTEM_HEADING` + `[role]` 标签 |
| 尾注 | 末尾追加一条 user 指令"现在产出摘要" | 给模型明确的产出触发点 |
| 回填 | 识别 input 中的 compaction item，解出摘要，还原为带标题的 assistant 消息 | Codex 会把上次的 compaction item 回传；摘要还原后须**位于**前置工具结果媒体之后（cc-switch 的 media bridge 顺序约束） |

### 3.4 响应合成规则

- **恰好一个** compaction output item。Codex 严格校验 `compaction_count != 1` 即 Fatal（`compact_remote_v2.rs:485-488`）。
- SSE 只需两个事件：`response.output_item.done`（携带 compaction item）+ `response.completed`。**不需要 `response.output_item.added`**——Codex 的 SSE 解析器对 `done` 独立解析（`codex-api/src/sse/responses.rs:353`），cc-switch 实测只发这两个。
- 摘要为空或被截断（`finish_reason == "length"`）时必须拒绝（cc-switch 做法），否则会把不完整摘要固化进 Codex 历史。
- 流式上游也要先缓冲完整摘要再合成，不能边收边转。
- 非流请求直接返回 JSON 形态的同一 response 对象。
- **SSE 重渲染必须跟随客户端声明**：合成好的 item 要改写成 SSE 只在客户端声明了 `stream: true` 时做（`upstream.rs:2449`）。Codex 恒为流式，所以这条分支实际不可达，但无条件重渲染会让非流式调用方拿到一个 `text/event-stream` 外壳、解析不出来的 body。

### 3.5 envelope 编码（无状态）

```rust
const COMPACTION_CAPSULE_PREFIX: &str = "ai-toolbox-compact-v1:";

// 编码：AES-256-GCM(摘要) → 前缀 + base64url(nonce ‖ ciphertext ‖ tag)
// 密钥由固定 secret 经 SHA-256 派生，见 COMPACTION_KEY_SECRET
```

设计要点：

- **自识别前缀**：真 OpenAI 密文不会有该前缀，因此天然区分，不需要 provider 指纹或额外标记。
- **无跨请求状态**：摘要随 Codex 历史流转，Gateway 重启、多实例、换 provider 都不影响。
- **解码失败即降级**：前缀不匹配或认证失败时返回 `None`，该 item 按"非本 Gateway 生成"处理，不做还原。
- 前缀常量必须与 cc-switch 不同（避免跨工具历史互相误解码）。

**为什么加密而不是 base64**（初版设计是 base64，审计后改为 AES-GCM，学 CLIProxyAPI）：

`encrypted_content` 会随 Codex 历史写进 rollout 文件与日志，base64 只是编码不是保护——摘要里的用户代码、密钥、路径会以近乎明文的形式落盘。AES-256-GCM 用仓库既有的 `ring::aead`（`src/coding/zcode/credential_cipher.rs` 同款原语），每次封装用新随机 nonce，认证标签保证密文不被篡改。

容量与淘汰、指纹绑定、持久化均**不需要**——这是相对初版设计的简化。

### 3.6 WebSocket 传输

Codex 的 remote compaction v2 走的是 `ModelClientSession::stream()`，与普通轮次**同一个入口**（`core/src/client.rs:2251`）：provider 支持 WebSocket 时，compaction 轮次同样发 `response.create` 帧，`compaction_trigger` 在 `input` 末尾。因此网关的 WS 路径必须同样处理。

三条与 HTTP 不同的约束：

| 约束 | 事实 | 后果 |
|---|---|---|
| 帧是裸 JSON | Codex 用 `serde_json::from_str::<ResponsesStreamEvent>(&text)` 直接解析文本帧（`codex-api/src/endpoint/responses_websocket.rs:739`），没有 SSE 的 `event:`/`data:` 外壳 | 合成事件必须逐帧发裸 JSON，不能复用 HTTP 的 SSE 文本 |
| 没有非流模式 | WS 只能一帧一帧收 | 摘要必须从上游自己的 delta 事件里重组，不能像 HTTP 那样请求一次性 JSON |
| 网关只转发同协议 | `websocket.rs` 的 `fallback_reason` 对任何需要协议转换的 provider 返回 426 | WS 压缩**只覆盖 Responses 直通**，Chat/Anthropic/Gemini 走 HTTP |

请求侧的改写与 HTTP 共用：`prepare_websocket_request_with_model` 调 `RemoteCompactionCompat::resolve_websocket` 判定，`build_upstream_body_for_provider` 内的改写逻辑不变（`stream` 字段本来就由 WS 路径统一剥除）。

响应侧在 `relay` 里拦截：compaction 轮次的每条上游帧先喂给 `CompactionStreamSummary` 累加，**不转发**；收到终态后合成为一个 `compaction` item，用 `build_compaction_frames` 发 5 个裸 JSON 帧。上游自己的 `response.completed` 被丢弃——否则 Codex 会看到两个终态。

摘要不可用时**不伪造 item**：Codex 把返回的 item 当作整段对话历史安装，占位文本会静默丢弃上下文。WS 侧改为转发上游自己的终态，让 Codex 按 error code 分类并重试可重试项。注意失败前到达的 delta 只是半截草稿，`finish()` 必须先判失败。

### 3.7 路由边界（与 `/responses/compact` 的分工）

legacy `/responses/compact` 端点**已经由** `CodexResponsesCompactCompat` 拥有（Codex 自身已废弃该端点）。本模块刻意只认 v2 信号，否则同一个请求会被改写两次。`RemoteCompactionCompat::resolve` 里对此有显式文档注释与回归测试（`the_legacy_compact_endpoint_is_not_claimed`）。

### 5.1 catalog 的 `auto_compact_token_limit` 缺口

cc-switch 在同一 PR 里补了一个 AI Toolbox 目前完全缺失的字段（`codex_config.rs:1200-1216`）：

```rust
if profile == CodexCatalogToolProfile::ProxyChat {
    entry_obj.insert("auto_compact_token_limit", json!(context_window * 9 / 10));
} else {
    entry_obj.remove("auto_compact_token_limit");
}
```

- **只为 ProxyChat profile 写**：Chat 中转路径需要显式阈值跟随 provider 的有效窗口。
- **NativeResponses / Anthropic 显式移除**：让 Codex 按 `context_window` 自己推导 90%。
- 原因见 PR review P1：给 Anthropic profile 声明自动压缩阈值，会让 Codex 在 90% 处发起压缩请求，而 Anthropic 路径当时没有 compaction 响应语义 → 长会话失败。

AI Toolbox 的 catalog 生成（`tauri/src/coding/codex/commands.rs`）当前**不写该字段**。若本设计补齐了四种目标协议的 compaction 支持，这个字段才有意义——但**必须与目标协议的实际支持范围严格对齐**，不能无条件写。这条列入实现清单。

## 4. 四目标协议差异

### 4.1 Responses 直通（identity route）

**这是现有设计的盲点**：`conversion_route()` 在 `source == target` 时返回 `None`（`upstream.rs:9304-9312`），请求走 raw 字节直通，transformer 不参与，`compaction_trigger` 会被原样透传给第三方上游。

方案：**专项旁路**（用户已确认）。在 runtime 层把「带 `compaction_trigger` 的 `/responses` 请求」从 identity 直通中拉出，走新增的 compaction 专项路径；其它 `/responses` 请求保持 raw 字节保真。

实际落点（初版设计写的「用 `conversion_route()` 强制切换转换路径」**没有采用**）：`build_upstream_body_for_provider` 对 identity 路由本来就会解析再重发 JSON，所以不需要造一条转换路由——在 `upstream.rs:5997` 按 `RemoteCompactionCompat::is_remote_compaction()` 就地改写即可，`conversion_route` 保持 `None`。这条最容易被后续改动静默破坏（改了 body 构造就可能让 identity 路由上的 trigger 原样透传），因此有专门的集成测试钉住：`identity_responses_compaction_is_rewritten_and_sealed`（`tauri/tests/coding/proxy_gateway/codex_remote_compaction_http.rs`）。

### 4.2 Chat / Anthropic / Gemini

走正常转换路径，在 runtime compat 层统一处理。差异只在"摘要请求"的目标形态：

| 目标 | 摘要请求形态 | 响应提取 |
|---|---|---|
| OpenAI Chat | `messages` 数组 + 无 tools | `choices[0].message.content` |
| Anthropic | `messages` + 无 tools，system 走顶层 | `content[].text` |
| Gemini | `contents` + 无 tools | `candidates[].content.parts[].text` |

上游摘要提取后统一走 3.5 的 envelope 存储与 3.4 的响应合成。

### 4.3 Anthropic 目标的已知风险

cc-switch 的 code review 明确指出：其 Anthropic 路径**未实现** compaction 支持，若 catalog 声明了自动压缩阈值会导致 Anthropic-backed 长会话失败。本设计要覆盖 Anthropic，必须为它单独补齐响应合成，不能只做 Chat。

## 5. 关键实现位置

（下表为**实际交付**的结构。）

| 层 | 位置 | 工作 |
|---|---|---|
| 请求识别 | `runtime/compat/codex_remote_compaction.rs` | `RemoteCompactionCompat::resolve`（HTTP，按路由+body）/ `resolve_websocket`（WS，只按 body）：`compaction_trigger` → `Summarize`，回传的本地 capsule → `ReplayCapsules` |
| 请求改写 | 同上 | `build_compaction_summary_request`：剥离 trigger、去五项工具字段、替换 instructions、追加产出指令、`stream: false` |
| capsule 回填 | 同上 | `expand_compaction_capsules`：解开本地 capsule，还原为带标题的可读上下文 |
| 响应合成 | 同上 | `build_compaction_response` / `build_websocket_compaction_response` → `build_compaction_sse` / `build_compaction_frames`（恰好一个 compaction item + `response.completed`） |
| 摘要抽取 | 同上 | `extract_summary_text`（HTTP，按结构识别四种协议） / `CompactionStreamSummary`（WS，从 delta 重组） |
| capsule | 同上（纯函数） | AES-256-GCM 封装 + 自识别前缀，无状态 |
| HTTP 挂载 | `runtime/upstream.rs` | `send_upstream_request` 解析一次，请求侧进 `build_upstream_body_for_provider`，响应侧进 `build_gateway_response` |
| WS 挂载 | `runtime/websocket.rs` | `prepare_websocket_request_with_model` 判定；`relay` 内 `PendingTurn.compaction` 累加上游帧并在终态合成 |
| 门控 | `codex-rs/model-provider/src/capabilities.rs` | `name = "OpenAI"`（或 Azure）或显式 `capabilities.remote_compaction = "v2"` 才启用（见 §8） |
| catalog | 未改动 | `auto_compact_token_limit` 缺口仍存在（§5.1） |

## 6. 测试矩阵

（下列为**已交付**的测试；全部在 `runtime/compat/codex_remote_compaction.rs` 与 `runtime/websocket/tests.rs` 内。）

### 6.1 协议契约

- 四种目标协议的摘要抽取各一条（`the_summary_comes_out_of_every_target_protocol`）。
- HTTP SSE 序列形状：5 个事件、恰好一个 `output_item.done`、开场合照不带 item（`the_compaction_stream_has_exactly_one_done_item`）。
- WS 帧形状：5 个裸 JSON 帧、不含 `event:`/`data:` 外壳、恰好一个 done（`the_websocket_frames_are_bare_json_with_one_compaction_item`）。
- 端到端：WS compaction 轮次被摘要并替换，上游看不到 trigger/tools，指令被替换，capsule 可回读且非明文，用量透传（`websocket_compaction_turn_is_summarized_and_replaced`）。

### 6.2 边界

- `compaction_trigger` 与本地 checkpoint compaction（仅 `request_kind` 头）区分：后者必须仍返回 assistant 摘要（`local_checkpoint_metadata_is_not_remote_compaction`）。
- 摘要为空 / 被截断 / 上游失败 → 一律不产出 item（HTTP 走错误路径，WS 转发上游终态）。
- WS 无 delta 时回退到 `output_item.done` 的完整文本（`a_summary_without_deltas_falls_back_to_the_completed_item`）。
- WS 上游失败/截断/空摘要均不产出 item（`a_failed_or_empty_summary_yields_no_item`；relay 层三条：`websocket_compaction_failure_is_forwarded_not_synthesized`、`websocket_empty_summary_forwards_the_upstream_terminal`、`websocket_truncated_summary_forwards_the_upstream_terminal`）。后两条钉住「上游**成功**但摘要不可用」这条最容易被误当成可合成的情况——它只转发上游自己的 `response.completed`/`response.incomplete`，Codex 因此报自己的 `expected exactly one compaction output item`，而不是被塞进一个伪造的上下文。
- legacy `/responses/compact` 不被本模块认领（`the_legacy_compact_endpoint_is_not_claimed`）。

### 6.3 capsule

- 加解密往返；密文与 base64 正文都不含明文；每次封装 nonce 不同；篡改/异源值被拒（`a_sealed_summary_round_trips`、`capsules_are_not_plaintext`、`each_seal_uses_a_fresh_nonce`、`corrupted_capsules_are_rejected`、`foreign_values_are_not_opened`）。
- 非本 Gateway 写的 compaction item 原样保留（`foreign_compaction_items_are_left_alone`）。

### 6.4 HTTP 挂载层（`tauri/tests/coding/proxy_gateway/codex_remote_compaction_http.rs`）

上面 6.1–6.3 都是纯函数/单传输层测试。**挂载点**另有一组真网关 + stub 上游的集成测试——纯函数全对但接线错了是这套代码最可能出的问题，而且这类错误单测看不出来：

- identity 路由端到端：v2 压缩轮次被改写（上游收不到 trigger/tools/`parallel_tool_calls`/`tool_choice`，指令被替换，末尾有产出指令，`stream: false`），客户端收到恰好 5 个 SSE 事件与一个 capsule，明文不入网（`identity_responses_compaction_is_rewritten_and_sealed`）。
- 非流式客户端：拿到的是 JSON 形态的同一 item，不是 SSE 外壳（`a_non_streaming_client_still_gets_one_compaction_item`）。
- 回放：下一轮把 item 回传后，上游收到 `Context summary from previous turns:` 开头的可读 assistant 消息，capsule 前缀 0 次出现，物品被展开而非透传（`a_replayed_capsule_becomes_readable_context_upstream`）。
- 空摘要：请求以 400 + `gateway_request_schema_rejected` 失败，不产出 item、不 seal 空 capsule（`an_empty_summary_fails_instead_of_faking_an_item`）。

### 6.5 回归

- 非 compaction 请求不受影响；其它 CLI 永不进入该分支（`other_clis_are_never_compaction`）。
- 现有 `/responses/compact` facade 不受影响。
- `cargo test --lib` 全量 + `pnpm test` + `pnpm exec tsc --noEmit`。

## 7. 参考项目对照

| 项目 | 做法 | 本设计取舍 |
|---|---|---|
| cc-switch PR #5536 | Chat 路径实现；摘要 base64 自包含进 `encrypted_content`；Anthropic 未实现；Responses 直通未处理 | **采纳**无状态自包含思路与三个 review 修复点；**改**用 AES-GCM 而非 base64（见 §3.5）；**补齐** Anthropic / Gemini / Responses 直通 / WebSocket |
| CLIProxyAPI | `antigravity_compaction.go`：AES-GCM capsule、四协议摘要回退、5 事件序列、`role: developer` 恢复、WS 侧 `executeCompactionTriggerFromWebsocketContext` | **采纳**其加密与事件序列；本实现另加 v2 的 `compaction_trigger` 识别（CPA 走的是 legacy compact 端点） |
| cc-switch 远端压缩开关 | 写 `name = "OpenAI"` 触发 | 不修改默认值（用户决定先只记录，见 §8） |
| cc-switch catalog 阈值 | 仅 ProxyChat 写 `auto_compact_token_limit`，其余 profile 移除 | 未采纳（超出本轮范围，见 §5.1） |
| Codex 官方 | 服务端加密，密文只有官方能解 | 不伪造官方密文语义，用自识别前缀区分 |

## 8. 待决问题

1. **`name = "OpenAI"` 门控未动**：Codex 只对 `name == "OpenAI"`（或 Azure）的 provider 默认启用 v2（`model-provider/src/capabilities.rs:31-41`）。纯字符串比较，与 `base_url` 无关——所以直连 OpenAI 官方但 `base_url` 指向网关时，Codex 仍会走 v2。本轮按用户决定**只记录不修改**。若要为第三方 provider 打开，两条路：接管时写 `capabilities.remote_compaction = "v2"`（Codex 0.162+ 支持），或改 provider 名。`[features] remote_compaction_v2 = false` 是 **no-op**（`features/src/lib.rs:656-658` 已把该 flag 变成无操作）。
2. **`tools` 剥离范围**：历史里已有工具调用结果时，是否也要从 `input` 里剥离对应的 tool 定义（cc-switch 只移除顶层 `tools` 字段）。当前实现同样只移除顶层字段。
3. **catalog 的 `auto_compact_token_limit` 缺口**（§5.1）未补：没有它，Codex 可能不主动触发压缩。
4. **WS 下的摘要质量**：WS 只覆盖 Responses 直通，摘要由上游模型在"无工具 + 纯摘要指令"下产出，质量未做线上验证。

（初版设计中的"envelope 生命周期"与"多 Gateway 实例"两个问题，在无状态 envelope 下**不存在**，已移除。）

## 9. 附：Codex 侧源码索引

| 事实 | 位置 |
|---|---|
| v2 能力判定 | `codex-rs/model-provider/src/capabilities.rs:31-41` |
| `is_openai()` | `codex-rs/model-provider-info/src/lib.rs:616-618` |
| 错误串 | `codex-rs/core/src/compact_remote_v2.rs:487` |
| 响应校验 | `codex-rs/core/src/compact_remote_v2.rs:441-500` |
| trigger 注入 | `codex-rs/core/src/compact_remote_v2_attempt.rs:95` |
| 历史保留 | `codex-rs/core/src/compact_remote_v2.rs:519` |
| wire 序列化 | `codex-rs/protocol/src/models.rs:1229-1237` |
| 废弃 feature flag | `codex-rs/features/src/lib.rs:656-658, 1979-1982` |
| capabilities 覆盖（0.162+） | `codex-rs/model-provider-info/src/capabilities.rs` |
| compaction 走 `stream()`（含 WS） | `codex-rs/core/src/client.rs:2237-2288` |
| WS 帧是裸 JSON | `codex-rs/codex-api/src/endpoint/responses_websocket.rs:731-745` |
| WS 请求构造 | `codex-rs/core/src/client.rs:2033-2075` |
| 自定义 provider 默认关 WS | `codex-rs/model-provider-info/src/lib.rs:775` |

### 9.1 cc-switch 参考实现索引

PR #5536（`refs/pull/5536/head`，截至 2026-10-07 **未合并**）。本地复现：`git fetch origin pull/5536/head:pr-5536`。

| 事实 | 位置（`main...pr-5536` 的 head） |
|---|---|
| 请求识别（两代协议） | `src-tauri/src/proxy/providers/codex.rs` `is_codex_remote_compaction_request` |
| 请求改写（摘要请求） | `src-tauri/src/proxy/providers/transform_codex_chat.rs` `responses_compaction_to_chat_completions_with_reasoning` |
| 历史 system 降级 | 同上 `take_compaction_historical_instructions` |
| 响应合成（Chat → compaction） | 同上 `chat_completion_to_compaction_response` |
| envelope 编解码 | 同上 `encode_local_compaction_summary` / `decode_local_compaction_summary` |
| 回填还原 | 同上 `append_responses_item_as_chat_message` 的 `compaction` 分支 |
| SSE 合成 | `src-tauri/src/proxy/providers/codex_responses_sse.rs` `compaction_completed` |
| 路由挂载 | `src-tauri/src/proxy/handlers.rs` `handle_codex_chat_compaction_transform` |
| forwarder 接入 | `src-tauri/src/proxy/forwarder.rs` `codex_chat_remote_compaction` |
| catalog 阈值 | `src-tauri/src/codex_config.rs` `codex_catalog_model_entry` |

### 9.2 CLIProxyAPI 参考实现索引

本地复现：`D:\GitHub\cli-proxy-api`。

| 事实 | 位置 |
|---|---|
| AES-GCM capsule 与四协议摘要抽取 | `internal/runtime/executor/helps/antigravity_compaction.go` |
| 5 事件流合成 | 同上 `BuildAntigravityCompactionStreamChunks` |
| WS 侧 compaction 分支 | `internal/runtime/executor/xai_websockets_executor.go:450-463`、`:946-1027` |
| 已消费 trigger 的重放防护 | 同仓库提交 `45c90e8d`（"drop consumed compaction triggers"） |

注：CPA 处理的是 legacy compact 端点（`opts.Alt == "responses/compact"`），本实现处理 v2 的 `compaction_trigger`，两者的**响应合成**可对齐，**请求识别**不同。

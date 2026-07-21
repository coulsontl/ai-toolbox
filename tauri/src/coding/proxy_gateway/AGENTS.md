# Proxy Gateway Module Notes

## 一句话职责

- 提供本机代理网关运行态、CLI 接管状态、配置备份恢复、请求详情文件、数据库请求摘要/统计和模型级健康状态。

## Source of Truth

- 全局网关设置来自 AI Toolbox 主数据库的 `proxy_gateway_settings`；必须直接读写 SQLite JSONB，旧 SurrealDB 仅用于启动时一次性导入。CLI 接管状态不进数据库，以 `proxy-gateway/cli-proxy/<cli>/manifest.json` 为准。
- CLI manifest 只保存接管元数据、目标文件路径、备份相对路径、hash/size、被管理字段、`mode` 和 `primary_provider_id`；不要写 settings_config、API key 明文或上游渠道配置。
- `manifest.mode` 是 single/failover 的事实源。被接管 CLI 的 runtime 配置内容不区分 single 和 failover；网关运行时根据 manifest 选择候选列表形态：single 只返回 P0，failover 把 P0 提到队首后再接其他 provider。
- 被接管 CLI 的真实运行时配置仍在各 CLI 自己的 runtime root：Claude Code `settings.json`、Codex `config.toml`/`auth.json`、Grok CLI `config.toml`/`auth.json`、Gemini CLI `.env`/`settings.json`。
- 请求列表和统计页的 Source of Truth 是 SQLite 中的 `proxy_request_logs` 请求摘要表和 `usage_daily_rollups` 日聚合表；这些表只保存 provider/model/token/cost/status/latency/时间，以及脱敏后的 `route_name` / `method` / `path` 等列表展示摘要字段。
- 请求详情仍然以 `proxy-gateway/request-logs/*.jsonl` 文件为准。`body`、`headers`、`upstream_request_body`、`response_body`、provider attempt 明细和 failover 过程不要写入数据库。数据库里的 `detail_file` / `detail_offset` 只是 JSONL 定位索引，用于 O(1) seek 详情行，不是详情内容存储。
- 当 `metrics_enabled=true` 但 `request_log_enabled=false` 时，详情文件可能不存在；详情命令可以从 SQLite 摘要降级返回 provider/model/token/status/latency 等基础字段，但仍不能把 body/header/attempt 明细写入数据库。
- 模型健康的持久化文件仍是 `proxy-gateway/model-health.json`；网关运行时的 Source of Truth 是启动时加载的内存 `ModelHealthRegistry`，请求路径只读写内存，变更后异步 flush，停止时最终保存。命令读取健康列表时应优先读运行时 registry，再回退文件。
- provider 的网关元数据放在各 CLI provider JSONB `data.meta` 中，不新增物理 provider 表列。内置渠道只持久化 `data.meta.gatewayProfile={tool,profileId,endpointId}` 引用和用户覆盖项；运行时读取 provider 时再从当前 `gateway_provider_profiles.json` cache/bundled defaults 动态解析 effective `provider_type`、`api_format`、`api_key_field`、`reasoning_field`、`codex_chat_reasoning`、图片策略和 `default_max_tokens`。`provider_type`、`cost_multiplier`、`pricing_model_source` 会随运行时 provider 读取进入请求摘要和成本计算。
- effective `providerType` 是后续 provider 专属兼容规则的识别键。内置供应商必须来自 `gatewayProfile` 指向的 profile；legacy provider 可以继续使用已保存的显式 `data.meta.providerType`，但自定义供应商不应靠模型名或 Base URL 伪造供应商身份。
- effective `apiFormat` 是上游真实目标协议。内置供应商必须来自 `gatewayProfile` 指向的 endpoint；普通自定义供应商才允许用户手动选择并保存 `data.meta.apiFormat`。
- `gateway_provider_profiles.json` 的 `compat` 字段只是 catalog 描述，但 `provider_profiles.rs` 会校验 compat 名称白名单。新增 compat 名称时必须同时补 runtime body/stream/header adapter、回归测试和白名单；不要只在 JSON 里声明一个尚无实现的兼容能力。
- 上游 provider `base_url` 以 `##` 结尾表示 AxonHub 风格 RawURL：runtime 读取 provider 时必须剥离 `##` 并把 provider 标记为 full URL，转发时只合并 query，不再追加 `/v1/chat/completions`、`/v1/messages`、`/v1/responses` 或 Gemini 默认路径。显式 `data.meta.is_full_url` / `isFullUrl` 仍然保留为等价配置。
- 每 CLI 的默认计费配置存放在 `ProxyGatewaySettings.app_configs` 中，只在 provider 记录没有显式 `data.meta.cost_multiplier` / `data.meta.pricing_model_source` 时作为缺省值；不要把默认配置误实现成覆盖所有 provider 的强制全局倍率。
- `model_pricing` 是独立 SQLite 物理表，不是 JSONB helper 表。模型定价 CRUD 必须直接查询/写入这张表，并继续使用字符串形式保存每百万 token 成本。官方默认价来自 bundled/cache/remote `model_pricing.json`，只允许 `INSERT OR IGNORE` 增量补齐，不能覆盖已有行。
- `ProxyGatewaySettings.enabled_on_startup` 表示上次应用退出前的网关运行态，不是用户可见的独立开关。启动成功后置 `true`，用户手动停止成功前置 `false`，应用启动时按它自动恢复网关。
- 网关“重启”是独立于“停止”的热重启命令：不做 CLI 接管 preflight、不改 manifest、尽量保持当前 host/port，并重建 runtime（清空 provider cache / side stores，重置模型健康冷却并写回 `model-health.json`）。请求历史与统计不删。
- 重启 bind 只临时关闭 `port_auto_select`，不要把用户设置永久写成 `false`；同端口释放有短重试。若重启在 stop 之后 start 失败，必须明确返回“网关已停止”错误，并提示重新启动或先恢复 CLI 直连；命令层无论成功失败都要按最终 running 状态 emit `gateway-running-changed`，避免中途失败后其它 UI 仍以为网关在跑。

## 核心设计决策（Why）

- CLI 接管使用文件 manifest，而不是数据库状态，原因是接管必须跟随本机 runtime 文件恢复，即使数据库记录损坏或迁移，仍能根据 manifest 找到备份并回滚。
- `OpenCode` adapter 暂不属于当前 MVP；不要把 `GatewayCliKey::OpenCode` 当成可接管 CLI 开启入口。
- 停止网关前必须做后端硬检查：只要存在 enabled manifest，就拒绝停止，要求先恢复对应 CLI 直连，避免用户 CLI 被留在不可用的本机网关地址上。
- 重启网关不能走 stop preflight，也不能实现成前端 `stop + start`。有接管时重启必须仍可用，否则网络切换后的半死状态只能靠用户先恢复直连才能自愈。
- 重新接管时必须复用已有 manifest 的原始备份，不要把已经被网关改写过的文件再次备份成“原始状态”。

## 关键流程

```mermaid
sequenceDiagram
  participant UI as CLI Page
  participant Cmd as Tauri Command
  participant Manifest as manifest.json
  participant Runtime as CLI Runtime Files

  UI->>Cmd: proxy_gateway_engage_single(cli, provider_id)
  Cmd->>Manifest: read existing enabled manifest
  Cmd->>Runtime: backup original files if needed
  Cmd->>Runtime: write gateway-managed config fields
  Cmd->>Manifest: write enabled manifest with mode=single and primary_provider_id
  Cmd-->>UI: takeover status
```

```mermaid
sequenceDiagram
  participant Settings as Gateway Settings
  participant Cmd as proxy_gateway_stop
  participant Manifest as CLI manifests
  participant Runtime as Gateway Runtime

  Settings->>Cmd: stop gateway
  Cmd->>Manifest: scan supported CLI manifests
  alt any manifest enabled
    Cmd-->>Settings: reject and list blocking CLIs
  else no CLI takeover
    Cmd->>Runtime: stop listener
    Cmd-->>Settings: stopped status
  end
```

## 易错点与历史坑（Gotchas）

- 不要用 `enabled_cli_keys` 表示“当前已接管”。它只是旧设置兼容字段；实际接管状态看 manifest。
- 不要把 UI 的停止前检查当成安全边界。全局停止保护必须在 `proxy_gateway_stop` 后端命令里执行。
- 接管状态必须先读取 enabled manifest，再处理 provider 候选加载错误。只要 manifest 表示 CLI 已被接管，即使 provider 表损坏、API key 缺失或 settings_config 解析失败，也必须保留恢复直连入口并阻止停止网关。
- 不要让保存设置时的隐藏字段把运行态恢复标记清掉。网关运行中保存设置时应保留 `enabled_on_startup=true`。
- 网关运行中保存日志/metrics 设置时必须同步更新运行态共享 settings，不能只写数据库；否则关闭 body/header 日志后重启前仍会继续落盘敏感内容。
- 控制台调试日志不等同于文件请求日志。文件请求日志必须按设置处理 headers/body 的脱敏、体积上限和保留策略；`/health` 这类健康检查不记录请求日志和 metrics。
- 网关请求路径不要向控制台打印 request/response debug 日志；请求排障需要走受设置控制的 JSONL 请求详情和 SQLite 摘要，不要重新引入 `println!`/`eprintln!` 级别的请求体、header 或上游响应输出。
- CLI 接管入口的根路径探测也属于本地探测，不是真实模型请求：Claude `GET/HEAD /anthropic`、Codex `GET/HEAD /openai/v1`、Grok `GET/HEAD /grok/v1`、Gemini `GET/HEAD /gemini/v1beta` 必须本地响应，不能进入上游 provider failover、SQLite 请求摘要、JSONL 请求详情或模型健康计分。无模型探测污染健康状态会导致后续真实请求被错误冷却。
- 请求摘要/统计可以写数据库，但必须保持 compact：不要把 body/header/attempt/response 这类大字段或敏感详情写进 SQLite。`route_name` / `method` / `path` 只允许保存脱敏后的轻量请求类型摘要，用于列表识别模型列表、上下文压缩、连接探测等无模型请求；详情展示需要继续按 trace id 读取 JSONL 文件。
- `cost_multiplier` 和 `pricing_model_source` 都是成本计算必需的 compact 摘要字段。新增或调整 provider 计费语义时，必须同时保证运行时 response、`proxy_request_logs` 落库和 SQLite summary fallback 详情都保留这两个字段，不能只保存倍率而丢掉按请求模型/返回模型计费的选择。
- 入站 HTTP 读取必须保留 header/body 大小上限，不能按 `Content-Length` 无限读入内存；流式响应 usage collector 也必须保持 bounded buffer，不能用全量事件列表累计长会话。
- `proxy_request_logs` 要保持与 cc-switch 核心 usage schema 兼容：统计、计费和筛选不要依赖 `route_name`、`method`、`path`、body byte count 或其他 AI Toolbox 额外列；这些额外列只能服务请求列表展示和 SQLite summary fallback。`path` 写入前必须脱敏 query 中的 `key`、`api_key` / `api-key`、`access_token`、`refresh_token`、`client_secret` / `client-secret`、`token` 等鉴权参数。
- 只要 `request_log_enabled` 或 `metrics_enabled` 任一开启，就要写 compact 请求摘要；否则请求 Tab 列表和统计页会丢当前请求。只有 `request_log_enabled=true` 时才写 JSONL 详情文件。
- 旧 metrics rollup 文件入口已经废弃；`metrics_enabled` 现在表示写入 SQLite compact 请求摘要供统计页使用，不再维护文件 rollup API。
- 请求日志里 `request_body` 表示网关收到的原始请求体，`upstream_request_body` 表示实际发往上游的请求体。两者都受 `store_request_body` 控制；后续新增请求体改写能力时必须同步保存上游快照，否则 UI 无法对比整流前后差异。
- 请求日志里 `upstream_response_body` 表示上游返回给网关、尚未转换的原始响应，`response_body` 表示网关最终返回给客户端的响应。两者都受 `store_response_body` 控制；非流式响应可以在转换前保留原始 body，流式响应只能保存 bounded snapshot，不能为了日志 full-buffer SSE。
- 请求详情导出必须同时脱敏 JSON 字段、header 值和 URL/query 文本中的鉴权参数。尤其是 Gemini 常见 `?key=...`、`api_key` / `api-key`、`access_token`、`refresh_token`、`client_secret` / `client-secret`、`token` 这类 query 不能以明文出现在 `summary.path`、`routing.upstream_url` 或 provider attempts 中。
- SSE/流式响应必须边读边写回客户端，不能为了日志、统计或 token 解析先 `bytes().await` 全量缓冲；统计采集只能在透传过程中维护 bounded snapshot 和 usage collector。
- 网关运行态必须保持 tokio async 链路：监听使用 `tokio::net::TcpListener`，连接处理使用 `tokio::spawn`，HTTP 读写和流式 body 写回都 `.await`。不要在请求路径里重新引入 `std::thread + block_on` 或 thread-per-connection。
- 流式 failover 只有在首个有协议意义的 chunk 到达后才算当前 provider 成功；单纯 SSE 控制事件、heartbeat、created/completed 但没有文本/工具/候选等实际内容时仍应按 empty response 失败并允许重试/故障转移。完全没有收到非空 chunk 的首包前断流仍按 timeout 处理。写回客户端时每个 chunk 读取都必须套 idle timeout，避免上游半开连接永久挂住。
- Gateway 连通性测试的 `timeoutSecs` 是从请求开始计算的整体预算，路由/建连/首字节和后续流读取共享同一个 deadline，不能在拿到 response 后重新分配完整时长。HTTP 2xx 流也必须把 `event: response.failed`、`type: response.failed` 和嵌套 `response.error` / failure status 判为失败。
- 非流式 2xx/3xx 响应也必须做 empty response detection：body 为空、或协议 JSON 没有 Chat choices / Responses output / Anthropic content / Gemini candidates 等实际内容时，按 `empty_response` 失败处理并进入现有同 provider retry / 跨 provider failover；不能把 200 空 body 当成功返回给 CLI。
- Header 大小写保真不能依赖 `reqwest::HeaderMap`，`HeaderName::from_bytes` 会规范化名称。需要保留原始大小写时走 `runtime/header_preserving_client.rs` 的原始 HTTP/1.1 写出路径；系统代理和 SOCKS 代理场景继续回退现有 reqwest 路径以尊重用户代理设置。
- `proxy_request_logs.latency_ms` 表示首 token/首 chunk 延迟；非流式或拿不到首包时间时才退回完整请求耗时。`duration_ms` 才表示完整请求耗时。
- 成本统计以 `model_pricing` 表和 `proxy_request_logs` 的 token 摘要计算，内部计算使用 `Decimal`，写库仍存字符串/数值文本格式。未知模型或未命中定价时 cost 可以为 0，但不能在写入路径把所有 cost 列固定写成 0。
- 更新模型定价只改变后续成本计算的价格来源；除非任务明确要求历史回填，不要在定价 CRUD 中重算 `proxy_request_logs` 或 `usage_daily_rollups`，避免把管理弹窗变成高成本数据迁移入口。
- 官方模型定价同步的核心保护是 `INSERT OR IGNORE`：用户编辑过的同 model_id 价格、迁移期已存在价格、以及手动新增价格都不能被 bundled 或 remote 默认值覆盖。用户删除某个默认模型价后，下一次启动或手动同步会按默认数据重新插入缺失行。
- 请求摘要中的 `input_tokens` 语义是“非缓存输入 token”，`cache_read_tokens` / `cache_creation_tokens` 单独记录缓存 token；`total_tokens` 和成本计算按这些分量相加/分别计价。OpenAI/Gemini 这类上游返回的 prompt/input 总数若包含 cached tokens，解析层要先拆分，成本层不能再二次扣减 cache。
- Anthropic usage 解析默认按官方语义处理：`input_tokens` 已是 fresh input，`cache_read_input_tokens` / `cache_creation_input_tokens` 另记。Moonshot/Kimi Anthropic-compatible target 是例外，provider-aware usage parser 必须按 `providerType=moonshot/kimi` 识别 `cached_tokens`，并处理 Moonshot 可能返回的负 `input_tokens` 折扣形态，最终仍写入本仓库统一语义的 fresh `input_tokens` + `cache_read_tokens`。不要在成本层再对 Moonshot 做第二次 cache 扣减。
- 请求 Tab 列表保留所有 compact 摘要供排障；统计页只统计真实模型请求和 Codex/OpenAI Responses Compact 请求。`usage_daily_rollups` 没有 `method` / `path`，只有无有效模型的 Compact 才使用内部模型哨兵保留在总量、趋势和 provider 统计里，并在模型维度隐藏；带有效模型的 Compact 仍按模型计入统计。不要把模型列表、连接探测或其他无模型请求聚合成 `unknown` 参与统计。
- `usage_daily_rollups` 聚合/裁剪不能放在每个请求的热路径里高频执行；如果需要触发，必须有节流或后台任务。
- 模型健康快照只持久化非健康状态。失败进入 degraded/cooling/probing 后写快照；恢复 healthy 后移除对应条目，避免后续成功请求继续重复写快照。
- 模型健康列表里的 provider id 只是稳定键，返回前应尽量从 Claude/Codex/Grok/Gemini provider 表注入 `provider_name`，避免前端展示数据库原始 ID。
- 模型健康过滤只在故障转移模式生效。单渠道代理只有一个 provider 候选时，即使模型健康处于 cooling/down 或 degraded，也必须始终尝试转发，避免单渠道被冷却后直接 502。
- 恢复直连时只恢复本模块管理的配置字段，尽量保留 CLI runtime 自己新增的未知字段和 OAuth/token 等运行时拥有字段。
- 配置写入要继续使用各 CLI 的 runtime location 解析结果，不要硬编码 `~/.claude`、`~/.codex`、`~/.grok` 或 `~/.gemini`。
- WSL Direct 接管地址替换只在 runtime location 的 `mode == RuntimeLocationMode::WslDirect` 且 `ProxyGatewaySettings.wsl_host` 非空时生效：写入 CLI runtime 配置和 manifest 前，把运行中网关的 `http://host:port` origin 中的 host 替换为 `wsl_host`，端口和 scheme 保持不变；drift 检测必须使用同一套有效 origin 计算。
- 普通 Windows->WSL 同步不是 WSL Direct 接管。开启/切换/恢复 Gateway 接管成功后只发 `wsl-sync-request-*` 事件，让 WSL 模块按用户设置决定是否同步；WSL 目标副本的地址改写必须走 `cli_proxy` 的 manifest + sentinel + managed fields 校验，只改 Gateway 托管字段，不能全局替换 loopback，也不能污染 Windows runtime 文件。
- Claude/Codex/Grok/Gemini 的 `category=official` provider 代表 CLI 原生 OAuth 官方订阅，只能由 CLI 自己直连使用，不存储可转发 API key；网关 provider 候选列表和 CLI 接管前置校验必须跳过这类 provider。接管状态/卡片 UI 可提示“官方订阅不参与代理”，但不要把它当成可代理渠道。
- Codex 第三方接管对齐 cc-switch：就地改写**当前** `model_provider` 对应表的 `base_url`/`wire_api`（缺省 id 为 `custom`），**不要**再插入 `ai-toolbox-gateway` 第二段，也不要切换顶层 `model_provider` id，否则会话按 `model_provider` 分桶会像“吞掉 custom 会话”。旧版残留的 `[model_providers.ai-toolbox-gateway]` 在 patch/restore 时删除，恢复时也不能写回备份里的 legacy 顶层 `model_provider = "ai-toolbox-gateway"`；若备份缺少本次接管创建/改写的 active provider 表，必须清掉 Gateway 写入字段并在空表时删除该表。`codex_preserve_official_auth_on_switch` 关闭时仍把 Gateway client token 写入 `auth.json` 的 `OPENAI_API_KEY`/`auth_mode=apikey`；开启时把 token 写到**当前 active** 表的 `experimental_bearer_token`，并按 manifest 原始备份恢复/清理 `auth.json` 受管字段。漂移检测读 active 表 `base_url`（兼容旧 sentinel）。
- single 模式下候选列表只有 P0，P0 请求失败不切换其他渠道；如果配置了同渠道重试，会先按 retry interval 重试 P0，耗尽后再把上游失败返回给客户端。
- single/failover 模式只要 manifest 仍是 enabled，普通 provider apply/select 入口就必须拒绝硬切换。原因不是 manifest 保存了渠道完整配置，而是接管期间 manifest、CLI runtime 文件、原始备份和 P0 provider 必须保持同一个状态机：manifest 记录 `primary_provider_id`、被管理文件、备份路径和 managed fields，Gateway runtime 也按它决定 single/failover 候选。如果绕过恢复直连直接切换 provider，普通 apply 会重写 runtime 文件，破坏 Gateway 托管字段和备份恢复语义；随后恢复直连仍会按接管时的备份回滚，可能覆盖代理期间改动或让 P0 与实际 runtime 状态不一致。允许切换 P0 时必须走专用 Gateway-aware 编排入口，并且页面 provider 卡片和系统托盘 provider 菜单必须复用同一条后端链路：先恢复直连，再应用目标 provider，再重新开启 single；如果原状态是 failover，还必须在 single 接管成功后重新开启 failover。编排内部应用目标 provider 时不要触发中间 `config-changed` 或 WSL 同步，只在重新接管完成后发一次最终 `config-changed`（托盘入口保持 `tray` payload）和最终同步事件，避免窗口/远端短暂刷新到直连中间态。failover 模式下 P0 固定为 manifest 的 `primary_provider_id`。
- 网关接管期间允许编辑“正在被代理的已应用渠道”，但保存后必须走 Gateway-aware 重接管链路：恢复直连、重新应用当前 provider、再按原模式重新开启 single/failover。不能依赖各 CLI `update_*_provider` 的普通 auto-apply，因为接管期间 runtime 文件由 Gateway 托管。通用配置保存、配置根目录保存/恢复默认仍必须在前端保存入口拦截并提示先恢复直连；这些入口不是 provider 专用保存，不能安全自动恢复接管。未应用渠道（包括 failover 的 PN 候选）编辑只更新 DB，不写 runtime 文件，安全，必须保持可编辑，不要按 CLI 级 `gatewayProxyActive` 一刀切禁用整列。
- PN family 兜底继续沿用运行时模型映射规则：PN.haikuModel/sonnetModel/opusModel/fableModel/reasoningModel 未配置时先用 PN.model，PN.model 也没有时才用请求里的标准模型名；Fable 未配置时先回退 Opus，再回退 PN.model。
- Claude family 模型映射由 manifest mode 决定：`single` 模式必须保持请求里的原始模型名直透，仅剥离 `[1M]` / `[1m]` 上下文标记；`failover` 模式必须继续按 provider family 映射转发，即使当前有效候选只剩 P0 一个 provider。
- Codex / Grok 没有 Claude 那种多 role family 映射；它们只有每个 provider 自己的默认模型。默认模型来源：Codex 读 `settings_config.config` 中的 `[chat].model`，没有时回退根级 `model`，以兼容前端仍保留的旧字段形态；Grok 读当前选中 model table 的 `model`/`key`。不要把 Codex `modelCatalog` / `model_catalog_json` 误当成这套默认模型映射源。
- 模型改写门控按 CLI 区分：Claude family 与 Codex default-model 只在 `failover` 且非连通性测试（无 `provider_override_id`）时生效；`single` 下 Claude/Codex 原样透传请求模型（仅剥 `[1M]`）。Grok 例外：接管写盘在 single/failover 都会把 CLI 请求模型固定成 `grok-build`，因此只要允许 provider 映射（非连通性测试），有 `default_model` 时 single 与 failover 都改写为真实默认模型；连通性测试仍保留用户点选的模型。没有默认模型时仍透传请求模型。这样跨渠道故障转移和 Grok 单渠道代理都不需要为了对齐模型而重启 CLI。
- Provider `data.meta.apiFormat` 表示上游真实目标协议，不表示入站 CLI 协议，也不要写进 CLI runtime 配置文件。Gateway runtime 先从 route 推导入站 `AiProtocol`，再与 provider 的 target protocol 组成 `ConversionRoute`；只有 source/target 不一致时才调用 `transformer`，一致时保持原有直通行为。
- Provider body 兼容规则属于 Gateway runtime outbound adapter 层，应基于 effective `providerType + apiFormat` 做窄范围处理；effective meta 可以来自 `gatewayProfile` 动态解析，也可以来自 legacy 显式 meta。不要把 DeepSeek/Moonshot/Zai/Doubao/Grok/Longcat/ModelScope/Bailian/Ollama 等供应商专属限制下沉到通用 `transformer` 协议转换语义里。
- Anthropic platform 兼容也属于 runtime 层，必须按 `providerType + target_protocol` 判断，不能只看 providerType 字符串：`bedrock` / `anthropic-bedrock` / `aws-bedrock` 在 Anthropic target 下走 Bedrock Claude Messages，`vertex` / `anthropic-vertex` / `claude-vertex` 在 Anthropic target 下走 Anthropic Vertex，`google-vertex` / `gemini-vertex` 在 Gemini target 下才走 Gemini Vertex。Bedrock Anthropic URL 必须使用 `/model/{model}/invoke` 或 `/model/{model}/invoke-with-response-stream`，header/body 使用 `anthropic-version: bedrock-2023-05-31`，最终 body 清理 `model` 和 `stream`；Vertex Anthropic URL 使用 base URL 已包含 project/location 的 `publishers/anthropic/models/{model}:rawPredict` 或 `:streamRawPredict`，header/body 使用 `anthropic-version: vertex-2023-10-16`。LongCat 和 Bedrock Anthropic target 使用 Bearer auth。Direct Anthropic body 含 native `web_search` tool 时要注入 `anthropic-beta: web-search-2025-03-05`；非 Direct/非 Bedrock Anthropic platform 要过滤 native web_search tool，避免发到不支持的平台。
- Codex official upstream 兼容使用 `providerType=codex` / `openai-codex` / `chatgpt-codex` / `codex-official` 且 target protocol 为 OpenAI Responses 的 runtime adapter。它不会改变“`category=official` provider 不参与 Gateway 候选”的安全边界；候选仍必须有可转发 bearer token。该 adapter 会强制 `stream=true`、`store=false`、`parallel_tool_calls=true`，删除 `max_tokens` / `max_completion_tokens` / `metadata`，默认补 `include:["reasoning.encrypted_content"]` 和 `reasoning.summary="auto"`；headers 会补 `Accept: text/event-stream`、缺省 `Originator: ai-toolbox`，并在缺少 `Session_id` 时从 `X-Codex-Turn-Metadata.session_id` 推导。客户端已有的 `Originator`、`User-Agent`、`Session_id`、`Chatgpt-Account-Id` 和 Codex passthrough headers 必须保留。由于官方 Codex 上游被强制流式，客户端请求非流式时 runtime 必须把上游 Responses SSE 聚合成非流式 JSON，再按需要执行同一次 response 协议转换；客户端本身请求流式时仍走 SSE wrapper，不做 full-buffer。
- OpenAI Responses target 当前只实现 HTTP JSON 和 HTTP SSE transport。AxonHub `origin/unstable` 已有 `llm/transformer/openai/responses/websocket_executor.go`，但 ai-toolbox 还没有 provider profile `transport` schema、runtime executor/session/header/pool 和非流聚合语义。不要在 transformer 内补半套 WebSocket；需要单独设计 runtime transport 后，再接入 Responses WebSocket executor 级能力。
- OpenAI Responses `/responses/compact` 是独立 endpoint，不属于普通聊天协议跨转换矩阵，也不要新增 `AiProtocol` 行。Gateway runtime 通过 `runtime/compat/codex_responses_compact.rs` 单独识别 Codex compact 请求：OpenAI Responses target 保持原 compact path 直通；OpenAI Chat、Anthropic Messages、Gemini Native target 走 compact 专项 IR/facade fallback，并把响应转回 `response.compaction`。显式 `stream:true` 或 `?stream=true` 必须在发送上游前拒绝；compact 路由上的本地请求 schema 拒绝和上游非 2xx fallback 错误也必须返回 OpenAI Responses 风格 `{error:{message,type,param,code}}` envelope，避免 Codex 客户端收到网关私有 `{error,message}` shape。
- 非流客户端的 SSE auto-aggregate 不再限定 Codex official：只要上游返回 `text/event-stream`，且客户端请求和 route 都没有声明 streaming，runtime 会按 target protocol 聚合为同协议非流 JSON，再按同一次 response conversion route 转回客户端协议。当前覆盖 OpenAI Responses SSE → Responses JSON、OpenAI Chat SSE → Chat JSON、Anthropic Messages SSE → Messages JSON、Gemini Native SSE → generateContent JSON。该能力仍是 runtime forced-stream 兜底，不替代正常流式请求的边读边转换。
- GitHub Copilot provider 兼容使用 `providerType=copilot` / `github-copilot` / `github_copilot` 的 runtime adapter。请求级 target protocol 会按 AxonHub 规则动态选择：`gpt-5+` / `gpt-6+` 等 `^gpt-(major)` 且 major >= 5 的模型走 OpenAI Responses，`gpt-5-mini` 和其他模型走 OpenAI Chat；这个选择只影响本次请求的 effective provider，不改 provider 记录。Copilot warmup 请求会在 route selection 前先把本次 effective model 降级为 `gpt-5-mini`：必须同时满足 provider 是 Copilot、请求头包含 `anthropic-beta`、body 不是 compact、`X-Initiator`/body 分类为 user、且没有 tools；带工具或 agent/subagent 的请求不能降级。该 adapter 还会在发送前把 GitHub access token 换成 Copilot bearer token：常见 GitHub token 前缀（`ghp_` / `github_pat_` / `gho_` / `ghu_` / `ghs_` / `ghr_`）会自动 exchange，或通过 `data.meta.apiKeyField=github_access_token` / `github_token` / `copilot_oauth` 等显式启用；raw Copilot token 继续按 Bearer 直发，不强制 exchange。exchange 请求使用全局 rustls HTTP client，结果按 token hash 缓存到过期前 5 分钟。该 adapter 还会注入并覆盖 Copilot fingerprint headers（`Editor-Version`、`Editor-Plugin-Version`、`User-Agent`、`Copilot-Integration-Id`、`Openai-Intent`、`X-Github-Api-Version`、`X-Vscode-User-Agent-Library-Version`）、按 body 重算 `X-Initiator`、检测 vision 内容写 `Copilot-Vision-Request:true`、为 subagent 写 `X-Interaction-Type:conversation-subagent`、基于 session/body 生成确定性 `X-Interaction-Id` / `X-Request-Id` / `X-Agent-Task-Id`，并做 Claude 4.x Copilot model ID 语法归一化、Chat/Responses orphan tool result 文本降级、Responses function call item id 修正。它不负责 GitHub device-code 登录 UI、账号存储或 live model list fallback；这些仍属于后续账号管理能力。不要把 cc-switch 的 Anthropic 原始 `merge_tool_results` 逐行搬进 transformer；ai-toolbox 当前 Copilot 出站 compat 应按最终 OpenAI Chat/Responses body 形态处理。
- Copilot 内置 provider profile 必须保存 origin base URL，不要用 `##` 固定 `/chat/completions` full URL。Copilot Chat endpoint 是 `/chat/completions`，GPT-5+ 动态切到 Responses 时 endpoint 是 `/responses`；固定 full URL 会绕过 runtime 的请求级 target protocol 切换。
- Ollama provider 兼容使用 `providerType=ollama` 或 `data.meta.apiFormat=ollama/chat`，target protocol 在 Gateway 中视作 OpenAI Chat，但 runtime 最后一跳使用 Ollama `/api/chat` wire format。发送前把最终 OpenAI Chat body 投影为 Ollama `model/messages/options/format/stream`：text content 合并为字符串，`image_url` 的 data URL 去掉前缀后写入 `images[]`，plain URL 原样保留，`max_tokens` / `max_completion_tokens` 写入 `options.num_predict`，`stop` 写入 `options.stop`，`response_format` 写入 Ollama `format`。非流 Ollama JSON response 先转 OpenAI Chat response，再按入站协议需要执行既有 response conversion；流式 Ollama `application/x-ndjson` 先转 OpenAI Chat SSE，再进入已有 SSE transformer。它不是新增第五个 transformer 协议，不能要求补 5×5 协议矩阵；full URL `##` 仍优先尊重用户提供的完整 URL。
- Codex -> OpenAI Chat 的多 vendor 推理参数矩阵由 runtime outbound adapter 执行，effective `codexChatReasoning` 优先；它通常从 `gatewayProfile` 指向的 Codex endpoint/profile 动态解析，也兼容 legacy 显式 meta。该配置可声明 `supportsThinking`、`supportsEffort`、`thinkingParam`、`effortParam`、`effortValueMode` 和 `outputFormat`，用于把 Codex/Responses 风格 `reasoning_effort` 映射到 DeepSeek、OpenRouter、SiliconFlow、DashScope/Qwen、Bailian、Kimi/Moonshot、GLM/Zai、MiniMax、Mimo、StepFun 等 Chat 兼容 provider 的 `thinking` / `enable_thinking` / `reasoning_split` / `reasoning.effort` / `reasoning_effort`。缺 `codexChatReasoning` 时，provider 专属 fallback 只能由明确的 effective `providerType` / `apiFormat` 触发；自定义渠道绝不能因为 body `model` 字符串包含 `deepseek`、`qwen`、`minimax`、`mimo` 等名称就套用对应 provider 规则。模型名只允许在已识别 provider 内做能力细分，例如 `providerType=stepfun` 后再用 `2603` 判断是否支持 effort。新增或修复 provider 兼容能力时，必须同步维护 `gateway_provider_profiles.json`、runtime profile resolver 和 runtime 回归测试，让用户通过明确预设或 legacy 显式 meta 选择兼容行为，而不是依赖模型名猜测。
- OpenAI Chat assistant 历史消息的 reasoning 字段策略也属于 runtime outbound adapter，不属于 transformer。provider 可通过 `data.meta.reasoningField` / `data.meta.reasoning_field` 控制最终出站字段：`reasoning_content`/`content`、`reasoning`、`none`、`all`；未显式配置时默认保持 `reasoning_content`，OpenRouter/NanoGPT 的 `providerType` 默认使用 `reasoning`。OpenRouter 还要把顶层 `reasoning_effort` 移入 `reasoning.effort`，并把 `max`/`xhigh` 归一为 `xhigh`。DeepSeek OpenAI Chat target 是更严格的最终门控：有非空 `tool_calls` 的 assistant 轮必须保留/回填 `reasoning_content`，无 `tool_calls` 的 assistant 轮必须剥离 `reasoning_content` 和 `reasoning`，避免纯文本多轮 400。
- DeepSeek legacy OpenAI Completion API 是 runtime passthrough 兼容，不属于 transformer 的聊天协议矩阵：Codex/OpenAI `/v1/completions` 或 `/completions` 入站不进 IR，也不套 OpenAI Chat body adapter；当 `providerType=deepseek` 时，上游 URL 要按 AxonHub 语义从 base URL 去掉尾部 `/v1` 后发送到 `/beta/completions`。不要把这条路径误扩展成 Completion ↔ Chat/Responses/Messages 的跨协议转换。
- Doubao/Volces OpenAI Chat target 的 `reasoning_effort` 不能按通用 Chat 字段直传：runtime adapter 必须改写为 `thinking.type=enabled|disabled` 对象，`none`/`off`/`disabled` 映射 disabled，其余非空 effort 映射 enabled；随后顶层 `reasoning_effort` 仍按通用 provider 兼容规则清理。
- Gemini Native target 且 `providerType` 为 `vertex` / `google-vertex` / `gemini-vertex` 时，runtime adapter 必须删除 `contents[].parts[].functionCall.id` 和 `functionResponse.id`。Vertex AI 不接受这些 ID；这只是出站 provider 兼容，不改变 transformer 内 Gemini provider-local signature 和 synthetic id 语义。
- Bailian/DashScope/Aliyun OpenAI Chat target 的 SSE 过滤属于 runtime provider stream adapter：见到 `tool_calls` 后，应从后续 delta 中剥离并缓冲 text content，在 finish chunk 前作为独立 text delta 重发；同一 tool call 已累计非空 arguments 后，上游再发 `{}` 参数片段时应清空该片段，避免污染已累计参数。该规则必须发生在 raw upstream SSE 转换前，确保直通 Chat 和 Chat->其他协议转换路径一致。
- xAI/Grok OpenAI Chat target 的 SSE 空 delta 过滤也属于 runtime provider stream adapter：只过滤 `choices[].delta` 为空且没有 finish/usage 的空事件，保留 role/content/tool_calls/finish/usage 事件。该规则必须发生在 raw upstream SSE 转换前，不能下沉到通用 transformer。
- Claude/Codex 的 provider target protocol 与 CLI 原生协议不一致时，不能走普通直连 apply。页面按钮应显示“应用并代理”，托盘菜单在网关未运行或不可接管时要置灰，后端普通 apply 也必须拒绝；可用时统一走 Gateway-aware provider switch 编排，先内部应用 provider，再开启 single 代理。
- 协议格式相同时必须走既有直通链路，而不是把请求/响应送进统一转换器再“转换回同格式”。这是 Gateway 调度原则，不是转换器 fallback：同格式场景只能做 runtime 既有的 path/header/auth、模型名改写、`[1M]` 剥离等处理，不能重写协议结构。
- 协议格式转换必须集中在 `transformer` 独立模块中。该模块不能依赖数据库、Tauri app handle、provider 表或 Gateway runtime context；新增 Gemini Native 等后续格式时继续扩展 `AiProtocol` / `ConversionRoute` / JSON/SSE 转换边界，不要把转换 helper 散落到 `runtime/upstream.rs`。
- 当 transformer 的请求转换返回 `ConversionContext` 时，runtime 必须把它原样带到同一次非流式 response 或 SSE response 转换。这个 context 是请求作用域内的协议映射状态，例如 Codex Responses `tool_search` / namespace 工具展平后需要在 Chat 响应里还原；它不是可落库、可跨请求复用或可由 runtime 解释的状态。
- 跨 HTTP 请求的协议补全状态属于 runtime `side_stores`，不属于 transformer：`CodexHistoryStore` 用于 Codex Responses follow-up 只带 `previous_response_id`/`function_call_output` 时补回上轮 assistant call item，当前只在 Responses -> Chat 和 Responses -> Anthropic 请求转换前运行；同协议 Responses target 不需要展开服务端历史，Gemini target 使用 `GeminiShadowStore` 在转换后的 Gemini body 上回放带 `thoughtSignature` 的上轮 model functionCall。side store 必须有容量上限，且只作为兼容缓存，不能成为 provider 配置或请求日志的 Source of Truth。Gemini shadow 只能在请求头、metadata、`previous_response_id` 或 `cachedContent` 等可靠会话线索存在时记录/回放；缺少线索时必须跳过，不能把所有请求归入 `"default"` 会话导致跨会话污染。
- Responses encrypted-content 负缓存必须绑定已解析 provider 配置身份，而不只是 provider ID；至少要隔离 base URL、target protocol、full URL/profile endpoint 和凭据变化，避免用户原地编辑同一 provider 后继续误删合法 reasoning item。
- `runtime/pipeline.rs` 和 `runtime/middleware.rs` 是 AxonHub 风格的轻量扩展骨架。`build_upstream_body_for_provider` 会为每个 request 构造 request-scoped pipeline context，并在转换前运行 inbound middleware、最终上游 body 发送前运行 outbound middleware。最终上游 JSON body 的 provider compat 已通过 `OutboundAdapterCompatMiddleware` 进入生产 pipeline，覆盖 ReasoningField、DeepSeek 门控、Codex→Chat reasoning 矩阵、Ollama 投影、预测式图片替换和通用 tool/reasoning 清理等现有规则；这些 helper 的实现仍在 `runtime/upstream.rs`，但挂载点必须继续保持 middleware 化。`BillingHeaderCchMiddleware` 只剥离 Claude Code billing header 中动态 `cch=...` 并存入 `PipelineContext`，Anthropic target 可回填到已有 billing header；非 Anthropic target 不能泄漏该动态字段。不要把它扩展成 API key/header 转发或替代 transformer 的非 Anthropic billing header 清理。`EnsureMaxTokensMiddleware` 只有 provider meta 显式声明 `defaultMaxTokens` / `default_max_tokens` 时才启用；无配置时不得默认改变用户请求。它按 target protocol 补齐/截断 `max_tokens`、`max_completion_tokens`、`max_output_tokens` 或 Gemini `generationConfig.maxOutputTokens`。`ChannelCustomizedExecutor` 已在 runtime pipeline 层具备可测试执行边界：customizer 可以替换默认 executor，错误会原样传播。side store、lossy 策略、response/stream adapter 和 rectifier 仍由 `upstream.rs` 请求编排负责；后续迁移时必须逐项搬迁并保留同等回归测试，不能只改目录结构。
- 协议转换后的请求体必须作为 `DebugHttpResponse.upstream_request_body` 保存，原始入站体仍由请求日志的 `request_body` 表示。直通路径可以保留当前模型改写/1M 标记剥离行为，但不能出现协议结构重写。
- 协议转换后的最终上游 body 可以在 `runtime/upstream.rs` 做窄范围 outbound adapter 兼容，职责是规避具体上游 provider 的严格校验，不是补充协议结构转换。当前规则：所有 JSON outbound body 最后递归过滤 `_` 开头的内部私有字段，但必须保留 JSON Schema `properties` / `patternProperties` / `definitions` / `$defs` 下的属性名；OpenAI Chat/Responses 或 Anthropic Messages target 在转换后没有非空 `tools` 时，清理 OpenAI 的 `tool_choice`/`parallel_tool_calls` 或 Anthropic 的 `tool_choice`；Gemini Native source 的 `toolConfig.functionCallingConfig` 可独立表达 tool choice，不能用这条规则清掉 Gemini-derived `tool_choice`。发往 OpenAI Chat target 时，无论是否发生协议转换，都要做第三方 Chat 基础兼容：`developer` 归一为 `system`、所有 system 消息合并到首条、纯文本 system content parts 压成字符串、空 tool call arguments 归一为 `{}`，清理 Responses/Codex 专属扩展（`responses_custom_tool` 工具和对应 custom tool call/result 历史、常见不支持的 `verbosity`/`reasoning_effort`/`prompt_cache_key`），并删除 tool call 上携带实际值的 Google 私有 `thought_signature` 容器，避免把 Gemini/OpenAI-Google 额外字段发给普通 Chat provider。这些清理只属于 provider 兼容层，不要下沉到 transformer roundtrip 语义。
- OpenAI Responses target 的 `prompt_cache_key` fallback 属于 runtime 请求上下文兼容：当最终 Responses body 没有非空 `prompt_cache_key` 时，`runtime/upstream.rs` 可从稳定会话 header/body 字段（如 `X-Session-Id`、`x-ai-toolbox-session-id`、`metadata.session_id`、`previous_response_id`）补齐；显式 `prompt_cache_key` 不能被覆盖，完全没有会话线索时也不要写入 `"default"`。
- 有损转换检测由 transformer 提供纯检测，runtime 负责策略执行。`ProxyGatewaySettings.lossy_rejection_enabled` 默认关闭：默认 best-effort 放过并在最终响应写 `X-Transformer-Lossy`，避免真实 CLI 请求因可降级字段被直接阻断；用户显式开启后，检测到明确不可逆字段才返回本地 400，且不计入 provider health/failover。请求头 `X-Allow-Lossy: true` 仍可绕过显式硬拒绝。当前检测按源协议覆盖 OpenAI Chat、OpenAI Responses、Anthropic Messages、Gemini Native 的明确不可逆字段：Chat audio/modalities/未知 content part/parallel tool calls，Responses code/computer/local shell/search/MCP/image generation/compaction call/item 与高风险 hosted tool declaration，Anthropic native/server tool definition 与 block，Gemini native tools、Gemini-only generation config、cachedContent/safetySettings 和非图片媒体。Responses 顶层 `web_search` / `web_search_preview` / `image_generation` 只是 hosted tool 声明，Responses→Chat 时会被过滤，不默认作为 blocking lossy issue；已经发生的 `web_search_call` / `image_generation_call` 仍是 lossy issue。新增不可逆字段时必须先补 `LossyIssue` 和测试。
- OpenAI Chat response 中 leading `<think>...</think>` 的 reasoning 提取属于 transformer，非流式和 Chat source stream 主路径都在 transformer 内处理。不要在 runtime provider adapter 中再写一套字符串解析；stream FSM 需要继续保证跨 chunk partial tag 不泄漏，并在 tool/finish 边界 flush 缓冲。
- 图片/多模态兼容有两层：发送前的预测式替换和上游错误后的反应式同 provider 重试。预测式替换只允许由 provider meta 或 `settings_config.modelCatalog.models` 的显式能力驱动：`imageInputPolicy=strip/text_only`、`textOnlyModels`、或模型 catalog 中 `supportsImage=false` / `attachment=false` / `modalities.input` 不含 `image` 时，把最终上游 body 里的图片块替换为 `[Unsupported Image]`；`imageCapableModels` 或 catalog 明确支持 image 时必须保留图片。`allowTextOnlyModelHeuristic=true` 才能启用 cc-switch 风格 text-only 模型名启发式，默认不猜。反应式兜底仍只在 400/415/422/501 响应明确表示 image/media/attachment/vision 等内容不支持时重试一次。替换时要保留原图片块的 `cache_control`，Gemini `inlineData`/`fileData` 只有 `mimeType` 明确为 `image/*` 才替换。
- SSE 协议转换必须用 stream wrapper 边读边转换，不能 full-buffer。结束事件要保持幂等：OpenAI `[DONE]`、Responses `response.completed`、Anthropic `message_stop` 或 Chat `finish_reason` 可能重复/组合出现，转换器只能向客户端输出一组完成事件。
- 上游鉴权和必需 header 必须按 target protocol 判断，而不是按入站 CLI 判断。例如 Codex route 选择 `anthropic_messages` target 时要使用 Anthropic API key 语义和 `anthropic-version`；Claude route 选择 OpenAI Chat/Responses target 时要使用 Bearer 语义。
- Claude/Codex 跨协议 provider 的单卡和批量连通性测试都必须走 Gateway 原生 CLI 入站协议，不能让单卡退回直连而与批量结果分叉。指定 provider 的测试应直接解析该 provider 并返回缺 key 等具体错误；诊断流只接受 2xx、非空且无 error event 的响应，并设置整体超时与有界响应预览，测试过程不得修改 provider health。
- Codex Gateway 协议和地址解析以当前 `model_provider` 指向的 `[model_providers.<id>]` 为首选，兼容根级 `wire_api` / `api_format` / `base_url`，最后才回退第一个 provider table。provider proxy 判定与 runtime provider 加载必须复用同一解析规则，不能维护两套不同的 TOML 语义。
- `GeminiNative` 已支持与 `AnthropicMessages`、`OpenAiChat`、`OpenAiResponses` 的聊天协议双向转换：Claude/Anthropic、OpenAI Chat 和 OpenAI Responses 请求可转 Gemini `generateContent`/`streamGenerateContent?alt=sse`，Gemini 入站请求也可转这些目标协议。具体 request/response/SSE/error 支持矩阵以 `proxy_gateway/transformer/AGENTS.md` 为准。
- 旧 manifest 缺少 `mode` 或 `primary_provider_id` 时必须反序列化失败并提示用户重新执行“网关代理”；不要给这两个字段加 serde default 静默兼容。
- Claude Thinking rectifier 与 Responses 密文恢复是两个独立、默认开启的反应式同渠道重试开关。`thinking_rectifier_enabled` 只处理 Claude 入站非流式请求的 thinking/signature 兼容类 HTTP 4xx：从重建的上游 body 移除顶层 `thinking`、顶层 `messages[].content[]` 中的 `thinking` / `redacted_thinking` 块和内容块直接携带的 `signature`。`responses_encrypted_content_rectifier_enabled` 只处理最终 target 为 OpenAI Responses 且明确报告 `encrypted_content` 无法验证/解密的 HTTP 4xx：结构化 `/error/code`（兼容顶层 `/code`）精确等于 `invalid_encrypted_content` 是首要且语言无关的恢复证据，`param=encrypted_content` 次之，旧英文 message 识别只作无结构化 code 的兼容兜底；`type=invalid_request_error` 过于宽泛，不能单独触发。恢复时仅在本次最终上游 body 副本中删除带非空 `encrypted_content` 的 `input[].type=reasoning` item。Responses 恢复以 `provider.target_protocol` 和转换后的最终 body 为事实源，覆盖 Responses 直通及其他协议转 Responses 后产生密文的路径；最终 target 非 Responses 时不能触发，`/responses/compact` 也必须排除。明确解密失败后，runtime side store 以“provider 运行时配置指纹 + `SHA-256(encrypted_content)`”记住具体失效密文；配置指纹必须覆盖 provider id、CLI、Base URL、target protocol、profile/endpoint 和鉴权身份等会影响实际上游的配置，配置变化后旧缓存不能继续命中。错误消息只负责定位具体密文，不负责决定结构化错误类别：完整 base64-like token 用等值匹配，`prefix...suffix`、`prefix…suffix` 或 `prefix……suffix` 用前后缀匹配，token 扫描不得依赖自然语言前后文；多个候选只遍历一次并在第二个命中时判歧义。完整或省略 token 都只在当前请求候选中唯一命中时缓存；请求仅有一条唯一密文且错误未提供可匹配标记时也允许缓存唯一候选。多密文但错误无法定位或省略标记命中多个候选时仍可删除整批完成本次恢复，却不能把整批写入跨请求负缓存。后续最终上游 body 只预删除同一配置身份下已知失效的 reasoning item，新密文和其他 provider 配置的同值仍按正常路径探测。该负缓存必须有容量上限、只驻留内存、不保存完整密文、不落数据库也不修改磁盘会话；不能按 session 或 provider 布尔值永久删除全部密文。两类恢复都只重试一次，不把 reasoning summary 改写成普通 message，并分别受自己的设置控制。Responses 重试还必须保留顶层 `include` / `store` / `reasoning`；Claude 清理不要递归扫描 metadata、tool input 或其他业务 payload，Responses 清理不要删除普通 message、function/custom tool call 或 output。
- `cache_injection_enabled` 和 `thinking_budget_rectifier_enabled` 必须按最终 `provider.target_protocol == AiProtocol::AnthropicMessages` 判断，而不是按入站 `route.cli_key == Claude` 判断。runtime 应先完成模型改写和必要的请求协议转换，再对最终 Anthropic body 注入 cache_control；budget 修正只针对目标 Anthropic、非流式、HTTP 4xx 的上游响应。
- CacheControl 注入使用 AxonHub 风格 4 断点规划，而不是简单“最后 system + 最后 user”：严格模式会先清空旧 `cache_control`，再按 structural anchors（最后一个 `tools[]`、最后一个 `system[]`）和 message anchors（短内容 1 个、长内容 2 个，第二个取距尾部约 20 block 的窗口边界）重建，最多 4 个 breakpoint。不要给 `thinking`、`redacted_thinking` 或空 text block 打 cache 点；string system/message content 可先规范化为 array 后再打点。
- `[1M]` / `[1m]` 只是客户端上下文能力标记，不是上游模型 ID 的一部分。Gateway 转发前必须从请求模型、provider 映射结果和 Gemini Native URL 的 `models/<model>` 段中剥离该后缀；仅剥离 1M 标记不算“模型重映射”，不能因此触发 Claude thinking rectifier 清理 thinking 块。
- Gemini Native URL 版本由 runtime 从 provider base URL 推断，支持 `v1` / `v1beta` / `v1alpha`。入站 route 可以匹配这些版本，但转发时要重写为 provider base URL 指定的版本；没有显式版本时才使用默认 Gemini 版本。这个规则只影响 runtime route/path 拼接，不改变 transformer 的 Gemini payload 语义。
- Provider 排序语义要与前端一致：`sort_index = None` 按 `0` 处理，而不是排到最后。
- Gateway 辅助说明文字统一使用 `fontSize: 10` 和 `color: var(--color-text-tertiary)`，不要在设置页临时放大或改成主文本颜色。

## 跨模块依赖

- 依赖 `coding::runtime_location` 解析 Claude Code、Codex、Grok CLI、Gemini CLI 的 runtime root；`RuntimeLocationMode::WslDirect` 还决定 CLI 接管时是否用 `ProxyGatewaySettings.wsl_host` 替换网关 origin 的 host。
- 前端 single 入口在已应用 provider 卡片上的“网关代理”按钮；常规恢复直连也在对应 provider 卡片上；provider 列表标题后的 `GatewayFailoverButton` 主要负责 single/failover 切换，但弹窗内必须保留 `status.can_restore_direct` 兜底恢复入口，避免 provider 被删除、解析失败或列表为空时无法解除接管。`GatewayPage` 顶部负责全局启动/停止、健康检查和刷新，设置面板只自动保存配置并展示网关地址/接管状态。
- 官方模型定价更新链路依赖 `tauri/resources/model_pricing.json`、app data 下的 `model_pricing.json` 缓存、以及远端 GitHub raw JSON。前端启动后台同步和定价弹窗手动同步都应调用同一后端命令，由后端校验 JSON、写缓存并 `INSERT OR IGNORE` 入 SQLite。
- 真实请求代理依赖 provider 表、模型健康、请求日志和 SQLite 使用摘要共同维护“按模型熔断、按供应商顺序路由”的契约：provider 列表从上到下就是网关优先级，后端只按 `sort_index` 和名称排序，不再把已应用 provider 提前；模型健康处于 cooling down 时跳过对应 provider/model。
- 运行时可以缓存 provider 候选列表，避免每个请求全量读 DB；缓存只能作为热路径优化，不能改变 provider 表的排序/禁用/模型映射语义。provider 增删改、排序和导入链路必须继续触发全局 `config-changed`，由监听器主动清空 Gateway provider 缓存；TTL 只是兜底，失效后必须重新从 SQLite 读取。
- Claude Code 进入故障转移模式时，运行时配置只写标准模型字段 `ANTHROPIC_MODEL`、`ANTHROPIC_DEFAULT_HAIKU_MODEL`、`ANTHROPIC_DEFAULT_SONNET_MODEL`、`ANTHROPIC_DEFAULT_OPUS_MODEL`、`ANTHROPIC_DEFAULT_FABLE_MODEL`，不写入 `ANTHROPIC_REASONING_MODEL`。同时写入 `ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME`、`ANTHROPIC_DEFAULT_SONNET_MODEL_NAME`、`ANTHROPIC_DEFAULT_OPUS_MODEL_NAME`、`ANTHROPIC_DEFAULT_FABLE_MODEL_NAME` 用于 Claude Code UI 显示真实 provider 模型名；退出故障转移回 single 时，从原始备份精确恢复这 9 个模型字段，原始文件不存在时删除这些 failover-only 字段。恢复直连还要兼容旧版本已写入的 `ANTHROPIC_REASONING_MODEL`：备份里有则恢复，备份里没有则删除。
- 上游失败后的重试策略是“同一 provider 最多重试 `per_provider_retry_count` 次，每次同渠道重试前等待 `retry_interval_secs` 秒，然后切下一个 provider（如果存在）”；跨 provider 故障转移不等待 retry interval。timeout 类失败例外：首包超时、请求超时或上游 timeout 不做同 provider retry，直接进入下一个 provider，避免同一慢 provider 阻塞整次请求。`max_retry_count` 是单个请求跨 provider 的额外重试总上限。请求日志里 `attempt_count` 表示最终 provider 内尝试次数，`total_attempt_count` 表示整个请求累计尝试次数。`retry_interval_secs=0` 表示保持立即重试。
- `ProxyGatewaySettings.retryable_status_codes` 控制哪些上游 HTTP 状态码可触发同渠道重试或跨渠道 failover。默认与历史行为一致：`400-404,408,429,500-599`。输入支持逗号分隔和闭区间，且只允许 `400-599`（1xx-3xx 不会进入失败重试路径，保存时直接拒绝）。空值在保存时回退默认并 normalize 成紧凑形式；默认集合本身就用折叠区间写法，避免保存后显示从逐码列表变成区间。保存设置时必须先 normalize/validate 并成功持久化，再把规范化结果写回运行态；不能先用未校验输入改 live runtime，再在 DB 失败时留下运行态与持久化分叉。该列表只影响“是否重试/切渠道”，不改变 `classify_status_failure` 的 failure kind 与 model health 计分。连接失败、超时和 2xx 空响应不是状态码失败，继续走既有 kind 规则，不受该列表约束。
- 上游 HTTP 400 在网关里按 `upstream_bad_request` 处理；默认在 `retryable_status_codes` 中，因此允许切换到下一个 provider。它的健康分较低，目的是处理 provider schema 差异，不要把它恢复成不可重试的 RequestSchema。用户可从配置中去掉 400 以关闭这类重试。
- 上游 HTTP 402 Payment Required 必须按 `Auth`（provider 级）处理；默认在 `retryable_status_codes` 中，因此允许跨 provider 故障转移，不能落入默认 `RequestSchema`。`RequestSchema` 对未列入可重试码表的状态码通常意味着不重试、不切换渠道；若把 402 误判成它，网关会把余额/额度耗尽错误原样回给 CLI，由 agent 自行重试同一渠道，最终导致对话中断。401/402/403 统一走 auth 故障转移语义。
- Session Usage 导入写入同一张 `proxy_request_logs`，`data_source='session'`。Claude 优先用 `SESSION:<message_id>` 做 request_id 幂等去重；其他 CLI 用文件/行内容派生的稳定 ID，并通过 `INSERT OR IGNORE` 保持可重复导入。
- 代理请求摘要和 Session Usage 导入成功写入 `proxy_request_logs` 后应发出 `usage-log-recorded` 事件，供前端静默刷新统计和请求列表。该事件只是“有新 usage 落库”的通知，不是统计数据源，也不要用它承载费用重算或历史 rollup 语义。
- 模型定价匹配需要先做 ID 归一化再查表：剥离聚合商命名空间、`[1M]` 上下文标记、Bedrock/Vertex `-vN` 版本、日期/effort 后缀，并把 Claude 点号版本归一成短横线版本。部分渠道会把 `max` effort 拼进模型 ID，因此允许在完整 ID 没有精确定价时剥离 `-max` 回退基础模型；但所有候选必须先按原始完整 ID 做精确查询，使 `qwen3.7-max`、`qwen3-max`、`gpt-5.1-codex-max` 等具有独立价格的正式模型自动优先命中，不能维护易过期的手工排除名单。前缀匹配只能用于明确的模型族和足够具体的 ID，避免 `gpt-5` 这类短 base 误命中 `gpt-5-mini`/`gpt-5-pro` 变体。
- 每个 CLI 可以通过 `ProxyGatewaySettings.app_configs` 覆盖首包超时、流式 idle timeout、非流式 timeout、单 provider 重试、全局重试和重试间隔；运行时必须用 `effective_app_config(cli_key)` 读取，不能只看全局字段。
- `runtime.rs` 只承载生命周期、async listener accept 和主流程编排。HTTP 读写放 `runtime/http_io.rs`，路由匹配和 URL 拼接放 `runtime/routes.rs`，provider 读取/解析放 `runtime/providers.rs`，上游转发和 failover 放 `runtime/upstream.rs`，请求日志/metrics 采集放 `runtime/observability.rs`，跨请求协议兼容缓存放 `runtime/side_stores/`，pipeline/middleware 扩展点放 `runtime/pipeline.rs` / `runtime/middleware.rs`。后续新增能力优先放入对应职责文件，不要重新堆回 `runtime.rs`。
- 统计页数据源拆分 (`DataSourceBreakdown`) 来自 `proxy_request_logs.data_source`，空值归并为 `proxy`，Session Usage 导入当前统一写 `session`；它只反映已落库的请求摘要分布，不要当成网关健康指标。

## 最小验证

- 修改 CLI 接管/恢复逻辑后至少跑 `cd tauri && cargo test`，并覆盖三类 CLI 文件写入、恢复、重新接管不覆盖原始备份、停止保护。
- 修改请求转发、请求日志、SQLite 使用摘要或模型健康后至少跑 `cd tauri && cargo test`，并覆盖本地文件 round trip、fallback 路由和失败健康状态更新。
- 修改前端接管入口或设置页状态后至少跑 `pnpm exec tsc --noEmit`、`pnpm test`；触及共享 UI、i18n 或构建入口时补跑 `pnpm build`。

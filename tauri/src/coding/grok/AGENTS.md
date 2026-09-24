# Grok 后端模块说明

## 一句话职责

- `grok/` 负责 Grok CLI 的 provider/common config、`config.toml`、`auth.json`、官方账号、prompt 和原生插件管理。

## Source of Truth

- Provider、common config、prompt 和 official account 长期主数据在 SQLite JSONB。
- 当前运行时根目录由 `runtime_location` 按应用内 `root_dir`、`GROK_HOME`、shell 配置、`~/.grok` 的顺序解析。
- MCP 主数据属于中央 MCP 模块；Plugins 和 Sessions 的事实源分别是 Grok CLI/runtime 与 `<root>/sessions/`。

## 核心设计决策

- 前端可复制 Codex，Grok TOML 和 OAuth 落盘逻辑不能复制 Codex schema。
- Provider 只拥有 `[models].default` 和自身受管 `[model.*]`；Common、MCP、Plugins、Skills 和未知配置必须字段级保留。
- `auth.json` writer 必须基于真实 Grok CLI fixture，只更新已确认 OAuth 字段，原子写入并保留未知字段。
- 官方 `auth.json` 是 `{ "<issuer>::<client_id>": { ...credential entry... } }` 的 scope map；`key` 是 access token。不得退回根级 `access_token/id_token/type/auth_kind` 扁平结构。
- 自定义 Provider 的 API Key 不得清除官方 OAuth；Grok 的模型级凭据优先级允许两者共存。
- Provider/Common 受管非模型字段使用 Codex 同款激进移除：只要字段曾受管，下次 apply 就移除（即使 live 值已与上次受管值不同）。
- Provider 受管 `[model.<key>]` **就是渠道配置**。切换/保存/应用时始终删除上一渠道 catalog 里的 key，再写入新渠道投影；不得按“用户手改”保留 `base_url`/`api_key`/`api_backend`，也不得发 `grok-config-warning` 假装保留。
- 真正本地、从未出现在上一渠道 catalog 的 `[model.*]` 才保留。官方渠道只拥有 `[models].default`，应用官方时清理上一 custom catalog keys 后不得再写任何 `[model.*]`。
- `settings_config` 顶层 `baseUrl` 是**前端渠道级 Base URL SoT**（卡片/编辑表单在模型列表被清空后仍要显示它，issue #391）。后端不读也不投影它：live 只认 `[model.<key>].base_url`，`validate_provider_settings` 也不校验；不要把它写进 TOML、common 或 provider 高级 `config` 字段。
- `apply_grok_provider_to_file` 默认应带上当前 common config 作为 previous，避免 common 字段在只切 Provider 时残留。
- 更新已应用 Provider 时，必须在覆盖 SQLite 记录前捕获旧 `settings_config` 和 `category`，并显式传给运行时重应用链路。写库后再查询 applied provider 得到的是新快照，会导致被删除的 `[model.<key>]` 和高级配置字段残留。
- `settings_config` 不存 `category`（category 在 provider 行）。清理前一 provider 的 `[model.*]` 时必须传入真实 `previous.category`；官方渠道只拥有 `[models].default`，清理时不得当 custom 去要求 `modelCatalog.models`。
- 应用 provider 时对齐 Codex 官方账号标记：官方 provider → `sync_grok_official_account_apply_status`；非官方 → `clear_all_grok_official_account_apply_status`，避免切到自定义后账号仍显示「已应用」。
- Device Code / OAuth 登录成功只入库官方账号（`is_applied=false`），不写 `auth.json`、不 `db_update_applied_status`、不 `emit_grok_sync`。真正应用只走 `apply_grok_official_account`（写 auth + 标记 applied）。对齐 Codex，避免当前已应用自定义渠道时新登录账号抢占「已应用」态并污染 runtime。
- 官方账号额度走 Grok CLI chat-proxy，不是 Codex `wham/usage`，也不是 Web SSO rate-limits：
  - `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits` → 周池 `creditUsagePercent` + `currentPeriod`
  - `GET https://cli-chat-proxy.grok.com/v1/billing` → 月账 `monthlyLimit`/`used` + 日历 `billingPeriod*`
  - `GET https://cli-chat-proxy.grok.com/v1/user?include=subscription` → 订阅等级
  - 必带 CLI 头：`X-XAI-Token-Auth: xai-grok-cli`、`x-grok-client-version`、`x-grok-client-identifier: grok-shell`、`x-grok-client-mode: headless`
  - `api.x.ai/v1/billing|user` 对 OAuth 账号是 404，不要回退到官方 API base
- OAuth access token 寿命约数小时。续期策略：
  - Lead：剩余 ≤ 30 分钟（或缺少 expires）视为临期
  - `apply_grok_official_account` / `refresh_grok_official_account_limits` 在写 auth 或拉 billing 前调用 `ensure_fresh`（临期才真正 refresh）
  - `refresh_grok_official_account` 强制 refresh（不看 lead）
  - **后台 / 启动巡检** 由共享模块 `coding::auth_refresh` 调度（startup + 15m interval），本模块只提供 `refresh_applied_grok_accounts_if_needed`
  - 巡检候选是**所有已入库官方 OAuth 账号**（含 `is_applied=false`），不是只刷已应用；登录后未点应用也要续期 SQLite 快照，避免关机重启后 token 静默过期
  - 并发：`OAUTH_REFRESH_LOCK` + 同 refresh_token 30s 缓存；续期成功后始终写 SQLite；**仅 applied** 时 merge live `auth.json` 并 `emit_grok_sync`
  - `config.toml` 的读-改-写（`apply_grok_provider_to_file_with_previous_settings`）与 `save_grok_common_config` 的无 provider 全量覆盖必须在模块级 `CONFIG_WRITE_LOCK`（`tokio::sync::Mutex`）内进行：每次 apply 都基于自己的读取快照重建整个文档，并发 apply 会从同一旧快照回写、后写者丢失先写者的投影。锁不可重入；新增写入口时复用这两个函数，不要在锁外自建读改写
- `refresh_grok_official_account_limits` 只写额度字段到 SQLite，不改 `is_applied`；若内部 ensure_fresh 续了 token 且账号已应用，才会写 auth。
- 额度字段语义：`plan_type`（上游 null → `free`）、`limit_weekly_text`（`100 - creditUsagePercent`，无 percent 则空）、周 reset 来自 credits `currentPeriod.end`。月限只在默认 billing 的 `monthlyLimit>0` 时投影；**禁止**把 credits 的 `billingPeriodEnd` 当月重置（free/unified 会把它写成与周周期相同）。不要伪造 5h 窗口。
- 删除 prompt 配置只删 SQLite 记录，不删除/清空当前 `AGENTS.md`。产品语义是“删除已保存的提示词记录”，不是“删除本地 runtime 提示词文件”；Claude Code / OpenCode / Codex / Gemini / Pi 统一此规则。
- 清空 `auth.json` 时，必须先 `remove_auto_synced_wsl_mapping_target`，不能只 `emit_grok_sync`（源缺失时普通同步会跳过而非删除远端）。

## Gotchas

- `extract_grok_common_config_from_current_file` 只能读当前根目录 `config.toml`，不要碰 `auth.json`。WSL UNC / 网络路径上同步文件 I/O 可能长时间阻塞；extract 必须走 `coding::file_io` 的 `spawn_blocking` + 超时读，超时错误要带实际路径。
- 不要整段删除 `[models]` 或全部 `[model.*]`。
- 模型 schema 必须保留 `env_key`、显式 `false`、sampling、retry、timeout、reasoning、`extra_headers` 和未知合法字段。
- 官方渠道思考等级写 `[models].default_reasoning_effort`（settings 字段 `defaultReasoningEffort`）。自定义 per-model：`reasoningEfforts` ↔ `reasoning_efforts`，`reasoningEffort` ↔ `reasoning_effort`，`supportsReasoningEffort` ↔ `supports_reasoning_effort`。`project_provider_models` 在 official 投影/清理 global effort，在 custom 清理 residual `default_reasoning_effort`。Common / local common 提取时必须移除 `default` 与 `default_reasoning_effort`，避免 provider 字段落入 common。
- Grok MCP 使用 `headers`，不是 Codex 的 `http_headers`；不写 `type`，Windows/WSL/SSH 都不添加 `cmd /c`。
- Device Code 和 OAuth token 只留在后端；事件和前端 payload 不得包含 OAuth 凭据。
- “预览当前配置”必须返回 live `config.toml` / `auth.json` 的真实内容，不做任何脱敏（包括 `api_key`、token、Authorization）。这是用户主动查看本地生效态的诊断入口。
- xAI Device Code scope 包含 `conversations:read conversations:write`；身份字段来自 access-token claims 与 OIDC userinfo。refresh 必须保留同 principal 的 CLI enrichment，apply/delete/logout 必须保留其他 auth scope，最后一个 scope 删除后才删除文件。
- 不要把 Device Code poll 成功路径重新改回「登录即 write_auth_json + is_applied=true」。那会在自定义渠道已应用时误标官方账号并改写 live auth。
- Free 账号的 billing credits 常有周周期但没有 `creditUsagePercent`；UI 周限额应显示 `-`，不要把 0 用量或缺失 percent 当成解析失败。credits 里的 `billingPeriodEnd` 可能等于周结束日，不能填进月限重置。
- 不要把 Grok lead 改成 Codex 的 3 天：Grok access token 只有数小时。不要给 list 账号路径加自动 refresh。
- 从 live `config.toml` 生成 `__local__` 时：模型级 `api_key` 不得进入 `modelCatalog` / `extraConfig`。若所有 `[model.*]` 的非空 `api_key` 完全一致，提升到 `settings.auth.API_KEY`，让 Local 编辑/收编可 round-trip；多模型 key 不一致或只有部分模型有 key 时保持 `auth` 为空，不强行猜测。
- Official xAI marketplace identifiers: manifest name `xai-official`, CLI list name `plugin-marketplace`, source `xai-org/plugin-marketplace` (`.grok-plugin/marketplace.json`). `is_curated` / hide-recommend must accept these aliases and the source URL. Claude-compatible marketplaces such as `claude-plugins-official` may still appear in CLI list or cache; keep them installable, do not treat as curated, and do not auto-delete. Resolve install sources from `.grok-plugin` first, then `.claude-plugin`.
- Official marketplace install sources use either `{source:"url",url,sha}` or `{type:"local",path}`. `marketplace_install_source` must pin `sha`/`ref` and resolve local path objects relative to the marketplace cache root.

## 最小验证

- Provider 执行 `read -> edit -> save -> apply -> read` 后 fixture 只出现预期差异。
- Common/Provider 写入后 MCP、Plugins、Skills、用户模型和未知字段仍存在。
- `auth.json` 写回后官方 Grok CLI 可识别，Unix 权限为 `0600`。
- 自定义渠道已应用时完成 Device Code 登录：新账号 `is_applied=false`，live `auth.json` 不变；点应用后才写 auth 并标记 applied。
- 点「刷新额度」：账号行出现 `plan_type`/`last_limits_fetched_at`；有 `creditUsagePercent` 时周剩余为 `100-used%`；无 percent 时周限额为空但可有周重置时间。free/`monthlyLimit=0` 时月限额与月重置均为空。
- 已入库官方 OAuth 账号（含未应用）access token 临期时：apply / 刷新额度 / 后台巡检会自动续期，明细里 Token 过期时间与最近刷新时间更新；未临期不发起 refresh 请求。未应用只更新 SQLite，不写 live `auth.json`。
- xAI refresh token 会轮换/吊销。token endpoint 返回 `invalid_grant`（常见 HTTP 400）时，自动续期无法恢复，必须重新 Device Code 登录；错误信息应带上 `error`/`error_description`，并写入账号 `last_error`。
- 前端区分「刷新额度」与「刷新 Token」：额度走 `refresh_grok_official_account_limits`（临期才 refresh），Token 走 `refresh_grok_official_account`（force）。`invalid_grant`/HTTP 400 应提示重新 OAuth 登录，并在账号列表/明细展示 `last_error`；成功后续期应清空 `last_error`。

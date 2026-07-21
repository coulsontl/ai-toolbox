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
- `apply_grok_provider_to_file` 默认应带上当前 common config 作为 previous，避免 common 字段在只切 Provider 时残留。
- 更新已应用 Provider 时，必须在覆盖 SQLite 记录前捕获旧 `settings_config` 和 `category`，并显式传给运行时重应用链路。写库后再查询 applied provider 得到的是新快照，会导致被删除的 `[model.<key>]` 和高级配置字段残留。
- `settings_config` 不存 `category`（category 在 provider 行）。清理前一 provider 的 `[model.*]` 时必须传入真实 `previous.category`；官方渠道只拥有 `[models].default`，清理时不得当 custom 去要求 `modelCatalog.models`。
- 应用 provider 时对齐 Codex 官方账号标记：官方 provider → `sync_grok_official_account_apply_status`；非官方 → `clear_all_grok_official_account_apply_status`，避免切到自定义后账号仍显示「已应用」。
- 删除 prompt 配置只删 SQLite 记录，不删除/清空当前 `AGENTS.md`。产品语义是“删除已保存的提示词记录”，不是“删除本地 runtime 提示词文件”；Claude Code / OpenCode / Codex / Gemini / Pi 统一此规则。
- 清空 `auth.json` 时，必须先 `remove_auto_synced_wsl_mapping_target`，不能只 `emit_grok_sync`（源缺失时普通同步会跳过而非删除远端）。

## Gotchas

- `extract_grok_common_config_from_current_file` 只能读当前根目录 `config.toml`，不要碰 `auth.json`。WSL UNC / 网络路径上同步文件 I/O 可能长时间阻塞；extract 必须走 `coding::file_io` 的 `spawn_blocking` + 超时读，超时错误要带实际路径。
- 不要整段删除 `[models]` 或全部 `[model.*]`。
- 模型 schema 必须保留 `env_key`、显式 `false`、sampling、retry、timeout、reasoning、`extra_headers` 和未知合法字段。
- 官方渠道思考等级写 `[models].default_reasoning_effort`（settings 字段 `defaultReasoningEffort`）；自定义写 `[model.<key>].reasoning_effort` + `supports_reasoning_effort`（catalog `reasoningEffort` / `supportsReasoningEffort`）。`project_provider_models` 在 official 投影/清理 global effort，在 custom 清理 residual `default_reasoning_effort`。Common / local common 提取时必须移除 `default` 与 `default_reasoning_effort`，避免 provider 字段落入 common。
- Grok MCP 使用 `headers`，不是 Codex 的 `http_headers`；不写 `type`，Windows/WSL/SSH 都不添加 `cmd /c`。
- Device Code 和 OAuth token 只留在后端；事件和前端 payload 不得包含 OAuth 凭据。
- “预览当前配置”必须返回 live `config.toml` / `auth.json` 的真实内容，不做任何脱敏（包括 `api_key`、token、Authorization）。这是用户主动查看本地生效态的诊断入口。
- xAI Device Code scope 包含 `conversations:read conversations:write`；身份字段来自 access-token claims 与 OIDC userinfo。refresh 必须保留同 principal 的 CLI enrichment，apply/delete/logout 必须保留其他 auth scope，最后一个 scope 删除后才删除文件。
- 从 live `config.toml` 生成 `__local__` 时：模型级 `api_key` 不得进入 `modelCatalog` / `extraConfig`。若所有 `[model.*]` 的非空 `api_key` 完全一致，提升到 `settings.auth.API_KEY`，让 Local 编辑/收编可 round-trip；多模型 key 不一致或只有部分模型有 key 时保持 `auth` 为空，不强行猜测。
- Official xAI marketplace identifiers: manifest name `xai-official`, CLI list name `plugin-marketplace`, source `xai-org/plugin-marketplace` (`.grok-plugin/marketplace.json`). `is_curated` / hide-recommend must accept these aliases and the source URL. Claude-compatible marketplaces such as `claude-plugins-official` may still appear in CLI list or cache; keep them installable, do not treat as curated, and do not auto-delete. Resolve install sources from `.grok-plugin` first, then `.claude-plugin`.
- Official marketplace install sources use either `{source:"url",url,sha}` or `{type:"local",path}`. `marketplace_install_source` must pin `sha`/`ref` and resolve local path objects relative to the marketplace cache root.

## 最小验证

- Provider 执行 `read -> edit -> save -> apply -> read` 后 fixture 只出现预期差异。
- Common/Provider 写入后 MCP、Plugins、Skills、用户模型和未知字段仍存在。
- `auth.json` 写回后官方 Grok CLI 可识别，Unix 权限为 `0600`。

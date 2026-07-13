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
- 前一 Provider 的受管 `[model.<key>]` 只有在 live table 仍与上次投影完全一致时才可删除。用户手改后的 table 必须跨 Provider 应用保留，即使新 Provider 使用同名 key，也要恢复用户 table 并向前端发送 warning。
- `apply_grok_provider_to_file` 默认应带上当前 common config 作为 previous，避免 common 字段在只切 Provider 时残留。
- 删除已应用 prompt 或清空 `auth.json` 时，必须先 `remove_auto_synced_wsl_mapping_target`，不能只 `emit_grok_sync`（源缺失时普通同步会跳过而非删除远端）。

## Gotchas

- 不要整段删除 `[models]` 或全部 `[model.*]`。
- 模型 schema 必须保留 `env_key`、显式 `false`、sampling、retry、timeout、reasoning、`extra_headers` 和未知合法字段。
- Grok MCP 使用 `headers`，不是 Codex 的 `http_headers`；不写 `type`，Windows/WSL/SSH 都不添加 `cmd /c`。
- Device Code 和 token 只留在后端；事件和前端 payload 不得包含凭据。
- xAI Device Code scope 包含 `conversations:read conversations:write`；身份字段来自 access-token claims 与 OIDC userinfo。refresh 必须保留同 principal 的 CLI enrichment，apply/delete/logout 必须保留其他 auth scope，最后一个 scope 删除后才删除文件。
- 从 live `config.toml` 生成 `__local__` 时必须跳过模型级 `api_key`，不能把它放进 `auth`、`modelCatalog` 或 `extraConfig` 后返回前端。

## 最小验证

- Provider 执行 `read -> edit -> save -> apply -> read` 后 fixture 只出现预期差异。
- Common/Provider 写入后 MCP、Plugins、Skills、用户模型和未知字段仍存在。
- `auth.json` 写回后官方 Grok CLI 可识别，Unix 权限为 `0600`。

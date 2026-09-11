# Kimi 后端模块说明

## 一句话职责

- `kimi/` 负责 Kimi Code CLI 的 provider/common config、`config.toml`、`credentials/` 官方账号、prompt 和原生插件/MCP/Skills 同步。

## Source of Truth

- 跨工具分享生成的直接聊天连接使用 Kimi 原生 `openai_legacy` provider type；`openai_responses`、`anthropic`、`gemini` 是其他协议的原生枚举，不能把通用展示词 `openai` 当作新增连接的 CLI type。分享模型的图片/视频能力分别写 `image_in` / `video_in`，协议与认证适配见 `coding/deeplink/AGENTS.md`。

- Provider、common config、prompt 和 official account 长期主数据在 SQLite JSONB。
- 当前运行时根目录由 `runtime_location` 按应用内 `root_dir`、`KIMI_CODE_HOME`、shell 配置、`~/.kimi-code` 的顺序解析。
- 备份/恢复链路归属于 DB 型 CLI（`OPTIONAL_BACKUP_CLI_TOOLS`），受 `backup_cli_config_files_enabled` 门控；备份时打包 `config.toml`、`AGENTS.md`、`credentials/` 官方凭据和 `plugins/`，恢复时由 `settings::backup` 恢复或由 `reapply_applied_runtime` 从 SQLite 重新应用。
- MCP 主数据属于中央 MCP 模块；Plugins 和 Sessions 的事实源分别是 Kimi CLI 与 `<root>/sessions/`。

## 核心设计决策

- `config.toml` 拥有 `[models].default_model` 和自身受管 `[models.<key>]`，以及 `[providers.<key>]`；Common、MCP、Plugins、Skills 和未知配置必须字段级保留。
- **`__local__` 投影的 official/custom 判定以 credentials 目录为准**：`determine_local_kimi_provider_category(has_credentials, has_custom_providers)` 中凭据信号最强——`<root>/credentials/*.json` 存在即判 `official`，即使 config.toml 同时有 `[providers]` 表（官方 apply 流程会把 channel 凭据投影进去）；无凭据时 `[providers]` 为空回退 `official`，非空则 `custom`。只用 `[providers]` 表判定的旧逻辑曾把官方登录用户导入成自定义配置。
- Gateway CLI 接管通过 `proxy_gateway` 的 `GatewayCliKey::Kimi` 驱动，接管文件只有 `<root>/config.toml`；Gateway 改写的是 `default_model → [models.<key>].provider` 解析出的**当前生效 provider 表**（自定义 provider 如 `axonhub` 时是 `[providers."axonhub"]`，官方账号回退 `[providers."managed:kimi-code"]`）的 `type/base_url/api_key`，不改写 models 表或其他 provider 表。写死 `managed:kimi-code` 的旧实现会让自定义 provider 流量完全绕过网关（统计恒 0）。
- Kimi 直连 apply/save common config 在 Gateway 接管期间必须被 `ensure_kimi_gateway_direct` 拒绝；切换已接管 provider 统一走 Gateway-aware switch 编排。门禁覆盖**所有会重写 `<root>/config.toml` 的入口**：`select_kimi_provider`、`select_kimi_model`（托盘模型切换也会重投影 live 文件，漏门禁会把接管中的直连值写回、流量绕过网关）、`save_kimi_common_config`、`update_kimi_provider`（该 provider 已应用、保存会重投影时）、`save_kimi_local_config`（`__local__` 收编也会重写 live 文件）、`apply_kimi_official_account`。共享判定抽在 `commands::ensure_gateway_direct_for_paths(&ProxyGatewayPaths)`，单测可直接喂 tempdir + Kimi manifest。接管期间托盘模型子菜单同步置灰（`get_kimi_model_tray_data` 读 `kimi_gateway_takeover_active`）。
- 所有对 `<root>/config.toml` 的读-改-写必须在模块级 `CONFIG_WRITE_LOCK`（`tokio::sync::Mutex`）内进行：每次写都基于自己的读取快照重建整个文档，并发 apply 会从同一旧快照回写、后写者丢失先写者的投影。写入口（save/update/select provider/model、收编 `__local__`、common config）必须**自己持锁覆盖「捕获旧快照 → 写 DB → 重投影」整个窗口**，再调用无锁本体 `apply_kimi_provider_to_file_locked` / `write_common_config_without_provider_locked`；锁外捕获旧快照会让并发保存按同一旧快照清理、残留先写者的受管字段。锁不可重入，无锁本体不得自行加锁；新增写入口时沿用该模式，不要在锁外自建读改写。
- 官方凭据存放在 `<root>/credentials/` 目录下（如 `token.json` 等），只更新已确认 OAuth 字段，原子写入并保留未知字段。
- 自定义 Provider 的 API Key 不得清除官方 OAuth 凭据。
- 清理前一 provider 的 `[models.<key>]` 和 `[providers.<key>]` 时，在覆盖 SQLite 记录前捕获旧快照，并显式传给运行时重应用链路。
- 应用 provider 时对齐 Codex/Grok 官方账号标记：官方 provider → `sync_kimi_official_account_apply_status`；非官方 → `clear_all_kimi_official_account_apply_status`。
- Device Code / OAuth 登录成功只入库官方账号（`is_applied=false`），不立刻写 credentials、不 `db_update_applied_status`。真正应用只走 `apply_kimi_official_account`；同一 provider 重新登录复用已有账号行，保留旧 `sort_index`/`created_at`，不追加重复行。
- 后台 / 启动巡检由共享模块 `coding::auth_refresh` 调度。巡检刷新成功后**不需要**重投影 config.toml：刷新后的 OAuth access token 只写 `credentials/<name>.json`（仅 applied 账号），config.toml 里 `[providers].api_key` 来自 provider 行的静态 `auth.API_KEY`，与 OAuth token 无关。
- 已应用状态是删除边界：`delete_kimi_provider` 拒绝删除 `is_applied` 的 provider；`delete_kimi_official_account` 拒绝删除已应用账号。否则 config.toml / credentials 的投影失去 applied 快照、永久残留，与 `toggle_kimi_provider_disabled` 的「已应用不可禁用」同一语义。`apply_kimi_official_account` 额外拒绝 disabled provider，与 `select_kimi_provider` 对齐。
- 无已应用 provider 时 `save_kimi_common_config` 也**不许整文件覆盖** live config.toml：走 `write_common_config_without_provider`，与 provider 投影同款 merge 语义（先 `remove_matching_unmanaged_config` 清上次受管字段，再 `merge_common_config` 写新受管字段），用户手写在 config.toml 里的 `[providers]` / `[models]` 必须保留。

## Gotchas

- `extract_kimi_common_config_from_current_file` 只能读当前根目录 `config.toml`。WSL UNC / 网络路径必须走 `coding::file_io` 的 `spawn_blocking` + 超时读；`read_optional_text` 已统一走 `read_optional_text_file_with_timeout`（预览、投影、本地快照、prompt 读取都经过它），不要在新代码里裸 `fs::read_to_string` runtime 文件。
- 会话扫描：首屏 recent quick path 复用共享 `collect_recent_files_by_modified` 早停扫描，同一 session 目录的 `state.json` / `summary.json` 按 source_path 去重，一个目录只出一条。resume 命令 `kimi -S <sessionId>` 依据 `docs/plan-kimi-code-cli.md` §8.1（备选 `kimi -c`）。native snapshot 导出递归不限深，UTF-8 存文本、非 UTF-8 用显式 `{"encoding":"base64","data":...}` payload，import 两种编码都要认；`delete_session` 必须校验 session_path 在 sessions_root 之内。
- Gateway 接管 origin 使用 `/kimi/v1` 前缀，而真实模型请求入口是 `/kimi/v1/chat/completions`；不要把 probe 路径和真实 OpenAI Chat 入站协议混为一谈。
- Kimi CLI 对每个投影的 `[models.<key>]` 硬校验 `max_context_size` 必须为正数，缺失时拒绝启动会话（`Failed to start a session: Model "x" must define a positive max_context_size`）。投影时缺失/非正数一律兖底 262144（256k，对齐官方 kimi-for-coding 保守值）；官方 k3 参考值是 1048576。前端表单模型目录有对应列（列名直接用字段名 max_context_size），新建行默认 262144。
- Kimi CLI 已弃用 `loop_control.max_retries_per_step`；字段级保留会让每次 CLI 运行都打印弃用警告。apply 投影写入前由 `migrate_deprecated_loop_control_fields` 自动重命名为 `max_attempts_per_step`（新键已存在时丢弃旧键，其余字段不动）。
- 模型目录表单只展示 key / model / max_context_size 三列；`displayName` 不再提供编辑入口（产品决策：key 兼任显示名），但数据层 parse/normalize 仍透传已有记录的 displayName 以兼容旧数据。
- 不要整段删除 `[models]` 或全部 `[providers]`。
- 删除 prompt 配置只删 SQLite 记录，不删除/清空当前 `AGENTS.md`。

## 最小验证

- Provider 执行 `read -> edit -> save -> apply -> read` 后 fixture 只出现预期差异。
- Common/Provider 写入后 MCP、Plugins、Skills、用户模型和未知字段仍存在。
- 官方账号与自定义 provider 切换状态与托盘联动正常。
- Prompt 保存仅操作 `AGENTS.md` / SQLite，与 Gateway `config.toml` 接管边界互不影响。
- `__local__` 投影 category 判定：`cargo test coding::kimi` 覆盖 credentials×providers 四象限。
- Gateway 接管 round trip 后仅当前生效 provider 表（`default_model` → `[models.<key>].provider` 链解析的 key，回退 `managed:kimi-code`）的 `type/base_url/api_key` 指向本地网关；恢复直连按 manifest 受管字段做字段级还原（空表删除），接管窗口内的其他改动保留。
- 托盘入口的 provider / model / prompt 切换统一发 `config-changed` 的 `tray` payload（前端收到 tray 才 reload 页面），窗口入口发 `window`。
- `cargo test kimi` 需覆盖：已应用 provider / 已应用官方账号删除被拒；无 provider 时 common config 合并保留用户手写 `[providers]`；Gateway 门禁 manifest 判定；会话扫描/解析/snapshot 往返 fixture（`tauri/tests/coding/kimi/sessions.rs`）。

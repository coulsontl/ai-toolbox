# Control API 后端模块说明

## 一句话职责

- `tauri/src/control_api/` 是仅绑定 `127.0.0.1` 的本地 Agent 控制面：HTTP 路由、Bearer 鉴权、L0/L1/L2 风险分级、桌面授权等待、按 CLI 分发到既有 Tauri 命令、凭据脱敏，以及 `aitoolbox` 安装/状态命令。
- 它不重新实现任何 CLI 配置写入逻辑；所有 provider/model/config/prompt/gateway/plugin 操作都通过 `app.state::<T>()` 调用既有 `#[tauri::command]` 实现。

## Source of Truth

- 控制面设置独立存于 SQLite `settings` 表的 `id = control_api` 行，**不要**并入 `settings/app`。`save_settings` 是全量替换，旧前端快照会把授权策略静默重置回默认值。
- 运行期状态（监听端口、Bearer Token、待授权请求）只在内存 `ControlApiState`；Token 同时写入发现文件 `~/.ai-toolbox/control.json`（Unix 0600），文件是 CLI 的唯一凭据来源。
- Token、明文 API Key、OAuth token、`plugin-auth.json` 永远不是 HTTP 响应、弹窗摘要或 Tauri 命令返回值的一部分。前端只看状态、设置和脱敏后的审批摘要。
- 各 CLI 的 provider/config/prompt 事实源仍是原模块：DB 接管型在 SQLite `cli_provider` / `cli_config`，文件型在各 CLI 的规范配置文件，运行时型在 `read_*_runtime_config` 返回的路径。控制面只是调用者。

## 核心设计决策（Why）

- **直接复用 Tauri 命令而不是抽服务层**：现有命令的参数就是 `tauri::State<'_, T>`，通过 `AppHandle::state::<T>()` 可以原样构造；这样 provider 归一化、self-loop 校验、备份、事件广播、WSL 同步、应用配置写入全部继承，避免复制一套易漂移的解析器。新增 CLI 能力时，先找既有命令，不要在 `cli_dispatch.rs` 重新写配置格式。
- **独立 Settings 行 + 默认 `always_ask`**：控制面默认开启但所有写操作都要桌面确认；模式包括 `always_ask` / `ask_high_risk` / `auto_approve` / `read_only`。`read_only` 直接拒绝写请求，不是靠前端隐藏按钮。
- **授权用 oneshot 而不是轮询**：`ApprovalHub` 注册 `requestId` + `oneshot::Sender<bool>`，HTTP handler `await` 同一个 channel；前端 `ControlApiApprovalModal` 调 `control_api_resolve_approval` 解阻塞。超时 408、拒绝 403，CLI 都映射为退出码 4。
- **本地插件 reload 的快速通道只限 L1**：`always_ask` + 用户开启 `auto_approve_local_plugin_reload` 时，只有本地目录来源、且风险为 L1 的 `plugin reload` 跳过弹窗；L2 永远需要确认。策略纯函数 `approval::decide` 必须保持可单测。
- **风险分级集中在路由层**：L0 只读；L1 可逆写（provider/model/config/prompt/reload/enable/restart/health reset）；L2 删除 provider、远程安装插件、卸载插件、gateway start/stop/takeover。不要在 `cli_dispatch` 里偷偷执行未过 `approve_write` 的写操作。
- **发现文件是同机唯一凭据载体**：CLI 读取 URL + Token，绝不打印；桌面端只显示路径。服务停止/退出时必须删除自己 pid 的发现文件，避免下一次启动误读旧 Token。
- **HTTP 客户端独立**：`cli/` 不依赖 Tauri/WebView，使用 clap + 手写阻塞 HTTP/1.1；不要为了复用后端类型把 CLI 拖进 Tauri 依赖树。

## 复用白名单与反腐蚀（Hard Rule 15）

- **允许直接调用的层**：`coding/<cli>/commands`、`coding/config_patch`（TOML/YAML 整文件合并）、`coding/proxy_gateway::commands`、`coding/proxy_gateway::provider_switch`、`plugins::commands`/`installer`/`store`/`cli_integration`、`workbuddy_family::channel`。除此之外不要在本目录新增配置解析器、模型目录合并器或凭据读写。
- **6 个网关接管型 CLI 的写入必须走 `provider_switch`**：`claudecode` / `claudedesktop` / `codex` / `grok` / `kimi` / `geminicli` 的 provider 创建、更新、应用、模型增删/默认模型、全局配置写入，都必须在 `apply_or_switch_provider`（应用）或 `with_gateway_reattach`（写源）内完成。控制面不得自己调用 restore/engage 命令拼流程；否则单测能过、真实 takeover 会被降级成 direct 或丢掉 aggregate 清单。
- **前端编排已下沉**：桌面端的 restore → save → re-engage 由 `proxy_gateway_begin_provider_write` / `proxy_gateway_finish_provider_write`（同一把 `DATA_DIR_CHANGE_LOCK` + `provider_switch_lock`）承担；TS 的 `providerSaveWithGatewayReengage` 在拿到 `gatewayCliKey` 时必须调这两个命令，不再自己串回调。新增页面直接传 `gatewayCliKey`。
- **脱敏只发生在响应边界**：`redact_value`/`redact_text`/`redact_json_string` 只用于 GET/LIST/日志/审批摘要/错误体。业务函数、canonical 捷径、provider test/fetch-models、PATCH 合并都必须从原始 DB 行或原始配置文件重新读取；公开 DTO 里的 `***` 永远不能进入写入或联网路径。
- **能力拒绝用稳定 code**：不支持的能力返回 `unsupported`（400），策略拒绝返回 `read_only`（403），两者都不能混用。CLI 的 `--json` 会原样透出 server code。
- **新命令/新路由必须同步内置 Skill**：`tauri/resources/builtin_skills/ai-toolbox/references/control-cli.md` 的命令树、`format × mode` 矩阵和退出码要与 `cli/src/args.rs` 一致，并跑 `pnpm run test:control-api-reuse`。

## Live E2E 验证 SOP（真实应用）

单测只能证明“路由调用了正确的函数”，证明不了授权弹窗、takeover 恢复/重接管、备份、WSL/SSH 同步、托盘事件这些真实副作用。任何新增或修改的 `aitoolbox` 命令（含 `aitoolbox` 参数、HTTP 路由、`cli_dispatch` 分支）在合并前，都必须按内置 Skill 对**正在运行的 debug 应用**做一轮 E2E；只跑单测不算完成。

### 1. 备份（先于任何写操作）

- 在项目外建带时间戳的备份目录，例如 `%TEMP%\aitoolbox-e2e-20261128-1530\`（macOS/Linux 用 `$TMPDIR`）。
- 备份 AI Toolbox 自己的数据：SQLite 数据库文件（含 `-wal`/`-shm`）、控制面发现文件相关设置、插件数据目录、`proxy-gateway/` 清单（`cli-proxy/*/manifest.json`、aggregate 草稿）。
- 备份本轮会触碰的每个 CLI 配置文件及其目录（Claude `~/.claude/settings.json`、Codex `~/.codex/config.toml`/`auth.json`、Gemini、Grok、Kimi 等）；保留原路径相对结构，恢复时才能核对。
- 记录测试前快照：文件 SHA-256、`aitoolbox gateway clis --json`、`aitoolbox cli list --json`、关键 provider/model 的 `aitoolbox ... --json` 输出（输出里的 `***` 原样保存即可，不要额外去取真实 Key）。

### 2. 每个用例先写四件事，再执行

| 项目 | 要求 |
| --- | --- |
| 现状 | 当前 CLI 是否被接管、主 provider、aggregate/failover 清单、配置文件路径与关键字段 |
| 预期 | 命令成功后的 DB/文件/网关/事件各应变成什么；失败时返回哪个稳定 code |
| 验证 | 用哪条 `aitoolbox` 只读命令、哪个文件 diff、哪条日志/事件来证明；不能只看退出码 |
| 回滚 | 若结果不符，先执行哪条命令/恢复哪个文件；需要用户操作时明确写出来 |

案例模板与各能力域的具体清单见 [`e2e-checklist.md`](e2e-checklist.md)。没有四要素就不要执行写命令。

### 3. 硬性禁令

- **禁止用 `claudecode` 做写测试**：`provider create/update/apply/delete`、`model add/delete/default`、`config set/extract`、`gateway engage/switch/restore` 一律不得对 `claudecode` 执行；只允许 `aitoolbox provider list/get`、`aitoolbox model list`、`aitoolbox config get`、`aitoolbox cli paths` 等只读操作。Paseo 的 Claude Code 子代理依赖这份配置，写坏会让当前会话和其他代理一起掉线。
- 其余 CLI 的写测试也要先确认没有别的 agent/用户正在使用；拿不准就先问。
- 不得用真实生产 Key 做“能不能写进去”的试验；需要新 provider 时用合成数据（见下）。

### 4. 合成数据与清理

- 没有可安全复用的数据时，创建带明显前缀的合成对象（如 `e2e-<timestamp>-provider`、模型 `e2e-model`），测试后删除；不要留下测试 provider、模型、提示词、插件或网关清单。
- 合成 provider 的 Key 用环境变量或占位值，不写入日志/提交/对话。
- 若测试必须触发真实上游请求（`provider test`/`fetch-models`），优先用本地 mock 或已知可失败的地址验证错误路径；不要把真实凭据发到外部。

### 5. 收尾（缺一不可）

1. 按备份恢复所有被改文件与 SQLite；逐项对比测试前快照（文件哈希、`gateway clis`、provider/model 列表）。
2. 删除合成数据；确认 `aitoolbox gateway clis` 回到测试前状态（该 direct 的已 restore，该接管的已按原清单重新 engage）。
3. 把控制面 `approval_mode` 重置为默认 `always_ask`（设置页或 `control_api` 设置行），避免留下 `auto_approve`。
4. 重启/刷新应用后跑 `aitoolbox status --json` 与一轮只读命令，确认发现文件、端口、权限正常。
5. 在测试记录里写清：执行了哪些用例、哪些通过、哪些未验证、备份目录位置。

### 6. 沉淀为自动化

E2E 中发现的有价值场景（真实 takeover 后的 create/update、aggregate 重接管、掩码回写、extract 读到代理配置等）必须转成自动化回归：优先 Rust 单测（纯逻辑/路由分发），其次 `cli/` 的参数/HTTP 测试，再次 `scripts/` 下的可重复脚本。只留在手工记录里的用例，下一轮会重新踩坑。

## 关键流程

```mermaid
sequenceDiagram
  participant Agent as aitoolbox / Agent
  participant Srv as hyper (127.0.0.1)
  participant Routes as routes*
  participant Gate as approve_write
  participant Cmd as existing Tauri command
  participant UI as Approval modal

  Agent->>Srv: Authorization: Bearer <token>
  Srv->>Srv: Host/Origin 校验 + 常量时间 Token 比较
  Srv->>Routes: /v1/...
  Routes->>Gate: L1/L2 写操作
  alt always_ask / ask_high_risk
    Gate->>UI: emit control-api-approval-request
    UI-->>Gate: control_api_resolve_approval(allow/deny)
  end
  Gate->>Cmd: app.state::<T>() + 既有参数
  Cmd-->>Routes: 原始结果
  Routes->>Routes: redact_value / redact_json_string
  Routes-->>Agent: JSON（无明文凭据）
```

插件写入走 `plugin_commands::*`；CLI 写入走 `cli_dispatch::*`；Gateway 写入走 `proxy_gateway::commands::*`。三者都不绕过既有命令。

## 易错点与历史坑（Gotchas）

- **Windows 栈溢出**：所有 `control_api_*` 命令必须收口在 `control_api::invoke_handler`（`#[inline(never)]`），由 `lib.rs` 在 `generate_handler!` 大 `match` 之前转发。不要把命令合并回根 handler；Windows MSVC debug 主线程栈只有 1 MiB，会把启动打爆。
- **`public_db_provider` 的 `hasApiKey` 必须在脱敏前计算**：`hasApiKey` 本身要加入 `redact_value` 的豁免名单（`normalized == "hasapikey"`），否则会被当成 `*ApiKey` 掩码掉。不要用公开 DTO 做 PATCH 的输入；更新路径必须重新读取原始 provider（含明文 settingsConfig），再做 secret-preserving merge。
- **`settingsConfig` 是字符串**：DB provider 的 settingsConfig 在存储层是 JSON 字符串，公开 GET 返回解析后的脱敏对象，PATCH 接受对象或字符串。`merge_settings_config` 必须先把对象合并回字符串再交给 update 命令，不能把临时 `Value` 丢弃。
- **Canonical 捷径必须落到各 CLI 的类型化结构**：`baseUrl` / `apiKey` / `model` 不是通用扁平字段。ZCode 写 `providerId`/`providerName`/`config.api`/`config.access`/`models`/`defaultModelId`；Grok 写顶层 `baseUrl` + `modelCatalog.models[]` + `defaultModelKey`；Kimi 写 `providerConfigs.<key>.base_url` + `modelCatalog.models[]` + `defaultModelKey`；Codex 写 `auth.OPENAI_API_KEY` 与 TOML `config`。新增 CLI 时先对照其 settings builder/validator，否则 create 能成功但 apply 会失败。
- **脱敏是结构 + 文本双层**：JSON 先结构脱敏，TOML 用 `toml_edit::DocumentMut`、YAML 用 `serde_yaml::Value` 逐表/逐值识别（包括 inline table 里的 `http_headers = { "x-api-key" = "..." }` 和裸 `token:`），最后才退回正则覆盖任意 `*_api_key` / `*_token` / `*_secret` / `*_password`。`apiKeyEnv`、`hasApiKey`、token 用量计数必须豁免。任何新的 GET/LIST/日志/网关响应都要显式调用 `redact_value`，不要假设底层日志已经脱敏；`ApiError::body()` 也必须过 `redact_text`。
- **掩码不可写回**：JSON patch/replace 用 `merge_patch_preserving_secrets` / `drop_redacted` 跳过 `***` 或 null secret；TOML/YAML 交给 `coding::config_patch` 做结构化递归合并（保留未知字段/注释），replace 时含 `***` 直接 400。控制面不得再出现 `toml_edit`/`serde_yaml`（`redact.rs` 的响应边界除外），`pnpm run test:control-api-reuse` 会拦截。`canonical_string` 与 OMO 的 create/update 必须显式拒绝 `is_redacted_value`，`update_omo_provider` 必须从原始 `models.json`/`auth.json` 起合并。这是“不传 Key 保持原 Key”的另一半。
- **config get/put 的 scope 是两套存储**：ZCode 的 `scope=cli` 读写 CLI 自己的配置，默认 `common` 读写公共配置；OMO Native 的 `scope=omo` 读写 `shared_base`，默认 `settings` 写 `other_settings`；OpenCode/OpenClaw 直接读写真实配置文件。`config extract` 只有 claudecode/codex/grok/kimi/geminicli/antigravity 有真实回读实现，其余必须 405，不能返回成功空对象。`extract` 读的是实时文件，必须包在 `with_gateway_reattach_api` 内；否则 takeover 期间会把网关代理配置当成用户的公共配置存回 DB。
- **读取配置不等于能写配置**：CodeBuddy 的 `settings.json` 由产品拥有，控制面 `readOnly: true`，PUT 必须拒绝；WorkBuddy 没有独立全局配置文件编辑入口，`/config` 返回只读错误，不要为对称性伪造写接口。
- **文件型 CLI 的 patch 必须保留未知字段**：OpenCode/OpenClaw/Pi/OMP/Hermes/DSH/OMO 的公开结构只是视图，写入要用模块的 `save_*` helper，不要在 `cli_dispatch` 手写整文件覆盖。
- **WorkBuddy/CodeBuddy 的 channel 身份是 `(baseUrl, apiKey)`**：channel id 由 FNV 派生；`update` 不能原地改这两项（会变成新 channel），模型编辑走 model 端点。`delete` 必须重新读取原始 channel 拿真实 key，不能使用脱敏后的 `***`。
- **Gateway 响应同样要脱敏**：status/logs/health/takeover 返回值可能包含 provider URL、header、请求体；`routes_gateway` 的每个 `to_value` 结果都要过 `redacted`。
- **Stop 保护来自后端**：`gateway.stop` 在仍有 CLI 被接管时会报错；不要为了“让 CLI 成功”在路由层跳过检查。接管/恢复本身是 L2，必须弹窗。
- **Host/Origin 防护不能放宽**：只接受 `Host: 127.0.0.1:<port>` / `localhost:<port>`，任何 `Origin` 头直接 403；Bearer 比较必须常量时间。不要加入 CORS 或“开发模式例外”。
- **发现文件删除要认 pid**：`remove_discovery_file_if_owned` 只删除属于当前进程的文件；测试/多实例时不要直接 `remove_file`。
- **CLI 安装路径**：Windows 写 HKCU `Environment\Path` 时必须保留 `REG_EXPAND_SZ` 的类型与 UTF-16 编码；解码现有值要裁掉尾部 `\0`，否则最后一个 PATH 项会带空字符。不要用 `setx`（会截断）或把 PATH 写成 `REG_SZ`。Unix 安装写标记注释 + `export PATH="..."` 两行，卸载必须两行一起删（只删注释会留下死 export）。找不到 `aitoolbox` 二进制时返回明确错误，不要把应用自身复制/改名。
- **授权超时与策略切换竞态**：超时分支必须先从 hub 移除再发 `control-api-approval-expired`，否则前端允许按钮会打到一个已死的 channel。停止控制面或切到 `read_only` 时必须 `ApprovalHub::cancel_all()` 并广播 expired；前端收到 `resolved: false` 只提示过期，不要重试 resolve。
- **插件页面路由会在 reload 后过期**：`page_server.sync_enabled_plugins` 不仅检查插件启用/page 是否存在，还比较入口文件名和解析后的 `service_port`；任一变化就撤销旧 token，让下次打开重新签发。否则新清单会继续用旧目录/旧 CSP 端口服务。

## 跨模块依赖

- 依赖 `db/`（`SqliteDbState`、`DbTable::Settings`）、`coding/<cli>/commands` 与 `types`、`coding/proxy_gateway/{commands,types}`、`plugins/{commands,store,installer}`、`coding/skills/builtin.rs`（内置技能嵌入 `references/control-cli.md`）、`app_paths::resolved_data_dir`。
- 被 `lib.rs` 托管（`ControlApiState` + `start_if_enabled` + `RunEvent::Exit` 清理）并转发 `control_api_*` 命令。
- 前端契约：`web/services/controlApi.ts`、`web/features/settings/components/ControlApiSettingsCard.tsx`、`web/components/common/ControlApiApprovalModal.tsx`。字段名（settings snake_case，status camelCase）是公开契约，改动要同步前端与 `references/control-cli.md`。
- `aitoolbox`（`cli/`）消费发现文件和 HTTP 契约；退出码 0/2/3/4/5 是 Agent 依赖的稳定接口，不要改语义。

## 最小验证

- `cd tauri && cargo test --lib control_api --jobs 2`
- `cd tauri && cargo test --lib plugins:: --jobs 2`（插件路由复用回归）
- `cd tauri && cargo check`
- `cd cli && cargo test --offline`
- `pnpm run test:control-api-reuse`（含脚本自检）
- 手工：设置页开关 → `aitoolbox status --json`；创建 provider 触发弹窗 → 拒绝看退出码 4 → 允许后 GET 确认已写入；`GET` 响应中搜索真实 Key 确认不存在；`aitoolbox config get` 的掩码原样 PATCH 后 GET 确认 Key 未变。涉及写操作的完整回归按上面的 Live E2E SOP 执行。

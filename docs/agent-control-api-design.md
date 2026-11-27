# AI Toolbox Agent Control API & CLI 详细设计与执行方案

## 1. 背景与目标

当前 AI Toolbox 的全部管理能力（插件生命周期、17 个 CLI 的供应商/模型/全局配置/全局提示词、Proxy Gateway 运行态与排障）仅通过 Tauri IPC 向桌面前端暴露。外部 Coding Agent 在帮助用户开发自定义插件或配置工具时，无法自行完成「安装 → 热重载 → 读日志 → 修改配置 → 连通性测试」闭环，仍需用户频繁切回桌面端手动点击或重启。

本方案参考 Paseo 的「常驻进程唯一事实源 + 本地控制面 + 薄 CLI + 行动导向 Skill」设计，为 AI Toolbox 新增：

1. **独立本机控制面 HTTP API（`tauri/src/control_api/`）**：绑定 `127.0.0.1:0`，每次启动生成随机 Bearer Token，写入 `0600` 权限发现文件 `~/.ai-toolbox/control.json`。
2. **默认需授权的安全确认机制（可在设置页修改）**：支持四档策略（`always_ask` 默认所有写操作弹窗授权、`ask_high_risk` 仅高危操作弹窗、`auto_approve` 自动允许本机 Agent、`read_only` 仅只读）以及本地插件热重载快捷免弹窗开关。
3. **轻量化独立 CLI（`aitoolbox`）**：零 Tauri/WebView 依赖，内部读取 `control.json` 自动携带 Token（Token 永不进入 Agent 对话上下文），统一支持 `--json` 与标准退出码；设置页提供「安装 CLI 到 PATH」一键入口。
4. **全量复用现有后端 Service 层**：覆盖 **插件生命周期与热重载**、**各 CLI 渠道与模型 CRUD/测试/应用**、**各 CLI 全局配置文件（Common Config / Other Settings / Global Prompt / Runtime Paths）**、**Proxy Gateway 状态/日志/健康/接管**。
5. **内置 `ai-toolbox` Skill 升级**：从「点按钮说明书」升级为「Agent 行动 SOP + 故障排查表」。

---

## 2. 核心工程原则（严格遵守）

1. **最大程度复用既有代码，绝不另起炉灶**：
   - **插件域**：直接复用 `tauri/src/plugins/` 既有 `installer` / `store` / `bun_host` / `middleware_rt` / `page_server` / `cli_integration` 逻辑。将 `tauri/src/plugins/commands.rs` 的业务核心下沉为接受 `&PluginManagerState`（和按需 `&AppHandle`）的 `pub(crate)` 函数，Tauri command 与 `control_api` 共同调用；新增的原子 `reload_plugins(manager, app, id: Option<&str>)` 同样同时服务前端刷新与控制面。
   - **渠道与模型域**：**严禁**为 17 个 CLI 重写配置读写器。
     - 跨工具标准化新增/映射优先复用 `tauri/src/coding/deeplink/`（`importer.rs` / `provider.rs` / `portable.rs`）与 `tauri/src/plugins/cli_integration.rs` 的规范写入适配；
     - 列表、详情、更新、删除、应用、模型增改均通过统一的薄分发层（`control_api/cli_dispatch.rs`）直接调用各 CLI 模块既有内部函数（如 `list_*_providers`、`create_*_provider_inner`、`update_*_provider`、`delete_*_provider`、`apply_*`、`save_*_models_provider`、`save_*_model`、`delete_*_model` 等），从而自动继承防回环校验（`self_loop`）、注释/未知字段保留、备份、事件广播（`config-changed` / `wsl-sync-request-*`）与托盘刷新。
     - 连通性测试直接复用 `proxy_gateway::runtime::test_gateway_provider_model_connectivity_with_plugins`。
   - **各 CLI 全局配置文件域**：直接复用各模块既有的 `get_*_common_config` / `save_*_common_config` / `extract_*_common_config_from_current_file`、`save_*_other_settings` / `save_*_other_config` / `read_*_runtime_config` / `read_*_settings`、`list/create/update/delete/apply/disable_*_prompt_config` 与 `runtime_location`。
   - **Proxy Gateway 域**：直接复用 `proxy_gateway::commands` / `runtime` / `cli_proxy` / `usage_stats` / `model_health`（尤其严格复用 `proxy_gateway_stop` 的接管保护硬检查）。
   - **HTTP Server 复用已有依赖**：复用 `Cargo.toml` 已有的 `hyper` + `tokio` + `ring` + `serde_json`，不引入额外重型 Web 框架。
2. **绝不触碰 `lib.rs` 根 `generate_handler!` 栈风险**：
   - 新增的 control_api 相关 Tauri 命令统一收口到独立的 `#[inline(never)]` 路由函数 `control_api::invoke_handler`（参照 `plugins::invoke_handler` 的成熟模式），绝不把新命令堆回 `lib.rs` 700+ 分支的大 `match`。
3. **凭据零泄漏红线**：
   - 任何控制面 GET/LIST 响应绝不返回明文 API Key、OAuth `access`/`refresh` token、`plugin-auth.json` 内容或备份加密口令；统一经 `control_api/redact.rs` 脱敏（输出 `hasApiKey` / 掩码提示等）。
   - CLI 写入凭据支持 `--api-key-env <VAR>` / `--api-key-stdin`（优先推荐），避免明文落在 shell history 与 Agent 对话记录中。
4. **严格遵守模块 `AGENTS.md` 与 `DESIGN.md`**：
   - 改动任意模块前先读该目录及父目录 `AGENTS.md`；
   - 前端 UI 严格遵守根目录 `DESIGN.md`（使用 `web/App.css` CSS 变量与 Ant Design token、不硬编码明暗色、不卡片套卡片、默认原生 Modal chrome、按钮留白规范、亮/暗/system 主题检查）。

---

## 3. 架构设计

### 3.1 总体拓扑

```text
Coding Agent / Terminal
   │  (aitoolbox <group> <action> --json)
   ▼
独立薄 CLI 二进制 (cli/ -> aitoolbox)
   │  读取 ~/.ai-toolbox/control.json (0600)
   │  HTTP 127.0.0.1:<random_port> + Authorization: Bearer <token>
   ▼
Tauri 主进程内 Control Plane (tauri/src/control_api/)
   ├── 1. 安全中间件：Loopback Host/Origin 校验 + Bearer Token 校验
   ├── 2. 权限与授权门禁 (approval.rs)：
   │      ├── 读取持久化策略 (ControlApiSettings)
   │      ├── L0 只读 -> 直接放行
   │      ├── L1/L2 写操作 -> 按策略决定：直接放行 / 拒绝 / 挂起等待桌面端授权弹窗确认
   │      └── 弹窗通过 Tauri event `control-api-approval-request` 推送前端，前端调用 `control_api_resolve_approval` 唤醒 oneshot
   └── 3. 薄 Router -> 直接调用各域复用 Service：
          ├── plugins 复用 service (含新增原子 reload)
          ├── control_api::cli_dispatch (provider / model / common_config / other_settings / prompt / runtime_paths)
          └── proxy_gateway 既有服务
```

### 3.2 发现文件与安全边界

- **路径**：用户主目录 `~/.ai-toolbox/control.json`（若 `AI_TOOLBOX_CONTROL_FILE` 环境变量存在则覆盖，方便隔离集成测试）。
- **权限**：Unix `0o600`；Windows 仅当前用户目录/私有权限。
- **内容**：
  ```json
  {
    "version": 1,
    "pid": 12345,
    "url": "http://127.0.0.1:54321",
    "token": "<64-hex random token from ring::rand::SystemRandom>",
    "appVersion": "1.2.4",
    "dataDir": "...",
    "startedAt": "2026-11-27T21:40:00+08:00"
  }
  ```
- **生命周期**：
  - 应用启动完成 `PluginManagerState` / `ProxyGatewayState` / `SqliteDbState` 初始化后，根据 `ControlApiSettings.enabled` 决定是否启动监听；
  - 设置页切换开关时可热启停监听（无需重启应用），停止或进程退出时删除 `control.json`（只删 pid 匹配自己的文件，防并发误删）。
- **HTTP 防护**：
  - 仅绑定 `127.0.0.1:0`；
  - 拒绝任意带 `Origin` 头的请求（防浏览器页面 CSRF）；
  - 校验 `Host` 头必须是 `127.0.0.1:<port>` 或 `localhost:<port>`；
  - 除 `GET /v1/ping` 外所有路由强校验 `Authorization: Bearer <token>`（使用常量时间比较 `ring::constant_time`）。

### 3.3 授权策略与桌面确认机制（默认需要授权，设置可改）

#### 持久化设置模型（`ControlApiSettings`）
将 `control_api` 作为 `settings` 表中独立 id=`control_api` 的 JSONB 记录（与 `backup_repository` 同理，独立于 `settings/app`，零破坏 `AppSettings` 全量保存契约），通过独立命令 `control_api_get_settings` / `control_api_save_settings` 读写：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlApiApprovalMode {
    /// 默认：所有写操作（L1 + L2）均弹窗请求用户确认
    AlwaysAsk,
    /// 平衡：L1 可逆变更自动放行，仅 L2 高危操作弹窗确认
    AskHighRisk,
    /// 信任：本机持有有效 Token 的 Agent 全部自动放行
    AutoApprove,
    /// 只读：仅允许 L0 只读查询，所有写操作直接返回 403
    ReadOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlApiSettings {
    /// 控制面总开关（默认 true，因默认 AlwaysAsk 所有写操作需人工授权）
    pub enabled: bool,
    /// 授权模式（默认 AlwaysAsk，可在设置页改为 AskHighRisk / AutoApprove / ReadOnly）
    pub approval_mode: ControlApiApprovalMode,
    /// AlwaysAsk 模式下，允许本地目录插件 `plugin reload` 自动放行（默认 false）
    pub auto_approve_local_plugin_reload: bool,
    /// 弹窗确认超时秒数（默认 60s，范围 15..=300）
    pub approval_timeout_secs: u64,
}
```

#### 权限级别划分
- **L0（ReadOnly，免弹窗）**：
  - `GET /v1/status`、`GET /v1/clis`
  - `GET /v1/plugins`、`GET /v1/plugins/:id`、`GET /v1/plugins/logs`、`GET /v1/plugins/middleware`、`GET /v1/plugins/providers`、`GET /v1/plugins/accounts`
  - `GET /v1/clis/:cli/providers`、`GET /v1/clis/:cli/providers/:id`、`GET /v1/clis/:cli/providers/:id/models`、`POST /v1/clis/:cli/providers/:id/test`、`POST /v1/clis/:cli/providers/:id/fetch-models`
  - `GET /v1/clis/:cli/config`、`GET /v1/clis/:cli/prompts`、`GET /v1/clis/:cli/paths`
  - `GET /v1/gateway/status`、`GET /v1/gateway/logs`、`GET /v1/gateway/health`
- **L1（Standard Write，可逆变更）**：
  - 本地目录 `POST /v1/plugins/install`、`POST /v1/plugins/reload`、`POST /v1/plugins/:id/enable`、`POST /v1/plugins/:id/disable`、`PUT /v1/plugins/:id/options`、`PUT /v1/plugins/providers/:provider/options`、`POST /v1/plugins/providers/:provider/refresh-models`、`POST /v1/plugins/accounts/check`、`POST /v1/plugins/accounts/usage`、`POST /v1/plugins/accounts/checkin`、`POST /v1/plugins/providers/:provider/link-cli`
  - `POST /v1/clis/:cli/providers`、`PATCH /v1/clis/:cli/providers/:id`、`POST /v1/clis/:cli/providers/:id/apply`
  - `POST/PATCH/DELETE /v1/clis/:cli/providers/:id/models...`、`PUT /v1/clis/:cli/providers/:id/default-model`
  - `PUT /v1/clis/:cli/config`、`POST /v1/clis/:cli/config/extract`
  - `POST/PUT/DELETE/POST:apply/POST:disable /v1/clis/:cli/prompts...`
  - `POST /v1/gateway/restart`、`POST /v1/gateway/health/reset`
- **L2（High Risk，信任边界 / 不可逆 / 破坏性）**：
  - 远程 `npm` / `github:` 来源的 `POST /v1/plugins/install`
  - `DELETE /v1/plugins/:id`
  - `DELETE /v1/clis/:cli/providers/:id`
  - `POST /v1/gateway/start`、`POST /v1/gateway/stop`、`POST /v1/gateway/takeover/engage`、`POST /v1/gateway/takeover/restore`

#### 实时授权弹窗流转
1. 控制面收到需确认请求 → 生成 `requestId` + 人类可读摘要（操作名、风险等级 L1/L2、目标 CLI/插件、脱敏参数摘要，**绝不显示 API Key 明文**）；
2. 在内存 `ApprovalHub` 注册 `oneshot::channel`，向前端发 Tauri 事件 `control-api-approval-request`（若主窗口隐藏/最小化则自动显示聚焦）；
3. 前端全局组件 `ControlApiApprovalModal` 队列展示请求、倒计时与「拒绝 / 允许本次」按钮，点击后调 `control_api_resolve_approval`；
4. 超时返回 HTTP `408 approval_timeout`，拒绝返回 HTTP `403 approval_denied`，CLI 均退出码 `4`。

---

## 4. API 与能力详细规格

### 4.1 插件域（`/v1/plugins/...`）
- `GET /v1/plugins`、`GET /v1/plugins/:id`
- `POST /v1/plugins/install` (`{ spec, marketId? }`)
- `POST /v1/plugins/reload` (`{ id? }`：重扫清单 → 刷 middleware → 同步 page_server → 按需重启 Bun host → emit `plugins-changed`)
- `POST /v1/plugins/:id/enable`、`POST /v1/plugins/:id/disable`、`DELETE /v1/plugins/:id?purgeData=false`
- `GET/PUT /v1/plugins/:id/options`
- `GET /v1/plugins/providers`、`PUT /v1/plugins/providers/:providerId/options`、`POST /v1/plugins/providers/:providerId/refresh-models`、`POST /v1/plugins/providers/:providerId/link-cli`
- `GET /v1/plugins/accounts`、`POST /v1/plugins/accounts/{check,usage,checkin}`
- `GET/DELETE /v1/plugins/logs`、`GET /v1/plugins/middleware`

### 4.2 渠道与模型域（`/v1/clis/...`）
统一 `cliKey`（支持常见别名如 `claude` → `claudecode`、`gemini` → `geminicli`）：
- DB 接管型（6）：`claudecode`、`claudedesktop`、`codex`、`grok`、`kimi`、`geminicli`
- 文件配置型（10）：`opencode`、`openclaw`、`pi`、`oh_my_pi`、`omo_native`、`hermes`、`dsh`、`zcode`、`workbuddy`、`codebuddy`
- 纯官方账号/配置型（1）：`antigravity`（支持全局配置与全局提示词，不支持自定义渠道 CRUD）

端点：
- `GET /v1/clis`（17 CLI 能力矩阵）
- `GET /v1/clis/:cli/providers`、`GET /v1/clis/:cli/providers/:id`（严格脱敏）
- `POST /v1/clis/:cli/providers`（新增，支持 Canonical 字段 + 复用 `deeplink` / 各模块 helper + `self_loop` 校验）
- `PATCH /v1/clis/:cli/providers/:id`（Patch 修改，不传 `apiKey` 保持原 Key）
- `DELETE /v1/clis/:cli/providers/:id`、`POST /v1/clis/:cli/providers/:id/apply`、`POST /v1/clis/:cli/providers/:id/test`
- `GET/POST /v1/clis/:cli/providers/:id/models`、`DELETE /v1/clis/:cli/providers/:id/models/:modelId`、`PUT /v1/clis/:cli/providers/:id/default-model`、`POST /v1/clis/:cli/providers/:id/fetch-models`

### 4.3 每个 CLI 的全局配置文件域（`/v1/clis/:cli/config` & `/prompts` & `/paths`）
1. **路径与位置信息**：`GET /v1/clis/:cli/paths`（复用 `runtime_location` 返回 `rootDir` / `configPath` / `promptPath` / `source` / `isWslDirect`）。
2. **通用配置 / 其他全局设置**：`GET / PUT /v1/clis/:cli/config`、`POST /v1/clis/:cli/config/extract`（携带 `kind`、`format`、`readOnly`，支持 `mode: "patch" | "replace"`，脱敏不暴露凭据）：
   - DB 通用配置型：`claudecode`、`claudedesktop`、`codex`、`grok`、`kimi`、`geminicli`、`antigravity`、`zcode`（`?scope=common|cli`）、`opencode`、`openclaw`；
   - 运行时其他设置型：`pi`、`oh_my_pi`、`omo_native`（`?scope=omo|settings`）、`hermes`、`dsh`、`codebuddy`（`readOnly: true` 拒绝写入）。
3. **全局提示词方案**：`GET/POST /v1/clis/:cli/prompts`、`PUT/DELETE /v1/clis/:cli/prompts/:id`、`POST /v1/clis/:cli/prompts/:id/apply`、`POST /v1/clis/:cli/prompts/disable`（全部复用各模块既有实现）。

### 4.4 Proxy Gateway 域（`/v1/gateway/...`）
- `GET /v1/gateway/status`、`POST /v1/gateway/{start,stop,restart}`
- `POST /v1/gateway/takeover/{engage,restore}`
- `GET /v1/gateway/logs`、`GET /v1/gateway/health`、`POST /v1/gateway/health/reset`

---

## 5. 独立轻 CLI（`cli/` → `aitoolbox`）设计

- 独立轻 crate `cli/`（不依赖 Tauri/WebView，使用 `clap` + 轻量 HTTP client + `serde_json`）；
- 读取 `~/.ai-toolbox/control.json`（或 `AI_TOOLBOX_CONTROL_FILE`）自动带 Token，不打印 Token；
- 支持 `status`、`plugin ...`、`cli ...`、`provider ...`（含 `--api-key-env` / `--api-key-stdin`）、`model ...`、`config ...`、`prompt ...`、`gateway ...`、`--json`、`--timeout`（默认 75s）；
- 退出码：`0` 成功、`2` 参数错误、`3` 应用未运行/控制面关闭、`4` 权限拒绝/用户取消或超时、`5` 执行错误；
- 后端提供 `control_api_cli_status` / `control_api_install_cli` / `control_api_uninstall_cli` 支持设置页一键安装/卸载 `aitoolbox` 到 PATH。

---

## 6. 前端 UI 设计（严格遵守 `DESIGN.md`）

1. **设置页「Agent 集成 / 本地控制」分区**（接入 `GeneralSettingsPage.tsx`）：
   - 启用开关 + 监听状态 Tag + 发现文件路径（绝不显示 Token）；
   - 授权策略选择（默认 `always_ask`，支持切 `ask_high_risk` / `auto_approve` / `read_only`）；
   - 本地开发插件热重载免弹窗开关；
   - `aitoolbox` CLI 安装状态与一键安装/移除按钮；
   - 中英文 i18n 完整同步，遵循 `DESIGN.md` 颜色 token、布局密度、按钮留白规范。
2. **全局授权确认弹窗（`ControlApiApprovalModal`）**：
   - 挂载到顶层布局，监听 `control-api-approval-request` / `control-api-approval-expired`；
   - Ant Design 原生 Modal chrome，展示风险等级标签、操作名、目标对象、脱敏参数摘要、倒计时、拒绝/允许按钮。

---

## 7. 内置 Skill 升级

更新 `tauri/resources/builtin_skills/ai-toolbox/`：
1. `SKILL.md`：升级为 Agent 行动 SOP（先查 `aitoolbox status --json`、优先通过 `aitoolbox` 完成操作与验收、说明授权确认与退出码 4、禁止绕过 CLI 改 DB/受管文件、安全红线）；
2. `references/plugins.md`：将本地调试升级为 `编写/构建 → aitoolbox plugin install → aitoolbox plugin reload → inspect/logs/middleware status → link-cli/test` 全自动闭环，补充 Symptom → Check 故障表；
3. 新增 `references/control-cli.md`：精简命令树、`cliKey`/协议/`scope` 枚举与退出码速查；并确认 `tauri/src/coding/skills/builtin.rs` 同步包含该文件。

---

## 8. 分阶段执行与验收清单

- **Milestone 1**：插件 Service 复用与原子 `reload_plugins` + `tauri/src/control_api/`（settings/approval/redact/server/routes_plugins/commands/invoke_handler）+ 单元测试与真实 HTTP 集成测试。
- **Milestone 2**：`cli_dispatch.rs`（17 CLI 渠道/模型/全局配置文件/全局提示词/路径复用分发）+ `routes_gateway.rs` + DB/文件型代表 CLI 集成测试（脱敏、patch 保 Key、codebuddy 只读、gateway stop 保护）。
- **Milestone 3**：独立轻 crate `cli/`（`aitoolbox`）+ PATH 安装/卸载命令 + CLI 单测与 mock HTTP 测试。
- **Milestone 4**：前端设置分区 + 全局授权弹窗 + i18n + 内置 Skill 更新 + `tauri/src/control_api/AGENTS.md` 及根/模块 `AGENTS.md` 更新。
- **Milestone 5**：全量验收（`cargo test --lib control_api`、`cargo test --lib plugins::`、`cd cli && cargo test`、`cargo check`、`pnpm typecheck`、`pnpm test`、`pnpm build`）并按模块粒度提交到 `feat/agent-control-api`（不 push）。

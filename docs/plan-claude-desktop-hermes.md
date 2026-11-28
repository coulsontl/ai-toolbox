# Plan: 新增 Claude Desktop 与 Hermes 支持

## 目标

在 AI Toolbox 中新增两个 coding 工具模块:

1. **Claude Desktop**(GUI 客户端,非 CLI)——用 **cc-switch 的 3P profile** 方案(reference),UI 复用现有 **claude code** 页样式。
2. **Hermes Agent**(CLI)——参考 cc-switch 的 `config.yaml` 语义,UI 复用现有 **Pi** 页样式。

同时:把现有 **Claude → Claude Code** 改名(区分新增的 Claude Desktop),并为两个新模块接入 ssh/wsl 同步、备份恢复、MCP、托盘、会话管理,图标上 Hermes 用 `@lobehub/icons` 的 `HermesAgent`。

> 关键决策(已与用户确认):
> - Claude Desktop 采用 **cc-switch 的 3P profile(deploymentMode + configLibrary profile + _meta)** 方案,支持 Direct/Proxy 两种模式。
> - 同步接入范围 = **全量**(备份 + MCP + WSL/SSH 默认映射 + 托盘 + 会话管理);Skill 仅在 CLI 自身支持时同步。
> - 已确认:本项目本地已有 `proxy_gateway`,Desktop 的 Proxy 模式接入的是**本项目自己的网关**,不照搬 cc-switch 的网关。

---

## 一、调研结论(Source of Truth)

### 1.1 参考项目 cc-switch 的关键事实

| 工具 | 配置对象 | 平台路径 |
|------|---------|---------|
| Claude Desktop | `claude_desktop_config.json` + `configLibrary/<PROFILE_ID>.json` + `configLibrary/_meta.json` | Win `%LOCALAPPDATA%\Claude`(+`Claude-3p`);mac `~/Library/Application Support/Claude`(+`Claude-3p`);Linux 不支持 |
| Hermes | `config.yaml`(YAML) | Win `%LOCALAPPDATA%\hermes`;mac/Linux `~/.hermes`;环境变量 `HERMES_HOME` 可覆盖 |

> **2026-11 路径修正**:上表是初版方案(照 cc-switch)的路径。后续按 Claude Desktop 本体行为修正为:1P 配置在 `%LOCALAPPDATA%\Claude-Data\claude_*`(3.x)> `%APPDATA%\Claude` > MSIX `LocalCache\Roaming\Claude`;3P 根目录在真实 `%LOCALAPPDATA%\Claude-3p` 或 MSIX `LocalCache\Local\Claude-3p`,按最后使用时间选择。详见 `tauri/src/coding/claude_desktop/AGENTS.md`。

**Claude Desktop 3P 方案要点**:
- 两个 `claude_desktop_config.json`(normal + 3p)都只写 `deploymentMode`(`"3p"` / 官方 `"1p"`),**保留文件其它字段**(含 `mcpServers`)。
- 真正的推理配置写在 `configLibrary/<PROFILE_ID>.json`:
  ```json
  {
    "coworkEgressAllowedHosts": ["*"],
    "disableDeploymentModeChooser": true,
    "inferenceGatewayApiKey": "<auth_token|gateway_token>",
    "inferenceGatewayAuthScheme": "bearer",
    "inferenceGatewayBaseUrl": "<direct base_url | proxy gateway>",
    "inferenceProvider": "gateway",
    "inferenceModels": [ {"name":"claude-sonnet-4-6","labelOverride":"Kimi K2","supports1m":true}, "claude-opus-5" ]
  }
  ```
- `_meta.json` 维护 `appliedId` + `entries`(去重保留其它官方 profile)。
- Direct 模式:凭据取 provider `settings_config.env` 的 `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`;Direct 不允许模型映射(只允许 claude-safe 模型)。
- Proxy 模式:base url 指向本地网关,`inferenceGatewayApiKey` 为网关 token;非 claude-safe 模型经 `catalog-safe-route` 改名 + `labelOverride` 回填。
- 模型 `is_claude_safe_model_id` 硬校验(claude-/*anthropic/claude- 前缀 + 角色后缀 + 非空标识)。
- 写盘带**快照回滚**(`snapshot_files`/`restore_snapshots`)与**原子写 + JSON 键排序**(确定性输出)。
- 判定当前状态:读 `meta_path`(appliedId)+ `profile_path`(baseUrl/models),不读 deploymentMode 文件。
- `import_claude_desktop_providers_from_claude` 从 Claude Code 表按 Direct/Proxy 判定转成桌面 provider;官方 seed `claude-desktop-official`。

**Hermes `config.yaml` 语义**:
- `custom_providers:`(列表,`{name, base_url, api_key, api_mode, models:{dict}, ...}`),**累加式**,所有 provider 共存。
- 顶层 `model:`(`{default, provider, base_url, context_length, max_tokens}`)表达默认选择;切换只更新 `model.provider` + `model.default`。
- `mcp_servers:`(YAML dict)——MCP 导入/同步目标。
- `agent:`(max_turns/reasoning_effort)。只读 `providers:` 字典(Hermes v12+,引导到 Web UI)。
- **section 级文本替换**写盘,保留注释与其他 section;带时间戳备份文件回滚。

### 1.2 本项目复用/模板事实

- **Claude Code 是「根目录模块」**;Claude Desktop 是 **「配置文件路径模块」**(像 opencode/openclaw,根不是 provider 语义,而是固定平台路径)。**不要**把 Desktop 当成根目录模块。
- **Hermes 是「配置文件路径模块」**,与 Pi/OMP 的资源发现语义接近(provider 事实源是运行时 YAML,不是 DB provider 表),DB 只存配置目录选择 + prompt 预设 + 网关/官方 seed。
- 内部 **sync key**:本项目 WSL/SSH、托盘、gateway 等用 `claude` 代表 Claude Code(不是 `claudecode`)。Desktop sync key 用 `claude_desktop`,Hermes 用 `hermes`。
- **图标**:Claude Code 用 `web/assets/claude.svg`(MainLayout `TAB_ICONS`)。`@lobehub/icons` 当前 **4.9.0 不含 `HermesAgent`**,需升级到 5.x(5.16.0 已含)。Claude Desktop 图标复用 `claude.svg`。
- 子 Tab 图标三选一:lobehub 品牌图标 / `TAB_ICONS[tab.key]`(svg)/ null。

---

## 二、后端新增模块

### 2.0 共享基础(两个模块都需要的注册点)

- `tauri/src/coding/mod.rs`:加 `pub mod claude_desktop;` `pub mod hermes;`
- `tauri/src/db/schema.rs`:`DbTable` 枚举加 `ClaudeDesktopProvider/ClaudeDesktopCommonConfig`、`HermesSettingsConfig/HermesPromptConfig`;`name()` 映射加 `claude_desktop_provider`/`claude_desktop_common_config`/`hermes_settings_config`/`hermes_prompt_config`;`ALL_TABLES` 加入。
- `tauri/src/db/migrations.rs`:bump `TARGET_SCHEMA_VERSION` 9→10,新增 `migrate_v10` 用 `create_jsonb_table` 建以上 4 表(+`is_applied`/`sort_index` 索引,参照 `migrate_v9`),注册进 `run_all` 链。
- `tauri/src/lib.rs` `invoke_handler`:逐条注册两模块全部 `#[tauri::command]`(注意命令名前缀避免冲突)。
- `tauri/src/tray.rs`:两模块 `tray_support` 接入(菜单 section 文案、provider/prompt 事件分支、`is_tab_visible(...)` 控制)。
- `tauri/src/coding/reapply_applied_runtime.rs`、`session_manager/mod.rs`(`SessionTool` + 全部分派)、`coding/deeplink/provider.rs`、`tools/builtin.rs`、`runtime_location.rs` 各加两模块键。
- `config_cleanup.rs`:两模块的目标端通用清理(非 Windows 目标处理,视需要)。

### 2.1 Claude Desktop 后端 `tauri/src/coding/claude_desktop/`

仿 claude_code 分文件:

- `constants.rs`:`CONFIG_FILE="claude_desktop_config.json"`、`CONFIG_LIBRARY_DIR="configLibrary"`、`PROFILE_ID`、`PROFILE_NAME`、`CLAUDE_DESKTOP_PROXY_PREFIX`(本项目用自身网关前缀,如 `/claude-desktop`)、`DEFAULT_PROXY_ROUTES`(sonnet-5/opus-5/haiku-4-5/fable-5)。
- `paths.rs`(或并入 commands):`ClaudeDesktopPaths` + `current_platform_paths()`(Win/mac 两平台;Linux 报不支持)。
- `types.rs`/`adapter.rs`:provider/common 记录;`ClaudeDesktopMode {Direct,Proxy}`(存 provider `meta.claude_desktop_mode`)、模型路由 `meta.claude_desktop_model_routes`。
- `commands.rs`:
  - `get_claude_desktop_paths`、`get_claude_desktop_status`(读 meta/profile)、`list/create/update/delete/reorder_claude_desktop_provider`、`select/apply_claude_desktop_config`。
  - `apply_provider_to_paths`:`with_rollback` → 写两个 config 的 `deploymentMode="3p"`、写 profile JSON、维护 `_meta.json`。
  - `restore_official_at_paths`(切回官方 `1p`):改 deploymentMode、清 `enterpriseConfig` 相关键、删 profile、清 meta。
  - `is_claude_safe_model_id`、`direct_gateway_credentials`、`proxy_model_routes`、`build_gateway_profile`、`inference_model_json`。
  - `import_claude_desktop_providers_from_claude`、`ensure_claude_desktop_official_provider`。
  - 网关锁:`claude_desktop_gateway_takeover_active` / `ensure_claude_desktop_gateway_direct`(仿 claude_code)。
  - apply 末尾 emit `config-changed` + `wsl-sync-request-claudedesktop`(?;Desktop 是 GUI,事件命名按本项目约定,若 Desktop 无 WSL 可用则只发 `config-changed`)。
- `gateway` 集成(见 2.3)。
- 不实现:prompt(CLAUDE.md)、plugins、skills、CLI 启动(Desktop 是 GUI,无 CLI shutdown;/`cli_launch.rs` 可改为 `reveal`/打开应用,可选)。

### 2.2 Hermes 后端 `tauri/src/coding/hermes/`

**以 `pi` 模块为模板克隆**(provider 事实源在运行时文件),做 YAML 语义改造:

- `constants.rs`:`HERMES_ENV_KEY="HERMES_HOME"`、`HERMES_CONFIG_FILE="config.yaml"`、`HERMES_BUILTIN_PROVIDERS`。
- `types.rs`/`adapter.rs`/`commands.rs`:
  - DB:`hermes_settings_config`(存配置目录/`config_dir`,id 固定 `"common"`)+ `hermes_prompt_config`(SOUL.md 预设)。
  - runtime view:`read_hermes_runtime_config`(读 config.yaml)→ provider views(`custom_providers` 并入只读 `providers` dict)+ 顶层 `model`。
  - `save_hermes_settings_config`(配置目录)、`save_hermes_models_provider`/`delete_hermes_runtime_provider`(upsert/remove `custom_providers`)、`save_hermes_model_settings`(顶层 `model`)、`save_hermes_other_settings`(agent 等,保留受管键之外)。
  - YAML 写:**section 级文本替换**保留注释(参考 cc-switch `write_yaml_section_to_config_locked` / ai-toolbox `oh_my_pi` 的 YAML 读写),**带时间戳备份**,原子写。
  - 默认配置目录与环境变量 HERMES_HOME 解析;`normalize_hermes_root_dir`(WSL 归一化)。
  - prompt CRUD + 写 `<root>/SOUL.md`(Hermes 仅从 `HERMES_HOME/SOUL.md` 读取全局提示词)。
  - emit `config-changed` + `wsl-sync-request-hermes`。
- `cli_resolver.rs`:`resolve_local_hermes_program`(候选路径)。
- `tray_support.rs`:模型/tray + prompt tray。
- MCP:`mcp_servers:` 段读写 + 同步(见 2.4)。
- **Extensions**:cc-switch 未管理 Hermes 插件,首版**不做** hermes extension 面板(待确认 Hermes 是否支持)。

### 2.3 Claude Desktop 接入本项目 `proxy_gateway`(Proxy/网关)

复用现有 `GatewayCliKey`/`provider_switch`/`cli_proxy`/`runtime` 一整套,新增 `GatewayCliKey::ClaudeDesktop`,并按调研报告的 A–J 清单逐处补齐:

- `proxy_gateway/types.rs`:`GatewayCliKey` 加 `ClaudeDesktop`;`as_str()="claude_desktop"`;`supported_mvp()` 加入。
- `provider_protocol.rs`:`native_cli_protocol` 加 `ClaudeDesktop→AnthropicMessages`;`provider_needs_gateway_proxy`/`provider_target_protocol` 加分支。
- `provider_switch.rs`:`provider_table()` 加映射;`apply_direct_provider[_without_events]` 加 match 臂 → `claude_desktop::commands::apply_config_internal_*`;`emit_gateway_cli_wsl_sync_request` 加事件名。
- `cli_proxy/mod.rs`:`is_supported_cli`、`resolve_targets`(读 desktop 路径)、`apply_gateway_config`/`restore_gateway_config`/`cli_gateway_endpoint`(desktop 入站前缀)、patch/restore、WSL 直连同写。
- `runtime/providers.rs`、`runtime/routes.rs`(desktop 前缀匹配)、`runtime/upstream.rs`(探针路径;协议 AnthropicMessages 复用 transformer)。
- `usage_stats.rs`:`cli_key_from_app_type`(加 `claude_desktop`)、`load_provider_names`(加 `claude_desktop_provider`)。
- 前端 `proxyGatewayApi.ts`、`shared/gateway/providerProfiles.ts`、`gateway_provider_profiles.json`(加 `tools["claude_desktop"]`)、`GatewaySettingsPanel.tsx`(CLI_OPTIONS)。
- 说明:Direct 模式是核心、立即可用;Proxy/Single/Failover 接入网关是增强层,按上述清单补齐。网关 token 复用项目固定哨兵 `ai-toolbox-gateway`。

### 2.4 MCP 与 Skills

- **Claude Desktop**:`claude_desktop_config.json` 的 `mcpServers` 是官方支持的 MCP 位点。通过 `tools/builtin.rs` 注册 `BuiltinTool{ key:"claude_desktop", mcp_config_path: <normal config path>, mcp_config_format:"json", mcp_field:"mcpServers" }`,让全局 MCP 面板支持"从 claude_desktop 导入";写 3P profile 时 `mcpServers` 由 `read_json_or_empty` 保留。
- **Hermes**:`mcp_servers:` 段,注册 `BuiltinTool{ key:"hermes", mcp_config_path:<config.yaml>, mcp_config_format:"yaml" }`;`config_sync`/`mcp_sync` 对接。
- **Skills**:Claude Desktop 无 skills(否)。Hermes 按 cc-switch 未管理 skills,首版不做;若后续确认 Hermes 有 skills 目录再补 `BuiltinTool.relative_skills_dir` 与 `resync_all_skills_*`。

---

## 三、前端新增模块

### 3.0 共享注册点(两模块)

- `web/features/coding/index.ts`:两模块 `export * from './...'`
- `web/app/routeConfig.ts`:主路由 + 会话详情路由(ownerTabKey/PARENT_PATH)
- `web/constants/modules.tsx`:`MODULES[0].subTabs` 加 `claudedesktop`、`hermes`
- `web/components/layout/MainLayout/index.tsx`:`TAB_ICONS`(desktop 复用 `claude.svg`)& hermes 用 lobehub `HermesAgent` 分支
- `web/stores/settingsStore.ts` + `web/services/settingsApi.ts`:`visibleTabs` / `SIDEBAR_PAGE_KEYS` 加 `claudedesktop`、`hermes`
- `web/features/coding/shared/sessionManager/*`:`SessionTool` union + 手动映射
- `web/features/settings`:WSLSyncModal / SSHSyncModal 的 `ALL_MODULE_KEYS` 与 label map、FileMapping / SSHFileMapping Select 项
- i18n(`web/i18n/locales/zh-CN.json` + `en-US.json`):`subModules.claudedesktop` / `subModules.hermes` + 各自特性文案块
- `web/features/settings/pages/GeneralSettingsPage.tsx`:CODING_TABS 等(如有)

### 3.1 Claude Desktop 前端 `web/features/coding/claudedesktop/`

**以 claudecode 页面为模板**,复用其样式(ProviderCard、SectionSidebarLayout 等);形态精简:
- Provider 卡片(应用/编辑/复制/删除/连通性测试)+ **Direct/Proxy 模式**表单。
- 会话管理 `SessionManagerPanel tool="claudedesktop"`。
- 无插件、无 prompt(Claude Desktop 无 CLAUDE.md)。可保留"通用配置/导入(cc-switch / All API Hub)"。
- 类型 `web/types/claudedesktop.ts`、服务 `web/services/claudeDesktopApi.ts`(+ 可选 prompt api)。
- 图标:复用 `web/assets/claude.svg`(`TAB_ICONS.claudedesktop = ClaudeIcon`)。

### 3.2 Hermes 前端 `web/features/coding/hermes/`

**以 pi 页面为模板**(SectionSidebarLayout + ProviderCard + ModelFormModal 等共享组件):
- Sections:Provider 管理(`custom_providers`)、默认模型(顶层 `model:`)、MCP(`mcp_servers:`)、通用设置(agent 等)、全局 prompt、会话管理。
- 类型 `web/types/hermes.ts`、服务 `web/services/hermesApi.ts`(+ prompt api)、utils(`hermesFetchedModels` 类似)。
- 图标:`import { HermesAgent } from '@lobehub/icons'`,MainLayout 加 `tab.key==='hermes' ? <HermesAgent size={16} .../> : ...`(需先升级 `@lobehub/icons` 到 5.x)。

---

## 四、改名 Claude → Claude Code

内部 sync key `claude`(WSL/SSH/gateway/tray)保持一致,只改**用户可见文案**:

- i18n(en/zh):`subModules.claudecode`: `"Claude"` → `"Claude Code"`;`claudecode.claudeCodeSettings`: `"Claude"` → `"Claude Code"`。
- `web/features/settings/components/WSLSyncModal.tsx:28`、`SSHSyncModal.tsx:38`:`claude: 'Claude'` → `'Claude Code'`。
- `web/features/settings/components/FileMappingModal.tsx:243`、`SSHFileMappingModal.tsx:145`:`<Select.Option value="claude">Claude</Select.Option>` → `"Claude Code"`。
- `web/features/coding/shared/sessionManager/detail/SessionDetailWorkbench.tsx:457`(`getAssistantLabel('claudecode')` → `'Claude Code'`)。
- 检查托盘 section 文案(已是 `Claude Code`),无需改。

---

## 五、跨模块:WSL / SSH / 备份恢复

### WSL/SSH 同步
- 事件:`lib.rs` 加 `wsl-sync-request-hermes` 监听(仿 `wsl-sync-request-omp`,调 `wsl_sync(..., Some("hermes"), None)`);desktop 若无 WSL 目标则不发或映射到同路径(config-file 模块可直接映射配置文件)。
- `wsl/commands.rs`、`ssh/commands.rs`:两模块的默认文件映射:
  - desktop:`claude_desktop_config.json`(+configLibrary)映射到远端同名路径(或按模块语义决定)。
  - hermes:`hermes-config`(config.yaml)。
- 前端 `ALL_MODULE_KEYS`(useWSLSync.ts / useSSHSync.ts)/ `ALL_MODULE_KEYS`(mapping 弹窗)/ `moduleStatuses` 字段补充。

### 备份恢复
- `tauri/src/settings/backup/utils.rs`:加两模块路径解析器:
  - `get_claude_desktop_settings_path*`(指向 `claude_desktop_config.json` + configLibrary)。
  - `get_hermes_config_path*`(指向 `config.yaml`)。
- 备份 zip / webdav / 自动备份链路随现有枚举自动覆盖(需确认枚举是否 per-tool 白名单,若是则加入)。

---

## 六、验证

- 后端:`cargo check` / `cargo clippy` / `cargo test`(runtime_location、desktop profile 写入+回滚、hermes section 替换,prompt 覆盖)。
- 前端:`pnpm tsc --noEmit`;`pnpm test:web -- <相关用例>`。
- 依赖:`pnpm add -D @lobehub/icons@^5`(HermesAgent)后确认无破坏。
- 组件自查(DESIGN.md):亮/暗色、空态、长文本、加载态、导航选中态。
- 托盘:切换 provider 后刷新;WSL/SSH/备份按钮生效。

---

## 七、实施顺序(增量)

1. **改名 Claude → Claude Code**(独立、低风险)
2. **共享基础**:DB schema+migrations、mod.rs、lib.rs invoke_handler 骨架、runtime_location 键
3. **Hermes 后端**(pi 模板,较小)
4. **Claude Desktop 后端核心**:路径/3P profile/Direct 模式/导入/回滚/托盘
5. **Claude Desktop 网关集成**:GatewayCliKey::ClaudeDesktop + provider_switch + cli_proxy + runtime + usage_stats + 前端网关
6. **前端 hermes**(pi 模板)+ 图标升级 + 图标
7. **前端 claudedesktop**(claudecode 模板)+ 图标
8. **跨模块**:WSL/SSH 默认映射 + 备份恢复 + MCP 导入/同步 + 托盘 + 会话管理 + i18n + 设置页
9. **构建与验证**

> 备注:Hermes 是否需要 skills、Claude Desktop 是否纳入 gateway 单/故障切换 manifest,是计划中标注的「视确认/可选」项;核心(Desktop Direct 3P + Hermes provider/model)始终优先。

---

## 八、实施状态(2026-08-14)

### 已完成
- **改名 Claude → Claude Code**:i18n(两语言 `subModules.claudecode`/`claudeCodeSettings`)、WSLSync/SSHSync 标签、FileMapping/SSHFileMapping Select、SessionDetailWorkbench assistant 标签。
- **共享基础**:`DbTable::ClaudeDesktopProvider/HermesSettingsConfig/HermesPromptConfig` + `migrate_v10`(schema 版本 9→10);`coding/mod.rs` 声明;`lib.rs` invoke_handler 注册两模块全部命令。
- **后端 claude_desktop**(`tauri/src/coding/claude_desktop/`):3P profile 写入(deploymentMode+configLibrary+_meta)、Direct 模式、`is_claude_safe_model_id`、快照回滚、恢复官方、import/官方 seed;单测 7 pass。
- **后端 hermes**(`tauri/src/coding/hermes/`):config.yaml(custom_providers+顶层 model+other)、HERMES_HOME、prompt CRUD、tray;单测 6 pass。
- **前端 claudedesktop**(复用 claude code 样式)与 **hermes**(复用 pi 样式)页面 + services + types。
- **图标**:`@lobehub/icons` 升级 4.9.0→5.16.0(引入 `HermesAgent`);claudedesktop 复用 `claude.svg`,hermes 用 `HermesAgent`,MainLayout 已接入。
- **跨模块**:托盘(tray.rs)、备份恢复(utils/local/webdav)、WSL/SSH 默认映射+动态路径+MCP 同步、session_manager 最小注册、reapply、tools/builtin+detection、前端 settingsStore/settingsApi/sync modals/BackupSettingsModal/SessionTool/sessionDetailNavigation。
- **构建**:`cargo check` 0 error;`pnpm tsc --noEmit` 0 error;两后端模块单测全绿。

### 已知遗留/最小占位(按计划中的"视确认/可选"项)
1. **claude_desktop Proxy 模式接入本项目 `proxy_gateway`(Single/Failover)**:Direct 模式已完整可用;Proxy 分支目前返回"未接入网关"。这是计划 2.3 的增强层,尚未做(需 `GatewayCliKey::ClaudeDesktop` + provider_switch/cli_proxy/runtime/usage_stats + 前端网关 + `gateway_provider_profiles.json` A-J 清单)。
2. **Hermes MCP 单 server 写盘**:`mcp/config_sync.rs` 只支持 json/jsonc/toml,`yaml` 格式写单个 server 会返回错误(不损坏 config.yaml);整文件经 WSL/SSH 按 `hermes-config` 原样同步正常。
3. **Hermes Skills**:按确认不纳入首版(cc-switch 未管理)。
4. **i18n 特性文案**:hermes 页面用的 `hermes.*` 键(44 个)未写入 locales(用 defaultValue 兜底,运行不受影响),claudedesktop 复用既有键;后续可补全。

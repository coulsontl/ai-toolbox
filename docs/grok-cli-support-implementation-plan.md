# Grok CLI（Grok Build）支持实现计划

## 1. 文档目的

本文档用于指导 AI Toolbox 新增官方 Grok CLI（官方名称为 **Grok Build**，命令名为 `grok`）的完整支持。

目标不是只增加一个导航入口，而是形成与现有 Codex 页面同级的产品闭环：

- Grok provider、默认模型和通用配置管理；
- 官方登录态与自定义 API Key 渠道共存；
- 全局提示词、Plugins、Session Manager 和托盘切换；
- MCP、Skills 中央仓库、WSL、SSH 同步；
- 本地/WebDAV 备份恢复与文件过滤；
- SQLite JSONB 数据库表、迁移和索引；
- 可选的 Proxy Gateway single/failover 接管。

前端视觉和交互有一个明确约束：**以 Codex 页面为模板直接复制后改名、改字段和删去不成立的 Codex 专属能力，不重新设计页面。**

图标固定使用：

```tsx
import { Grok } from '@lobehub/icons';
```

主导航 Tab 使用 `<Grok size={16} />`，不新增本地 SVG，不自行绘制图标。

---

## 2. 研究基线

### 2.1 官方资料

本计划基于 2026-07-12 可访问的 xAI 官方资料：

- [Grok Build Overview](https://docs.x.ai/build/overview)
- [Settings](https://docs.x.ai/build/settings)
- [CLI Reference](https://docs.x.ai/build/cli/reference)
- [AGENTS.md / Project Rules](https://docs.x.ai/build/features/project-rules)
- [Sessions](https://docs.x.ai/build/features/sessions)
- [MCP Servers](https://docs.x.ai/build/features/mcp-servers)
- [Skills, Plugins & Marketplaces](https://docs.x.ai/build/features/skills-plugins-marketplaces)
- [Hooks](https://docs.x.ai/build/features/hooks)
- [Enterprise Deployments](https://docs.x.ai/build/enterprise)
- [xAI API Overview](https://docs.x.ai/build/overview)

本机还使用隔离的 `HOME` / `GROK_HOME` 对官方 CLI 做了只读行为核对：

```text
grok 0.2.93 (f00f96316d)
```

核对过的命令包括：

```bash
grok --help
grok version
grok inspect --help
grok inspect --json
grok sessions --help
grok mcp --help
grok plugin --help
```

### 2.2 仓库研究范围

实现时必须把以下现有模块作为主要参照：

- `web/features/coding/codex/`
- `tauri/src/coding/codex/`
- `tauri/src/coding/pi/`
- `web/features/coding/shared/`
- `tauri/src/coding/runtime_location.rs`
- `tauri/src/coding/tools/`
- `tauri/src/coding/mcp/`
- `tauri/src/coding/skills/`
- `tauri/src/coding/session_manager/`
- `tauri/src/coding/wsl/`
- `tauri/src/coding/ssh/`
- `tauri/src/settings/backup/`
- `tauri/src/coding/proxy_gateway/`

其中：

- Codex 是前端页面结构、provider CRUD、common config、prompt、tray 和 Gateway 交互的主要模板；
- Pi 是“新工具接入 runtime location、MCP、Skills、WSL/SSH、备份和 Session Manager”的近期参考；
- Grok 的配置格式虽然也是 TOML，但 provider 模型与认证语义不能照搬 Codex。

---

## 3. 官方 Grok Build 行为事实

### 3.1 安装与命令

官方安装方式：

```bash
curl -fsSL https://x.ai/cli/install.sh | bash
```

Windows PowerShell：

```powershell
irm https://x.ai/cli/install.ps1 | iex
```

也支持 npm 安装：

```bash
npm install -g @xai-official/grok
```

主命令为：

```bash
grok
```

AI Toolbox 未来需要调用 Grok CLI 时，必须复用 `cli_resolver.rs`，不能假设 GUI 进程的 `PATH` 一定包含 `grok`。尤其 macOS 从 Dock/Finder 启动时，应先解析官方安装位置和 npm shim，再回退到 `PATH`。

### 3.2 Runtime 根目录

默认根目录：

```text
~/.grok
```

Windows：

```text
%USERPROFILE%\.grok
```

环境变量覆盖：

```text
GROK_HOME
```

AI Toolbox 应按现有根目录模块语义实现以下优先级：

```text
应用内 root_dir > GROK_HOME > shell 配置中的 GROK_HOME > ~/.grok
```

Grok 应加入 `runtime_location` 的根目录模块集合，和 Claude Code、Codex、Gemini CLI、Pi 一样支持本机路径与 WSL Direct UNC 路径。

### 3.3 主要运行时文件

| 路径 | 官方语义 | AI Toolbox 首版处理 |
|---|---|---|
| `<root>/config.toml` | 用户主配置、models、MCP、plugins、permissions 等 | 管理受控字段并保留未知字段 |
| `<root>/auth.json` | OAuth/OIDC/外部认证缓存 | 只识别、备份、同步和保留，不整文件覆盖 |
| `<root>/AGENTS.md` | 用户级全局规则之一 | 作为 AI Toolbox 全局提示词目标文件 |
| `<root>/sessions/` | 会话目录 | 接入 Session Manager，不默认进入普通文件同步 |
| `<root>/skills/` | 用户级 Skills | 接入中央 Skills 仓库同步 |
| `<root>/plugins/` | 用户级 Plugins、marketplaces | 接入 Grok Plugins UI、WSL/SSH 和备份 |
| `<root>/mcp_credentials.json` | MCP OAuth 凭据 | 默认不跨设备同步、不默认备份 |
| `<root>/trusted_folders.toml` | 项目 hooks/MCP/LSP 信任记录 | 首版不管理，不跨机器复制 |
| `<root>/memory/` | 跨会话 memory | 首版不管理 |
| `<root>/agents/` | 用户级 agent definitions | 首版不管理 |
| `<root>/hooks/` | 用户级 hooks | 首版不管理 |
| `<root>/lsp.json` | 用户级 LSP 配置 | 首版不管理 |
| `<root>/pager.toml` | TUI 外观 | 首版不管理 |

### 3.4 配置分层

官方配置优先级从低到高为：

1. `/etc/grok/managed_config.toml`
2. `~/.grok/managed_config.toml`
3. `~/.grok/config.toml`
4. `~/.grok/requirements.toml`
5. `/etc/grok/requirements.toml`

AI Toolbox 首版只管理用户级 `<root>/config.toml`，不编辑 managed/requirements 文件，也不假装可以覆盖更高优先级的企业策略。

当 `grok inspect --json` 显示更高层配置正在覆盖用户配置时，页面可以显示诊断提示，但不能尝试修改企业文件。

### 3.5 认证语义

Grok 支持：

- 浏览器 OIDC：`grok login`
- Device Code：`grok login --device-auth`
- 外部认证命令：`[auth].auth_provider_command`
- API Key：`XAI_API_KEY`、`[model.*].api_key` 或 `[model.*].env_key`

每个模型的凭据优先级：

1. `[model.*].api_key`
2. `[model.*].env_key` 指向且实际存在的环境变量
3. `auth.json` 中的当前 session token
4. `XAI_API_KEY`

这决定了一个重要实现差异：

- Codex 切换第三方 provider 时可能需要处理 `auth.json` 与官方 OAuth 的冲突；
- Grok 的自定义渠道 API Key 可以直接绑定到 `[model.*]`，因此 **切换自定义渠道不应改写或清除 `auth.json`**；
- Grok 不需要复制 Codex 的“切换第三方时保留官方登录”设置，保留官方认证应是固定行为。

### 3.6 自定义模型

官方配置示例：

```toml
[models]
default = "my-model"

[model.my-model]
model = "model-id"
base_url = "https://api.example.com/v1"
name = "Display Name"
env_key = "API_KEY"
api_backend = "responses"
context_window = 200000
```

支持三种 `api_backend`：

| 值 | 协议 |
|---|---|
| `chat_completions` | OpenAI Chat Completions |
| `responses` | OpenAI Responses |
| `messages` | Anthropic Messages |

Grok 原生支持这三种协议，因此普通自定义渠道不需要像 Codex 那样仅为了协议转换强制走 Gateway。

### 3.7 MCP

Grok 原生 MCP 配置位于：

```toml
[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
env = { API_KEY = "${MY_API_KEY}" }
enabled = true
startup_timeout_sec = 30
tool_timeout_sec = 6000
```

远程 MCP 使用 `url` 和 `headers`。

Grok 官方在 Windows 会自行解析 `npx`、`npm`、`pnpm`、`yarn` 的 `.cmd` shim，因此同步到 Grok 的 MCP 配置 **不要额外改写成 `cmd /c`**。这与当前 Codex Windows 目标行为不同，不能因为两者都使用 TOML 就完全共用 wrapping 判断。

### 3.8 Skills 与 Plugins

用户级 Skills：

```text
<root>/skills/<skill>/SKILL.md
```

Grok 还会扫描 `.grok/skills/`、Claude Code Skills 和 `.agents/skills/`。AI Toolbox 的中央 Skills 同步目标只负责 `<root>/skills/`，不接管项目目录和兼容目录。

用户级 Plugins：

```text
<root>/plugins/
```

官方 CLI 提供：

```bash
grok plugin list --json
grok plugin install <source>
grok plugin enable <name>
grok plugin disable <name>
grok plugin uninstall <name>
grok plugin marketplace list --json
```

插件的事实源是 Grok runtime 与 CLI 输出，不应新增第二套 `grok_plugin` 数据库主表。

### 3.9 全局规则

Grok 会加载 `<root>` 下的全局规则，并在项目树中读取 `AGENTS.md`、`CLAUDE.md` 等文件。

AI Toolbox 的“全局提示词”首版只管理：

```text
<root>/AGENTS.md
```

不能删除或覆盖同目录下的其他 Markdown 规则文件，也不能把所有规则合并后写回 `AGENTS.md`。

### 3.10 Sessions

官方会话目录：

```text
<root>/sessions/<encoded-cwd>/<session-id>/
```

主要文件：

```text
summary.json
updates.jsonl
chat_history.jsonl
plan.json
rewind_points.jsonl
signals.json
feedback.jsonl
subagents/
```

`summary.json` 是索引元数据，`updates.jsonl` 是会话恢复和展示的主要事件流。

Grok 不能套用 Codex 的单个 `rollout-*.jsonl` parser，必须新增独立 `session_manager/grok.rs`。

---

## 4. 产品范围

### 4.1 完整目标范围

最终交付应包含：

1. 顶部 Grok Tab、路由、可见性设置和二级 Session Detail 路由；
2. Grok root path、配置目录打开、刷新、预览；
3. provider 新增、编辑、复制、删除、禁用、排序、应用；
4. 官方登录 provider 与自定义模型 provider；
5. 通用配置编辑；
6. 全局提示词 presets；
7. Grok Plugins 管理；
8. Session Manager；
9. 托盘 provider/model 和 prompt 切换；
10. MCP 中央存储同步到 Grok；
11. Skills 中央仓库同步到 Grok；
12. WSL Direct、Windows→WSL、SSH 同步；
13. 本地/WebDAV 备份恢复与文件过滤；
14. Gateway single/failover 接管；
15. i18n、测试和模块 AGENTS 文档。

### 4.2 首版明确不做

- 不管理 `/etc/grok/*`、`managed_config.toml`、`requirements.toml`；
- 不伪造 Grok 套餐、quota、余额或 plan type；官方账号只管理经过真实 Device Code/OIDC 获得的认证快照；
- 不管理 project `.grok/config.toml`；
- 不提供 hooks、agents、LSP、memory、pager、sandbox profile 的专用 UI，但读写 `config.toml` 时必须保留相关合法配置；
- 不同步 `mcp_credentials.json` 和 `trusted_folders.toml`；
- 不默认备份 sessions 和 memory；
- 不自行设计新的前端信息架构。

---

## 5. 前端复用原则

### 5.1 复制而不是重做

第一轮前端实现直接从 `web/features/coding/codex/` 复制到 `web/features/coding/grok/`，再做语义替换。

建议映射：

| Codex 文件 | Grok 文件 | 处理 |
|---|---|---|
| `pages/CodexPage.tsx` | `pages/GrokPage.tsx` | 复制后改 service/types/key；删除 Codex quota/history 专属逻辑 |
| `CodexProviderCard.tsx` | `GrokProviderCard.tsx` | 复制样式与布局以及官方账号区；删除 Codex quota/plan/token copy，替换为 Grok Device Code 账号操作 |
| `CodexProviderFormModal.tsx` | `GrokProviderFormModal.tsx` | 复制表单结构；字段改成 Grok model/api_backend |
| `CodexCommonConfigModal.tsx` | `GrokCommonConfigModal.tsx` | 复制 Modal；后端保护段改为 Grok ownership |
| `CodexPluginsPanel.tsx` | `GrokPluginsPanel.tsx` | 复制 UI；service 改调 Grok CLI |
| `ImportConflictDialog.tsx` | `ImportConflictDialog.tsx` | 可直接复制或提取共享 |
| `ImportFromAllApiHubModal.tsx` | `ImportFromAllApiHubModal.tsx` | 复制转换逻辑并映射 Grok backend |
| Codex `.module.less` | Grok 同名 `.module.less` | 原样复制，除命名外不重新设计 |

首轮不要为了减少重复代码把 Codex 页面大规模抽象成泛型页面。优先完成一份语义正确的 Grok 副本；只有已经存在的 shared 组件继续直接复用。

### 5.2 必须继续复用的 shared 能力

- `SectionSidebarLayout`
- `RootDirectoryModal`
- `useRootDirectoryConfig`
- `GlobalPromptSettings`
- `SessionManagerPanel`
- `ProviderConnectivityTestModal`
- favorite providers helpers
- All API Hub 公共能力
- `SidebarSettingsModal`
- `ImportProviderModal`
- `GatewayFailoverButton`（Gateway Phase）
- provider billing shared UI（Gateway Phase）

### 5.3 页面结构

Grok 页面保持 Codex 的四段结构：

1. Providers
2. Global Prompt
3. Plugins
4. Sessions

页面 Header、根目录行、Collapse、provider 卡片密度、按钮位置、空态、拖拽排序、侧边 section 导航都照搬 Codex。

### 5.4 图标接入

`web/components/layout/MainLayout/index.tsx`：

```tsx
import { Gemini, Grok, OpenClaw as OpenClawIcon } from '@lobehub/icons';
```

Tab 渲染增加：

```tsx
tab.key === 'grok' ? (
  <Grok size={16} className={styles.tabIconColor} />
) : ...
```

不要加入 `TAB_ICONS` 的 SVG 字符串表。

---

## 6. 数据库设计

### 6.1 新增表

新增四个 SQLite JSONB 表：

```rust
DbTable::GrokProvider
DbTable::GrokOfficialAccount
DbTable::GrokCommonConfig
DbTable::GrokPromptConfig
```

物理表名：

```text
grok_provider
grok_official_account
grok_common_config
grok_prompt_config
```

不新增：

```text
grok_plugin
grok_session
```

原因：

- 官方 Device Code 登录产生的账号快照需要支持保存、切换、刷新和删除；数据库保存 AI Toolbox 的账号管理元数据与脱敏前不可展示的认证快照，runtime `auth.json` 仍是当前 Grok CLI 生效态；
- Plugins 事实源是 Grok CLI/runtime；
- Sessions 事实源是 `<root>/sessions/`。

### 6.2 Schema migration

在当前 `TARGET_SCHEMA_VERSION = 7` 基础上新增 v8：

```rust
fn migrate_v8(conn: &Connection) -> Result<(), String> {
    create_jsonb_table(conn, DbTable::GrokProvider)?;
    create_jsonb_table(conn, DbTable::GrokOfficialAccount)?;
    create_jsonb_table(conn, DbTable::GrokCommonConfig)?;
    create_jsonb_table(conn, DbTable::GrokPromptConfig)?;

    create_json_index(conn, DbTable::GrokProvider, &JsonFieldPath::new("is_applied")?)?;
    create_json_index(conn, DbTable::GrokProvider, &JsonFieldPath::new("sort_index")?)?;
    create_json_index(conn, DbTable::GrokOfficialAccount, &JsonFieldPath::new("provider_id")?)?;
    create_json_index(conn, DbTable::GrokOfficialAccount, &JsonFieldPath::new("is_applied")?)?;
    create_json_index(conn, DbTable::GrokOfficialAccount, &JsonFieldPath::new("sort_index")?)?;
    create_json_index(conn, DbTable::GrokPromptConfig, &JsonFieldPath::new("is_applied")?)?;
    create_json_index(conn, DbTable::GrokPromptConfig, &JsonFieldPath::new("sort_index")?)
}
```

同时更新 `DbTable`、`ALL_TABLES`、`name()` 和 migration tests。

### 6.3 Grok provider JSONB payload

建议沿用 Codex provider 的外层字段，降低前端复制成本：

```json
{
  "name": "xAI API",
  "category": "custom",
  "settings_config": "{...JSON string...}",
  "meta": {
    "apiFormat": "openai_responses",
    "gatewayProfile": null,
    "costMultiplier": null,
    "pricingModelSource": null
  },
  "notes": null,
  "source_provider_id": null,
  "is_applied": true,
  "is_disabled": false,
  "sort_index": 0
}
```

`settings_config` 内部结构：

```json
{
  "config": "# provider scoped advanced TOML only",
  "auth": {
    "API_KEY": "xai-..."
  },
  "defaultModelKey": "grok-4.5",
  "modelCatalog": {
    "models": [
      {
        "key": "grok-4.5",
        "model": "grok-4.5",
        "displayName": "Grok 4.5",
        "description": null,
        "baseUrl": "https://api.x.ai/v1",
        "apiBackend": "responses",
        "apiKey": null,
        "envKey": "XAI_API_KEY",
        "contextWindow": 1000000,
        "maxCompletionTokens": 8192,
        "temperature": 0.7,
        "topP": 0.95,
        "supportsBackendSearch": true,
        "supportsReasoningEffort": true,
        "reasoningEffort": "high",
        "streamToolCalls": true,
        "maxRetries": 3,
        "inferenceIdleTimeoutSecs": 600,
        "extraHeaders": {},
        "extraConfig": {},
        "modalities": {
          "input": ["text", "image"],
          "output": ["text"]
        }
      }
    ]
  }
}
```

规则：

- `defaultModelKey` 对应 `[models].default`；
- `modelCatalog.models[*].key` 对应 `[model."<key>"]` 的 TOML table key；
- `model` 是发给 API 的真实模型 ID；
- API Key 可以由安全字段投影为 `api_key`，也可以通过 `envKey` 投影为 `env_key`；UI、日志和预览均不得回显明文；
- `extraConfig` 保存当前 UI 未识别、但官方 schema 合法的模型级字段，避免结构化编辑后丢失；
- 显式 boolean 的 `false`、空对象、`extra_headers`、sampling、retry、timeout、reasoning 和 modalities 必须无损保留；
- provider 高级 TOML 可以包含受管 `[model.*]` 的高级字段，但必须解析并归并到对应 model entry/`extraConfig`，不能以“高级 TOML 不管理 model”为由丢弃字段；
- `[mcp_servers]`、`[plugins]`、marketplace 和用户其他非受管配置不属于 Provider。

### 6.4 Grok official account payload

`grok_official_account` 每条记录至少包含：

```json
{
  "provider_id": "official-provider-id",
  "name": "user@example.com",
  "kind": "oauth",
  "email": "user@example.com",
  "subject": "xai-sub",
  "auth_snapshot": "{...encrypted-or-existing-secret-storage-compatible-json...}",
  "token_endpoint": "https://auth.x.ai/...",
  "expires_at": "2026-07-12T00:00:00Z",
  "last_refresh": "2026-07-12T00:00:00Z",
  "last_error": null,
  "is_applied": true,
  "sort_index": 0,
  "created_at": "2026-07-12T00:00:00Z",
  "updated_at": "2026-07-12T00:00:00Z"
}
```

约束：

- `provider_id` 关联 official provider；删除 provider 前必须处理关联账号；
- 同一 provider 最多一个 `is_applied = true`，应用新账号时在同一事务中清除旧值；
- `email` 来自 OIDC userinfo，`subject` 来自官方 entry 的 `user_id`/`principal_id`；缺少 email 时以 subject 或稳定的本地别名展示；
- `auth_snapshot` 不通过普通列表命令返回前端，序列化、日志、错误上下文和备份预览必须掩码；
- 不增加未经官方接口确认的 quota、余额、plan、5h/weekly/monthly 字段。

### 6.5 Grok common config payload

固定 id：

```text
common
```

建议 payload：

```json
{
  "root_dir": null,
  "config_toml": "[ui]\npermission_mode = \"ask\"\n",
  "updated_at": "2026-07-12T00:00:00Z"
}
```

`root_dir` 与 TOML 分离，继续复用 `RootDirectoryModal`。

### 6.6 Prompt config payload

沿用现有 prompt schema：

```json
{
  "name": "default",
  "content": "...",
  "is_applied": true,
  "sort_index": 0
}
```

运行时目标固定为 `<root>/AGENTS.md`。

### 6.7 Settings payload

需要扩展：

- `visible_tabs` 默认加入 `grok`；
- `sidebar_hidden_by_page` 加入 `grok: false`；
- 不新增 `grok_preserve_official_auth_on_switch`；
- 如 Gateway 支持 Grok，`ProxyGatewaySettings.app_configs` 增加 `grok`。

`visible_tabs` 顺序建议：

```text
opencode, claudecode, codex, grok, geminicli, openclaw, pi, gateway, image, ssh, wsl
```

升级已有用户时必须保留自定义顺序，只对旧默认序列做一次兼容插入，不能重排用户自己拖动后的 Tab。

---

## 7. config.toml 所有权、官方认证与投影

### 7.1 所有权矩阵

| 配置段 | 所有者 | 保存行为 |
|---|---|---|
| `[models].default` | Provider | applied provider 管理 |
| `[models]` 其他字段 | Common Config | 字段级提取与合并，不能剥离整个 table |
| 当前/前一 Provider 受管 `[model.*]` | Provider | 无损管理已识别字段和 `extraConfig` |
| 用户其他 `[model.*]` | runtime/user | 始终保留 |
| `[mcp_servers]` | MCP | Provider/Common/Plugin 写入必须保留 |
| `[plugins]`、marketplace | Plugins | 其他 writer 必须保留 |
| `[skills]` | Common Config 或 Skills | Phase 0 按官方 fixture 确认字段级 owner；路径投影归 Skills，其余归 Common |
| `[auth]`、`[grok_com_config]` | runtime/企业认证 | 保留 |
| `[ui]`、`[features]`、`[session]`、`[tools]`、`[toolset]`、`[cli]`、`[hints]` | Common Config | 可在高级 TOML 中管理 |
| `[sandbox]`、`[permission]`、`[subagents]`、`[memory]`、`[compat.*]` | Common Config | 没有专用 UI 也必须无损保留 |
| unknown sections | runtime/user | 默认保留 |

Common Config Modal 直接复制 Codex 的 ignored/protected fields Alert，并替换为上述 Grok ownership。后端返回实际忽略/保护字段，不能只显示静态前端文案。

### 7.2 结构化 TOML merge

所有 writer 使用 `toml_edit::DocumentMut`，禁止字符串拼接或整文件模板覆盖。Provider apply 顺序：读取 live 文件、定位前一 Provider 的 managed model keys、仅移除仍匹配上一轮投影的受管 table、字段级合入 Common Config、生成新 Provider model tables、更新 `[models].default`、原子写入、事务更新 `is_applied`、发送 `config-changed` 和 `wsl-sync-request-grok`。

如果用户已经手工修改前一 Provider 的受管 table，保留该 table 并返回 warning。必须用真实 fixture 覆盖：

```text
read → edit → save → apply → read
```

往返后不得丢失 `env_key`、值为 `false` 的 boolean、unknown fields、`extra_headers`、sampling、retry、timeout、reasoning、空对象和合法 table 顺序语义。

### 7.3 官方 Provider 与账号 UI

官方 Provider 使用 `category = "official"`，默认模型 `grok-build`，不保存 API Key，不进入 Gateway 上游候选。应用官方 Provider 时移除前一个自定义 Provider 的精确受管 model tables并设置默认模型；只有“应用某个官方账号”才会字段级更新 runtime `auth.json`。

前端完整复制 Codex Provider Card 的官方账号区域，保留：官方账号标题、账号数量、登录、邮箱、OAuth 标签、当前应用标签、刷新、查看详情、应用、保存本地账号、删除。删除 Codex 专属的 plan type、5h/weekly/monthly quota、reset 和 token copy。

### 7.4 `__local__` 语义

数据库无 Provider 而 live 配置已有模型时生成临时 `__local__`：从默认 model key 读取完整 model table，所有未知字段进入 `extraConfig`，不读取 token 到前端。用户保存后正式入库并保持 applied。live 配置为官方模型且 `auth.json` 有效则可创建持久化 official provider，不显示为不可管理的自定义卡片。

### 7.5 auth.json 真实 fixture 与 writer

不能把 CLIProxyAPI 自己的扁平 token 文件格式复制到 Grok runtime。Phase 0 必须在隔离 `GROK_HOME` 中执行官方 `grok login --device-auth`，只提取字段名、嵌套和类型并将 token 脱敏；保存登录后、refresh 后和 logout 后 fixture；验证 AI Toolbox 写回后官方 CLI 可识别；验证 Unix 权限。

2026-07-13 的真实隔离授权确认，官方文件不是扁平 token 对象，而是以 OIDC scope 为顶层 key：

```json
{
  "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828": {
    "key": "*** access token ***",
    "auth_mode": "oidc",
    "create_time": "RFC3339",
    "user_id": "...",
    "email": "...",
    "first_name": "...",
    "profile_image_asset_id": "...",
    "principal_type": "User",
    "principal_id": "...",
    "team_id": "...",
    "coding_data_retention_opt_out": false,
    "refresh_token": "***",
    "expires_at": "RFC3339",
    "oidc_issuer": "https://auth.x.ai",
    "oidc_client_id": "b1a00492-073a-47ea-816f-4c329264a828"
  }
}
```

`key` 才是 access token；不得写旧方案中的根级 `access_token`、`id_token`、`type`、`auth_kind`、`expired` 或 `token_endpoint`。身份 enrichment 来自 access-token claims 与 OIDC userinfo。`coding_data_retention_opt_out`、`team_name`、`is_zdr`、`team_role`、`subscription_tier` 等字段可能由官方 CLI 后续 enrichment 写入，AI Toolbox refresh/apply 必须保留。

Writer 只替换 fixture 已确认的 OAuth managed fields，保留 unknown/runtime-owned fields，使用同目录临时文件加 rename 原子写入，Unix 设置 `0600`。保存账号时只保存目标 scope；应用时保留其他 scope，同一 principal 才合并当前 CLI enrichment，切换 principal 时替换旧 entry。删除/退出只移除 xAI scope，最后一个 scope 删除后才删除文件。token 不得进入日志、事件、前端 payload、普通错误文本或未加密诊断文件。

### 7.6 Grok 官方账号 Device Code

实现参考 `/mnt/d/GitHub/cli-proxy-api` 只读 checkout（研究时 HEAD `9418054a3b2184cc6fa618f1bbef51ffca17c32d`）中的 `internal/auth/xai/`、`sdk/auth/xai.go`、`internal/cmd/xai_login.go` 和 `internal/runtime/executor/xai_executor.go`。

协议基线：

```text
Issuer: https://auth.x.ai
Discovery: https://auth.x.ai/.well-known/openid-configuration
Client ID: b1a00492-073a-47ea-816f-4c329264a828
Scope: openid profile email offline_access grok-cli:access api:access conversations:read conversations:write
Grant type: urn:ietf:params:oauth:grant-type:device_code
```

完整流程：OIDC discovery；校验 issuer/device/token/userinfo endpoint 必须为 HTTPS 且 host 为 `x.ai` 或 `*.x.ai`；请求 device code；后端保存 device code；向前端返回 verification URI/user code/过期时间/轮询间隔；打开浏览器；后端轮询 token endpoint；正确处理 `authorization_pending`、`slow_down`、`expired_token`、`access_denied`；支持取消；获得 access/refresh token；从 access-token claims 读取 principal/team/client，从 userinfo 读取 sub/email/given_name/picture；生成官方 scope-map snapshot；应用到 runtime `auth.json`。

启动结果只包含：

```json
{
  "sessionId": "opaque-id",
  "verificationUri": "https://...",
  "verificationUriComplete": "https://...",
  "userCode": "ABCD-EFGH",
  "expiresAt": 0,
  "pollIntervalSeconds": 5
}
```

事件 `grok-auth-status` 至少包含 `starting`、`waiting_for_user`、`authorized`、`saving`、`completed`、`denied`、`expired`、`cancelled`、`failed`。登录 Modal 复制 Codex/现有 Modal，只展示 URI、user code、复制、打开浏览器、倒计时、waiting 和 cancel；前端永远不能获得 device code 或 token。

---

## 8. 后端模块设计

新增：

```text
tauri/src/coding/grok/
  AGENTS.md
  mod.rs
  constants.rs
  types.rs
  adapter.rs
  commands.rs
  plugin_ops.rs
  tray_support.rs
```

### 8.1 constants.rs

至少定义：

```rust
pub const GROK_LOCAL_PROVIDER_ID: &str = "__local__";
pub const GROK_CONFIG_FILE: &str = "config.toml";
pub const GROK_AUTH_FILE: &str = "auth.json";
pub const GROK_PROMPT_FILE: &str = "AGENTS.md";
pub const GROK_SKILLS_DIR: &str = "skills";
pub const GROK_PLUGINS_DIR: &str = "plugins";
pub const GROK_SESSIONS_DIR: &str = "sessions";
```

### 8.2 主要 commands

路径与预览：

```text
get_grok_config_dir_path
get_grok_root_path_info
get_grok_config_file_path
reveal_grok_config_folder
read_grok_settings
```

provider：

```text
list_grok_providers
create_grok_provider
update_grok_provider
delete_grok_provider
reorder_grok_providers
select_grok_provider
toggle_grok_provider_disabled
save_grok_local_config
```

官方账号：

```text
start_grok_official_account_device_auth
cancel_grok_official_account_device_auth
get_grok_official_account_auth_status
list_grok_official_accounts
save_grok_official_local_account
apply_grok_official_account
refresh_grok_official_account
delete_grok_official_account
logout_grok_official_runtime
```

登录 session 只驻留内存，使用不可预测 id，完成/取消/过期后立即清除 device code；同一 runtime location 同时只允许一个有效登录 session。

common config：

```text
get_grok_common_config
extract_grok_common_config_from_current_file
save_grok_common_config
```

prompt：

```text
list_grok_prompt_configs
create_grok_prompt_config
update_grok_prompt_config
delete_grok_prompt_config
apply_grok_prompt_config
reorder_grok_prompt_configs
save_grok_local_prompt_config
```

Plugins：

```text
get_grok_plugin_runtime_status
list_grok_installed_plugins
list_grok_marketplaces
list_grok_marketplace_plugins
install_grok_plugin
enable_grok_plugin
disable_grok_plugin
uninstall_grok_plugin
set_grok_installed_plugins_enabled
update_grok_plugin
get_grok_plugin_details
validate_grok_plugin
add_grok_plugin_marketplace
remove_grok_plugin_marketplace
update_grok_plugin_marketplace
```

### 8.3 CLI 调用

Plugins、login、inspect 等命令：

- 本机走 `cli_resolver`；
- WSL Direct 在目标 distro 内调用 Linux `grok`；
- 动态 root、PATH、plugin source 必须作为独立参数传递；
- 不把用户输入插值进 shell command string；
- 优先消费 `--json` 输出；
- CLI 不存在时返回可展示的诊断，不让页面整体崩溃。

### 8.4 lib.rs 注册

需要：

- `pub mod grok;`
- 启动时 refresh Grok runtime location cache；
- 注册全部 Tauri commands；
- 监听 `wsl-sync-request-grok`；
- `config-changed` 继续触发 tray refresh 和 Gateway cache invalidation。

---

## 9. Provider 表单映射

保持 Codex 表单布局，字段替换为：

### 9.1 官方模式

- 渠道名称；
- 默认模型：`grok-build`，允许从 `grok models` 读取官方模型；
- 登录状态提示；
- 不显示 Base URL、API Key、api backend、模型映射和 billing。

### 9.2 自定义模式

- 渠道/内置供应商选择；
- 渠道名称；
- API Key；
- Base URL；
- API Backend：Chat Completions / Responses / Messages；
- 默认模型；
- 获取模型；
- 模型映射；
- 高级 TOML；
- Gateway billing meta；
- 备注。

### 9.3 模型映射复用

直接复用 Codex 的“模型映射”交互，不新增模型管理页面。

每一行映射到一个 `[model.<key>]`，默认 `key = model`。高级用户需要 alias 时再显示可选 key 字段，不为首版新增复杂别名编辑器。

### 9.4 获取模型

- 当前 `grok 0.2.93` 的 `grok models` 没有 `--json`，官方模式应调用 `grok models`，用版本化文本 parser 读取，并在输出格式变化或命令失败时回退内置官方模型列表；
- 自定义模式复用 `fetch_provider_models`；
- Base URL 缺失时不请求；
- 获取结果只辅助填表，不自动覆盖用户已选默认模型；
- API backend 决定连通性测试协议。

---

## 10. Plugins

Grok 官方原生支持 Plugins，页面保留 Codex 的 Plugins 区域。

实现原则：

- installed、discover、marketplace sources、local source、update、details、validate、启停和卸载都通过 Grok CLI；
- `config.toml` 的 `[plugins]` 和 marketplace 节点属于 runtime/plugin owner；
- provider/common apply 必须保留这些节点；
- 不新增 plugin 数据库表；
- 单个和批量启停只作用于 CLI 返回的已安装插件；批量操作返回逐项结果，部分失败不能伪装成整体成功；
- 安装前使用现有确认交互；用户确认后后台显式传 `--trust`，避免 CLI 等待不可见的交互输入；
- CLI 操作后无论成功或部分失败都 refresh 列表，成功变更时发 `config-changed`、`wsl-sync-request-grok`；
- 本地 plugin 路径和 marketplace 安装均复用 Codex Panel 的展示结构，不扩展新的 UI 模式。
- `[plugins].paths` 是否对应 Codex workspace roots 语义必须用官方 fixture/CLI 行为确认后再映射，不能仅因字段名相似直接复制；
- Grok 没有 Codex 的 plugins feature toggle，复制 UI 后删除该 Alert 和 toggle button；
- 所有 CLI 操作都使用当前 runtime location；WSL Direct 在 distro 中执行，不能拿 Windows 路径传给 Linux CLI。

---

## 11. MCP 接入

### 11.1 RuntimeTool

在 `tauri/src/coding/tools/builtin.rs` 新增：

```rust
BuiltinTool {
    key: "grok",
    display_name: "Grok",
    relative_skills_dir: Some("~/.grok/skills"),
    relative_detect_dir: Some("~/.grok"),
    mcp_config_path: Some("~/.grok/config.toml"),
    mcp_config_format: Some("toml"),
    mcp_field: Some("mcp_servers"),
}
```

动态路径必须经 `runtime_location::get_tool_mcp_config_path_*` 解析，不能固定使用 `~/.grok/config.toml`。

### 11.2 Grok 专用 TOML formatter/importer

MCP 中央数据库、页面、tool selection、事件和导入工作流继续复用，但不能直接复用 Codex 的 `build_toml_edit_server_config`。当前 Codex formatter 会写 `type`、使用 `http_headers`，且不完整支持 Grok 的 `cwd`、`enabled`、`startup_timeout_sec`、`tool_timeout_sec`、`tool_timeouts`、`bearer_token_env_var`。

Grok stdio schema：

```toml
[mcp_servers.foo]
command = "npx"
args = []
env = {}
cwd = "..."
enabled = true
startup_timeout_sec = 30
tool_timeout_sec = 6000
tool_timeouts = {}
```

远程 schema：

```toml
[mcp_servers.foo]
url = "https://..."
headers = {}
bearer_token_env_var = "TOKEN"
enabled = true
```

新增 Grok 专用 formatter/importer：写 `headers` 而非 `http_headers`，不写 Codex 的 `type`，对所有上述字段做无损 import/export。Windows 本机、WSL 和 SSH 目标都保持 `command = "npx"`，不添加 `cmd /c`。`grok mcp list --json` 和 `grok mcp doctor --json` 用作投影后的诊断，而不是中央数据源。

### 11.3 中央存储语义

MCP 主数据仍在 `mcp_server` 表。Grok `config.toml` 只是派生结果。

新增、编辑、删除或切换 Grok 工具时继续发：

```text
config-changed
mcp-changed
```

`mcp_credentials.json` 是机器和身份相关 OAuth 凭据，继续不进入 MCP 中央表、不默认备份、不跨 WSL/SSH 复制。

---

## 12. Skills 接入

### 12.1 工具 key

Skills 内部 key 使用：

```text
grok
```

目标目录：

```text
<resolved GROK_HOME>/skills
```

### 12.2 需要修改

- `tools/builtin.rs` 内置工具定义；
- `runtime_location::get_tool_skills_path_sync/async`；
- `skills/tool_adapters.rs`；
- `skills/onboarding.rs`；
- `wsl/skills_sync.rs` allowlist；
- `ssh/skills_sync.rs` allowlist；
- 前端工具名称、图标和 preferred tool 列表；
- Inventory import/export 对新 tool key 的兼容。

### 12.3 Source of Truth

中央仓库仍是唯一源：

```text
central_repo_path -> <GROK_HOME>/skills/<skill>
```

不能把 Grok 已有 `skills/` 目录反过来当长期源。已有未管理 Skill 通过 onboarding/import 进入中央仓库。

---

## 13. Runtime location 与 WSL Direct

### 13.1 runtime_location.rs

新增 `grok` 到 `MODULE_KEYS` 和 `normalize_module_key`。

新增：

```text
get_grok_runtime_location_sync/async
get_grok_config_path_sync/async
get_grok_auth_path_sync/async
get_grok_prompt_path_sync/async
get_grok_plugins_path_sync/async
get_grok_sessions_path_sync/async
get_grok_wsl_target_path_async
```

路径规则：

- 本机 custom root：所有派生文件跟随 custom root；
- WSL Direct custom root：文件 I/O 使用 UNC，CLI 在对应 distro/linux path 运行；
- 普通 Windows→WSL 同步：远端目标默认 `~/.grok/*`，不跟随本机 custom root；
- 当前 runtime 本身是 WSL Direct 时，WSL 设置页置灰并跳过普通 WSL 同步。

### 13.2 缓存

保存 Grok root 后：

1. 写 `grok_common_config`；
2. `refresh_runtime_location_cache_for_module_async(db, "grok")`；
3. 再 apply provider/prompt；
4. 再发同步事件。

不能让后续 helper 继续读旧 root。

---

## 14. WSL 同步

### 14.1 默认 mappings

新增：

| id | 名称 | module | Windows | WSL | 默认 |
|---|---|---|---|---|---|
| `grok-auth` | Grok 认证 | `grok` | `~/.grok/auth.json` | `~/.grok/auth.json` | 开启 |
| `grok-config` | Grok 配置 | `grok` | `~/.grok/config.toml` | `~/.grok/config.toml` | 开启 |
| `grok-prompt` | Grok 全局提示词 | `grok` | `~/.grok/AGENTS.md` | `~/.grok/AGENTS.md` | 开启 |
| `grok-plugins` | Grok 插件目录 | `grok` | `~/.grok/plugins` | `~/.grok/plugins` | 开启 |

Skills 不走普通 mapping，继续走独立 Skills WSL sync。

### 14.2 defaults version

`CURRENT_DEFAULTS_VERSION` 从 8 升到 9。

已有用户只 backfill 本版本新增的四个 `grok-*` mapping；不能恢复用户主动删除的其他旧 mapping。

### 14.3 MCP 专用同步

把 `grok-config` 加入 MCP mapping 白名单和 WSL Direct skip 判断。

Grok 配置后处理保持裸 `npx`，不加 `cmd /c`。

整个 `config.toml` 还可能包含 MCP `cwd`、`skills.paths`、`plugins.paths`、`auth_provider_command` 和 local marketplace path。禁止对整份 TOML 做猜测式字符串路径替换。跨平台同步顺序固定为：

```text
复制普通受管配置
→ 复制 prompt/plugins/auth
→ 最后由 MCP 中央库针对目标平台重新投影 MCP
→ 解析并校验目标 config.toml
```

如果某个非 MCP 路径没有可靠的字段级转换规则，保留原值并返回 warning，不能静默改坏。

### 14.4 自动同步事件

新增：

```text
wsl-sync-request-grok
```

provider、common config、prompt、plugin 操作完成后发出事件；真正是否同步仍由 `lib.rs` listener 和 WSL auto-sync 设置决定。

---

## 15. SSH 同步

SSH 默认 mappings 与 WSL 相同：

```text
grok-auth
grok-config
grok-prompt
grok-plugins
```

规则：

- `ssh_defaults_version` 8 → 9；
- 只 backfill 本次 Grok mappings；
- SSH 仍是手动/启用/切连接时全量同步，不新增自动事件监听；
- 本机 Grok 若为 WSL Direct，SSH local source 显示/解析为 UNC，但不禁用；
- Skills 走独立 SSH skills sync；
- MCP 专用同步把 `grok-config` 纳入 TOML mapping；
- `auth.json` 恢复/上传后尽量设置仅用户可读权限；
- `plugins/` 目录同步复用通用临时目录替换和目录排除规则，不跟随 dangling symlink。

SSH 默认同步 `auth.json` 以保持与 Codex 当前产品语义一致，但设置页必须提示其包含敏感凭据；用户可以关闭 `grok-auth` mapping，并在远端直接运行 Device Code 登录。同步顺序同 WSL：普通文件先完成，MCP 最后按远端平台重新投影。

---

## 16. 备份恢复

### 16.1 SQLite

Grok provider/common/prompt 表会随 `sqlite/ai-toolbox.db` 自动进入本地和 WebDAV 备份，不需要单独导出 JSON。

### 16.2 external-configs

新增：

```text
external-configs/grok/root-dir.txt
external-configs/grok/auth.json
external-configs/grok/config.toml
external-configs/grok/AGENTS.md
external-configs/grok/plugins/**
```

默认不加入：

```text
sessions/**
memory/**
mcp_credentials.json
trusted_folders.toml
logs/**
downloads/**
```

理由：sessions/memory 体积可能很大；MCP OAuth 和 trust 状态有机器/身份边界；logs/downloads 可重建。

`config.toml` 本身也可能含 `[model.*].api_key`、Authorization header 或外部认证命令，因此安全等级不能低于 `auth.json`：不得打印内容，preview 必须掩码，Unix restore 后设置安全权限，WebDAV 页面显示敏感数据风险提示，并允许通过 file filter 排除。

### 16.3 文件过滤

`list_backup_file_filter_path_options` 增加 Grok 当前实际存在的：

- `auth.json`
- `config.toml`
- `AGENTS.md`
- `plugins/<relative path>`

过滤规则仍按 `tool = "grok" + relative path` 精确匹配，备份排除和恢复跳过共用同一 helper。

### 16.4 restore

恢复流程：

1. 读取 `root-dir.txt`；
2. 跨平台校验 custom root；
3. 无效/不可用时回退当前用户 `~/.grok` 并记录 warning；
4. 先在 staging 目录校验 zip entry，拒绝 `..`、绝对路径、盘符逃逸和 symlink 逃逸；
5. 排除 plugin 内 `.git`、`node_modules`、cache 和 build artifacts，避免重复 zip entry；
6. 用安全 relative path resolver 和原子替换写回；
7. `auth.json` 与含 secret 的 `config.toml` 写回后设置安全权限；
8. 数据库恢复完成后刷新 Grok runtime location cache；
9. 恢复 Provider/Common/Prompt 数据并按 applied 状态投影；
10. 最后依次执行 MCP 和 Skills 中央数据投影，避免整个 config restore 覆盖刚生成的 MCP；
11. 前端提示重启应用并展示 warning。

### 16.5 最小备份测试

- backup zip 包含 SQLite 与 Grok external configs；
- filter 排除 `grok/auth.json` 时备份和恢复都跳过；
- custom root 往返；
- Windows 备份恢复到 Linux/macOS 时路径安全；
- `plugins/**` 不产生重复 zip entry；
- restore 后 MCP/Skills resync 不覆盖用户 unknown TOML。

---

## 17. Session Manager

### 17.1 后端

新增：

```text
tauri/src/coding/session_manager/grok.rs
```

扩展 `ToolSessionContext` / tool enum：

```text
grok
```

需要实现：

- list / recent scan；
- detail；
- rename 仅在真实 fixture 证明官方 CLI或稳定文件语义后启用；未确认前不加入 `canRenameSession()`；
- delete；
- AI Toolbox JSON export；
- Grok 官方 Markdown export；
- native complete directory snapshot export/import；
- normalized message blocks；
- resume command；
- cache/search metadata。

官方 CLI 已确认支持 list、search、delete、Markdown export 和 external JSONL import，但没有明确 rename CLI。delete 优先调用 `grok sessions delete`，不要自行删半个目录。`grok import` 是外部 JSONL 导入，不得拿来恢复 AI Toolbox native snapshot。

### 17.2 数据源

列表从 `summary.json` 读取 metadata，详情从 `updates.jsonl` 解析 ACP session updates。

首版 fixture 必须由真实 Grok CLI 生成并脱敏，至少包含：

- user/assistant text；
- thinking；
- bash/tool call + result；
- file edit；
- MCP tool；
- plan；
- compact；
- fork；
- subagent；
- failed tool / permission denied。

native snapshot 必须包含完整会话目录及 plan、rewind、signals、feedback、subagents 等伴随状态；恢复后校验目录结构并实际验证 `grok --resume <session-id>`。三种导出格式必须在 UI 和后端类型中明确区分，不能都叫“导出会话”。

### 17.3 resume command

```bash
grok --resume <session-id>
```

如果 `summary.json` 能解析真实 cwd：

- Windows drive/UNC：`pushd "<cwd>" && grok --resume <id>`；
- macOS/Linux：`cd '<cwd>' && grok --resume <id>`。

不能使用 sessions 存储目录冒充项目 cwd。

### 17.4 前端

新增二级路由：

```text
/coding/grok/sessions/detail
```

复用 `SessionDetailWorkbench`，只增加 Grok tool key、API alias 和 route chrome，不新增专用详情 UI。

---

## 18. 托盘

新增 `tauri/src/coding/grok/tray_support.rs`。

Provider/model 菜单：

- 按 `sort_index`；
- 排除 `__local__`；
- disabled provider 不可选；
- 选中项来自 `is_applied`；
- 点击走统一 `select_grok_provider` 或 Gateway-aware switch。

Prompt 菜单直接复用 Codex/Pi prompt tray 结构。

需要修改：

- `tray.rs` provider event id：`grok_provider_<id>`；
- prompt event id；
- visible tab 判断；
- tray 文案；
- section 插入顺序与顶部 Tab 一致。

---

## 19. Proxy Gateway

Grok 原生支持 Chat/Responses/Messages，因此 Gateway 不是普通自定义渠道的协议兼容前提。Gateway 只提供：

- single 本机代理；
- failover；
- 请求日志、统计和健康；
- provider billing。

### 19.1 新增 CLI key

```rust
GatewayCliKey::Grok
```

同步扩展：

- TS `GatewayCliKey`；
- `ProxyGatewaySettings.app_configs.grok`；
- manifest 路径；
- provider table 映射到 `GrokProvider`；
- provider loader；
- Gateway 设置页；
- usage provider name lookup；
- CLI stop protection；
- WSL target origin rewrite。

### 19.2 Grok 入站路由

当前 Gateway 只有 `/anthropic/v1`、`/openai/v1`、`/gemini/v1beta`，不存在 `/grok/v1`。如果接管配置写 `base_url = "http://127.0.0.1:<port>/grok/v1"`，必须新增完整路由：

```text
/grok/v1/responses
→ cli_key = Grok
→ forwarded_path = /v1/responses
```

同步补齐 route matcher、auth strategy、usage parser、provider loader/cache、billing、health、requests/statistics filters、settings、manifest、takeover/re-engage/restore、WSL origin rewrite、所有 Rust/TS unions 和 route tests。不能只加 enum 或 UI 开关。

### 19.3 接管投影

Gateway 接管时在 Grok `config.toml` 写一个 AI Toolbox 专用 model entry，固定以 OpenAI Responses 作为 Grok→Gateway 入站协议：

```toml
[models]
default = "ai-toolbox-gateway"

[model.ai-toolbox-gateway]
model = "<P0 default model>"
name = "AI Toolbox Gateway"
base_url = "http://127.0.0.1:<port>/grok/v1"
api_key = "ai-toolbox-gateway"
api_backend = "responses"
```

manifest 记录：

- 原始 config backup；
- managed model key；
- `[models].default`；
- mode；
- primary provider id；
- root/config path；
- 文件 hash/size。

恢复直连只恢复受管字段，保留接管期间 Grok runtime 写入的其他 unknown fields。

Gateway 接管期间必须锁定 Root Directory、Common Config、普通 Provider apply/save 等会覆盖接管配置的入口；active provider 保存后走 re-engage；提供 restore direct。锁的交互和 tooltip 复用 Codex Gateway takeover，不允许普通保存静默覆盖 Gateway manifest 所管理的字段。

### 19.4 官方 provider

`category=official` 没有可转发 API key，不进入 Gateway 候选。

### 19.5 UI

直接保留 Codex provider card 的 Gateway button 位置和 `GatewayFailoverButton`。

Grok 的 `providerNeedsGatewayProxy` 始终为 false，因为三种协议均可直连；只有用户主动点击“网关代理”才接管。

---

## 20. 导航、设置和 i18n

### 20.1 前端入口

需要更新：

- `web/features/coding/index.ts`
- `web/constants/modules.tsx`
- `web/app/routeConfig.ts`
- `web/components/layout/MainLayout/index.tsx`
- `web/services/settingsApi.ts`
- `web/stores/settingsStore.ts`
- `web/features/settings/pages/GeneralSettingsPage.tsx`
- WSL/SSH hooks 和 Modal tool maps
- Backup tool order
- sync message translator

### 20.2 i18n

新增 `grok.*`、`subModules.grok`、同步和备份文案。

必须使用：

```bash
pnpm i18n:set-key ... --write
pnpm i18n:find-key ...
pnpm i18n:check
```

不要直接编辑完整 locale JSON。

### 20.3 名称

页面主标题建议显示：

```text
Grok CLI
```

辅助文案可写：

```text
Grok Build
```

代码模块统一使用 `grok`，不要同时出现 `xai_cli`、`grok_build`、`grokcli` 多套 key。

---

## 21. 官方能力范围矩阵

“不提供专用 UI”不等于可以删除配置或文件。所有 Grok writer、备份和同步实现都必须按下表处理：

| 官方能力 | 首版状态 | AI Toolbox 行为 |
|---|---|---|
| Headless / Device Code | 支持 | 官方账号 Device Code；不依赖可见终端 |
| ACP | 保留 | Session parser 消费已确认事件，不改 ACP 配置 |
| Hooks | 保留 | 无编辑 UI；保留配置和文件，默认不跨机器复制 |
| Worktrees | 保留 | Session cwd/resume 不改写 worktree |
| Dashboard | 不管理 | 不复制官方 Dashboard，可展示诊断入口 |
| Background Tasks | 保留 | snapshot 不漏相关事件，无管理 UI |
| Memory | 后续 | 不默认备份/同步；配置字段保留 |
| Subagents / Agents | 保留 | config、session、snapshot 无损；无专用 UI |
| LSP | 保留 | 不删除 `lsp.json` 或相关配置，默认不备份 |
| Sandbox / Permissions | Common Config | 高级 TOML 无损管理，不缩窄 schema |
| Themes / pager | 保留 | 无 UI，不删除配置和 `pager.toml` |
| managed config / requirements | 只读诊断 | 不写企业文件；提示 effective config 覆盖 |
| project `.grok/config.toml` | 后续 | 不管理；用户级 writer 不影响项目配置 |
| Cursor / Claude compatibility | 保留 | 不把兼容来源重新写为 Grok 用户配置 |
| Remote session sync | 后续 | 默认 mapping 不复制 sessions；snapshot 需显式操作 |
| CLI update | 后续/外部 | 显示版本和安装说明，首版不自动升级 |
| completions | 不管理 | 不修改 shell profile |
| setup | 外部 | 提供官方命令，不复制 setup UI |

Phase 0 必须对官方 Overview、Settings、CLI Reference 和本机 `grok --help` 做逐项 checklist，新能力先归类为“支持、保留、不管理、后续”之一，再实现 writer，避免只对齐 Codex 已有功能。

---

## 22. Codex 官方账号登录升级计划

此项与 Grok 认证可以共用安全写入、登录 session 和状态事件等基础设施，但必须作为独立 Phase、独立命令和独立测试交付，严禁混用 endpoint、Client ID、scope 或 snapshot schema。

### 22.1 当前实现与参考

主要入口为 `tauri/src/coding/codex/official_accounts.rs`，参考 `/mnt/d/GitHub/cli-proxy-api/internal/auth/codex/`、`sdk/auth/codex.go`、`sdk/auth/codex_device.go`。本机 `codex-cli 0.144.1` 已确认支持 `codex login --device-auth`。

保留现有浏览器 OAuth；Device Code 用于 WSL、SSH、headless、1455 端口被占用、无法打开浏览器等场景，或作为 callback bind 失败后的明确 fallback。

### 22.2 HTTP、协议和登录会话

OAuth exchange/refresh 改为复用：

```rust
http_client::client_with_timeout(&db, 20)
```

确保继承应用代理和 rustls，不在认证模块单独 new 默认 reqwest client。Device Code 参考 endpoint：

```text
POST https://auth.openai.com/api/accounts/deviceauth/usercode
POST https://auth.openai.com/api/accounts/deviceauth/token
verification https://auth.openai.com/codex/device
exchange redirect https://auth.openai.com/deviceauth/callback
```

实现 typed login session、cancel、loading、过期清理、状态事件和 typed errors；端口占用时向用户提供 Device Code fallback。

本轮未能从实时 OpenAI 官方手册确认 scope 差异，因此不得仅按 CLIProxyAPI 修改当前 Client ID、scope、originator 或 connectors scopes。实现前应通过已注册的 OpenAI Developer Docs MCP或最新官方 CLI source 再核对；未确认时保持现值。

### 22.3 Refresh 并发与字段保留

- 同一账号并发 refresh 使用 single-flight/互斥合并，避免 `refresh_token_reused`；
- response 缺少新 refresh token 时保留旧值；
- response 缺少新 ID token 时保留旧值；
- `refresh_token_reused` 返回 typed error 并引导重新登录，不无限重试；
- 账号切换、refresh 与 applied 状态更新保持事务性；
- browser OAuth 和 Device Code 最终共用 snapshot validation、保存和 runtime apply writer。

### 22.4 Codex auth.json 安全写入

只改 fixture 已确认的 OAuth/API key managed fields，保留 `auth_mode`、`tokens` 内未知字段、`last_refresh` 等 runtime-owned 数据；原子写入，Unix 权限 `0600`，token 不进入日志、事件和普通错误。

### 22.5 Codex 升级测试

覆盖 browser OAuth、device flow、1455 occupied fallback、cancel、timeout、invalid state、concurrent refresh、`refresh_token_reused`、missing refresh token、missing ID token、proxy/rustls、atomic write、Unix permission、existing runtime-owned fields preserve，以及 WSL/SSH/headless 手工验收。

---

## 23. 当前项目改动清单

### 23.1 新增文件

```text
tauri/src/coding/grok/**
tauri/src/coding/session_manager/grok.rs
web/features/coding/grok/**
web/services/grokApi.ts
web/services/grokPromptApi.ts
web/types/grok.ts
web/test/features/coding/grok/**
tauri/tests/fixtures/grok/**
```

### 23.2 核心后端修改

```text
tauri/src/db/schema.rs
tauri/src/db/migrations.rs
tauri/src/coding/mod.rs
tauri/src/coding/runtime_location.rs
tauri/src/coding/tools/builtin.rs
tauri/src/coding/tools/detection.rs
tauri/src/coding/mcp/config_sync.rs
tauri/src/coding/mcp/command_normalize.rs
tauri/src/coding/skills/tool_adapters.rs
tauri/src/coding/skills/onboarding.rs
tauri/src/coding/wsl/commands.rs
tauri/src/coding/wsl/mcp_sync.rs
tauri/src/coding/wsl/skills_sync.rs
tauri/src/coding/ssh/commands.rs
tauri/src/coding/ssh/mcp_sync.rs
tauri/src/coding/ssh/skills_sync.rs
tauri/src/coding/session_manager/mod.rs
tauri/src/settings/backup/utils.rs
tauri/src/settings/backup/local.rs
tauri/src/settings/types.rs
tauri/src/settings/adapter.rs
tauri/src/tray.rs
tauri/src/lib.rs
```

Codex 官方账号升级额外涉及：

```text
tauri/src/coding/codex/official_accounts.rs
tauri/src/coding/codex/commands.rs
web/features/coding/codex/**
web/services/codexApi.ts
web/types/codex.ts
web/test/features/coding/codex/**
```

Gateway Phase 额外修改：

```text
tauri/src/coding/proxy_gateway/**
web/services/proxyGatewayApi.ts
web/features/settings/pages/GatewaySettingsPanel.tsx
web/features/coding/shared/gateway/**
```

### 23.3 文档

新增：

```text
tauri/src/coding/grok/AGENTS.md
web/features/coding/grok/AGENTS.md
```

并在根 `AGENTS.md` Index 中登记。

如果实现引入 Grok TOML ownership、Windows MCP 不包装等高风险规则，还应同步补到相应 `mcp/AGENTS.md`、`wsl/AGENTS.md`、`ssh/AGENTS.md` 和 `backup/AGENTS.md`。

---

## 24. 分阶段实施

### Phase 0：官方 fixtures 与边界确认

- 固定 Grok CLI、Codex CLI 和参考仓库 commit；记录版本漂移检查方式；
- 使用隔离 `GROK_HOME` 生成 official/custom/config/auth/login/refresh/logout 脱敏 fixtures；
- 完成官方能力矩阵 checklist 和 config 字段 ownership 表；
- 生成完整 session fixture，覆盖 plan/rewind/signals/feedback/subagents；
- 核实 plugin、marketplace、MCP、sessions、models 的 JSON/文本输出；
- Windows 实机验证 Grok MCP `npx` 不需 `cmd /c`；
- 验证 auth writer、CLI识别、Unix `0600` 和 logout 行为；
- 从最新官方资料核对 Codex Device Code 参数，未确认的 Client ID/scope 保持不变。

退出条件：fixtures 已脱敏入库，所有 writer 所有权不再依赖猜测，未确认项明确标为阻塞或后续。

### Phase 1：数据库与共享安全基础设施

- v8 四表 migration、indexes、DbTable/ALL_TABLES 和 migration tests；
- Grok module、runtime location、CLI resolver；
- 原子 JSON/TOML writer、secret masking、权限 helper；
- in-memory auth session、cancel/status/cleanup 基础设施；
- 新增/更新模块 AGENTS 记录高风险 ownership 和凭据规则。

### Phase 2：Grok Provider、Common Config、Prompt

- Provider CRUD/copy/reorder/disable/apply/local import；
- 完整 model schema 和 `extraConfig` 无损往返；
- Common Config 字段级 protected/ignored extraction；
- official/custom/local Provider；
- prompt preset 与 `<root>/AGENTS.md`；
- 后端命令、事件、tray/Gateway cache invalidation 基础连接。

退出条件：不用前端即可完成 `read → edit → save → apply → read`，fixture diff 只包含预期字段。

### Phase 3：Grok 官方账号 Device Code

- OIDC discovery endpoint allowlist；
- device code start/poll/slow_down/cancel/expire/deny；
- access-token principal/team/client claims 与 OIDC userinfo 解析；
- official account CRUD、refresh、apply、save local、logout；
- runtime `auth.json` 字段级 writer；
- 并发与错误状态测试。

### Phase 4：复制 Codex 前端

- 复制 Codex 页面、卡片、表单、common modal 和 Less；
- 接入 Grok types/services；
- 保留官方账号区并替换 Device Code Modal；删除 quota/plan/token copy、history sync、unified history、preserve auth UI；
- 加导航、`Grok` icon、visible tab 和 i18n；
- provider CRUD、排序、复制、导入、连通性测试；
- Common Config protected Alert、配置预览和 Gateway lock placeholder。

完成标准：UI 结构与 Codex 一致，不出现新视觉方案。

### Phase 5：Plugins、MCP、Skills

- Grok Plugins Panel 全部 CLI 操作、`--trust` 确认和 partial failure；
- RuntimeTool；
- Grok 专用 MCP TOML formatter/importer、diagnostics 和 Windows no-wrap；
- Skills 中央仓库、onboarding、tool preferences；
- Skills 路径 ownership 验证和独立同步。

### Phase 6：Session Manager 与托盘

- Grok session parser；
- list/detail/search/delete、三类 export/import；rename 通过 fixture 后才启用；
- 二级详情路由；
- tray provider/model/prompt；
- resume command 和 restored snapshot 实际验证。

### Phase 7：WSL、SSH、备份恢复

- Grok default mappings；
- defaults version 9 backfill；
- WSL Direct 状态与 skip；
- SSH 动态 local source；
- auth 敏感提示和可关闭 mapping；
- 跨平台字段级路径处理和“MCP 最后投影”顺序；
- external-configs/grok；
- config/auth masking、filter、permission、zip traversal/symlink 防护；
- local/WebDAV restore roundtrip。

### Phase 8：Codex 官方账号升级

- OAuth HTTP client 切换为全局 rustls/proxy client；
- 保留 browser OAuth，新增 Device Code 与端口占用 fallback；
- refresh single-flight、缺失字段保留和 typed errors；
- auth.json 原子写入/0600/runtime field preserve；
- 独立前后端测试和 WSL/SSH/headless 验收。

### Phase 9：Gateway

- `GatewayCliKey::Grok`；
- `/grok/v1/responses` route matcher/forwarding/tests；
- single/failover、provider loader/cache；
- manifest/backup/restore；
- provider loading、billing、usage、health；
- UI Gateway controls 和 takeover locks/re-engage/restore direct；
- WSL gateway origin rewrite。

### Phase 10：完整验证与文档固化

- 全量测试；
- Windows/macOS/Linux；
- WSL Direct；
- Windows→WSL；
- SSH；
- backup→restore；
- 模块 AGENTS 更新；
- `grok inspect --json` 对照最终 runtime。

---

## 25. 测试计划

### 25.1 Rust 单元/集成测试

至少覆盖：

- v8 四表 migration 和 indexes；
- root precedence；
- WSL UNC 解析；
- common/provider TOML merge；
- unknown sections 保留；
- MCP/plugins/skills/auth sections 保留；
- 只删除前一 provider managed model keys；
- official/custom/local provider；
- API key 只投影到受管 model；
- provider 保存→应用→读取；
- model `env_key`/false/unknown/sampling/retry/timeout/reasoning 无损往返；
- Grok Device Code pending/slow_down/deny/expire/cancel/success；
- official account refresh/apply/delete/logout 和 secret 不出现在 payload；
- prompt 文件；
- Grok MCP 全字段往返、`headers`/`bearer_token_env_var`、Windows no-wrap；
- WSL/SSH mapping backfill；
- backup/filter/restore；
- session fixtures；
- tray selection；
- Gateway engage/restore/re-engage。

### 25.2 前端测试

测试文件放在 `web/test/`：

- settingsConfig parse/build；
- provider category copy/edit 锁定；
- model mapping 保留 false/modalities/envKey/extraConfig；
- Device Code Modal 状态、cancel 和无 token payload；
- official account card 无 quota/plan/token copy；
- API backend 映射；
- favorite provider payload；
- disabled provider batch test 过滤；
- Session Manager tool key/route；
- visible tab normalization；
- Grok icon branch至少通过 TS/build 校验。

### 25.3 全量命令

这是跨模块、跨层、影响保存/应用/同步/恢复的大功能，最终交付前必须执行：

```bash
pnpm test
cd tauri && cargo test
pnpm exec tsc --noEmit
pnpm build
```

同时运行：

```bash
pnpm i18n:check
cd tauri && cargo fmt --check
git diff --check
```

### 25.4 手工验收

- 官方登录后切自定义 provider，`auth.json` 不丢；
- 切回 official 后 `grok` 可直接启动；
- custom Chat/Responses/Messages 三种 backend；
- 获取模型和默认模型；
- `grok inspect --json` 能看到 AI Toolbox 写入的 config/MCP/Skills/Plugins；
- Windows MCP `npx` 未被写成 `cmd /c`；
- WSL Direct root 指向正确 UNC/Linux path；
- WSL auto sync 开关行为；
- SSH Sync Now；
- backup→删除本地状态→restore→重启→provider/MCP/Skills/Plugins 可用；
- Session detail、resume、delete；rename 仅在能力已验证时验收；
- tray 切 provider/prompt；
- Gateway single/failover/restore。

另行执行 Codex browser/device OAuth、port occupied fallback、并发 refresh、proxy/rustls、auth.json preserve 的完整测试矩阵，不能因 Grok 测试通过而视为 Codex 升级通过。

---

## 26. 关键风险

### 26.1 不能把 Codex TOML 语义复制给 Grok

Codex 使用 `model_provider` / `[model_providers.*]`；Grok 使用 `[models].default` / `[model.*]`。页面可以复制，落盘逻辑必须重写。

### 26.2 不能把 auth.json 当 provider secret store

Grok OAuth token 是 runtime-owned。自定义 API Key 应投影到受管 `[model.*]`，不能覆盖 OAuth 文件。

### 26.3 不能全删 `[model.*]`

用户可能手写多个模型。只能删除前一 AI Toolbox provider 的精确 keys。

### 26.4 MCP Windows wrapping 相反

Grok 官方自己解析 npm `.cmd` shim。照搬 Codex wrapping 会产生不必要甚至错误的 `cmd /c`。

### 26.5 Session 格式不是 Codex JSONL

必须使用 `summary.json + updates.jsonl`，并通过真实 fixture 维护 parser。

### 26.6 Plugins/Skills/MCP 有多个发现来源

AI Toolbox 只管理自己的用户级目标和中心存储，不应扫描后把 Claude/project compatibility 来源重新写成 Grok 用户配置。

### 26.7 高优先级企业配置可能覆盖用户配置

AI Toolbox 保存成功不等于最终 effective config 一定采用。诊断应建议用户运行/查看 `grok inspect`，不能尝试绕过 requirements。

### 26.8 认证参考实现不等于 runtime 文件规范

CLIProxyAPI 用于理解 Device Code、refresh 和错误处理，不是 Grok/Codex runtime `auth.json` schema 的 Source of Truth。没有真实 fixture 就不得写 writer。

### 26.9 配置和备份都可能泄露凭据

secret 不只在 `auth.json`；`config.toml`、MCP headers、provider JSONB 和 WebDAV zip 都可能含凭据。日志、preview、错误和测试 fixture 必须统一掩码。

### 26.10 Gateway 不能只增加 enum

缺少 `/grok/v1/responses` matcher、usage、health、filters、manifest 或 takeover locks 中任一环节，都会形成“UI 看似支持但请求不可用或无法恢复”的半成品。

---

## 27. 完成定义

只有同时满足以下条件才算“新增支持 Grok CLI”完成：

- Grok Tab、图标、路由、设置可见性完整；
- UI 明确复用 Codex 页面结构，没有额外视觉发挥；
- provider/official account/common/prompt 有四张独立 SQLite JSONB 表及索引；
- `config.toml` 结构化 merge 且不破坏用户/runtime sections；
- Provider model 全字段完成无损往返；
- Grok Device Code、账号保存/刷新/应用/删除/退出可用，前端拿不到 token；
- OAuth 与自定义 API Key 共存，切换自定义 Provider 不清除官方登录；
- Plugins 的 installed/discover/marketplace/local/update/details/validate/enable/disable/bulk/partial failure 可用；
- Grok 专用 MCP formatter/importer 和 Skills 中央同步可用；
- WSL Direct、WSL、SSH 同步可用；
- 本地/WebDAV 备份恢复通过 traversal、symlink、secret masking 和投影顺序测试；
- Session Manager 支持已确认的 list/detail/search/delete/export/import/resume；rename 只在官方能力已验证时启用；
- tray provider/model/prompt 可用；
- Gateway `/grok/v1/responses`、single/failover、takeover locks、re-engage 和 restore direct 可用；
- Codex browser OAuth 保持可用，Device Code、proxy/rustls、并发 refresh 和安全 writer 升级通过独立测试；

---

## 28. 实施复核补充（2026-07-12）

本节记录开发过程中对官方 `grok 0.2.93` 内置文档、真实 runtime 文件和 Codex 现有能力再次交叉核对后的修正。若本节与前文早期假设冲突，以本节和当前官方文档为准。

### 28.1 Provider 存储与 TOML 投影修正

前端虽然机械复用 Codex 页面，但数据库和 TOML 语义必须使用 Grok 模型：

```text
settingsConfig.defaultModelKey
settingsConfig.modelCatalog.models[]
              │
              ├──> [models].default
              └──> [model.<key>]
```

不得在 Grok 数据中生成 Codex 的 `model_provider`、`[model_providers.*]`、`wire_api` 或 `model_catalog_json`。Provider 表单的 Base URL、API backend、API key、模型映射和默认模型先进入结构化 JSONB；后端应用时再生成官方 Grok TOML。Provider 高级 TOML 只管理非模型附加配置，并与 Common Config 一样禁止覆盖 `[models].default`、`[model.*]`、`[mcp_servers]`、`[plugins]` 和 marketplace。

官方文档确认未设置 `api_backend` 时默认是 `chat_completions`，因此新建普通自定义渠道默认选择 OpenAI Chat，而不是 Responses。Grok 原生支持 `chat_completions`、`responses`、`messages`；不原生支持 `gemini_native`。Gemini Native endpoint 只能作为 Gateway 转换目标，不能作为 Grok 直连 TOML backend。

### 28.2 通用配置“隐私保护”快捷项

Common Config Modal 顶部沿用 Codex 快捷选项的同一布局，只新增用户指定的“隐私保护”，不增加新卡片或新视觉结构。开启时字段级写入：

```toml
[features]
telemetry = false
codebase_indexing = false

[telemetry]
trace_upload = false

[harness]
disable_codebase_upload = true
```

关闭时只删除值仍分别为 `false`、`false`、`false`、`true` 的上述四个受管字段；如果用户在编辑器中改成其他值，则快捷项不得删除。空 section 才删除，`[features]`、`[telemetry]`、`[harness]` 中其他用户字段始终保留。

从 Codex 复制来的 Goal mode 和“远程压缩”快捷项已经删除：Grok 官方配置没有 Codex 的 `features.goals` 或 `[model_providers.*].name = "OpenAI"` 语义。Grok 的压缩配置属于 `[session]` / `[compaction]`，继续通过高级 TOML 管理，不伪装成 Codex 快捷开关。

### 28.3 Plugins 当前官方命令矩阵

当前实现应覆盖并通过当前 runtime location 调用：

```text
grok plugin list --json
grok plugin list --json --available
grok plugin install <source> --trust
grok plugin update [name]
grok plugin details <name>
grok plugin validate [path]
grok plugin enable <name>
grok plugin disable <name>
grok plugin uninstall <name> --confirm
grok plugin marketplace list --json
grok plugin marketplace add/remove/update <source-or-name>
```

available 列表中的 plugin name 不是可靠安装 source。AI Toolbox 必须读取 `<grok-root>/marketplace-cache/*/.claude-plugin/marketplace.json`，把相对目录、git URL、ref 和 subdirectory 解析为 CLI 可接受的安装 source。无法解析 source 时保留展示但禁用安装，不把 plugin name 误传给 `install`。

### 28.4 Session Manager 真实格式与安全边界

当前 Grok session 目录除 `summary.json`、`chat_history.jsonl`、`events.jsonl` 外，还可能包含 `system_prompt.txt`、`rewind_points.jsonl`、`updates.jsonl`、`prompt_context.json`、terminal logs 和 plan state。详情 parser 至少识别 user/assistant text、`reasoning`、assistant `tool_calls` 和 `tool_result`，不能把 reasoning/tool 记录当空消息丢弃。

原生快照导入对 `relativeDir` 和每个文件名执行跨平台路径校验：拒绝绝对路径、盘符、空 segment、`.`、`..` 和反斜杠形式的穿越。导出只包含 session 自身目录，不把根级 `session_search.sqlite` 或同项目其他 session 带入。

本机官方 `grok 0.2.93` 再次核验后确认：`grok sessions` 只有 `list`、`search`、`delete`，但根命令提供 `grok export <SESSION_ID> [OUTPUT]`，可对任意历史 session 无交互导出 Markdown；不能因为 `sessions` 子命令中没有 export 就漏掉根命令。`grok import` 接收 session ID 或外部 `.jsonl`，用于迁移其他来源会话，不是 Grok native session snapshot 的恢复命令。Session Manager 因此明确提供三种导出：`ai-toolbox.session-export.v2` JSON（normalized messages + native snapshot）、调用官方 `grok export` 的 Markdown、以及可独立导入的 `ai-toolbox.grok-native-snapshot.v1` 原生目录快照 JSON。单个和批量导出都先选择格式；原生快照导入由 AI Toolbox 严格校验并恢复，不拿 `grok import` 冒充 native restore。

### 28.5 Codex 官方登录复核结论

本机官方 `codex-cli 0.144.1` binary 直接包含 scope：

```text
openid profile email offline_access api.connectors.read api.connectors.invoke
```

因此本轮保留该 scope，不按参考项目的精简 scope 盲改。官方 binary 同时包含动态 `http://localhost:<port>/auth/callback` 逻辑和“Unable to determine the server port”错误，支持优先 1455、占用时绑定 loopback 随机端口的当前实现；Device Code 仍作为无浏览器、远程或 callback 失败时的独立登录方式。

Codex OAuth HTTP 请求统一复用应用 rustls/proxy-aware client；refresh token 轮换缺少新 refresh/id token 时保留旧字段；相同 refresh token 的并发刷新通过锁和短期响应缓存收敛；`auth.json` 使用同目录临时文件原子替换并在 Unix 设置 `0600`。备份恢复 Codex/Grok `auth.json` 后也重新设置 `0600`。

当前会话的 OpenAI Docs MCP 在加入后需要重启才能暴露，因此上述 Codex scope/端口事实来自本机官方 CLI 0.144.1 binary 与 `cli-proxy-api` 交叉验证；最终验收不得写成“已通过当前会话 OpenAI Docs MCP 验证”。

### 28.6 真实授权与官方 CLI 端到端验证

2026-07-13 已在仓库外使用隔离 `GROK_HOME` 完成真实 `grok login --device-auth`。官方 CLI 生成的 `auth.json` 为 OIDC scope-map schema，而不是此前参考项目推测的扁平 token schema；真实字段结构已写入 7.5，token 值未进入仓库、日志或文档。

基于真实 token 另建 writer 验证目录，只写当前 AI Toolbox writer 会生成的最小字段（不复制 `coding_data_retention_opt_out` 等 CLI enrichment），Unix 权限为 `0600`。官方 `grok models` 明确返回“已使用 grok.com 登录”、默认模型 `grok-4.5` 和可用模型列表，证明新 writer 结构可被官方 CLI 识别。

随后使用真实 refresh token 请求官方 token endpoint，响应包含轮换后的 `refresh_token`。按 writer 规则更新 `key`、轮换 refresh token 和 `expires_at` 后再次执行 `grok models`，仍成功识别登录。最后执行官方 `grok logout`，在只有一个 scope 时 `auth.json` 被删除。实现据此改为：refresh 保留 CLI enrichment；apply 保留其他 scope；logout/delete 只删除 xAI scope，且仅在最后一个 scope 消失时删除文件。

本机官方 binary 中可见的 `MintGrokAuthResponse` 字段仍属于 devbox/internal mint 链路，不能直接等同于本地 `auth.json` schema；最终实现以本次官方授权生成的脱敏 fixture 和官方 CLI 往返结果为 Source of Truth。

### 28.7 `__local__` 收编与字段级撤销补充

数据库为空但当前 runtime 已存在 `config.toml` 时，`list_grok_providers` 必须返回只读临时 `__local__` Provider。后端从 `[models].default` 和 `[model.*]` 构造 `defaultModelKey + modelCatalog.models`，将已知模型字段映射为 camelCase，未知合法模型字段放入 `extraConfig`；其余非 Provider 所有者配置拆为 Common Config。`mcp_servers`、Plugins、marketplace 等 runtime-owned section 不进入 Provider/Common，`auth.json` 的 access/refresh/id token 绝不进入前端 Provider payload。

“预览当前配置”仍可展示 `auth.json` 的结构和非敏感诊断字段，但后端返回前必须递归脱敏 token、secret、authorization、password 和 API key 类字段；前端不得拿到真实 OAuth 凭据。

用户保存 `__local__` 时是收编当前已生效配置，因此新记录保持 `is_applied=true`。如果用户同时修改模型 key，应用前必须使用收编前的模型快照删除旧的、仍与原投影完全一致的 `[model.<old>]`，再写入新模型；用户手工改过的旧模型表不能误删。

Common Config 和 Provider 高级 TOML 的“删除字段”不能只更新 SQLite。重新投影 runtime 前，必须递归删除上一份受管配置中仍与 runtime 当前值一致的字段，再合并新配置；值已被用户或 runtime 修改时保留。该规则保证关闭“隐私保护”后四个快捷项字段真正从 `config.toml` 撤销，同时保留 section 内其他字段和用户改写值。

### 28.8 Gateway provider profile 复用补充

Grok Provider Form 复用 Gateway 内置供应商快捷选择时，`gateway_provider_profiles.json` 必须真实包含 `tools.grok`，不能只在 TypeScript union 增加 `grok`。当前 22 个 profile 的 Grok endpoint 机械复制相同 profile 已验证的 `tools.codex` OpenAI 兼容 endpoint 数据，不单独发明 URL、模型或兼容规则；Grok 的 `api_backend` 和 Gateway transformer 继续负责 Chat/Responses/Messages/Gemini Native 的协议边界。后端 profile validator、runtime resolver、前端 endpoint 推断和 provider meta 引用统一使用 `tool="grok"`。

### 28.9 托盘模型、用户模型表与本地密钥边界

Grok 托盘除 Provider 和 Prompt 外，已补齐当前 applied Provider 的模型子菜单。模型显示名直接来自 `modelCatalog.models`，切换时只更新该 Provider 的 `defaultModelKey` 并重新投影 `[models].default`；Gateway 接管期间模型项保持禁用，避免绕过 manifest 覆盖接管配置。

应用新 Provider 前，如果上一 Provider 的 `[model.<key>]` 已被用户手动修改，后端会暂存并在新投影后恢复该 table。该保护对新旧 Provider 使用同名 key 的情况同样生效，并通过 `grok-config-warning` 将保留的 table key 返回页面提示，不能只写日志。

数据库为空时生成的 `__local__` Provider 仍会结构化读取模型表，但模型级 `api_key` 必须完全跳过，既不能进入 `auth.API_KEY`，也不能落入模型 `apiKey` 或 `extraConfig` 后暴露给前端。用户收编本地配置时需要在受控 API Key 输入框中重新确认凭据。

### 28.10 备份与 Session 完整性补充

Grok `plugins/**` 备份排除 `.git`、`node_modules`、cache、build、dist、target 等可重建目录；WalkDir 不跟随 symlink，恢复时还会拒绝目标相对路径中的现有 symlink component，避免插件条目借已有链接写出 Grok root。Unix 恢复后的 `auth.json` 和 `config.toml` 都收紧到 `0600`。

Grok native session snapshot 不再只保存可解码 UTF-8 的文件。文本文件保持字符串以便检查，非 UTF-8 文件使用 `{ encoding: "base64", data: "..." }` 保存并在导入时严格解码，因此 `subagents`、rewind 以及其他伴随状态中的二进制文件不会被静默丢弃。导入除了拒绝绝对路径、`..`、盘符和空路径段，还会拒绝 sessions root 或目标相对路径中的现有 symlink component，防止外部快照借链接写出 runtime root。

Session Manager 的单个与批量导出均提供三种明确格式：AI Toolbox JSON、官方 Grok Markdown、Grok 原生目录快照 JSON。官方 Markdown 通过 `grok export <session-id> <output>` 生成；已使用本机真实 session 验证命令退出码为 0 且输出文件非空。独立原生快照使用 `ai-toolbox.grok-native-snapshot.v1` schema，并有包含二进制 subagent 状态文件的导出→导入回归测试。自 Review 还补齐了运行环境边界：本机会话调用本机 Grok；WSL 会话进入对应 distro，使用 Linux `GROK_HOME`，并将 Windows 导出目标转换为 WSL 可访问路径。CLI 删除同样跟随会话来源，不能把 WSL UNC root 交给本机 Grok。

### 28.11 Codex 官方账号安全写回复核

Codex browser OAuth 新增了 1455 被占用时随机 loopback 端口、callback state mismatch 和 timeout 的直接测试；Device Code cancel 会发出取消信号并清理内存 session。官方账号应用到 runtime 时不再整段替换 `tokens`：access/refresh/id/account 等下一账号字段覆盖同名值，runtime 自己增加的未知 token metadata、根级未知字段和未被下一 snapshot 更新的 `last_refresh` 保留。原子 writer 有独立文件级回归，验证旧内容被替换且 Unix 权限为 `0600`。

- 官方能力矩阵中“不管理”的配置均经 fixture 证明不会被 writer 删除；
- 全量测试、TypeScript、build、i18n、fmt、diff check 全部通过；
- 新模块 `AGENTS.md` 和根索引已同步更新。

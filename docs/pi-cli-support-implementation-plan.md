# Pi CLI 支持实现计划

## 背景与研究来源

本计划用于在 AI Toolbox 中新增 Pi CLI 支持，使它在产品形态上尽量接近 OpenCode 的原生多渠道体验：可管理多个供应商和模型、设置默认 provider/model、维护全局 prompt、查看会话、接入托盘、备份恢复和 WSL/SSH 同步。

本计划基于以下来源整理：

- Pi 官方文档：
  - Quickstart: `https://pi.dev/docs/latest/quickstart`
  - Providers: `https://pi.dev/docs/latest/providers`
  - Settings: `https://pi.dev/docs/latest/settings`
  - Custom Models: `https://pi.dev/docs/latest/models`
  - Custom Providers: `https://pi.dev/docs/latest/custom-provider`
  - Using Pi / CLI reference: `https://pi.dev/docs/latest/usage`
  - Session File Format: `https://pi.dev/docs/latest/session-format`
  - Skills / Extensions: `https://pi.dev/docs/latest/skills`, `https://pi.dev/docs/latest/extensions`
- 当前项目：
  - 本仓库工作目录
  - 已阅读根 `AGENTS.md`，以及 Claude Code / Codex / OpenCode 前后端模块级 `AGENTS.md`

## 官方事实

这些结论必须作为实现的 Source of Truth，不能被现有其他 CLI 的实现习惯覆盖。

### 安装与命令

- Pi npm 包名是 `@earendil-works/pi-coding-agent`。
- 官方安装命令：

```bash
npm install -g --ignore-scripts @earendil-works/pi-coding-agent
```

- CLI 命令是 `pi`。
- 常用模式：
  - `pi`：交互模式。
  - `pi -p "..."` / `pi --print "..."`：一次性输出。
  - `pi --mode json`：JSON event stream。
  - `pi --mode rpc`：stdin/stdout RPC。
  - `pi -c` / `pi --continue`：继续最近会话。
  - `pi -r` / `pi --resume`：选择历史会话。
  - `pi --session <path|id>`：打开指定 session。
  - `pi --fork <path|id>`：fork session。
  - `pi --session-dir <dir>`：本次运行使用指定 session 存储目录。

### 运行时根目录

- 默认全局配置根目录是 `~/.pi/agent/`。
- `PI_CODING_AGENT_DIR` 可以覆盖配置目录。
- 卸载 Pi 不会删除 `~/.pi/agent/` 里的 settings、credentials、sessions、packages 等数据。

AI Toolbox 应把 Pi 归类为“根目录模块”，类似 Claude Code / Codex，而不是 OpenCode 那种“单配置文件路径模块”。

推荐路径解析优先级：

1. AI Toolbox 应用内 `root_dir`
2. 环境变量 `PI_CODING_AGENT_DIR`
3. shell 配置解析得到的 `PI_CODING_AGENT_DIR`
4. 默认 `~/.pi/agent`

第 3 点是 AI Toolbox 当前跨 CLI 的本地体验增强；第 2/4 点是官方语义，不能改名成 `PI_HOME` 或其他自造变量。

### 全局文件

默认根目录下的关键文件：

| 文件/目录 | 官方语义 | AI Toolbox 处理建议 |
|---|---|---|
| `settings.json` | 全局 settings。项目 `.pi/settings.json` 可覆盖全局设置，嵌套对象 merge | 管理全局设置；首版不管理项目设置 |
| `auth.json` | OAuth token 和 API key credentials | 作为 provider credential Source of Truth 读取和 patch；用户编辑哪个 key 就覆盖哪个 key |
| `models.json` | custom providers / models / provider overrides | 作为 provider registry Source of Truth 读取和 patch；允许覆盖官方内置 provider key |
| `AGENTS.md` | 全局 context file | 作为 Pi 全局 prompt 的首版运行时文件 |
| `SYSTEM.md` | 替换默认 system prompt | 首版可只展示/保留，不作为 GlobalPromptSettings 主流程 |
| `APPEND_SYSTEM.md` | 追加默认 system prompt | 首版可只展示/保留，不作为 GlobalPromptSettings 主流程 |
| `sessions/` | 自动保存的 JSONL sessions | 接入 Session Manager |
| `skills/` | 用户级 Pi skills | 首版接入 Skills 同步目标 |
| `extensions/` | 用户级 extensions | 首版只保留 settings 路径；后续接入 Packages / Extensions 管理 |
| `prompts/` | prompt templates | 后续接入资源管理 |
| `themes/` | terminal themes | 非首版 |
| `trust.json` | project trust decisions | 只备份恢复；不要在 provider 页面编辑 |
| `npm/` | 用户级 Pi packages 安装目录 | 只备份恢复可选；首版不做包管理 |

项目级 `.pi/settings.json`、`.pi/skills/`、`.pi/extensions/` 等受 project trust 控制，不应由全局 Pi 页面默认编辑。后续如果做项目级管理，需要先设计当前项目目录选择和 trust 语义。

### Provider 与凭据

Pi 原生支持多供应商 / 多渠道，整体更接近 OpenCode 的 provider registry，而不是 Claude Code/Codex 这种“切换后写成当前唯一渠道”的模型。

关键事实：

- `auth.json` 是按 provider key 存储的 credentials map，可以同时存在 `anthropic`、`openai`、`google`、`openrouter` 等多个 entry。
- `models.json.providers` 是 provider registry，可以同时注册多个 custom provider，也可以覆盖或扩展内置 provider。
- `settings.json.defaultProvider` / `defaultModel` 只是默认启动选择，不代表其他 provider 被禁用或不可用。
- Pi CLI 可通过 `/model`、`--provider`、`--model provider/id`、`enabledModels` 在多个 provider/model 之间选择。
- 因此 AI Toolbox 的 Pi provider 页面应是 Pi runtime 配置文件的可视化编辑器：`auth.json`、`models.json` 和 `settings.json.defaultProvider` 里现在出现了什么 provider key，就展示什么 provider key；用户编辑某个 provider key 时就原地覆盖该 key，不额外生成 `ai-toolbox-*` 包装 provider。
- Pi 官方内置 provider registry 也必须参与校验和选项构建。一个内置 provider 即使没有出现在 `auth.json` 或 `models.json`，也可能通过环境变量可用；不要把这种情况误判为 missing。只有 provider key 既不是官方内置 key、也不在 `auth.json`、也不在 `models.json.providers` 中时，才显示 `Missing runtime provider`。
- `settings.json.defaultProvider/defaultModel` 只标记默认选择，不代表其他 provider 被禁用或不可用。

Pi 支持两类供应商入口：

1. 订阅 / OAuth：通过交互模式 `/login`，内置包含：
   - ChatGPT Plus/Pro (Codex)
   - Claude Pro/Max
   - GitHub Copilot

2. API key：通过环境变量、`auth.json` 或 `/login` 录入 API key。

官方 credential 解析顺序：

1. CLI `--api-key`
2. `auth.json` entry，即 API key 或 OAuth token
3. 环境变量
4. `models.json` 中 custom provider 的 key

AI Toolbox 不能改变这个顺序。尤其是自定义供应商的 `models.json.apiKey` 在官方顺序里低于 `auth.json` 和环境变量；表单文案要明确这是 Pi 官方解析规则，不要承诺它一定压过环境变量。

`auth.json` API key entry 示例：

```json
{
  "anthropic": { "type": "api_key", "key": "sk-ant-..." },
  "openai": { "type": "api_key", "key": "sk-..." },
  "cloudflare-ai-gateway": {
    "type": "api_key",
    "key": "$CLOUDFLARE_API_KEY",
    "env": {
      "CLOUDFLARE_API_KEY": "...",
      "CLOUDFLARE_ACCOUNT_ID": "account-id",
      "CLOUDFLARE_GATEWAY_ID": "gateway-id"
    }
  }
}
```

`key`、`models.json.apiKey` 和 header value 都支持同一套 value resolution：

- `!command`：执行命令并用 stdout。
- `$ENV_VAR` / `${ENV_VAR}`：读取环境变量。
- `$$`：字面量 `$`。
- `$!`：字面量 `!`。
- 普通字符串：字面量，不是环境变量。例如 `MY_API_KEY` 是字面量，`$MY_API_KEY` 才是环境变量。

### models.json 自定义供应商

官方 custom models 文件是 `~/.pi/agent/models.json`。最小结构：

```json
{
  "providers": {
    "ollama": {
      "baseUrl": "http://localhost:11434/v1",
      "api": "openai-completions",
      "apiKey": "ollama",
      "models": [
        { "id": "llama3.1:8b" }
      ]
    }
  }
}
```

支持的 `api` 至少包含：

- `openai-completions`
- `openai-responses`
- `anthropic-messages`
- `google-generative-ai`

`custom-provider` extension API 还列出了更广的 API 类型，例如 `azure-openai-responses`、`openai-codex-responses`、`mistral-conversations`、`google-vertex`、`bedrock-converse-stream`。首版表单可以只提供官方 models 文档中最常用的四种，并允许高级 JSON 补充，避免过度设计。

Provider 字段：

| 字段 | 说明 | 首版表单建议 |
|---|---|---|
| `baseUrl` | API endpoint URL | 新增全新 provider 时通常必填；覆盖内置 provider 时可只覆盖 `baseUrl` / `headers` / `modelOverrides` 的一部分 |
| `api` | streaming API 类型 | 下拉；新增全新 provider 且定义 models 时应在 provider 或 model 层提供 |
| `apiKey` | literal/env/command | 新增全新 provider 且需要 fallback key 时填写；覆盖内置 provider 时可以留空继续使用 `auth.json` / env / OAuth |
| `headers` | 自定义 headers，value 同样支持解析语法 | 高级 JSON |
| `authHeader` | true 时自动加 `Authorization: Bearer` | 开关 |
| `models` | 模型列表。定义后 custom models 会 merge/upsert 到 provider；对内置 provider 来说不是必填 | 模型编辑器 |
| `modelOverrides` | 内置模型覆写 | 高级 JSON，首版不做结构化表单 |

Model 字段：

| 字段 | 必填 | 默认 | 说明 |
|---|---:|---|---|
| `id` | 是 | - | 真实 model id |
| `name` | 否 | `id` | 显示名和匹配辅助名；不会替代 footer/status 中的 id |
| `api` | 否 | provider api | 单模型覆盖 API |
| `baseUrl` | 否 | provider baseUrl | 单模型覆盖 endpoint |
| `reasoning` | 否 | `false` | 是否支持 extended thinking |
| `thinkingLevelMap` | 否 | omitted | Pi thinking level 到上游值的映射；`null` 表示不支持 |
| `input` | 否 | `["text"]` | `["text"]` 或 `["text", "image"]` |
| `contextWindow` | 否 | `128000` | context tokens |
| `maxTokens` | 否 | `16384` | max output tokens |
| `cost` | 否 | 全 0 | 每百万 token 成本 |
| `compat` | 否 | provider compat | OpenAI / Anthropic 兼容性开关 |
| `headers` | 否 | 无 | 单模型 headers |

重要 gotcha：

- `models.json` 在 `/model` 打开时会重新加载，编辑期间不一定需要重启 Pi；但 `settings.json` / `auth.json` / context files 的变更通常需要重启或 `/reload` 才能对当前会话生效。UI 提示要区分。
- 写 `models.json` 时只 patch 用户当前编辑的 provider key；删除也只能删除用户明确选择的 exact key。
- 如果用户用 `models.json` 覆写内置 provider，例如 `"anthropic": { "baseUrl": "..." }`，AI Toolbox 应直接展示这个 `anthropic` provider，并允许用户在原 key 上编辑保存。
- AI Toolbox 保存 custom/override provider 时应 upsert 到 `models.json.providers.<providerKey>`；`providerKey` 就是用户看到和编辑的运行时 key，例如 `openrouter`、`anthropic`、`local-ollama`，不要自动加任何 AI Toolbox 前缀。设置默认 provider/model 时才 patch `settings.json.defaultProvider/defaultModel`。
- 内置 provider override 和新增 custom provider 的校验规则不同：override 可以只包含 `baseUrl`、`headers`、`modelOverrides` 或 `models` 中的一部分；新增全新 provider 如果要出现在 `/model` 中，至少需要一个 `models[].id`，并且要能从 provider 或 model 层推导出 `api`。

### settings.json

全局 settings 文件位置：`~/.pi/agent/settings.json`。

项目 settings 文件位置：`.pi/settings.json`，会覆盖全局，嵌套对象 merge。

首版应支持这些全局字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `defaultProvider` | string | 默认 provider，例如 `anthropic` / `openai` / 自定义 provider id |
| `defaultModel` | string | 默认 model id |
| `defaultThinkingLevel` | string | `off` / `minimal` / `low` / `medium` / `high` / `xhigh` |
| `hideThinkingBlock` | boolean | 是否隐藏 thinking blocks |
| `thinkingBudgets` | object | thinking level 到 token budget 的映射 |
| `enabledModels` | string[] | Ctrl+P scoped model patterns |
| `theme` | string | `dark` / `light` / custom |
| `httpProxy` | string | 全局 HTTP/HTTPS proxy |
| `sessionDir` | string | session 存储目录，支持绝对/相对/`~` |
| `defaultProjectTrust` | string | `ask` / `always` / `never`，global only |
| `packages` | array | Pi packages |
| `extensions` | string[] | extension paths |
| `skills` | string[] | skill paths |
| `prompts` | string[] | prompt template paths |
| `themes` | string[] | theme paths |
| `enableSkillCommands` | boolean | 是否注册 `/skill:name` |
| `markdown.codeBlockIndent` | string | code block indentation |

首版不建议做完整 settings 大表单。更稳妥的实现：

- 设置默认 provider 只 patch `defaultProvider`、`defaultModel`、可选 `defaultThinkingLevel`。
- Other Configuration 提供结构化少量字段 + “其他设置 JSON”。
- 保存时结构化 merge，保留未知字段和嵌套对象。
- 清空 optional 字段要显式删除对应 runtime 字段，不能 truthy 判断导致旧值残留。

### Session 格式

默认位置：

```text
~/.pi/agent/sessions/--<path>--/<timestamp>_<uuid>.jsonl
```

`<path>` 是工作目录路径，把 `/` 替换成 `-`。

Session 是 JSONL，每行一个 JSON object。当前版本 v3：

- 第一行 `type: "session"`，包含 `version`、`id`、`timestamp`、`cwd`，可选 `parentSession`。
- 后续 entry 通过 `id` / `parentId` 形成树。
- `type: "message"` 的 `message` 字段包含 AgentMessage。
- 还有 `model_change`、`thinking_level_change`、`compaction`、`branch_summary`、`custom`、`custom_message`、`label`、`session_info` 等 entry。

Message roles：

- `user`
- `assistant`
- `toolResult`
- `bashExecution`
- `custom`
- `branchSummary`
- `compactionSummary`

首版 Session Manager 要能：

- 列出 sessions。
- 解析 `session` header 得到 `cwd`、session id、创建时间。
- 从 `message` / `session_info` 推导标题。
- 构建当前主分支消息列表。第一版可以按文件顺序展示所有 entry，后续再做 tree navigator；但导出/恢复要保留原生 JSONL snapshot。
- resume command 使用 `pi --session <path|id>`，并按当前仓库已有 `build_resume_command` 逻辑加 `cd` / `pushd`。

`sessionDir` 解析建议：

1. 如果后端读取到 `PI_CODING_AGENT_SESSION_DIR`，用它。
2. 否则读取全局 `settings.json.sessionDir`。
3. 否则使用 `<piRoot>/sessions`。

CLI `--session-dir` 只影响某次命令运行，AI Toolbox 页面无法可靠读取过去所有一次性运行参数；只在 session 文件本身出现时被动展示，不作为全局路径来源。

## 产品范围

### 首版目标

首版目标是“官方配置完整匹配 + AI Toolbox 基础体验闭环”：

1. Pi 页面出现在 Coding tab。
2. 支持 Pi root directory 管理。
3. 支持读取/保存全局 `settings.json` 中 Pi 官方字段。
4. 支持原生多供应商管理：
   - 内置 API key provider：读取/写入 `auth.json.<providerKey>`，providerKey 原样来自 runtime 文件或用户输入。
   - 自定义/覆写 provider：读取/写入 `models.json.providers.<providerKey>`，允许覆盖官方内置 provider key，例如 `openrouter`、`anthropic`。
   - 订阅 / OAuth provider：识别 `auth.json` 现有 token，可设为默认；首版不从 AI Toolbox 内发起 `/login`。
   - 同时展示多个 provider；编辑一个 provider 只覆盖该 key，删除 provider 只删除用户明确选择的 entry。
5. 支持模型列表编辑、获取模型、连通性测试。
6. 支持 Global Prompt：管理全局 `AGENTS.md`。
7. 支持 Session Manager：列出/查看/删除/导出 Pi JSONL sessions。
8. 支持托盘 provider / prompt 快捷切换。
9. 支持备份恢复 Pi 全局配置。
10. 支持 WSL/SSH 同步 Pi 全局配置；Skills 走现有 Skills 独立同步链路。
11. Skills 作为 Pi 官方资源目录接入现有 Skills 中央仓库同步目标。

### 非目标

这些不要塞进首版：

- 不在 AI Toolbox 内实现 Pi `/login` 的 OAuth browser/device flow。先识别并保留 `auth.json` 中已有 OAuth credential。
- 不做 Pi project `.pi/settings.json` 编辑。
- 不做完整 Pi extension/package marketplace 管理。
- 不接入 AI Toolbox Gateway。Pi 原生支持多渠道和模型选择，首版只做渠道/模型管理，不做 single/failover takeover。
- 不接入 MCP。Pi 官方没有 Claude/Codex/OpenCode 那种 MCP runtime 配置文件；本计划不新增 MCP target、不做 bridge extension、不在 MCP 页面显示 Pi。
- 不实现 Pi custom streaming `streamSimple` 编辑器。
- 不直接操作 project trust 决策。
- 不实现 Pi TUI 内所有 slash commands。

## 数据模型设计

### Rust DbTable

新增表：

- `pi_settings_config`
- `pi_prompt_config`

不新增 `pi_provider` 主表。Pi 的 provider Source of Truth 是官方 runtime 文件：

- API key / OAuth provider 来自 `<root>/auth.json`。
- custom / override provider 来自 `<root>/models.json.providers`。
- 默认 provider/model 来自 `<root>/settings.json`。

这点要和 OpenCode 保持一致：页面展示的是当前配置文件内容，不是 AI Toolbox 数据库里的渠道库。SQLite 只保存 root path、少量 UI 辅助状态、prompt presets 等，不保存一套需要再投影到 Pi runtime 的 provider 主数据。

暂不新增 `pi_official_account`。原因：

- Pi 的订阅/OAuth entry 都存于 `auth.json`。
- 官方 `/login` 流程首版不在 AI Toolbox 内实现。
- 如果首版把 OAuth token 复制入数据库，反而扩大敏感数据面。

如果后续需要像 Codex 一样管理多个 official account，再新增 `pi_official_account`，但应先确认 Pi OAuth credential schema 的稳定性。

修改点：

- `tauri/src/db/schema.rs`
  - `DbTable::PiSettingsConfig`
  - `DbTable::PiPromptConfig`
  - `ALL_TABLES`
  - `name()`
- `tauri/src/db/migrations.rs`
  - 如果当前版本直接追加表，需 `TARGET_SCHEMA_VERSION += 1`
  - `migrate_v6` 调 `create_jsonb_table` 新表
  - 为 `pi_prompt_config.is_applied`、`pi_prompt_config.sort_index` 建 JSON index

### TypeScript types

新增 `web/types/pi.ts`：

```ts
export type PiProviderCategory = 'subscription' | 'api_key' | 'custom';

export type PiProviderSource = 'official_builtin' | 'auth_json' | 'models_json' | 'settings_json';

export type PiCredentialKind = 'api_key' | 'oauth' | 'env_possible' | 'none';

export type PiProviderWarning =
  | 'missing_provider'
  | 'missing_credential'
  | 'parse_partial';

export type PiApiType =
  | 'openai-completions'
  | 'openai-responses'
  | 'anthropic-messages'
  | 'google-generative-ai';

export interface PiApiKeyCredential {
  type: 'api_key';
  key: string;
  env?: Record<string, string>;
}

export interface PiOAuthCredential {
  refresh?: string;
  access?: string;
  expires?: number;
  [key: string]: unknown;
}

export interface PiProviderModelCost {
  input?: number;
  output?: number;
  cacheRead?: number;
  cacheWrite?: number;
}

export interface PiProviderModelConfig {
  id: string;
  name?: string;
  api?: PiApiType | string;
  baseUrl?: string;
  reasoning?: boolean;
  thinkingLevelMap?: Partial<Record<'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh', string | null>>;
  input?: Array<'text' | 'image'>;
  contextWindow?: number;
  maxTokens?: number;
  cost?: PiProviderModelCost;
  headers?: Record<string, string>;
  compat?: Record<string, unknown>;
}

export interface PiModelsProviderConfig {
  baseUrl?: string;
  api?: PiApiType | string;
  apiKey?: string;
  headers?: Record<string, string>;
  authHeader?: boolean;
  models?: PiProviderModelConfig[];
  modelOverrides?: Record<string, Partial<PiProviderModelConfig>>;
  compat?: Record<string, unknown>;
}

export interface PiDefaultSelection {
  model?: string;
  thinkingLevel?: 'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | string;
}

export interface PiSettingsConfig {
  rootDir?: string | null;
  uiState?: {
    otherConfigExpanded?: boolean;
    lastViewedSection?: 'model' | 'providers' | 'other' | 'prompt' | 'sessions';
  };
  updatedAt?: string;
}

export interface PiRuntimeProviderView {
  providerKey: string;
  name: string;
  sources: PiProviderSource[];
  categories: PiProviderCategory[];
  credentialKind: PiCredentialKind;
  credential?: PiApiKeyCredential | PiOAuthCredential;
  modelsProvider?: PiModelsProviderConfig;
  defaultSelection?: PiDefaultSelection;
  runtimeFiles: Array<'auth.json' | 'models.json' | 'settings.json'>;
  meta?: Record<string, unknown>;
  isBuiltin?: boolean;
  isOverride?: boolean;
  isDefault?: boolean;
  warnings?: PiProviderWarning[];
}
```

说明：

- `providerKey` 是 Pi 运行时 provider id，例如 `anthropic`、`openai`、`openrouter`、`local-ollama`。它直接对应 `auth.json.<providerKey>` 或 `models.json.providers.<providerKey>`，不要自动加 AI Toolbox 前缀。
- `PiRuntimeProviderView` 是后端从 `settings.json`、`auth.json`、`models.json` 解析出的页面视图，不是数据库 provider 记录。
- `sources` 用来表达该 key 来自哪里。同一个 key 可以同时来自 `auth_json`、`models_json` 和 `settings_json`；官方内置 key 额外标记 `official_builtin`。
- `categories` 是表单/操作分类。同一个 key 可以同时有 `api_key` credential 和 `custom` provider config，因此这里用数组而不是单值。
- `subscription` 表示 Pi 内置 OAuth/订阅 provider。首版只展示已有 auth 状态，不创建 OAuth token。
- `api_key` 表示该 key 在 `auth.json` 中有 API key credential，凭据写 `auth.json.<providerKey>`。
- `custom` 表示该 key 在 `models.json.providers` 中有 custom/override provider config。
- `credentialKind: 'env_possible'` 表示该 key 是官方内置 provider，但 runtime 文件中没有保存 credential；Pi 仍可能通过环境变量使用它，不能显示成 missing。
- `isOverride` 表示 `models.json.providers.<providerKey>` 覆盖了官方内置 provider；`isBuiltin` 表示 providerKey 存在于官方内置 provider registry。
- `isDefault` 只表示该 provider 对应当前 `settings.json.defaultProvider`。它不是“已应用/唯一生效”的含义；Pi runtime 可以同时保留多个 provider。
- `warnings: ['missing_provider']` 只用于 providerKey 既不是官方内置 key、也不在 `auth.json`、也不在 `models.json.providers` 中的情况。

### 数据库存储 payload 细节

AI Toolbox 的 Pi provider Source of Truth 是 runtime 文件。数据库只保存 root/UI 辅助记录和 prompt presets，不保存 provider 主数据。不要引入第二套 profile/provider 主数据，也不要引入 `pi_provider` 表来再投影到 runtime。

`pi_settings_config` 的 payload：

```json
{
  "id": "__default__",
  "rootDir": null,
  "uiState": {
    "otherConfigExpanded": false,
    "lastViewedSection": "providers"
  },
  "updatedAt": "2026-06-20T00:00:00.000Z"
}
```

`pi_prompt_config` 的 payload 对齐现有 Global Prompt 数据模型：

```json
{
  "id": "uuid",
  "name": "Default",
  "content": "Project-independent instructions for Pi.",
  "sortIndex": 0,
  "isApplied": true,
  "createdAt": "2026-06-20T00:00:00.000Z",
  "updatedAt": "2026-06-20T00:00:00.000Z"
}
```

不新增这些表：

| 表 | 首版不新增原因 |
|---|---|
| `pi_official_account` | 首版不实现 `/login` OAuth 流，也不复制 OAuth token 入库 |
| `pi_mcp_config` | 本计划不接入 MCP；Pi 官方也没有 MCP runtime 配置文件 |
| `pi_extension_config` | 首版只把 `settings.json.extensions/packages` 当 Other Configuration 资源字段处理，不做包安装、更新、信任和 node_modules 管理 |

敏感字段处理：

- `auth.json` 和 `models.json` 允许保存用户输入的 API key literal；UI 展示必须 mask。
- 如果用户使用 `$ENV`，数据库保存 `$ENV` 引用，不解析环境变量值。
- 获取模型和连通性测试时不执行 `!command` 型 key；后续若支持，必须加显式确认和审计日志。

## 文件投影规则

### root path resolver

后端新增 `tauri/src/coding/pi/`，Pi 按根目录模块处理。

新增函数建议：

- `get_pi_root_dir_from_settings_config_async(db)`
- `get_pi_root_path_info(db)`
- `get_pi_root_dir_async(db)`
- `get_pi_settings_path_async(db)` -> `<root>/settings.json`
- `get_pi_auth_path_async(db)` -> `<root>/auth.json`
- `get_pi_models_path_async(db)` -> `<root>/models.json`
- `get_pi_prompt_path_async(db)` -> `<root>/AGENTS.md`
- `get_pi_sessions_root_async(db)` -> `<root>/sessions` 或 settings/env 覆盖

同步改 `tauri/src/coding/runtime_location.rs`：

- `MODULE_KEYS` 加 `pi`
- `normalize_module_key` 支持 `pi`
- `refresh_runtime_location_cache_for_module_async` 支持 Pi
- 导出 `get_pi_runtime_location_async`
- WSL Direct 状态返回 `module: "pi"`

### settings.json merge

读取：

- 不存在时返回 `{}`，不要报错。
- JSON parse 失败时返回明确 parse error，前端显示可修复提示。

写入：

- 使用 `serde_json::Map` patch，不要整文件重建。
- 只 patch 用户在 Pi 页面明确编辑的字段：
  - 设置默认 provider/model：`defaultProvider`、`defaultModel`、可选 `defaultThinkingLevel`。
  - Other Configuration：用户在 Other Configuration section 中编辑的 Pi 官方 settings 字段。
- 未知字段保留。
- 用户清空字段时要从 JSON 中 remove，而不是跳过提交。

### auth.json merge

读取：

- 不存在时 `{}`。
- 展示 OAuth/API key credential 概览时必须 mask key/token。

写入内置 API key provider：

```json
{
  "<providerKey>": {
    "type": "api_key",
    "key": "$ENV_OR_LITERAL",
    "env": {
      "OPTIONAL_PROVIDER_ENV": "value"
    }
  }
}
```

保留：

- 所有非当前 `providerKey` entry。
- 当前 `providerKey` 下未在表单里展示/修改的 unknown 字段，除非用户在 Advanced JSON 中明确删除或覆盖。
- OAuth token fields，例如 `refresh`、`access`、`expires`，不要因为用户切换 custom provider 就删除。

保存 API key provider 时：

- upsert 当前 `auth.json.<providerKey>`。
- 如果该 key 当前是 OAuth credential，必须弹确认：确认后才允许把它替换成 `type: "api_key"`；取消则不写入。
- 不写 `x-ai-toolbox`、owner、managedBy 这类自定义 metadata。

保存 custom/override provider 时：

- 不写 `auth.json`，除非该 provider 明确绑定某个内置 provider credential。
- 只写 `models.json.providers.<providerKey>`。只有用户在顶部 Model Settings 保存默认选择时，才 patch `settings.json.defaultProvider/defaultModel/defaultThinkingLevel`。

### models.json merge

写 custom provider：

- provider id 使用用户正在编辑的运行时 key，例如 `openrouter`、`anthropic`、`local-ollama`。不要自动改写成 `ai-toolbox-<stableSlug>`。
- 如果 provider key 与 Pi 内置 provider 相同，就是对该内置 provider 的 override；AI Toolbox 要按用户意图原地覆盖这个 key。
- 保存 provider 时只 upsert 当前 `models.json.providers.<providerKey>`，其他 provider 原样保留。

```json
{
  "providers": {
    "openrouter": {
      "baseUrl": "https://openrouter.ai/api/v1",
      "api": "openai-completions",
      "apiKey": "$OPENROUTER_API_KEY",
      "models": [
        {
          "id": "anthropic/claude-sonnet-4.5",
          "name": "Claude Sonnet 4.5 (OpenRouter)",
          "reasoning": true,
          "input": ["text", "image"],
          "contextWindow": 200000,
          "maxTokens": 32000,
          "cost": {
            "input": 3,
            "output": 15,
            "cacheRead": 0.3,
            "cacheWrite": 3.75
          }
        }
      ]
    }
  }
}
```

保存、删除和重命名规则：

- 保存当前 custom provider 时覆盖同名 provider key。
- 如果用户在 UI 中重命名 provider key，应明确提示这是“移动/重命名 runtime provider”：写入新 key，并按用户选择的 scope 删除旧 key。
- 不区分 AI Toolbox 新增的 provider 和运行时文件里已有的 provider。页面展示从 `auth.json`、`models.json.providers`、`settings.defaultProvider` 合并出的 provider view；`models.json.providers` 中所有 entry 必须展示，`auth.json` 中所有 credential entry 也必须展示，同 key 合并为一张卡片。
- 同一个 provider key 同时存在 `auth.json.<key>` 和 `models.json.providers.<key>` 时，Delete 必须让用户选择 scope：
  - `Delete credential from auth.json`
  - `Delete provider config from models.json`
  - `Delete both runtime entries`
- Rename 也必须让用户选择 scope：
  - `Rename credential key`
  - `Rename models provider key`
  - `Rename both`
- 如果 renamed key 等于 `settings.json.defaultProvider`，弹确认并同步 patch `settings.json.defaultProvider`；如果用户删除的是当前 default provider，不自动清空 default，但保存后必须返回 `Missing runtime provider` warning，提示用户到顶部 Model Settings 重新选择。
- 不把 `models.json` 中的 unknown top-level 字段清掉。

### 最终 Pi runtime 配置示例

下面是一个完整例子，用来说明 AI Toolbox 最终写入 Pi runtime 的文件长什么样。

场景：

- Pi root 是默认 `~/.pi/agent`。
- 用户在 AI Toolbox 新增或编辑并保存了内置 API key provider：`anthropic`。
- 用户在 AI Toolbox 新增或编辑了 custom/override provider：`openrouter`。这个 key 与 Pi 内置 provider key 相同，因此最终会覆盖/扩展 Pi 内置 `openrouter` provider。
- 用户在顶部 Model Settings 中把默认 provider 设为 `openrouter`，默认模型设为 `anthropic/claude-sonnet-4.5`。
- 用户在 Other Configuration 中设置了 `enabledModels`、`theme`、`httpProxy`、`sessionDir`、`defaultProjectTrust` 和资源路径。
- Pi runtime 里原本已经有一个 GitHub Copilot OAuth credential 和一个已有 custom provider；AI Toolbox 必须保留它们，除非用户明确编辑或删除对应 exact key。

最终 `<piRoot>/settings.json`：

```json
{
  "defaultProvider": "openrouter",
  "defaultModel": "anthropic/claude-sonnet-4.5",
  "defaultThinkingLevel": "medium",
  "enabledModels": [
    "anthropic/*",
    "openrouter/*",
    "github-copilot/*"
  ],
  "theme": "dark",
  "httpProxy": "http://127.0.0.1:7890",
  "sessionDir": "sessions",
  "defaultProjectTrust": "ask",
  "packages": [],
  "extensions": [
    "./extensions"
  ],
  "skills": [
    "./skills",
    "~/.claude/skills"
  ],
  "prompts": [
    "./prompts"
  ],
  "enableSkillCommands": true,
  "compaction": {
    "enabled": true,
    "reserveTokens": 16384,
    "keepRecentTokens": 20000
  },
  "retry": {
    "enabled": true,
    "maxRetries": 3,
    "baseDelayMs": 2000,
    "provider": {
      "maxRetries": 0,
      "maxRetryDelayMs": 60000
    }
  }
}
```

字段来源：

- `defaultProvider/defaultModel/defaultThinkingLevel` 来自顶部 Model Settings。
- `enabledModels/theme/httpProxy/sessionDir/defaultProjectTrust/packages/extensions/skills/prompts/enableSkillCommands/compaction/retry` 来自 Other Configuration。
- 设置默认 provider 只 patch `settings.json`，不会删除 `auth.json` 或 `models.json` 里的其他 provider。

最终 `<piRoot>/auth.json`：

```json
{
  "anthropic": {
    "type": "api_key",
    "key": "$ANTHROPIC_API_KEY",
    "env": {
      "ANTHROPIC_API_KEY": "sk-ant-..."
    }
  },
  "github-copilot": {
    "refresh": "runtime-owned-refresh-token",
    "access": "runtime-owned-access-token",
    "expires": 1780000000000
  },
  "openai": {
    "type": "api_key",
    "key": "$OPENAI_API_KEY"
  }
}
```

说明：

- `anthropic` 是用户在 AI Toolbox 里编辑 API key provider 时写入或更新的 runtime entry。
- `auth.json` 不写 `x-ai-toolbox` 这类自定义 owner metadata，也不在 SQLite 里另存 provider 所有权。页面重新读取文件即可还原当前 provider 状态。
- `github-copilot` 是 Pi `/login` 产生的 OAuth credential，AI Toolbox 只展示状态，不复制 token 入 DB，也不在切换默认 provider 时删除。
- `openai` 假设是用户原本手写或 Pi `/login` 写入的 API key credential。只要用户没有在 AI Toolbox 中明确编辑 `openai`，就必须原样保留；如果用户编辑 `openai`，保存会覆盖这个 exact key。
- Unix 下恢复或创建 `auth.json` 时尽量保持 `0600`。

最终 `<piRoot>/models.json`：

```json
{
  "providers": {
    "openrouter": {
      "name": "OpenRouter",
      "baseUrl": "https://openrouter.ai/api/v1",
      "api": "openai-completions",
      "apiKey": "$OPENROUTER_API_KEY",
      "headers": {
        "HTTP-Referer": "https://github.com/coulsontl/ai-toolbox",
        "X-Title": "AI Toolbox"
      },
      "models": [
        {
          "id": "anthropic/claude-sonnet-4.5",
          "name": "Claude Sonnet 4.5",
          "reasoning": true,
          "thinkingLevelMap": {
            "minimal": null,
            "low": "low",
            "medium": "medium",
            "high": "high",
            "xhigh": "max"
          },
          "input": ["text", "image"],
          "contextWindow": 200000,
          "maxTokens": 32000,
          "cost": {
            "input": 3,
            "output": 15,
            "cacheRead": 0.3,
            "cacheWrite": 3.75
          },
          "compat": {
            "supportsDeveloperRole": false,
            "supportsReasoningEffort": true
          }
        },
        {
          "id": "openai/gpt-5.1",
          "name": "GPT 5.1",
          "reasoning": true,
          "input": ["text", "image"],
          "contextWindow": 400000,
          "maxTokens": 128000,
          "cost": {
            "input": 1.25,
            "output": 10,
            "cacheRead": 0.125,
            "cacheWrite": 1.25
          }
        }
      ]
    },
    "local-ollama": {
      "baseUrl": "http://localhost:11434/v1",
      "api": "openai-completions",
      "apiKey": "ollama",
      "models": [
        {
          "id": "llama3.1:8b",
          "name": "Llama 3.1 8B"
        }
      ]
    }
  }
}
```

说明：

- `openrouter` 是 `models.json.providers.openrouter` 的 runtime entry。因为 key 与 Pi 内置 provider 相同，它会覆盖/扩展 Pi 内置 OpenRouter provider。
- `local-ollama` 是 `models.json.providers.local-ollama` 的另一个 runtime entry。AI Toolbox 保存 `openrouter` 时不能删除或改写它；但如果用户在页面上明确编辑 `local-ollama`，保存就会原地覆盖 `local-ollama`。
- Pi 官方支持在 `models.json.providers` 里同时存在多个 custom provider；这就是 Pi 原生多渠道的最终 runtime 形态。
- 这里不写 `ai-toolbox-gateway`，因为 Pi 首版不接入 AI Toolbox Gateway。

最终 `<piRoot>/AGENTS.md`：

```md
# Global Instructions

- Prefer concise, direct answers.
- Before editing code, inspect the relevant files.
- Preserve user-owned configuration unless explicitly asked to migrate it.
```

可选 `<piRoot>/SYSTEM.md` / `<piRoot>/APPEND_SYSTEM.md`：

首版不通过普通 Global Prompt 主流程写这两个文件，但备份恢复要保留它们。如果后续做高级 system prompt 管理，示例形态是：

```md
# SYSTEM.md

You are Pi running with configuration edited from AI Toolbox.
```

```md
# APPEND_SYSTEM.md

Follow repository instructions and keep changes minimal.
```

这个示例对应的用户操作和文件变化关系：

| 用户操作 | 写入文件 | 关键变化 |
|---|---|---|
| 保存 `anthropic` API key provider | `auth.json` | upsert `auth.anthropic` |
| 保存 `openrouter` custom/override provider | `models.json` | upsert `providers.openrouter` |
| 设置默认 provider/model | `settings.json` | patch `defaultProvider/defaultModel/defaultThinkingLevel` |
| 编辑 Other Configuration | `settings.json` | patch `enabledModels/theme/httpProxy/sessionDir/resources/...` |
| 应用 Global Prompt | `AGENTS.md` | 覆盖全局 prompt 文件 |

### prompt 文件

首版 Global Prompt 使用 `<root>/AGENTS.md`。

Pi 官方还会读取：

- 全局 `~/.pi/agent/AGENTS.md`
- cwd 父目录链路中的 `AGENTS.md` 或 `CLAUDE.md`

AI Toolbox 的 Pi Global Prompt 只管理全局 `AGENTS.md`。文案要避免误导用户以为项目 `AGENTS.md` 会被修改。

后续可增加 system prompt 高级区：

- `SYSTEM.md`
- `APPEND_SYSTEM.md`

这两个不应混进现有 GlobalPromptSettings 的普通 prompt 列表，除非设计明确区分“context instructions”和“system prompt replacement/append”。

## 前端页面设计

新增目录：

```text
web/features/coding/pi/
├── AGENTS.md
├── index.ts
├── pages/PiPage.tsx
├── pages/PiPage.module.less
├── components/PiModelSettingsCard.tsx
├── components/PiProviderFormAdapter.tsx
├── components/PiOtherConfigPanel.tsx
├── components/PiSettingsModal.tsx
├── components/ImportConflictDialog.tsx
├── components/ImportFromAllApiHubModal.tsx
├── hooks/usePiConfigState.ts
└── utils/localProvider.ts
```

### 页面结构

页面骨架沿用 Codex 的“根目录模块”结构，但 provider/model 语义按 OpenCode 式原生多渠道处理：

- 顶部路径区：
  - 标题：Pi
  - docs 链接：`https://pi.dev/docs/latest/quickstart`
  - root path info：`~/.pi/agent` 或自定义根
  - 自定义根目录按钮
  - 打开文件夹
  - 刷新
- Section：
  - Model Settings
  - Providers
  - Other Configuration
  - Global Prompt
  - Sessions
  - 后续：Packages / Skills / Extensions

### 页面视觉与交互规格

Pi 页面不是 landing page，也不要做 hero 区。它应当是和现有 OpenCode 页面一致的配置工作台：信息密度适中、可快速扫描 provider 状态、主操作明确，所有颜色使用 Ant Design token 或仓库 CSS 变量，不写硬编码色值。

优先复用 OpenCode 现有组件和样式，不为 Pi 单独发挥一套视觉语言：

- Model Settings 顶部卡片复用 OpenCode `modelCard` / `modelCardTitle` / `modelCardContent` 结构。实现时优先把这套样式抽到 shared coding card 样式或共享组件；如果短期无法抽离，Pi 的 Less 必须逐项对齐 OpenCode，不新增更大的圆角、阴影或不同间距。
- Provider 列表复用 OpenCode 的 `Collapse` + `collapseCard` 结构。
- Other Configuration 复用 OpenCode 的 `Collapse` + `JsonEditor` 结构，但 Pi 需要在折叠内容顶部提供结构化字段分组，再用 Advanced JSON 兜底。
- Provider 条目优先复用 `@/components/common/ProviderCard`，通过 adapter 把 Pi provider/model 映射成 `ProviderDisplayData` / `ModelDisplayData`。只有 Pi 的 OAuth/custom override/default 状态无法表达时，才给 `ProviderCard` 增加通用扩展点，不新增 `PiProviderCard` 自己画。
- Provider 表单优先复用 `@/components/common/ProviderFormModal`；模型编辑优先复用 `ModelFormModal` 和 `FetchModelsModal`。Pi 只增加 adapter / props，不复制一份 modal。
- 官方/订阅 credential 展示优先复用 `OfficialProviderCard` 的信息密度和状态布局。

#### 模型预设复用边界

Pi 的 custom provider 模型编辑器可以复用 OpenCode 已经使用的通用预设模型能力，但只能把它当作“新增模型时的候选模板”，不能把 OpenCode 的模型契约变成 Pi 的 Source of Truth：

- 可复用 `@/constants/presetModels` 的 `PRESET_MODELS`、`findPresetModelById`、版本订阅和远端刷新链路。它是跨工具通用模型预设，适合给 `ModelFormModal` / `FetchModelsModal` 做候选列表、能力字段填充和新增模型模板。
- 可复用 OpenCode 页面里的 `buildOpenCodeModelFromPreset` 思路，但需要新增 `buildPiModelFromPreset(preset)` adapter，把字段映射到 Pi 官方 `models.json.providers.<providerKey>.models[]` schema：
  - `id` -> `id`
  - `name` -> `name`
  - `contextLimit` -> `contextWindow`
  - `outputLimit` -> `maxTokens`
  - `modalities` 包含 image 时 -> `input: ["text", "image"]`
  - `reasoning` -> `reasoning`
  - `cost` / `options` / `variants` 不能照搬；只有 Pi 官方 schema 有对应字段时才写入，否则放进 model advanced JSON 或丢弃。
- 不能直接复用 OpenCode `models.dev.json` / `getOpenCodeUnifiedModels` 作为 Pi 模型列表事实源。`models.dev.json` 是 OpenCode 默认模型数据，包含 OpenCode 专属 provider、free model、experimental modes 展开、`provider_id/model_id` 选择语义；Pi 的官方内置 provider/model 列表应以 Pi 官方内置 registry 和 runtime 文件为准。
- Pi 顶部 `defaultModel` 保存的是 Pi settings 字段。下拉展示时可以用 `providerKey / modelId` 形式便于搜索，但选项内部必须保留 `{ providerKey, modelId }` 两段；写回时按 Pi 官方语义分别保存：`defaultProvider` 写 provider key，`defaultModel` 写 model id。注意 model id 自身可能包含 `/`，例如 OpenRouter 的 `anthropic/claude-sonnet-4.5`；不要额外把 provider key 拼进 `settings.json.defaultModel`，也不要照搬 OpenCode 的完整 `provider_id/model_id` 存储规则。
- 从预设创建 custom provider model 时，只写当前用户编辑的 `models.json.providers.<providerKey>.models[]`，不生成 `ai-toolbox-*` provider key，不把预设模型写进数据库 provider 表，也不自动覆盖 Pi 官方内置 provider。

#### 桌面首屏布局

桌面端建议为单列工作台，不做左右分栏。宽度、间距跟现有 Coding 子页面保持一致：

```text
┌────────────────────────────────────────────────────────────────────┐
│ Pi                                             [Docs] [Open] [Root] │
│ ~/.pi/agent · default provider: anthropic · 3 runtime providers     │
├────────────────────────────────────────────────────────────────────┤
│ Model Settings                                                      │
│ ┌────────────────────────────────────────────────────────────────┐ │
│ │ Default Provider                                                │ │
│ │ [Provider: Anthropic        v] [Model: claude-sonnet...      v] │ │
│ │ [Thinking: medium           v]                  [Save default]  │ │
│ └────────────────────────────────────────────────────────────────┘ │
├────────────────────────────────────────────────────────────────────┤
│ Providers                                          [+ Add] [Refresh]│
│ ┌────────────────────────────────────────────────────────────────┐ │
│ │ Anthropic                      API Key · Default · 4 models     │ │
│ │ claude-sonnet-4-20250514       auth.json · key: $ANTHROPIC...  │ │
│ │ [Edit] [Fetch models] [Test] [More]                            │ │
│ └────────────────────────────────────────────────────────────────┘ │
│ ┌────────────────────────────────────────────────────────────────┐ │
│ │ OpenRouter                     Custom override · 12 models     │ │
│ │ https://openrouter.ai/api/v1   models.json · openai-compatible │ │
│ │ [Edit] [Fetch models] [Test] [More]                            │ │
│ └────────────────────────────────────────────────────────────────┘ │
├────────────────────────────────────────────────────────────────────┤
│ Other Configuration                                      collapsed  │
├────────────────────────────────────────────────────────────────────┤
│ Global Prompt                                                      │
│ Sessions                                                           │
└────────────────────────────────────────────────────────────────────┘
```

布局细节：

- 页面背景使用 `--color-bg-layout`，内容容器使用现有页面最大宽度和 padding，不额外加装饰背景。
- 顶部路径区使用紧凑 header：左侧是 `Pi` 标题和 root path meta，右侧是 icon + text 按钮。按钮包括 Docs、Open folder、Root directory、Refresh。
- root path meta 一行展示：当前 root、路径来源、默认 provider/model、provider 数量。路径过长时中间省略，hover tooltip 显示完整路径。
- Model Settings、Providers、Other Configuration、Global Prompt、Sessions 是同一页面内的连续 section。Model Settings 用 OpenCode 顶部 model settings card；Providers 和 Other Configuration 用 OpenCode collapse card；不要再额外套一层 page-level 大卡片。
- 卡片圆角、阴影、标题 padding、内容 padding 以 OpenCode 当前 `modelCard` / `collapseCard` 为准。不要用渐变、发光边框或装饰图形。

#### Model Settings 顶部卡片

Pi 的 `defaultProvider/defaultModel/defaultThinkingLevel` 不放在 provider 卡片里，也不作为 provider card 的“当前应用”状态。它应像 OpenCode 的 Model Settings 一样，作为独立卡片放在 provider 列表上方。

卡片标题：

- 标题文案：`Model Settings` 或 `Default Model`，不要叫 `Common Config`。
- 左侧 icon 复用 OpenCode 的 `RobotOutlined` 或同类模型设置图标。

卡片内容使用和 OpenCode `modelSettings` 相同的纵向字段结构：

| 字段 | 控件 | 数据来源 | 保存行为 |
|---|---|---|---|
| Default provider | Select | 官方内置 provider registry + 从 `auth.json` / `models.json.providers` / `settings.defaultProvider` 合并出的 runtime provider view；如果 settings 指向未知 key，也显示为 warning option | 保存后只 patch `settings.json.defaultProvider` |
| Default model | Select | 选中 provider 的 models；允许手输 | 保存后 patch `settings.json.defaultModel` |
| Thinking level | Select | `off/minimal/low/medium/high/xhigh` | 保存后 patch 或删除 `settings.json.defaultThinkingLevel` |

交互规则：

- 顶部卡片保存按钮命名为 `Save default`，不要叫 `Apply provider`。
- 改 Default provider 不自动删除、禁用或重写其他 provider；只更新 settings 默认选择。
- 如果默认 provider 是官方内置 key，但没有 `auth.json` credential 或 `models.json` override，不显示 missing；显示 `Built-in · credential from env or /login` 或 `Built-in · no stored credential`。
- 如果默认 provider 既不是官方内置 key、也不在 `auth.json` 或 `models.json.providers` 中，卡片内展示 `Missing runtime provider` warning，并提供 `Refresh files` / `Add provider` 快捷动作。
- 如果 selected provider 只来自 `settings.json.defaultProvider`、且不是官方内置 key、也没有对应 auth/models entry，Default model 字段旁显示 `Missing runtime provider` warning；允许保存，但提示 Pi 运行时可能无法使用该 provider。
- `enabledModels` 不放在顶部卡片里，它属于 Other Configuration。

#### Provider 列表与卡片

Provider 区域是 Pi 页面最重要的信息面板。它展示的是当前 runtime 文件和 `settings.defaultProvider` 中实际出现的 provider key，让用户一眼分清 credential 来源、custom/override 配置、内置 provider 状态和当前默认 provider：

- Section header 左侧标题 `Providers`，右侧按钮：
  - `Add provider`：主按钮，打开空白新增表单。
  - `Refresh`：icon button，重新读取 `settings.json`、`auth.json`、`models.json`。
- Provider 列表不要把所有官方内置 provider 都展开成卡片；官方内置 provider registry 只用于 Model Settings 下拉、Add provider 预设、override 判断和 missing 校验。列表卡片只展示 auth/models/default 中实际出现的 key。
- 卡片左侧显示 provider 名称、providerKey 和来源标签；同一 key 同时来自 `auth.json` 和 `models.json` 时合并为一张卡片。
- 卡片右侧顶部显示状态 tag：
  - `API Key`：来自 `auth.json.<providerKey>.type = "api_key"`。
  - `Built-in`：providerKey 存在于 Pi 官方内置 provider registry。
  - `Env possible`：内置 provider 没有 stored credential，但 Pi 仍可能通过环境变量解析凭据。
  - `Custom`：来自 `models.json.providers.<providerKey>`，且 key 不是内置 provider 或无法识别为内置 provider。
  - `Override`：来自 `models.json.providers.<providerKey>`，且 key 与 Pi 内置 provider key 相同。
  - `OAuth detected`：subscription provider 在 `auth.json` 中检测到 OAuth credential。
  - `Default`：`settings.json.defaultProvider` 指向该 provider。
- 当前 default provider 可以在卡片标题旁显示一个轻量 `Default` tag，但设置入口仍只在顶部 Model Settings 卡片里，不在 provider 卡片底部放主按钮。
- 状态 tag 不只靠颜色区分，必须有文字；错误/警告状态配 icon。
- 卡片主体第一行展示默认模型或推荐模型；第二行展示落盘文件来源：
  - API Key：`auth.json · key: $ANTHROPIC_API_KEY` 或 masked literal。
  - Custom：`models.json · openai-completions · 12 models`。
  - Subscription：`auth.json · OAuth managed by Pi /login`。
- 卡片底部动作：
  - `Edit`：编辑当前 exact provider key；保存后覆盖对应 `auth.json.<key>` 或 `models.json.providers.<key>`。
  - `Fetch models`：有 baseUrl/apiKey 时可用。
  - `Test`：打开连通性测试。
  - `More` menu：Copy、Rename runtime key、Delete credential、Delete provider config、Delete both、Open runtime file。
- `Set default` 不作为 provider card 底部主操作；用户要改默认项时回到顶部 Model Settings 卡片完成。
- 不要出现“Applied”“Registered”“Not registered”文案。Pi 页面统一使用 `Default`、`API Key`、`OAuth detected`、`Custom`、`Override`、`Missing runtime provider`，避免和 Claude/Codex 的单当前渠道语义混淆。

#### Provider 空状态和异常状态

- 无 provider entry 时显示一个简洁 empty state：标题 `No provider entries in runtime files`，主按钮 `Add provider`，次按钮 `Refresh files`。如果 `settings.defaultProvider` 指向官方内置 provider，则不显示 empty state，而显示该 built-in/default provider row。
- 读取 `settings.json` 成功但 `auth.json` / `models.json` 缺失时，不显示错误，只显示空列表或对应的 `No custom providers`；如果 default provider 是官方内置 key，只显示 built-in/default 状态，不显示 missing。
- JSON parse 失败时在 Providers section 顶部显示 Alert，内容包括文件名、错误摘要和 `Open file` 按钮；不要阻塞其他可读数据展示。
- 如果 `settings.defaultProvider` 指向既非官方内置 key、也不在 auth/models runtime 文件中的 provider，顶部 meta 和列表里显示一个 `Missing default provider` warning row，允许用户重新设默认。

#### Provider Form Adapter

`PiProviderFormAdapter` 只做 Pi 数据结构到共享表单 props 的转换，UI 本体优先复用 `@/components/common/ProviderFormModal`。只有 Pi 的 `api` 类型、credential resolution 或 provider-scoped env 确实无法通过现有 props 表达时，才给共享表单补通用字段，不复制一份 Pi 专属 Modal。

共享 Modal 继续沿用现有普通 Modal 风格，参考 `ConnectivityTestModal` / `FetchModelsModal`：

- Modal 宽度桌面端建议 `760px` 到 `860px`；内容过高时 body 内部滚动，不改全局 modal chrome。
- 表单默认水平布局：左侧 label，右侧输入控件。窄屏下自动变成纵向。
- 外层只用 sectionCard / Collapse 分组，不嵌套卡片。
- category segmented control 只在空白新增时出现；编辑和复制都隐藏 category，并在标题下用小号 meta 显示 inherited category。对同时包含 `auth_json` 和 `models_json` 的 merged provider，编辑入口必须先选择编辑 `Credential` 还是 `Provider config`，不要在一个表单里静默同时改两份文件。
- 标题规则：
  - 新增：`Add Pi provider`
  - 编辑：`Edit <provider name>`
  - 复制：`Copy <provider name>`
- Footer 主按钮：
  - 新增/编辑：`Save`
  - 复制：`Create provider`
  - 重命名：`Rename key`
  - 保存并设默认：不建议和保存混在一个按钮里；首版保存后回到顶部 Model Settings 卡片修改默认 provider/model，避免用户误以为保存 provider 会删除其他 provider。

表单分组：

| 分组 | 适用 category | 字段 | 展示规则 |
|---|---|---|---|
| Basic | 全部 | Name、Provider key、Website、Notes、Icon | Provider key 是 runtime key；编辑后提示会覆盖或重命名运行时配置 |
| Credential | API Key | API key、Provider env JSON | key 默认 password/masked，支持 `$ENV`、`${ENV}`、literal；`!command` 显示安全提示 |
| OAuth status | Subscription | Provider key、Credential status | 不展示 token，不提供 token 编辑 |
| Endpoint | Custom | Base URL、API type、API key、Auth header | 新增全新 custom provider 时按 API 类型校验；覆盖内置 provider 时这些字段都可选 |
| Models | Custom | Models editor | 新增全新 provider 要至少有一个 model id；覆盖内置 provider 可只写 `modelOverrides` 或只覆盖 endpoint/header |
| Advanced | Custom | Headers JSON、Model overrides JSON、Compat JSON | 默认折叠，JSON 编辑器错误就近显示 |

#### Models editor

Custom provider 的模型编辑器不要只给一个大 JSON：

- 首版用可编辑表格或紧凑列表，每行一个 model。
- 每行默认列：Model ID、Display name、Reasoning、Input、Context、Max tokens、Actions。
- Cost 字段放在 row expand 或侧边抽屉里，避免表格横向溢出。
- 高级字段通过 `Advanced JSON` 折叠面板编辑，保存前合并回该 model。
- `Add model` 按钮在表格右上角；`Fetch models` 后弹出选择列表，用户勾选导入，不自动覆盖已有模型。
- 删除模型是普通确认，不使用危险色覆盖整个表格，只在确认按钮使用 danger。

#### Other Configuration

OpenCode 没有“通用配置”卡片，Pi 也不要引入这个概念。`settings.json` 中除 `defaultProvider/defaultModel/defaultThinkingLevel` 和 provider credential/model registry 以外的配置，统一放到 `Other Configuration` section。

位置和样式：

- 放在 Providers 下方、Global Prompt 上方。
- 复用 OpenCode `otherConfig` 的 `Collapse` 样式，默认折叠。
- 标题用 `Other Configuration`，左侧复用 `ToolOutlined`。
- 顶部说明只写一行：`Edit Pi settings not covered by provider management.` 不写长教程。
- 内容区使用“结构化字段 + Advanced JSON”组合：常用字段用表单控件，未结构化字段通过 JSON 编辑器保留。
- 保存行为是 patch `settings.json`；未知字段必须保留。用户在 Advanced JSON 删除某个已展示字段时，要按删除处理，不用 truthy 判断跳过。

完整 Pi settings UI 分组如下。首版可以先实现 `MVP` 标记字段，其余字段在同一设计里保留位置，或者只在 Advanced JSON 中编辑。

| UI 分组 | 字段 | 控件 | 首版 | 说明 |
|---|---|---|---|---|
| Model & Thinking | `hideThinkingBlock` | Switch | Later | 是否隐藏 thinking blocks |
| Model & Thinking | `thinkingBudgets` | JSON/Object editor | Later | thinking level 到 token budget 的映射 |
| Model cycling | `enabledModels` | Select tags / pattern list | MVP | Ctrl+P 可循环模型 pattern；不放顶部 Model Settings |
| UI / Display | `theme` | Select | MVP | `dark` / `light` / custom string |
| UI / Display | `quietStartup` | Switch | Later | 是否隐藏启动 header |
| UI / Display | `collapseChangelog` | Switch | Later | 更新后是否折叠 changelog |
| UI / Display | `doubleEscapeAction` | Select | Later | `tree` / `fork` / `none` |
| UI / Display | `treeFilterMode` | Select | Later | `/tree` 默认过滤模式 |
| UI / Display | `editorPaddingX` | Number stepper | Later | 0-3 |
| UI / Display | `autocompleteMaxVisible` | Number stepper | Later | 3-20 |
| UI / Display | `showHardwareCursor` | Switch | Later | IME/终端光标兼容 |
| Network | `httpProxy` | Input | MVP | 全局 HTTP/HTTPS proxy |
| Runtime | `sessionDir` | Path input | MVP | session 目录；支持绝对/相对/`~` |
| Trust | `defaultProjectTrust` | Select | MVP | `ask` / `always` / `never` |
| Telemetry / Update | `enableInstallTelemetry` | Switch | Later | 安装/更新 telemetry |
| Telemetry / Update | `enableAnalytics` | Switch | Later | analytics opt-in |
| Telemetry / Update | `trackingId` | Readonly/Input | Later | analytics tracking id |
| Warnings | `warnings.anthropicExtraUsage` | Switch | Later | Anthropic subscription extra usage warning |
| Compaction | `compaction.enabled` | Switch | Later | 是否启用 auto-compaction |
| Compaction | `compaction.reserveTokens` | Number input | Later | 为回复保留 token |
| Compaction | `compaction.keepRecentTokens` | Number input | Later | 保留最近 token |
| Branch summary | `branchSummary.reserveTokens` | Number input | Later | branch summary token reserve |
| Branch summary | `branchSummary.skipPrompt` | Switch | Later | `/tree` 是否跳过 summarize prompt |
| Retry | `retry.enabled` | Switch | Later | agent-level retry |
| Retry | `retry.maxRetries` | Number input | Later | agent-level 最大重试 |
| Retry | `retry.baseDelayMs` | Number input | Later | agent-level backoff |
| Retry provider | `retry.provider.timeoutMs` | Number input | Later | provider timeout |
| Retry provider | `retry.provider.maxRetries` | Number input | Later | provider/sdk retry，默认建议 0 |
| Retry provider | `retry.provider.maxRetryDelayMs` | Number input | Later | provider retry delay cap |
| Message delivery | `steeringMode` | Select | Later | `one-at-a-time` / `all` |
| Message delivery | `followUpMode` | Select | Later | `one-at-a-time` / `all` |
| Message delivery | `transport` | Select | Later | `auto` / `sse` / `websocket` / `websocket-cached` |
| Message delivery | `httpIdleTimeoutMs` | Number input | Later | HTTP idle timeout |
| Message delivery | `websocketConnectTimeoutMs` | Number input | Later | WebSocket connect timeout |
| Terminal / Images | `terminal.showImages` | Switch | Later | 终端内显示图片 |
| Terminal / Images | `terminal.imageWidthCells` | Number input | Later | 图片终端宽度 |
| Terminal / Images | `terminal.clearOnShrink` | Switch | Later | 内容缩小时清空空行 |
| Terminal / Images | `images.autoResize` | Switch | Later | 图片自动缩放 |
| Terminal / Images | `images.blockImages` | Switch | Later | 阻止图片发送到模型 |
| Markdown | `markdown.codeBlockIndent` | Input | Later | code block indentation string |
| Shell / NPM | `shellPath` | Path input | Later | 自定义 shell |
| Shell / NPM | `shellCommandPrefix` | Input | Later | 每个 bash 命令前缀 |
| Shell / NPM | `npmCommand` | argv editor | Later | npm 命令 argv |
| Resources | `packages` | List editor | MVP | packages 资源路径/包 |
| Resources | `extensions` | List editor | MVP | extension paths |
| Resources | `skills` | List editor | MVP | skill paths；不替代 AI Toolbox Skills 同步 |
| Resources | `prompts` | List editor | MVP | prompt template paths |
| Resources | `themes` | List editor | Later | theme paths |
| Resources | `enableSkillCommands` | Switch | MVP | 是否注册 `/skill:name` |
| Advanced JSON | unknown settings | JsonEditor | MVP | 保留所有未知字段和未来字段 |

Other Configuration 首版交互建议：

- 默认只展开 MVP 分组：Model cycling、Network、Runtime、Trust、Resources、Advanced JSON。
- Later 分组可以先折叠显示为 disabled/coming later，或不做结构化控件但必须能通过 Advanced JSON 保留和编辑。
- `mcp` 不应出现在 Other Configuration 中。Pi 官方没有 MCP runtime config，本计划不接 MCP。
- `defaultProvider/defaultModel/defaultThinkingLevel` 默认由顶部 Model Settings 卡片展示，不在 Advanced JSON 里重复展示；如果用户在 Advanced JSON 手写这些字段，保存时要和顶部 Model Settings 合并并刷新顶部卡片。
- Resources 字段必须保留官方输入形态：
  - `packages` 支持 string 和 object mixed list；首版结构化 UI 可以只提供 string list，但 Advanced JSON 必须保留 object form。
  - `extensions`、`skills`、`prompts`、`themes` 支持 glob、`!pattern` exclusion、`+path` force include、`-path` force exclude；结构化 UI 不应把这些字符串当非法值。

#### Global Prompt 与 Sessions 区域

- Global Prompt 复用 shared GlobalPromptSettings，但标题和 helper 必须写清楚只管理全局 `<root>/AGENTS.md`，不修改项目目录里的 `AGENTS.md` / `CLAUDE.md`。
- Prompt 卡片可以继续使用现有 active prompt 语义，`isApplied` 只属于 prompt，不用于 provider。
- Sessions 区域复用现有 SessionManagerPanel，默认折叠或放在 Providers 下方，避免 provider 管理被 session 列表挤出首屏。
- Session row 展示 cwd、session id、last active、model/provider；长 cwd 中间省略，hover 显示完整路径。

#### 响应式与可访问性

- `>= 1024px`：provider 卡片单列，动作按钮横向排列；卡片内容用 grid，左信息右状态。
- `720px - 1023px`：卡片动作允许换行，顶部 header 按钮保留 icon + text。
- `< 720px`：顶部 header 按钮收进 More menu；provider card 动作变成两行或 More menu；表单 label 改纵向。
- 所有 icon-only 按钮必须有 tooltip 和 `aria-label`。
- 可点击目标至少 44px 高；卡片 hover 不能是唯一可发现交互。
- Loading 超过 300ms 显示按钮 loading 或 section skeleton；save/delete/rename/test/fetch 都要有独立 loading 状态，不能锁死整页。
- 文案只写必要标签、状态和错误，不在页面里放“如何使用 Pi”的长教程；Docs 链接负责跳转官方文档。

### Provider 表单

Provider category 只在空白新增时选择。复制和编辑时隐藏 category 选择，并继承源 category。复制 provider 的本质是用源配置预填一个新的 runtime key；编辑 provider 的本质是覆盖当前 exact runtime key。

Category：

- Subscription：内置订阅 provider。首版只允许选择 providerKey，不录入 OAuth。
- API Key：内置 API key provider。写 `auth.json`。
- Custom：写 `models.json`。新增时需要在 `New custom provider` 和 `Override built-in provider` 之间选择；编辑时根据 providerKey 是否属于官方内置 provider 自动判定。

内置 providerKey 初始列表按官方 Providers 文档：

- `anthropic`
- `ant-ling`
- `azure-openai-responses`
- `openai`
- `deepseek`
- `nvidia`
- `google`
- `mistral`
- `groq`
- `cerebras`
- `cloudflare-ai-gateway`
- `cloudflare-workers-ai`
- `xai`
- `openrouter`
- `vercel-ai-gateway`
- `zai`
- `zai-coding-cn`
- `opencode`
- `opencode-go`
- `huggingface`
- `fireworks`
- `together`
- `kimi-coding`
- `minimax`
- `minimax-cn`
- `xiaomi`
- `xiaomi-token-plan-cn`
- `xiaomi-token-plan-ams`
- `xiaomi-token-plan-sgp`

API Key 模式字段：

- Provider name
- Provider key
- API key value
- Provider-scoped env JSON
- Test model，可选，只用于连通性测试，不写 `settings.json.defaultModel`
- Notes / website / icon

Custom 模式字段：

- Provider name
- Runtime provider key，默认使用用户输入的 slug 或从官方内置 provider 下拉中选择的 key；不要自动加 `ai-toolbox-` 前缀
- API type
- Base URL
- API key
- Auth header switch
- Headers JSON
- Models editor
- Model overrides JSON
- Compat JSON

Custom 校验规则：

- `Override built-in provider`：provider key 必须来自官方内置 provider registry。允许只保存 `baseUrl`、`headers`、`modelOverrides`、`models`、`compat` 中的任意一个或多个字段；不强制 `apiKey`，因为内置 provider 可以继续通过 `auth.json`、OAuth 或环境变量解析凭据。
- `New custom provider`：provider key 必须不是官方内置 key。若要让 provider 出现在 Pi `/model` 中，至少需要一个 `models[].id`；`api` 必须能从 provider 或 model 层推导；`baseUrl` 和 `apiKey` 按所选 API 类型和用户场景校验，不做比 Pi 官方 schema 更窄的硬编码限制。
- Provider 表单永远不写 `defaultProvider/defaultModel/defaultThinkingLevel`。这些字段只在顶部 Model Settings 卡片保存。

Models editor 第一版支持：

- `id`
- `name`
- `reasoning`
- `input` text/image
- `contextWindow`
- `maxTokens`
- cost 四字段
- 高级 JSON 覆盖：`thinkingLevelMap`、`compat`、`headers`、model-level `api` / `baseUrl`

### 获取模型

可复用现有共享 `fetch_provider_models` 思路，但 Pi 自定义 provider 的 API 类型更多。

模型来源要分三层处理：

1. Runtime 已有模型：来自 `models.json.providers.<providerKey>.models[]`，这是页面展示和保存的事实源。
2. 官方/内置模型：来自 Pi 官方内置 provider registry，只用于 Model Settings 下拉、built-in provider 校验和空 runtime 时的候选提示；首版不要把所有官方内置模型批量写入 `models.json`。
3. AI Toolbox 通用预设模型：来自 `PRESET_MODELS`，只用于“添加模型”“从预设补全字段”“Fetch models 返回为空时手动选择模板”等 UI 辅助；保存时必须转换成 Pi 官方 model schema 后写入当前 provider 的 `models[]`。

首版：

- `openai-completions` / `openai-responses`：尝试 `<baseUrl>/models`，如果 baseUrl 不以 `/v1` 结尾再尝试 `<baseUrl>/v1/models`。
- `anthropic-messages`：尝试 `<baseUrl>/v1/models`、`<baseUrl>/models`。
- `google-generative-ai`：尝试 Google models list 路径，需要单独适配，不建议首版阻塞。

注意：

- 后端 HTTP client 必须复用全局 rustls `http_client` 规则，不能 new 默认 native-tls `reqwest::Client`。
- `apiKey` 需要按 Pi value resolution 的 `$ENV` / literal 简单解析；`!command` 首版可以显示“不执行命令型 key 获取模型”，避免在后台执行任意命令。

### 连通性测试

复用 `ProviderConnectivityTestModal` 的交互，但请求构造要按 Pi API type：

- `openai-completions`：`POST <baseUrl>/chat/completions`
- `openai-responses`：`POST <baseUrl>/responses`
- `anthropic-messages`：`POST <baseUrl>/messages`，带 `x-api-key` 和 `anthropic-version`
- `google-generative-ai`：后续单独做

如果 provider 使用 `authHeader: true` 且 API 非标准，也要明确测试路径可能不可靠，允许只验证模型列表或保存。

## 后端模块设计

新增目录：

```text
tauri/src/coding/pi/
├── AGENTS.md
├── mod.rs
├── constants.rs
├── types.rs
├── adapter.rs
├── commands.rs
├── settings_merge.rs
├── auth_merge.rs
├── models_merge.rs
├── session_paths.rs
└── tray_support.rs
```

### commands

首版 Tauri command：

- `get_pi_root_path_info`
- `get_pi_config_dir_path`
- `get_pi_settings_file_path`
- `reveal_pi_config_folder`
- `read_pi_runtime_config`
- `read_pi_settings`
- `get_pi_settings_config`
- `save_pi_settings_config`
- `extract_pi_other_config_from_current_file`
- `save_pi_model_settings`
- `save_pi_other_configuration`
- `list_pi_runtime_providers`
- `save_pi_runtime_provider`
- `delete_pi_runtime_provider`
- `rename_pi_runtime_provider`
- `copy_pi_runtime_provider`
- `fetch_pi_provider_models`
- `test_pi_provider_connectivity`
- `list_pi_prompt_configs`
- `create_pi_prompt_config`
- `update_pi_prompt_config`
- `delete_pi_prompt_config`
- `apply_pi_prompt_config`
- `reorder_pi_prompt_configs`
- `save_pi_local_prompt_config`

接入：

- `tauri/src/coding/mod.rs`：`pub mod pi;`
- `tauri/src/lib.rs`：注册 command，启动时刷新 runtime_location cache，加 WSL listener
- `tauri/src/tray.rs`：Pi provider / prompt section
- `web/services/piApi.ts`：invoke wrapper

### 读取、保存 provider 与设为默认流程

```mermaid
sequenceDiagram
  participant UI as PiPage
  participant Cmd as pi::commands
  participant Settings as settings.json
  participant Auth as auth.json
  participant Models as models.json

  UI->>Cmd: read_pi_runtime_config()
  Cmd->>Settings: read settings.json
  Cmd->>Auth: read auth.json
  Cmd->>Models: read models.json
  Cmd-->>UI: merged PiRuntimeProviderView[]

  UI->>Cmd: save_pi_runtime_provider(providerKey, payload)
  alt API key provider
    Cmd->>Auth: upsert exact auth.json.<providerKey>
  else custom or override provider
    Cmd->>Models: upsert exact models.json.providers.<providerKey>
  else OAuth/subscription provider
    Cmd-->>UI: reject token edit; tell user to use Pi /login
  end
  Cmd-->>UI: emit pi-config-changed
  Cmd-->>UI: emit wsl-sync-request-pi
  opt user saves Model Settings
    UI->>Cmd: save_pi_model_settings(providerKey, model, thinkingLevel)
    Cmd->>Settings: patch defaultProvider/defaultModel/defaultThinkingLevel
    Cmd-->>UI: emit pi-config-changed
    Cmd-->>UI: emit wsl-sync-request-pi
  end
```

关键语义：

- `read_pi_runtime_config` 是页面主读取入口：一次读 `settings.json`、`auth.json`、`models.json`，后端合并成 provider view 和 Other Configuration view。
- `save_pi_runtime_provider` 保存 exact runtime key：API key 写 `auth.json.<providerKey>`，custom/override 写 `models.json.providers.<providerKey>`。没有 `ai-toolbox-*` 包装 key，也没有 owner metadata。
- `save_pi_model_settings` 只更新 `settings.json` 的默认选择；`isDefault` 是下一次读取时从 `settings.json.defaultProvider` 推导出来的 UI 状态，不入库。
- 切换默认 provider 不删除 `auth.json` 里的其他 credential，也不删除 `models.json.providers` 里的其他 custom provider。

### refresh runtime config

Pi 不做 Claude/Codex 式 `__local__` 收编，也不提供本地配置收编命令。页面的 `Refresh` 只重新读取当前 runtime 文件：

- 读取当前 `settings.json`、`auth.json`、`models.json`。
- 合并官方内置 provider registry、`auth.json` credential keys、`models.json.providers` keys 和 `settings.defaultProvider`。
- 不写 SQLite provider 记录，不生成 import metadata，不写 `x-ai-toolbox`。
- 如果 `settings.defaultProvider` 指向官方内置 key，但没有 stored credential，返回 built-in/default view，不返回 missing。
- 如果 `settings.defaultProvider` 指向既非官方内置、也不存在于 auth/models 的 key，返回 `missing_provider` warning view，供顶部 Model Settings 和 provider 列表展示。

## WSL / SSH 同步

Pi 应加入现有同步系统：

### WSL

新增事件：

- `wsl-sync-request-pi`

`tauri/src/lib.rs` 增加 listener：

- 检查 `coding::wsl::is_wsl_auto_sync_enabled`
- 调 `wsl_sync(..., Some("pi"), None)`

默认 mappings：

| mapping key | 本机源 | 远端目标 |
|---|---|---|
| `pi-settings` | `<root>/settings.json` | `~/.pi/agent/settings.json` |
| `pi-auth` | `<root>/auth.json` | `~/.pi/agent/auth.json` |
| `pi-models` | `<root>/models.json` | `~/.pi/agent/models.json` |
| `pi-agents` | `<root>/AGENTS.md` | `~/.pi/agent/AGENTS.md` |
| `pi-system` | `<root>/SYSTEM.md` | `~/.pi/agent/SYSTEM.md` |
| `pi-append-system` | `<root>/APPEND_SYSTEM.md` | `~/.pi/agent/APPEND_SYSTEM.md` |

Pi Skills 不作为普通 WSL file mapping。现有仓库中 Skills 的 WSL 同步是独立链路：源始终是 Skills 中央仓库，目标由工具 adapter 解析。Pi 只需要在 Skills runtime tool adapter 中提供 `<piRoot>/skills`，`skills-changed` 后由 `sync_skills_to_wsl` 负责把中央仓库同步到 WSL 侧统一中央仓库，再链接/复制到 Pi 的 WSL runtime skills 目录。

是否同步 `sessions/`：

- 默认不启用自动同步，避免大文件和跨机器 cwd 语义混乱。
- 可作为手动/高级 mapping。

WSL Direct：

- 如果 root path 是 WSL UNC，则 runtime_location mode 是 WSL Direct。
- 普通 WSL 同步应跳过 Pi，因为实际写入已直接落到 WSL 路径。

### SSH

SSH 仍按现有手动/full-sync 模式处理：

- 增加 Pi 文件映射。
- provider 注册或设置默认不自动触发 SSH 上传，除非后续全局设计新增 SSH event。
- 文案要区分“最终文件能通过手动 SSH sync 上传”和“注册/设置默认 provider 是否自动触发 SSH sync”。
- Pi Skills 不走普通 SSH file mapping。SSH Skills 同步是独立链路，源仍是 Skills 中央仓库；Pi 加入 runtime tool adapter 后，手动 SSH sync / 启用或切换连接触发的全量同步会把中央仓库同步到远端统一中央仓库，再链接/复制到远端 `~/.pi/agent/skills`。

## 备份恢复

备份恢复必须遵循当前 `tauri/src/settings/backup/AGENTS.md` 的语义：备份包里 SQLite 快照和 `external-configs/` 两者缺一不可；新增 Pi 时要同时改本地备份、WebDAV 备份、本地恢复、WebDAV 恢复和过滤规则候选项，不要只改其中一个入口。

### 备份内容

Pi external config 增加：

```text
external-configs/pi/root-dir.txt
external-configs/pi/settings.json
external-configs/pi/auth.json
external-configs/pi/models.json
external-configs/pi/AGENTS.md
external-configs/pi/SYSTEM.md
external-configs/pi/APPEND_SYSTEM.md
external-configs/pi/trust.json
```

说明：

- `root-dir.txt` 只在 Pi root source 为 `custom` 时写入，内容是当前 Pi root host path。
- `settings.json`、`auth.json`、`models.json` 不存在时跳过，不创建空文件。
- `AGENTS.md` 是首版 Global Prompt 主文件；`SYSTEM.md` / `APPEND_SYSTEM.md` / `trust.json` 如果存在就备份，避免恢复后丢失用户手写系统提示或 trust 决策。
- 不把 `<piRoot>/skills/**` 放进 `external-configs/pi/`。Skills 的 Source of Truth 是 AI Toolbox 中央仓库，当前备份包已有顶层 `skills/` 目录用于中央仓库备份；Pi runtime skills 目录只是 sync target，恢复后由 Skills resync 重建。

首版不默认备份：

- `sessions/**`
- `npm/**`
- `extensions/**`
- `skills/**` under `<piRoot>`

可以在备份设置中作为 custom entries 由用户选择。

修改点：

- `tauri/src/settings/backup/utils.rs`
  - 新增 `get_pi_restore_dir()` / `get_pi_restore_dir_from_db()`，默认返回 `~/.pi/agent`，DB 版本走 `runtime_location::get_pi_runtime_location_async(db).host_path`。
  - 新增 `get_pi_settings_path_from_db()`、`get_pi_auth_path_from_db()`、`get_pi_models_path_from_db()`、`get_pi_prompt_path_from_db()`、`get_pi_system_prompt_path_from_db()`、`get_pi_append_system_prompt_path_from_db()`、`get_pi_trust_path_from_db()`。
  - `get_custom_root_dir_path_info(db, "pi")` 支持 Pi custom root，source 为 `custom` 时写 `external-configs/pi/root-dir.txt`。
  - `backup_filter_option_path()` 增加 `pi => "~/.pi/agent/{relative_path}"`。
  - `list_backup_file_filter_path_options()` 增加 Pi 文件候选：`settings.json`、`auth.json`、`models.json`、`AGENTS.md`、`SYSTEM.md`、`APPEND_SYSTEM.md`、`trust.json`。`root-dir.txt` 仍不作为过滤项。
  - `write_backup_zip_contents()` 使用现有 `add_external_config_file_to_zip()` 写 Pi 文件，确保统一经过 `should_filter_external_config_entry()`，并复用 `added_zip_directories` 避免 zip duplicate entry。
- `tauri/src/settings/backup/local.rs`
  - 读取 `external-configs/pi/root-dir.txt`。
  - 调 `resolve_restore_dir_override("pi", pi_restore_dir_override, get_pi_restore_dir()?)` 得到 Pi restore dir 和 warning。
  - 在 restore loop 增加 `external-configs/pi/` 分支，跳过目录 entry 和 `root-dir.txt`，其余文件统一走 `should_filter_external_config_entry(&filter_rules, "pi", relative_path)`。
  - 用 `resolve_external_config_restore_output_path(&pi_restore_dir, relative_path)` 防 path traversal，不直接 `join(relative_path)`。
  - 写 `auth.json` 后在 Unix 尽量设 `0600`，失败只记录 warning/log，不让 restore 主流程失败。
  - 如果 `resolve_restore_dir_override` 发生 fallback，需要同步修正恢复后的 SQLite 中 `pi_settings_config.rootDir`，否则 DB 仍指向备份机器的旧 root，页面刷新后会再次读错目录。
- `tauri/src/settings/backup/webdav.rs`
  - 与 `local.rs` 做同等 Pi restore 分支，不能只支持本地备份。
  - WebDAV restore 也要使用当前机器 restore 开始前的 `backup_file_filter_rules`，不允许被备份包里的旧 settings 反向覆盖。
- `tauri/src/settings/types.rs`
  - `BackupFileFilterRule.tool` 注释加入 `pi`。
  - 默认 filter rules 仍为空；不要给新用户注入默认排除项。
- `web/features/settings/`
  - 备份过滤 UI 不需要硬编码 Pi 路径；只要后端 `list_backup_file_filter_path_options()` 返回 `tool=pi`，前端应自然显示。
  - 如果有 tool display name map，则补 `pi -> Pi`。

恢复规则：

- 如果备份中有 `root-dir.txt`，按现有 root override 规则处理：可用的绝对路径或 WSL UNC 直接使用；不可用路径 fallback 到默认 `~/.pi/agent` 并给 `RestoreWarning`。
- 如果发生 fallback，必须 patch 恢复后的 `pi_settings_config.rootDir` 为 fallback path 或清空为默认路径语义，保证后续 runtime_location、托盘、WSL/SSH、provider 保存和默认选择读取同一个目录。
- 恢复 Pi 文件只覆盖备份包里存在的文件，不清空目标目录里额外文件；这是备份恢复，不是镜像同步。
- `auth.json` 恢复必须保留文件权限语义。Unix 下写入后尽量设 `0600`。
- 不在恢复时解析、刷新、清理 OAuth token。Pi `/login` 写入的 credential 是 runtime-owned，恢复只做文件级还原。
- `.resync_required` 仍需要创建。它现在会触发 Skills 等恢复后重同步；Pi runtime skills 目录不随 `external-configs/pi` 恢复，而是依赖这个后续 resync 从中央仓库重建。

### 备份恢复最小测试

Rust 测试建议补在 `tauri/src/settings/backup/utils.rs` 和 local/webdav restore 相邻测试中：

- `list_backup_file_filter_path_options` 会返回 Pi 的 `~/.pi/agent/auth.json`、`settings.json`、`models.json` 等候选。
- Pi `auth.json` filter rule 会同时影响备份写入和 restore 跳过。
- `external-configs/pi/root-dir.txt` 不会被过滤规则排除。
- restore `external-configs/pi/settings.json` 会写到 Pi restore dir。
- restore 遇到不可用 `root-dir.txt` 会产生 warning，并同步修正 `pi_settings_config.rootDir`。
- 备份 zip 不包含 `external-configs/pi/skills/**`、`sessions/**`、`extensions/**`、`npm/**`。

## Session Manager 接入

新增：

- `tauri/src/coding/session_manager/pi.rs`
- `web/features/coding/shared/sessionManager/types.ts`：`SessionTool` 加 `pi`
- `web/features/coding/shared/sessionManager/utils.ts`：显示名
- `web/features/coding/shared/sessionManager/detail/SessionDetailPage.tsx`：Pi route wrapper
- `web/app/routeConfig.ts`：`/coding/pi/sessions/detail`

后端 `SessionTool` 加 `Pi`，`ToolSessionContext` 加：

```rust
Pi {
    sessions_root: PathBuf,
}
```

扫描逻辑：

- 遍历 `sessions_root/**/*.jsonl`
- 读取第一行 `type=session`
- `session_id` = header `id`，缺失时从文件名提取 uuid
- `project_dir` = header `cwd`
- `created_at` = header timestamp 或文件名 timestamp
- `last_active_at` = 文件 mtime 或最后 entry timestamp
- `title` = 最新 `session_info.name`，否则第一条 user message 摘要
- `resume_command` = `pi --session <source_path>` 或 `pi --session <session_id>`

消息解析：

- `message.role=user`：text/image blocks
- `assistant`：text/thinking/toolCall blocks，usage/cost/provider/model
- `toolResult`：tool result block
- `bashExecution`：command block + output block
- `compaction` / `branch_summary`：summary block
- `model_change` / `thinking_level_change`：system/event block
- `custom` / `custom_message`：custom block，保留 metadata

导入/导出：

- 原生 snapshot format：`pi-session-jsonl`
- 导出保留原始 JSONL payload。
- 导入时写入目标 sessions root，避免只保存 normalized messages。
- 删除 session 可以删除 `.jsonl` 文件。官方交互里可能用 `trash` CLI，但 AI Toolbox 后端可直接删除，保持和其他工具一致并加确认。

## Skills / Extensions

Pi skills 官方加载路径：

- `~/.pi/agent/skills/`
- `~/.agents/skills/`
- `.pi/skills/`
- `.agents/skills/` in cwd and ancestors

首版只把 `~/.pi/agent/skills` 纳入 Skills 同步目标：

- Skills 中央仓库仍是 Source of Truth。
- Pi runtime skills 目录是 sync target。
- 用户如果想让 Pi 读取 Claude/Codex skills，也可以在 Pi `settings.json.skills` 添加 `~/.claude/skills`、`~/.codex/skills`。AI Toolbox 可以后续提供快捷开关。

Extensions / packages：

- 先作为 Other Configuration 的资源路径字段处理。
- 不做 package install/update UI。
- 不执行 `pi install`，避免引入项目 trust 和 npm side effect。

### Skills 同步落地方案

首版需要把 Pi 加入 Skills 工具列表，但只加官方明确的一等 skills 目录：

| 项 | 决策 |
|---|---|
| tool key | `pi` |
| display name | `Pi` |
| supports skills | true |
| skills target path | `<piRoot>/skills` |
| 默认同步模式 | 复用现有 Skills 引擎：优先 symlink，Windows junction fallback，最后 copy；WSL UNC 目标走 copy |
| source of truth | 仍是 Skills 中央仓库 `skill_settings:skills.central_repo_path` |
| runtime target 是否可编辑回写 | 否。Pi runtime `skills/` 是同步目标，不是源目录 |

后端改动点：

- `tauri/src/coding/tools/`：工具注册表新增 Pi skills target，`supports_skills=true`。
- `tauri/src/coding/skills/`：`get_tool_skills_path_*` 支持 `pi`，路径来自 `get_pi_root_dir_async(db)/skills`。
- `tauri/src/coding/wsl/`：不新增 `pi-skills` 普通 mapping；Pi Skills 由现有 Skills WSL 独立链路处理。
- `tauri/src/coding/ssh/`：不新增 Pi skills 普通 mapping；Pi Skills 由现有 Skills SSH 独立链路处理。
- `web/features/coding/skills/`：工具 pill 和批量同步工具列表显示 Pi。

不要把 Pi 文档里的 `~/.agents/skills` 当作首版同步目标。它是 Pi 的额外发现路径，且可能被其他 agent 共用；AI Toolbox 的“工具 runtime skills 目录”应先选 Pi 自己的 `<piRoot>/skills`，避免跨工具互相覆盖。

如果用户想复用 Claude/Codex skills，首版推荐在 Pi Other Configuration 的 `skills` 字段里写：

```json
{
  "skills": ["~/.claude/skills", "~/.codex/skills"]
}
```

这只是 Pi 官方 settings 资源路径，不改变 AI Toolbox Skills 中央仓库同步模型。

## MCP 非目标

> **已过时（2026-10-23）**：Pi 0.99+ 已原生支持 MCP，读取 `~/.pi/agent/mcp.json`（项目级 `.pi/mcp.json`），`pi-mcp-adapter` 读取同一文件；AI Toolbox 早已把 Pi 注册为 MCP 同步目标。本节仅保留为历史决策记录，现行事实以 `tauri/src/coding/pi/AGENTS.md` 与 `tauri/src/coding/mcp/AGENTS.md` 为准。

用户已明确 MCP 暂时不接。本计划只保留非目标边界：

- 不在 `tauri/src/coding/mcp/` 给 Pi 注册内置 MCP target。
- 不新增 `sync_mcp_to_pi`、`import_mcp_from_pi` 或 `get_pi_mcp_config_path_async`。
- 不把 MCP server JSON 写入 `settings.json`、`models.json` 或自造的 `mcp.json`。
- 不在 MCP 页面默认出现 Pi 工具 pill。
- 不生成 Pi MCP bridge extension。

MCP 与 Skills / Extensions 的区别必须写进 UI/文档文案：

| 能力 | Pi 官方状态 | AI Toolbox 首版处理 |
|---|---|---|
| Skills | 官方一等资源目录，Agent Skills 标准 | 接入 Skills 中央仓库同步目标 |
| Extensions | 官方 TypeScript 扩展机制，可注册 tools | 只保留 settings 路径，不做安装/桥接 |
| MCP | 没有官方 MCP runtime 配置 | 本计划不接入 |

## Pi 横切同步矩阵

| 能力 | 本机落盘 | WSL auto sync | WSL Direct | SSH sync | 备份恢复 |
|---|---|---|---|---|---|
| `settings.json` | set default / Other Configuration patch | `pi-settings` 默认启用 | root 是 WSL UNC 时直接写目标，普通 WSL sync 跳过 Pi | 手动/full-sync 上传 | 默认备份 |
| `auth.json` | API key provider patch，OAuth 保留 | `pi-auth` 默认启用 | 同上 | 手动/full-sync 上传 | 默认备份，mask 只影响 UI 不影响文件 |
| `models.json` | custom provider patch，保留其他 provider | `pi-models` 默认启用 | 同上 | 手动/full-sync 上传 | 默认备份 |
| `AGENTS.md` | Global Prompt apply | `pi-agents` 默认启用 | 同上 | 手动/full-sync 上传 | 默认备份 |
| `SYSTEM.md` / `APPEND_SYSTEM.md` | 首版只保留/高级展示 | `pi-system` / `pi-append-system` 可启用 | 同上 | 手动/full-sync 上传 | 默认备份 |
| `skills/` | Skills 中央仓库同步到 Pi runtime target | Skills 独立 WSL 链路，不走普通 mapping | WSL UNC 目标由 Skills adapter 解析 | Skills 独立 SSH 链路，不走普通 mapping | 备份顶层中央仓库 `skills/`，恢复后 resync 重建 Pi target |
| `extensions/` | 首版不管理 | 默认不启用 | 不适用 | 默认不启用 | 不默认备份大目录 |
| `sessions/` | Session Manager 读取/删除/导入导出 | 默认不启用 | 直接读取目标 root sessions | 可选手动 mapping | 默认不备份，可自定义 |
| MCP servers | 本计划不接入 | 不同步 | 不适用 | 不同步 | 不备份 Pi MCP 文件，因为不存在 |

## i18n

新增多语言 key 时必须使用仓库脚本，不要手动通读完整 locale JSON：

```bash
node scripts/i18n-keys.mjs find-key <key>
node scripts/i18n-keys.mjs find-text <text>
node scripts/i18n-keys.mjs check
```

建议 namespace：

- `pi.title`
- `pi.viewDocs`
- `pi.configPath`
- `pi.rootPathSource.*`
- `pi.provider.*`
- `pi.prompt.*`
- `pi.settings.*`
- `pi.sessions.*`

同时补：

- `subModules.pi`
- Settings visible tabs 文案

## 当前项目改动清单

### 后端

- `tauri/src/coding/pi/**`：新增 Pi 模块。
- `tauri/src/coding/mod.rs`：导出 Pi。
- `tauri/src/coding/runtime_location.rs`：Pi root / WSL Direct。
- `tauri/src/coding/cli_resolver.rs`：新增 `resolve_local_pi_program()`，Node global / fnm / home bin 候选。
- `tauri/src/coding/tools/**`：Pi skills target；不把 Pi 加入 MCP target。
- `tauri/src/coding/session_manager/mod.rs`：注册 Pi tool。
- `tauri/src/coding/session_manager/pi.rs`：Pi JSONL parser。
- `tauri/src/coding/wsl/**`：Pi config 默认 mappings、动态路径、同步事件；Skills 只接 adapter，不新增 `pi-skills` 普通 file mapping。
- `tauri/src/coding/ssh/**`：Pi config mappings；Skills 只接 adapter，不新增 Pi skills 普通 file mapping。
- `tauri/src/coding/skills/**`：Pi runtime skills path resolver。
- `tauri/src/coding/mcp/**`：不改；Pi 暂不接入 MCP。
- `tauri/src/db/schema.rs`：Pi tables。
- `tauri/src/db/migrations.rs`：Pi table migration/index。
- `tauri/src/lib.rs`：commands、startup runtime cache、WSL listener。
- `tauri/src/tray.rs`：Pi provider/prompt tray section。
- `tauri/src/settings/backup/**`：Pi external config backup/restore。
- `tauri/src/settings/types.rs` / `adapter.rs`：visible tabs、sidebar hidden、backup filter options。

### 前端

- `web/features/coding/pi/**`：新增页面和 Pi adapter；Model Settings 顶部卡片复用或抽取 OpenCode `modelCard` 结构，Other Configuration 复用 OpenCode `Collapse` + `JsonEditor` 结构，Provider 列表复用共享 `ProviderCard` / `ProviderFormModal` / `ModelFormModal` / `FetchModelsModal`。
- `web/features/coding/index.ts`：导出 Pi。
- `web/services/piApi.ts`：Tauri API wrapper。
- `web/types/pi.ts`：TS 类型。
- `web/constants/modules.tsx`：Coding subTabs 加 Pi。
- `web/app/routeConfig.ts`：Pi page + Pi session detail route。
- `web/stores/settingsStore.ts`：visibleTabs 默认值、sidebarHiddenByPage。
- `web/features/settings/**`：可见 tab、备份过滤路径、WSL/SSH 状态展示。
- `web/features/coding/shared/sessionManager/**`：Pi session 工具类型和详情页。
- `web/features/coding/skills/**`：工具列表显示 Pi skills sync target。
- `web/features/coding/mcp/**`：不显示 Pi MCP target。
- `web/i18n/locales/*.json`：通过脚本新增 key。

### 文档

- 新增 `tauri/src/coding/pi/AGENTS.md`：
  - Source of Truth
  - 官方路径与文件语义
  - `settings.json` / `auth.json` / `models.json` merge gotchas
  - WSL/SSH/备份最小验证
- 新增 `web/features/coding/pi/AGENTS.md`：
  - 页面职责
  - 复用 OpenCode 顶部 Model Settings 卡片、Other Configuration collapse 和共享 provider 组件，不单独设计 Pi 卡片样式
  - provider category / copy mode 规则
  - root path info 规则
  - prompt/session/Skills/WSL/SSH 注意事项
- 更新根 `AGENTS.md` Index：
  - `tauri/src/coding/pi/`
  - `web/features/coding/pi/`

## 实施阶段

### Phase 0：官方行为基线与 fixtures

目标：先把不可变事实固化成 fixtures/tests，避免后续靠印象实现。

任务：

1. 新增 Pi 模块目录和 `AGENTS.md`。
2. 添加 fixtures：
   - `settings.json` with default provider/model。
   - `auth.json` with API key + OAuth credential。
   - `models.json` with custom/override provider + another runtime provider。
   - Pi session JSONL v3。
3. 写 Rust 单元测试：
   - root resolver 默认 `~/.pi/agent`。
   - `PI_CODING_AGENT_DIR` 覆盖。
   - `settings.sessionDir` 解析。
   - `settings.defaultProvider = "anthropic"` 且 auth/models 都没有该 key 时，runtime view 标记为 official built-in，不返回 missing。
   - 同一个 key 同时存在 `auth.json` 和 `models.json.providers` 时，runtime view 合并为一条 provider view，并保留两个 source。
   - `models.json` merge 只覆盖当前 provider key，并保留其他 provider。
   - `auth.json` patch 保留 OAuth credential。
   - session parser 能解析 user/assistant/toolResult/bashExecution/model_change。

验收：

```bash
cd tauri && cargo test pi
```

### Phase 1：后端 runtime config / prompt 闭环

目标：不做 UI 前先保证 Tauri commands 可用。

任务：

1. DB tables + adapter，只保存 root/UI 辅助状态和 prompt presets，不保存 provider 主表。
2. `constants.rs` 固化官方内置 provider key registry；`types.rs` 定义 Pi runtime config/provider view/prompt 数据结构。
3. `settings_merge.rs` / `auth_merge.rs` / `models_merge.rs`。
4. commands：
   - read runtime config，合并 official provider registry / settings / auth / models
   - save runtime provider，按 exact key 写 auth 或 models
   - delete/rename/copy runtime provider，支持 credential / provider config / both scope
   - save model settings
   - get/save root/UI settings config 和 Other Configuration
   - list/create/update/delete/apply prompt
5. 保存 provider / model settings / Other Configuration 后 emit：
   - `pi-config-changed`
   - `wsl-sync-request-pi`

验收：

```bash
cd tauri && cargo test pi
cd tauri && cargo test coding::runtime_location::tests
```

### Phase 2：前端 Pi 页面

目标：页面可完成 Model Settings、Other Configuration、provider 新增/复制/编辑/删除/重命名、root path、prompt；默认 provider/model 通过顶部独立卡片管理，provider card 不承担默认设置主入口。

任务：

1. 新增 `PiPage`。
2. 新增 `PiModelSettingsCard`，复用或抽取 OpenCode `modelCard` / `modelCardTitle` / `modelCardContent` 样式。
3. 用 adapter 复用共享 `ProviderCard` / `ProviderFormModal` / `ModelFormModal` / `FetchModelsModal`，不要新建 `PiProviderCard`；Provider 表单不出现 default model/default thinking 字段。
4. 新增 `piModelPresetAdapter.ts`，复用 `PRESET_MODELS` 作为模型新增模板，并把通用预设字段转换成 Pi `models.json` 的 model schema；不要复用 OpenCode `models.dev` / unified model 存储语义。
5. 新增 `PiOtherConfigPanel`，复用 OpenCode Other Configuration 的 collapse + JsonEditor 结构，并按 Pi settings 分组补结构化字段。
6. 接入 `RootDirectoryModal` / `useRootDirectoryConfig`。
7. 接入 shared GlobalPromptSettings。
8. 接入 SessionManagerPanel。
9. i18n key 通过脚本写入。

验收：

```bash
pnpm exec tsc --noEmit
node scripts/i18n-keys.mjs check
```

### Phase 3：Session Manager

目标：Pi sessions 可浏览、详情可读、可删除、可导出/导入。

任务：

1. 后端 `session_manager/pi.rs`。
2. `SessionTool` 加 Pi。
3. 前端 `SessionTool` union 加 Pi。
4. route detail 加 Pi。
5. resume command 支持 `pi --session`。
6. export/import native snapshot。

验收：

```bash
cd tauri && cargo test session_manager::pi
pnpm exec tsc --noEmit
```

### Phase 4：托盘、Skills、WSL/SSH、备份恢复

目标：横切面达到其他 CLI 的基础一致性。

任务：

1. `tray_support.rs` 与 `tauri/src/tray.rs`。
2. `tools` / `skills` 注册 Pi skills target，路径为 `<piRoot>/skills`；WSL/SSH Skills 继续走现有独立链路。
3. Skills 页面工具列表显示 Pi，可对 Pi 执行单项/批量 skills sync。
4. WSL mappings + event listener。
5. SSH mappings。
6. Backup local/webdav external-configs/pi。
7. Settings visible tabs / sidebar hidden defaults。
8. moduleStatuses 支持 Pi。
9. 明确不在 MCP 页面新增 Pi target，不新增 Pi MCP sync command。

验收：

```bash
cd tauri && cargo test wsl
cd tauri && cargo test skills
cd tauri && cargo test settings
pnpm exec tsc --noEmit
```

### Phase 5：完整验证

因为这是跨模块、跨层、会影响保存/应用/同步/恢复/配置落盘的大功能，合入前必须跑仓库全量最小集合：

```bash
pnpm test
cd tauri && cargo test
pnpm exec tsc --noEmit
```

如果期间改到前端入口、路由、公共组件、i18n、Vite/TS 配置，再补：

```bash
pnpm build
```

## 功能对比

| 能力 | Claude Code 当前 | Codex 当前 | OpenCode 当前 | Pi 目标 |
|---|---|---|---|---|
| 模块类型 | 根目录模块 | 根目录模块 | 配置文件路径模块 | 根目录模块 |
| 默认根/配置 | `~/.claude`，另有 `~/.claude.json` 规则 | `~/.codex` | `~/.config/opencode/opencode.jsonc` | `~/.pi/agent` |
| 环境变量覆盖 | `CLAUDE_CONFIG_DIR` | `CODEX_HOME` | `OPENCODE_CONFIG` | `PI_CODING_AGENT_DIR` |
| 主设置文件 | `settings.json` | `config.toml` | `opencode.jsonc` | `settings.json` |
| 凭据文件 | `settings.json` env / `.claude.json` 等 | `auth.json` | auth/provider 配置分散 | `auth.json` |
| 自定义供应商文件 | `settings.json` env + extra settings | `config.toml` + `auth.json` + `model_catalog_json` | `opencode.jsonc.provider` | `models.json.providers` |
| 官方订阅 | Claude Pro/Max，本仓库不做官方账号表 | Codex official accounts + quota | OpenCode Zen/Go auth/provider | Pi `/login` 内置 ChatGPT/Codex、Claude Pro/Max、GitHub Copilot；首版只识别已有 auth |
| API key provider | env 字段写入 `settings.json` | API key 在 `auth.json` 或 preserve-mode provider token | provider config | 内置 provider 写 `auth.json`；custom provider 写 `models.json.apiKey` |
| 自定义 provider schema | Anthropic env/model 字段 + extra settings | TOML provider + auth + model catalog | provider/model JSON | 官方 `models.json` ProviderConfig/ProviderModelConfig |
| 模型列表 | 表单字段 + presets | 官方模型目录 + custom modelCatalog | unified models + free models | 内置 provider 依赖 Pi 内置列表；custom provider 用 `models.json.models` |
| 模型 ID 语义 | provider 自己字段 | `model` / catalog | 完整 `provider_id/model_id` | settings 是 `defaultProvider` + `defaultModel`；CLI `--model` 支持 `provider/id` |
| Global Prompt | `CLAUDE.md` | active `AGENTS.override.md` / `AGENTS.md` | config 同目录 `AGENTS.md` | 全局 `<root>/AGENTS.md` |
| System prompt | Claude runtime 自身 | Codex runtime 自身 | OpenCode config | Pi `<root>/SYSTEM.md` / `APPEND_SYSTEM.md`，后续高级管理 |
| Sessions | Claude project sessions | Codex JSONL/state | OpenCode DB/export | Pi JSONL tree sessions |
| Session resume | `claude --resume <id>` | `codex resume <id>` | `opencode -s <id>` | `pi --session <path|id>` |
| Plugins/Extensions | Claude plugins | Codex plugins | OpenCode plugins | Pi extensions/packages；首版不做 package 管理 |
| Skills | `skills/` sync target | `skills/` sync target | Skills 由 runtime 语义决定 | `~/.pi/agent/skills` sync target |
| MCP runtime config | 支持 MCP 配置同步 | 支持 `config.toml.mcp_servers` 同步 | 支持 OpenCode MCP 配置同步 | 本计划不接入 MCP；不显示 Pi MCP target |
| AI Toolbox Gateway | 已接管 single/failover | 已接管 single/failover，含 preserve official auth | 不作为首要路径，主要依赖 OpenCode 原生 provider/model | 不接入；Pi 原生多 provider/model 足够覆盖首版渠道管理 |
| WSL Direct | 支持 | 支持 | 支持 | 应支持，root UNC 即 WSL Direct |
| WSL/SSH 同步 | 配置、prompt、MCP、Skills 按工具语义同步 | config/auth、prompt、MCP、Skills 同步 | config/prompt/MCP 按配置路径同步 | settings/auth/models/prompt 走文件映射；Skills 走独立链路；MCP 不同步；extensions 默认不同步 |
| 备份恢复 | external-configs/claude | external-configs/codex | external-configs/opencode | external-configs/pi |

## 关键风险与处理

1. 自定义供应商不要写错文件

   Pi 官方 custom provider 的主入口是 `models.json`，不是 `settings.json`，也不是 `auth.json`。内置 API key provider 才写 `auth.json`。

2. 不要覆盖 OAuth token

   `auth.json` 里 subscription `/login` 写入的 token 是 Pi runtime-owned。切换 custom provider 时不应删除。

3. 不能只做页面，不做横切面

   新增 CLI 页面必须同步处理 DB、settings visible tabs、runtime_location、tray、session_manager、backup、WSL/SSH 和 Skills。否则用户会遇到“页面能保存，但托盘/备份/同步找不到 Pi”的断层。

4. value resolution 不能误导

   表单中 `apiKey` 可以是 literal、`$ENV` 或 `!command`。获取模型和连通性测试时不应默认执行 `!command`，除非后续做明确确认和安全边界。

5. project `.pi` 不属于全局页面默认管理范围

   Pi 有 project trust 机制。全局 Pi 页面编辑 `.pi/settings.json` 会绕开 trust 心智，首版不做。

6. i18n 文件很大

   新增/查询多语言 key 使用 `scripts/i18n-keys.mjs`，不要手动通读完整 JSON。

7. MCP 不能按其他 CLI 惯性硬接

   Pi 官方扩展点是 extensions/custom tools，不是 MCP runtime config。如果把 MCP server JSON 写进 Pi 配置，会制造一个 Pi 不读取的“假同步”。本计划暂不接 MCP，不做 bridge。

## 最小首版验收清单

- 新增 Pi tab，默认显示 `<root>/settings.json` 路径。
- 自定义 root 保存后，刷新页面读取新 root。
- 页面顶部有独立 Model Settings 卡片，样式复用 OpenCode `modelCard` 结构；Providers 下方有 Other Configuration collapse；provider card 底部不提供 `Set default` 主按钮。
- `settings.json.defaultProvider = "anthropic"` 且 `auth.json` / `models.json` 都没有 `anthropic` 时，页面显示 `anthropic` 为 built-in/default provider，不显示 missing；提示 credential 可来自 env 或 `/login`。
- 新增或编辑 API key provider `anthropic`，保存后：
  - `auth.json.anthropic.type = "api_key"`
  - 既有 OAuth entry 未被删除
- 将 `anthropic` 设为默认后：
  - `settings.json.defaultProvider = "anthropic"`
  - `settings.json.defaultModel` 正确写入
- 新增或编辑 custom/override provider `openrouter`，保存后：
  - `models.json.providers.openrouter` 正确写入
  - 其他 runtime provider 保留
- 覆盖内置 provider `anthropic` 时允许只写 `baseUrl` 或 `headers`，不强制填写 `models`、`apiKey`、`api`。
- 将 `openrouter` 设为默认后：
  - `settings.json.defaultProvider = "openrouter"`
- 当同一个 key 同时存在 `auth.json.openrouter` 和 `models.json.providers.openrouter` 时，Delete/Rename 必须让用户选择 credential、provider config 或 both scope，不能静默同时改两份文件。
- 复制 provider 时隐藏 category selector，并继承源 category。
- Provider 表单不出现 Default model / Default thinking level；测试模型只能作为 `Test model`，不能写入 `settings.json.defaultModel`。
- Other Configuration 覆盖官方 settings 字段：至少包括 `enabledModels`、`theme`、`httpProxy`、`sessionDir`、`defaultProjectTrust`、resources、`hideThinkingBlock`、`thinkingBudgets`、`markdown.codeBlockIndent`；Advanced JSON 必须保留 unknown fields、package object form 和 glob/exclusion patterns。
- Global Prompt 应用后写 `<root>/AGENTS.md`。
- Session Manager 能读取 Pi JSONL session，至少展示 user/assistant/toolResult/bashExecution。
- 托盘能切换 Pi provider 和 prompt。
- Skills 页面能把中央仓库 skill 同步到 `<root>/skills`，Pi 作为 sync target 显示。
- MCP 页面不显示 Pi 普通同步 target；本计划不实现 Pi MCP bridge。
- WSL auto sync 打开时，保存 provider、设置默认 provider/model、编辑 Other Configuration、应用 prompt 都发出 `wsl-sync-request-pi`，并同步 settings/auth/models/prompt。
- WSL/SSH 同步矩阵覆盖 settings/auth/models/AGENTS；Skills 走独立同步链路；sessions 和 extensions 默认不自动同步。
- 备份包含 `external-configs/pi` 的 root-dir/settings/auth/models/AGENTS/SYSTEM/APPEND_SYSTEM/trust，且不包含 Pi runtime `skills/`。

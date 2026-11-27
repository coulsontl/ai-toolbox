# `aitb` 与本地控制面速查

`aitb` 是 AI Toolbox 随应用提供的轻量命令行客户端。它只与本机 `127.0.0.1` 上的控制面通信，令牌从发现文件读取，不会打印到终端，也不需要手工复制。

## 1. 启动前检查

```bash
aitb status --json
```

- 成功：返回 `enabled`、`running`、`url`、`discoveryFile`、`approvalMode` 等字段。
- 退出码 `3`：应用没启动、控制面被关闭、发现文件缺失或令牌失效。先让用户在「设置 → Agent 集成（本地控制）」中启用服务。
- 发现文件默认是 `~/.ai-toolbox/control.json`，可用 `AI_TOOLBOX_CONTROL_FILE` 或 `--control-file` 覆盖。
- **不要 `cat` 发现文件，也不要在命令、日志或提交里输出 `token`。** 只通过 `aitb` 间接使用。

全局参数：

```bash
aitb [--json] [--timeout 120] [--control-file PATH] <command>
```

`--json` 输出机器可读结果；写操作默认等待桌面授权。`--timeout` 必须大于设置页的 `approvalTimeoutSecs`（默认 60 秒），否则本地 socket 会先超时、CLI 看不到真实的授权结果。120 秒是默认值；把 `--timeout` 调小不会缩短桌面弹窗，只会让 CLI 提前放弃。

`aitb` 未安装或版本过旧时：`command not found` 说明还没在「设置 → Agent 集成」安装 CLI；`unknown subcommand` / 参数与本文不一致说明安装的是旧二进制，先在设置页重新安装，再执行 `aitb status --json` 核对 `version`。

## 2. 命令树

### 2.1 状态与能力

```bash
aitb status
aitb cli list
aitb cli paths --cli codex
```

`cli list` 返回 17 个 `cliKey` 及其能力：`providerCrud`、`modelCrud`、`config`、`configReadOnly`、`prompts`、`apply`、`test`、`paths`。完整矩阵见 §3；调用前先核对，不要假设所有 CLI 都支持全部端点。

### 2.2 插件

```bash
aitb plugin list
aitb plugin get <id>
aitb plugin install <本地目录|npm 包|github:owner/repo>
aitb plugin reload [id]
aitb plugin enable <id>
aitb plugin disable <id>
aitb plugin remove <id> [--purge-data]
aitb plugin options <id> [--set '{"key":"value"}']
aitb plugin logs [--limit 200]
aitb plugin clear-logs
aitb plugin middleware
aitb plugin providers
aitb plugin provider-options <provider>
aitb plugin refresh-models
aitb plugin accounts
aitb plugin account-check <provider> [--account <id>] [--proxy <url>]
aitb plugin account-usage <provider> [--account <id>] [--proxy <url>]
aitb plugin account-checkin <provider> [--account <id>] [--proxy <url>]
aitb plugin link-cli <provider> --json-body <path|->
```

`link-cli` 的 JSON 与桌面「添加到 CLI」弹窗一致。`pluginSpec`、`providerId`、`cliKey` 必填，`displayName` 建议填写；URL 路径里的 provider 必须与 body 的 `providerId` 一致，否则 400：

```json
{
  "cliKey": "codex",
  "pluginSpec": "@acme/ai-plugin",
  "providerId": "demo",
  "displayName": "Demo",
  "accountStrategy": "auto_rotate",
  "modelIds": []
}
```

- `pluginSpec` 用 `aitb plugin providers --json` 里该 Provider 对应的插件 spec/id，空字符串会被拒绝。
- `modelIds` 为空表示插件 Provider 暴露的全部模型；`accountStrategy` 可选 `single` / `auto_rotate`，需要固定账号时再给 `accountKey`。
- 不确定 Provider 元数据时先 `aitb plugin providers --json` 查看 `models`、`defaultModel`，不要猜 id。

### 2.3 供应商（渠道）

```bash
aitb provider list --cli <cliKey>
aitb provider get --cli <cliKey> --id <providerId>
aitb provider create --cli <cliKey> --name "Demo" --base-url https://api.example.com/v1 --model demo-model --api-key-env DEMO_KEY
aitb provider update --cli <cliKey> --id <providerId> --name "Renamed" --base-url https://new.example.com/v1
aitb provider delete --cli <cliKey> --id <providerId>
aitb provider apply --cli <cliKey> --id <providerId> [--model <modelId>]
aitb provider test --cli <cliKey> --id <providerId> [--model <modelId>] [--prompt "say hi"]
aitb provider fetch-models --cli <cliKey> --id <providerId>
```

- **API Key 只走 `--api-key-env <ENV>` 或 `--api-key-stdin`**，不要写在 argv 里；也不要让 Agent 把 Key 写进日志。
- 复杂原生结构用 `--json-body <path|->`，例如 OpenCode 的 `provider` 对象、OpenClaw 的 `models.providers` 对象。`--name/--base-url/--model/--api-key-*` 会覆盖 JSON 中的同名顶层字段。
- `--base-url/--model/--api-key-*` 会被翻译成各 CLI 自己的 `settingsConfig` 结构（Claude 的 `env`、Codex 的 `auth` + TOML、ZCode 的 `config.api`/`config.access`、Grok/Kimi 的 `modelCatalog` 等）。Grok、Kimi、ZCode 的自定义渠道请同时给 `--model`，否则可能没有可应用/可切换的模型目录；需要完全控制时用 `--json-body`。
- `PATCH` 语义：不传 `apiKey` 时保留原 Key；GET 返回里的 `***` 是掩码，原样回传也不会覆盖真实 Key。`create` / `update` / `canonical` 字段若整值是 `***` 会被 400 拒绝，不要用 GET 结果直接拼新 Provider。
- Antigravity 仅支持 Google OAuth，对它的渠道 CRUD 会返回 `unsupported`（400），不是只读策略。
- **网关接管中的 CLI 不要绕过 `apply`**：`claudecode`、`claudedesktop`、`codex`、`grok`、`kimi`、`geminicli` 正在被网关接管时，`provider create/update/apply`、模型增删/默认模型、`config set` 都会由后端先恢复直连、写入、再按原 single/failover/aggregate 清单重新接管。不要先手工 `gateway restore` 再写；那样会丢掉 aggregate 的站点选择、别名和 `[agents]` 默认值。只想换主 Provider 时用 `provider apply` 或 `gateway switch`，不要重建接管。
- 插件 Provider（`meta.pluginProvider` 或 URL 含 `/plugin/<id>` 路由）同样视为网关接管型：`provider apply` 会走网关 Provider 路径，`provider test` / `fetch-models` 使用原始插件凭据。`customUrl` 指向其他 origin 时不会带上本机凭据，会直接 400。

### 2.4 模型

```bash
aitb model list --cli <cliKey> --provider <providerId>
aitb model add --cli <cliKey> --provider <providerId> --id <modelId> [--name "Display"]
aitb model delete --cli <cliKey> --provider <providerId> --id <modelId>
aitb model default --cli <cliKey> --provider <providerId> --id <modelId>
```

### 2.5 全局配置文件

```bash
aitb config get --cli <cliKey> [--scope common|cli|omo|settings]
aitb config set --cli <cliKey> --file <path|-> [--mode patch|replace] [--root-dir <path>] [--clear-root-dir]
aitb config extract --cli <cliKey> [--scope ...]
```

返回结构：

```json
{
  "cli": "codex",
  "scope": "default",
  "kind": "common",
  "format": "toml",
  "readOnly": false,
  "config": "model = \"gpt-5\"\n",
  "rootDir": null,
  "updatedAt": "..."
}
```

`format × mode` 矩阵：

| `format` | CLI | `config` 入参 | `mode=patch` | `mode=replace` |
| --- | --- | --- | --- | --- |
| `json` | `claudecode`、`claudedesktop`、`geminicli`、`antigravity`、`opencode`、`openclaw`、`pi`、`workbuddy`、`codebuddy` | 对象或 JSON 字符串 | 递归合并；`***`/`null` 保留旧密钥 | 整份替换；`***` 从旧树回填 |
| `jsonc` | `omo_native` | 对象或 JSONC 字符串 | 递归合并（JSONC 注释不保留） | 整份替换；`***` 从旧树回填 |
| `toml` | `codex`、`grok`、`kimi`、`zcode` | 对象或 TOML 字符串 | 递归合并，保留未知字段/注释 | 整份替换 |
| `yaml` | `oh_my_pi`、`hermes`、`dsh` | 对象或 YAML 字符串 | 递归合并 Mapping | 整份替换 |

- TOML/YAML 的入参（对象或原文）含字面量 `***` 会 400；保持旧密钥的正确做法是**省略该键**，不是回传掩码。
- `scope` 不是统一语义词：ZCode 用 `scope=cli` 读写 CLI 自有配置、默认 `common`；OMO Native 用 `scope=omo` 读写 `shared_base`、默认 `settings`；其余 CLI 忽略 `scope`。`config get` 会返回实际 `configPath`，OpenCode/OpenClaw 读写的就是磁盘上的真实配置。
- `readOnly: true`（CodeBuddy）写入返回 `read_only` 403；WorkBuddy 能力是 `config=false`，`config get`/`set` 返回 `unsupported`，它的 `models.json` 通过 `provider`/`model` 命令维护。
- `config extract` 只有 `claudecode`、`codex`、`grok`、`kimi`、`geminicli`、`antigravity` 支持从当前文件回读；其他 CLI 返回 405 `method_not_allowed`，不要反复重试。

### 2.6 全局提示词

```bash
aitb prompt list --cli <cliKey>
aitb prompt create --cli <cliKey> --name "Reviewer" --content-file ./prompt.md
aitb prompt update --cli <cliKey> --id <promptId> [--name ...] [--content-file ...]
aitb prompt apply --cli <cliKey> --id <promptId>
aitb prompt disable --cli <cliKey> [--id <promptId>]
aitb prompt delete --cli <cliKey> --id <promptId>
```

- `prompt update` 允许只改 `--name` 或只改 `--content-file`；两者必须至少给一个。只改一项时不会覆盖另一项。
- `prompt disable` 不传 `--id` 时自动查找当前 `isApplied=true` 的方案。
- OpenClaw 和 WorkBuddy 不支持提示词方案，返回 `unsupported`。

### 2.7 网关

```bash
aitb gateway status
aitb gateway start
aitb gateway stop
aitb gateway restart
aitb gateway logs [--page 0] [--limit 20] [--only-failed] [--cli <cliKey>]
aitb gateway health
aitb gateway reset-health
aitb gateway clis
aitb gateway engage --cli <cliKey> --mode single --provider <providerId>
aitb gateway engage --cli <cliKey> --mode failover
aitb gateway engage --cli <cliKey> --mode aggregate --provider <id1> --provider <id2> [--separator "|"]
aitb gateway switch --cli <cliKey> --provider <providerId>
aitb gateway restore --cli <cliKey>
```

- `gateway clis` 返回每个可接管 CLI 的 `mode`、`primaryProviderId`、`aggregate` 与运行状态；先看它再决定 `engage` 还是 `switch`。
- `gateway switch` 只切换已被接管的 CLI 的主 Provider，保留 single/failover/aggregate 其余配置（aggregate 的站点选择、别名、`[agents]` 默认值不变）。未接管时用 `provider apply`，不要用 `switch`。
- `gateway stop` 在仍有 CLI 被接管时会被后端拒绝；先 `restore` 再停。
- `engage` / `switch` / `restore` / `start` / `stop` 都是 L2 高风险操作，默认每次都要桌面确认。停止控制面或切到 `read_only` 时，所有等待中的授权会立即过期，CLI 得到退出码 4。

## 3. `cliKey`、来源类型与能力矩阵

`aitb cli list --json` 是能力的唯一权威来源；下表是 17 个 CLI 的当前矩阵，避免直接调用不支持的能力。

| `cliKey` | `kind` | `providerCrud` | `modelCrud` | `config` | `configReadOnly` | `prompts` | `apply` | `test` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `claudecode` | database | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `claudedesktop` | database | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `codex` | database | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `grok` | database | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `kimi` | database | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `geminicli` | database | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `zcode` | database | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `antigravity` | official | — | — | ✓ | — | ✓ | — | — |
| `opencode` | file | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `openclaw` | file | ✓ | ✓ | ✓ | — | — | ✓ | ✓ |
| `pi` | file | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `oh_my_pi` | file | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `omo_native` | file | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `hermes` | file | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `dsh` | file | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| `workbuddy` | file | ✓ | ✓ | — | — | — | — | ✓ |
| `codebuddy` | file | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |

- `antigravity` 只有 Google OAuth：渠道/模型 CRUD、`apply`、`test` 返回 `unsupported`；`config` 与提示词仍可用。
- `workbuddy` 没有独立全局配置文件，`config` 为 `false`；模型走 `provider`/`model` 命令。
- `codebuddy` 的 `settings.json` 由产品拥有，`config` 可读、`configReadOnly=true`，写入返回 `read_only` 403。
- 不支持 ≠ 授权失败：`unsupported` 是能力边界，换命令或换 CLI；`read_only` 才是用户策略拒绝。

别名：`claude` → `claudecode`、`gemini` → `geminicli`、`omp` / `oh-my-pi` → `oh_my_pi`、`omo` → `omo_native`。

## 4. 授权与退出码

| 退出码 | 含义 | 处理 |
| --- | --- | --- |
| `0` | 成功 | 读取 stdout |
| `2` | 参数错误（未知 cliKey、缺参数、Key 环境变量不存在） | 修正命令后重试 |
| `3` | 应用未运行 / 控制面关闭 / 令牌失效 | 请用户启动应用并在设置中启用 |
| `4` | 用户拒绝、授权超时、只读模式或能力不支持 | **不要重试**；按 `--json` 的 `error.code` 区分后向用户说明 |
| `5` | 执行错误（校验失败、上游失败、404、方法不允许等） | 读取 stderr/`--json` 错误体，修复后重试一次 |

`--json` 错误体的 `error.code` 会原样保留服务端 code；退出码 `4` 的决策表：

| `error.code` | 含义 | Agent 的正确反应 |
| --- | --- | --- |
| `approval_denied` | 用户在桌面点了拒绝 | 停止；说明被拒绝的操作，等待新指示 |
| `approval_timeout` | 桌面弹窗超时 | 停止；提示用户可调大 `approvalTimeoutSecs` 或当前 `--timeout`，不要连续重试 |
| `read_only` | 控制面处于只读模式 | 停止写操作；请用户在设置里切换授权模式 |
| `unsupported` | 该 CLI/能力不存在（如 WorkBuddy 的 `config`） | 换 CLI 或换能力；不要重试、不要尝试改数据库 |

退出码 `5` 的常见 code：`bad_request`、`not_found`、`conflict`、`method_not_allowed`、`internal_error`。这些可以修正参数/目标后重试一次。

授权请求会显示在桌面模态框：

- L1（可逆写法）：新增/修改供应商、模型、配置、提示词、应用配置、切换网关主 Provider。
- L2（高风险）：删除供应商、停止网关、接管/恢复 CLI、卸载插件、远程安装插件。
- `read_only` 模式下所有写操作直接返回 403；停止控制面或切到 `read_only` 时挂起弹窗会立即过期，CLI 得到退出码 4。
- 不要尝试通过直接改 SQLite、`control.json`、CLI 配置文件或调用内部 IPC 来绕过授权。授权是用户对信任边界的决定。

## 5. 相关端点（排障时看）

```text
GET    /v1/status
GET    /v1/clis
GET    /v1/clis/:cli/providers
POST   /v1/clis/:cli/providers
PATCH  /v1/clis/:cli/providers/:id
DELETE /v1/clis/:cli/providers/:id
POST   /v1/clis/:cli/providers/:id/apply
POST   /v1/clis/:cli/providers/:id/test
POST   /v1/clis/:cli/providers/:id/fetch-models
POST   /v1/clis/:cli/providers/:id/models
DELETE /v1/clis/:cli/providers/:id/models/:modelId
PUT    /v1/clis/:cli/providers/:id/default-model
GET    /v1/clis/:cli/config?scope=...
PUT    /v1/clis/:cli/config?scope=...
POST   /v1/clis/:cli/config/extract?scope=...
GET    /v1/clis/:cli/prompts
GET    /v1/clis/:cli/paths
GET    /v1/gateway/status
GET    /v1/gateway/clis
POST   /v1/gateway/start|stop|restart
GET    /v1/gateway/logs?cliKey=...
POST   /v1/gateway/health/reset
POST   /v1/gateway/takeover/engage|restore|switch
GET    /v1/plugins/providers
GET    /v1/plugins/providers/:provider/options
POST   /v1/plugins/providers/refresh-models
POST   /v1/plugins/providers/:provider/refresh-models
DELETE /v1/plugins/:id?purgeData=true
```

所有端点（除 `GET /v1/ping`）都要求 `Authorization: Bearer <token>`，且拒绝带 `Origin` 的浏览器请求；`Host` 必须是 `127.0.0.1:<port>` 或 `localhost:<port>`。

## 6. 安全红线

- 不读取/打印发现文件的 `token`，不把 token 放进 argv、URL、日志或提交。
- 不在命令输出里暴露 API Key / OAuth token；GET 结果里的 `***` 是掩码，不是可写值。
- 安装远程插件、删除供应商、启停网关前，先向用户说明影响；等待桌面确认，不要代点。
- 插件凭据只由宿主管理，不要手工改 `plugin-auth.json` 或数据库。

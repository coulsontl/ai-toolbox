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
aitb [--json] [--timeout 75] [--control-file PATH] <command>
```

`--json` 输出机器可读结果；写操作默认等待桌面授权，最多 `--timeout` 秒。

## 2. 命令树

### 2.1 状态与能力

```bash
aitb status
aitb cli list
aitb cli paths --cli codex
```

`cli list` 返回 17 个 `cliKey` 及其能力：`providerCrud`、`modelCrud`、`config`、`prompts`、`apply`、`test`、`paths`。

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

`link-cli` 的 JSON 与桌面「添加到 CLI」弹窗一致，最小形态：

```json
{ "cliKey": "codex", "providerId": "demo", "providerName": "Demo" }
```

实际字段以当前版本的 `AddPluginProviderToCliInput` 为准；不确定时先 `aitb plugin providers --json` 查看 Provider 的 `models`、`defaultModel` 等元数据。

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
- `PATCH` 语义：不传 `apiKey` 时保留原 Key；GET 返回里的 `***` 是掩码，原样回传也不会覆盖真实 Key。
- Antigravity 仅支持 Google OAuth，对它的渠道 CRUD 会返回 403/只读错误。

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

- `format` 为 `json` / `jsonc` 时，`config` 是对象；`toml` / `yaml` 时是原文。
- `mode=patch` 合并字段，`mode=replace` 整份替换。
- JSON 配置中的 `***` 会优先用旧值回填，不会把掩码写进文件；TOML/YAML 原文含 `***` 会被拒绝，需发送真实值或省略该键。
- `readOnly: true`（CodeBuddy）拒绝写入；WorkBuddy 的 `models.json` 通过 `provider`/`model` 命令维护。

### 2.6 全局提示词

```bash
aitb prompt list --cli <cliKey>
aitb prompt create --cli <cliKey> --name "Reviewer" --content-file ./prompt.md
aitb prompt update --cli <cliKey> --id <promptId> [--name ...] [--content-file ...]
aitb prompt apply --cli <cliKey> --id <promptId>
aitb prompt disable --cli <cliKey> [--id <promptId>]
aitb prompt delete --cli <cliKey> --id <promptId>
```

`prompt disable` 不传 `--id` 时自动查找当前 `isApplied=true` 的方案。OpenClaw 和 WorkBuddy 不支持提示词方案。

### 2.7 网关

```bash
aitb gateway status
aitb gateway start
aitb gateway stop
aitb gateway restart
aitb gateway logs [--page 0] [--limit 20] [--only-failed] [--cli <cliKey>]
aitb gateway health
aitb gateway reset-health
aitb gateway engage --cli <cliKey> --mode single --provider <providerId>
aitb gateway engage --cli <cliKey> --mode failover
aitb gateway engage --cli <cliKey> --mode aggregate --provider <id1> --provider <id2> [--separator "|"]
aitb gateway restore --cli <cliKey>
```

- `gateway stop` 在仍有 CLI 被接管时会被后端拒绝；先 `restore` 再停。
- `engage` / `restore` / `start` / `stop` 都是 L2 高风险操作，默认每次都要桌面确认。

## 3. `cliKey` 与来源类型

| 类型 | `cliKey` |
| --- | --- |
| DB 接管型 | `claudecode`、`claudedesktop`、`codex`、`grok`、`kimi`、`geminicli`、`zcode` |
| 文件配置型 | `opencode`、`openclaw`、`pi`、`oh_my_pi`、`omo_native`、`hermes`、`dsh`、`workbuddy`、`codebuddy` |
| 官方账号型 | `antigravity`（无自定义渠道 CRUD） |

别名：`claude` → `claudecode`、`gemini` → `geminicli`、`omp` / `oh-my-pi` → `oh_my_pi`、`omo` → `omo_native`。

## 4. 授权与退出码

| 退出码 | 含义 | 处理 |
| --- | --- | --- |
| `0` | 成功 | 读取 stdout |
| `2` | 参数错误（未知 cliKey、缺参数、Key 环境变量不存在） | 修正命令后重试 |
| `3` | 应用未运行 / 控制面关闭 / 令牌失效 | 请用户启动应用并在设置中启用 |
| `4` | 用户拒绝、授权超时或只读模式拒绝 | 不要重复轰炸弹窗；向用户说明并等待指示 |
| `5` | 执行错误（校验失败、上游失败、404 等） | 读取 stderr/`--json` 错误体，修复后重试 |

授权请求会显示在桌面模态框：

- L1（可逆写法）：新增/修改供应商、模型、配置、提示词、应用配置。
- L2（高风险）：删除供应商、停止网关、接管/恢复 CLI、卸载插件、远程安装插件。
- `read_only` 模式下所有写操作直接返回 403，退出码 `4`。
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
GET    /v1/clis/:cli/config
PUT    /v1/clis/:cli/config
GET    /v1/clis/:cli/prompts
GET    /v1/clis/:cli/paths
GET    /v1/gateway/status
POST   /v1/gateway/start|stop|restart
POST   /v1/gateway/takeover/engage|restore
```

所有端点（除 `GET /v1/ping`）都要求 `Authorization: Bearer <token>`，且拒绝带 `Origin` 的浏览器请求；`Host` 必须是 `127.0.0.1:<port>` 或 `localhost:<port>`。

## 6. 安全红线

- 不读取/打印发现文件的 `token`，不把 token 放进 argv、URL、日志或提交。
- 不在命令输出里暴露 API Key / OAuth token；GET 结果里的 `***` 是掩码，不是可写值。
- 安装远程插件、删除供应商、启停网关前，先向用户说明影响；等待桌面确认，不要代点。
- 插件凭据只由宿主管理，不要手工改 `plugin-auth.json` 或数据库。

# `aitoolbox` 与本地控制面速查

`aitoolbox` 是 AI Toolbox 随应用提供的轻量命令行客户端。它只与本机 `127.0.0.1` 上的控制面通信，令牌从发现文件读取，不会打印到终端，也不需要手工复制。

## 1. 启动前检查

```bash
aitoolbox status --json
```

- 成功：返回 `enabled`、`running`、`url`、`discoveryFile`、`approvalMode` 等字段。
- 退出码 `3`：应用没启动、控制面被关闭、发现文件缺失或令牌失效。先让用户在「设置 → Agent 集成（本地控制）」中启用服务。
- 发现文件默认是 `~/.ai-toolbox/control.json`，可用 `AI_TOOLBOX_CONTROL_FILE` 或 `--control-file` 覆盖。
- **不要 `cat` 发现文件，也不要在命令、日志或提交里输出 `token`。** 正常情况只通过 `aitoolbox` 间接使用；仅在本机没有 `aitoolbox` 时按 §1.1 的受限降级处理。

全局参数：

```bash
aitoolbox [--json] [--timeout 120] [--control-file PATH] <command>
```

`--json` 输出机器可读结果；写操作默认等待桌面授权。`--timeout` 必须大于设置页的 `approvalTimeoutSecs`（默认 60 秒），否则本地 socket 会先超时、CLI 看不到真实的授权结果。120 秒是默认值；把 `--timeout` 调小不会缩短桌面弹窗，只会让 CLI 提前放弃。

### 1.1 `aitoolbox` 不在 PATH 时

`command not found` 只说明命令不在 PATH 中，不代表控制面没启动；不要先让用户重装应用。

1. 先按常见落点用绝对路径调用：
   - Windows：`%LOCALAPPDATA%\AI Toolbox\bin\aitoolbox.exe`；或桌面端安装目录同级的 `aitoolbox.exe`。
   - macOS：`~/.local/bin/aitoolbox`；或 `/Applications/AI Toolbox.app/Contents/MacOS/aitoolbox`。
   - Linux：`~/.local/bin/aitoolbox`；或 `/usr/bin/aitoolbox`。
   - 本地开发仓：`cli/target/release/aitoolbox`、`cli/target/debug/aitoolbox`（Windows 带 `.exe`）。

   找到后先跑 `<绝对路径> status --json` 验证，后续命令沿用同一路径。
2. 只是 PATH 未刷新：请用户在「设置 → Agent 集成（本地控制）」点一次「安装 aitoolbox 到 PATH」，然后重开终端。安装过一次后，桌面端升级启动时会自动同步已安装的 `aitoolbox`，不需要重复安装。`unknown subcommand` / 参数与本文不一致说明装的是旧二进制，重新安装或使用升级后自动同步的版本。
3. 本机确实找不到任何 `aitoolbox` 二进制时，才用受限降级直连控制面：
   - 仅在内存中读取发现文件（默认 `~/.ai-toolbox/control.json`，可用 `AI_TOOLBOX_CONTROL_FILE` 覆盖）的 `url` 与 `token`；文件里**没有**独立的 `port` 字段，端口从 `url`（`http://127.0.0.1:<port>`）解析。
   - 请求 `http://127.0.0.1:<port>/v1/...`，带 `Authorization: Bearer <token>`；不要带 `Origin`，`Host` 必须是 `127.0.0.1:<port>` 或 `localhost:<port>`。请求体是 JSON（没有 body 的 GET 可省略）。
   - **严禁打印、回显、写日志或提交 token 明文**；不要把 token 放进 URL、argv、环境变量示例或脚本，用一次性脚本从文件读取并只放入请求头。
   - 该方式只用于临时排障；长期使用仍应安装 `aitoolbox`。

## 2. 命令树

### 2.1 状态与能力

```bash
aitoolbox status
aitoolbox cli list
aitoolbox cli paths --cli codex
```

`cli list` 返回 17 个 `cliKey` 及其能力：`providerCrud`、`modelCrud`、`config`、`configReadOnly`、`prompts`、`apply`、`test`、`paths`。完整矩阵见 §3；调用前先核对，不要假设所有 CLI 都支持全部端点。

### 2.2 插件

```bash
aitoolbox plugin list
aitoolbox plugin get <id>
aitoolbox plugin install <本地目录|npm 包|github:owner/repo>
aitoolbox plugin reload [id]
aitoolbox plugin enable <id>
aitoolbox plugin disable <id>
aitoolbox plugin remove <id> [--purge-data]
aitoolbox plugin options <id> [--set '{"key":"value"}']
aitoolbox plugin logs [--limit 200]
aitoolbox plugin clear-logs
aitoolbox plugin middleware
aitoolbox plugin providers
aitoolbox plugin provider-options <provider> [--set '{"key":"value"}']
aitoolbox plugin provider-auto-checkin <provider> --enabled true|false
aitoolbox plugin refresh-models
aitoolbox plugin accounts
aitoolbox plugin account-check <provider> [--account <id>] [--proxy <url>]
aitoolbox plugin account-usage <provider> [--account <id>] [--proxy <url>]
aitoolbox plugin account-checkin <provider> [--account <id>] [--proxy <url>]
aitoolbox plugin link-cli <provider> --json-body <path|->
aitoolbox plugin settings [--bun-path <PATH> | --clear-bun-path] [--npm-registry <URL>] [--china-mirror true|false] [--follow-app-proxy true|false] [--json-body <path|->]
aitoolbox plugin bun-status
aitoolbox plugin bun-ensure
aitoolbox plugin bun-probe <path>
aitoolbox plugin host-status
aitoolbox plugin host-restart
aitoolbox plugin pages
```

- `plugin settings` 不带修改参数时是 GET，返回 `bunManualPath` / `npmRegistry` / `followAppProxy` / `providerAutoCheckin`；带任一参数或 `--json-body` 时是 PUT，只更新传入字段（未传字段保留）。`--china-mirror true|false` 映射到 `https://registry.npmmirror.com` / `https://registry.npmjs.org`；`--clear-bun-path` 把 `bunManualPath` 置空，恢复自动探测。
- `plugin provider-options` 不传 `--set` 时读取，传 `--set` 时整对象替换该渠道的选项（与 `plugin options` 相同语义）。
- `plugin provider-auto-checkin` 只切换渠道级每日自动签到开关；账号级旧标记的兼容合并由后端处理。
- `plugin bun-status` / `plugin bun-probe <path>` 只读：分别查看当前 Bun 运行时来源与探测任意可执行文件；`plugin bun-ensure` 会下载/校验托管 Bun（L1，需要授权）。
- `plugin host-status` 查看 Bun 宿主进程；`plugin host-restart` 是 L1 可逆写操作，会中断进行中的插件请求。
- `plugin pages` 返回已启用插件的自定义页面清单（`pluginId`、`pageId`、`title`、`entryFile` 等），不签发页面 token。

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

- `pluginSpec` 用 `aitoolbox plugin providers --json` 里该 Provider 对应的插件 spec/id，空字符串会被拒绝。
- `modelIds` 为空表示插件 Provider 暴露的全部模型；`accountStrategy` 可选 `single` / `auto_rotate`，需要固定账号时再给 `accountKey`。
- 不确定 Provider 元数据时先 `aitoolbox plugin providers --json` 查看 `models`、`defaultModel`，不要猜 id。

### 2.3 供应商（渠道）

```bash
aitoolbox provider list --cli <cliKey>
aitoolbox provider get --cli <cliKey> --id <providerId>
aitoolbox provider create --cli <cliKey> --name "Demo" --base-url https://api.example.com/v1 --model demo-model --api-key-env DEMO_KEY
aitoolbox provider update --cli <cliKey> --id <providerId> --name "Renamed" --base-url https://new.example.com/v1
aitoolbox provider delete --cli <cliKey> --id <providerId>
aitoolbox provider apply --cli <cliKey> --id <providerId> [--model <modelId>]
aitoolbox provider test --cli <cliKey> --id <providerId> [--model <modelId>] [--prompt "say hi"]
aitoolbox provider fetch-models --cli <cliKey> --id <providerId>
```

- **API Key 只走 `--api-key-env <ENV>` 或 `--api-key-stdin`**，不要写在 argv 里；也不要让 Agent 把 Key 写进日志。
- 复杂原生结构用 `--json-body <path|->`，例如 OpenCode 的 `provider` 对象、OpenClaw 的 `models.providers` 对象。`--name/--base-url/--model/--api-key-*` 会覆盖 JSON 中的同名顶层字段。
- `--base-url/--model/--api-key-*` 会被翻译成各 CLI 自己的 `settingsConfig` 结构（Claude 的 `env`、Codex 的 `auth` + TOML、ZCode 的 `config.api`/`config.access`、Grok/Kimi 的 `modelCatalog` 等）。Grok、Kimi、ZCode 的自定义渠道请同时给 `--model`，否则可能没有可应用/可切换的模型目录；需要完全控制时用 `--json-body`。
- `PATCH` 语义：不传 `apiKey` 时保留原 Key；GET 返回里的 `***` 是掩码，原样回传也不会覆盖真实 Key。`create` / `update` / `canonical` 字段若整值是 `***` 会被 400 拒绝，不要用 GET 结果直接拼新 Provider。
- Antigravity 仅支持 Google OAuth，对它的渠道 CRUD 会返回 `unsupported`（400），不是只读策略。
- **网关接管中的 CLI 不要绕过 `apply`**：`claudecode`、`claudedesktop`、`codex`、`grok`、`kimi`、`geminicli` 正在被网关接管时，`provider create/update/apply`、模型增删/默认模型、`config set` 都会由后端先恢复直连、写入、再按原 single/failover/aggregate 清单重新接管。不要先手工 `gateway restore` 再写；那样会丢掉 aggregate 的站点选择、别名和 `[agents]` 默认值。只想换主 Provider 时用 `provider apply` 或 `gateway switch`，不要重建接管。
- 插件 Provider（`meta.pluginProvider` 或 URL 含 `/plugin/<id>` 路由）同样视为网关接管型：`provider apply` 会走网关 Provider 路径，`provider test` / `fetch-models` 使用原始插件凭据。`customUrl` 指向其他 origin 时不会带上本机凭据，会直接 400。
- `provider test` 是“测试”：上游连不通时仍返回 200，结果里每个模型带 `status: "error"` 与 `errorMessage`，不要把它当成命令失败；`fetch-models` 拉取失败才返回错误体（退出码 5）。

### 2.4 模型

```bash
aitoolbox model list --cli <cliKey> --provider <providerId>
aitoolbox model add --cli <cliKey> --provider <providerId> --id <modelId> [--name "Display"]
aitoolbox model delete --cli <cliKey> --provider <providerId> --id <modelId>
aitoolbox model default --cli <cliKey> --provider <providerId> --id <modelId>
```

- `model add --name` 会写入该 CLI 的显示名（Codex/Grok/Kimi/ZCode 的 `displayName`、OpenCode/OpenClaw/运行时/OMO 的 `name`）。
- 删除当前默认模型时，后端把默认切到剩余的第一个模型；没有剩余模型时清空默认字段（Kimi/Grok/Codex/ZCode/OpenCode/OpenClaw）。不会留下指向已删除模型的悬空默认。

### 2.5 全局配置文件

```bash
aitoolbox config get --cli <cliKey> [--scope common|cli|omo|settings]
aitoolbox config set --cli <cliKey> --file <path|-> [--mode patch|replace] [--root-dir <path>] [--clear-root-dir]
aitoolbox config extract --cli <cliKey> [--scope ...]
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
- **ZCode 两个 scope 的格式不同**：默认 `common` 是 TOML；`scope=cli` 读的是产品自有的 `cli/config.json`，返回 `format: "json"`，写入也要传 JSON（对象或 JSON 字符串）。
- **运行时 CLI 的 `other_settings` 是“受管视图”**：`pi` / `oh_my_pi` / `hermes` / `dsh` / `omo_native` 的 `config get` 返回可编辑的受管块；`mode=replace` 只替换提交的键，文件里未出现在该视图中的顶层键/注释会保留（避免误删用户数据）。要删除某个键，请用 `mode=patch` 提交该键的新值或使用对应 CLI 原生设置页。
- **dsh 0.1.7+** 把配置迁到 `profiles/<profile>/cordis.patch.yml`，AI Toolbox 不再改写其中的其它配置块；此时 `config set --cli dsh` 返回 `unsupported` 400（`config get` 仍可读）。provider/model 命令不受影响。
- `readOnly: true`（CodeBuddy）写入返回 `read_only` 403；WorkBuddy 能力是 `config=false`，`config get`/`set` 返回 `unsupported`，它的 `models.json` 通过 `provider`/`model` 命令维护。
- `config extract` 只有 `claudecode`、`codex`、`grok`、`kimi`、`geminicli`、`antigravity` 支持从当前文件回读；其他 CLI 返回 405 `method_not_allowed`，不要反复重试。

### 2.6 全局提示词

```bash
aitoolbox prompt list --cli <cliKey>
aitoolbox prompt create --cli <cliKey> --name "Reviewer" --content-file ./prompt.md
aitoolbox prompt update --cli <cliKey> --id <promptId> [--name ...] [--content-file ...]
aitoolbox prompt apply --cli <cliKey> --id <promptId>
aitoolbox prompt disable --cli <cliKey> [--id <promptId>]
aitoolbox prompt delete --cli <cliKey> --id <promptId>
```

- `prompt update` 允许只改 `--name` 或只改 `--content-file`；两者必须至少给一个。只改一项时不会覆盖另一项。
- `prompt disable` 不传 `--id` 时自动查找当前 `isApplied=true` 的方案。
- OpenClaw 和 WorkBuddy 不支持提示词方案，返回 `unsupported`。

### 2.7 网关

```bash
aitoolbox gateway status
aitoolbox gateway start
aitoolbox gateway stop
aitoolbox gateway restart
aitoolbox gateway logs [--page 0] [--limit 20] [--only-failed] [--cli <cliKey>]
aitoolbox gateway health
aitoolbox gateway reset-health
aitoolbox gateway clis
aitoolbox gateway engage --cli <cliKey> --mode single --provider <providerId>
aitoolbox gateway engage --cli <cliKey> --mode failover
aitoolbox gateway engage --cli <cliKey> --mode aggregate --provider <id1> --provider <id2> [--separator "|"]
aitoolbox gateway switch --cli <cliKey> --provider <providerId>
aitoolbox gateway restore --cli <cliKey>
```

- `gateway clis` 返回每个可接管 CLI 的 `mode`、`primaryProviderId`、`aggregate` 与运行状态；先看它再决定 `engage` 还是 `switch`。
- `gateway switch` 只切换已被接管的 CLI 的主 Provider，保留 single/failover/aggregate 其余配置（aggregate 的站点选择、别名、`[agents]` 默认值不变）。未接管时用 `provider apply`，不要用 `switch`。
- `gateway stop` 在仍有 CLI 被接管时会被后端拒绝；先 `restore` 再停。
- `engage` / `switch` / `restore` / `start` / `stop` 都是 L2 高风险操作，默认每次都要桌面确认。停止控制面或切到 `read_only` 时，所有等待中的授权会立即过期，CLI 得到退出码 4。

## 3. `cliKey`、来源类型与能力矩阵

`aitoolbox cli list --json` 是能力的唯一权威来源；下表是 17 个 CLI 的当前矩阵，避免直接调用不支持的能力。

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

- `antigravity` 只有 Google OAuth：渠道/模型 CRUD、`apply`、`test`、`fetch-models` 返回 `unsupported`；`config` 与提示词仍可用。
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
| `request_timeout` | CLI 本地 `--timeout` 先到期（常见于写操作还在等桌面授权，或服务端很慢） | 停止；把 `--timeout` 调到 `approvalTimeoutSecs` 以上再试一次，不要当成应用未运行 |
| `read_only` | 控制面处于只读模式 | 停止写操作；请用户在设置里切换授权模式 |
| `unsupported` | 该 CLI/能力不存在（如 WorkBuddy 的 `config`） | 换 CLI 或换能力；不要重试、不要尝试改数据库 |

退出码 `5` 的常见 code：`bad_request`、`not_found`、`conflict`、`method_not_allowed`、`internal_error`。这些可以修正参数/目标后重试一次。

授权请求会显示在桌面模态框：

- L1（可逆写法）：新增/修改供应商、模型、配置、提示词、应用配置、切换网关主 Provider。
- L2（高风险）：删除供应商、停止网关、接管/恢复 CLI、卸载插件、远程安装插件。
- `read_only` 模式下所有写操作直接返回 403；停止控制面或切到 `read_only` 时挂起弹窗会立即过期，CLI 得到退出码 4。
- 不要尝试通过直接改 SQLite、CLI 配置文件或调用内部 IPC 来绕过授权。`control.json` 只在 §1.1 的降级场景中于内存内读取 `url`/`token`，不写回、不回显。授权是用户对信任边界的决定。

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
PUT    /v1/plugins/providers/:provider/options
PUT    /v1/plugins/providers/:provider/auto-checkin
POST   /v1/plugins/providers/refresh-models
POST   /v1/plugins/providers/:provider/refresh-models
GET    /v1/plugins/settings
PUT    /v1/plugins/settings
GET    /v1/plugins/pages
GET    /v1/plugins/bun
POST   /v1/plugins/bun/ensure
POST   /v1/plugins/bun/probe
GET    /v1/plugins/host
POST   /v1/plugins/host/restart
DELETE /v1/plugins/:id?purgeData=true
```

所有端点（除 `GET /v1/ping`）都要求 `Authorization: Bearer <token>`，且拒绝带 `Origin` 的浏览器请求；`Host` 必须是 `127.0.0.1:<port>` 或 `localhost:<port>`。

## 6. 安全红线

- 不打印发现文件的 `token`，不把 token 放进 argv、URL、日志或提交；仅 §1.1 的降级场景可在内存内读取并使用。
- 不在命令输出里暴露 API Key / OAuth token；GET 结果里的 `***` 是掩码，不是可写值。
- 安装远程插件、删除供应商、启停网关前，先向用户说明影响；等待桌面确认，不要代点。
- 插件凭据只由宿主管理，不要手工改 `plugin-auth.json` 或数据库。

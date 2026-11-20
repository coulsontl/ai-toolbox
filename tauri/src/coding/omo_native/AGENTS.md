# OmO Native 后端模块说明

## 一句话职责

- `omo_native/` 负责 **OmO Native**（上游 oh-my-openagent 5.0 起的独立 edition：`omo` 二进制 + senpi 引擎，npm 包 `omo-ai`）的运行时根目录、`[native]` 配置块、以及引擎 agent 目录（provider / MCP / skills）的可视化管理。

## 与 OpenCode 插件版 OMO 的边界（最重要）

上游 5.0 起一个仓库里并存**两套 agent 名单不相交的 edition**：

| | OpenCode 插件版 | OmO Native（本模块） |
|---|---|---|
| 用户侧 | `opencode.json` 里的 `oh-my-openagent` 插件 | `omo` 二进制 / `bun add -g omo-ai` |
| 上游包 | `packages/omo-opencode` | `packages/omo-native`（+ `omo-senpi` / `senpi-task`） |

> 包名口径：`omo-senpi` 与 `senpi-task` 是 **Native edition 的引擎/任务子包**，不是独立产品——代码注释里引用的 `packages/omo-senpi/src/...`、`packages/senpi-task/src/...` 路径都指上游 monorepo 里这两个子包（早期统称 senpi 引擎；`[senpi]` 这个 legacy harness 块拼写也由此而来）。命名以 `packages/omo-native` 为主，本仓库模块名 `omo_native`。
| 配置块 | `[opencode]` | `[native]` |
| 后端模块 | `tauri/src/coding/oh_my_openagent/` | `tauri/src/coding/omo_native/` |
| 前端 | `web/features/coding/opencode/` 的 OMO 区块 | `web/features/coding/omo_native/` |

**两套名单必须分开维护**：

- Native agents（7 个）= `explore` / `librarian` / `plan-consultant` / `plan-reviewer` / `omo-native-code-reviewer` / `omo-native-gate-reviewer` / `omo-native-qa-executor`
- 插件版 agents（神话命名）= `sisyphus` / `hephaestus` / `prometheus` / `oracle` / `librarian` / `explore` / `multimodal-looker` / `metis` / `momus` / `atlas` / `sisyphus-junior`
- Native categories（10 个）**多一个 `architect`**；两边的 `deep` 都已拆成 `deep-low` / `deep-high`
- `metis` / `momus` 作为 `plan-consultant` / `plan-reviewer` 别名的窗口已在上游 5.0.0-beta.51 后关闭

把 Native 名单写进 `[opencode]`、或把神话名写进 `[native]`，都会静默变成「自定义 agent/category」而不是报错——**改动任一模块的名单前先确认改的是哪一边**。issue #405 的诉求来自 Native 文档，方向**不适用于** `oh_my_openagent`。

## Source of Truth

- **统一配置文件 `~/.omo/omo.jsonc` 是两版共用**（JSONC，允许注释与尾逗号）。本模块只读写 `[native]` 块，**绝不触碰 `[opencode]` 块与共享 base 键**。写入必须保留注释，走 `crate::coding::omo_jsonc_patch` 的原地补丁。
- 引擎状态目录 `<agentDir>`（默认 `~/.omo/agent`）：`settings.json`（默认模型；**引擎优先读 `settings.jsonc`**，存在即用之——所有读写走 `providers::resolve_omo_native_settings_file_name`，别再硬编码文件名）/ `models.json`（自定义 provider）/ `auth.json`（密钥，0600）/ `mcp.json`（`mcpServers`）/ `AGENTS.md`（全局提示词）/ `skills/` / `sessions/`。
- 长期主数据在 SQLite JSONB 表 `omo_native_settings_config` / `omo_native_agents_config` / `omo_native_prompt_config`（schema v26）；数据库为空时页面先看到从本地文件读出的 `__local__` 桥接态。
- 运行时根目录由 `runtime_location::get_omo_native_runtime_location_async()` 决议。

## 决议顺序（镜像上游）

`<agentDir>` 按 `packages/omo-senpi/src/components/agent-home/resolve-agent-home.ts`：

1. 应用内自定义根目录（DB `omo_native_settings_config` 的 `root_dir`）
2. `OMO_CODING_AGENT_DIR` / `SENPI_CODING_AGENT_DIR` / `PI_CODING_AGENT_DIR`（进程环境）
3. 上述变量的 shell 配置（与 OMP/Pi 一致）
4. `~/.omo/agent`（哨兵 `settings.json`）
5. pre-unification 扁平布局 `~/.omo`（哨兵 `settings.json`）
6. `~/.senpi/agent`（无 branded 布局时的引擎回退）

`[native]` 块解析按 `packages/omo-config-core/src/loader/resolution.ts`：**共享 base 键 → `[senpi]`（legacy 拼写）→ `[native]`，后者胜**。所以界面读配置要展示「Native 生效视图」（折叠后），不能只看块内——否则显示的值和实际运行的不符。

## Gotchas

- **`[native]` 块的键是 `.strict()` 校验的**（上游 `OmoTypedHarnessConfigSchema`）。只允许：`formatOnMutation` / `gateway` / `categories` / `agents` / `git_master` / `task` / `teams` / `models` / `model_profiles` / `model_profile` / `memory` / `telemetry` / `computer` / `disabled_skills`。写别的（尤其 `[opencode]` 的 `disabled_agents` / `sisyphus_agent` / `claude_code` / `background_task` / `browser_automation_engine`）会触发上游 unknown-key diagnostic。`build_native_block` 与 `collect_other_native_keys` 都按这个白名单过滤。
- **不要 stamp `2026-07-opencode-config-unification`** 到 `_migrations`——那是插件版的迁移标记，Native 不该写。本模块也不动 `$schema`。
- **`models.json` 没有对外发布的 schema**，字段形状反推自上游 `packages/omo-native/bin/lib/setup-opencode-providers.js` 的 `convertProvider`。写入必须按 provider key 局部更新、保留其他 provider 与未知字段，不能全量校验式重写。
- **内建 provider 名单的事实源是引擎，不是这份常量**：`constants.rs` 的 `OMO_NATIVE_BUILTIN_PROVIDERS`（48 条）是手工副本，会随引擎升级过期（2026-10-07 就发现漏了 `anthropic-subscription` / `cursor-cli-oauth`、还留着早不是内建的 `bai` / `ollama` / `typesafe`）。重建名单的方式：`omo --list-models` 按 provider 分组 **减去** `models.json` 里的自定义 provider。⚠️ `--list-models` **不列「目录按账号发现」的 provider**（如原生 `cursor`），要对着 `<runtime>/docs/providers.md` 补回——别只看一条命令。三条回归测试守护：有序无重复 / OAuth 列表是内建子集 / kebab-case。
- **「引擎内建渠道」只列已配置凭据的**（2026-10-07 改，对齐 OpenCode 的官方 Auth 渠道）：候选 = `--list-models` 的 provider **∪** `auth.json` 的键（后者覆盖 `cursor` 这类目录按账号发现的），过滤到内建，再逐个跑 `omo auth check` 只留 `ready` 的。`check_omo_native_provider_auth` 这条单查命令**已删除**——前端不再按需补查。空结果不是错误，走空态。
- **`omo auth check` 只能逐个 provider 查**：不带 `--provider` 报错，`--provider a --provider b` 只认最后一个，`--model '*'` 报 `invalid_state`。所以 `probe_ready_providers` 用 `futures_util::future::join_all` 做 **12 路有界并发**（`OMO_AUTH_CHECK_CONCURRENCY`）——48 个串行要 30 秒以上，全部一起起反而更慢（实测 48 路 10.4s vs 12 路 8.5s）。
- **⚠️ auth check 绝不能加 `--no-refresh`**：引擎对 `cursor-cli-oauth` 这类 provider 在不刷新时直接报 `provider_not_found`，会把真实可用的渠道误判成未配置。
- **`omo auth check` 已包含环境变量判定**：只设 `ANTHROPIC_API_KEY` 而没写 `auth.json` 时同样报 `ready`。所以不要另外自己读环境变量做判断——两套判定必然会分叉。
- ⚠️ **`apiKey` 的 config value 语法是 Pi 那一套，不是 OMP 那一套**。`fetch_provider_models` / 连通性测试的 `configValueMode` 必须传 **`omo`**（不是 `omp`）：omo 的 `apiKey` 支持 `$ENV_VAR` / `${ENV_VAR}` 插值、`!command`、`$$` / `$!` 转义（实测确认），而 OMP 的模式只认「`!command` 或整值精确匹配环境变量名」、**不插值**——传 `omp` 会把 `$MY_KEY` 当字面量发出去。`ConfigValueMode::Omo` 复用 Pi 的解析器，只有 `!command` 的主机跟随 omo 的运行时位置。
- **「获取模型」必须带上密钥**：卡片要把 `fetchModelsProvider.apiKey` 传给 `FetchModelsModal`。不传的话请求不带 `Authorization`，直接 401（2026-10-07 用户报的）。
- **`omo --list-models` 与 `omo auth check` 都走 `run_omo_command`**（20 秒超时，`OMO_PROBE_TIMEOUT`）。它们 spawn 外部进程，不要在请求路径上做无超时的调用。
- **`save_omo_native_other_config` 只补丁 `omo.jsonc` 的顶层共享键**：走 `omo_jsonc_patch::patch_top_level_block`，`[native]` / `[opencode]` 两个块与控制键（`_migrations` / `$schema`）一律不碰，传入对象里**缺失的键会被删除**。前端「其他配置」区块的 `omo.jsonc` 页签失焦保存的就是它。
- **`save_omo_native_other_settings` 只改 `settings.json` 的非受管键**：`defaultProvider` / `defaultModel` / `defaultThinkingLevel` 归「模型设置」，`packages` / `extensions` 归「扩展」；这 5 个键在 `otherSettings` 读取时被剔除，`set` / `remove` 里即使出现也不生效。保存前用 `read_settings_object_for_write`（json5，解析失败直接报错）读盘，再按前端提交的 `set` / `remove` 在盘上最新内容打补丁——**不要**换成宽松的 `read_settings_object`，否则坏文件会被当成空对象清空；也**不要**改成整份切片覆盖，页面打开期间引擎写入的 `tipsHistory` 等键会被内存快照回退。`set` / `remove` 里只有受管键时直接返回、不重写文件。写入按 strict JSON 重新序列化，**不保证保留注释**（引擎自己保存 `settings.json` 时也是 `JSON.stringify`）。前端「其他配置」区块的 `settings.json` 页签用它（`lookAt` 等引擎高级键走这里）。
- **provider 的展示顺序 = `providers` 对象的「键序」**（`serde_json` 开了 `preserve_order`）。`reorder_omo_native_providers` 按给定 key 重建该对象；`list_omo_native_providers` **不得**再对自定义 provider 排序——一旦按 key 排，拖拽结果下次读取就被抹掉。纯内建（`models.json` 里没有条目的）只参与计数，不参与这个顺序。
- **`auth.json` 的值是「config value」语义**：`$NAME` / `${NAME}` 插值、`!cmd` 执行 shell、`$$` / `$!` 是字面量转义。写用户密钥必须走 `providers::escape_literal_config_value`，否则含 `$` 的 key 会被引擎当变量展开。注意转义只能做一次（幂等性不成立）。
- **密钥一律归 `auth.json`（`{type:"api_key", key}`），`models.json` 里不留 `apiKey`**（2026-10-07 定）。引擎两处都认且**优先 `auth.json`**（实测：两处都有值时 `omo auth print-api-key` 返回 `auth.json` 的那个），但只有 `auth.json` 是 0600 权限；`models.json` 是 644，还会随 WSL/SSH 同步与备份跑到别处。`save_omo_native_provider` 会**顺手把 `models.json` 里的旧式 `apiKey` 移走**（显式传入的 key 优先，没有就用移出来的那个）——留着就是第二份真相，用户改了 key 之后旧的还在。
- **密钥必须两处都查，而且要把明文返回给前端**（`provider_api_key`）：只看 `auth.json` 会让 `models.json` 里存过 `apiKey` 的记录显示成「没存过」——用户报的「api key 又变成空了」根因就在这。`OmoNativeProvider.api_key` 返回**当前生效的明文**（`auth.json` 优先），界面直接回填——本应用是配置管理器，密钥要能看能改（2026-10-07 用户明确要求）。**不要**用 `has_key` 布尔替代它。
- **`save_omo_native_auth_config` 是整份覆盖**（与 provider 弹窗的按 key 局部更新不同）：用户在编辑器里删掉一个 provider 就是要删掉它。写入时对每个条目的 `key` 走 `escape_literal_config_value`，并沿用 0600 权限。
- **`OmoNativeRuntimeConfig.auth_content` 返回 `auth.json` 原文**（含明文密钥），供「引擎内建渠道」的 auth.json 编辑弹窗与页头预览使用。同样是「配置管理器」定位下的有意为之。
- `model_profile` 是**车道 id**（`daily-normal` / `daily-heavy` / `geeky-normal` / `geeky-heavy`，或未设置时的隐式 `recommended`）或字面 `provider/model` pin。`model_profiles.<name>` 与内建档同名时是**整体替换**，不是逐字段合并。
- **resume 旗标与 OMP 不同**：OMP 的 `-r/--resume` **接受**值（ID 前缀/路径），OmO 的 `--resume` 是**无值的交互式选择器**。按路径续接要用 `omo --session <path|id>`；写成 `omo --resume <path>` 会打开选择器并把路径当成一条聊天消息发出去。`OMO_NATIVE_IDENTITY.resume_template` 是唯一事实源，有回归测试守护。
- `[native]` 块写入是**幂等**的（连续写两次结果逐字相同）——`omo_jsonc_patch` 会保留值后原有的空白。改补丁逻辑时要保住这个性质，`writing_native_block_twice_is_idempotent` 是回归守护。
- 禁用已应用方案 = 撤回运行文件（移除 `[native]` 块），不留「已禁用但仍生效」的悬挂状态。

## 跨模块依赖

- `runtime_location`：模块 key 是 `omo_native`（别名 `omo`）。
- `omo_jsonc_patch`：与 `oh_my_openagent` 共用的 JSONC 原地补丁助手（`[native]` 块与顶层共享键都走它）。
- `coding/local_bridge`：`__local__` 桥接项的 id / 名字的**后端唯一事实源**（`LOCAL_CONFIG_ID` = `__local__`、`LOCAL_CONFIG_NAME` = `default`）。本模块与其余 8 个 CLI 模块共用，**不要再写字面量**。
- 事件：`config-changed` + `wsl-sync-request-omo-native`（仅 Windows）。
- 备份：`omo_native` 在 `ALWAYS_BACKUP_CLI_TOOLS` 里（运行文件由 CLI 自己拥有）。
- 重放：`reapply_applied_runtime` 的 `reapply_omo_native` 在备份恢复后按同一份方案重新写 `[native]` 块与全局提示词（恢复期间不 emit 事件，走 `apply_omo_native_prompt_config_internal_without_events`）。
- WSL/SSH 默认文件映射：`omo-native-config` / `-models` / `-mcp` / `-auth` / `-prompt`（后者是 2026-10-07 加的，WSL 版本 22 / SSH 版本 21）。**共用的 `~/.omo/omo.jsonc` 不在这里**——它归 opencode 模块的 `opencode-oh-my`。
- **前端不再有 Agent·Category 方案的编辑入口**（2026-10-07 页面收敛），但本模块的 `agents` 相关命令、表与 `tray_support` 一律保留：托盘方案切换与备份恢复仍在用。**不要当死代码删。**

## 最小验证

- 写入 `[native]` 块后，同一文件的 `[opencode]` 块、共享键与注释逐字未变（`tests/coding/omo_native/jsonc_isolation.rs`）。
- 连续写两次 `[native]` 块，产物逐字相同。
- `omo doctor` 对写出的 `[native]` 块不报 unknown-key / invalid-value。
- 清除 `[native]` 块后 `[opencode]` 块仍在。
- 数据库为空时能从本地 `[native]` 块读出 `__local__` 桥接态（名字是 `default`）。
- apply / clear 后 `~/.omo/omo.jsonc` 的 `[native]` 块与方案一致，且 WSL 同步事件已发出。
- 全局提示词：apply 一条预设后 `<agentDir>/AGENTS.md` 内容与该预设一致；`disable` 后文件清空而 DB 记录保留；库里没有 applied 预设时 `<agentDir>/AGENTS.md` 显示为 `__local__` 桥接态。
- 内建渠道：`list_omo_native_builtin_providers` 只返回**已配置凭据**的渠道（本机实测 1 个：`anthropic`，来自 `ANTHROPIC_AUTH_TOKEN`）；返回值里每个都 `ready: true`，且都**不在** `models.json` 的自定义条目里。设了凭据但没有模型目录的渠道（如原生 `cursor`）也会返回，`models` 为空数组。
- 内建渠道：把 `auth.json` 临时改名后重跑，`anthropic` 仍应出现（环境变量判定）——这条验证的是「凭据来源不限于 auth.json」。
- 其他配置：`save_omo_native_other_config` 写入一个顶层键后，`[native]` / `[opencode]` 块、控制键与注释逐字未变；传入对象里缺失的键被删除。
- 其他设置：给 `settings.json` 加 `lookAt` 后保存，`defaultProvider` / `defaultModel` / `defaultThinkingLevel` / `packages` / `extensions` 的值未变，`lookAt` 出现在 `otherSettings` 里并可回读；`set` / `remove` 里只有受管键时不重写文件。
- 拖拽排序：`reorder_omo_native_providers` 之后立刻 `list_omo_native_providers`，顺序与传入的 key 顺序一致（**不被字母序覆盖**）。
- 密钥迁移：给一个 `models.json` 里带旧式 `apiKey` 的 provider 保存一次（弹窗里不填 key），该 `apiKey` 应被移到 `auth.json`，`models.json` 里不再有它，而 `api` / `baseUrl` / `models` / `name` 逐字保留；`omo auth print-api-key --provider <p>` 仍返回同一个值。
- 密钥回填：`list_omo_native_providers` 返回的 `apiKey` 与 `omo auth print-api-key --provider <p>` **逐字相同**（引擎优先 `auth.json`，界面回填的必须是同一个值）。
- 「获取模型」：带 `apiKey` 且 `configValueMode: "omo"` 时返回 200；把 `apiKey` 去掉则 401——这条能同时守住「忘了传 key」和「用错了 config value 模式」两种回归。

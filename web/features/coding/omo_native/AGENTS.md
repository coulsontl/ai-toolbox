# OmO Native 前端模块说明

## 一句话职责

- `omo_native/` 页面负责 **OmO Native**（`omo` 二进制 / senpi 引擎）的可视化编辑面：
  Provider（`models.json` + `auth.json`）、引擎内建渠道（只读）、全局提示词
  （`<agentDir>/AGENTS.md`）、`omo.jsonc` 共享键、会话。

## 页面形态：五个区块（2026-10-07 起）

与其余 coding tab 同构：**`CodingPageHeader` + 供应商列表 + 引擎内建渠道 + 全局提示词 + 其他配置 + 会话管理**。

> ⚠️ **这个清单不是「本 CLI 特有的三个」，而是全仓页面的并集减去本 CLI 没有的。**
> 2026-10-07 第一次收敛时按「默认三区块」的旧规则做，结果砍掉了「其他配置」
> （全仓 7 个页面有）与「官方认证渠道」（5 个页面有），用户随即指出缺块。
> 规则原文与取并集的命令见 `docs/new-cli-onboarding-sop.md` §4.0.2-I。

| 区块 | 组件 | 数据源 |
|---|---|---|
| 供应商列表 | `OmoNativeProvidersSection`（`ProviderListSection` + `OpenCodeStyleCard`） | `models.json` 的 `providers` + `auth.json` |
| 引擎内建渠道 | `OmoNativeBuiltinProvidersSection`（共享 `OfficialProviderCard`）+ 标题栏 `auth.json` 入口 | `omo --list-models` + `omo auth check --json`，**只读**；**只列已配置凭据的**。`auth.json` 入口打开 `OmoNativeAuthConfigModal`（**应用内可编辑**，不跳资源管理器） |
| 全局提示词 | 共享 `GlobalPromptSettings` | `<agentDir>/AGENTS.md` |
| 其他配置 | `OmoNativeOtherConfigSection` | `omo.jsonc` 顶层共享键（原地补丁） |
| 会话管理 | 共享 `SessionManagerPanel` | `<agentDir>/sessions/` |

早先这里还挂着四个 Native 专属区块，已按统一形态移除：

| 移除的区块 | 去哪了 |
|---|---|
| 生效配置（effective view 表格） | 与页头的「预览配置」重复；后者还多给了 settings/models/mcp 三份文件 |
| Agent·Category 方案 | **前端入口撤下，后端与托盘全部保留**（见下） |
| MCP 与 Skills | 原本只是一段指引；两个能力各有顶层独立页面 |
| 更多选项 | 内容（CLI 手动路径）在页头 ⋯ 打开的侧栏设置弹窗里已有同一份 |

> ⚠️ **Agent·Category 方案不是死代码。** 撤下的只是编辑界面：
> `omo_native_agents_config` 表、`list/create/update/delete/reorder/toggle/apply/clear_omo_native_agents_config`
> 全套命令、`save_omo_native_local_config`、`tray_support.rs` 的方案切换菜单、
> `reapply_applied_runtime::reapply_omo_native` 的恢复逻辑**都还在运行**。
> 删它们会打断托盘与备份恢复。先例见 `oh_my_pi/AGENTS.md` 对 `listOmpAgents` 的同类说明。

### 没有 UI 入口的 API 封装（**不要当死代码删**）

`web/services/omoNativeApi.ts` 里下面这些**当前没有前端调用点**。§12.1 的死封装审计会把它们全部列出来，逐条结论如下：

| 封装 | 状态 |
|---|---|
| `listOmoNativeAgentsConfigs` / `create` / `update` / `delete` / `reorder` / `toggleDisabled` / `apply` / `clearApplied` / `saveLocalConfig` | **因页面收敛而失去入口**（原 `OmoNativeSettings` 用），**且托盘也不再展示方案组**（2026-10-07）。后端仍在跑（备份恢复、`reapply_applied_runtime`），保留 |
| `listOmoNativeBuiltinAgents` / `listOmoNativeBuiltinCategories` / `listOmoNativeModelProfiles` | **建模块起就没有入口**（agent 名单是前端常量 `OMO_NATIVE_AGENTS` / `OMO_NATIVE_CATEGORIES` 写死的，没有拉后端）。保留：它们是「名单以后改成动态拉取」的现成通道 |
| `getOmoNativeCliInfo` | **建模块起就没有入口**（`omo --version` 检测结果）。保留：给后续「CLI 版本提示」用 |
| `listOmoNativeMcpServers` / `saveOmoNativeMcpServer` / `deleteOmoNativeMcpServer` | **建模块起就没有入口**——MCP 由顶层的 MCP 页统一管理，本页只给指引。保留：后端 `tools/builtin.rs` 的注册表会发现 `omo_native` |

**判断依据**：前端无入口 ≠ 后端无消费者。撤 UI 时先跑 §12.1 的审计命令，再查后端命令是否被 `tray.rs` / `reapply_applied_runtime.rs` / 备份链路 / 顶层页调用。

## 与 opencode tab 里 OMO 区块的边界（最重要）

上游 5.0 起同一仓库并存两套 **agent 名单不相交**的 edition。本页面只服务 Native：

| | OpenCode 插件版 | OmO Native（本模块） |
|---|---|---|
| 前端位置 | `web/features/coding/opencode/` 的 OMO 区块 | `web/features/coding/omo_native/` |
| 配置块 | `[opencode]` | `[native]`（legacy 拼写 `[senpi]`） |
| Agent 名 | 神话命名（`sisyphus` / `hephaestus` / …） | `explore` / `librarian` / `plan-consultant` / `plan-reviewer` / `omo-native-*` |
| Category | 无 `architect`，`deep` 未拆分 | 多 `architect`，`deep-low` / `deep-high` |

**两套名单必须分开维护**。把 Native 名单写进 `[opencode]`、或把神话名写进 `[native]`，都**不会报错**——只会静默变成「自定义 agent/category」。名单常量在 `web/types/omoNative.ts`（`OMO_NATIVE_AGENTS` / `OMO_NATIVE_CATEGORIES` / `OMO_NATIVE_BLOCK_KEYS`），改动前先确认改的是哪一边。

## Source of Truth

- **统一配置文件 `~/.omo/omo.jsonc` 两版共用**（JSONC，注释与尾逗号合法）。本页面只读写 `[native]` 块与共享 base 键，**绝不触碰 `[opencode]` 块**。
- ⚠️ **本页当前没有任何入口能写 `[native]` 块**（2026-10-07 页面收敛时撤掉了「Agent·Category 方案」编辑界面；`save_omo_native_local_config` / `apply_omo_native_agents_config` 只剩后端与托盘在调）。因此：
  - **预览弹窗显示 `omo.jsonc` 全文**（`runtimeConfig.configContent`），不是只显示 `[native]` 块或共享键——用户要看的是文件本身。
  - **托盘不再展示方案切换组**（只留「默认模型」+「全局提示词」，与 Pi 同形）。`omo_native_config_` 的 dispatch 分支已删，`tray_support.rs` 里的 `get_omo_native_tray_data` / `apply_omo_native_agents_config` **无调用点但保留**（备份恢复、`reapply_applied_runtime` 仍在用）。
- `runtimeConfig.effective` 是后端按上游 `resolveOmoConfigView` 折叠出的**生效视图**（共享 base → `[senpi]` → `[native]`，后者胜），不是 `[native]` 块本身。`runtimeConfig.nativeBlock` 才是块内原始内容。
- 引擎状态目录 `<agentDir>`（默认 `~/.omo/agent`）：`settings.json` / `models.json` / `auth.json` / `mcp.json` / `skills/` / `sessions/` / **`AGENTS.md`**。长期主数据在 SQLite。
- **全局提示词的运行时文件是 `<agentDir>/AGENTS.md`**（`OMO_NATIVE_PROMPT_FILE`），由 senpi 引擎作为项目规则文件读取——与 OMP 的 `OMP_PROMPT_FILE`、Pi 的 `PI_PROMPT_FILE` 同名同语义。预设记录在 `omo_native_prompt_config` 表（schema v26）。

## 核心设计决策（Why）

- **供应商卡片用 `OpenCodeStyleCard`**，判定依据是 §0.1.1 的两个定性：`models.json` 有 per-provider 模型目录（→ 非 Claude 式）；多个 provider 同时可用、没有「切换到这个」的动作（→ 非 Codex 式，**卡片上没有头部「应用」按钮**）。
- **供应商列表支持拖拽排序，顺序存在 `models.json` 的「键序」上**。`providers` 是 JSON 对象，靠 `serde_json` 的 `preserve_order` 保序；`reorder_omo_native_providers` 按给定 key 重建这个对象。**读取端 `list_*` 绝不能再按 key 排序**——那样用户拖完一刷新就跳回字母序，表现成「拖拽没生效」（2026-10-07 修过一次）。只在 `custom` 排序模式且无搜索词时允许拖（其余模式展示顺序 ≠ 存储顺序）。模型行的拖拽由 `ModelListSection` **自带的** `DndContext` 处理，与外层这个无关。
- **供应商编辑弹窗是独立的 `OmoNativeProviderFormModal.tsx`**（水平布局 + `labelCol`/`wrapperCol`），与其余 CLI 的 `*ProviderFormModal.tsx` 同构，也因此被 `scripts/verify-form-modal-layout.mjs` 覆盖。**不套 `ProviderFormSections`**：那个组件无条件渲染「备注」，而本模块的 provider 存在引擎文件里、没有备注字段可存（网关三件套同理，本 CLI 不在网关支持列表内）。
- **模型目录的操作放模型列表工具栏，不放卡片头部**：「连通性测试」与「获取模型」是针对**模型目录**的操作，走 `ModelListSection` 的 `onTest` / `onFetchModels`（与 ZCode / Codex 一致）。它们曾经挂在卡片头部的 `extraActions` 里，语义上像是针对这张卡片的操作。
- **`OpenCodeStyleCard` 的模型行可选 prop 台账**（SOP §4.0.2-F-1）：

  | prop | 传？ | 理由 |
  |---|---|---|
  | `onAddModel` / `onEditModel` / `onCopyModel` / `onDeleteModel` | ✅ | 四个一起传；`onCopyModel` 是最常漏的一个 |
  | `onReorderModels` | ✅ | `models.json` 的模型是数组，顺序有意义 |
  | `onTestModels` / `onFetchModels` | ✅ | 见上一条 |
  | `onSetPrimaryModel` | ❌ | `models.json` 没有「默认模型」指针（默认模型在 `settings.json` 的 `defaultProvider`/`defaultModel`，不在本页管理） |
  | `onToggleModelDisabled` | ❌ | `models.json` 的模型条目没有 `enabled` 键 |
  | `modelSelectionMode` / `onToggleBatchDeleteMode` / `onBatchDeleteModels` | ✅ | 模型级批量删除（2026-10-07 补；此前只有供应商级，用户报「少了一个批量删除」） |

- **复制模型走「新增 + 预填」**：`models.json` 按模型 `id` 索引目录，所以不能照抄 Codex 的「同 id 加名字后缀」（会造出重复 id）。`handleCopyModel` 打开**新增**弹窗并预填副本，让用户自己起新 id。
- **「模型设置」里选中的默认渠道不可删**（2026-10-08 用户要求，规则与 Pi 一致）：删除按钮**保留但置灰**，hover 给原因（`omoNative.provider.deleteDisabledDefault`），并且**排除出批量选择**——只置灰单个按钮挡不住「全选 → 批量删除」这条绕过路径。判据是 `provider.key === runtimeConfig.modelSettings.providerKey`（`settings.json` 的 `defaultProvider`），由页面把 key 传给供应商区块算，**不改后端**：默认值本来就只在 `settings.json` 里，`list_omo_native_providers` 没有也不需要这个字段。删掉默认渠道会让引擎的 `defaultProvider` 指向一个不存在的 provider。
  > **哪些页面适用这条规则**：默认指针**写在 provider 记录之外**（settings/config 里的 `defaultProvider` / `model`）的 CLI——pi / ohMyPi / dsh / hermes / openclaw / opencode / omo_native。Claude 式（claudecode / codex / gemini / grok / kimi）是「应用」语义，不适用。新增 CLI 时按这条判据决定传不传 `actions.deleteDisabledReason`。
- **`[native]` 块是 `.strict()` 校验的**（上游 `OmoTypedHarnessConfigSchema`）。只允许 `OMO_NATIVE_BLOCK_KEYS` 那 14 个键；写别的（尤其 `[opencode]` 的 `disabled_agents` / `claude_code` / `background_task`）会触发上游 unknown-key diagnostic。
- 会话管理直接复用 `shared/sessionManager` 的 `SessionManagerPanel tool="omo_native"`。Native 与 OMP 同源引擎（senpi），JSONL 格式同构，后端 `session_manager/oh_my_pi.rs` 是参数化的同一份解析器——**不要**在前端另写一套解析。
- Provider 面读取的是引擎自己的 `models.json` / `auth.json`，不是本应用的 provider 表。`auth.json` 的值是「config value」语义（`$NAME` 插值、`!cmd` 执行 shell），写入必须走后端 `escape_literal_config_value`。
- ⚠️ **密钥一律回填明文**（2026-10-07 用户明确要求，见记忆库 `projects/ai-toolbox/ai-toolbox-shows-plaintext-credentials.md`）：本应用就是**管理配置**的，用户看自己的密钥天经地义，与 Claude Code / Codex / Kimi 的弹窗一致。列表接口的 `OmoNativeProvider.apiKey` 就是当前生效的明文（`auth.json` 优先，其次 `models.json` 的 `apiKey`），弹窗直接 `apiKey: provider?.apiKey ?? ''`。
  **禁止**用 `••••••••` placeholder + 「已保存」文案这类「防肩窥」套路——它在这个项目里是反模式：用户根本分不清「存了 key」和「没存 key」，直接导致「我保存了但看起来没保存」的误报。`hasKey` 布尔仍可用于判断有无，但**不能替代**明文回填。
- **「引擎内建渠道」区块只列已配置凭据的渠道，数据全部来自引擎自身**：`OmoNativeBuiltinProvidersSection` 用共享 `OfficialProviderCard`（`i18nPrefix="omoNative"`），筛选语义对齐 OpenCode 的「官方 Auth 认证渠道」——未配置的渠道在本模块里没有可做的事，列出来只是噪音。候选与就绪判定**全在后端**（`omo --list-models` ∪ `auth.json` 的键 → 过滤到内建 → 逐个 `omo auth check`），前端只拿结果。判断「是不是内建」的事实源是后端 `constants.rs` 的 `OMO_NATIVE_BUILTIN_PROVIDERS`（48 条，三条回归测试守护：有序无重复 / OAuth 是子集 / kebab-case）。⚠️ **不要照抄这份名单到前端**，也不要只凭 `--list-models` 判断——它不列「目录按账号发现」的 provider（如原生 `cursor`）。
- **⚠️ 凭据检查是页面加载时跑一次的后端调用，约 8 秒**（引擎只能逐个查，后端做 12 路并发）。所以：① 不要把它放进依赖频繁变化的重渲染路径；② 区块内要有 loading 态（`providers === null` 时转圈），不能因为慢就显示成空列表——「还没查完」和「一个都没有」在界面上必须是两种样子；③ 折叠标题上的数量只在查完后才显示，不要先写个 0。
- **标题栏的 `auth.json` 入口在应用内打开编辑弹窗**（`OmoNativeAuthConfigModal`），**不要**照抄 OpenCode 的 `revealItemInDir` 跳资源管理器：本应用是配置管理器，密钥要能看能改。弹窗保存走 `saveOmoNativeAuthConfig`（**整份覆盖**），保存后**同时**重读页面配置与重查内建渠道列表——列表是按凭据筛的，不重查就会显示过期的就绪状态。`content` 每次打开都从磁盘原文重读，不要缓存上一次的副本（别处改过的内容会被静默覆盖）。
- **「其他配置」区块只写 `omo.jsonc` 的顶层共享键**：`OmoNativeOtherConfigSection`（`Collapse` + `JsonEditor` + 失焦保存，形态对齐 OpenCode / Pi / OMP）调后端 `save_omo_native_other_config`，后者按顶层键做**原地补丁**——`[native]` / `[opencode]` 两个块、控制键与全部注释原样保留。不要改成前端拼整份 JSON 落盘。

## 易错点与历史坑（Gotchas）

- **只写 `[native]` 块**：apply/clear 走 `omo_jsonc_patch` 的原地补丁，保留注释与 `[opencode]` 块。不要改成前端拼整份 JSON 落盘，那会抹掉注释并可能覆盖插件版的配置。
- **不要 stamp `2026-07-opencode-config-unification`** 到 `_migrations`——那是插件版的迁移标记。
- **禁用已应用方案 = 撤回运行文件**（移除 `[native]` 块），不留「已禁用但仍生效」的悬挂状态。
- `__local__` 是本地文件桥接态，不是 DB 记录：不可 apply、不可删除，托盘里也不出现。提示词区块同理（`GlobalPromptSettings` 自己处理）。**id 与名字常量已抽到共享层**（前端 `shared/localConfig.ts` 的 `LOCAL_CONFIG_ID` / `shouldLoadOfficialAccounts`，后端 `coding/local_bridge.rs` 的 `LOCAL_CONFIG_ID` / `LOCAL_CONFIG_NAME`），本模块不要再写字面量 `'__local__'` / `'default'`。桥接项名字统一是 `default`（不是 `Local AGENTS.md`）——副标题已经在说「来自本地 AGENTS.md」，名字里不重复。
- **预览配置的标签要写「文件 → 块」而不是只有文件名**：本页只拥有 `[native]` 块，`omo.jsonc` 常只有 `[opencode]` 块——只写文件名的后果是这一栏显示 `{}`，看起来像「配置文件是空的」。`OmoNativePage` 的标签是 `omo.jsonc → [native]`，空块时显示 `omoNative.preview.emptyNativeBlock` 解释文案。
- WSL/SSH 默认映射里 `omo-native-*` 只覆盖引擎文件（`settings.json` / `models.json` / `mcp.json` / `auth.json` / `AGENTS.md`）；**共用的 `~/.omo/omo.jsonc` 归 opencode 模块的 `opencode-oh-my`**，两边都登记会互相覆盖。
- **编辑模型行时未知字段必须原样保留**：`models.json` 没有公开 schema，保存一条模型不能顺手删掉引擎写的其它键。`handleSaveModel` 以 `{...asRecord(existing), ...}` 为基底。
- ⚠️ **模型字段形状以 senpi 为准，不要照抄 OMP**。本模块与 Pi 同用 **senpi** 引擎、同读 `models.json`；**OMP 读的是 `models.yml`，字段形状不同**。权威定义在 `<runtime>/docs/models.md`（本机 `~/.omo/binary-runtime/<ver>/docs/models.md`）。两处曾照 OMP 写错（2026-10-07 修复，`git show HEAD` 确认是建模块时就有）：

  | 字段 | senpi（正确） | OMP（曾误用） |
  |---|---|---|
  | 思考级别 | `thinkingLevelMap`：三态 map（省略 / 字符串 / `null` 表示不支持） | `thinking: { efforts, defaultLevel }` |
  | 输入类型 | `input`：**数组** `["text","image"]` | 同（但弹窗交回的是 JSON **字符串**，必须 `parseInputTypes`） |
  | 模型级 `api` | 可选**覆盖**项；provider 已有时**不要写** | — |

  对应 `ModelFormModal` 的开关是 **`showThinkingLevelMap`**（不是 `showOmpThinking`）。回归测试在 `web/test/features/coding/omo_native/utils/omoNativeProviders.test.ts`。
- **provider 编辑弹窗的 JSON 编辑器校验失败时必须禁用保存**：`providerHeadersJsonValid` 为假时 `handleSaveProvider` 直接返回，否则会把非法 JSON 写进 `models.json`。

## 跨模块依赖

- 后端 `omo_native::*` 命令（`web/services/omoNativeApi.ts`、`omoNativePromptApi.ts` 一一对应）。
- `shared/sessionManager`：会话列表与详情页（路由 `/coding/omo-native/sessions/detail`）。
- `shared/toolIcon`：`omo_native` 复用侧栏的 `web/assets/omo-native.svg`。
- `shared/useRootDirectoryConfig` + `shared/RootDirectoryModal`：根目录自定义。
- `shared/providerCardVariants` + `shared/ProviderListSection` + `shared/providerList` + `shared/favoriteProviders`（`omo_native:` 前缀）：供应商列表与卡片。
- `shared/localConfig`：`__local__` 桥接态的 id / 名字与「是否加载官方账号」判定的**前端唯一事实源**（后端对应 `coding/local_bridge.rs`）。
- `components/common/OfficialProviderCard`：引擎内建渠道的卡片（共享组件，`i18nPrefix="omoNative"`）。
- `shared/prompt`（`GlobalPromptSettings`）+ `services/omoNativePromptApi`：全局提示词。
- 事件：`config-changed` + `wsl-sync-request-omo-native`（仅 Windows）。

## 最小验证

- 打开本页：页头显示引擎状态目录；**五个区块**（供应商 / 引擎内建渠道 / 全局提示词 / 其他配置 / 会话管理）都在，侧栏五项都能跳转。
- 供应商：能新建 / 编辑 / 复制 / 删除；编辑弹窗是**水平布局**（标签在左）；卡片第二行是 `ID • SDK • 端点`；模型折叠区能增删改模型、能进入模型批量删除模式；连通性测试与获取模型可用。
- 默认渠道：在「模型设置」选一个自定义渠道作默认 → 该卡片删除按钮置灰（hover 显示「该渠道已设为默认，不可删除」），批量选择模式下它没有复选框、「全选」也不含它；把默认换成别的渠道后按钮恢复可点。
- 排序：切到「自定义」并清空搜索后，卡片左侧出现拖拽把手；拖完重载页面顺序不变（顺序落在 `models.json` 的键序上）。
- 引擎内建渠道：区块内先转圈约 8 秒（后端在跑逐个凭据检查，区块默认展开，加载在页面挂载时就开始），然后只列出已配置凭据的渠道；未配置的一个都不出现。列表不含 `models.json` 里的自定义 provider。本机实测只有 `anthropic`（来自环境变量）。
- 全局提示词：能读到 `~/.omo/agent/AGENTS.md`（存在时显示为 `default` 桥接态）；apply 后文件内容变化。
- 其他配置：改一个共享键并失焦 → 保存成功；`omo.jsonc` 的 `[native]` / `[opencode]` 块与注释逐字未变。
- 预览配置：`omo.jsonc → [native]` 标签正确；文件里没有 `[native]` 块时显示解释文案而不是 `{}`。
- apply 一个方案后：`~/.omo/omo.jsonc` 的 `[opencode]` 块与全部注释逐字未变，只新增/修改了 `[native]` 块。
- 反向验证：在 opencode tab 改 OMO 配置 → `[native]` 块不受影响。
- 会话管理能列出 `~/.omo/agent/sessions/` 下的记录并打开详情。

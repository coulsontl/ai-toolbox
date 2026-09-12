# Codex 前端模块说明

## 一句话职责

- `codex/` 页面负责 Codex provider/common config、根目录管理、prompt、plugin 与导入交互。

## Source of Truth

- 根目录来源于后端 `getCodexRootPathInfo()`，并决定页面实际针对哪份 `config.toml` / `auth.json` / active global prompt 文件工作。
- provider 最终生效状态以后端应用结果为准，前端本地状态只是展示。
- prompt 管理最终作用的是当前根目录下的 Codex active global prompt 文件。后端会按 upstream 语义优先使用非空 `AGENTS.override.md`，否则使用 `AGENTS.md`。
- 历史同步入口作用于后端解析出的 Codex history source。会话管理来源为 `all` 时，历史同步这种写操作默认落到本机；当前 Codex root 本身是 WSL Direct 时只作用于该 WSL root。前端只展示后端状态和触发命令，不自行推断数据库或 session 文件格式。

## 核心设计决策（Why）

- Codex 与 Claude Code 一样使用共享根目录编辑逻辑，保证 `custom/env/shell/default` 语义一致。
- provider 导入同样先做 `sourceProviderId` 冲突判断，避免重复导入同一来源时形成歧义。
- 页面操作后需要显式 `refreshTrayMenu()`，因为托盘是另一套消费者，不能假设 React 页面重绘就等于托盘已刷新。
- provider 表单的模型获取分两条链路：官方订阅读取后端共享模型目录，自定义网关继续走通用 `fetch_provider_models`；不要让官方模式依赖 Base URL/API Key 输入。

## 关键流程

```mermaid
sequenceDiagram
  participant Page as CodexPage
  participant Api as codexApi
  participant Modal as RootDirectoryModal

  Page->>Api: getCodexRootPathInfo + load config
  Page->>Modal: edit root directory
  Modal->>Api: saveCodexCommonConfig
  Api-->>Page: reload config
  Page->>Api: refreshTrayMenu
```

## 易错点与历史坑（Gotchas）

- 不要把页面上的 root path 只当展示信息。它直接决定当前读写哪份 `config.toml` / `auth.json` / active prompt 文件。
- 导入 provider 时的冲突分支、favorite provider 备份和 tray refresh 是一组相邻语义，改一个时通常要一起检查。
- 前端表单不要引入比后端更强的 paired validation，尤其是可选字段和导入数据兼容性相关字段。
- 普通“新建 provider”和“复制已应用 provider”都应走普通创建语义，默认不自动应用；不要因为复制源当前已应用，就在提交对象或页面状态里把新记录当成已应用配置处理。
- 页面里的 `__local__` 不是普通新增 provider，而是当前生效本地配置的收编入口；当用户把它保存为正式 provider 时，产品语义是“把当前生效配置正式落库”，不是“基于当前配置再新建一个未应用草稿”。
- UI 上 `__local__` 即使后端 `isApplied=true`，也不要显示「已应用」标签、选中高亮或「应用」按钮；只保留本地来源提示。用户应通过编辑后保存收编入库，再进入正式 applied 管理语义。
- `__local__` 还没有正式 provider 数据库记录，不能进入依赖持久化 provider ID 的官方账号管理链路；页面应先让用户保存收编，再展示或调用官方账号接口。
- 官方订阅模型列表只是辅助填写 `model` 字段。账号套餐、quota 和真实可调用性以 Codex 官方账号明细/运行时请求为准，前端不应在模型下拉阶段做额外拦截。
- 官方账号额度窗口由后端解析并投影；页面只展示返回的 `5h`、weekly、monthly 明细，不根据套餐、字段顺序或文案自行推断。
- provider 模式只允许在空白新增 provider 时选择。模式入口并入表单顶部“渠道”选择行：空白新增可在“自定义/官方/内置渠道”之间切换；复制 provider 仍走创建新记录语义，但必须沿用源 provider 的 `category`；编辑已保存 provider 也必须保留既有 `category`，不要允许官方/自定义互相切换。
- 自定义模式下的内置供应商 endpoint 会填入 Base URL、API 格式和模型映射，但只锁定 API 格式，不锁定 Base URL；保存内置 endpoint 时只写 `meta.gatewayProfile={tool:"codex",profileId,endpointId}` 引用，`settingsConfig.config` 中的 `base_url` 必须使用用户当前表单里的 Base URL。切回普通“自定义”时必须清掉 `gatewayProfile`，只保留用户手动选择的 `apiFormat`；不要把 `providerType` / `apiKeyField` / `reasoningField` / `defaultMaxTokens` / 图片策略这类 profile 派生快照写进 provider meta。
- OpenAI Chat/Responses 的 Base URL 通常需要 `/v1` 后缀（例如 `https://api.openai.com/v1`）。表单只做文案提示 + 保存软确认（对齐 Grok / OpenCode）：`apiFormat` 为 `openai_chat`/`openai_responses` 且 base 去尾 `/` 后不以 `/v1` 结尾、也不是 `##` full URL 时 `Modal.confirm`；**禁止**静默自动补 `/v1`（会破坏 Anthropic 类、Gemini Native、反代前缀与 full-URL 配置）。
- 内置供应商 endpoint 的 Gateway meta 不只包含 `providerType` / `apiFormat`。如果 profile endpoint 带 `reasoningField` 或 `codexChatReasoning`，Codex provider 表单也只保存 `gatewayProfile` 引用；runtime 每次按当前 profile catalog 动态解析 effective meta，保证 ReasoningField 和 Codex -> Chat 多 vendor reasoning/thinking 矩阵跟随内置 catalog 更新。切回官方或普通自定义时不能伪造这些 provider 专属字段。
- 自定义 Codex provider 的“模型映射”入口放在“获取模型”按钮后面；打开表单时如果 `settingsConfig.modelCatalog.models` 有有效模型则默认展开。手动模式下 API 格式不是默认 `openai_responses` 时也要自动展开模型映射区域，切回默认格式不自动收起，避免覆盖用户手动状态。官方订阅模式不保存模型映射。
- “导入模型映射”按钮复用共享 `FetchModelsModal`（与 OpenCode/Grok 等页面同源）按勾选合并映射行，合并逻辑在 `utils/codexCatalogModels.ts` 的 `importModelsIntoCatalog`：合并后**整表按弹窗的分组显示顺序（`orderedModelIds`）重排**，不在本次接口返回中的行保持相对顺序垫底；已存在的 model id 不重建（保留用户自定义，弹窗对已存在 id 禁用勾选）；`removedModelIds` 只在用户显式勾选“移除已下线”时非空，不得做隐式全量替换；新行只按 preset **精确**匹配补 `contextWindow`，`reasoningLevels` 故意不写——生成 catalog 时省略即默认全档，禁止替用户预设档位子集。Codex 通过 `priorityOwnedBy={['openai']}` 把 openai 渠道置顶（排序工厂在共享弹窗的 `sort.ts`，默认无置顶；该偏好仅限 Codex 表单，不得写进共享组件默认行为）。
- Codex 的 `settingsConfig.modelCatalog` 由 `useCodexConfigState` 的 `codexCatalogModels` 状态持久化，不是普通 Form 字段。内置 endpoint 提供 `modelCatalog` 时，应同步该 hook 状态；不要用 `form.setFieldValue('modelCatalog', ...)` 伪造保存。
- Codex 的“模型名称”与“模型映射”是两份独立数据：前者是 `config.toml` 的当前默认 `model`，后者只生成 `model_catalog_json`。不得用映射第一项填充、覆盖或兜底模型名称；保存时应基于已校验的表单快照直接构建 `settingsConfig`，不要先异步 `setState` 再立即读取 hook 闭包。
- `modelCatalog.models` 不是只给模型下拉用的三字段列表。`supportsImage`、`vision`、`attachment`、`modalities` 这类能力字段会被 Gateway runtime 用来判断 text-only/vision 行为，前端解析和保存规范化时必须保留显式 boolean（尤其是 `false`）和 `modalities.input/output`，不能只写回 `model/displayName/contextWindow`。
- 自定义 provider 的「自动审批模型」是 provider 级单值（`settingsConfig.autoReviewModelOverride`），UI 放在模型映射面板内、「添加模型映射」按钮下方，不是映射表格的每一行。只在自定义模式且展开模型映射时可见；留空表示不覆盖。可提供“设为自身”把当前默认 `model` 填入该字段（不改主模型）。后端 apply 时默认投影到 catalog 全部 entry；若默认 model 不在映射中会自动补入，确保 Codex 能读到 override。不提供应用范围选项。
- Codex 内置 Anthropic/Claude 协议 endpoint 如果没有显式 `modelCatalog`，添加供应商时应从同一渠道的 Claude endpoint 角色模型派生初始模型映射；如果 endpoint 自带 `modelCatalog`，仍以 endpoint 自身目录为准。派生逻辑只用于补齐添加表单的初始值，不能改变 Base URL 可编辑和保存用户当前输入值的语义。
- Gateway 现在是 direct → single → failover 三态。single 入口在已应用 provider 卡片的“网关代理”按钮；single/failover 接管期间都必须锁定其他 provider 的“应用”入口，failover 时卡片额外显示 P0/P1 优先级，切 P0 必须先恢复直连。
- 前端不要假设 Codex prompt 文件名永远是 `AGENTS.md`。展示路径、删除已应用 prompt 后的刷新和同步结果都以后端返回/事件为准。
- 插件页的全部启用/全部禁用只作用于“已安装”Tab 中当前 runtime 的已安装插件，不作用于市场可安装列表；全部启用需要允许后端同时开启 plugins feature，成功后仍按现有规则提示用户重启 Codex。
- 市场添加入口与 Grok/Claude 对齐：marketplaces Tab 工具栏与空态各有“添加”按钮，打开带文本输入框的 Modal。输入框接受 git 仓库 URL / GitHub `owner/repo` 简写 / `marketplace.json` 直链 / 本地目录；“选择目录”按钮把本地目录回填到输入框（不直接提交）。提交统一走 `addCodexPluginWorkspaceRoot({ path })`，由后端识别源类型并下载/克隆；失败时 Modal 保持打开（`marketplace-add-failed`）。不要为本地目录再加单独的直选入口，避免重复。
- 历史同步入口放在会话管理区域标题栏右侧，不属于 provider 卡片或已应用 provider 菜单；同步和恢复都是高影响本地写操作，必须使用后端返回的来源、统计、备份路径和锁等待信息展示结果，恢复最新备份必须强确认。历史同步默认只修复 provider 路由，不应在 UI 文案中承诺会同步或改写 model。
- “统一 Codex 会话历史”入口属于 Codex 更多选项，不属于手动历史同步弹窗；开关行应沿用 SidebarSettingsModal 的左右布局，说明文字放在 Switch 下方。开启确认里“迁入现有官方会话历史”默认不勾，关闭确认里只有存在当前 Codex root 的迁移账本时才提供按账本恢复；Gateway 接管期间前端应禁用开关并提示先恢复直连。
- “切换第三方时保留官方登录”同样在更多选项里，但语义不同：它会影响当前已应用渠道的 live 投影，不能只写 settings store。前端必须走 `setCodexPreserveOfficialAuthOnSwitch` → 后端 `set_codex_preserve_official_auth_on_switch`，由后端重投影（未接管直接 apply；Gateway 下 restore → apply → re-engage）。不要前端自己 `saveSettings` 或拼 Gateway 开关顺序。
- 记忆管理（`CodexMemoriesPanel`，issue #296）挂在 Codex 页 sidebar 第 5 分区（`codex-memories`），UI 形态对齐会话管理：本机/WSL 来源切换（复用 `sessionManager.sourceMode.*` 文案）、相对路径面包屑导航、左侧紧凑表格 + 右侧详情分栏。
- memories 来源切换后 `currentDir`、选中文件和批量选择必须整体重置；本机来源不可用时禁用本机选项并自动落到 WSL，必须同时更新真实 `sourceMode` 与 remembered mode，不能只从当前列表派生 effective mode，否则 WSL 读取失败清空列表时会反复切回本机并重试。异步列表仅允许当前来源/目录的最新请求更新结果，旧写操作回调不得发起跨上下文刷新；保存、重命名、删除的回显还需核对当前文件，避免旧结果覆盖新选择。手动切换来源立即使旧请求失效，同来源根目录的重复点击不得取消正在加载的请求；WSL 选项只显示 "WSL"，不带 distro 名。
- `list` 响应的 `unavailable=true` 表示请求来源在本机不存在：列表区显示“来源不可用”空态并禁用新建/打开/删除/清空入口，来源切换仍可用；不要把该状态当成加载失败弹错误。页面级 `refreshToken` 变化只刷新列表，不得重置用户的目录导航与选中状态。
- `MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/*` 的详情区要显示“Codex 会自动重建”辅助提示（10px tertiary 文字）；空态必须区分“目录为空”（表格内 Empty）和“未选中文件”（详情区 Empty）两种。
- 编辑保存走 `write_codex_memory_file`：.md 预览用共享 `MarkdownPreview`，编辑统一用 `MarkdownEditor`（自带编辑/预览切换），非 .md 只读预览用 `PlainTextEditor`。文件大小/字符数显示在详情头部，不要弹窗编辑。
- 新建记忆文件必须传 `createNew: true`，由后端拒绝重名，不能复用默认覆盖保存；加载失败时来源能力尚未知，允许用户切换来源重试，不得把错误状态当成两个来源都不存在。

## 跨模块依赖

- 依赖共享 `RootDirectoryModal` / `useRootDirectoryConfig`。
- 依赖后端 `codex::commands` 和共享 favorite provider、All API Hub 组件、以及共享 `ImportFromCcSwitchModal`（CC Switch 只读导入；无 db 不显示按钮；Codex TOML 经白名单重建后写入）。
- 间接受 `settings/` 和 `runtime_location` 的 WSL Direct 语义影响，但页面本身只显示 path info。

## 典型变更场景（按需）

- 改根目录逻辑时：
  同时检查页面顶部 path info、modal 回填、历史同步目标和保存后 reload。
- 改 provider 删除/导入时：
  同时检查冲突处理、favorite provider 兜底和 tray refresh。
- 改会影响 live 投影的更多选项开关时：
  走专用后端命令重投影当前已应用渠道，不要只写 settings；参考「保留官方登录」。

## 最小验证

- 至少验证：修改根目录后页面重新读取到新的路径来源。
- 至少验证：导入同源 provider 冲突时有明确覆盖/副本分支。
- 若本机存在 `~/.cc-switch/cc-switch.db`，Codex 页应出现「从 CC Switch 导入」；导入后带 `sourceProviderId=ccs:codex:...`、默认未应用；MCP-only 空壳不应进入列表。
- 改历史同步 UI 时，至少验证会话管理标题栏入口、来源切换、本机/WSL 状态弹窗、同步确认、恢复最新备份强确认和 Session Manager 刷新触发。
- 改「保留官方登录」开关时，至少验证：切换后不必再手动应用渠道；store 仅在后端成功后更新；失败不误翻转 UI。
- 改统一会话历史 UI 时，至少验证开启/关闭确认弹窗、迁移/恢复结果提示、Gateway 接管禁用态和 settings store 刷新。

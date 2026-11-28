# Plugins 前端模块说明

## 一句话职责

- `web/features/coding/plugins/` 提供顶栏独立入口 `PluginsButton` 与 `/plugins` 左侧可折叠菜单工作台（已安装 / 渠道与账号 / 插件市场 / 运行时与日志 + 插件注册的自定义页面），并协同 `web/features/coding/shared/` 为全部 CLI Tab 提供插件渠道导入、卡片徽标、模型同步与网关代理门控。

## Source of Truth

- 插件列表、自定义市场源、Bun 运行时状态、宿主状态、中间件性能统计、Provider 列表（含持久化 `optionValues`）与账号摘要全部以后端 `plugins_*` Tauri 命令（封装在 `web/services/pluginsApi.ts`）为准，前端不自行持久化插件配置或凭据。
- 顶栏「插件」入口可见性由全局 `visibleTabs` 的 `'plugins'` 键决定（设置页「模块显示」位于右侧 `OTHER_TABS`，不进 `CODING_TABS`，也不进 WSL/SSH runtime 同步集合）。隐藏入口仅隐藏 UI 并从 `/plugins*` 重定向，不卸载、不停用任何插件。
- CLI 卡片识别插件来源的唯一纯函数源是 `web/features/coding/shared/gateway/providerProtocol.ts`（`getPluginProviderRef` / `isPluginBackedProvider` / `extractPluginProviderIdFromUrl` / `isPluginRoutedProvider`）与 `web/features/coding/shared/plugins/pluginCardMeta.ts`。

## 核心设计决策（Why）

- **遵循 `DESIGN.md` 桌面工作台视觉体系 + Magpie 局部主题（强调色统一为全局主色蓝）**：插件中心使用 `styles/pluginTheme.less` 的 `.pluginThemeLight()` / `.pluginThemeDark()` mixin，把 `--pl-*` 令牌挂在 `PluginsPage` 根节点；市场详情弹窗因 Portal 到 `document.body`，必须在自己的 `rootClassName` 上再挂同一对 mixin。`--pl-accent*` 必须复用 `var(--ant-color-primary*)` / `var(--color-primary)` / `var(--color-bg-selected)` 并用 `color-mix()` 派生淡底与描边，不得再为插件页引入靛紫系局部强调色。业务样式统一使用 `var(--pl-*)`、`web/App.css` CSS 变量与 `color-mix()`，确保亮色/暗色/system theme 一致；不堆砌卡片套卡片、渐变装饰或营销 hero 区。
- **`/plugins` 是左侧可折叠菜单 + 右侧主舞台**：`PluginSidebar` 宽 216px、折叠 56px，折叠态持久化到 `localStorage`，窗口 `< 860px` 自动折叠并按需浮层展开；管理项与自定义页面分组渲染，无页面图标时用「标题首字圆角方框」降级。`/plugins` 路由必须配置 `chrome: { contentPadding: 'none' }`，页面根节点用 `height: calc(100vh - var(--content-top-offset))` 铺满顶栏以下的整个视口：外层 `MainLayout` 不再补 24px 内边距，页面自身的 12px padding 与 `--pl-bg` 直接构成工作台留白，也不能再写成 `- 48px`（会重新露出 `contentArea` 的白底并产生一圈白边）。管理视图在 `.stageManage` 内滚动，页面视图由 `PluginPageFrame` 填满，粘性筛选栏依赖根部 `--plugins-header-height: 0px`。
- **自定义页面路由不占用管理 Tab**：`/plugins/page/:pluginId/:pageId?` 由 `resolvePluginsRoute` 解析，页面 id 缺省时取该插件第一页；`routeConfig` 继续用 `/plugins/*` 覆盖，不新增 KeepAlive 路径。
- **托盘「插件：渠道与账号」直达账号页**：托盘菜单仅在 `visible_tabs` 含 `plugins` 时显示该项，点击后由后端 `tray-navigate` 事件携带 `/plugins/accounts` 交给 `MainLayout` 导航；轻量模式下后端 pending 路由由冷启动 drain，且阻止「恢复上次 Tab」覆盖。该入口复用普通路由，不新增 KeepAlive 路径或插件页面逻辑。末尾 `  ›`（两个空格 + 箭头）是标签文本的一部分：原生菜单项只有做成子菜单才有真正的右对齐箭头，而那样会变成两次点击；该箭头只跟在文字后面，不贴菜单右边缘。
- **页面宿主只经 Magpie Bridge v1 访问宿主能力**：`PluginPageFrame` 用 `sandbox="allow-scripts"` 渲染 iframe，严格校验 `event.source === iframe.contentWindow` 与 `msg.magpie === 1`；`hello` 返回动态读取的 AI Toolbox 主题 tokens（含 `font` / `code` 字体栈），`options.*` / `read` / `image.*` / `plugin.open` 分别落到对应 `plugins_*` Tauri 命令，页面未握手前拒绝其它请求，10 秒（仅可见时计时）未握手或页面报 `error` 时显示错误边界。iframe 首次 `onLoad` 后，若 `frameKey` / `sessionUrl` 未变化却再次触发 `onLoad`（外链点击或脚本重定向），立即清空握手、断开桥接并卸载 iframe，必须重新加载才能恢复原始插件页。`image.copy` / `image.save` 同时接受 base64 字符串与 `Blob`（`FileReader` 转 dataURL，上限 20 MiB），作为纵深兼容。
- **页面保活与回收**：`PluginsPage` 同时渲染管理舞台与页面舞台，仅用 `display:none` 切换；iframe 可见性由 `isPageRoute && useKeepAlive().isActive && key === activePageKey` 决定。切到管理 Tab 或离开 `/plugins` 时 iframe 不卸载，只推送 `visibility:false`；`keepAlive !== true` 的页面隐藏 10 分钟后卸载并调用 `closePluginPage` 释放后端 token；插件禁用/卸载（`plugins_list_pages` 不再返回）时立即销毁对应 iframe，并同样关闭页面会话。4 个管理 Tab（`installed` / `accounts` / `market` / `runtime`）采用首次访问懒挂载 + `display:none` 保活，`PluginRuntimeView` 的 5 秒轮询绑定 `active` 切走即停；`pluginNavigation` 记忆上次访问的 `/plugins/...` 子路径，顶栏 `PluginsButton` 切回时恢复原停留子页、已在 `/plugins` 内不重置；4 个管理页均在页内工具栏提供显式刷新按钮，不依赖侧栏底部小图标。
- **`magpie.actions` 经页面桥的 `action` 事件派发**：渠道头部的「插件动作」菜单只列出 `menu === 'provider'` 的 `RegisteredPluginAction`；触发时 `PluginsPage` 以自增 `seq` 记录 `(pluginId, pageId)` 的最新 `pendingAction` 并导航到 `getPluginsCustomPagePath`。`PluginPageFrame` 只在 `handshakeDone` 后按 `seq` 去重派发 `{ magpie: 1, nonce, event: 'action', data: { id, provider? } }`，与 `page_bridge.js` 的 `onAction` 契约一致；交付后回调父级清除队列，防止重新打开页面时重放旧动作。`listPluginActions` / `listPluginPanels` 与 `plugins-updated` 一起静默刷新，命令缺失时保持空列表，不得把失败计入页面级 `loadError`。
- **`magpie.panels` 只挂 `provider` / `settings` 两个稳定插槽**：`PluginPanelSlot` 按 `slot` 过滤，无匹配面板时直接返回 `null`（零 DOM、零 `openPluginPanel` 请求）；每个面板独立调用 `openPluginPanel` 并在卸载时 `closePluginPanel`，用 `sandbox="allow-scripts"` 与 `PluginPageFrame` 相同的 Bridge v1 握手，`context` 必须包含 `{ pluginId, panelId, slot, ...contextExtra }`。面板接收 `size` 事件（无 nonce，靠 `event.source` 校验）并把高度限制在 `80..720px`、默认 `220px`；provider 上下文变化时以 `contextExtra` 作为 key 重挂载，确保 `magpie.context` 与当前渠道一致。
- **异步 `magpie.hooks` 统计是可选诊断**：`PluginRuntimeView` 在 `loadAll` 中与运行时状态并行静默拉取 `getPluginGatewayHookStates()`，失败或空数组一律降级为 `[]` 且不写入页面级错误；仅当结果非空时在「网关中间件状态」下方用同一 `statsTable` 样式展示插件名、调用、失败、平均耗时（ms）与最近结果码。
- **侧边栏「渠道与账号」徽标 = 已登录账号数**：直接使用 `listPluginAccounts()` 返回的 `accounts.length`，与右侧每个渠道胶囊的账号计数语义一致，不依赖 Bun 宿主或 provider 数量。
- **账号额度缓存秒开与自动拉取**：`PluginAccountsView` 收到 `listPluginAccounts()` 时立刻将持久化的 `account.quota` 水合到界面；当页面可见且所选渠道声明 `usage` 时，对当前会话尚未拉取过的已启用账号自动静默拉取一次额度（拉取中且无缓存显示加载态而非「尚未读取额度」），切换 Tab 保活不丢失。
- **选中渠道头部固定两行**：`.detailHeader` 为纵向 flex。第一行 `.detailHeaderMain` 放图标、渠道名/`providerSpec`/账号数，以及唯一实心主按钮「登录账号」（`margin-left` 由 `flex:1` 的标题自然推开）；第二行 `.detailActions` 用 `1px solid var(--pl-divider)` 细线分隔，放「添加到 CLI」「刷新模型」「刷新额度」「一键签到」和插件动作菜单，`自动签到` Switch 用 `margin-left: auto` 靠右。这样按钮不会再和标题挤在同一行导致频繁换行；所有操作保持可见，不收进下拉菜单。`<760px` 时 `自动签到` 取消自动外边距，随按钮组左对齐换行。
- **自动签到是渠道级开关**：`selectedProvider.checkin` 为真时，只在渠道头部第二行操作栏的右端展示一个「自动签到」Switch，经 `plugins_set_provider_auto_checkin` 持久化；账号行不再出现该开关。后端 `plugins_list_providers` 返回 `autoCheckin`，并对未显式配置的渠道兼容旧账号级 `auto_checkin` 标记。账号行继续保留各自的「立即签到」图标和 `lastCheckinOutcome` / `lastCheckinAt` 尾注。
- **能力矩阵与账号约束只做声明式提示**：`InstalledPluginsView` 仅在 `plugin.contributions?.unsupportedFeatures` 非空时渲染一个克制的 warning 胶囊 + `Tooltip`，逐项列出本地化能力名（`hooks` / `panels` / `actions` / `replaces` / `agent` / `privacyModels` / `search`，未知 key 原样显示），能力名必须用字面量 `t('plugins.installed.capabilities.*')` 分支解析；`PluginAccountsView` 在 `provider.single === true` 且该渠道已有 ≥1 个账号时禁用「登录账号」入口并用 `Tooltip` 说明单账号约束；`outputsSaid && !(outputs ?? []).includes('text')` 的模型显示中性「非对话」徽标，说明并入模型胶囊既有 `Tooltip`，不嵌套 Tooltip。
- **已安装行的操作区位于标题行**：`InstalledPluginsView` 的 `.rowActions` 必须渲染在 `.rowHeading` 内并通过 `margin-left: auto` 右对齐，这样下方的 `.rowStatus`（渠道登录状态）可以使用整行宽度；不要把操作区移回 `.row` 的独立列，否则它会在整行高度上预留宽度，导致状态列表在右侧明显有空位时仍提前换行。
- **插件工具 MCP 信息条严格按需展示**：`InstalledPluginsView` 仅在 `listPluginTools()` 返回 `tools.length > 0` 时渲染紧凑 MCP 信息条；该命令失败必须静默降级为 `null`，不得阻断已安装列表。信息条展示工具总数、只读等宽 `endpointUrl` 与网关未运行提示；「复制地址」走 `copyTextToClipboard`，「同步到 MCP 管理」调用 `syncPluginToolsMcpServer()` 成功后静默重取摘要，`mcpServerId` 已存在时按钮改为「更新 MCP 配置」并附「已同步」徽标。工具摘要随已安装列表的每次刷新（挂载、手动刷新、启停、卸载、更新、`plugins-updated`）静默重取；单行工具数胶囊按 `pluginId`（回退 `pluginSpec`）聚合，`Tooltip` 最多列 8 个 `mcpName` + 简短描述，超出显示剩余数量。
- **国内镜像开关两处共享同一份 `PluginSettings.npmRegistry`**：`PluginsPage` 加载 settings 并用 `handleToggleChinaMirror` 立即 `savePluginSettings`，再把 `pluginSettings` / `chinaMirrorBusy` / `onToggleChinaMirror` 传给「插件市场」「运行时」；`isChinaMirrorRegistry` 负责判定，运行时页同步草稿与已保存基线。用户手填自定义 registry 时开关显示关闭，运行时页保留原 URL。已安装页不再携带镜像开关与工具栏「安装插件」按钮（用户要求精简，安装入口保留在市场页、侧栏「安装插件」和空态「手动安装」）。
- **自动更新是运行时全局设置，检查结果由 `PluginsPage` 共享**：`PluginRuntimeView` 只负责编辑 `autoUpdateEnabled` / `autoUpdateIntervalHours`（关闭时禁用间隔选择）并展示 `lastAutoUpdateCheckAt`；`PluginsPage` 持有 `updateItems` 与 `checkingUpdates`，把 `checkPluginUpdates()` 的结果同时供给 Runtime（toast）与 Installed（徽标/更新按钮），避免两个视图各查一次导致版本数据不一致。`lastAutoUpdateCheckAt` 是后端只读字段：`savePluginSettings` 强制传 `null`，检查结果只在前端用 `summary.checkedAt` 同步显示。
- **`plugins-updated` 只做静默刷新**：桌面按钮、后台自动更新、Control API 触发的更新都会发该事件；`PluginsPage` 监听后按 `updatedIds` 清除对应徽标，并用 `Promise.allSettled` 并发重拉 installed / settings / pages，失败不弹全局 toast。事件本身不代表用户操作，不要用它触发通知。
- **市场页固定三列卡片 + 详情弹窗**：桌面 `repeat(3, minmax(0, 1fr))`，960px 两列、640px 单列；整卡点击打开 `PluginMarketDetailModal`（安装按钮阻止冒泡），卡片只保留图标、名称、来源、两行简介、类型与版本，长包名、provider 标签、npm 链接和安装命令移入 560px 详情弹窗。
- **市场工具栏只留一级筛选，次级操作收进「更多」菜单**：工具栏只保留类型分段器、市场源 Select、搜索框、刷新图标按钮、`MoreHorizontal` 更多按钮和总数；「包含 npm 在线搜索」「国内镜像」「管理市场源」放进下拉菜单（前两项是可勾选菜单项，勾选态用 `Check` 图标而非仅靠颜色）。刷新是高频操作，始终以独立图标按钮留在工具栏，不埋进「更多」。刷新按钮用 `margin-left: auto` 与更多按钮、总数组成右侧操作簇；`<720px` 时取消该外边距，让 Select/搜索框独占整行换行。不要把其余次级操作重新铺回工具栏——常见窗口宽度下会折到第二行，这正是本次改版的起因。
- **轻量持久化只经 Bridge 转发**：`storage.*` 与 `files.*` 分别调用 `plugins_page_bridge_kv` / `plugins_page_bridge_files`；前端先校验 `key` / `path` / `content` 的基本类型，权限由 Rust 按 `storage:kv` / `storage:files` 强制校验，宿主不缓存也不读取插件数据。
- **卸载默认保留插件数据**：`uninstallPlugin(id, { purgeData })` 的 `purgeData` 默认 `false`；`InstalledPluginsView` 用 Modal + 默认未勾选的 Checkbox 让用户显式选择是否同时删除 KV 与文件。
- **Sticky 页头高度实测发布**：`PluginsPage` 与 `GatewayPage` 一致，用 `ResizeObserver` 实测顶栏高度并发布 `--plugins-header-height` 供子 Tab 筛选栏吸附，KeepAlive 隐藏量到 0 时保持上次有效值，防止两层粘性栏错位或留缝。
- **17 个 CLI Tab 通过共享层一次性接线**：插件徽标（`ProviderPluginBadge`）与「同步插件模型 / 管理插件账号」菜单项统一落在 `shared/providerCardVariants/`、`shared/providerCardView/ProviderCompactCard` 与 `shared/ProviderListSection`（`PluginProviderCardContext`），不新建第四种卡片布局，继续由 `scripts/verify-provider-card-layout.mjs` 机械守护。
- **插件渠道强制网关代理门控**：`providerNeedsGatewayProxy` 与 `grokProviderNeedsGatewayProxy` 对插件渠道无条件返回 `true`，阻止用户误点“恢复直连”把占位密钥/插件配置写成无效直连配置。
- **AddPluginToCliModal 账号策略按目标模式展示**：仅当所选目标含 `gateway_takeover` 且渠道有多个账号时显示策略选择器；`direct_plugin_route` 永远按 `auto_rotate` 提交，纯直连选择只提示自动轮询，混合选择时说明策略仅作用于网关接管型 CLI。
- **前后端 `cliKey` 统一使用 coding 子 Tab 键**：调用 `plugins_add_provider_to_cli` 与 `plugins_sync_cli_provider_models` 时统一传 `claudecode` / `claudedesktop` / `geminicli` / `workbuddy` / `codebuddy` 等 `MODULES` subTab key，不得混传网关 `app_type`（如 `claude` / `workbuddy_desktop`）。

## 易错点与历史坑（Gotchas）

- **Provider 声明式开关必须以持久化值覆盖默认值**：`PluginAccountsView` 初始化选项开关时必须按 `{ ...声明默认值, ...provider.optionValues }` 合并，确保用户显式关闭（`false`）能覆盖插件声明的 `default: true`，避免重进页面开关回弹。
- **KeepAlive 隐藏页后台刷新不弹全局 toast**：页面与子 Tab 的后台数据加载统一使用 `Promise.allSettled` + 内联错误展示；仅用户显式触发操作才弹 `message`，日志自动轮询必须在 Tab 非激活或页面不可见时暂停。
- **更新徽标与按钮复用既有视觉令牌**：`InstalledPluginsView` 的「有新版 vX.Y.Z」徽标用 `.updateBadge`（`--pl-accent-bg` / `--pl-accent-border` / `--pl-radius-tag`），行内「更新」用 `.textButton` + `.textButtonAccent`，工具栏用 `.actionButton`，不引入新的按钮尺寸或色板；`updateItems` 只保留 `status === 'available'` 的项，`failed` 项保留徽标以便重试，`updated` / `up_to_date` / `skipped` 由 `onUpdateSummary` 清除。更新期间不要乐观修改 `plugin.enabled`——后端会恢复原启用状态。
- **前端日志展示与复制前二次脱敏**：虽然 Rust 后端已对 `RuntimeLogEntry` 脱敏，前端展示与复制前仍经过 `pluginFormatters.ts` 二次兜底脱敏，防止插件异常输出泄漏凭据片段。
- **Antigravity 目标不可勾选**：后端 `plugins_list_cli_targets` 对 `antigravity` 返回 `supported: false` 与原因说明；`AddPluginToCliModal` 必须置灰禁用并展示原因，不得强行提交或静默过滤。
- **i18n 键禁止模板拼接**：所有动态标签分支必须使用字面量 `t('plugins....')`，保证 `pnpm run i18n:check` 静态扫描覆盖全部中英键且无假未用告警。
- **`contributions` 与 `single` 均为老记录兼容字段**：`InstalledPluginRecord.contributions` 缺失时按“无未支持能力”处理，不渲染任何提示；单账号禁用只依据 `provider.single` 与现有账号数，不从 `fellBack` / `listError` 推断。
- **`PluginIcon` 品牌匹配顺序敏感**：`opencode` 必须排在 `openai` 前，`qoder|qwenwork` 必须排在 `qwen` 前，`groq`（`\bgroq\b`）必须排在 `grok` 前；`ark`、`aws`、`poe`、`zai`、`meta` 等短词必须带词边界，避免误中 `metadata` 一类普通词。`workbuddy` 用本地 PNG、`factory[\s-]?droid` 或 `droid` 用 `droid.svg` 的 `currentColor`，其余品牌优先 `.Color`，无 `.Color` 的 mono 图标继承容器颜色自动适配亮暗主题。

## 跨模块依赖

- 依赖 `@/services/pluginsApi`、`MainLayout` 顶栏动作区、`routeConfig` KeepAlive 路由、`settingsStore.visibleTabs` 与 `GeneralSettingsPage` `OTHER_TABS`。
- 与 `web/features/coding/shared/`（`providerCardVariants`、`providerCardView`、`ProviderListSection`、`gateway/providerProtocol.ts`）双向协同；后端运行契约见 `tauri/src/plugins/AGENTS.md`。

## 最小验证

- `pnpm tsc --noEmit`
- `pnpm run i18n:check`
- `pnpm run test:provider-card-layout`
- `pnpm run test:css-module-classes`
- `node --test web/test/features/coding/shared/gateway/providerProtocol.test.ts web/test/features/coding/plugins/utils/pluginFormatters.test.ts`
- `pnpm build`

# Plugins 前端模块说明

## 一句话职责

- `web/features/coding/plugins/` 提供顶栏独立入口 `PluginsButton` 与 `/plugins` 左侧可折叠菜单工作台（已安装 / 渠道与账号 / 插件市场 / 运行时与日志 + 插件注册的自定义页面），并协同 `web/features/coding/shared/` 为全部 CLI Tab 提供插件渠道导入、卡片徽标、模型同步与网关代理门控。

## Source of Truth

- 插件列表、自定义市场源、Bun 运行时状态、宿主状态、中间件性能统计、Provider 列表（含持久化 `optionValues`）与账号摘要全部以后端 `plugins_*` Tauri 命令（封装在 `web/services/pluginsApi.ts`）为准，前端不自行持久化插件配置或凭据。
- 顶栏「插件」入口可见性由全局 `visibleTabs` 的 `'plugins'` 键决定（设置页「模块显示」位于右侧 `OTHER_TABS`，不进 `CODING_TABS`，也不进 WSL/SSH runtime 同步集合）。隐藏入口仅隐藏 UI 并从 `/plugins*` 重定向，不卸载、不停用任何插件。
- CLI 卡片识别插件来源的唯一纯函数源是 `web/features/coding/shared/gateway/providerProtocol.ts`（`getPluginProviderRef` / `isPluginBackedProvider` / `extractPluginProviderIdFromUrl` / `isPluginRoutedProvider`）与 `web/features/coding/shared/plugins/pluginCardMeta.ts`。

## 核心设计决策（Why）

- **遵循 `DESIGN.md` 桌面工作台视觉体系 + Magpie 局部主题（强调色统一为全局主色蓝）**：插件中心使用 `styles/pluginTheme.less` 的 `.pluginThemeLight()` / `.pluginThemeDark()` mixin，把 `--pl-*` 令牌挂在 `PluginsPage` 根节点；市场详情弹窗因 Portal 到 `document.body`，必须在自己的 `rootClassName` 上再挂同一对 mixin。`--pl-accent*` 必须复用 `var(--ant-color-primary*)` / `var(--color-primary)` / `var(--color-bg-selected)` 并用 `color-mix()` 派生淡底与描边，不得再为插件页引入靛紫系局部强调色。业务样式统一使用 `var(--pl-*)`、`web/App.css` CSS 变量与 `color-mix()`，确保亮色/暗色/system theme 一致；不堆砌卡片套卡片、渐变装饰或营销 hero 区。
- **`/plugins` 是左侧可折叠菜单 + 右侧主舞台**：`PluginSidebar` 宽 216px、折叠 56px，折叠态持久化到 `localStorage`，窗口 `< 860px` 自动折叠并按需浮层展开；管理项与自定义页面分组渲染，无页面图标时用「标题首字圆角方框」降级。页面根节点固定为视口高度，管理视图在 `.stageManage` 内滚动，页面视图由 `PluginPageFrame` 填满，粘性筛选栏依赖根部 `--plugins-header-height: 0px`。
- **自定义页面路由不占用管理 Tab**：`/plugins/page/:pluginId/:pageId?` 由 `resolvePluginsRoute` 解析，页面 id 缺省时取该插件第一页；`routeConfig` 继续用 `/plugins/*` 覆盖，不新增 KeepAlive 路径。
- **页面宿主只经 Magpie Bridge v1 访问宿主能力**：`PluginPageFrame` 用 `sandbox="allow-scripts"` 渲染 iframe，严格校验 `event.source === iframe.contentWindow` 与 `msg.magpie === 1`；`hello` 返回动态读取的 AI Toolbox 主题 tokens（含 `font` / `code` 字体栈），`options.*` / `read` / `image.*` / `plugin.open` 分别落到对应 `plugins_*` Tauri 命令，页面未握手前拒绝其它请求，10 秒（仅可见时计时）未握手或页面报 `error` 时显示错误边界。iframe 首次 `onLoad` 后，若 `frameKey` / `sessionUrl` 未变化却再次触发 `onLoad`（外链点击或脚本重定向），立即清空握手、断开桥接并卸载 iframe，必须重新加载才能恢复原始插件页。`image.copy` / `image.save` 同时接受 base64 字符串与 `Blob`（`FileReader` 转 dataURL，上限 20 MiB），作为纵深兼容。
- **页面保活与回收**：`PluginsPage` 同时渲染管理舞台与页面舞台，仅用 `display:none` 切换；iframe 可见性由 `isPageRoute && useKeepAlive().isActive && key === activePageKey` 决定。切到管理 Tab 或离开 `/plugins` 时 iframe 不卸载，只推送 `visibility:false`；`keepAlive !== true` 的页面隐藏 10 分钟后卸载并调用 `closePluginPage` 释放后端 token；插件禁用/卸载（`plugins_list_pages` 不再返回）时立即销毁对应 iframe，并同样关闭页面会话。4 个管理 Tab（`installed` / `accounts` / `market` / `runtime`）采用首次访问懒挂载 + `display:none` 保活，`PluginRuntimeView` 的 5 秒轮询绑定 `active` 切走即停；`pluginNavigation` 记忆上次访问的 `/plugins/...` 子路径，顶栏 `PluginsButton` 切回时恢复原停留子页、已在 `/plugins` 内不重置；4 个管理页均在页内工具栏提供显式刷新按钮，不依赖侧栏底部小图标。
- **侧边栏「渠道与账号」徽标 = 已登录账号数**：直接使用 `listPluginAccounts()` 返回的 `accounts.length`，与右侧每个渠道胶囊的账号计数语义一致，不依赖 Bun 宿主或 provider 数量。
- **账号额度缓存秒开与自动拉取**：`PluginAccountsView` 收到 `listPluginAccounts()` 时立刻将持久化的 `account.quota` 水合到界面；当页面可见且所选渠道声明 `usage` 时，对当前会话尚未拉取过的已启用账号自动静默拉取一次额度（拉取中且无缓存显示加载态而非「尚未读取额度」），切换 Tab 保活不丢失。
- **自动签到是渠道级开关**：`selectedProvider.checkin` 为真时，只在渠道头部操作栏、紧邻「一键签到」展示一个「自动签到」Switch，经 `plugins_set_provider_auto_checkin` 持久化；账号行不再出现该开关。后端 `plugins_list_providers` 返回 `autoCheckin`，并对未显式配置的渠道兼容旧账号级 `auto_checkin` 标记。账号行继续保留各自的「立即签到」图标和 `lastCheckinOutcome` / `lastCheckinAt` 尾注。
- **国内镜像开关三处共享同一份 `PluginSettings.npmRegistry`**：`PluginsPage` 加载 settings 并用 `handleToggleChinaMirror` 立即 `savePluginSettings`，再把 `pluginSettings` / `chinaMirrorBusy` / `onToggleChinaMirror` 传给「已安装」「插件市场」「运行时」；`isChinaMirrorRegistry` 负责判定，运行时页同步草稿与已保存基线。用户手填自定义 registry 时开关显示关闭，运行时页保留原 URL。
- **市场页固定三列卡片 + 详情弹窗**：桌面 `repeat(3, minmax(0, 1fr))`，960px 两列、640px 单列；整卡点击打开 `PluginMarketDetailModal`（安装按钮阻止冒泡），卡片只保留图标、名称、来源、两行简介、类型与版本，长包名、provider 标签、npm 链接和安装命令移入 560px 详情弹窗。
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
- **前端日志展示与复制前二次脱敏**：虽然 Rust 后端已对 `RuntimeLogEntry` 脱敏，前端展示与复制前仍经过 `pluginFormatters.ts` 二次兜底脱敏，防止插件异常输出泄漏凭据片段。
- **Antigravity 目标不可勾选**：后端 `plugins_list_cli_targets` 对 `antigravity` 返回 `supported: false` 与原因说明；`AddPluginToCliModal` 必须置灰禁用并展示原因，不得强行提交或静默过滤。
- **i18n 键禁止模板拼接**：所有动态标签分支必须使用字面量 `t('plugins....')`，保证 `pnpm run i18n:check` 静态扫描覆盖全部中英键且无假未用告警。
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

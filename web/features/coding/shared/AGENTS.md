# Coding Shared 前端模块说明

## 一句话职责

- `shared/` 提供多个 coding 页面共用的前端语义层，不拥有独立业务主数据，但封装了跨模块必须一致的交互规则。

## Source of Truth

- `shared/` 中的大多数组件只是消费各模块自己的 service/store，不能反过来成为业务事实源。
- 根目录来源、prompt 列表、session 数据、favorite provider、provider 诊断等真实数据都在各自 owning module 或后端命令。
- `shared/` 真正需要维护的是“相同概念在不同页面的统一解释”，例如 root path source、favorite provider storage key、session tool API 形态。

## 核心设计决策（Why）

- 供应商「分享」从当前记录或 runtime view 生成快照，复用 `deepLink/ProviderTransferModal` 和统一后端导入入口；新增卡片入口使用 `providerShare/useProviderSharing` 保持完成事件刷新。独立 API Key 才可迁移，官方 OAuth 不随分享导出；模型目录的不同连接分别分享。协议与边界见 `web/features/shared/deepLink/AGENTS.md`。

- `useRootDirectoryConfig` + `RootDirectoryModal` 把 Claude/Codex 的根目录编辑语义统一起来，避免两个页面对 `custom/env/shell/default` 的解释漂移。
- Claude/Codex/Grok CLI/Gemini CLI 复用共享根目录交互，而 OpenCode/OpenClaw 继续使用各自的配置文件路径弹窗；这是“根目录模块”和“文件路径模块”的前端分层，不要为了复用把两类语义硬揉到一个 modal 里。
- `favoriteProviders.ts` 用 source 前缀和 payload 约定把 OpenCode/Claude/Codex/OpenClaw 的收藏 provider 统一建模，避免不同页面各存一套不兼容 key。
- `GlobalPromptSettings`、`SessionManagerPanel`、`ProviderConnectivityTestModal` 等共享组件都要求业务方通过 service/api 注入，不自己硬编码某个模块的存储细节。
- Grok 连通性测试的 modelIds 只能是上游模型 ID。`settingsConfig.defaultModelKey` 是本地 catalog key（可能是 `custom`），不是 API model 名；必须从 `modelCatalog.models[].model` 取。
- `SessionManagerPanel` 的标题栏可通过 `extra` 注入模块自有动作；动作归 owning page 处理，shared 面板只负责摆放入口，不接管模块业务状态。
- `sessionManager/detail/` 是共享会话详情二级页和 workbench。它只消费后端 normalized message/block 契约，并通过 domain helpers 做搜索、过滤、导航、工具块配对和工具展示归一化；不要在 renderer 组件里直接读取某个 CLI 的 raw message shape。
- `allApiHub` 共享 modal 和模型缓存属于“共享交互层”，不是某个页面的私有实现。
- `gateway/providerProfiles.ts` 是前端 Gateway 内置供应商 profile 的共享内存态，启动时从后端缓存/bundled defaults 加载，再由远端刷新更新。它只提供 catalog、订阅和 endpoint 推断 helper，不持久化业务 provider。当前共享 tool key 覆盖 Claude Code、Claude Desktop、Codex、Grok CLI、Kimi Code 和 Gemini CLI；其中 Kimi Code 只是类型上被接受，bundled catalog 暂无 `tools.kimi` 节点（Kimi 暂无已验证内置 endpoint，后端 runtime 不解析其 gatewayProfile 引用）。新增内置 endpoint 时应按对应 `tools.claude` / `tools.codex` / `tools.grok` / `tools.gemini` 写入（为 Kimi 提供 endpoint 时需同步后端 `SUPPORTED_PROFILE_TOOLS` 与 catalog `tools.kimi`），而不是让页面靠模型名或 URL 猜供应商。Grok endpoint 默认机械复用已验证的 Codex OpenAI 兼容 endpoint 数据，协议差异继续由 Grok `api_backend` 和 Gateway transformer 处理。
- `management/` 下的控件和 `VirtualGrid` 只提供高密度管理页的纯 UI 行为，例如原生按钮、菜单、搜索、分段控件、空/加载态和可视区渲染；它们不保存业务选择、搜索、分组、排序或同步状态。
- `providerList/` 是全部 coding tab 供应商列表共享的搜索 / 排序 / 最近使用语义层（纯函数 + `useProviderListSort` + `ProviderSortDropdown` + `ProviderSearchInput`）。排序模式与最近使用时间存储在后端 settings 单例的 `provider_sort_modes` / `provider_last_used` 嵌套 key（`tauri/src/settings/provider_list_state.rs`，commands 为 `get_provider_list_state` / `save_provider_sort_mode` / `record_provider_last_used`），module key 沿用 `favoriteProviders.ts` 的 source 前缀约定。排序模式只是前端展示层操作，永远不改后端 provider 返回顺序；`custom`（默认）即拖拽落盘的 `sort_index` / 配置文件顺序。
- `management/useAutoGridColumns` 是排序模式复用浏览模式自适应列数的共享 hook：浏览分支由 `VirtualGrid` 内部按容器宽度算列数，排序分支（完整列表渲染、不走虚拟化）用「每行展示自动」(`gridColumnSetting === 'auto'`) 时没有那个计算，容易各自写死列数导致同一行卡片数在浏览/排序间漂移。该 hook 用 callback ref 挂 `ResizeObserver`，与 `VirtualGrid` 用同一套 `Math.max(1, Math.min(maxColumns, Math.floor((w + gap) / minColumnWidth)))` 公式，消费方把 `containerRef` 放到排序列表容器、用返回的 `columnCount` 写 `--management-grid-columns` CSS 变量即可。选 callback ref 而非 `useRef + useEffect`，是因为排序/浏览分支切换时容器是条件挂载、ref 在 layout 后才可靠；`enabled=false` 时不监听并返回 `undefined`，让固定列（`gridColumnSetting !== 'auto'`）路径完全不受影响。容器 CSS 的 `--management-grid-columns` 回退值要选接近宽屏 auto 结果的稳定常数（如 `repeat(3, minmax(0,1fr))`），避免 `ResizeObserver` 首个回调落地前首帧跳到 4 列再回跳。
- `magicContext/` 是 OpenCode 和 Pi 共用的 CortexKit 用户级配置管理入口。它只消费后端 `magic_context` 文件命令和调用方传入的安装状态，不拥有插件安装、扩展安装、项目级配置或配置主数据。
- `toolIcon/` 是 Skills 与 MCP 共用的工具品牌图标组件（见 `shared/toolIcon/AGENTS.md`）。它只做图标解析与暗色适配，不持有工具清单；Skills/MCP 都从共享路径导入，不再在各自模块内维护第二套品牌映射。

## 关键流程

```mermaid
sequenceDiagram
  participant Page as Tool Page
  participant Shared as shared/*
  participant Api as module service/api

  Page->>Shared: provide translation/service/tool context
  Shared->>Api: invoke module-specific operations
  Api-->>Shared: business data
  Shared-->>Page: consistent UI behavior
```

## 易错点与历史坑（Gotchas）

- 不要把 `shared/` 写成新的业务层。它应该统一交互语义，而不是偷存一份自己的持久化状态。
- 供应商批量选择的 `allIds` 必须由 owning page 按真实可删除条件过滤；卡片复选框统一消费 hook 的 `isSelectable`，避免全选排除了默认项、卡片却仍显示可选。进入选择模式时禁用供应商拖拽。确认执行时使用最新的可选集合和页面回调，防止弹窗打开期间切换默认项后误删。
- 批量删除回调只有整批成功才返回成功状态；失败时先刷新 owning module，再保留尚未删除的选择。绑定官方账号的 Codex/Grok/Gemini/Kimi provider 也应排除，不能让第一条后端拒绝中断整批后再静默清空选择。
- 收藏备份统一先经 `backupProvidersBeforeDelete` 完成整批准备；任一备份失败就停止删除并显示供应商名。不要复用吞掉异常的 best-effort wrapper，也不要在循环中一边删除一边取下一条备份；DSH 的多个渠道可能共享凭据，前一次删除会改变后一次读取的凭据状态。
- 供应商列表的"最近使用"写入约定：DB 型 tab（claudecode/claudedesktop/codex/grok/geminicli/kimi/openclaw）在各自后端 apply 汇聚函数内调用 `record_provider_last_used_in_sqlite_state`，覆盖托盘和窗口两条路径；配置文件型 tab（opencode/pi/omp/hermes/dsh）在托盘 internal 记录，窗口路径由页面在"设为默认模型/应用"成功后调 `noteProviderUsed`（内部走 `record_provider_last_used` command）兜底。前端重复记录与后端记录重叠是幂等无害的，页面接入时必须调用它以同步 hook 内存缓存。
- 非 `custom` 排序模式或搜索词非空时必须禁用拖拽：DB 型页面传 `sensors={[]}` 给 `DndContext`，共享 `ProviderCard` 传 `draggable={false}`（同时隐藏把手）。否则过滤/排序后的卡片顺序与 `sort_index` 错位，拖拽会把错误顺序写回后端。
- 在 Collapse `extra` 里放 antd Dropdown 时，只在触发按钮上 `stopPropagation` 不够：菜单浮层虽 portal 到 `document.body`，但 React 合成事件沿**组件树**冒泡（portal 的 React 祖先链），menu item 点击仍会触发 Collapse header 的 onClick 导致面板收起。必须同时在 menu `onClick` 里对 `domEvent.stopPropagation()`（`ProviderSortDropdown` 还在外层包了一个 stopPropagation span 双保险）。这是通用坑，不只针对排序菜单。
- antd 6 的 `Button size="small"` 默认字号是 `token.fontSize`（14px），不是 12px（`button/style/token.js` 中 `contentFontSizeSM ?? token.fontSize`）。供应商区标题 extra 的 link 按钮族约定显式 `style={{ fontSize: 12 }}`（DESIGN.md 紧凑层级）；给某页 extra 新增按钮时，若该页原有按钮漏写该样式，先补齐再保持整组一致，否则同组按钮会出现 12px/14px 混排。
- `useProviderListSort` 用模块级缓存让所有 tab 共享一次 `get_provider_list_state` 往返；hydrate 完成前排序模式回退 `custom` 且页面必须仍以默认顺序渲染，不能因慢读显示空列表。`provider_last_used` 的 key 形如 `<module>:<providerId>`，删除 provider 后残留条目无害（排序时不会渲染），不做清理。
- 最近使用与创建时间必须按解析后的时间点比较；后端写本地 RFC3339 offset，前端即时缓存写 UTC ISO 字符串，不能直接按字符串排序。缺失或无效时间排后，同一时间保持原顺序。
- 改 root directory、favorite provider、session manager 这类共享能力时，要先确认是不是所有消费页面都要同步调整，而不是只修当前页面。
- `RootDirectoryModal` 只对 `source === custom` 的值做输入框回填；不要把 env/shell/default 的当前生效路径直接塞回输入框，否则用户会误以为那是显式保存的自定义路径。
- Claude/Codex/Grok CLI/Gemini CLI 的根目录保存最终会走各自 common config 保存命令。Gateway 接管期间必须像通用配置保存一样锁住根目录保存和恢复默认，否则会绕过 provider 卡片的代理中编辑保护并触发 runtime auto-apply。
- `favoriteProviders.ts` 的 key/payload 规则会影响多个模块的数据迁移和去重；这里不能随意改前缀或 payload 结构。
- 对 OpenCode/Claude/Codex/OpenClaw 这些页，“favorite provider” 的语义更接近“历史库 + 诊断缓存”，不是当前配置快照。改共享 helper 时不要把它偷偷重定义成当前配置镜像。
- `SessionManagerPanel` 依赖 `tool + sourcePath` 契约，不能把 `sourcePath` 当作纯展示字段。
- 改会话详情展示时，要优先维护 `sessionManager/detail/domain/` 的纯函数，再让组件消费这些结果。搜索、过滤、导航和工具卡片预览必须基于同一套 normalized blocks，否则多 CLI 会出现同一消息在不同入口表现不一致的问题。
- 会话详情视觉结构参考 `D:\GitHub\claude-code-history-viewer`，但不要引入左侧 ProjectTree。普通 user/assistant 文本使用轻量 meta 行 + 聊天气泡，不放消息右下角的长文本“展开/收起”；tool/thinking/system/summary/image/unknown 等结构化 block 使用紧凑 Renderer 卡片，卡片默认收起并通过 header 展开。不要把每条消息重新包成带编号 rail、Tag header、整块 border 的日志卡片，也不要把工具卡嵌在普通文本气泡里。
- 会话详情底部状态栏是 workbench 的网格行，`sourcePath` 必须作为可换行的整行内容处理；状态栏和路径节点都要允许收缩，Windows 长路径使用 `overflow-wrap: anywhere`，不能用 `white-space: nowrap` 让 grid 最小内容宽度撑开整个详情页。
- 会话详情顶部过滤 chip 是独立“显示/隐藏”开关，不是单选 Tab。用户/助手、文本/思考过程/工具调用/命令都应分别维护布尔可见状态；点击某个类型只切换该类型，不能影响其他类型。关闭内容类型时还要在 renderer 层隐藏对应 block，而不是只做整条消息级过滤。
- 会话详情的 `roleFilter` / `contentFilter` 是**跨会话、跨工具共享的用户偏好**，不是会话私有态：模块级 `rememberedSessionRoleFilter` / `rememberedSessionContentFilter` 负责进程内跨会话保留，同时每次变化都会经 `settingsApi.saveSessionDetailFilters` 持久化到 SQLite settings 单例记录的 `session_detail_filters` 嵌套 key；首次挂载时异步 hydrate（`getSessionDetailFilters`，无记录或失败时回退默认全开），hydrate 完成前必须禁止持久化 effect 写库，防止慢读被首帧默认值覆盖；切换 `sourcePath` 时只重置搜索词、搜索范围、滚动、匹配与 navigator 折叠，不能把六个 chip 重置回默认全开。持久化走专用 command（`get_session_detail_filters` / `save_session_detail_filters`，store 层用 `db_patch_fields` 只 patch 该嵌套 key），不要改用全量 `save_settings`，否则会与整份 settings 保存互相覆盖；也不要写 localStorage。
- 会话详情右侧 `MessageNavigator` 要按参考项目侧栏处理：标题显示“消息”、带总数、用户过滤按钮、收起/展开按钮、本地“筛选消息...”输入框，条目使用彩色点 + `#turnIndex` + 工具标记 + 时间 + 两行预览。不要把 `(assistant message)`、`unknown` 或纯占位工具消息放进 navigator；二级页外层上下左右只保留很小一致间距，workbench 需要填满页面主体，不能再用内部 `vh` 限高制造底部空白。
- 会话详情右侧 navigator 点击定位依赖消息/工具 target refs。targetId 必须由 normalized `message.id` 通过 `sessionManager/detail/domain/messageTargets.ts` 统一派生；后端各 CLI parser 对缺失 id 的消息必须补稳定 fallback id，前端不要再按某个 CLI 或 render index 分叉生成定位规则。React ref 会在 commit 阶段注册，随后才执行父组件 `useEffect`；不要在 `detail.meta.sourcePath` 这类普通 effect 里 `clear()` refs，否则首次渲染后会把刚注册的 refs 清空，导致 Claude/Codex/Gemini/OpenCode 共享详情页都无法定位。refs 应靠节点 unmount 回调删除，或仅在 workbench 卸载时清理。
- 会话详情里的 Markdown 文本必须复用全局提示词同款 `MarkdownPreview`，不要再单独用裸 `ReactMarkdown` 造一套样式。超过 5 行的 Markdown 默认收起；“展开更多...”按钮样式参考 `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\MessageContentDisplay.tsx`，使用小号 chevron + 浅色文字的轻量文本操作，不要做成有边框的块状按钮。
- 会话详情滚动按钮参考 `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\MessageViewer.tsx`：控件必须浮在消息视图区内部右下角，使用纵向半透明圆形按钮，并根据当前滚动位置隐藏无效方向；不要挂在 workbench 根节点后再按右侧 navigator 宽度计算偏移。
- SubAgent 会话在详情页里是独立导航，不是父时间线的一段消息。父会话只展示后端发现的 SubAgent 摘要列表，点击后加载子会话详情并显示返回父会话 breadcrumb；父时间线必须排除 `isSidechain` 消息，不能靠前端从 Agent tool block 推断 `messageIndex` 后滚动到父消息。
- 会话详情必须走隐藏二级路由页面，不再用大 Modal 承载。列表页只负责 `tool + sourcePath` 跳转；详情页通过 URL query 编码 `sourcePath` / `subagentSourcePath`，并复用 `SessionDetailWorkbench` 展示内容。二级页必须通过 `routeConfig.chrome` 声明 `mode: 'secondary'` 和所属 `ownerTabKey`，再复用 `SecondaryPageShell` 作为页面骨架；`MainLayout` 只能消费路由 chrome 元数据，不能按业务路径后缀判断。进入详情二级页时，`MainLayout` 的主 Header / CLI 顶部 Tabs / 右侧全局动作都必须隐藏，内容区不再预留 Header 高度，只保留详情页自身返回入口；SubAgent 使用返回父会话 breadcrumb，避免同一视图出现多个关闭/返回 `X`。
- Bash/terminal 类工具应优先展示 description、深色 command block、stdout/stderr/result 分区和状态 badge；Read/Write/Edit/Todo/Web/MCP 等工具应走工具 catalog + normalized block 的中间层，避免各 CLI renderer 直接读取 raw message shape。
- `SessionManagerPanel` 如果加批量操作，选择范围必须和当前已加载列表严格一致；搜索词、目录筛选或 reload 改变列表后，要同步清理旧选择，不能保留“用户当前看不见但仍会被删”的隐式选中态。
- `SessionManagerPanel` 运行在 KeepAlive 页面里，工具页切走后组件通常不会卸载，只会隐藏。任何异步操作完成后的 `message.success/error`、loading 收尾或详情回写，都必须先判断当前页面是否仍处于可见上下文；不要让 OpenCode 等隐藏页的旧请求在用户切到 Codex/Claude/OpenClaw 后继续向全局 UI 吐成功或错误提示。
- `SessionManagerPanel` 在 KeepAlive 隐藏页里即使放弃提示或结果回写，也不能漏掉本地 loading 收尾。尤其是列表请求失败后，路径筛选器这类局部 loading 必须按请求代次自行复位，不能完全绑在“当前页面仍可见”这个条件上。
- `SessionManagerPanel` 做整页 reload 时，不要把“刷新列表”和“刷新路径下拉”拆成两次 `forceRefresh` 请求去重扫同一份会话索引。优先复用同一次列表结果里派生出的 path options，避免一次删除/导入/手动刷新触发两轮整库扫描。
- `SessionManagerPanel` 的产品理念是“先让用户看到最近会话，再后台补齐完整事实源”，不是分页列表。首屏 `cache-first` 只是快速快照，不代表第一页；后台 `full` 完成后必须一次性替换成完整列表；`hasMore` 只能作为旧 API 兼容字段，不能驱动 UI。
- `SessionManagerPanel` 禁止出现“加载更多”按钮或滚动翻页 sentinel。首屏加载时，如果还没有任何可展示列表，可以显示内容区全局 loading；首屏已有快照后，后台完整补全只能在列表底部显示轻量 loading 文案，不能遮罩已展示内容；`full` 完成后底部 loading 必须消失，也不能再显示任何“更多”入口。
- 会话列表的完整数据与可见 DOM 必须分开：`SessionList` 复用单列 `management/VirtualGrid`，只挂载可见行和 overscan；完整结果仍留在面板中用于筛选、计数和批量操作。“选择已加载”必须覆盖完整过滤结果，不能缩成屏幕内的行。选择以 `sourcePath` 为身份，使用集合查询；后台替换结果也要清理不再存在的选中项。搜索框状态不能让全部卡片重新渲染，列表/卡片和传入回调需保持稳定。长路径允许换行并由虚拟行实测高度，继续消费外层 `main` 滚动容器和 KeepAlive 的返回位置。
- `SessionManagerPanel` 的用户主动刷新和后台补全必须区分。用户点击标题右侧刷新按钮时才进入可感知的完整刷新，可以显示标题刷新状态和内容区 loading；自动后台 `full`、正文深搜、导入/删除后的静默收敛不能把已显示列表盖住。折叠关闭时要清理刷新 nonce/loading，避免下次普通展开重放旧的手动刷新。
- `SessionManagerPanel` 的首屏加载 effect 必须按 `tool/sourceMode/query/pathFilter/refreshNonce` 这类真实请求条件去重，不能只依赖一个会被父组件状态、i18n 或列表结果重建的 callback。后台 `full` 返回 `availableSources`、路径选项或完整列表后，不能因此重新触发同条件 `cache-first` 并打开全局 loading。
- `SessionManagerPanel` 的后台 `full` 必须等首屏 `cache-first` 请求已经落地后才能启动；初次展开时不能同时出现内容区全局 loading 和底部“正在加载完整会话”。如果已有完整 `all` 列表快照，切换本机/WSL 应从这份快照本地派生并作废旧请求，不能再发起后台完整刷新。
- `SessionManagerPanel` 的搜索先用已加载或缓存 metadata 立即响应，包括 `session_id`、标题、摘要、项目目录、`sourcePath`、runtime source/distro。完整 `session_id` 匹配必须短路。只有后台 `full/refresh` 才继续做正文深搜；正文深搜期间用搜索区域状态提示用户等待，不使用全局 loading，也不把 `cache-first` 放大成全库正文扫描。
- 高密度管理列表可复用 `management/VirtualGrid`，但拖拽排序模式不要和虚拟化混用。排序应继续渲染完整可排序集合，普通浏览/分组展开才使用虚拟网格，避免 dnd 命中区域和虚拟占位高度漂移。
- `VirtualGrid` 的行高测量必须忽略 `0`：KeepAlive 用 `display:none` 隐藏缓存页，ref / ResizeObserver 此时读到的零高度不是有效行高。写入它会逐步压缩虚拟占位高度，切回深处滚动位置时出现空白或错位；隐藏期间应保留最后一次可见测量。
- 虚拟网格的位置不只随滚动和自身尺寸变化。上方配置区展开/收起会改变 `listOffsetTop`，但网格宽高和 `scrollTop` 可以都不变；应沿网格到 `main` 的布局链监听尺寸变化并更新偏移，否则列表进入视口后可能只挂载首行，必须滚动一下才恢复。
- `management/ManagementMenu` 是按需 portal 渲染的轻量菜单。不要为了每张卡片重新引入常驻 overlay 菜单或 tooltip；几百项列表里这会明显放大 DOM 和事件监听成本。
- `management/ManagementMenu` 的 portal 弹层必须按实际菜单尺寸收敛到视口内，不能只靠 `transform` 做左右对齐；卡片工具行为空或接近右侧边缘时，触发按钮可能贴近窗口边界。
- `shared/gateway/GatewayFailoverButton` 主要负责已进入 single/failover 后的故障转移开关；single 的“网关代理”入口和常规“恢复直连”动作属于各 CLI 的已应用 provider 卡片。进入或退出 single/failover 后要刷新系统托盘，因为托盘 provider 菜单也必须随 Gateway 接管状态锁定/解锁。但弹窗内必须保留基于 `status.can_restore_direct` 的兜底恢复入口，避免 provider 被删除、解析失败或列表为空时用户无法解除接管。若当前 P0 provider 的目标协议与 CLI 原生协议不一致，弹窗右下角“恢复直连”必须禁用并展示提示，因为该 provider 离开 Gateway 协议转换后不可直连使用。
- `providerBilling/` 只封装 provider 表单里的供应商级计费 UI 和 meta 读写语义。计费开关关闭时必须从 meta 删除 `costMultiplier` 与 `pricingModelSource`；UI 的“继承全局默认”也不写 `pricingModelSource`。后端现有存储值是 `requested` / `upstream`，不要把 UI 文案里的“请求模型/返回模型”保存成 `request` / `response`。渠道表单中的高级设置、计费配置和备注应使用共享的自绘折叠区样式，不要混用 AntD Collapse/Switch/Select。
- 内置供应商 profile 的 endpoint 是 provider 兼容能力、API 格式、默认 URL 和默认模型/模型目录的事实源；消费页面保存内置 provider 时只写 `meta.gatewayProfile={tool,profileId,endpointId}` 引用和用户覆盖项，不再把 `providerType` / `apiFormat` / `apiKeyField` / `reasoningField` / `defaultMaxTokens` / 图片策略 / `codexChatReasoning` 这类 profile 派生快照固化到 provider meta。Base URL 允许用户在表单中覆盖，保存时必须使用用户当前输入值而不是无条件写回 endpoint 默认 URL。
- 已保存内置 provider 重新打开表单时，优先用 `meta.gatewayProfile.profileId + endpointId` 回显 endpoint；Base URL 是可编辑连接地址，不参与 endpoint 身份判断。没有 `gatewayProfile` 的 legacy provider 只能在 `providerType + apiFormat` 唯一命中时自动回显内置 endpoint，多匹配时必须回到自定义渠道，等待用户从渠道下拉显式选择。`providerType + apiFormat` 是 runtime effective compat 输出，不是内置渠道身份。
- Codex 消费内置 profile 时，显式 `modelCatalog` 优先；Anthropic/Claude 协议 endpoint 如果没有目录，前端可从同一 profile 的 Claude endpoint `models` 派生添加供应商表单的初始映射，避免协议切换时模型映射丢失。不要把这个派生规则反过来写成共享 profile JSON 的新必填字段。
- Magic Context 配置卡片只在调用方确认插件/扩展已安装时展示；OpenCode 由页面检查 `config.plugin`，Pi 由扩展列表检查 `@cortexkit/pi-magic-context`。不要在 shared 组件里重复实现安装扫描。
- `GlobalPromptSettings` 的 `__local__` 只是本地 prompt 文件的临时桥接项（DB 为空时读本地 `CLAUDE.md` / `AGENTS.md` 等映射出来）。后端可能把它标成 `isApplied=true`，表示“当前文件内容就是这份镜像”，但 UI 不能把它当正式已应用预设：不要显示「已应用」标签、不要高亮选中态、折叠标题也不要显示「当前: default」，也不要露出「应用」按钮。用户应通过编辑后 `saveLocalConfig` 收编入库，才进入真正的 applied 管理语义。
- `GlobalPromptSettings` 卡片三点菜单的「禁用」只对"已应用且非 `__local__`"的配置显示（`showAsApplied` 即 `isApplied && !isLocalConfig`）。点击后 `Modal.confirm` 二次确认（文案 key 为各模块 `{prefix}.confirmDisable`，菜单标签用 `common.disable`），确认后调 `service.disableConfig`：后端语义是取消应用 + 清空 runtime 提示词文件 + 保留 DB 记录（可重新应用）。新增带全局提示词的工具 tab 时，service 的 disable 命令名必须与后端 `lib.rs` 注册一致，`web/test/services/globalPromptCommands.test.ts` 会校验这条映射，不要漏配。

## 跨模块依赖

- 被 `claudecode/`、`codex/`、`grok/`、`geminicli/`、`opencode/`、`openclaw/` 多个页面共同依赖。
- 依赖各 owning module 的 service/api，而不是直接操作数据库。
- 与后端 `session_manager/`、各工具 commands、favorite provider 后端服务形成跨模块契约。

## 典型变更场景（按需）

- 改共享 root directory 逻辑时：
  同时检查 Claude/Codex/Grok CLI/Gemini CLI 四页 modal 回填、source label 和 reset/save 语义。
- 改 favorite provider 规则时：
  同时检查 storage key、source payload、去重、迁移和多页面导入逻辑。
- 改 session manager 共享面板时：
  同时检查 list/detail/import/export/rename/delete API 契约。

## 最小验证

- 至少验证：一个共享改动在两个以上消费页面中仍表现一致。
- 至少验证：favorite provider 和 session manager 的 key/sourcePath 契约未被破坏。
- 会话虚拟列表使用 `node scripts/benchmark-session-list.mjs --verify-only` 验证真实面板的全选、过滤、深处返回、KeepAlive、变高行和主题；需要已安装的 Chrome/Edge，可通过 `--browser` 指定路径。性能对照用 `--baseline <commit> --repetitions 3`，只比较同一元数据夹具的前端渲染，不把它当作真实磁盘扫描或 Tauri WebView FPS 测量。
- 供应商批量操作回归覆盖整批备份失败不删除、部分删除后保留剩余选择、搜索/默认项变更后的选择清理；见 `web/test/features/coding/shared/providerList/providerBatchOperations.test.ts`。

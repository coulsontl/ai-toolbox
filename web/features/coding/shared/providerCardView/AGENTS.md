# 供应商卡片视图（providerCardView）

## 一句话职责

- 提供**供应商列表的第二种呈现方式**：把同一批供应商渲染成紧凑卡片网格，由用户的布局偏好切换。已接 Claude Code（2026-10-11）与 Codex（2026-10-12），其余 CLI 复用同一套组件。

## Source of Truth

- 组件本身（`ProviderCompactCard` / `ProviderCardGrid` / `ProviderDetailPanel`）是这一呈现方式的唯一实现。
- **卡片的 props 复用 `providerCardVariants/types.ts` 的 `ProviderCardState` / `ProviderCardActions`**：同一份对象既喂给列表卡片也喂给紧凑卡片。这是「两种布局下按钮行为一致」的**结构性保证**——不是靠人工对齐两套代码。
- 布局偏好的存储在后端 `provider_view_modes` 嵌套键；前端经 `shared/providerList/useProviderListViewPrefs` 读写。

## 核心设计决策（Why）

- **这不是 `providerCardVariants` 的第四种样式。** 那三种样式按 **provider 模型形状**选型（有没有模型目录、有没有单一 active provider），是语义差异；本组件按**用户选的显示密度**切换，同一个 CLI 的同一份数据两种画法。把它混进变体里，会让「该用哪种样式」变成由显示偏好回答——正是那个模块要防止的混淆（见 `providerCardVariants/AGENTS.md` 与根 `AGENTS.md` Hard Rule 14）。
- **文件名刻意避开 `*ProviderCard.tsx`。** `scripts/verify-provider-card-layout.mjs` 的后缀作用域管的是「各 CLI 的映射层不许自建 chrome」。本组件**正是** chrome 的宿主（与 `CardShell` 同级），所以叫 `ProviderCompactCard.tsx`。改名成 `CompactProviderCard.tsx` 会被守卫当成映射层扫，然后报出三条它本该做的事。
- **它自己注册 `useSortable` / 渲染 `ManagementCheckbox` / 挂 `data-provider-id`。** 这三件正是 `CardShell` 为列表样式做的事；两个外壳共享同一套契约，`data-provider-id` 仍是「定位」在 DOM 里找卡片的唯一锚点。
- **网格不虚拟化**（与 Skills / MCP 的关键差异）。供应商是几十条，列表布局本身也是全量渲染；一旦虚拟化，屏幕外的卡片不在 DOM 里，「定位」的 `[data-provider-id]` 查询会直接查不到——用户会看到「当前列表里找不到已应用的供应商」，而它其实就在下面一点。Skills/MCP 虚拟化是因为它们要管几百项。
- **详情侧栏的删除必须自己确认，并遵守删除禁用原因。** 侧栏 footer 只渲染调用方给的节点，不是 `Popconfirm` 的宿主：列表卡的 `actions.deleteConfirm` 管不到这里。映射层要在 `deleteDisabledReason` 存在时置灰按钮（外层 `<span>` 承载 Tooltip），否则 `Modal.confirm` 之后再调 handler。Codex 与 ZCode 的 handler 自带 `Modal.confirm`；OpenCode 的 handler 不弹确认（列表卡由 `Popconfirm` 负责），所以它必须在侧栏里自己弹。

- **详情侧栏是卡片视图的一部分，不是可选装饰。** 一个格子只放得下标题、模型行、两个主按钮；供应商的其余事实（地址、全部模型绑定、模型目录、官方账号、备注）和其余动作（测试/编辑/复制/分享/删除）总得有个去处，否则卡片视图就是「信息更少的列表」。侧栏的**外壳**是共享的 `ProviderDetailPanel`，**内容**由各 CLI 的映射层算出来喂进去（`sections: { key, title, fields, content }[]`）：字段名与动作是 CLI 特有的（Claude 的角色绑定、Codex 的 TOML 绑定与 `modelCatalog`），摆位（定宽标签列、等宽值、复制按钮、折叠区）才是共享的——把后者复制两份，就是「两个 CLI 的侧栏慢慢长歪」的起点。卡片那一行动作在侧栏里与状态标签同处标题行（主行动要在首屏），次要动作留在 footer。
- **侧栏里的 `content` 区不写 `title` 时才不带标题**（catalog、官方账号块都是自带标题的组件）；`fields` 行用 `emptyText` 存在与否控制「这行在缺值时是否还在」——没有 `emptyText` 的行是**条件行**（官方订阅没有协议，那一行直接消失，而不是显示「未配置」）。

## 关键约束（改这里之前必读）

- **accent 高亮必须走 `providerCardVariants/cardChrome.ts` 的 `resolveProviderAccentChrome`，不要内联复制一份。** 这是仓库记录在案的高危漂移点：四个 bespoke 卡片迁到共享外壳时没把 accent 一起搬，卡片**静默**变成统一灰边框——没有报错、类型也通过。两个外壳（`CardShell` 与 `ProviderCompactCard`）现在共用同一个函数，改一处两边生效。
- **accent 必须走 inline style，不能靠 CSS 类的 hover 规则。** 高亮是「这张卡片处于某个状态」，不是「指针在它上面」；写进 class 会让 hover 重绘它。
- **hover 浮出的「更多」按钮只能改 `opacity`，不能改 `display`/`width`。** 按钮在未 hover 时也占着盒子，否则指针划过卡片时标题会重排（DESIGN.md 明确禁止布局跳动）。
- **hover 的形状是「格子」的形状，不是「列表卡片」的形状。** 格子跟随它的来源 `SkillCard`：上浮 1px、底色抬到 `--color-bg-elevated`、标题变主色、阴影加深（2026-10-13 补上，此前只有阴影）。**上浮挂在 `[data-provider-card-clickable]` 上**：批量选择模式下主体点击是被抑制的，会浮起的卡片等于承诺一个它不会兑现的点击。列表卡片（`CardShell`）**只加深阴影、不上浮**——它是整行卡片，而且列表布局不在重设计范围内（用户 2026-10-13：「列表的样式不要变啊」）。两者共用的只有阴影这一半（`cardChrome.ts` 的 `PROVIDER_CARD_SHADOW*`）。带 accent 的卡片 hover 时保留自己的底色：accent 是 inline style，天然盖过类规则——底色说的是「这个 provider 是什么」，不是「指针在哪」。
- **第二行的高度必须预留，且预留值要连 `.body` 自己的下内边距一起算。** 网格把一行里每张卡片都拉到最高那张的高度，差额由 `.body`（`flex: 1`）吸收——**没有模型行的卡片**（Codex 的官方/`__local__` 行）因此把 17px 余量堆在 footer 分隔线正上方，用户 2026-10-12 报「副标题离下面的分隔线太远」。预留值**按动作挂在哪一行取两个数**：动作在这条线上（`actionPlacement="subtitle"`，`.bodyWithAction`）→ 24px 的按钮 + 6px 下内边距 = **30px**；动作在 footer（`.body`）→ 只有 17px 的模型行要放，= **23px**（盒子是 `border-box`，内边距要算进这个数里——写 22px 就差 1px）。判据：**卡片高度是「有没有这一行」的函数，不是「这一行多长」的函数，也不是「邻居有没有动作」的函数**——30px 让「只说动作」的格子（ZCode 的 provider 没有 provider 级动作）和「说模型 / 说登录」的格子一样高，全应用的格子因此都是 68px；而给 footer 形态留 30px 会在第二行和分隔线之间多出 7px 空白。回归：`test:codex-card-view` 断言「有/无模型行的卡片等高」且「两张卡片的第二行到分隔线距离相同」。
- **第二行里的动作与模型行必须同心。** 行是 `align-items: center` 的：按钮 24px 对模型行 17px，行若顶对齐，按钮中心会比模型行中心低 **3.5px**（2026-10-13 在 Codex 与 Kimi 的网格上量出来的），而这是每个格子都有的偏差。**不要用 `margin-top: -4px` 这类经验值去补**——同心由布局给出，改字号/按钮尺寸时不会失效。
- **`content` 区的两组组件都是共享的**：模型目录走 `providerCardVariants/ProviderModelSection.tsx`（Codex / OpenCode 的列表卡片与侧栏共用同一份 `modelSection.*` 映射），官方账号块走 `shared/officialAccounts/OfficialAccountsSection`。**不要在侧栏里另起一个「侧栏版目录」**——目录是**能用的管理器**（批量删除 / 模型测试 / 获取模型 / 添加模型），搬进去的必须是同一个组件，否则卡片视图下的模型管理会是残缺的一套。**侧栏里这两个区块都是常开的**（2026-10-12 用户指定）：它们存在的意义就是展示那些行，一个折叠标题只是挡在中间。**但这只是侧栏这一侧的形态，不是共享组件的形态**——列表布局的卡片照旧折叠（用户 2026-10-13：「列表的样式不要变啊」）。两个组件都按「宿主给不给折叠 props」判断：模型目录传 `appearance="detail"`，账号块**不传** `collapsed`/`onToggleCollapsed`（两个都传才画 caret），于是侧栏那侧没有 caret、行也不缩进（缩进的作用正是让开 caret）；`(n)` 计数不因折叠消失，两种形态都在标题行上。
- **详情抽屉的宽度与 Skills / MCP 的详情抽屉一致：`size="min(60vw, 760px)"`**（antd 6 用 `size` 而不是废弃的 `width`/`height`，它接受数字或 CSS 长度字符串）。Codex 侧栏原本是固定 560px，因为它的模型目录工具栏有四个带文字的按钮——760px 上限同样装得下，而「整个应用的供应商侧栏是同一个表面」比给某一个 CLI 多留 200px 更值。改宽度只改这一处，两个 CLI 的 `Drawer` 都读它。
- **详情面板的视觉语言对齐两个兄弟面板（Skill / MCP），不另立一套。** 三块抽屉同宽、同一种内部结构：内容区 `padding: 20px 24px 12px`、标题 18px、**分区标题是 11px/650/`letter-spacing: 0.05em` 的次要色眉标**（不是 13px 的粗标题——`SkillDetailPanel` 与 `McpDetailPanel` 都是眉标，内容才是主角），**字段成组放在一个 `--color-bg-elevated` 的描边卡里**（`SkillDetailPanel` 的 `.metaCard` / `McpDetailPanel` 的 `.configCard` 同款），标签列 72px、11px 三级色。2026-10-13 之前本面板是「白底上一行行裸字段」，在一个 760px 的抽屉里没有任何结构——这是用户在「详情页样式」上最直观的那一眼。**字段卡只在真有一行要渲染时出现**（`renderableFields` 与 `FieldRow` 用同一条判据，否则一个只有条件字段的区会留下一个空盒子）。
- **网格自己给间距，卡片自己给的那份要让出来。** ZCode 的官方账号卡片（`data-provider-id` 那一层）带 `margin-bottom: 12`：列表布局里卡片是彼此没有 gap 的兄弟，间距各自负责；进了网格这个 12px 会叠在 10px 的 gap 上，那一行的间距变成 22px（2026-10-13 量到）。`ProviderCardGrid.module.less` 因此对网格内的 `[data-provider-id]` 清零下外边距——**用 `!important`**，因为那份 margin 是 inline style，样式表规则压不过它。
- **列表里「不是 provider」的成员，在网格里也是一个格子，不是一行、也不是一块裸内容。** ZCode 的官方账号卡片经历过两版错的做法：先**保留列表那张 `CardShell`**（全宽的描边卡浮在网格里，读起来像第四种卡片样式），再矫枉过正**不套外壳**（一块裸内容摆在格子之间，既不是格子也不是卡片）。定稿是它穿**同一个** `ProviderCompactCard`，**和邻居同宽同高**，**用共享卡片的 `detail` 开它自己的侧栏**。它因此仍是**一个普通网格项**：没有跨整行、没有 `rowItem`、没有「网格之外的特殊路径」，排序/定位/搜索/空态全都按格子走。标题行用**同一个** `OfficialAccountHeadingIcon` + `OfficialAccountCount`（与 `standalone` 区块自己的标题行同源），所以 🔗 与 `(n)` 不会在两种布局里长歪。
- **格子的内容如果是「管理器」，管理器进这张格子自己的侧栏——不是塞进格子。** 官方账号块是一行一个账号、每行各有动作的管理器（Codex / Kimi / ZCode 三家同一个块），一个格子放不下也不该放下；三家在卡片模式下因此是同一个处置：格子只陈述**一件事**（官方渠道说「当前登录: <账号>」，用共享的 `common.officialAccount.currentLogin`，值取它自己那行的 label），管理器由 `detail` 打开。**「一个格子陈述一件事」是这条的判据**：格子那行要么是模型、要么是登录，两处都写就是两个答案。
- **底部两个主按钮走 `providerCardVariants/providerCardActions.tsx` 的 `renderProviderLaunchAction` / `renderProviderPrimaryAction`，不要在本模块内联一份。** 详情侧栏渲染的是**同一对**函数——「侧栏提供的能力和卡片一致」因此是代码事实，不是每次改按钮都要重新核对的对齐工作。两个样式卡片（`ClaudeStyleCard` / `CodexStyleCard`）的主按钮也已改为调用它。
- **标题后面不再显示地址，但标题本身仍是链接。** 地址（`baseUrl`）搬到了详情侧栏的「连接信息」区；标题则把 `baseUrl` 交给 `ProviderNameLink`，于是**悬停显示该 provider 的来源域名、点击用系统浏览器打开**——与列表卡片逐字同款。标题因此不再有 `max-width: 65%` 的上限（那是为了给旁边的长 URL 让位，URL 一走就只是浪费空间）。
- **第二行由映射层定，各 CLI 说的是各自最重要的那个模型：Claude 给「全部角色模型」，Codex 只给主模型。** `subtitle` 收 `{ label?, value }[]`，每个条目渲染成 `Role: value`（label 次要色、value 等宽）；**`label` 可省，省掉时只渲染等宽的 value**——用于自己就能说明自己的事实（OpenCode 的 `@ai-sdk/openai-compatible` 是 npm 包名而不是某个角色的取值，`NPM 包:` 前缀只挤占这一行）；`title` 属性同样按有无 label 拼装。**相邻条目之间由 `::before` 画一根 `|` 分隔线**（是伪元素不是节点：它是两件事之间的标点，不是第三件事；做成 `<span>` 就得让每个重建这一行的地方——`title`、浏览器夹具——都把它跳过去）。**必须用 block + `white-space: nowrap` + `text-overflow: ellipsis`，不能用 flex 行**：flex 子项只在整项处裁断、不出现省略号，inline 内容才会在截断点放一个。卡片高度在网格里必须一致，所以这一行永不换行，完整内容走 `title` 属性。条目之间与 label/value 之间的间距都是 CSS margin（不是文本节点）——任何按 `textContent` 断言的测试必须重建这一行，否则会得到 `Opus:claude-opus-4-8Sonnet:...`。
- **字号跟随 Skills 卡片的三段式预算：标题 14px/600、第二行 11px、footer 与状态标签 11px/10px。** 这是「同一张卡片语言」的可观察部分，`SkillCard.module.less` 是它的 source of truth（标题 14px/600，footer meta 栏整条 11px）。**字号由上层面（`.footerRow` / `.nameTags`）设定，不写在 `providerCardActions.tsx` 的共享渲染函数里**：列表卡片与详情侧栏渲染的是同一对按钮，各自要各自的密度，内联字号会盖住两边。
- **状态标签走 `nameTags`，与列表卡片同一个槽位。** 映射层把「哪些事实这张卡片要陈述」算成一棵树，两种布局各取所需：列表卡片那行含地址，卡片视图用**去掉地址的同一棵子树**（Claude 的映射层里是 `stateTags` + `localConfigHint`）。**不要用「卡片视图自己判断 `isApplied` 然后画个标签」来替代**——那正是「同一状态在一种布局里可见、在另一种里消失」的成因（2026-10-12 用户报「卡片上的已应用标识没了」：格子当时只有 accent 边框，边框说了「它特殊」却没说「为什么特殊」）。header 行里 `.nameTags` 用 `flex-shrink: 0`，让标题先截断而不是把标签压成一条缝。
- **`namePrefix` 是名称行前面的槽，映射层用它给「被改名的卡片」补上那个前缀图标。** Codex / Kimi 的官方渠道卡片在映射层里被命名成「官方账号」（`common.officialAccount.headingTitle`），前面那个 🔗 就走这个槽——和 `nameTags`（承载 `(n)` 计数）合起来，卡片自己的名称行就是它账号区块的标题，两个宿主画不歪（见 `shared/officialAccounts/AGENTS.md`）。
- **卡片主体点击开侧栏，但 `handleCardClick` 必须过滤掉自身控件、并在多选模式下整体让位。** `button, a, input, [role="menuitem"], [role="checkbox"]` 的点击不冒泡成「开侧栏」，否则点「应用」或拖拽把手也会盖上一层抽屉；多选模式下主体点击的含义是「选中这张卡片」，再弹侧栏就没法连续多选了（`SkillCard` 同款处理）。卡片主体是 `<div>` 而不是 `<button>`：卡片内部有自己的交互元素，嵌套进 button 是不合法标记。可点击时根节点带 `data-provider-card-clickable`，`cursor: pointer` 挂在它上面——无条件加 `cursor: pointer` 会承诺一个并不存在的点击目标。
- **闪光环半径读 `--locate-flash-radius`。** 列表卡片是 antd Card 的 8px，这里是 10px（Skills 口径）；`ProviderListSection.module.less` 的 `.locateFlash` 用 `var(--locate-flash-radius, 8px)`，本组件从自己的根元素发布 10px。不发布就会在圆角卡片外画一个方角环。
- **「更多」菜单走 `providerCardVariants/providerCardMenu.tsx` 的 `buildProviderCardMenuItems`**，不要在本模块重写一份：菜单顺序（启用 → 编辑 → 复制 → 分享 → 额外项 → 分隔线 → 删除）是契约。额外项（Claude 的「模型测试」）走 `extraMenuItems`，插在「分享」之后、分隔线之前——它是「对这个 provider 做点什么」，不是破坏性动作。
- **用 antd `Dropdown`，不要换成 `management/ManagementMenu`。** 启用项里要放真实 `<Switch>`，而 `ManagementMenu` 的项是 `<button>`，嵌套交互元素不合法。
- **`launchAction` 是单个动作，`actions.gatewayActions` / `actions.primaryAction` 原样透传。** 底部按钮的**出现条件**由映射层算（`ClaudeProviderCard` 里那套 `canShowGatewayProxyButton` / `showDirectApplyAction` / …），本组件只负责摆位——与列表卡片同一套判据，这是「按钮逻辑一致」的落点。
- **`actionPlacement` 决定那组动作挂在哪一行，两种形态都由映射层选**：`'footer'`（默认）把「网关/应用」放在底部隔断线下的 `footerRow`，适合动作多或第二行内容多的 CLI（Claude 给全部角色模型）；`'subtitle'` 把它挂在第二行末尾（`margin-left: auto`），适合「一个模型 + 一个动作」的形态——Codex 只有一个主模型行，再单开一条 footer 会得到一条只有一个按钮的空栏（2026-10-12 用户指定）。**两种形态渲染的是同一份动作节点**（组件里只构建一次再择一处放置），所以不存在「两种形态行为不一致」的可能；差别只有位置。
- **`'subtitle'` 形态下动作无条件渲染，只有第二行本身是条件渲染的**：没配模型时卡片没有第二行，但动作仍占着那一行（`subtitleRow` 是 flex 行，动作 `margin-left: auto`）。动作是卡片的主要用途，不能因为「宿主行没内容」就消失。
- **`cellControl` 是「这张格子自己的控件」的插槽，落在动作槽里、主操作之前**——即别的 CLI 放 应用 / 网关代理 的那个位置。CLI 的 provider 永远不会被应用时（ZCode：`设为默认` 在模型行上），那一行原本什么都不说，而每个邻居格子都在说自己的控件；它的唯一 per-provider 控件是启用开关，列表卡把同一颗开关画在头部，所以传进 `cellControl` 就是把它放到**同一个位置**（2026-10-15 用户指定）。**插槽而不是布尔开关**：卡片不决定一个 provider 的状态控件是什么，它的 CLI 决定（同 `loginAction` 的理由）。
  > 传了 `cellControl`，**「更多」菜单里那一条启用项就让出来**（组件里 `cellControl ? undefined : onToggleDisabled`）：同一个控件在同一张卡片上说两遍，正是本仓反复拦的那类事（同 默认 徽章 + 灰掉的切换按钮）。菜单之所以收着它是「旁边有 应用 就争抢」，格子那行没有主操作，这个理由就不成立。

## 跨模块依赖

- `providerCardVariants/types.ts`：`ProviderCardModel` / `ProviderCardState` / `ProviderCardActions`（复用，不重定义）。
- `providerCardVariants/cardChrome.ts`：accent 与阴影常量。
- `providerCardVariants/providerCardMenu.tsx`：菜单构建与分发。
- `providerCardVariants/providerCardActions.tsx`：底部两个主按钮的渲染函数（本组件与详情侧栏共用）。
- `providerCardVariants/ProviderModelSection.tsx`：`modelSection.*` → `ModelListSection` 的唯一映射（列表卡片与侧栏都用它）。
- `shared/management`：`ManagementCheckbox`、`useAutoGridColumns`。
- `shared/providerList`：`useProviderListViewPrefs`（布局偏好）、`ProviderViewMode` 类型。
- 使用方：`claudecode`（2026-10-11 接入）、`codex`（2026-10-12 接入）、`zcode` / `kimi`（2026-10-13 接入）、`opencode`（2026-10-19 接入）、`claudedesktop` / `geminicli` / `grok` / `omo_native`（2026-10-20 接入）、`openclaw` / `hermes`（2026-10-21 接入，五个配置型 CLI 里剩下的两个）。接入方式：映射层（`*ProviderCard.tsx`）加 `viewMode` 一个 prop，`'card'` 分支渲染 `ProviderCompactCard` 并把侧栏**内容**交给它的 `detail`（抽屉本身是卡片的，见上）；页面把列表容器换成 `ProviderCardGrid`，`SortableContext` 的 strategy 与 `DndContext` 的 modifiers 随视图切换，卡片 props 由页面（或区块组件）上一份 `renderProviderCard(provider)` 辅助函数统一产出（两种布局渲染同一份，不许各算一份）。布局偏好一律 `useProviderListViewPrefs('<moduleKey>')`。回归：`test:provider-card-view` / `test:codex-card-view` / `test:zcode-card-view` / `test:kimi-card-view` / `test:migrated-provider-cards`（后四家共一份夹具）。
- **2026-10-21 这两家的格子第二行**：`openclaw` 说 AI SDK 包名（`@ai-sdk/…`，每个渠道都有，所以永不为空），`hermes` 说包名 + 该渠道拥有的默认模型（`当前主模型`，非默认渠道只出包名）。两家的模型目录都整块进详情侧栏且不折叠。`hermes` 没有 provider 拖拽（后端无 reorder 命令），`openclaw` 有。
- **2026-10-20 这四家的格子第二行**：`claudedesktop` 说角色路由（`Sonnet: … | Opus: …`，与 Claude Code 同形）、`geminicli` 普通渠道说主模型、官方渠道说「当前登录」（三态，同 Codex）、`grok` 普通渠道说主模型、官方渠道说「当前登录」、`omo_native` 说裸 npm 包名（默认渠道追加「当前主模型」，同 OpenCode）。四家的官方账号块都进各自的详情侧栏且不折叠；`omo_native` 的「更多」菜单遵守 `deleteDisabledReason`（默认渠道的删除置灰）。
- **列表里的「非 provider 成员」在网格里也要有位置，且要有形状**：ZCode 的官方账号卡片是一个可拖动排序的列表成员，但不是 provider。判据：**「它在列表里是谁？」**——是成员就留在网格里，形状由宿主定（一个格子，与邻居同宽同高；管理器进它自己的侧栏）。**两种布局的成员顺序来自同一个表达式**（ZCode 的 `listMembers`），否则「账号卡片放在哪」会有两个答案。不要为它发明第三种形状（跨整行 `gridColumn: '1 / -1'`、独立外壳），也不要让它不穿形状（裸内容）——两版都被用户退回（SOP #150）。
- **Codex 的接入与 Claude 的差异**：它的第二行只陈述**一个**事实——普通 provider 是主模型（`主模型: gpt-5 (high)`，没有模型名就不出这一行），**官方渠道是「当前登录: <账号>」**（Codex / Kimi / ZCode 三家一致，用共享的 `common.officialAccount.currentLogin`，值取该账号在自己那行的 label；**取值顺序＝应用的账号 → 否则磁盘上那份登录（`isVirtual`）→ 否则 `currentLoginNone`（未登录）**，见下条）。模型目录（管理器）整块搬进侧栏，官方账号块也跟着进侧栏——因为官方卡片在卡片视图里没有 footer 可挂，而账号块是管理器。它的元信息行在列表卡片上含地址/目录，卡片视图一格放不下，所以**格子只留标题行、第二行与两个主按钮**。
- **「只说一件事」的那一行，在每个状态下都要说话——空白第二行读起来是坏卡片，不是「无话可说」。** 第二行的高度是**无条件预留**的（见第 28 条：`.bodyWithAction` 30px / `.body` 23px），所以宿主那一行由「有值才渲染」变成「没值就不画」时，用户看到的是一条空带子（2026-10-14 用户报「为什么 codex 和 kimi 的卡片下面没有显示当前的账号啊」——那两条渠道只是闲置，官方账号一个都没被应用，#153 / 模式一百零二）。判据：**把宿主的一行枚举成状态，每条状态都要有输出**（有值 → 磁盘上那份 → 明说「未登录」/「未配置」），不要用 `undefined` 当「没有更好的答案」；只断言「有值那一态」的测试等于没断言——夹具要为**没值**的那态单开一个模式（`accounts=idle`）。
- **布局切换控件在 `shared/ProviderListSection`**，是**一个「图标 + 文字」按钮**（不是分段控件），位置在**一键测试之前**（与它左边的定位、搜索、排序同属「列表本身怎么摆」那一组）：`data-provider-view-toggle` 带当前模式，**图标与文字都表示「点了会切到哪种布局」**（文字是目标布局名：卡片 / 列表），tooltip 与 `aria-label` 用整句说明同一件事。两处细节是踩过的坑：图标用 antd 的 `LayoutOutlined` / `UnorderedListOutlined`，**不能用 lucide 的 `LayoutGrid`——那是 2×2 方块，和「通用配置」的 `AppstoreOutlined` 一模一样**（用户 2026-10-12 报「布局切换按钮和通用配置的 icon 一样」）；`data-provider-view-toggle` 即使有了文字也保留，因为文字是会被改写的文案，夹具不该匹配它。

## 最小验证

- `pnpm run test:provider-card-view`（53 项真浏览器断言）：切换生效并落库、**切换按钮的图标不与「通用配置」重复且文字跟随目标布局**、副标题三种形态与 `Role:` 前缀、**相邻条目之间有 `|` 分隔而单条时没有**、卡片标题不再带地址但**悬停给出域名、点击调 `open_url`**、**三段式字号（14/11/11/10）**、**已应用卡片陈述「已应用」标签而非只有边框**、已应用卡片是网关按钮而非「应用」、hover 浮出「更多」、菜单项与列表卡片同构（多一个「模型测试」）、**点卡片主体开侧栏而点控件不开**、**多选模式下主体点击被抑制**（进入/离开两侧都验）、侧栏「连接信息」陈述协议、**长 API Key 单行中间省略而复制按钮给出完整值**、侧栏 footer 动作与卡片一致且字号同为 11px、`data-provider-id` 仍在、**网格内真拖拽后顺序变化且回写后端**、切回列表恢复、暗色主题。
- `pnpm run test:codex-card-view`（81 项真浏览器断言，挂**真实 `CodexPage`**）：切换生效并落库、格子只有主模型且无地址/目录、`主模型: gpt-5 (high)`、没配模型就不出第二行、**官方渠道的第二行是「当前登录: <账号>」而不是模型**（三态：应用中的账号 / 磁盘上那份登录与一份没在用的保存账号并存（用户报缺陷时的真实形状，须报「在用」的那份，不报闲置的）/ 「未登录」）、三段式字号、官方卡片是 🔗 官方账号 (2) 且图标在标题之前、footer 是网关/应用、hover 浮出「更多」、标题悬停给域名并可点开、**侧栏含连接信息/模型配置/模型列表目录/官方账号，且目录带全部管理动作**、自定义供应商**没有**账号块、长 key 中间省略而复制给出完整值、`data-provider-id` 仍在、网格内真拖拽后回写、切回列表后目录仍在、暗色主题。
- `pnpm run test:kimi-card-view`（37 项）/ `pnpm run test:zcode-card-view`（49 项）：两个宿主各自钉住「网格里这个成员是一个普通格子」（Kimi 那份与 Codex 一样钉官方格子的三态「当前登录」），ZCode 那份还钉「格子的动作槽陈述启用开关而菜单不再重复它」、「管理器的账号行不在格子里、在它自己的侧栏里且常开无 caret」与「拖它不会连带打开侧栏」。
- `pnpm run test:provider-card-layout`：新组件不得让映射层守卫报错，`PENDING_MIGRATION` 为空（2026-10-20 清空）；`ProviderModelSection` 已列入该脚本的「render point」文件表，否则 `modelSection.*` 会被误报成死 prop。
- `pnpm run test:migrated-provider-cards`：六家（claudedesktop / geminicli / grok / omo_native / openclaw / hermes）的映射层列表 + 卡片两种布局，53 项真浏览器断言（OpenClaw 的格子只陈述 npm 包、Hermes 的格子陈述包名 + 默认模型且默认渠道可删、只读渠道无动作，角色绑定、tierAlias tag、备注 footer 位置、dragDisabled 无把手、Claude Desktop 官方行标题 `🔗 官方账号` 且菜单无删除、官方行第二行说明 `应用会切回 1P` 且无「模型测试」、官方格子说 `官方登录: 由 Claude Desktop 管理`、模型测试不换行的几何断言、映射渠道只有「应用并代理」、官方卡片标题与计数、账号行跨满整宽、virtual 行只给保存、Grok 刷新 Token 插槽、未登录官方渠道的「应用」锁定、模型工具栏顺序、OmO npm 包与默认渠道、菜单删除保护、暗色主题）。
- `pnpm run test:provider-list-locate` / `test:provider-list-drag`：列表布局未受影响。

## 何时更新本文件

- 接入下一个 CLI 时：补充「该 CLI 的第二行说什么、它的模型目录/账号块放在哪」。
- 网格开始需要虚拟化时（供应商数量真的上千）：先解决「定位」如何在虚拟列表里工作，再动这里。

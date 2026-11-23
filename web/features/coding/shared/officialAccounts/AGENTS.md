# 官方账号区块（officialAccounts）

## 一句话职责

- 把「官方登录的账号区块长什么样」固定成**一个**共享区块，形状是**两行标题 + 一串行**：
  - 标题行（heading）：`🔗 官方账号 (n)`
  - 说明行（hint）：这个 CLI 切换账号到底做了什么
  - 列表行（list）：`▾ 账号列表 (n)`，右端是登录入口
  - 账号行：切换 / 保存 / 刷新 / 详情 / 删除
- 各 CLI 只做两件事：把自家账号记录映射成 `OfficialAccountRowView`，再挑一个宿主。
- **给新 CLI 接入时按 `docs/new-cli-onboarding-sop.md` §4.2.9 走**（判宿主 → 写映射 → 三条后端不变量 → 最小验证）；本文件是「为什么这样设计」，那份是「照着做」。

## Source of Truth

- 区块本体是 `OfficialAccountsSection.tsx`；`types.ts` 是行视图与 props 的定义。
- **宿主由调用方决定**，组件不关心外层是什么：
  - `embedded` —— 区块挂在供应商卡片**内部**（Codex、Kimi）。卡片自己的头部行**就是**标题行，因此区块只画「列表行 + 账号行」，另有上方分隔线与行缩进；空态用一行文字（卡片中间插一张空插画会把下面的模型区推出首屏）。
  - `standalone` —— 区块**自持一张卡片**（ZCode：官方登录不在供应商表里，没有官方行可挂）。此时区块把标题行、说明行、列表行、账号行都画出来；卡片外壳用共享的 `CardShell`（拖拽把手在内容列**之外**，见「关键约束」）。
- 这个 `variant` 是**两个原实现真实存在的差异**，不是可调参数。新增第三种宿主形态之前不要加值。

## 核心设计决策（Why）

- **🔗 属于「标题行」，而「标题行」在不同宿主上是不同的那一行。** `standalone` 的标题行由区块自己画（`headingTitle` + `OfficialAccountHeadingIcon` + `OfficialAccountCount`）；`embedded` 的标题行是**卡片自己的名称行**，由卡片画：名称前用 style card 的 `namePrefix` 插 🔗，名称后用 `OfficialAccountCount` 插 `(n)`。两处共用同两个导出，所以画不歪。
  > 返工记录：曾把 🔗 当成「区块标题行上的装饰」无条件画，于是 Codex/Kimi 的**卡片头部**没有 🔗、而列表标题多了一个。用户要的是卡片头部那两行换成 `🔗 官方账号 (2)` + 说明。**区块是卡片的一部分时，「区块的标题行」≠「卡片那行」。**
- **登录入口挂在「列表行」右端**（`loginAction`），不在卡片头部。它加的是列表里的一行，就该和它改动的那些行在一起。曾放到卡片头部右侧（`extraActions`），用户直接指出「登录按钮应该是放到账号列表标题的右侧啊」。
- **说明（`hint`）在 `standalone` 里由区块画在标题行下方；`embedded` 里由卡片画在自己的第二行**（`metaEntries`）。归属判据是「这句话讲的是这张卡片，还是讲这一串行」。
- **说明句子的样式也由组件给**（`OfficialAccountHint`）：两条渲染路径都必须用它，不许一边用 antd `<Text type="secondary">`、另一边手写 `--color-text-secondary`。这两个名字看着等价，实际差一档灰度（antd 的 secondary = `colorTextDescription` = alpha 0.45；应用 CSS 变量是 0.65），于是 ZCode 那句比另外两家深一点（用户 2026-10-10 发现）。**「同一句话」= 同一个组件，不是一个字符串加两处手抄样式。** 为此 `ProviderCardMetaEntry.value` 从 `string` 放宽成 `ReactNode`：值本身可以是个共享组件，style card 只管它**放哪**。
- **折叠箭头只属于列表行**，永远不在标题行上。开合的是列表；标题行是这一块的身份证，两端的读法一致。
- **措辞统一在 `common.officialAccount.*`。** `headingTitle`（官方账号）与 `listTitle`（账号列表）是**三端同一句话**，各 CLI 不再各写一份——用户明确要求「Kimi Official 标题都统一使用官方账号」；`官方订阅` 那个类别标记也随之从官方卡片上去掉（标题已经说明它是什么）。**每个 CLI 仍自己传已翻译串**（区块不认识任何 CLI），但传的是同一批 key。
- **折叠能力按宿主给，默认状态也可以不同**：列表卡片上三端都有折叠（Codex 默认收起——它的卡片下面还有模型区；Kimi / ZCode 默认展开——它们的账号一直露在外面）；**明细侧栏那处不折叠**（2026-10-12 用户指定：侧栏打开来就是为了读这些行）。共享的是「能力」，不是「初始状态」，也不是「谁都得有」。开关就是 `collapsed` / `onToggleCollapsed` **两个都传才画 caret**。
- **卡片布局下三端一个处置：区块进那张格子的详情侧栏（常开、无 caret），格子只陈述「当前登录: <账号>」。** 区块是一行一个账号、每行各有动作的**管理器**，一个格子放不下；格子第二行因此说它讲的这件事（`common.officialAccount.currentLogin`，值取该账号在自己那行的 label）——Codex 的官方格子、Kimi 的官方格子、ZCode 的官方账号格子因此逐字同形（2026-10-14 用户指定：「kimi和codex的官方账号卡片上也显示当前登录账号，不要显示主模型了」）。**那一行在任何状态下都必须说话**（同日用户报「为什么 codex 和 kimi 的卡片下面没有显示当前的账号啊」后定稿）：取值顺序＝**应用的账号（`isApplied`）→ 否则磁盘上那份登录（`isVirtual`）→ 否则 `common.officialAccount.currentLoginNone`（未登录）**，绝不留空——它的位置无论说不说都被共享卡片占着，空着读起来是坏卡片。**列表布局那侧一字未改**：卡片的 footer（Codex / Kimi）与自持卡片（ZCode）照旧，标题行、说明行、折叠都在。
- **`loginAction` 是插槽，不是布尔开关。** Codex/Kimi 是一个按钮，ZCode 是「选 OAuth provider」的下拉菜单。做成 `showLogin` + `loginProviders` 之类的 prop，等于把下拉的形态知识塞进共享组件——它就不该知道一个 CLI 有几个官方 provider。
- **行级动作按语义固定五个**（`onApply` / `onSaveLocal` / `onRefresh` / `onViewDetails` / `onDelete`），**谁传哪个就渲染哪个**。CLI 没有的动作不传即可，组件不做「这个 CLI 有没有这个能力」的判断。
- **虚拟行（`isVirtual`）只给一个动作：保存。** 它镜像的是磁盘上已经生效的那份登录——切到它自己是无意义的，删掉它也不会让 CLI 登出。保存之后它变成普通行，切换/删除随之出现。
- **已应用的行不画切换按钮**（画「默认」徽章）。曾把「切换」画成 disabled：一个灰掉的「切换」挨着「默认」，说的是同一件事两遍。
- **`pending` 带的是 `{accountId, action}`，不是一个 id。** 同一时刻可能有两行在不同动作上（Codex 刷新一个、保存另一个），只给 id 会让转圈落在错误的按钮上。

## 关键约束（改这里之前必读）

- **行标记靠结构，不靠 class 名。** 夹具通过「带内联 `border-bottom` 的行」定位账号行——`ant-space` 之类会同时命中卡片头部的动作簇（那里也有 `CheckOutlined`），把行数算错。
- **几何必须量，不能读代码。** 账号行右边缘 == 卡片内容右边缘 == 头部动作右边缘；「登录按钮在列表标题行的右侧」也只能由坐标断言（`login.top > heading.bottom` 且 `login.left >= listTitle.right` 且两者中线的差 ≤ 8px）。把区块塞进头部两栏的**左栏**里，右边缘会静默少掉一组动作链接的宽度（Codex 上实测 160px，用户报成「右侧空了一大块」）。
- **卡片的壳（边框、拖拽把手、悬停反馈、内容列）一律来自 `CardShell`，不许手抄。** ZCode 的官方卡片曾经自己写 `<Card>` + `useSortable` + 把手：于是把手丢了悬停反馈、还被放进标题行内部，正文因此贴到卡片左边缘（用户圈出「为什么下面的内容左边距没了」）。把手术：外壳换 `CardShell`，把手自成一列，正文在内容列里——与所有供应商卡片同构。
- **`actionsDisabled` 是「登录进行中，谁都别动」。** ZCode/Kimi 的登录会重写同一份凭据文件，中途点行级动作就是和它抢。
- **新增 prop 必须在至少一个调用方真的用上**，否则按「零消费方 prop 不许存在」删掉（同 `providerCardVariants` 的纪律）。`leadingAction` 就是这样被删掉的：把手换成 `CardShell` 之后它没有消费方了。
- **行的缩进（18px）只在画 caret 时施加**（`paddingLeft: collapsible ? 18 : 0`）——缩进的作用就是让开箭头，没有箭头就无事可让，不折叠的宿主里行直接与标题文本左对齐。画 caret 时**标题文本位置 = 折叠箭头宽度 + 6px 间距**，两者天然差 ~3px（箭头宽度来自图标字体）。ZCode 的浏览器检查因此用 ≤4px 容差——**别把它钉到像素级**，那是在钉字体而不是钉布局；真正要拦的是「整段贴到卡片边缘」那种 18px 级的偏差。
- **迁移一个 CLI 时不要顺手改措辞。** 但「区块标题 / 列表标题」这两句是**刻意统一**的共享 key；行内的「切换 / 保存当前登录 / 默认」仍是用户确认过的口径：账号之间叫**切换**，渠道之间才叫**应用**。

## 使用方

| CLI | 宿主 | 标题行在哪 | 说明在哪 | 折叠 |
|---|---|---|---|---|
| codex | `embedded`（官方供应商卡片内；同一块内容还出现在卡片视图的详情侧栏，那一处**不折叠**） | 卡片名称行（`namePrefix` + `OfficialAccountCount`） | 卡片第二行（`metaEntries`） | 列表卡片有、默认收起 |
| kimi | `embedded`（官方供应商卡片内；卡片布局下这块内容在**该格子的详情侧栏**，那一处不折叠） | 同上 | 同上 | 有，默认展开 |
| zcode | `standalone`（列表里是自己的卡片，壳用 `CardShell`；卡片布局里是**一个普通格子 + 该格子自己的详情侧栏**，侧栏那份不折叠） | 列表：区块自己画（`headingTitle`）；卡片布局：格子的标题行（`namePrefix` + `OfficialAccountCount`），侧栏用面板标题 | 区块画在标题行下方（`OfficialAccountHint`） | 有，默认展开 |
| geminicli | `embedded`（官方供应商卡片内；卡片布局下这块内容在**该格子的详情侧栏**，那一处不折叠） | 卡片名称行（`namePrefix` + `OfficialAccountCount`） | 卡片第二行（`metaEntries`，`OfficialAccountHint`） | 有，默认收起（迁移前即如此） |
| grok | 同上 | 同上 | 同上 | 有，默认收起（迁移前即如此） |
| antigravity | `standalone`（独立账号 Collapse 分区内，不套外层 `<Card>`） | 外层 Collapse 标题（不传 `headingTitle`） | 区块内 `hint`（`OfficialAccountHint`） | 有，默认展开 |

> 全仓 `PENDING_MIGRATION` 已清零（antigravity 于 2026-11-23 迁完）。geminicli / grok 已于 2026-10-20 按同一份映射规则接入。

### 行内的 CLI 专属动作：`renderExtraActions`

- `renderExtraActions?: (account: OfficialAccountRowView) => React.ReactNode`，渲染在「刷新」之后、「详情」之前。Grok 的「刷新 Token」（重新签发 OAuth token）走这个槽——它是唯一需要第六个行内动作的 CLI。
- **没有做成具名动作**（如 `onRefreshToken`）：那会把一个 CLI 的语义塞进共享类型，而共享类型按「行级动作的语义」定义，Grok 的 token 续签不属于任何一条公认语义。插槽返回节点，按钮的形态（图标、字号、`paddingInline`）与 loading 都由调用方自带——区块不知道也不该知道某个 CLI 有没有这个能力。
- 调用方按行 id 回查原始账号记录；返回 `null` 即这一行不渲染。Grok 只用它渲染非 `__local__` 的行。新增消费方时保持「插槽是自由的、语义留在调用方」这条边界。

守卫：`pnpm run test:official-accounts-shared`（源码棘轮，禁止再手写行级按钮）、
`pnpm run test:codex-official-accounts`、`pnpm run test:official-account-card`（Kimi）与
`pnpm run test:zcode-official-account`（真浏览器：几何 + 标题形状 + 折叠真点击 +
**说明句的 `getComputedStyle` 颜色/字号**——三端各一份，接线各测各的）。

## 何时更新本文件

- 新增第四种宿主形态：先确认它确实是**宿主差异**而非 `variant` 的参数变体。
- 某条经验上升为跨模块通用规则时，同步补到根 `AGENTS.md` 与 `docs/new-cli-onboarding-sop.md`。
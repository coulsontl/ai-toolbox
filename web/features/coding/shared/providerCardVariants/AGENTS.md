# 供应商卡片三样式（providerCardVariants）

## 一句话职责

- 把「CLI 供应商卡片长什么样」这件事**固定成三种样式**，各 CLI 按自己的 provider 模型形状选用其一，不再各写各的布局。

> **「卡片视图」不是这里的第四种样式。** `shared/providerCardView/` 是同一批数据的另一种**呈现密度**（紧凑卡片网格），由用户的布局偏好切换，与 provider 模型形状无关。本模块的三种样式按语义选型；把显示偏好混进来会让「该用哪种样式」这个问题失去意义。两者的共用面是 `cardChrome.ts`（accent）、`providerCardMenu.tsx`（「更多」菜单）、`providerCardActions.tsx`（底部两个主按钮）与 `ProviderModelSection.tsx`（`modelSection.*` → `ModelListSection` 的映射），改通用行为改这四处。

## Source of Truth

- 样式的事实源就是这三个组件文件本身。`index.ts` 的对照表是**选型规则**（哪个 CLI 用哪种、依据是什么），不是样式定义。
- 各 CLI 传给卡片的 `ProviderCardVariantProps` 是**纯展示数据**：卡片不读任何 CLI 的存储格式，映射由各 CLI 自己的 `*ProviderCard.tsx` 完成。
- `CardShell` 是三种样式共用的外框，**拖拽注册、选中复选框、卡片边框/阴影只在这里定义一次**。

## 三种样式与选型

| 样式 | 何时用 | 判定问题 | 第二行 | 头部主操作 |
|---|---|---|---|---|
| `ClaudeStyleCard` | 没有模型目录（模型写在 provider 配置里） | CLI 是否管理一个模型清单？否 → 此样式 | 带标签的绑定（`默认: …` / `Haiku: …`） | 文字链「应用」 |
| `CodexStyleCard` | 有模型目录，且有单一 active provider | 是，且「应用」有意义 → 此样式 | 自由格式（端点/模型/masked key/备注） | 文字链「应用」 |
| `OpenCodeStyleCard` | 有模型目录，且**没有**单一 active provider | 是，且「启用哪个渠道」不成立 → 此样式 | 固定顺序 `ID • SDK • 端点` | **无**（默认在模型行上选） |

**选型依据是 provider 模型形状，不是个人偏好，也不是「这个 CLI 现在长什么样」。** 一个 CLI 后来获得或失去模型目录，就应该换样式——这是设计意图，不是例外。

## 核心设计决策（Why）

- **三种，不是一套可配置的 prop 组合。** 样式差异的本质是**语义差异**（有没有模型目录、有没有单一 active provider），不是参数差异。做成一套带十几个开关的组件，每个调用点都要重新决定一遍组合，等于把分叉从「文件级」挪到「调用点级」，没有解决问题。
- **`OpenCodeStyleCard` 刻意没有头部「应用」按钮。** 这类 CLI 里多个渠道同时可用，「应用」会暗示其他渠道被关掉。它的「用哪个」表达在**模型行**的「设为默认」上。给这类卡片加回一个头部应用按钮是**语义错误**，不是样式偏好。
- **样式组件只做布局，不做数据映射。** 各 CLI 的 `*ProviderCard.tsx` 保留为薄映射层（把存储形状转成 `ProviderCardVariantProps`）。这样样式统一了，但各 CLI 的存储语义仍留在自己的模块里。
- **`metaEntries` 由调用方决定内容，样式只决定位置。** 这是三种样式能共用同一份 props 的原因：差异在「这行放在哪、怎么排」，不在「这行装什么」。

## 关键约束（改这里之前必读）

- **`CardShell` 的 `setNodeRef` 必须无条件挂载。** dnd-kit 需要测量节点来计算 transform；未注册的节点在父级重新启用拖拽时拖不动。写成 `draggable ? setNodeRef : undefined` 会引入「禁用一次就再也拖不动」的隐性 bug。
- **`useSortable` 每次渲染都必须执行。** 没有 id 时传占位符并置 `disabled`，不要条件调用 hook——违反 hooks 规则会在拖拽开关切换时崩。
- **卡片自己留底部间距（`marginBottom: 12`），不靠父容器的 `gap`；间距挂在外层 `div` 上，不挂在 antd `Card` 上。** 这样重排后间距不会错位；靠父容器 gap 时，把官方账号卡片之类的异类插进列表会丢间距。间距必须在**外层**是因为外层才是「这张卡片的盒子」：它带 `data-provider-id`、也承载定位闪光环（`ProviderListSection` 的「定位」），写在 `Card` 上会让间距落在盒子**内部**（flex item 不会与子元素折叠外边距），环就画到卡片下方 12px 处去了。视觉上两种写法等价（间距值不变），差别只在盒子边界。
- **每张卡片必须带 `data-provider-id={sortableId}`（外层 `div`）。** 这是「定位当前已应用供应商」在 DOM 里找到卡片的唯一锚点：`ProviderListSection` 的定位动作按这个属性查询、滚动并闪光。复用 `sortableId` 而不是新增 id prop，因为 dnd-kit 本来就要求它在列表内唯一——正是定位查询需要的那条保证。删掉或改名会让定位静默失效（点了没反应，或者只弹「找不到」）。回归：`pnpm run test:provider-list-locate`。
- **卡片的拖拽把手带 `data-provider-card-handle`。** 卡片里的把手与它**模型行上的把手**（`ModelItem`）都是 `HolderOutlined`，所以「这张卡片里的那个把手图标」在模型列表展开后有两种答案：夹具按图标找会抓到模型行，报出「非自定义排序下也能拖」这种不存在的结论（2026-10-12 `ProviderListDragFixture` 实际发生过）。夹具要按这个属性定位，别按图标。
- **「已应用 / 网关 P0」的高亮属于 `CardShell`，不属于映射层。** 四个 bespoke 卡片各自抄了一份「选中 > 网关 P0 > 已应用」的优先级；迁到共享组件时如果不把它一起搬进来，卡片会静默变成统一的灰边框——**没有任何报错，只是状态看不见了**。映射层只负责把 `providerState.accent` 算出来。
- **标签与它的值必须在同一个 `metaEntries` 项里。** 拆成两个项（`{kind:'text', value:'Haiku:'}` + `{kind:'code', value:'…'}`）数据上等价，渲染上不等价：行的 `gap: 16px` 会插到标签和值之间，而 `alignItems` 让两者的盒子按顶边对齐——`<code>` 的 padding 一撑，标签就明显偏高。这是 Claude Code 卡片上真实出现过的错位。用 `entry.label`，组件把它渲染成同一个 flex 项。
- **`metaEntries` 那一行永远只有一行：条目压缩，尾部动作不换行**（2026-10-21）。行容器是 `flexWrap: 'nowrap'`，每个条目 `minWidth: 0`、`code`/`text` 值开 antd `ellipsis`，`inlineActions` 外面那个 `<span>` 是 `flexShrink: 0`。用户报的是 Claude Desktop 卡片：端点 + 四个角色绑定把「模型测试」挤到第二行。判据是**动作属于这一行，内容是这一行可以压的**——先省略事实，不要折行（同 `ProviderCompactCard` 的 `subtitle` 与 `actions` 的关系）。回归：`test:migrated-provider-cards` 的「the meta line keeps 模型测试 on the same row as the facts」（几何断言：行高 ≤26px + 动作与端点的中线差 ≤6px + 不越出卡片；夹具专门复刻「长端点 + 四个角色绑定」那种会溢出的形状，别读代码）与「a compressed entry still carries its full value on hover」。
- **`metaEntries` 里的每个条目把未截断的值写进 `title`**（2026-10-21）。一行是硬约束，被压掉的端点/模型名必须还能读到——用原生 `title`，不要换成 antd 的 ellipsis tooltip（后者会接管这一行的测量）。**`value` 是节点时（官方账号说明句由 `OfficialAccountHint` 渲染）字符串回退失效，必须显式传 `entry.title`**，否则窄窗口下那句话被截断且无处可读；`title` 属性挂在外层条目 span 上，标签 + 值一起触发。`inlineActions` 为 `undefined` 时外层不渲染那个 `flexShrink: 0` 的 span：空 flex 项仍会吃掉一个 gap。
- **`primaryAction.disabledReason` 是「为什么不能点」，只在 `disabled` 时出现**（2026-10-21）。`tooltip` 是「这个动作做什么」，按钮可用时也要显示；把禁用原因写进 `tooltip` 会让可用状态的按钮也在解释一个不存在的阻塞。Grok 官方渠道没有账号时就是这样锁住「应用」并说明原因（见 `grok/AGENTS.md`）。禁用**不要**改成隐藏：缺按钮会被读成缺功能。
- **`transparentRows` 自带透明规则**（`ModelListSection.module.less`）。该规则曾以 `.codex-model-list-collapse` / `.grok-model-list-collapse` 的形式在 CLI 侧各存一份，迁移时漏 import 就会在**有底色的卡片**上露出白色标题条——没有底色时完全看不出来，所以能潜伏很久。现在由共享组件统一施加，CLI 侧不要再复制：卡片形态的 Collapse 每一层都由 `transparentCollapse` 清掉，行（`ModelItem`）默认画自己的底色，由 `transparentRows` 清掉。
- **改通用行为（间距、按钮、拖拽、选中态）改在变体组件里。** 只在某个 CLI 的卡片里改，样式就会重新分叉——这正是本模块要消除的问题（见根 `AGENTS.md` Hard Rule 14）。
- **有状态的回调必须**透传新值**，不要让调用方自己算。** `onToggleDisabled: (enabled: boolean) => void` 与 antd `Switch.onChange` 同契约；组件里写 `onChange={onToggleDisabled}`，**不要**写成 `onChange={() => onToggleDisabled()}`。后者迫使映射层从当前状态反推新值，推反了就得到一个「渲染正常、点了没反应」的开关——不报错、类型也通过。2026-10-07 在 Claude Code / Codex 卡片上真实发生过。
- **props 的分组按语义层级，不按「当初谁在用」。** 卡片级的东西（拖拽、选中、禁用、高亮）进 `providerState`；模型列表级的（行、工具栏、批量删除）进 `modelSection`。判据是「**没有模型列表的样式还需不需要它**」——需要就放卡片级。`draggable` / `sortableId` 曾误放在 `modelSection` 下，于是 Claude 式卡片（无模型区）的拖拽把手静默消失。
- **同一个 interface 里的字段，不代表所有样式都会读。** `ProviderCardVariantProps` 是三个样式共用的扁平结构，加字段时要在**每个**样式的渲染点指认一次（同 13.1 模式二 / #70）。
- **样式统一不等于数值统一：原来就不一致的数值要按变体保留。** 两个值是真差异，不是分叉——判据是「原实现是不是这个值」（`git show <迁移前>:<原文件>`）。
  - body padding：`16`（claudecode/codex 迁移前就是 16，claudedesktop/geminicli/grok 迁移前也是 16，迁后保持）。曾从 `components/common/ProviderCard` 抄了 `8px 12px`，导致每个迁移过的 tab 静默少 8px 上下。
  - 标题→副标题间距：**Claude 式 8**（原实现是 `Space size={4}` 叠在带 `marginTop: 4` 的行上），**Codex 式 4**（原实现是 `flexDirection: column; gap: 4`）。
  - 两条都有浏览器断言：`pnpm run test:provider-card-spacing`（只挂卡片组件本身，不挂页面）。**间距类改动必须量，不能读代码**（SOP 13.1 模式六十三 / 六十五）。
- **`footer` 必须渲染在头部两栏之外。** 头部是「左内容 / 右动作链接」两栏 flex（`justify-content: space-between`），任何放进**左栏内部**的区块，右边缘都会比卡片少一截——正好是那组动作链接的宽度（Codex 官方账号区实测差 160px）。2026-10-08 就是这样：`footer` 被写在左栏里，账号行的「切换 / 删除」比下方模型区工具栏短一截，用户报成「账号列表右侧空了一大块」。**凡是卡片的一个「区」，就应该和模型区一样跨满整宽**；判据是**用几何量对齐**，不是看代码：`scripts/verify-codex-official-accounts.mjs` 断言账号行右边缘 == 卡片内容右边缘 == 头部动作右边缘。

## 跨模块依赖

- `features/coding/shared/ModelListSection`：模型列表区的**唯一实现**（列表卡片里是可折叠的，`appearance` 见下）。Codex 式和 OpenCode 式都用它，不要另写一份。它不支持 official models，需要展示只读目录的 CLI 走 `footer` 插槽。
- `ProviderModelSection.tsx`（本模块）：`ProviderCardModels` → `ModelListSection` 的**唯一映射**，`transparentRows` 固定开启。两个风格卡片与 `providerCardView` 的详情侧栏都渲染它——同一份目录因此带着**同一组工具**出现在卡片内和侧栏里（批量删除 / 模型测试 / 获取模型 / 添加模型）。**侧栏那处传 `appearance="detail"`，风格卡片不传**（默认 `'card'`）：这是两侧唯一的差别，折叠状态与行外观都由此决定，数据对象是同一个。这份映射曾在 `CodexStyleCard` 与 `OpenCodeStyleCard` 里各写一遍，漂移出一条（`onToggleModelDisabled` 只有 OpenCode 转发）。
- `features/coding/shared/management`（`ManagementCheckbox`）：批量选择的复选框。
- `features/coding/shared/providerConnectivity/ProviderConnectivityStatus`：连通性状态点。
- `components/common/ProviderNameLink`、`components/common/SdkTag`：名称链接与 SDK 标签。
- `features/coding/shared/providerCardView/`：紧凑卡片网格视图（同一批数据的另一种呈现密度，不是第四种样式）。共用 `cardChrome.ts`、`providerCardMenu.tsx` 与 `providerCardActions.tsx`；`ProviderCompactCard` 是它自己的 chrome 宿主，与 `CardShell` 同级。
- 使用方：`claudecode` + `geminicli` + `claudedesktop`（Claude 式）、`codex` + `kimi` + `grok`（Codex 式）、`zcode` + `omo_native` + `opencode` + `openclaw` + `hermes`（OpenCode 式）。三种样式**均已落地**，`scripts/verify-provider-card-layout.mjs` 的 `PENDING_MIGRATION` 已清空——全仓不再有 bespoke 供应商卡片，也不再有任何映射层 import `components/common/ProviderCard`（2026-10-21 起由守卫机械拦截）。
- 2026-10-20 迁完最后三家时给 Claude 式补了三个能力（都是 Codex 式已有的既定插槽，不是新机制）：
  - **`namePrefix`**：Gemini 的官方卡片要 `🔗 官方账号 (n)`，🔗 走这个名字前缀槽。
  - **`kind: 'tag'`**：Claude 式此前把 tag 降级成次要色文本；Gemini 的模型标签与 Claude Desktop 的 `tierAlias` 需要真 `Tag`。
  - **`footer` 移出头部两栏**：Claude 式此前把它放在左栏内，会重现「账号行右侧空一大块」的缺陷（见下文 footer 约束）。Claude Code 的备注 footer 因此变宽——这是既定修正。
  - `inlineActions` 的渲染条件从「有 `metaEntries`」放宽为「有 `metaEntries` **或** `inlineActions`」，官方卡片没有 meta 行时连通性按钮仍在。
- **2026-10-21：`openclaw` 与 `hermes` 迁到 OpenCode 式。** 两家都是「per-provider 模型目录 + 没有 provider 级 应用」的形状（OpenClaw 的 `agents.defaults.model.primary`、Hermes 的 `model.defaultProvider` 都在别处编辑），所以头部没有主操作。两家的模型目录探针（模型测试 / 获取模型）都从旧卡的头部 `extraActions` 移进了 `modelSection`（工具栏），与 ZCode / Codex 一致——位置变了，写进提交说明。
  - **OpenClaw**：`apiToNpm` 从页面抽到 `utils/apiToNpm.ts`（页面、映射层、连通性/获取模型弹窗共用）。删除确认下沉到页面 handler（`Modal.confirm`）并令 `deleteConfirm: false`：旧卡只在列表上包了 `Popconfirm`，格子菜单与详情侧栏是绕过它的两个入口。
  - **Hermes**：新增 `components/HermesProviderCard.tsx`。只读渠道（`providers:` 字典）不传编辑/复制/删除与模型增改，并用 `nameTags` 画「只读」标签——此前这条事实被拼进了 Base URL 的兜底文本（`<key> (只读)`），既伪造了 URL 又不可靠。**模型行不画把手**：Hermes 没有模型重排命令，旧卡把模型把手画成可拖但 `onReorderModels` 从未传过。**「复制」顺手修好**：此前打开的是空白新增弹窗，现在预填源渠道、Key 加 `_copy`、弹窗标题走「添加」分支（与 OpenClaw 的 `isCopy` 同型）。**供应商卡片可以拖**（2026-10-22 补）：Hermes 把可写渠道存成 `custom_providers` YAML **序列**，序列顺序就是显示顺序，拖动调 `reorder_hermes_providers` 重写它；只读 `providers:` 字典行没有可拖的位子（`draggable: !isReadOnly && !selectable && !dragDisabled`）。在此之前的读取端用 `BTreeSet` 重建列表，任何顺序都会被读回成字母序。
- **第二行只承载「这张卡片要陈述的事实」，用户的备注不要被官方分支吞掉。** Gemini / Grok 的官方卡片把连接事实换成了切换说明（`OfficialAccountHint`），但备注在两种模式下都可编辑、迁移前的卡片也照画——所以官方分支在说明之后仍追加 `provider.notes`；Claude Desktop 则在**没有角色绑定**时把备注放进第二行（有绑定时进 footer，与 Claude Code 同规则）。2026-10-20 的首次迁移漏了这三处，审阅时由「官方渠道的备注会消失」发现。
- **固定成员不要再用类别标签重复标题。** 一个由应用命名、位置固定的成员（Claude Desktop 的官方渠道）标题已经写成 `🔗 官方账号`，再挂一枚「官方订阅」标签就是把同一句话说两遍（SOP §4.2.9）。这类成员也不该出现删除入口：`actions.onDelete` 直接不传，列表菜单、格子菜单与详情侧栏三处会一起消失——比"渲染一个 disabled 的删除"更准确，因为那张行删了也会被下次列表 seed 回来。**只保护"固定成员"本身**：同类别但非固定的行（历史重复、用户复制出来的）仍是普通行，照常有删除。

## 迁移一个 CLI 的步骤

1. 按上表判定该 CLI 属于哪种样式（看它有没有模型目录、有没有单一 active provider）。
2. 把原 `*ProviderCard.tsx` 改成薄映射层：解析自己的存储形状 → 构造 `ProviderCardVariantProps` → 渲染对应样式组件。
3. 原卡片里**该 CLI 特有的业务逻辑**（如 ZCode 的 `isDefault` 双写、网关接管按钮）留在映射层，不要塞进样式组件。
4. 检查被删掉的 props 是否在页面侧变成死代码（`onApply`、`onTest` 之类），一并清理。
5. 核对原卡片是否有样式组件没有的能力（官方模型只读列表、网关标签、优先级徽章）——有则用 `footer` / `nameTags` / `inlineActions` / `gatewayActions` 插槽补，**不要**为此给样式组件加 CLI 专属 prop。
6. **逐条对照原卡片的 `style={{` / `className=`**，而不只是对照 props：状态类样式（边框色、底色、透明度）不经过 props，最容易整块丢掉（`accent` 就是这么漏的）。同时按 class 名反查 CLI 的 `.less`，看有没有属于「组件在自己容器里该怎么显示」的规则该搬进来（`transparentRows` 就是这么漏的）。
7. **跑 `pnpm run test:provider-card-layout`，并从脚本的 `PENDING_MIGRATION` 里删掉这个文件**。守卫是「棘轮」：迁完不删会报错，没迁却不在名单里也会报错。

## 插槽清单（补能力时先看这里）

| 插槽 | 位置 | 用途 | 现有消费方 |
|---|---|---|---|
| `providerState.accent` | 卡片外框（`CardShell`） | `applied`（主色边框 + 选中底色）/ `gatewayPrimary`（成功色 + 渐变）；批量选中优先于两者 | claudecode、codex、kimi、geminicli、grok、claudedesktop |
| `providerState.draggable` / `sortableId` | 卡片级拖拽把手（`CardShell`） | **卡片级**顺序；不带模型区的样式也要能拖，所以不放 `modelSection` | 九个 coding 页全部（claudecode、claudedesktop、codex、geminicli、grok、kimi、omo_native、opencode、zcode） |
| `namePrefix` | 名称**之前** | 被改名的卡片补前缀图标（官方卡片的 🔗） | codex、kimi、geminicli、grok、zcode（官方账号卡片） |
| `nameTags` | 名称右侧 | 已应用 / 官方 / 代理 / 网关优先级徽章、官方账号计数 | claudecode、claudedesktop、codex、geminicli、grok、kimi、zcode |
| `metaEntries` | 第二行 | 有序的 `text` / `code` / `tag` 项；**标签用 `entry.label`，不要拆成前一个 `text` 项** | claudecode（角色绑定）、claudedesktop（角色路由 + tierAlias tag）、codex、geminicli、grok、kimi |
| `inlineActions` | 第二行末尾 | 行内动作（连通性测试、CLI 启动） | claudecode、codex、kimi、claudedesktop（`InlineConnectivityButton`）。**Grok 刻意不传**：它的模型工具栏已经是唯一入口，再放一个就是同一张卡片说两遍 |
| `footer` | **头部两栏之下、模型区之上**（与模型区一样**跨满卡片整宽**） | 自由区块（官方账号折叠区、备注、只读模型目录） | codex、geminicli、grok、kimi、claudecode（备注）、claudedesktop（备注）、opencode（只读目录） |
| `actions.gatewayActions` | 头部主操作**之前** | 网关接管/恢复直连/切换主渠道 | claudecode、codex、kimi、geminicli、grok、claudedesktop |
| `actions.primaryAction` | 头部主操作 | 文字链「应用」 | claudecode、codex、kimi、geminicli、grok、claudedesktop |
| `actions.enabledStateLabel` | 「更多」菜单里启用开关的副标题 | `配置已启用` / `配置已禁用`（各 CLI 措辞不同，不共用 `common.provider.*`） | claudecode、codex、kimi、geminicli、grok、claudedesktop |
| `modelSection.aboveList` / `renderModelExtraActions` | 模型区内 | Codex 的自动审批行与行级动作 | codex |
| `modelSection.className` / `bodyStyle` | 模型 Collapse | 缩进适配（透明背景由 `transparentRows` 负责） | codex |

## 「更多」菜单是契约，不是装饰

> **菜单的构建已抽到 `providerCardMenu.tsx`**（`buildProviderCardMenuItems` / `dispatchProviderCardMenuClick`），因为 `shared/providerCardView` 的紧凑卡片也要用同一份。顺序、图标、分隔线、副标题文案都在那里定义一次；改菜单改那一处，不要在某个卡片里另写一遍。`verify-provider-card-layout.mjs` 的 prop 覆盖检查把该文件也列为渲染点（`enabledStateLabel` 只在那里渲染）。

Claude / Codex 式的菜单**固定为**：启用（含副标题）→ 编辑 → 复制 → 分享 → 分隔线 → 删除，每项带图标，`trigger={['click']}`。

**那颗启用开关共用一个实现：`ProviderEnabledSwitch`（`providerCardActions.tsx`）。** 它出现在三处——列表卡片的头部（`OpenCodeStyleCard`，也是 ZCode 的列表卡）、**把动作折叠起来的卡片的「更多」菜单里**（Claude / Codex 式，以及带主操作的紧凑卡片）、以及**没有主操作的格子自己的动作槽**（`ProviderCompactCard` 的 `cellControl`，2026-10-15 用户指定：「针对 zcode 这种不需要应用的渠道，把它在列表模式下的开关按钮放出来，放到现在应用的地方」）。三处必须是同一个控件：`checked` 说的是**启用**（记录里存的是 `isDisabled`，全仓唯一的取反就在这里），尺寸、tooltip 文案（`common.provider.*` 的已启用 / 已禁用）都在组件里定义一次。**菜单那份带 `tooltip={false}`**——菜单项在自己的行上已经写了「启用」，tooltip 会把同一件事说两遍。
> **开关只在「旁边没有主操作」时才从菜单里出来**：菜单把它收起来本来就是因为「一颗 Switch 紧挨着 应用 / 网关代理 会与之争抢」（见 `providerCardMenu.tsx` 的注释）。ZCode 的 provider 永远不被应用（`设为默认` 在模型行上），格子的动作槽是空的，开关就成了这张格子唯一的控件——所以它上槽、菜单把那一条让出来（`ProviderCompactCard` 里 `cellControl ? undefined : onToggleDisabled`）。同一个控件在同一张卡片上说两遍正是本仓反复拦的那类事。

这些细节（图标、分隔线、副标题文案、点击而非 hover）**迁移前就存在**，用户看得见。迁移时把它们简化掉——比如只留 `{ key, label }` 而丢掉 `icon` / `divider`，或把「编辑」提到头部——**不会有任何报错**，只是菜单长得像另一个产品。改这个菜单前先 `git show <迁移前的 commit>:<原文件>` 对照一遍。

**原实现里契约之外的菜单项，按语义找插槽，不要删也不要破例。** 例：Kimi 把「连通性测试」放在菜单里（全仓唯一），而它本质是「对模型目录的探测」——迁到模型区工具栏（`modelSection.onTestModels`）与卡片第二行（`inlineActions`），与同样有模型目录的 Codex 卡一致。位置变了就是行为变了：**写进提交说明**，否则用户按肌肉记忆去菜单里找会找不到（同 #103）。

> **每个可选 prop 都必须在某个样式里有渲染点**。声明了却没渲染 = 调用方传了等于没传，且类型检查完全通过（历史坑 #70、#97）。守卫脚本的「渲染点」文件表除三个样式外还列了三个共享文件（`providerCardMenu.tsx` / `providerCardActions.tsx` / `ProviderModelSection.tsx`）——**把某个 prop 的渲染搬进共享文件时，那个文件必须同时在表里**，否则守卫会把活着的能力报成死的（2026-10-12 搬 `modelSection.*` 映射到 `ProviderModelSection` 时真实发生）。2026-10-07 删掉了三个零消费方 prop：`metaEntries` 的 `kind: 'id'` / `'sdk'` 与 `ProviderCardModels.modelSourceTag`。

## 最小验证

- `pnpm run test:provider-card-layout` 通过（薄映射层守卫）。
- 该 CLI 页面能正常列出卡片：名称、第二行、操作按钮、模型折叠区（若有）与迁移前一致。
- 拖拽：能拖动排序；在搜索/非自定义排序下拖拽被禁用（**且界面说明了原因**——把手消失而没有任何解释时，用户会把「有意禁用」报成「拖拽坏了」，2026-10-08 真实发生）；批量选择模式下拖拽句柄换成复选框。
  > **必须用真事件验证，不能只读链路。** `pnpm run test:provider-list-drag` 在真 Chromium 里对真实 `CodexPage` 发指针事件并核对回写。这条守卫存在的原因就是：拖拽被「读链路确认每一层都对」的方式修好过两次，两次都没修好（SOP #112 / 13.1 模式六十）。
- 选中态：批量选择时卡片边框变主色。
- **高亮态**（若有 `accent`）：已应用的卡片是主色边框 + 选中底色；网关 failover 的 P0 卡片是成功色边框 + 渐变底。两者同时成立时 P0 胜出。
- 若该 CLI 有「设为默认」：点击后卡片标记与磁盘上的运行时指针**同时**更新（只查一处会漏掉脱节）。
- 禁用开关（若有）：能翻转状态，卡片随之变灰。

## 何时更新本文件

- 新增第四种样式时：先确认它确实是**语义差异**而非现有样式的参数变体；是的话在根 `AGENTS.md` 的 Index 与 SOP §4.2.1 同步登记。
- 某条经验上升为跨模块通用规则时，同步补到根 `AGENTS.md`。

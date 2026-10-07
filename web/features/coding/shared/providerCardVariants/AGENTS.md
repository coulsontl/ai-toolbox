# 供应商卡片三样式（providerCardVariants）

## 一句话职责

- 把「CLI 供应商卡片长什么样」这件事**固定成三种样式**，各 CLI 按自己的 provider 模型形状选用其一，不再各写各的布局。

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
- **卡片自己留底部间距（`marginBottom: 12`），不靠父容器的 `gap`。** 这样重排后间距不会错位；靠父容器 gap 时，把官方账号卡片之类的异类插进列表会丢间距。
- **「已应用 / 网关 P0」的高亮属于 `CardShell`，不属于映射层。** 四个 bespoke 卡片各自抄了一份「选中 > 网关 P0 > 已应用」的优先级；迁到共享组件时如果不把它一起搬进来，卡片会静默变成统一的灰边框——**没有任何报错，只是状态看不见了**。映射层只负责把 `providerState.accent` 算出来。
- **标签与它的值必须在同一个 `metaEntries` 项里。** 拆成两个项（`{kind:'text', value:'Haiku:'}` + `{kind:'code', value:'…'}`）数据上等价，渲染上不等价：行的 `gap: 16px` 会插到标签和值之间，而 `alignItems` 让两者的盒子按顶边对齐——`<code>` 的 padding 一撑，标签就明显偏高。这是 Claude Code 卡片上真实出现过的错位。用 `entry.label`，组件把它渲染成同一个 flex 项。
- **`transparentRows` 自带透明 Collapse**（`ModelListSection.module.less`）。该规则曾以 `.codex-model-list-collapse` / `.grok-model-list-collapse` 的形式在 CLI 侧各存一份，迁移时漏 import 就会在**有底色的卡片**上露出白色标题条——没有底色时完全看不出来，所以能潜伏很久。现在由共享组件统一施加，CLI 侧不要再复制。
- **改通用行为（间距、按钮、拖拽、选中态）改在变体组件里。** 只在某个 CLI 的卡片里改，样式就会重新分叉——这正是本模块要消除的问题（见根 `AGENTS.md` Hard Rule 14）。
- **有状态的回调必须**透传新值**，不要让调用方自己算。** `onToggleDisabled: (enabled: boolean) => void` 与 antd `Switch.onChange` 同契约；组件里写 `onChange={onToggleDisabled}`，**不要**写成 `onChange={() => onToggleDisabled()}`。后者迫使映射层从当前状态反推新值，推反了就得到一个「渲染正常、点了没反应」的开关——不报错、类型也通过。2026-10-07 在 Claude Code / Codex 卡片上真实发生过。
- **props 的分组按语义层级，不按「当初谁在用」。** 卡片级的东西（拖拽、选中、禁用、高亮）进 `providerState`；模型列表级的（行、工具栏、批量删除）进 `modelSection`。判据是「**没有模型列表的样式还需不需要它**」——需要就放卡片级。`draggable` / `sortableId` 曾误放在 `modelSection` 下，于是 Claude 式卡片（无模型区）的拖拽把手静默消失。
- **同一个 interface 里的字段，不代表所有样式都会读。** `ProviderCardVariantProps` 是三个样式共用的扁平结构，加字段时要在**每个**样式的渲染点指认一次（同 13.1 模式二 / #70）。

## 跨模块依赖

- `features/coding/shared/ModelListSection`：模型折叠区的**唯一实现**。Codex 式和 OpenCode 式都用它，不要另写一份。它不支持 official models，需要展示只读目录的 CLI 走 `footer` 插槽。
- `features/coding/shared/management`（`ManagementCheckbox`）：批量选择的复选框。
- `features/coding/shared/providerConnectivity/ProviderConnectivityStatus`：连通性状态点。
- `components/common/ProviderNameLink`、`components/common/SdkTag`：名称链接与 SDK 标签。
- 使用方：`claudecode`（Claude 式）、`codex` + `kimi`（Codex 式）、`zcode` + `omo_native`（OpenCode 式）。三种样式**均已落地**。
- 仍持 bespoke 卡片的三个（claudedesktop / geminicli / grok）已登记在 `scripts/verify-provider-card-layout.mjs` 的 `PENDING_MIGRATION` 里，只减不增。

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
| `providerState.accent` | 卡片外框（`CardShell`） | `applied`（主色边框 + 选中底色）/ `gatewayPrimary`（成功色 + 渐变）；批量选中优先于两者 | claudecode、codex |
| `providerState.draggable` / `sortableId` | 卡片级拖拽把手（`CardShell`） | **卡片级**顺序；不带模型区的样式也要能拖，所以不放 `modelSection` | claudecode、codex、zcode、omo_native |
| `nameTags` | 名称右侧 | 已应用 / 官方 / 代理 / 网关优先级徽章 | claudecode、codex、zcode |
| `metaEntries` | 第二行 | 有序的 `text` / `code` / `tag` 项；**标签用 `entry.label`，不要拆成前一个 `text` 项** | claudecode（角色绑定）、codex（端点/模型/key/备注）、zcode |
| `inlineActions` | 第二行末尾 | 行内动作（连通性测试、CLI 启动） | claudecode、codex（`InlineConnectivityButton`） |
| `footer` | 第二行下方、模型区上方 | 自由区块（官方账号折叠区） | codex |
| `actions.gatewayActions` | 头部主操作**之前** | 网关接管/恢复直连/切换主渠道 | claudecode、codex |
| `actions.primaryAction` | 头部主操作 | 文字链「应用」 | claudecode、codex |
| `actions.extraActions` | 头部图标按钮，在「更多」**之前** | 工具专属头部动作 | zcode、omo_native（OpenCode 式）；Claude/Codex 式亦可 |
| `actions.enabledStateLabel` | 「更多」菜单里启用开关的副标题 | `配置已启用` / `配置已禁用`（各 CLI 措辞不同，不共用 `common.provider.*`） | claudecode、codex、kimi |
| `modelSection.aboveList` / `renderModelExtraActions` | 模型区内 | Codex 的自动审批行与行级动作 | codex |
| `modelSection.className` / `bodyStyle` | 模型 Collapse | 缩进适配（透明背景由 `transparentRows` 负责） | codex |

## 「更多」菜单是契约，不是装饰

Claude / Codex 式的菜单**固定为**：启用（含副标题）→ 编辑 → 复制 → 分享 → 分隔线 → 删除，每项带图标，`trigger={['click']}`。

这些细节（图标、分隔线、副标题文案、点击而非 hover）**迁移前就存在**，用户看得见。迁移时把它们简化掉——比如只留 `{ key, label }` 而丢掉 `icon` / `divider`，或把「编辑」提到头部——**不会有任何报错**，只是菜单长得像另一个产品。改这个菜单前先 `git show <迁移前的 commit>:<原文件>` 对照一遍。

**原实现里契约之外的菜单项，按语义找插槽，不要删也不要破例。** 例：Kimi 把「连通性测试」放在菜单里（全仓唯一），而它本质是「对模型目录的探测」——迁到模型区工具栏（`modelSection.onTestModels`）与卡片第二行（`inlineActions`），与同样有模型目录的 Codex 卡一致。位置变了就是行为变了：**写进提交说明**，否则用户按肌肉记忆去菜单里找会找不到（同 #103）。

> **每个可选 prop 都必须在某个样式里有渲染点**。声明了却没渲染 = 调用方传了等于没传，且类型检查完全通过（历史坑 #70、#97）。2026-10-07 删掉了三个零消费方 prop：`metaEntries` 的 `kind: 'id'` / `'sdk'` 与 `ProviderCardModels.modelSourceTag`。

## 最小验证

- `pnpm run test:provider-card-layout` 通过（薄映射层守卫）。
- 该 CLI 页面能正常列出卡片：名称、第二行、操作按钮、模型折叠区（若有）与迁移前一致。
- 拖拽：能拖动排序；在搜索/非自定义排序下拖拽被禁用；批量选择模式下拖拽句柄换成复选框。
- 选中态：批量选择时卡片边框变主色。
- **高亮态**（若有 `accent`）：已应用的卡片是主色边框 + 选中底色；网关 failover 的 P0 卡片是成功色边框 + 渐变底。两者同时成立时 P0 胜出。
- 若该 CLI 有「设为默认」：点击后卡片标记与磁盘上的运行时指针**同时**更新（只查一处会漏掉脱节）。
- 禁用开关（若有）：能翻转状态，卡片随之变灰。

## 何时更新本文件

- 新增第四种样式时：先确认它确实是**语义差异**而非现有样式的参数变体；是的话在根 `AGENTS.md` 的 Index 与 SOP §4.2.1 同步登记。
- 某条经验上升为跨模块通用规则时，同步补到根 `AGENTS.md`。

# ZCode 前端模块说明

## 一句话职责

- `zcode/` 页面负责 **ZCode** 的 provider 列表、模型编辑、官方账号切换与 OAuth 登录、通用配置、根目录与全局提示词、会话浏览的可视化。

## Source of Truth

- provider 的主数据在后端 `v2/provider_config.json`（按 `providerId` 归属本模块）；前端只在表单里编辑自己那一份，**不持有整份文档**。
- `settings_config` 是 provider 行的字符串化 JSON，由 `parseZcodeProviderSettings` 解析。**解析失败返回 `null` 而不是抛错**——单行损坏退化成空卡片，不拖垮整页。改动这条要慎重：抛错会让一个坏 provider 白屏整个页面。
- 官方账号列表由后端计算 `is_applied`（以 `credentials.json` 的 live 身份为准），前端**不做二次推断**。
- 通用配置是 `v2/provider_config.json` 的共享部分（见下），不是 `cli/config.json`。

## 与相邻 tab 的边界

- **`cli/config.json` 不在这里编辑**。MCP server、hooks、plugin、权限开关各有自己的 tab；在这里再编辑一次就是两个地方写同一个文件。
- 官方账号卡片**不是 provider**：ZCode 把官方登录放在 `credentials.json`，与 provider 注册表相互独立、独立切换。但它作为列表里的**平级卡片**参与排序和拖拽，所以它自己带 `marginBottom`（见 Gotchas）。
- **说明文字是「标题的副标题」，紧贴标题下方**（标题行 → 说明 → 账号行），不是卡片底部的脚注：空态时底部说明会被整块空插画推得离标题一屏远（2026-10-08 用户圈出 Kimi 的同一写法后，两张同构卡片一起改）。Kimi 的 `KimiOfficialAccountCard` 与此**必须保持一致**——它是照这张卡抄的，改一处就要改两处。
- 共用组件优先用 `features/coding/shared/` 的：`CodingPageHeader`、`ProviderListSection`、`ModelListSection`、`SessionManagerPanel`、`useRootDirectoryConfig`、`providerList`（搜索/排序/批量）、`favoriteProviders`、`providerConnectivity/batchTest`。**不要在 zcode 里重写这些**。

## 核心设计决策（Why）

- **通用配置 = `provider_config.json` 的共享部分**，即 provider 表单不拥有的那部分。`providerConfigRules` / `modelConfigRules` 是按 provider 写的，所以共享 blob 承载的是外围结构——作用于整个注册表的默认值与设置。上游 schema 是 `.strict()`，**写未知键会被拒绝**。
- **`ZCODE_MODALITY_FIELDS` 把「多选」投影回「独立布尔」**：ZCode 在 `properties.inputFormat` 里为每种模态各声明一个布尔（`supportsText` / `supportsImage` / …），而表单展示的是多选。`storageKey` 是两边映射的锚点，改模态清单必须同时改这里和后端类型。
- **收藏（favorites）按 npm SDK family 分组**（`ZCODE_FAVORITE_NPM_BY_API_TYPE`），让 ZCode 的 provider 能和别的 CLI 的等价 provider 并排展示。未知格式回退到 openai 系——这是「未指定的 OpenAI 形状端点」最可能的事实。
- **导入来源统一收敛**：CC Switch（凭据在 `env`）与 All API Hub（在 `providerConfig.options`）各自抽成 `ZcodeImportedProvider`，**由一个 builder 产出 ZCode 形状**。新增来源时加一个 extractor，不要复制 builder。
- **连通性弹窗的「移除选中」走 `persistProviderModels`**（issue #451，2026-11-01 补）：共享弹窗只有在页面传了 `removableModelIds` 时才渲染移除按钮，本页传 `connectivityInfo.modelIds`（`buildZcodeProviderConnectivityInfo` 的 modelIds 就是 provider 目录里的 `modelId`）。`handleRemoveConnectivityModels` 删完行后必须保证还有一行 `isDefault`——否则 provider 变成不可应用状态；沿用 `handleDeleteModel` 的同一条回落规则。**不要复用 `handleBatchDeleteModels`**：它自带 `Modal.confirm`，会与弹窗的 `Popconfirm` 重复确认，且只认卡片勾选状态。

## 页面结构（与其他 tab 同构）

页头 = 大标题 + 文档链接 + 预览配置链接 + 配置路径行（自定义目录 / 打开目录 / 刷新）+ 右侧更多选项；
下面是分区卡片。**不要**把分区做成裸堆叠或给内容加内层 padding——会与其他 tab 的留白不一致。

## 模型表单：严格对齐官方「可编辑字段白名单」

**ZCode 的模型配置分「可编辑叶子」与「系统字段」两类**，判据在上游 `packages/provider/src/config/manual-model-config.ts`：

> 手动模式只冻结产品明确开放的叶子；**新增系统字段默认不属于个人手动配置**。

**可编辑白名单**（`manualModelConfigSchema` 的 `pick`，就这 7 个）：

| 字段 | 表单控件 |
|---|---|
| `contextWindow` | 上下文窗口 |
| `supportsJsonSchemaOutput` / `supportsNativeWebSearch` / `supportsMidConversationSystem` | 模型能力（**只有这三个**） |
| `inputFormat.supportsImage` / `supportsVideo` / `supportsPdf` | 输入类型（**只有这三个**；`supportsText` 是 smart-only） |
| `optionSpecs.reasoningLevel` | 推理等级 + 推理参数映射 |
| `optionSpecs.maxOutputTokens.max` | 最大输出 Token |

**系统字段**（官方 UI 不渲染控件、保存时原样带过）：

- `supportsToolCall`
- `requiresMfjsToolSchema`
- `inputFormat.supportsText`
- `outputFormat`
- `optionSpecs.maxOutputTokens.map`

**规则**：

1. **不要给系统字段加输入控件。** 加了会造成「smart 模式能填、manual 模式不能填」——同一个字段两种行为，而且写出的键官方编辑器永远不会产生。
2. **保存时原样保留**（`pickZcodeSystemProperties`）。ZCode 自己写的行带这些键，重新保存**不能静默删掉**。
3. **「不存在」与「false」语义不同**：键缺失 = 让 ZCode 目录决定；显式 `false` = 覆盖。所以保留函数只在键存在时复制，不补默认值。
4. 预设（`buildZcodeModelRowFromPreset`）**不写系统字段**，即使预设目录声称 `tool_call: true`——官方也是由 CLI 而非预设决定这个值。

### `enabled` 也在列表行上，不在编辑弹窗里

「这个模型启用还是禁用」同样**不在编辑弹窗**（官方 `ProviderModelMetadataDialog` 只有 basic / tokens / modalities / capabilities / reasoning 五组）。官方的开关在**模型列表行右侧**（`ProviderFormControls.tsx` 的 `Switch`，`onEnabledChange`）。

所以本模块：编辑弹窗**没有** enabled 控件；开关在模型行上（`ModelListSection.onToggleModelDisabled` → `handleToggleModelDisabled`）。写入时**只在禁用时写 `false`**，重新启用是**删掉这个键**而不是写 `true`——「键缺失 = 启用」是 ZCode 的语义。

> 弹窗保存时 `enabled` 仍要**从原行带过**（`initialValues?.enabled === false ? false : undefined`），否则编辑一个已禁用的模型会把它悄悄启用。

## 字段说明的问号（`FieldHelp`）

上下文窗口 / 最大输出 / 输入类型 / 模型能力 / 推理等级 / 推理参数映射六个标题后有问号，hover 显示说明。组件在 `components/common/FieldHelp/`（**通用**，其他 CLI 可复用）；文案在 `zcode.model.help.*`，**逐字取自官方 `settings.modelProvider.help.*`**——改了就对不上官方文档。

文案是**一个纯字符串**，不是 JSX：`\n\n` 分段、`- ` 起头成列表、`**粗体**` 与 `` `代码` `` 内联。解析在 `parseHelpText.ts`（纯函数，有测试）。**直接把字符串丢给 `Tooltip title` 会渲染成一整行**——`\n` 和 `**` 都不会被解释。

## 供应商卡片：OpenCode 式，且没有头部「应用」按钮

卡片用 `shared/providerCardVariants/OpenCodeStyleCard`（选型理由见根 `AGENTS.md` Hard Rule 14 与 SOP §4.2.1），本模块只负责把 ZCode 的存储形状映射上去。

**ZCode 没有单一 active provider**：`providerOrder` 里可以有任意多个 provider 同时可用，「用哪个」由注册表的 `defaultModelSelection` 表达。所以：

- 头部**不放「应用」按钮**——那会暗示其他渠道被关掉了。
- 「用哪个」表达在**模型行**的「设为默认」上（`handleSetPrimaryModel`）。
- 启用/禁用是 `Switch`（**列表卡头部那颗 inline 的**——OpenCode 式卡片没有「更多」菜单，动作全在头部行上；卡片视图里它改在格子的动作槽里，见下），与「哪个是默认」是两件独立的事。

**设默认要写两处，缺一不可**：

| 位置 | 含义 |
|---|---|
| `settings.models[].isDefault` | provider 目录内的偏好（卡片「当前默认」标记读它） |
| 注册表 `defaultModelSelection` | **运行时实际使用**的指针 |

只写前者 → 卡片显示「当前默认」但 ZCode 从不使用那个模型（见 SOP 教训 #55 / 模式十五）。判断「谁是默认」时以 `defaultModelSelection` 为准。

**卡片布局（2026-10-13）**：`ZcodeProviderCard` 收 `viewMode`，`'card'` 分支渲染共享 `ProviderCompactCard`，侧栏**内容**交给它的 `detail`（抽屉本身是共享卡片的，本模块不再 import `Drawer` / `ProviderDetailPanel`）；`ZcodePage` 用 `useProviderListViewPrefs('zcode')` 读写偏好，列表容器换 `ProviderCardGrid`，`SortableContext` 用 `rectSortingStrategy` + `restrictToWindowEdges`。**格子只陈述默认模型**（`默认模型: <displayName>`，取自 `models[].isDefault`）；「连接信息」（Base URL / API 格式 / API Key）与模型目录进侧栏，目录是**常开**形态（`appearance="detail"`）——列表卡片那份照旧折叠。格子那一侧没有 provider 级「应用」可放（ZCode 永远不会被应用，「设为默认」在模型行上），所以第二行只承载事实，`actionPlacement="subtitle"`；**但那一行的动作槽不是空的**：它放这颗 provider 唯一的 per-provider 控件——**启用开关**（`cellControl` + 共享的 `ProviderEnabledSwitch`，与列表卡片头部那颗是同一个组件、同一套 tooltip），也就是别的 CLI 放 应用 / 网关代理 的位置（2026-10-15 用户指定：「针对 zcode 这种不需要应用的渠道，把它在列表模式下的开关按钮放出来，放到现在应用的地方」）；传了 `cellControl`，「更多」菜单里的启用项随之让位（一个控件不在同一张卡片上说两遍）。

**官方账号卡片是列表成员，网格里就是一个普通格子**（2026-10-14 定稿，此前错了两版）：它不是 provider，但它和 provider 卡片共享同一套排序，所以在两种布局里它都**按槽位**（`officialAccountIndex`）参与排序。**形状归宿主布局**：列表里它走 `CardShell`（和 provider 卡片同一张描边卡，块自己画 🔗 标题行）；网格里走 `ProviderCompactCard`——**和邻居同宽同高**，标题行用同一对 `OfficialAccountHeadingIcon` / `OfficialAccountCount`（🔗 与 `(n)` 两种布局同源），第二行陈述**当前登录**（`common.officialAccount.currentLogin`，值取该账号在自己那行的 label；取值顺序＝**应用的账号 → 否则磁盘上那份登录（`isVirtual`，直读凭据文件的那条）→ 否则 `common.officialAccount.currentLoginNone`（未登录）**——**任何状态都要有输出**：这一行的位置无论说不说都被共享卡片占着，留空读起来是坏卡片；2026-10-14 用户报「zcode 也改成始终展示当前正在登录的账号」，与同日 Codex / Kimi 的定稿同一条规则）。**账号管理器（账号行 + 各自的动作）不在格子里，在这张格子自己的详情抽屉里**，且抽屉里那份**常开、无 caret**——与 Codex / Kimi 卡片模式下的同一个块完全同形；抽屉本身（宽度 `min(60vw, 760px)`、面板）由共享卡片给，本模块只提供内容。**不要**让它跨整行、也不要让它裸着摆在格子里：前者是一张全宽的描边卡（读起来像第四种卡片样式），后者是一块既不是格子也不是卡片的内容——用户 2026-10-13 贴图报的「卡片模式 zcode 的官方账号怎么还是列表样式」是第一种，第二次又贴了一次是第二种。排序载荷仍只含 provider id：卡片 id 只是渲染顺序里的占位。回归：`pnpm run test:zcode-card-view`（「它是一个格子宽」+「它穿的是共享格子外壳」+「格子陈述当前登录」+「管理器在它自己的侧栏里且常开」+「它参与网格拖拽」+「拖它不会打开侧栏」）。

**ZCode 的卡片上没有「应用」，也没有「网关代理」——这两个按钮在 ZCode 上不成立，不是漏了**（2026-10-14 用户问「zcode 的卡片上应用和网关代理的按钮没了」）：① 网关**无法接管 ZCode**（后端 `cli_proxy::is_supported_cli` 是 Claude / Claude Desktop / Codex / Grok / Kimi / Gemini，不含 zcode；`ZcodeProviderFormModal` 里也因此隐藏了 billing / 自定义头 / 模型改写三组 gateway-only 字段）；② ZCode 没有「单一已应用 provider」（见「供应商卡片」一节），「应用」会编造一个不存在的状态。**卡片级动作用户可见的只有模型行上的「设为默认」与「更多」菜单**。判据：**「这个按钮改动的东西在 ZCode 里存不存在？」**——答不出就先别加，并回答「为什么不加」（13.1 模式五十九）。

**通用配置没有 UI 入口**：那个编辑器写的内容从未被合并进 `provider_config.json`，且上游 schema 是 `.strict()` 的（只认 `providerConfigRules` / `modelConfigRules` / `providerOrder` / `defaultModelSelection`）。`saveZcodeCommonConfig` 命令保留——根目录弹窗仍通过它读写同一条记录的 `rootDir` 字段。

## 易错点与历史坑（Gotchas）

- **每张卡片自己留底部间距**（`marginBottom: 12`），不要靠父容器的 `gap`。官方账号卡片与 provider 卡片是**两个组件**，只给其中一个加间距会把零间距挪到另一对卡片之间——两处都要加。
- **「保存当前登录」按钮挂在那一行上，不在卡片顶部**。只有 `isVirtual` 的条目（未保存的 live 登录）可保存，所以按钮渲染在该行的操作区里：它作用的条目就是携带它的条目。后端在 live 登录没有快照时列出虚拟条目、保存后就不再列出，因此「条目存在」本身就是全部条件——保存后仍留着按钮会诱导用户重复保存，而重复保存什么也不做。
- **预览配置有 4 个文件**，不是 1 个。少列一个会让用户以为某个文件不存在。
- **登录弹窗里不要用 `okButtonProps={{ loading: true }}`**：Ant Design 的 `loading` 会给按钮加 `pointer-events: none`，导致取消按钮点不动。要表达「进行中」用弹窗正文里的 `Spin`。
- **官方账号行的按钮用无边框样式**（`type="text"` + `style={{ fontSize: 12 }}`），与模型列表右侧一致；带边框的按钮在这一排里视觉过重。
- **排序项 `sortableId` 必须传给 `useSortable`**，且 `useSortable` 每次渲染都要执行——没有 id 时传占位符并置 `disabled`，不要条件调用 hook。

## 跨模块依赖

- 后端 `zcode::*` 命令（`web/services/zcodeApi.ts` 一一对应）。
- `shared/sessionManager`：会话列表与详情页（`v2/sessions/<workspaceId>/<taskId>.json`）。
- `shared/providerConnectivity/batchTest`：批量连通性测试。
- `shared/ccSwitch` / `ImportProviderModal` / `AllApiHubIcon`：三个导入入口。
- 事件：`config-changed` + WSL 同步请求；托盘项由后端 `tray_support.rs` 提供。

## 最小验证

- 打开本页：能列出 provider 与官方账号；本机无 ZCode 安装时显示空态而非报错。
- 新建一个 provider 后，`v2/provider_config.json` 里其他 provider 与未知键逐字未变。
- 官方账号：未保存的 live 登录显示为「当前登录」条目并带保存按钮；保存后该条目消失、按钮一并消失，且恰好一个条目带「默认」标签。
- 切换账号后「默认」标签跟着移动，且不会出现两个「默认」。
- 模型表单里勾选多种模态 → 保存 → 重新打开，勾选状态与 `inputFormat` 的独立布尔一致。
- **系统字段保留**：手工在 `provider_config.json` 给某个模型加 `supportsToolCall: true` 与 `optionSpecs.maxOutputTokens.map`，从本应用打开该模型再保存 → 这两个键仍在，且表单里没有它们的控件（`zcodeModelFields.test.ts` 有对应断言）。
- 点模型行的「设为默认」后：卡片该行显示「当前默认」，且 `v2/provider_config.json` 的 `defaultModelSelection` 指向 `{providerId, modelId}`——两处都要核对，只看卡片会漏掉脱节。
- 卡片头部的启用/禁用开关能翻转 `isDisabled`，卡片随之变灰/恢复（卡片视图下同一颗开关在格子的动作槽里）。
- 卡片布局：`pnpm run test:zcode-card-view`（49 项真浏览器断言：切换生效并落库、格子只陈述默认模型且不重复地址、**格子在自己的动作槽里陈述启用开关**（真悬停读 tooltip 文案、真点一下禁用该 provider 并核对回写的 `{providerId, isDisabled}`、卡片随即陈述「已禁用」、回来的路仍在同一行、点它不会打开侧栏、**「更多」菜单不再重复这一条**）、官方账号是一个普通格子且保留槽位、**它在格子里只陈述当前登录（且闲置时有「未登录」这一态，不留空行）**、账号管理器在它自己的侧栏里（常开无 caret）、**拖它不会打开侧栏**、点主体开侧栏且侧栏含连接信息/模型目录（常开）/备注、网格内真拖拽后回写、切回列表恢复折叠、暗色主题）。
- 网关 tab 明细页筛选 `zcode` 能改变结果（下拉选中即生效）。

## 何时更新本文件

- 改动卡片布局/间距约定、通用配置的语义、导入来源适配、或官方账号的交互位置时，同一任务内更新本文件。
- 若某条经验上升为跨 CLI 通用规则（如「卡片间距各自负责」），同步补到根 `AGENTS.md`。

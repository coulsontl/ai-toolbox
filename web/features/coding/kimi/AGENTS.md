# Kimi Code CLI 前端模块说明

## 一句话职责

- `web/features/coding/kimi/` 负责 Kimi Code CLI 的页面展示、Provider 配置、通用配置、官方账号、Prompt 提示词及会话交互。

## 页面组织与设计原则

- 遵循根目录 `DESIGN.md` 设计规范。
- 页面与 Codex/Grok 结构保持一致，复用 `SectionSidebarLayout`、`RootDirectoryModal`、`GlobalPromptSettings`、`SessionManagerPanel` 和共享 Gateway 入口。
- **页面外壳与卡片全部走共享组件**（2026-10-08 迁移）：头部 `CodingPageHeader`、供应商区 `ProviderListSection`、卡片 `CodexStyleCard`（Kimi 有模型目录 + 单一 active provider，按选型表属 Codex 式）。`KimiProviderCard.tsx` 是**薄映射层**，只把 `settings_config` 解析成 `ProviderCardVariantProps`；布局改动一律改在 `shared/providerCardVariants/`（Hard Rule 14，由 `pnpm run test:provider-card-layout` 守护）。Kimi 原 `KimiProviderCard.module.less` 已删除——卡片外框与拖拽把手样式归 `CardShell`。
- 迁移时两处**位置变化**（不是功能丢失，改前改后都只有一处入口）：连通性测试从「更多」菜单移到卡片第二行（`inlineActions`）+ 模型区工具栏（`onTestModels`）；「官方账号登录」按钮与账号列表从列表工具栏 / 页面底部合并进 `KimiOfficialAccountCard`（见下条）。
- **官方账号是一张列表成员卡片，不是工具按钮、也不是页底区块**（2026-10-08 修正）。`KimiOfficialAccountCard` 与供应商卡片**平级**：同款外壳、同一个 `DndContext` / `SortableContext`、自带拖拽把手、自带 `marginBottom`，位置存 `KimiCommonConfig.officialAccountIndex`（按「上面有几张供应商卡」计，不用列表下标；写库走 `save_kimi_official_account_index`）。`「登录」`在卡片右上角，「切换 / 删除」在账号行右侧。
  - **卡片同时传给 `children`（槽位内）与 `alwaysVisible`**，两者渲染分支互斥：列表非空时走 `children` 里的槽位，列表为空/搜索无结果时走 `alwaysVisible`——**否则空列表下没有任何登录入口**。拖拽索引一律取 `sortableItemIds`（合并了哨兵 id），不要用 `providers.findIndex`，那会漏掉卡片自身占的一格。
  - `sortableId={providerDragDisabled ? undefined : KIMI_OFFICIAL_ACCOUNT_CARD_ID}`：与供应商卡片一致，搜索 / 非默认排序下没有把手（那里拖拽本就被禁用）。
  - **说明文字是「标题的副标题」，紧贴标题下方**（标题行 → 说明 → 账号行），不是卡片底部的脚注：空态时底部说明会被整块空插画推得离标题一屏远（2026-10-08 用户圈出）。ZCode 的同构卡片同步改了，**两张卡片的这段位置必须一致**。`pnpm run test:official-account-card` 里有一条「卡片里没有任何一句话被渲染两次」的断言，专门拦「搬到新位置却没删旧位置」。
  - 参照实现是 ZCode 的 `ZcodeOfficialAccountCard`（SOP #42 / #61），**不是** Codex 卡片里的账号折叠区。Codex 能把账号放进官方订阅卡片内部，是因为它的官方 provider 行由 CLI 的 `auth.json` 投影而来、恒存在；Kimi 的官方行是**点「登录」时才懒创建的**（`handleStartOfficialAccountAuth`），入口放进那张卡片就成了先有鸡还是先有蛋。
- 模型行接入共享 `ModelListSection` 后**多出两个行级按钮**：复制、设为主模型（对应新增的 `handleCopyModel` / `handleSetPrimaryModel`）。行身份是 `model.key`（catalog 别名），不是上游 model id——见 `modelRows` 与 `rowKeyByDisplay` 的注释。
- Gateway 现在是 direct → single → failover 三态。single 入口在已应用 provider 卡片的“网关代理”按钮；single/failover 接管期间锁定其他 provider 的直连应用入口，failover 卡片显示 P0/P1 优先级。
- i18n 键集中在顶层 `kimi.*`（如 `kimi.provider.*`、`kimi.providerForm.*`）；全局提示词区块使用 `kimi.prompt.*`（`GlobalPromptSettings` 的 `translationKeyPrefix` 必须传 `kimi.prompt`，传不存在的键会直接把键名字面量渲染成展开栏标题）。
- 侧边栏与其他 agent 页一致：`sidebarTitle` 只传 `t('kimi.title')` 纯字符串，section 图标经 `getIcon`（providers=Database / prompt=FileText / plugins=Appstore / sessions=Message），`onSectionSelect` 负责展开对应 Collapse（prompt/session 用 nonce 触发共享组件重挂载展开）。
- 「查看额度」外链（kimi.com/code/console）只属于官方订阅语义：只在官方账号行展示，不要加回供应商列表头部等全局位置。
- 供应商展开栏只放供应商列表本身。插件（`<root>/plugins/`）是全局资源、不随供应商切换，因此 `KimiPluginsPanel` 是独立 Collapse section（`kimi-plugins`），对齐 Grok 的 `grok-plugins` 布局。Kimi CLI 没有非交互式插件管理命令（仅 TUI 内 `/plugins`），面板保持只读列表 + 打开目录，不要仿 Pi 扩展页实现安装/卸载。

## 关键设计决策与 Why

- 批量删除资格以 `utils/providerDeletion.ts` 的 `canDeleteKimiProvider` 为前端共用判断：临时本地项、已应用项和仍绑定官方账号的项不能进入选择集合。后端仍是最终守卫；其拒绝或中途失败必须刷新列表、保留未处理选择。旅程测试中的内存后端也必须拒绝删除 applied provider，不能用允许删除的假实现掩盖真实拒绝路径。

- **保存决策由页面态 `editingProvider` 决定，不是表单回传值**：`KimiProviderFormModal` 提交的 `KimiProviderFormData` 不含 `id`；曾因 `handleSaveProvider` 依赖 `values.id` 导致编辑永远走新建分支（点确认就多一条未应用记录，原 applied 状态丢失，代理按钮随之消失）。决策逻辑已抽到 `utils/providerSaveFlow.ts` 的 `buildKimiProviderSavePlan`（`adopt_local` / `update` / `create`）和 `shouldReengageKimiGatewayOnSave`，修改保存链路先看这两个纯函数和 `web/test/features/coding/kimi/kimiProviderJourney.test.ts` 的旅程用例。
- **`gatewayCliStatus` 必须随 `loadConfig` 刷新**：代理按钮可见性依赖 `can_takeover`，而它随 provider 行（可代理候选）变化。曾因只在 `GatewayFailoverButton` 挂载时加载一次，provider 修复后前端仍缓存旧的 error/can_takeover=false，按钮一直不出现。`loadConfig` 里统一 `getProxyGatewayCliStatus('kimi')` 刷新（独立 catch，不阻塞主列表）。
- **网关接管期间凡重写 live `config.toml` 的保存都必须先恢复直连再重接管**：后端 `ensure_kimi_gateway_direct` 会拒绝接管期间的直连保存；前端统一走 `saveProviderWithGatewayReengage`（restore → save → re-engage）。覆盖范围由 `shouldReengageKimiGatewayOnSave` 判定：已应用 provider 编辑与 `__local__` 收编（两者 `isApplied=true`，都会重投影 live 文件）需要 re-engage；未应用记录的 create/update 只动 DB 行，不需要。
- **额度查询（Billing / Quota）**：落地为外链按钮（跳转 `https://www.kimi.com/code/console`，Kimi Code 控制台）。原因：Kimi CLI 本地无任何会员额度数据源（`kimi -p "/usage"` 在非交互模式不拦截 `/usage` 并当成普通 prompt 发给模型；0.39.1 源码确认 TUI `/usage` 仅输出本地会话 token 统计、无会员/计费计划字段、无 billing/quota API）。
- **计费倍率与自定义请求头**：provider 表单集成共享 `BillingConfigCollapse` + `CustomHeadersCollapse`，状态存 `provider.meta`（`costMultiplier` / `pricingModelSource` / `customHeaders`），official 类别强制禁用；merge 语义用共享 `mergeBillingConfigIntoMeta` / `mergeCustomHeadersIntoMeta`，用户清空后 meta 字段会被显式删除，不会残留旧值。
- **Provider 表单布局对齐 Codex/Grok**：`layout="horizontal"` + `labelCol`（zh 4 / en 6）/ `wrapperCol` 20，整宽区块用 `wrapperCol={span:24}`；高级 JSON、备注必须用共享 `ProviderConfigCollapse` / `ProviderNotesCollapse`（备注折叠区即 `notes` 表单控件本体），不要回退成 AntD Collapse 或裸 TextArea。`KimiProviderFormModal.module.less` 只保留真实被引用的类，此前整份文件是从 Grok 复制的死代码导致布局全部失效。
- **弹窗 onOk 错误必须兼容字符串 reject**：Tauri invoke 失败 reject 的是字符串而不是 `Error` 实例，`catch` 里只判 `instanceof Error` 会把后端校验错误完全吞掉（表现为「点确认没反馈」）。统一写法：`message.error(error instanceof Error ? error.message : String(error))`，且弹窗 onOk 必须 try/catch。
- **通用配置弹窗只提交 TOML payload，不含 rootDir**：rootDir 的唯一编辑入口是 `RootDirectoryModal`（经 `useRootDirectoryConfig`，显式处理 clear 语义）。后端 `save_kimi_common_config` 在不传 `rootDir`/`clearRootDir` 时保留旧值；旧表单固定 `clearRootDir: false` 又允许清空输入，清空后静默保留旧根目录（P1 已修）。契约由 `utils/commonConfigForm.ts` 的 `buildKimiCommonConfigSubmitValues` 锁定并有回归测试。
- **Provider 卡片禁用开关对齐 Grok**：更多菜单首项放 `common.enable` + Switch（非 `__local__` 才显示），已应用 provider 禁用前用 `common.disableAppliedConfigWarning` 前端拦截（后端 `toggle_kimi_provider_disabled` 也会拒绝），禁用卡片整体 0.6 透明度；成功提示复用 `kimi.providerDisabled` / `kimi.providerEnabled`。
- **Device auth 终态提示必须一次性**：事件监听与 5s 轮询 fallback 会重复观测同一终态，且失败后轮询持续到弹窗卸载。`utils/deviceAuthStatus.ts` 的 `createKimiDeviceAuthStatusClassifier` 保证每个 auth session 终态只产生一次 success/error 提示，`cancelled` 是静默终态；组件收到终态后立即 `clearInterval` 并解绑事件。
- **`__local__` 临时 provider 的保存路径**：DB 无 provider 时 `list_kimi_providers` 会把当前 config.toml 投影成 id=`__local__` 的临时 provider（「自动加载的本地配置」）。编辑它保存时必须走 `save_kimi_local_config`（落库为真实记录并标记 applied），不能调 `update_kimi_provider`（后端拒绝并返回 “Local Kimi provider must be saved before it can be updated”）；前端常量 `KIMI_LOCAL_PROVIDER_ID` 在 `web/types/kimi.ts`。
- **Provider 默认模板必须过后端校验**：非 official provider 只要带 `defaultModelKey` 就必须同时提供 `modelCatalog.models`（`validate_provider_settings`），且 model 的 `provider` 字段要与 `providerConfigs` 的 key 对应（参照 `project_writes_providers_models_and_default_model` 测试夹具）；改默认模板时先对照该校验。
- **官方账号「应用/删除」入口在 `KimiPage` 供应商区账号行内**：Device auth 登录成功只入库账号（`is_applied=false`），真正激活必须走 `applyKimiOfficialAccount`（后端 `apply_kimi_official_account`，接管期间会被 `ensure_kimi_gateway_direct` 拒绝并把错误透出）；已应用账号显示 `kimi.account.applied` Tag、删除按钮禁用（后端也拒绝删除已应用账号），未应用账号显示「应用」按钮。账号行操作后统一 `loadConfig(true)` + `refreshTrayMenu()`。曾缺失该入口导致登录后无法激活账号（P1 已修）。
- **Device auth 状态文案必须本地化**：后端状态串（`waiting`/`completed`/`failed`/`expired`/`cancelled`）经 `DEVICE_AUTH_STATUS_TEXT_KEYS` 映射到 `kimi.provider.deviceAuthStatusValue.*`，未知状态回退原串；轮询间隔尊重后端 `pollIntervalSeconds`（下限 3s），`onCompleted` 经 ref 稳定化，避免父组件重渲染重挂事件监听/重置轮询。
- **`defaultModelKey` Select 不带 `allowClear`**：清空后 `buildKimiSettingsConfig` 会回填第一个模型 key（后端投影本身就回退首条），清空没有运行时意义，保留清除入口只会造成「有意清空被静默还原」的误导。

# 官方账号区块（officialAccounts）

## 一句话职责

- 把「官方登录的账号列表长什么样」固定成**一个**共享区块：标题 → 说明 → 账号行（切换 / 保存 / 刷新 / 详情 / 删除）。
- 各 CLI 只做一件事：把自家账号记录映射成 `OfficialAccountRowView`，再挑一个宿主。

## Source of Truth

- 区块本体是 `OfficialAccountsSection.tsx`；`types.ts` 是行视图与 props 的定义。
- **宿主由调用方决定**，组件不关心外层是什么：
  - `embedded` —— 区块在供应商卡片**内部**（Codex、Kimi）。需要上方分隔线、行缩进、空态用一行文字（卡片中间插一张空插画会把下面的模型区推出首屏）。
  - `standalone` —— 区块**自持一张卡片**（ZCode：官方登录不在供应商表里，没有官方行可挂）。
- 这个 `variant` 是**两个原实现真实存在的差异**，不是可调参数。新增第三种宿主形态之前不要加值。

## 核心设计决策（Why）

- **标题行的 🔗 图标由组件自己画**，不由调用方传。三份原实现里有两份（Kimi、ZCode）把它画在标题行、一份（Codex）画在登录按钮上；抽组件时照了 Codex，于是图标从标题行消失，用户对照 ZCode 截图发现。**任何两份原实现共有的装饰都进组件**，调用方不再重复传（见 13.1 模式七十四）。

- **登录入口是插槽（`loginAction`），不是布尔开关。** Codex/Kimi 是一个按钮，ZCode 是「选 OAuth provider」的下拉菜单。做成 `showLogin` + `loginProviders` 之类的 prop，等于把下拉的形态知识塞进共享组件——它就不该知道一个 CLI 有几个官方 provider。
- **行级动作按语义固定五个**（`onApply` / `onSaveLocal` / `onRefresh` / `onViewDetails` / `onDelete`），**谁传哪个就渲染哪个**。CLI 没有的动作不传即可，组件不做「这个 CLI 有没有这个能力」的判断。
- **虚拟行（`isVirtual`）只给一个动作：保存。** 它镜像的是磁盘上已经生效的那份登录——切到它自己是无意义的，删掉它也不会让 CLI 登出。保存之后它变成普通行，切换/删除随之出现。
- **已应用的行不画切换按钮**（画「默认」徽章）。曾把「切换」画成 disabled：一个灰掉的「切换」挨着「默认」，说的是同一件事两遍。
- **`pending` 带的是 `{accountId, action}`，不是一个 id。** 同一时刻可能有两行在不同动作上（Codex 刷新一个、保存另一个），只给 id 会让转圈落在错误的按钮上。
- **文案一律由调用方传已翻译串。** 额度、套餐、显示名都是 CLI 自己的措辞；区块只负责排版。

## 关键约束（改这里之前必读）

- **行标记靠结构，不靠 class 名。** 夹具通过「带内联 `border-bottom` 的行」定位账号行——`ant-space` 之类会同时命中卡片头部的动作簇（那里也有 `CheckOutlined`），把行数算错。
- **几何必须量，不能读代码。** 账号行右边缘 == 卡片内容右边缘 == 头部动作右边缘。把区块塞进头部两栏的**左栏**里，右边缘会静默少掉一组动作链接的宽度（Codex 上实测 160px，用户报成「右侧空了一大块」）。
- **`actionsDisabled` 是「登录进行中，谁都别动」。** ZCode/Kimi 的登录会重写同一份凭据文件，中途点行级动作就是和它抢。
- **新增 prop 必须在至少一个调用方真的用上**，否则按「零消费方 prop 不许存在」删掉（同 `providerCardVariants` 的纪律）。
- **迁移一个 CLI 时不要顺手改措辞。** 三个原实现的差异大多是无意造成的，但「切换 / 保存当前登录 / 默认」这套词是用户确认过的口径：账号之间叫**切换**，渠道之间才叫**应用**。

## 使用方

| CLI | 宿主 | 传的行级动作 |
|---|---|---|
| codex | `embedded`（官方供应商卡片内） | 刷新 / 详情 / 保存 / 切换 / 删除（全量） |
| kimi | `embedded`（官方供应商卡片内） | 保存 / 切换 / 删除 |
| zcode | `standalone`（自己的卡片） | 保存 / 切换 / 删除 |
| antigravity / geminicli / grok | **仍手写**（`PENDING_MIGRATION`，3 个） | — |

> 这三个是 2026-10-09 修守卫时才发现的：原守卫匹配的是 `anticon-swap` / `anticon-check`（渲染后的 DOM class），源码里永远不命中，所以它们从未被扫出（见 13.1 模式七十三）。迁移时按同一份映射规则接进来即可。

守卫：`pnpm run test:official-accounts-shared`（源码棘轮，禁止再手写行级按钮）、
`pnpm run test:codex-official-accounts` 与 `pnpm run test:official-account-card`（真浏览器几何断言）。

## 何时更新本文件

- 新增第四种宿主形态：先确认它确实是**宿主差异**而非 `variant` 的参数变体。
- 某条经验上升为跨模块通用规则时，同步补到根 `AGENTS.md` 与 `docs/new-cli-onboarding-sop.md`。

# WorkBuddy 前端模块说明

## 一句话职责

- `web/features/coding/workbuddy/` 是 WorkBuddy 桌面版页面：配置根目录、`models.json` 渠道列表、四个按账号隔离的身份文件。

## Source of Truth

- 渠道数据来自后端 `read_workbuddy_runtime_config`（现读 `models.json` 聚合出的视图），页面**不保留第二份副本**。新增/编辑/删除后一律用命令返回值整体替换 state。
- 身份文件（`IDENTITY.md` / `USER.md` / `SOUL.md` / `BOOTSTRAP.md`）是**直接编辑的活文档**，不是可切换的提示词预案。页面没有「应用」动作，只有「保存」。
- **身份文件面板复用 `GlobalPromptSettings` 那套卡片**（`GlobalPromptConfigCard` + `GlobalPromptConfigModal` + `Collapse`），因为对读者来说它们就是提示词；差别在于**集合是固定的**：只能编辑，不能新增 / 删除 / 拖拽 / 应用。这不是偏好而是结构：产品只读这四个文件名（第五行是产品永远打不开的名字），删除等于把助手运行的文档清空（保存空内容才是可逆的「清空」），而文件本身就是记录、顺序无处可写。编辑态名称字段 `disabled`（名字就是磁盘上的文件名）。
- 渠道列表、模型弹窗与 CodeBuddy 页面**共用** `web/features/coding/shared/modelsJson/`：两个产品同引擎、同文件，这一层不允许在任一页面里分叉。

## 核心设计决策（Why）

- **渠道卡片：列表布局 `OpenCodeStyleCard`，卡片布局 `ProviderCompactCard`**：`models.json` 是扁平的模型列表，没有 provider 概念，多个渠道同时可用、没有「当前生效」的那一个，所以卡片没有主操作、没有「设为默认」。两种布局是密度差异：格子只写「端点 + 模型数」，模型管理在侧栏里常开。
- **渠道级「编辑」写的是连接，不是名字**：文件里没有渠道记录，所以 ✎ 打开供应商表单的编辑态，保存时把新的端点 / key / vendor 逐条写回该渠道的每一个模型（每条的 id / name / 能力开关原样保留）。
- **预览读原文**：`read_workbuddy_models_file` 直接 `read_to_string`。用运行时视图重新 `JSON.stringify` 会丢掉没有 URL 的条目、重排字段、丢掉产品自有键——预览的意义就是「磁盘上到底是什么」。
- **身份文件面板按账号可用性分态**：未登录时 `unavailableReason` 有值，面板只显示原因，不显示不存在的路径。

## 关键流程

```mermaid
sequenceDiagram
  participant Page as WorkbuddyPage
  participant Cmd as workbuddy commands
  participant File as ~/.workbuddy/models.json

  Page->>Cmd: read_workbuddy_runtime_config
  Cmd->>File: ModelsDocument::load（聚合为渠道视图）
  Cmd-->>Page: FamilyRuntimeConfig
  Page->>Cmd: save_workbuddy_model(input)
  Cmd->>File: 深合并 + 原子写
  Cmd-->>Page: 新的 FamilyRuntimeConfig
```

## 易错点与历史坑（Gotchas）

- **页面外壳必须走 `SectionSidebarLayout` + `.pageContent { display: block }`**（2026-10-30 初版漏了，全仓其余 16 个 coding 页面都有）。当时还漏了 `routeConfig` 的 `contentPadding: 'compact'`，内容区退回 24px。三处都不报错。改页面外壳后跑 `pnpm run test:page-rhythm`。
- **`SidebarSettingsModal` 必须接线**（页头 `onMoreOptions` + 弹窗里的「显示侧栏」开关）。全仓 14 个 coding 页面都有；漏了它，用户把侧栏隐藏后**没有任何办法再打开**——这是个不可逆的界面状态。
- **「预览配置」必须读文件原文**（`readWorkbuddyModelsFile`），不要用运行时视图重新 `JSON.stringify`：视图会丢掉无 URL 条目、重排字段、丢产品自有键。回归：`workbuddy::commands::tests::a_saved_model_survives_the_runtime_view_and_the_raw_file_read`。
- **新增区块必须同时进三处**：DOM 上的 `data-workbuddy-sidebar-section="true"` + `data-sidebar-title`、页面顶部的 `sidebarSections` 数组、`SIDEBAR_ICON_BY_SECTION_ID` 映射。漏了第一处，侧栏点了不会滚动；漏了后两处，侧栏里没有这一项。
- **i18n 动态前缀已登记进 `scripts/i18n-keys.mjs` 的 `useRootDirectoryConfig` 保护表**。漏登记时 `i18n:check` 是**通过**的（它按保护表决定扫哪些展开式），而 `i18n:prune` 会把 `workbuddy.rootPathSource.modal.*` 整块删掉——第一次补登记就一次性暴露了 18 个缺失 key。
- `translationKeyPrefix: 'workbuddy'` 与 `sidebarHiddenByPage.workbuddy` 必须同名；从别的 CLI 复制页面时读侧最容易残留来源模块的 key。
- **页面夹具的桩必须复刻后端的真实形状与字段拼写**：`get_workbuddy_identity_state` 的真实形状是 `{ identityDir, unavailableReason, files: [{ fileName, content, exists }] }`——**camelCase**（Rust 侧 `#[serde(rename_all = "camelCase")]`）。写成 snake_case 时列表仍然渲染，但每行的名字和标题都是空的，布局看起来依然「合理」，这是最容易漏掉的一种桩错误。返回裸 `null` 则会让面板解析失败并在页面右上角弹出「操作失败」。夹具现在用**已登录**形状，否则四条固定条目根本不会被渲染，身份区块的断言等于没写。同理 `WorkbuddyProviderView.id` 必须给：卡片用 `data-provider-id` 做定位锚点，缺了它卡片仍然渲染，但「定位」静默失效。两条都是 2026-10-30 做真机视觉核对时踩到的。
- **`App` provider 不能省**：页面用 `App.useApp()` 取 `message`，夹具只包 `ConfigProvider` 时会抛 `t.error is not a function`。

## 最小验证

- `pnpm run test:workbuddy-page`：真实页面 + 桩 Tauri 桥的浏览器夹具（亮/暗或 `WORKBUDDY_PAGE_THEME=dark`）。它守的是组件级测试看不见的东西：卡片越界、页面横向滚动、卡片套卡片、侧栏三区块标记齐全、**「添加供应商」打开的是供应商表单且不含任何能力开关**（SOP §4.2.1）、**渠道铅笔打开的是编辑态供应商表单且端点已回填**、**卡片视图是紧凑格 + 模型管理在侧栏**，以及**身份文件是四条固定条目：无新增、无拖拽把手、菜单里只有「编辑」、编辑态名称不可改**。
- `pnpm run test:page-rhythm`（含 WorkBuddy 两项：页头→首区块 16px、区块→区块 16px）。
- `pnpm run test:provider-card-layout` / `test:css-module-classes` / `test:form-modal-layout`。
- 手工：在产品里加一条模型 → 本页能看到 → 在本页编辑另一条 → 第一条不丢；点「预览配置」看到的是 `models.json` 原文（含注释/未知键）。

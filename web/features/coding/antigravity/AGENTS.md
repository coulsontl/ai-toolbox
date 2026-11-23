# Antigravity Frontend Module Guide

## Source of Truth

- Antigravity 前端页面入口为 `AntigravityPage.tsx`，路由对应 `/coding/antigravity`。
- 页面只面向 Google 官方 OAuth 账号管理，不向用户暴露自定义 API Key、Base URL 或供应商列表；根目录设置、全局提示词和会话管理仍保留。
- 类型定义位于 `web/types/antigravity.ts`。
- API 调用集中在 `web/services/antigravityApi.ts` 与 `web/services/antigravityPromptApi.ts`。

## Key Decisions

1. 视觉风格严格遵守根目录 `DESIGN.md`，使用 Ant Design 原生 Modal chrome 与 CSS Modules。
2. 内部官方 provider 仅作为账号归属和运行时状态载体，前端不提供新增、编辑、删除、排序或 API Key 设置入口。
3. 官方账号 OAuth、本地登录保存、配额、详情和应用通过 `AntigravityProviderCard.tsx` 复用共享 `OfficialAccountsSection`（`variant="standalone"`，不套多余 `<Card>`）展示。
4. 账号列表不再常驻无信息的“默认配置”占位行；仅当 OS 凭据库 `gemini:antigravity` 存在未纳管的真实登录时，后端才返回一条虚拟本地行（`isVirtual = true`），优先展示从 `id_token` 解出的邮箱，缺失时回退本地化文案 `t('antigravity.provider.officialAccountLocal')`（「本地官方登录」），并提供「保存当前登录」动作将其转为正式托管账号；无本地登录且无托管账号时展示标准空态。
5. 全局提示词默认写入 `agy` 专属全局规则文件 `~/.gemini/config/GEMINI.md`，并允许在新建/编辑弹窗中按方案选择 4 个官方叠加规则目标之一（`~/.gemini/config/GEMINI.md`、`~/.gemini/config/AGENTS.md`、`~/.gemini/GEMINI.md`、`~/.gemini/AGENTS.md`，均基于 `$HOME`、不跟随 `settings.json` 或自定义根目录）；同时通过共享 `GlobalPromptSettings` 的可选插槽（`extraHint` / `footer` / `targetFileOptions`，不影响其他 CLI）在顶部写明叠加加载规则，并在底部列出本地已存在的其他规则文件（含 `~/.gemini/config/rules/*.md`）供只读预览和定位目录。本页不展示 `oauth_creds.json`，登录凭证实际存放在操作系统凭据库 `gemini:antigravity`。
6. `disableConfig` 会向 `disable_antigravity_prompt_config` 传入 `configId`（不是 `id`），这是后端强校验的参数名。
7. “Antigravity CLI 登录”按钮在 OAuth 期间进入 loading 并禁用，期间再点不再发请求（后端对同一时刻的并发 OAuth 也是拒绝的）；回调页只提示“已收到授权，请返回应用完成登录”，最终成功/失败以后端命令返回为准，避免在换取令牌前误报成功。
8. 页面头部必须与 Claude Code / Codex 保持同构：第一行是标题 + 内联的“官方文档”“预览配置”链接与右侧“更多选项”，第二行是配置路径 + “自定义配置目录 / 打开文件夹 / 刷新配置”。不要再给页面 body 套 `maxWidth` / `padding` 容器，`MainLayout` 的 `.contentArea` 已提供内边距。

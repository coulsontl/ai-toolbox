# Shared ToolIcon 前端模块说明

## 一句话职责

- `shared/toolIcon/` 提供 Skills 与 MCP 共用的工具品牌图标解析组件 `ToolIcon`，以及共享的 `GitHubSourceIcon` 来源图标。

## Source of Truth

- 工具 key 与展示名由各调用方传入；组件本身不持有工具清单或后端状态。
- 品牌图标来源优先级固定为：**tab 优先资产 → LobeHub → `web/assets/agent-icons/` 参考 mark → 自定义 `iconUrl` → 两字母兜底**。
- `web/assets/agent-icons/` 是共享静态资产目录，`NOTICE.md` 记录来源与许可；不要复制图标副本到业务模块。

## 核心设计决策（Why）

- 从 `skills/components/ToolIcon.tsx` 迁到 `shared/toolIcon/`，让 Skills 和 MCP 共用同一套解析逻辑，避免两套品牌映射漂移。
- Skills 页面仍从共享路径导入，不保留旧组件文件。
- 图标暗色适配全部封装在 `ToolIcon.module.less`（`.rawIconFixedColor` / `.iconHost` / `.rawIcon`）；业务组件不新增 filter、不复制资产。

## 易错点与历史坑（Gotchas）

- 不要改回 `<img src>` 资产 URL 渲染 tab 类 mark；运行环境实测不渲染，必须用内联 SVG 组件或 `?raw` 内联。
- `currentColor` 类 mark 不要加 `.rawIconFixedColor`，否则暗色下双重反转不可见。
- 内联 SVG 里的渐变/滤镜 id 必须按实例唯一（`React.useId()`），不能写死：`url(#id)` 解析到文档里第一个同 id 元素，KeepAlive 用 `display:none` 缓存的隐藏页面会成为这个"第一个"，导致所有可见图标画不出渐变（OMP 图标曾因此在 MCP 设置弹窗里消失）。
- LobeHub mono 图标与 lucide 图标必须包在 `.iconHost` 宿主里，否则按钮/菜单不继承页面文字色，暗色下变黑。
- 新增内置工具时按 AGENTS.md 中 ToolIcon 解析顺序补映射；不要复制 `web/assets/agent-icons/` 资产到 feature 目录。

## 跨模块依赖

- 被 `web/features/coding/skills/**` 与 `web/features/coding/mcp/**` 依赖。
- 消费 `@lobehub/icons`、`lucide-react`、`web/assets/agent-icons/` 与 `web/stores/themeStore`。

## 最小验证

- 至少验证 Skills 与 MCP 卡片/详情面板/添加工具菜单中品牌图标在亮暗主题下均可见。
- 改动解析顺序或暗色适配后，必须补跑 `pnpm exec tsc --noEmit` 与 `pnpm test`。
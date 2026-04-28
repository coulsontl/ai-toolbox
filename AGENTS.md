# AGENTS.md - AI Toolbox Development Guide

This document provides essential information for AI coding agents working on this project.

## Communication Language

与用户的所有对话必须使用**中文**，包括问题澄清、方案说明、进度反馈和结果总结。代码注释和 commit message 仍使用英文。

## Module Documentation

主要功能模块可以在自身目录下放置 `AGENTS.md`，用于记录该模块的 `Source of Truth`、设计决策、关键流程、易错点和最小验证。修改模块代码前，如果目标目录或更近的父目录存在模块级 `AGENTS.md`，必须先阅读。

### Hard Rules

1. 修改任何模块目录下的文件前，若该模块目录或更近的父目录存在 `AGENTS.md`，必须先读，再设计或实现。
2. 当一次改动引入新的非显而易见设计决策、修复未来高概率复发的问题、或重构跨文件关键流程时，应在同一任务内同步更新对应模块的 `AGENTS.md`。
3. 根 `AGENTS.md` 只保留全局规则。模块专属内容应下沉到对应模块目录下的 `AGENTS.md`。
4. 模块级 `AGENTS.md` 只写高价值信息：`Source of Truth`、`Why`、`关键流程`、`Gotchas`、`最小验证`。不要写文件清单、类型定义、API 清单、数据库字段表、或可被代码直接证明的事实。
5. 当根与模块 `AGENTS.md` 冲突时，以作用域更近、语义更具体的文档为准。
6. 新增模块级 `AGENTS.md` 时，需在本节索引中同一任务内补上入口。
7. 模块级 `AGENTS.md` 建设处于迁移期时，根 `AGENTS.md` 中已有的高价值信息只能补充重组，不能因为下沉而删弱；确认模块文档已完整覆盖前，不要移除根中的对应规则。
8. 后续开发中，只要遇到值得长期沉淀的知识、用户反复强调的约束、或已经复发/高风险复发的坑点，必须在同一任务内及时写回对应模块的 `AGENTS.md`，不能等“下次再整理”。
9. 如果某条经验已经上升为跨模块通用规则、全局架构约束、或多个模块都会反复踩到的铁律，应同步写回根 `AGENTS.md`，而不是只埋在单个模块文档里。
10. 数据库相关经验默认按全局规则处理，优先写回根 `AGENTS.md`；只有某条数据库约束严格局限于单一模块时，才在对应模块 `AGENTS.md` 补充模块特有语义。
11. 仓库内用于给 agent 阅读的模块文档 `AGENTS.md` 不是应用运行时资源；本地开发 watcher 和类似热重载链路应尽量忽略它们，避免把文档编辑误当成代码改动。
12. 不要把上一条误用到产品运行时 prompt 文件上。当前仓库里的 OpenCode / Codex 运行时 prompt 文件名就是 `AGENTS.md`，Claude Code 运行时 prompt 文件名是 `CLAUDE.md`；它们属于真实业务数据，备份、恢复、WSL/SSH 同步和页面交互都依赖这些文件，不能按“仅 agent 文档”排除。

### Template

- `docs/module-agents-template.md`：模块级 `AGENTS.md` 模板。新增模块文档时先复制，再删除不适用小节。

### Index

| 模块目录 | 说明 |
|---------|------|
| `tauri/src/coding/` | Coding 域共享规则：runtime location、事件驱动托盘、WSL Direct、跨工具 CLI/路径语义 |
| `tauri/src/coding/claude_code/` | Claude Code 后端配置、prompt、plugin、MCP 与 WSL 同步约束 |
| `tauri/src/coding/codex/` | Codex 后端配置、auth/config.toml、prompt、plugin 与 WSL 同步约束 |
| `tauri/src/coding/mcp/` | MCP Server 后端存储、工具配置同步、导入导出与 WSL 联动 |
| `tauri/src/coding/open_code/` | OpenCode 后端配置文件、provider、prompt、tray 与 WSL 同步约束 |
| `tauri/src/coding/open_claw/` | OpenClaw 后端配置文件与 WSL 同步约束 |
| `tauri/src/coding/oh_my_openagent/` | Oh My OpenAgent 后端配置、临时本地态、应用链路与 OpenCode WSL 联动 |
| `tauri/src/coding/oh_my_opencode_slim/` | Oh My OpenCode Slim 后端配置、临时本地态、应用链路与 OpenCode WSL 联动 |
| `tauri/src/coding/session_manager/` | 会话浏览、详情、重命名、导入导出与运行时路径解析 |
| `tauri/src/coding/skills/` | Skills 中央仓库、导入发现、同步、托盘和 WSL/SSH 相关链路 |
| `tauri/src/coding/tools/` | Skills/MCP 共用工具适配、检测与自定义工具存储 |
| `tauri/src/coding/wsl/` | WSL 同步配置、自动同步监听、WSL Direct 状态消费 |
| `tauri/src/coding/ssh/` | SSH 连接、文件映射、手动同步、MCP/Skills 远端同步 |
| `tauri/resources/` | 编译期嵌入的模型默认数据资源：`preset_models.json`/`models.dev.json` 的来源、顺序语义与缓存边界 |
| `web/features/coding/claudecode/` | Claude Code 前端页面、根目录配置、provider 与 prompt 交互 |
| `web/features/coding/codex/` | Codex 前端页面、根目录配置、provider 与 prompt 交互 |
| `web/features/coding/image/` | Image 前端页面、工作台、渠道管理、历史与结果交互 |
| `web/features/coding/mcp/` | MCP 前端页面、服务器管理、导入流程与工具同步交互 |
| `web/features/coding/opencode/` | OpenCode 前端页面、配置路径、provider、prompt 与模型刷新交互 |
| `web/features/coding/openclaw/` | OpenClaw 前端页面、配置路径、provider 与配置文件交互 |
| `web/features/coding/shared/` | coding 共享前端语义：根目录弹窗、全局 prompt、favorite provider、会话面板、连通性测试 |
| `web/features/coding/skills/` | Skills 前端页面、中央仓库视角、分组展示与批量同步交互 |
| `web/features/settings/` | WSL/SSH 设置页、同步入口、moduleStatuses 消费和 UI 边界 |
| `tauri/src/settings/backup/` | 备份恢复、WebDAV、自动备份与恢复后续链路 |
| `tauri/src/coding/image/` | Image 后端渠道配置、任务、资产落盘、图片 API 调用与备份联动 |

后续新增模块级 `AGENTS.md` 时，继续在此表追加，不在根文档其他位置零散登记。

## Project Overview

AI Toolbox is a cross-platform desktop application built with:
- **Frontend**: React 19 + TypeScript 5 + Ant Design 5 + Vite 7
- **Backend**: Tauri 2.x + Rust
- **Database**: SurrealDB 2.x (embedded SurrealKV)
- **Package Manager**: pnpm

## Directory Structure

```
ai-toolbox/
├── web/                    # Frontend source code
│   ├── app/                # App entry, routes, providers
│   ├── components/         # Shared components
│   ├── features/           # Feature modules
│   │   ├── coding/         # Coding tools (claudecode, codex, opencode, skills)
│   │   ├── daily/          # Daily notes
│   │   └── settings/       # App settings
│   ├── stores/             # Zustand state stores
│   ├── i18n/               # i18next localization
│   ├── constants/          # Module configurations
│   ├── hooks/              # Global hooks
│   ├── services/           # API services
│   └── types/              # Global type definitions
├── tauri/                  # Rust backend
│   ├── src/                # Rust source
│   │   ├── coding/         # Coding modules (claude_code, codex, open_code, skills)
│   │   └── settings/       # Settings modules
│   └── Cargo.toml          # Rust dependencies
└── package.json            # Frontend dependencies
```

## Build & Development Commands

### Frontend (pnpm)

```bash
# Install dependencies
pnpm install

# Start development server (frontend only)
pnpm dev

# Build frontend for production
pnpm build

# Type check
pnpm tsc --noEmit
```

### Tauri (Full App)

```bash
# Start full app in development mode
pnpm tauri dev

# Build production app
pnpm tauri build
```

### Rust (Backend)

```bash
# Check Rust code
cd tauri && cargo check

# Build Rust in release mode
cd tauri && cargo build --release

# Format Rust code
cd tauri && cargo fmt

# Lint Rust code
cd tauri && cargo clippy
```

### Testing

```bash
# Frontend tests
pnpm test

# Run single test file
node --test web/test/path/to/test.test.ts

# Rust tests
cd tauri && cargo test

# Run single Rust test
cd tauri && cargo test test_name
```

### Test Execution Rules

- 对跨模块、跨层、会影响“保存/应用/同步/恢复/导入导出/配置落盘”的**大功能迭代**，不要只跑针对性测试；在交付前必须补跑当前仓库可用的全量测试集合。
- 当前仓库前端测试统一通过 `pnpm test` 执行；该脚本会发现并运行 `web/test/**` 下的 `.test.ts` / `.spec.ts` 文件。
- 前端测试文件必须放在 `web/test/` 下，并镜像对应功能目录结构；不要把 `.test.ts` 文件继续与实现文件并排放在 `web/features/**`、`web/components/**` 等源码目录里。
  - 例如：`web/features/coding/opencode/components/foo.ts` 对应测试应放在 `web/test/features/coding/opencode/components/foo.test.ts`
- Rust 测试保持分层约定：
  - 依赖模块私有实现的单元测试，继续放在 `tauri/src/**` 的 `#[cfg(test)]` / `#[test]` 中
  - 面向公开行为或黑盒回归的集成测试，放在 `tauri/tests/**`，并按功能镜像组织目录与 fixtures
- 当前仓库全量校验的最小集合是：
  - `pnpm test`
  - `cd tauri && cargo test`
  - `pnpm exec tsc --noEmit`
- 发版相关 GitHub Actions 也必须在打包 job 开始前先跑同一套全量测试闸门；只要测试未全部通过，就不能进入构建产物的打包与发布步骤。
- 如果本轮改动直接影响前端构建入口、路由、公共组件、i18n 资源、Vite/TS 配置，且成本可接受，还应额外跑：
  - `pnpm build`
- 如果全量测试中存在**与本轮改动无关的既有失败**，不要跳过不报；需要在结果总结里明确写出：
  - 跑了哪些命令
  - 哪些通过
  - 哪些失败
  - 失败是否为本轮新增
  - 失败定位到的文件或测试名
- 如果本轮只改一个非常局部的点，但用户明确要求“全量测试”或“完整验证”，仍然按上面的全量集合执行，而不是自行降级为 smoke test。
- 新增或修复高价值回归时，应优先补**最贴近用户路径**的自动化用例；不要只补实现细节测试而漏掉“表单提交 -> 持久化 -> 再读取”这类关键往返语义。

## Code Style Guidelines

### TypeScript/React

#### Imports Order
1. React and React-related imports
2. Third-party libraries (antd, react-router-dom, etc.)
3. Internal aliases (`@/...`)
4. Relative imports
5. Style imports (`.less`, `.css`)

```typescript
// Example
import React from 'react';
import { Layout, Tabs } from 'antd';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { MODULES } from '@/constants';
import { useAppStore } from '@/stores';
import styles from './styles.module.less';
```

#### Naming Conventions
- **Components**: PascalCase (`MainLayout.tsx`)
- **Hooks**: camelCase with `use` prefix (`useAppStore.ts`)
- **Stores**: camelCase with `Store` suffix (`appStore.ts`)
- **Services**: camelCase with `Service` suffix (`noteService.ts`)
- **Types/Interfaces**: PascalCase (`interface AppState {}`)
- **Constants**: SCREAMING_SNAKE_CASE for values, PascalCase for configs

#### Component Structure
```typescript
import React from 'react';

interface Props {
  // Props interface
}

const ComponentName: React.FC<Props> = ({ prop1, prop2 }) => {
  // Hooks first
  const { t } = useTranslation();
  const navigate = useNavigate();
  
  // State and derived values
  const [state, setState] = React.useState();
  
  // Effects
  React.useEffect(() => {}, []);
  
  // Handlers
  const handleClick = () => {};
  
  // Render
  return <div />;
};

export default ComponentName;
```

#### Zustand Stores

Use Zustand without persistence middleware - all data must go through the service layer to SurrealDB:

```typescript
interface SettingsState {
  settings: AppSettings | null;
  initSettings: () => Promise<void>;
  updateSettings: (settings: AppSettings) => Promise<void>;
}

export const useSettingsStore = create<SettingsState>()((set) => ({
  settings: null,

  initSettings: async () => {
    const settings = await getSettings(); // Call service API
    set({ settings });
  },

  updateSettings: async (newSettings) => {
    await saveSettings(newSettings); // Save to database
    set({ settings: newSettings });
  },
}));
```

**Never use persist middleware** - all persistent data must be stored in SurrealDB via Tauri commands.

#### Path Aliases
Use `@/` for imports from `web/` directory:
```typescript
import { useAppStore } from '@/stores';
import { MODULES } from '@/constants';
```

### Rust

#### Naming Conventions
- **Functions/Methods**: snake_case
- **Structs/Enums**: PascalCase
- **Constants**: SCREAMING_SNAKE_CASE
- **Modules**: snake_case

#### Tauri Commands
```rust
#[tauri::command]
fn command_name(param: &str) -> Result<ReturnType, String> {
    // Implementation
    Ok(result)
}
```

#### Error Handling
- Use `thiserror` for custom errors
- Return `Result<T, String>` for Tauri commands
- Use `?` operator for error propagation

#### HTTP / TLS Compatibility

- 后端发起 HTTPS 请求时，默认优先复用仓库里的全局 `http_client`，不要在业务模块里随手 new 一个默认 `reqwest::Client`。
- 当前仓库的全局 `http_client` 需要显式使用 `rustls` TLS 后端；不要回退到 Windows Schannel / native-tls 默认行为。
- 原因不是“风格统一”，而是实战兼容性：某些机器上 `reqwest + Schannel` 会在 TLS 建连阶段直接报 `SEC_E_NO_CREDENTIALS` / “安全包中没有可用的凭证”，表现为浏览器、Node、curl 正常，但 Rust `send()` 在真正发出业务请求前就失败。
- 如果新增特殊 HTTP client（自定义 timeout、禁压缩、直连、HTTP/1 only 等），也必须在同一 builder 链路里显式保留 `use_rustls_tls()`，不要只复制 timeout / proxy / compression 配置而漏掉 TLS 后端。

#### Async Runtime Safety

- **Never call `tauri::async_runtime::block_on()` or `tokio::runtime::Handle::block_on()` inside any async call chain.**
  This includes Tauri commands, startup tasks spawned by `tauri::async_runtime::spawn`, event listeners, background sync tasks, and any helper that may be reached from those paths.
- If a sync Rust helper needs database-backed or other async-derived data, do not hide the async query inside the sync helper. Provide a parallel `*_async` function and make async call sites use it directly.
- When reviewing a sync helper that internally queries SurrealDB with `block_on`, treat it as **sync-boundary only**. Before reusing it, first verify whether the caller may run under Tokio/Tauri async runtime.
- For path/config resolution utilities, prefer this rule:
  sync callers use `*_sync` or pure sync helpers; async callers use `*_async`; do not mix them.
- If you fix a high-value engineering pitfall that is likely to recur, you should also update this `AGENTS.md` in the same task so the rule becomes part of repo workflow guidance.
- For cross-platform restore or backup flows that normalize on-disk directory names, do not only fix extracted file paths. Any persisted metadata still used by later sync, tray, WSL, or SSH flows, such as `skill.name` and `central_path`, must be normalized in the same task or a startup migration before those flows run.
- When a settings/status API returns the primary config plus derived diagnostic metadata, do not let best-effort metadata resolution break the primary read path. For example, WSL/SSH `module_statuses`, tray visibility hints, or runtime-location summaries must degrade gracefully with logs instead of making the whole settings payload fail.

#### Optional Field And Compatibility Rules

- For optional config fields, do not use simple truthy checks like `if (values.someField) { ... }` when saving edited data. This collapses "user intentionally cleared the field" into "field was absent" and leaves stale values behind.
- When a form edits persisted data that already allows partial optional structures, the form layer must not be stricter than the storage model unless a migration is handled in the same task.
- Before adding paired validation such as "both filled or both empty", first verify backend types, existing imported data, restore flows, and edit flows. If stored data already permits one-sided values, blocking save in the form is a regression.
- When removing or clearing provider-derived env/config keys, explicitly clean known keys before merging newly selected values. Do not assume omission in the new payload will delete old values automatically.
- For tools whose runtime config file mixes AI Toolbox-managed fields with runtime-owned fields, rewrites must follow the same semantics as Claude Code settings writes: remove the previous AI Toolbox-managed fields first, then write the new managed fields. Do not preserve previous managed fields by default.
- For third-party config nodes that AI Toolbox does not fully own, preserve unknown fields and legal schema shapes across read -> write round trips. If a field allows multiple valid forms such as `string | tuple`, `bool | object`, or `string | string[]`, do not normalize it to a narrower shape unless the task explicitly migrates user data and tests that migration.
- In Claude Code `settings.json`, explicitly preserve runtime-owned top-level fields such as `enabledPlugins`, `extraKnownMarketplaces`, and `hooks` during provider/common-config rewrites. These fields are not the same thing as AI Toolbox-managed provider/common config.
- For Claude plugin runtime JSON files such as `known_marketplaces.json`, never deserialize into a partial Rust struct and then serialize the whole file back. If AI Toolbox only owns one field like `autoUpdateEnabled`, patch that field in the raw JSON object and preserve all CLI-owned fields verbatim.
- In Codex `config.toml`, explicitly preserve runtime-owned sections such as `mcp_servers`, `features`, and `plugins` during provider/common-config rewrites. These sections are not the same thing as AI Toolbox-managed provider/common config.
- In Codex `auth.json`, do not full-overwrite runtime-owned OAuth fields when switching providers. AI Toolbox may manage `OPENAI_API_KEY`, but fields such as `auth_mode`, `tokens`, and `last_refresh` belong to Codex runtime login state and must be preserved unless the task explicitly migrates or clears them.

### Modal & Dialog Design Guidelines

**Reference implementations**: `ConnectivityTestModal` and `FetchModelsModal` are the gold-standard for modal styling. Always follow their patterns when creating new modals.

#### Modal Shell

Do NOT heavily override Ant Design modal chrome (`.ant-modal-content`, `.ant-modal-header`, `.ant-modal-footer`, `.ant-modal-close`). Keep modal wrapper styles minimal — only adjust body padding if needed:

```less
// ✅ Minimal modal override (like FetchModelsModal)
.modal {
  :global(.ant-modal-body) {
    padding: 20px 24px 22px;
  }
}

// ❌ Don't do this — heavy chrome overrides with gradients, custom backgrounds, etc.
.modal {
  :global(.ant-modal-content) { background: ...; border-radius: 20px; }
  :global(.ant-modal-header) { background: linear-gradient(...); }
  :global(.ant-modal-footer) { background: ...; border-top: ...; }
  :global(.ant-modal-close) { top: ...; border-radius: ...; transition: ...; }
}
```

#### Section Cards (Non-collapsible)

Use plain `<section>` or `<div>` with `.sectionCard` class. Style must match ConnectivityTestModal:

```less
.sectionCard {
  border: 1px solid var(--color-border);
  border-radius: 16px;
  background: var(--color-bg-elevated);
  padding: 18px;
  // NO box-shadow
}
```

#### Collapse Sections (Collapsible)

The global `.ant-collapse` in `App.css` already provides `background + box-shadow + border-radius`. When using Collapse inside modals, override the shadow to match sectionCard style:

```less
.sectionCollapse {
  border: 1px solid var(--color-border) !important;
  border-radius: 16px !important;
  background: var(--color-bg-elevated) !important;
  box-shadow: none !important;  // Remove global shadow

  :global(.ant-collapse-item) {
    border-bottom: none !important;
  }
  :global(.ant-collapse-header) {
    background: transparent !important;
  }
  :global(.ant-collapse-content) {
    border-top: 1px solid var(--color-border) !important;
    background: transparent !important;  // Override antd default colorBgContainer
  }
  :global(.ant-collapse-content-box) {
    padding: 18px !important;
    background: transparent !important;  // Override antd default colorBgContainer
  }
}
```

**Common pitfalls:**
- Don't set `background: transparent` on the outer Collapse — it removes the card appearance
- Don't add `border + background + box-shadow` on `.ant-collapse-item` inside — it creates a nested card effect with gaps that don't reach the modal edge
- Don't fight global styles with aggressive `!important` overrides on every element; only override what differs (shadow)
- **Must set `background: transparent !important` on both `.ant-collapse-content` and `.ant-collapse-content-box`** — antd defaults these to `colorBgContainer` (white), which overrides the parent's `bg-elevated` background. The global `App.css` also sets `.ant-collapse-header` background to `bg-container`. Without transparent overrides, the content area shows white instead of the card's elevated background.
- **Must add `bordered={false}` (or `ghost`) prop on `<Collapse>`** — without it, antd's CSS-in-JS injects default backgrounds (white header, white content) and border styles that override module-level `!important` overrides. Even though `.sectionCollapse` has `background: transparent !important` on sub-elements, antd's runtime styles can still win. Always pass `bordered={false}` to disable default chrome before applying custom sectionCollapse styles.
- **Wrap modal body in `<div className={styles.content}>` and add `className={styles.form}` to `<Form>`** — the `.content` wrapper provides flex layout for alert + form spacing; the `.form` class applies consistent form-item margins, label color, and input border-radius across all sections. Omitting these causes inconsistent spacing and unstyled inputs inside Collapse sections.

#### Horizontal Field Layout (Preferred)

For information density and compactness, prefer **left-right (horizontal) layout** for input fields and info display: label/title on the left, input/value on the right. Use CSS Grid for consistent alignment:

```less
// ✅ Preferred: Grid-based horizontal field layout (like ConnectivityTestModal)
.formFieldRow {
  display: grid;
  grid-template-columns: 108px minmax(0, 1fr);
  gap: 12px;
  align-items: center;
}

.fieldLabel {
  display: flex;
  align-items: center;
  min-height: 32px;
  color: var(--color-text-primary);
}

// Responsive: stack vertically on narrow screens
@media (max-width: 720px) {
  .formFieldRow {
    grid-template-columns: 1fr;
    gap: 8px;
  }
}
```

This applies to:
- Form inputs (label left, input right)
- Information display (title left, value right)
- Config fields in modal sections

Use vertical layout only when horizontal is impractical (very long labels, single-field quick inputs, or very narrow containers).

### Styling

- Use CSS Modules with Less (`.module.less`)
- Class naming: camelCase in Less files
- Use Ant Design's design tokens when possible

```less
.container {
  display: flex;

  &.active {
    background: rgba(24, 144, 255, 0.1);
  }
}
```

### Form & Modal Layout

**Modal forms should use horizontal (left-right) layout by default**, where labels are on the left and input fields are on the right. This provides better visual alignment and more efficient use of space.

#### Layout Guidelines

1. **Prefer Horizontal Layout**: Use Ant Design Form with `layout="horizontal"` for modal forms
2. **Label Placement**: Labels should be right-aligned and placed on the left side of inputs
3. **Consistent Label Width**: Use `labelCol` and `wrapperCol` to maintain consistent proportions

#### Implementation Pattern

```typescript
// ✅ Recommended: Horizontal layout for modal forms
<Form layout="horizontal" labelCol={{ span: 6 }} wrapperCol={{ span: 18 }}>
  <Form.Item label={t('name')} name="name">
    <Input />
  </Form.Item>
  <Form.Item label={t('description')} name="description">
    <Input.TextArea />
  </Form.Item>
</Form>

// ❌ Avoid: Vertical layout in modals (unless space is very limited)
<Form layout="vertical">
  <Form.Item label={t('name')} name="name">
    <Input />
  </Form.Item>
</Form>
```

#### When to Use Vertical Layout

Use vertical layout (`layout="vertical"`) only in these cases:
- Very narrow containers where horizontal layout would be cramped
- Forms with very long labels that don't fit well horizontally
- Single-field quick input forms

### Theme System (Dark Mode)

**IMPORTANT: The application supports full dark mode / light mode / system theme switching. ALL UI colors must use theme variables or Ant Design tokens - NEVER hardcode color values.**

#### Theme Architecture

The app uses a multi-layer theming system:

1. **Theme Store** (`web/stores/themeStore.ts`):
   - Manages theme mode: `'light'`, `'dark'`, or `'system'`
   - Automatically syncs with system theme when mode is `'system'`
   - Persists preference to database

2. **Theme Provider** (`web/app/providers.tsx`):
   - Applies Ant Design theme algorithm (`darkAlgorithm` or `defaultAlgorithm`)
   - Sets `data-theme` attribute on `document.documentElement`
   - Updates window background color for native titlebar

3. **CSS Variables** (`web/App.css`):
   - Defines theme-aware CSS variables
   - All custom variables automatically switch when `data-theme` attribute changes

#### Available CSS Variables

**Background Colors:**
- `--color-bg-base` - Base background color
- `--color-bg-container` - Container background
- `--color-bg-layout` - Layout background
- `--color-bg-elevated` - Elevated surface (dropdowns, modals)
- `--color-bg-hover` - Hover state background
- `--color-bg-selected` - Selected state background

**Text Colors:**
- `--color-text-primary` - Primary text (high emphasis)
- `--color-text-secondary` - Secondary text (medium emphasis)
- `--color-text-tertiary` - Tertiary text (low emphasis)

**Border Colors:**
- `--color-border` - Default border color
- `--color-border-secondary` - Secondary border (higher contrast)
- `--color-border-card` - Card border

**Other:**
- `--color-shadow` - Primary shadow
- `--color-shadow-secondary` - Secondary shadow
- `--color-scrollbar` - Scrollbar color

#### Usage Guidelines

**DO:**
```less
// ✅ Use CSS variables
.container {
  background: var(--color-bg-container);
  color: var(--color-text-primary);
  border: 1px solid var(--color-border);
}

// ✅ Use Ant Design tokens (via ConfigProvider)
.container {
  color: #1890ff; // OK for brand colors managed by Ant Design
}

// ✅ Dark mode specific overrides
.icon {
  opacity: 0.7;

  :global([data-theme="dark"]) & {
    filter: invert(1);
  }
}
```

**DON'T:**
```less
// ❌ Never hardcode colors
.container {
  background: #ffffff; // Wrong! Use var(--color-bg-container)
  color: rgba(0, 0, 0, 0.88); // Wrong! Use var(--color-text-primary)
}

// ❌ Don't use media queries for theme
@media (prefers-color-scheme: dark) { // Wrong! Use [data-theme="dark"]
  .container { ... }
}
```

#### Dark Mode Patterns

**Pattern 1: CSS Variables (Recommended)**
```less
.myComponent {
  background: var(--color-bg-container);
  color: var(--color-text-primary);
}
// Automatically adapts to theme changes
```

**Pattern 2: Attribute Selector Overrides**
```less
.myComponent {
  background-color: rgba(255, 255, 255, 0.2);

  :global([data-theme="dark"]) & {
    background-color: rgba(20, 20, 20, 0.2);
  }
}
```

**Pattern 3: Image/Icon Filters**
```less
.icon {
  // Default: black icon on light background

  :global([data-theme="dark"]) & {
    filter: invert(1); // Inverts to white icon
  }
}
```

#### Accessing Theme in TypeScript

```typescript
import { useThemeStore } from '@/stores/themeStore';

const MyComponent = () => {
  const { mode, resolvedTheme } = useThemeStore();
  // mode: 'light' | 'dark' | 'system'
  // resolvedTheme: 'light' | 'dark' (computed value)

  // Use resolvedTheme for conditional rendering
  const iconColor = resolvedTheme === 'dark' ? '#fff' : '#000';
};
```

#### Testing Theme Support

When implementing new components or features:

1. **Test both themes**: Switch between light and dark mode in Settings
2. **Test system theme**: Set to "System" and toggle OS theme
3. **Check all states**: Hover, active, disabled, selected
4. **Verify readability**: Ensure text contrast meets accessibility standards
5. **Review hardcoded colors**: Search for hex colors (`#`) in your styles

#### Common Mistakes to Avoid

1. **Hardcoding opacity values**: Use theme variables instead
   - ❌ `rgba(0, 0, 0, 0.88)` → ✅ `var(--color-text-primary)`

2. **Using media queries for theme**: Use `[data-theme]` attribute selector
   - ❌ `@media (prefers-color-scheme: dark)` → ✅ `[data-theme="dark"]`

3. **Inline styles with hardcoded colors**: Extract to CSS modules or use theme variables
   - ❌ `<div style={{ color: '#000' }}>` → ✅ Use CSS class with var()

4. **Forgetting images/icons**: Dark backgrounds require inverted icons
   - Add `filter: invert(1)` for dark mode when needed

### Internationalization

- All user-facing text must use i18next
- Translation keys in `web/i18n/locales/`
- Use nested keys: `modules.daily`, `settings.language`

```typescript
const { t } = useTranslation();
<span>{t('modules.daily')}</span>
```

## Feature Module Structure

Each feature in `web/features/` follows this pattern:

```
features/
└── feature-name/
    ├── components/     # Feature-specific components
    ├── hooks/          # Feature-specific hooks
    ├── services/       # Tauri command wrappers
    ├── stores/         # Feature state
    ├── types/          # Feature types
    ├── pages/          # Page components
    └── index.ts        # Public exports
```

## Key Configuration Files

| File | Purpose |
|------|---------|
| `tsconfig.json` | TypeScript config with path aliases |
| `vite.config.ts` | Vite build config, dev server on port 5173 |
| `tauri/tauri.conf.json` | Tauri app config |
| `tauri/Cargo.toml` | Rust dependencies |

## Important Notes

1. **Strict TypeScript**: `noUnusedLocals` and `noUnusedParameters` are enabled
2. **SurrealDB**: Uses embedded SurrealKV engine, data stored locally
3. **i18n**: Supports `zh-CN` and `en-US`
4. **Theme**: Full dark mode / light mode / system theme support implemented (see Theme System section in Code Style Guidelines)
5. **Dev Server**: Runs on `http://127.0.0.1:5173`

## Skills / WSL / SSH Quick Notes

迁移期说明：本节原有规则继续保留，不能弱化。实际修改代码时，还应同时阅读模块级文档：
- `tauri/src/coding/skills/AGENTS.md`
- `tauri/src/coding/wsl/AGENTS.md`
- `tauri/src/coding/ssh/AGENTS.md`
- `web/features/settings/AGENTS.md`

- Skills 的**唯一源目录**是中央仓库 `central_repo_path`。不要把 Claude/Codex/OpenCode/OpenClaw 当前运行时的 skills 目录当作同步源；这些目录只是目标目录或运行时消费目录。
- `skills_sync_to_tool` 的职责是：把中央仓库内容同步到工具运行时目录。这个运行时目录可能是普通本机路径，也可能因为模块配置目录位于 WSL 而解析成 `\\\\wsl.localhost\\...` UNC 路径。
- WSL `skills` 自动同步和 SSH `skills` 自动同步都不是复用文件映射。它们各自有独立链路，但**源端仍然是中央仓库**，不是工具当前目录。
- WSL 直连模块要特别区分“源目录”和“目标目录”：
  - 源目录仍是中央仓库。
  - 工具目标目录可能已经是 WSL 运行时目录。
  - UI 中为了可读性把路径显示成 WSL/UNC 形式，并不代表同步链路改成了从该显示路径取源。
- 处理 Skills 的 WSL 自动同步时，不要把“当前运行时路径不是 WSL UNC”误判成“没有 WSL 目标目录”。
  - 对 Claude/Codex/OpenCode/OpenClaw 这 4 个内置工具，如果当前运行时路径是本机 Windows 默认/自定义路径，WSL skills 目标仍应回退到各自默认 Linux 目录，如 `~/.claude/skills`、`~/.codex/skills`、`~/.config/opencode/skills`、`~/.openclaw/skills`。
  - 只有真正的 WSL Direct 场景，才应优先根据 UNC 运行时路径动态解析目标目录。
- 排查 “更新 Skill 后哪里没同步” 时，优先按这三个层次拆分：
  - 中央仓库内容是否已更新。
  - 本地工具运行时目录是否因为路径变化触发了重新同步。
  - `skills-changed` 后的 WSL/SSH 后续链路是否执行，以及它们各自写入的是哪个远端目标目录。

## 4 Tabs WSL Direct Notes

迁移期说明：本节原有规则继续保留，不能弱化。实际修改这 4 个 tab 或设置页联动时，还应同时阅读模块级文档：
- `tauri/src/coding/AGENTS.md`
- `tauri/src/coding/wsl/AGENTS.md`
- `tauri/src/coding/ssh/AGENTS.md`
- `web/features/coding/shared/AGENTS.md`
- `web/features/settings/AGENTS.md`

- 适用范围：OpenCode、Claude Code、Codex、OpenClaw 这 4 个配置页。
- 先区分两个概念：
  - `source` 表示当前配置路径来自哪里，取值是 `custom` / `env` / `shell` / `default`。
  - `is_wsl_direct` 表示当前**生效路径**是否是 `\\\\wsl.localhost\\...` 这类 WSL UNC 路径。
  - 这两个维度彼此独立。最常见的组合就是 `source=custom` 且 `is_wsl_direct=true`。
- 4 个 tab 的“自定义配置”并不完全同类：
  - OpenCode、OpenClaw 保存的是**配置文件路径**。
  - Claude Code、Codex 保存的是**配置根目录**，后续再在该目录下派生 `settings.json`、`config.toml`、`CLAUDE.md`、`AGENTS.md`、`skills` 等路径。
- 后端对这 4 个 tab 的 WSL 判定统一走 `runtime_location`：
  - 先按各模块自己的优先级解析“当前生效路径”。
  - 如果该路径能被解析为 WSL UNC 路径，就标记为 `WslDirect`，并产出 `distro`、`linux_path`、`linux_user_root` 等元数据。
  - 前端和 WSL/SSH 设置页消费的 `moduleStatuses` 就来自这一步，而不是直接看页面上的 `pathInfo.source`。
- 当前生效路径的优先级规则如下：
  - OpenCode：应用内 `config_path` > 环境变量 `OPENCODE_CONFIG` > shell 配置 > 默认配置文件路径。
  - Claude Code：应用内 `root_dir` > 环境变量 `CLAUDE_CONFIG_DIR` > shell 配置 > 默认根目录。
  - Codex：应用内 `root_dir` > 环境变量 `CODEX_HOME` > shell 配置 > 默认根目录。
  - OpenClaw：应用内 `config_path` > 默认配置文件路径。
- 一旦 4 个 tab 的生效路径是 WSL UNC，后续派生路径都会跟着切换到同一份 WSL 运行时位置：
  - OpenCode/OpenClaw 这类“文件路径模块”会基于该配置文件所在位置继续推导 prompt、plugins、skills 等目录。
  - Claude/Codex 这类“根目录模块”会在该根目录下继续推导配置文件、prompt、auth、skills 路径。
  - `get_tool_skills_path_async` 也会基于这个运行时位置，把 4 个内置工具的 skills 目标解析成对应的 WSL UNC 路径。
- 前端页面当前的展示逻辑也要单独理解：
  - 4 个 tab 顶部路径行显示的 tag 只反映 `source`，不会单独显示一个 “WSL” tag。
  - 所以“绿色 custom tag + 完整 `\\\\wsl.localhost\\...` 路径”是当前预期，不代表状态丢失。
  - Claude/Codex 的通用 `RootDirectoryModal`、OpenCode 的 `ConfigPathModal`、OpenClaw 的 `OpenClawConfigPathModal` 打开时，只会把 `source === custom` 的当前值回填到输入框。
- WSL/SSH 设置页对这份状态的消费也不同：
  - WSL Sync 设置页会读取 `moduleStatuses`，把 `is_wsl_direct` 的模块 tab 置灰并显示 tooltip，同时手动 WSL 同步时也会把这些模块加入 `skipModules`。
  - SSH Sync 设置页当前不会禁用这些模块；它只会用 `moduleStatuses` 把左侧“本地路径”改写成完整 UNC 显示，真正同步仍走后端动态解析。
- 和这 4 个 tab 联动时最容易误判的点：
  - 不要把 `source === custom` 当成 “一定是 WSL”。
  - 也不要把 `moduleStatuses.is_wsl_direct` 反推成 “一定来自应用内自定义路径”，因为它也可能来自 env 或 shell。
  - 排查问题时要分开看“页面展示的 source/path”“runtime_location 的 WSL 判定”“WSL/SSH 设置页消费到的 moduleStatuses”，这三层不是同一个状态对象。
- **CLI 调用规则必须单独遵守**：
- 对 OpenCode、Claude Code、Codex、OpenClaw 这 4 个 tab，只要后端需要调用对应工具 CLI，禁止直接假设 `Command::new("<tool>")` 总能工作。
  - 必须先通过对应的 `runtime_location::*_runtime_location_async` 解析当前运行时。
  - 如果运行时是本机路径，才直接调用本机 CLI。
  - 如果运行时是 `WslDirect`，必须改成 `wsl -d <distro> --exec ...` 执行，并把传给 CLI 的配置路径、数据路径、导入导出文件路径、工作目录等参数转换成 Linux 路径。
  - 纯文件读写可以继续直接访问 `\\\\wsl.localhost\\...` UNC 路径；但“文件 I/O 可用”不代表“CLI 也可以直接吃 UNC 路径”。
  - 新增 CLI 能力时，要同时检查这 4 个 tab 是否存在同类调用点，避免只在当前模块修补。
  - 对 Claude Code、Codex、OpenCode、OpenClaw 这类用户自行安装的 CLI，不要在 GUI 进程里直接依赖 `PATH` 做 `Command::new("<tool>")`。macOS 从 Dock/Finder/Spotlight 启动时常拿不到 shell PATH；新增 CLI 调用时，必须优先解析已知安装路径或显式配置路径，再回退到 PATH。

## Data Storage Architecture

**IMPORTANT**: All data storage and retrieval must go through the service layer API and interact directly with the backend database (SurrealDB). This is a local embedded database with very fast performance.

### DO NOT use localStorage

- **Never** use `localStorage` or `zustand/persist` for data that needs to be persisted
- **Never** sync data from localStorage to database - this pattern is not allowed
- All persistent data must be stored directly in SurrealDB via Tauri commands

### Correct Data Flow

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐     ┌──────────────┐
│  Component  │ ──► │  Service Layer   │ ──► │  Tauri Command  │ ──► │  SurrealDB   │
│  (React)    │ ◄── │  (web/services/) │ ◄── │  (Rust)         │ ◄── │  (Database)  │
└─────────────┘     └──────────────────┘     └─────────────────┘     └──────────────┘
```

### Service Layer Structure

All API services are located in `web/services/`:

```typescript
// web/services/settingsApi.ts
import { invoke } from '@tauri-apps/api/core';

export const getSettings = async (): Promise<AppSettings> => {
  return await invoke<AppSettings>('get_settings');
};

export const saveSettings = async (settings: AppSettings): Promise<void> => {
  await invoke('save_settings', { settings });
};
```

### Backend Command Pattern

All Tauri commands interacting with SurrealDB must follow the **Adapter Pattern** and use **Raw SQL** to ensure backward compatibility and avoid versioning issues.

#### 1. Database Naming Convention
- **Database Fields**: Must use `snake_case`.
- **Rust Structs**: Use `snake_case`.
- **Do NOT** use `#[serde(rename_all = "camelCase")]` for database records.

#### 2. Adapter Layer (Required)
Always implement an adapter layer to decouple Rust structs from database records. This handles missing fields and type mismatches robustly.

```rust
// adapter.rs
use serde_json::Value;
use super::types::AppSettings;

pub fn from_db_value(value: Value) -> AppSettings {
    AppSettings {
        // Robust extraction with defaults
        language: value.get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en-US")
            .to_string(),
        // ... other fields with default values
    }
}

pub fn to_db_value(settings: &AppSettings) -> Value {
    serde_json::to_value(settings).unwrap_or(json!({}))
}
```

#### 3. Persistence Pattern (Updates & ID Handling)
To avoid SurrealDB versioning conflicts (`Invalid revision` errors) and deserialization failures (`invalid type: map`):

1.  **Reads**: Handle the `Thing` ID type explicitly.
    *   **Best Practice**: Use **`type::string(id)`** in your query to convert the ID to a string before returning to Rust.
    *   **Why**: SurrealDB's default `id` is a `Thing` object (e.g., `{ tb: "table", id: "id" }`). Direct deserialization into a `String` field in Rust will fail. Explicit conversion ensures compatibility.
    *   **Code**: `SELECT *, type::string(id) as id FROM table:id`
    *   **IMPORTANT**: The converted ID includes the table prefix (e.g., `"claude_provider:abc123"`). When passing this ID to the frontend or using it in subsequent operations, **you must strip the table prefix** (e.g., `"abc123"`) in the adapter layer before returning to business logic.
    *   **Use Common Utility**: Always use the `db_id` module for ID handling:
        ```rust
        // In adapter.rs
        use crate::coding::db_id::db_extract_id;

        pub fn from_db_value_provider(value: Value) -> ClaudeCodeProvider {
            let id = db_extract_id(&value);
            // ...
        }
        ```
    *   **Available Functions** (`crate::coding::db_id`):
        *   `db_extract_id(record: &Value) -> String` - Extract and clean ID from a record
        *   `db_extract_id_opt(record: &Value) -> Option<String>` - Same but returns Option
        *   `db_clean_id(raw_id: &str) -> String` - Clean a raw ID string
        *   `db_build_id(table: &str, id: &str) -> String` - Build a record ID string
        *   `db_record_id(table: &str, id: &str) -> String` - Build backtick-escaped record reference for queries (e.g., `` table:`id` ``)
        *   `db_new_id() -> String` - Generate a new record ID (UUID v4, no hyphens)

2.  **Record ID Reference in Queries**: Use `db_record_id()` to build backtick-escaped record references.
    *   **Problem**: `type::thing('table', $id)` behavior changed across SurrealDB versions (e.g., 2.4 → 2.6), causing "not found" errors even for existing records.
    *   **Solution**: Use `db_record_id(table, id)` which generates `` table:`id` `` format. Backtick-escaped IDs are treated as literal strings regardless of content, avoiding version-specific parsing issues.
    *   **NEVER** use `type::thing()` in any query. Always use `db_record_id()` instead.
    *   **Code**:
        ```rust
        use crate::coding::db_id::db_record_id;

        // SELECT by ID
        let record_id = db_record_id("claude_provider", &id);
        db.query(&format!("SELECT *, type::string(id) as id FROM {} LIMIT 1", record_id))

        // UPDATE by ID
        let record_id = db_record_id("mcp_server", &server_id);
        db.query(&format!("UPDATE {} SET enabled = $enabled", record_id))
            .bind(("enabled", true))

        // DELETE by ID
        let record_id = db_record_id("mcp_server", server_id);
        db.query(&format!("DELETE {}", record_id))
        ```
    *   **Applies to**: All queries that target a specific record by ID:
        *   `SELECT ... FROM {record_id}`
        *   `UPDATE {record_id} SET ...` or `UPDATE {record_id} CONTENT $data`
        *   `DELETE {record_id}`
        *   `CREATE {record_id} CONTENT $data`
        *   `UPSERT {record_id} CONTENT $data` or `UPSERT {record_id} SET ...`

    *   **Batch record lookup rule**: Do **NOT** use `SELECT ... FROM $ids` and then deserialize directly into a strong Rust list when any referenced record may be missing.
        *   **Why**: SurrealDB can yield `NONE` elements for missing records, which then explode during Rust deserialization with errors like `Expected a string but cannot convert NONE into a string`.
        *   **Safe pattern**: Query the concrete table and filter with `WHERE id INSIDE $record_refs`, where `$record_refs` is a `Vec<surrealdb::sql::Thing>` built from real record refs such as `Thing::from(("image_asset", "abc"))`.
        *   **Ordering**: Do not assume SurrealDB preserves the input ID order. If caller-visible order matters, rebuild the output order in Rust using the original ID list.
        *   **Duplicates**: If duplicate IDs in the input are semantically meaningful, restore duplicates during the Rust-side reorder step rather than expecting the database to return repeated rows.

3.  **Record ID Generation**: Prefer SurrealDB auto-generated IDs. When manual IDs are needed, use `db_new_id()`.
    *   **Preferred**: Let SurrealDB auto-generate IDs via `CREATE table CONTENT $data` (no ID specified). This is how `claude_provider`, `codex_provider`, `oh_my_opencode_config`, etc. work.
    *   **When manual IDs are needed** (e.g., MCP servers, skills): Use the shared `db_new_id()` function which generates UUID v4 without hyphens.
    *   **NEVER** call `uuid::Uuid::new_v4()` directly in store/command files. Always use `db_new_id()` from the `db_id` module.
    *   **Code**:
        ```rust
        use crate::coding::db_id::{db_record_id, db_new_id};

        // Create with manual ID
        let id = db_new_id();
        let record_id = db_record_id("mcp_server", &id);
        db.query(&format!("CREATE {} CONTENT $data", record_id))
            .bind(("data", payload))
        ```

4.  **Updates**: Use **Blind Writes (Overwrite)** to bypass version checks.
    *   **Avoid**: Do NOT send the `version` or `revision` field back to the database in the `CONTENT` block. This triggers optimistic currency control checks which often fail.
    *   **Avoid**: Do NOT include the `id` field in the `CONTENT` block. It can cause type conflicts.
    *   **Pattern 1 (Update by ID)**: Use `db_record_id()` to target a specific record:
        ```rust
        let record_id = db_record_id("claude_provider", &id);
        db.query(&format!("UPDATE {} CONTENT $data", record_id))
            .bind(("data", payload))
        ```
    *   **Pattern 2 (Create or Update)**: Use `UPSERT` with `db_record_id()` for singleton or known-ID records:
        ```rust
        let record_id = db_record_id("settings", "app");
        db.query(&format!("UPSERT {} CONTENT $data", record_id))
            .bind(("data", payload))
        ```
        Or use hardcoded backtick format for fixed singleton IDs: `UPSERT settings:\`app\` CONTENT $data`
    *   **Pattern 3 (Single Field)**: `UPDATE {record_id} SET field = $value`
    *   **Pattern 4 (Batch by condition)**: `UPDATE table SET field = $value WHERE condition = true` (no ID targeting needed)

5.  **SurrealDB Wrapper Characters**: Handled automatically by `db_extract_id()` / `db_clean_id()`. No manual handling needed — these functions strip table prefixes and `⟨⟩` wrappers transparently.

```rust
// commands.rs
#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, DbState>) -> Result<AppSettings, String> {
    let db = state.0.lock().await;

    // CRITICAL: Convert `Thing` ID to string to match Rust struct types
    // This avoids "invalid type: map, expected a string" errors
    let mut result = db
        .query("SELECT *, type::string(id) as id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query settings: {}", e))?;

    let records: Vec<serde_json::Value> = result.take(0).map_err(|e| e.to_string())?;

    if let Some(record) = records.first() {
        Ok(adapter::from_db_value(record.clone()))
    } else {
        Ok(AppSettings::default())
    }
}

#[tauri::command]
pub async fn save_settings(
    state: tauri::State<'_, DbState>,
    settings: AppSettings,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Serialize settings but EXCLUDE sensitive system fields
    // Ensure `adapter::to_clean_payload` removes 'id' and 'version'/'revision'
    let json_payload = adapter::to_clean_payload(&settings);

    // CRITICAL for Updates:
    // 1. Use CONTENT with a clean payload (no version = no lock check).
    // 2. ID is used in the query target with native format, NOT in the content.
    // 3. Use UPSERT for singleton records to handle both create and update:
    //    UPSERT settings:`app` CONTENT $data
    db.query("UPSERT settings:`app` CONTENT $data")
        .bind(("data", json_payload)) // Clean data without ID/Version
        .await
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    Ok(())
}
```

### Benefits of Direct Database Access

1. **Performance**: SurrealDB with SurrealKV engine is embedded and extremely fast
2. **Consistency**: Single source of truth for all data
3. **Backup**: Database files can be backed up/restored as a whole
4. **No Sync Issues**: Avoids complex synchronization between localStorage and database

---

## System Tray Menu Integration

### Overview

The system tray menu provides quick access to configuration selections without opening the main window. When configurations are changed (either from the main window or the tray menu), the tray menu must stay in sync.

### Event-Driven Architecture

All configuration changes use the `config-changed` Tauri event to synchronize state:

| Source | Event Payload | Tray Refresh | Page Reload |
|--------|---------------|--------------|-------------|
| Main Window | `"window"` | ✅ | ❌ |
| Tray Menu | `"tray"` | ✅ | ✅ |

### Backend Implementation

#### 1. Internal Function Pattern

All modules should implement an internal function `apply_config_internal` that handles configuration saving and event emission:

```rust
// commands.rs
pub async fn apply_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle<R>,
    config: ModuleConfig,
    from_tray: bool,
) -> Result<(), String> {
    // 1. Save configuration to file/database
    save_config_to_file(state, &config).await?;

    // 2. Update database state if needed
    update_db_state(state, &config).await?;

    // 3. Emit event based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    Ok(())
}
```

#### 2. Tauri Command (Main Window)

The Tauri command called by the frontend passes `from_tray: false`:

```rust
#[tauri::command]
pub async fn save_module_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config: ModuleConfig,
) -> Result<(), String> {
    apply_config_internal(state, &app, config, false).await
}
```

#### 3. Tray Support Module

The tray support module calls with `from_tray: true`:

```rust
// tray_support.rs
pub async fn apply_module_selection<R: Runtime>(
    app: &AppHandle<R>,
    selection_id: &str,
) -> Result<(), String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    // Build config from selection
    let config = build_config_from_selection(&db, selection_id)?;

    // Apply with from_tray: true
    super::commands::apply_config_internal(&db, app, config, true).await?;

    Ok(())
}
```

#### 4. Global Event Listener (lib.rs)

The main entry point registers a global listener that refreshes the tray menu on any `config-changed` event:

```rust
// lib.rs
let app_handle_clone = app_handle.clone();
tauri::async_runtime::spawn(async move {
    let value = app_handle_clone.clone();
    let value_for_closure = value.clone();
    let listener = value.listen("config-changed", move |_event| {
        let app = value_for_closure.app_handle().clone();
        let _ = tauri::async_runtime::spawn(async move {
            let _ = tray::refresh_tray_menus(&app);
        });
    });
    let _ = listener;
});
```

### Frontend Implementation

#### 1. Event Listener (providers.tsx)

The app's main provider listens for `config-changed` events and triggers a page reload only for tray menu changes:

```typescript
// web/app/providers.tsx
use { listen } from '@tauri-apps/api/event';

React.useEffect(() => {
  const setupListener = async () => {
    unlisten = await listen<string>('config-changed', (event) => {
      const configType = event.payload;
      // Only reload page when change comes from tray menu
      if (configType === 'tray') {
        window.location.reload();
      }
      // Changes from main window only refresh the tray menu (handled by backend)
    });
  };
  setupListener();
  return () => { if (unlisten) unlisten(); };
}, []);
```

### Tray Support Module Structure

Each coding module with tray integration should have:

```
tauri/src/coding/{module_name}/
├── commands.rs          # Tauri commands + apply_config_internal
├── tray_support.rs      # Tray-specific functions
├── adapter.rs           # DB value adapters
└── types.rs             # Type definitions
```

### Tray Support Module Functions

The `tray_support.rs` must export:

```rust
// Data structures
pub struct TrayData {
    pub title: String,           // Section title
    pub items: Vec<TrayItem>,    // Selection items
}

pub struct TrayItem {
    pub id: String,              // Unique identifier
    pub display_name: String,    // Display text
    pub is_selected: bool,       // Current selection state
}

// Required functions
pub async fn get_{module}_tray_data<R: Runtime>(app: &AppHandle<R>)
    -> Result<TrayData, String>;

pub async fn apply_{module}_selection<R: Runtime>(app: &AppHandle<R>, id: &str)
    -> Result<(), String>;
```

### Menu Refresh Function

The `tray.rs` module exports:

```rust
pub async fn refresh_tray_menus<R: Runtime>(app: &AppHandle<R>)
    -> Result<(), String> {
    // 1. Fetch data from all modules
    let module_data = module_tray::get_module_tray_data(app).await?;

    // 2. Build menu items with checkmarks
    let items = build_menu_items(app, &module_data)?;

    // 3. Update tray menu
    let tray = app.state::<tauri::tray::TrayIcon>();
    tray.set_menu(Some(menu))?;

    Ok(())
}
```

### File Structure

```
tauri/src/
├── tray.rs                    # Main tray menu builder
├── lib.rs                     # Global event listener setup
└── coding/
    └── {module}/
        ├── commands.rs        # apply_config_internal + Tauri commands
        ├── tray_support.rs    # Tray data fetching + apply functions
        ├── adapter.rs
        └── types.rs

web/
├── app/
│   └── providers.tsx          # config-changed event listener
└── services/
    └── {module}Api.ts         # Backend API wrappers
```

### Implementation Checklist for New Tray Integration

1. **Backend** (`tauri/src/coding/{module}/`):
   - [ ] Add `apply_config_internal` function with `from_tray` parameter
   - [ ] Implement Tauri command for main window (calls with `false`)
   - [ ] Implement tray support functions:
     - `get_{module}_tray_data()` - returns current selections
     - `apply_{module}_selection()` - handles tray menu selection (calls with `true`)
   - [ ] Emit `config-changed` event with `"window"` or `"tray"` payload

2. **Frontend** (`web/app/providers.tsx`):
   - [ ] Ensure `config-changed` event listener reloads page only for `"tray"` payload

3. **Main Entry** (`tauri/src/lib.rs`):
   - [ ] Global listener already exists - no changes needed

---

## OpenCode Configuration Format

迁移期说明：本节原有规则继续保留，不能弱化。实际修改 OpenCode 配置、tray 或页面交互时，还应同时阅读模块级文档：
- `tauri/src/coding/open_code/AGENTS.md`
- `web/features/coding/opencode/AGENTS.md`

### Model Selection

- OpenCode 的 `model` / `small_model` 都使用 `provider_id/model_id` 格式，例如 `openai/gpt-4o`。
- tray 选择模型时，必须沿用同一格式写回配置；不能在 tray 或页面层把它降成裸 `model_id`。
- tray 改动属于真实配置写入，不是纯展示切换；它会发出 `config-changed` 的 `"tray"` payload，并触发前端 reload。
- 具体 tray 展示、过滤和选中态细节见：
  - `tauri/src/coding/open_code/AGENTS.md`
  - `web/features/coding/opencode/AGENTS.md`

### Provider Import Semantics

- 详细 Why / Gotcha / 历史语义已迁到：
  - `tauri/src/coding/open_code/AGENTS.md`
  - `web/features/coding/opencode/AGENTS.md`
  - `web/features/coding/shared/AGENTS.md`
- 根文档只保留关键事实：
  - `favorite provider` / `导入我使用过的供应商` 不是 OpenCode 当前配置的镜像。
  - 它的产品语义是“使用过的供应商历史库 + 诊断缓存”。
  - 需要读取“当前 OpenCode provider”时，应直接读当前配置文件，而不是复用 favorite provider 库。

---

## HTTP Client Guidelines

All HTTP requests in the Rust backend MUST use the unified `http_client` module to ensure proxy settings are respected.

### Usage

```rust
use crate::http_client;
use crate::db::DbState;

// Standard request (30s timeout, auto proxy)
let client = http_client::client(&state).await?;

// Custom timeout
let client = http_client::client_with_timeout(&state, 60).await?;

// Bypass proxy (special cases only)
let client = http_client::client_no_proxy(30)?;

// Get proxy URL directly (for non-HTTP use cases like git)
let proxy_url = http_client::get_proxy_from_settings(&state).await?;
// Returns empty string if not configured
```

### Rules

1. **NEVER** use `reqwest::Client::new()` or `reqwest::Client::builder()` directly
2. **ALWAYS** use `http_client::client()` for requests that should respect proxy settings
3. Use `http_client::client_no_proxy()` only when you explicitly need to bypass proxy
4. **For non-HTTP proxy needs** (e.g., git operations, external CLI tools): Use `http_client::get_proxy_from_settings()` to retrieve the proxy URL and apply it appropriately (e.g., set environment variables like `HTTP_PROXY`/`HTTPS_PROXY`)

### Supported Proxy Formats

- HTTP: `http://proxy.example.com:8080`
- HTTP with auth: `http://user:pass@proxy.example.com:8080`
- SOCKS5: `socks5://proxy.example.com:1080`
- SOCKS5 with auth: `socks5://user:pass@proxy.example.com:1080`

### Files Using http_client

- `tauri/src/update.rs` - Update checking
- `tauri/src/settings/backup/webdav.rs` - WebDAV operations
- `tauri/src/coding/open_code/models_api.rs` - Provider model fetching
- `tauri/src/skills/installer.rs` - Git operations proxy
- `tauri/src/skills/commands.rs` - Git operations proxy

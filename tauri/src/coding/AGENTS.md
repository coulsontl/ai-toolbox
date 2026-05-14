# Coding 模块说明

## 一句话职责

- `tauri/src/coding/` 是内置 coding 工具、Skills、MCP、WSL/SSH 同步和运行时定位的共享后端域。
- 这里最重要的不是某个工具单独怎么存，而是跨工具共享的路径决议、事件约定和跨平台执行语义。

## Source of Truth

- 各工具的业务配置主数据分别存于 SurrealDB 和对应运行时配置文件，两者都重要，但“当前生效路径”不是简单看页面输入框，而是由 `runtime_location` 统一决议。
- `runtime_location` 是各 coding tab 当前运行时位置、WSL Direct 状态和派生文件路径的唯一共享规则源。
- `runtime_location` 的同步 helper 只允许读取进程内 runtime location cache 或无 DB fallback；需要 SurrealDB、环境变量和 shell 配置参与解析时，必须走异步 refresh API 先刷新缓存，不能在同步 helper 里查 DB 或 `block_on`。
- 对这些 runtime tab，`source` 与 `is_wsl_direct` 是两个独立维度：`source` 只说明路径来源，`is_wsl_direct` 只说明当前生效路径是否为 WSL UNC；`module_statuses` 来自后者，不来自页面展示。
- runtime tab 分成两类：OpenCode/OpenClaw 是“配置文件路径模块”，Claude/Codex/Gemini CLI 是“根目录模块”。后续 prompt、auth、plugins、skills 等派生路径都必须先尊重这个分层。
- `config-changed`、`wsl-sync-request-*`、`skills-changed`、`mcp-changed` 是跨模块联动的主事件契约；事件本身不保存状态，只触发后续动作。

## 核心设计决策（Why）

- runtime tab 的运行时路径统一收敛到 `runtime_location.rs`，避免每个模块各自判断 WSL UNC、默认路径和派生路径，导致逻辑分叉。
- 托盘刷新采用全局 `config-changed` 事件，而不是每个模块各自直接操作托盘，这样主窗口和托盘入口可以共享一套刷新机制。
- WSL 自动同步用 `lib.rs` 里的事件监听器集中触发，而不是在每个业务命令里直接调用 WSL 同步实现；这样可把“是否开启自动同步”判断统一放在监听器层。

## 关键流程

```mermaid
sequenceDiagram
  participant UI as Frontend
  participant Cmd as Tool Command
  participant DB as SurrealDB
  participant Runtime as Runtime Files
  participant App as lib.rs Listeners

  UI->>Cmd: save/apply config
  Cmd->>DB: update records if needed
  Cmd->>Runtime: write config/prompt/auth file
  Cmd-->>App: emit config-changed / wsl-sync-request-*
  App->>App: refresh tray / trigger WSL auto sync
```

## 易错点与历史坑（Gotchas）

- 不要把“页面上显示的 `source`”和“WSL/SSH 设置页里的 `moduleStatuses.is_wsl_direct`”混为一谈。前者是路径来源标签，后者是对当前生效运行时路径的统一诊断结果。
- 不要把 OpenCode/OpenClaw 与 Claude/Codex 按同一种“自定义配置”处理。前两者改的是文件路径，后两者改的是根目录；一旦混写，后续所有派生路径都会偏掉。
- Claude Code 2.1.126 本机实测路径语义：未设置 `CLAUDE_CONFIG_DIR` 时，`settings.json` / `CLAUDE.md` / `config.json` / `plugins/` / `skills/` 位于 `~/.claude/` 下，MCP 与 onboarding 使用用户 home 下的 `~/.claude.json`；显式 `root_dir` / `CLAUDE_CONFIG_DIR` / shell 配置目录后，上述目录和 `.claude.json` 都位于该配置目录内。即使显式配置目录正好是 `~/.claude`，`.claude.json` 也应按显式目录解析为 `~/.claude/.claude.json`，不能只按路径值等于默认目录来判断。
- Claude plugins 还有独立的 `CLAUDE_CODE_PLUGIN_CACHE_DIR` 覆盖项；实测设置后 `known_marketplaces.json` / `installed_plugins.json` 位于该 plugin cache 目录，而不是 `CLAUDE_CONFIG_DIR/plugins`。这个路径同样要通过 runtime location cache 派生，不能在同步 helper 里临时查环境变量或 shell 配置。
- Claude Code 的本机自定义配置目录只影响本机 Claude CLI 消费路径；普通 WSL/SSH 同步仍把远端写到默认 `~/.claude/*` 与 `~/.claude.json`。只有当前运行时路径本身是 WSL Direct 自定义根目录时，WSL 目标才跟随该 Linux 配置目录，例如 `/home/user/custom-claude/settings.json`、`/home/user/custom-claude/plugins`、`/home/user/custom-claude/.claude.json`。
- 改 `root_dir` / `config_path` 保存逻辑时，保存 DB 后要先刷新对应 runtime location cache，再继续 apply 配置文件、比较 Skills 目标路径、发 WSL/SSH 相关同步事件。否则后续同步 helper 可能继续消费旧路径。
- 对 OpenCode、Claude Code、Codex、OpenClaw、Gemini CLI 这类模块，文件 I/O 能直接读写 UNC 路径，不代表 CLI 也能直接吃 UNC 路径。新增 CLI 能力时必须先经过 `runtime_location::*_runtime_location_async` 判定。
- 对 OpenCode、Claude Code、Codex、OpenClaw、Gemini CLI 这类用户自行安装的 CLI，不要默认 GUI 进程里 `PATH` 可用。尤其 macOS 从 Dock/Finder 启动时，新增调用应优先解析已知安装位置或显式配置路径，再回退到 `PATH`。
- 本机 CLI 查找统一走 `cli_resolver.rs`。不要在单个工具模块里各自手写 `which`/`where`、nvm、volta、fnm 或 Windows `.cmd`/`.bat` 处理，否则 Dock/Finder 启动和 Node 版本管理器安装场景会再次分叉。
- 新增跨工具共享规则时，优先放在共享层，不要把通用逻辑塞进某个单独工具目录，否则后续很快出现“相邻工具修了一边，另一边继续错”。

## 跨模块依赖

- 被 `wsl/`、`ssh/`、`skills/` 和各工具模块依赖：它们都会消费 `runtime_location` 的派生路径或 WSL Direct 状态。
- 被前端 `web/features/settings/` 和各工具页面间接依赖：前端展示的路径来源、WSL Direct 提示和同步跳过逻辑最终都依赖这里的后端状态。

## 典型变更场景（按需）

- 新增需要调用工具 CLI 的能力时：
  先检查现有内置工具是否都存在同类调用点，并确认本机/WSL Direct 两套执行路径。
- 新增新的跨模块事件时：
  先判断是否应复用现有事件契约；如果新增，必须同时梳理监听端和前端刷新端。

## 最小验证

- 改 `runtime_location` 后，至少验证一个本机路径场景和一个 WSL UNC 路径场景。
- 改事件约定后，至少验证主窗口保存、托盘刷新、WSL 设置页状态三者是否仍一致。

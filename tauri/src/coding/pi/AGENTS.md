# Pi 后端模块说明

## 一句话职责

- `pi/` 负责 Pi CLI 全局 root、`settings.json`、`auth.json`、`models.json`、全局 prompt、扩展和页面 runtime view。

## Source of Truth

- Pi provider 的事实源是 Pi runtime 文件，不是 AI Toolbox 数据库 provider 表。
- `auth.json.<providerKey>` 是 API key / OAuth credential entry。
- `models.json.providers.<providerKey>` 是 custom provider 或 built-in provider override。
- `settings.json.defaultProvider/defaultModel/defaultThinkingLevel` 只表示默认启动选择，不表示唯一生效 provider。
- Pi extensions 的事实源是 Pi CLI 输出和当前 runtime root 下的 `extensions/` 目录，不是 AI Toolbox 数据库。
- `settings.json.packages` 属于 Pi 扩展/包管理链路，不属于 Other Configuration；Other Configuration 读取时隐藏它，保存时保留现有值。
- Pi MCP 配置由 `pi-mcp-adapter` 扩展消费，文件位于当前 Pi runtime root 下的 `mcp.json`；MCP server 主数据仍属于全局 MCP 模块。
- SQLite 只保存 Pi root 选择和 prompt presets；不要新增 `pi_provider`、`pi_extension` 或类似第二套主数据。

## 核心设计决策

- Pi 原生支持多 provider / model，产品形态按 OpenCode 的“运行时配置可视化”处理。
- 保存 provider 时只 upsert 当前 exact runtime key；如果 key 是 `anthropic`、`openrouter` 等官方内置 key，也是在原 key 上覆盖/补充，不生成 `ai-toolbox-*` 包装 provider。
- `defaultModel` 写 Pi 官方 settings 的裸 model id。model id 本身可能包含 `/`，不要拼成 OpenCode 风格的 `provider_id/model_id`。
- 扩展管理优先通过 Pi CLI 执行 `list/install/remove/update`，并优先附带 `--no-approve`，避免非交互环境下 project trust 提示卡住；若用户 Pi CLI 过旧或不识别该 flag（例如 `Unknown option --no-approve for "list"`），必须降级重试一次不带该 flag。本地 `.ts` 文件扩展只扫描当前 runtime root 派生的 `extensions/` 目录。

## Gotchas

- 删除 prompt preset 只删 SQLite 记录，不改写/清空当前 runtime prompt 文件。产品语义是“删除已保存的提示词记录”，不是“清空本地 runtime 提示词”。
- 内置 provider 即使没有写入 `auth.json` 或 `models.json`，也可能通过环境变量或 Pi `/login` 可用；不要显示为 missing。
- `auth.json` OAuth token 是 Pi runtime-owned。AI Toolbox 可以识别和保留，但首版不编辑 token、不发起 `/login`。
- `models.json` 允许 unknown top-level 和 provider/model unknown fields。读写必须 preserve unknown fields。
- Pi 原生不支持 MCP；只有安装 `pi-mcp-adapter` 后才会读取 `<runtime-root>/mcp.json`。MCP 页面可以把 Pi 作为同步目标，但不要把它误认为 Pi provider/native config。
- 不要硬编码 `~/.pi/agent/extensions`。Pi root 可能来自应用内 custom root、`PI_CODING_AGENT_DIR`、shell 配置、默认路径或 WSL Direct，扩展目录必须从当前 runtime location 派生。
- Windows 文件夹选择器在 WSL UNC 下可能只能选到 `~/.pi`，但末段名为 `.pi` 的目录也可能是合法 custom root。Pi 设置保存和 runtime cache 刷新只有在当前目录没有 Pi runtime 数据、且其 `agent` 子目录已存在 Pi runtime 布局时，才归一化并回写为 `~/.pi/agent`；不能只凭目录名迁移。
- WSL Direct 扩展命令需要把 mise/asdf/bun/Volta/fnm/用户 npm bin 前置到原 WSL `$PATH`。动态 root 和扩展 source 必须保持为独立进程参数，不能拼进 shell 命令字符串；否则含空格路径会拆参，Shell 元字符还会改变命令结构。
- 本机 `pi` CLI 解析走共享 `cli_resolver`。Windows 上用 `bun install -g` 安装的 `pi` 默认在 `%USERPROFILE%\.bun\bin`（或 `$BUN_INSTALL\bin`）；GUI 启动时不一定继承终端 PATH，必须把 bun 全局 bin 纳入候选路径，不能只查 nvm/volta/fnm/npm。
- 多路径 `pi` 时**不会**比版本选最新：先用进程 PATH 的 `which`/`where` 结果（非 Windows 取第一条，Windows 按 `.exe`/`.cmd`/… 扩展名优先级），PATH 查不到再按候选顺序（`~/.local/bin` → `/opt/homebrew/bin` → `/usr/local/bin` → node/bun 全局 bin）。扩展 list/install/remove/update 失败错误必须附带解析到的 `pi_cli=` 路径；list 成功响应应带 `cliPath`/`cliVersion` 方便对照终端里的 `where pi`。
- `pi-deck-*` 和 `ai-toolbox-*` 本地扩展按内置/受保护处理，页面不要提供直接删除入口。
- 保存 Other Configuration 时不要清空或覆盖 `settings.json.packages`；扩展管理区已经负责 package 安装、列表和卸载入口。

## 最小验证

- `settings.defaultProvider = "anthropic"` 且 `auth.json`/`models.json` 没有 `anthropic` 时，provider view 应标记 built-in/default，不是 missing。
- 同一个 key 同时存在 `auth.json` 和 `models.json.providers` 时，应合并成一条 provider view。
- 保存 `models.json.providers.<key>` 只覆盖该 key，其他 providers 和 unknown top-level 字段原样保留。
- 自定义 Pi root 下的扩展列表应扫描 `<custom-root>/extensions`，不是默认 home 目录。
- WSL Direct Pi root 或扩展 source 含空格和 Shell 元字符时，`list/install/remove/update` 的命令参数边界必须保持不变，且补充 shim 后仍保留原 WSL `$PATH`。
- `pi list`（优先带 `--no-approve`，不支持时回退）返回的 package 扩展和 `extensions/*.ts` / `extensions/<dir>/index.ts` 本地扩展应合并展示。
- 扩展命令遇到 `Unknown option --no-approve` 时不能把该错误直接当作最终失败；应去掉 flag 重试，并在 UI/错误里仍允许后续提示用户升级官方 Pi CLI。
- 扩展 CLI 失败文案应包含 `pi_cli=<resolved path or wsl -d <distro> -- pi>`；list 成功时 meta 区应能看到同一路径和 `pi --version` 探测结果（探测失败可省略版本）。
- `settings.json` 中已有 `packages` 时，`read_pi_runtime_config().other_settings` 不返回该字段，`save_pi_other_settings` 也不会删除或覆盖它。
- Pi 0.80.6 起共享 thinking ladder 是 `off/minimal/low/medium/high/xhigh/max`。AI Toolbox 的前端选项、preset `thinkingLevelMap` 转换和后端校验白名单必须同步维护这七档；缺省的标准档 `off/minimal/low/medium/high` 按 identity mapping 支持，扩展档 `xhigh/max` 必须由模型显式提供非 `null` 映射才算支持。`ultra` 属于 Codex 的主动多智能体档位，不是 Pi thinking level，不能加入 Pi settings 或模型映射。

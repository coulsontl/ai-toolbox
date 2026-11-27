---
name: ai-toolbox
description: AI Toolbox 桌面应用的内置使用指南。当用户需要管理 Coding CLI 配置、配置本机 Proxy Gateway、同步 Skills/MCP、浏览会话、处理 WSL/SSH 同步、备份恢复或使用 Magpie 插件时，先读本技能。
---

# AI Toolbox 使用指南

AI Toolbox 是面向 AI 编程工具的个人配置工作台。它把分散在各 CLI 里的供应商、模型、提示词、Skills、MCP 和会话集中到一处管理，并可在本机启动统一代理网关。

本技能是随应用版本发布的内置文档，只读且不可删除。需要编写或调试插件时，继续读同目录的 `references/plugins.md`。

## 1. 核心能力速览

| 能力 | 能做什么 | 用户通常在哪里操作 |
| --- | --- | --- |
| Coding CLI 配置管理 | 统一管理 17 个 Coding CLI 的供应商、模型、官方账号、全局提示词、插件和根目录 | 左侧对应 CLI 的 Tab |
| Proxy Gateway | 在本机启动多协议代理网关，接管 Claude Code / Claude Desktop / Codex / Grok / Kimi / Gemini CLI / Antigravity / OpenCode / WorkBuddy / CodeBuddy 等客户端，做协议转换、故障切换、请求日志和用量统计 | `Gateway` Tab |
| Skills 中央仓库 | 从 Git 仓库或本地目录安装 Skill，保存在中央仓库，再按工具同步；支持 Universal 公共目录、自动发现、分组、标签和批量操作 | `Skills` Tab |
| MCP 管理 | 集中维护 MCP Server，导入导出、分组、收藏，并同步到多个支持 MCP 的工具 | `MCP` Tab |
| 会话管理 | 浏览、搜索、重命名、导入、导出和删除各 CLI 的会话记录 | 各 CLI Tab 的会话区或 `Session` 入口 |
| WSL / SSH 同步 | 把本机配置、MCP 和 Skills 同步到 WSL 或远程 SSH 主机 | `Settings` → WSL / SSH |
| 备份恢复 | 本地、WebDAV、GitHub/Gitee 私有仓库备份，可加密、可自动执行 | `Settings` → 备份 |
| 插件体系 | 安装 Magpie / OpenCode 兼容插件，提供 Provider 渠道、Gateway 中间件和自定义页面 | `Plugins` / 插件中心 |
| 系统托盘 | 不打开主窗口即可切换常用供应商、模型、提示词、MCP 和 Skills | 托盘菜单 |
| 深链接 | 通过 `aitoolbox://` 导入或分享供应商配置 | 浏览器 / 聊天应用 |

## 2. 常用操作

### 2.1 新增或切换供应商

1. 打开目标 CLI 的 Tab，例如 `Claude Code`、`Codex`、`OpenCode`。
2. 点击「添加供应商」，填写名称、Base URL、API Key 和模型。
3. 保存后在供应商卡片上点击「启用 / 应用」，AI Toolbox 会按该 CLI 的原生配置格式写入，并保留未知字段。
4. 需要跨工具复用时，用供应商卡片的分享/深链接功能，而不是手工复制多份配置。

注意：

- 每个 CLI 的配置路径和文件格式不同，不要用同一段 JSON/TOML 到处粘贴。
- 配置根目录可在对应 Tab 的根目录设置中修改；保存后依赖该路径的 Skills/MCP 目标也会跟着重算。
- 官方账号与自定义供应商是两条链路：官方账号负责 OAuth/额度，自定义供应商负责 Base URL/API Key。

### 2.2 开启 Gateway 接管与跨协议转换

1. 打开 `Gateway` Tab，确认监听端口（默认 `37123`）和启动状态。
2. 在「接管 / 应用」区域选择要接管的 CLI。接管后，该 CLI 的请求会先进入本机网关。
3. 在供应商上配置目标协议。网关支持四类协议的互转：
   - Anthropic Messages：`/v1/messages`
   - OpenAI Chat Completions：`/v1/chat/completions`
   - OpenAI Responses：`/v1/responses`
   - Gemini Native：`/v1beta/...`
4. 外部客户端也可以直接调用网关，不需要经过 CLI 接管。
5. 请求日志和用量统计在 `Gateway` 的统计/明细页查看；后台事件只静默刷新数据。

客户端连接网关时，Base URL 通常填写：

```text
http://127.0.0.1:37123
```

Anthropic 风格客户端使用 `http://127.0.0.1:37123/v1/messages`；OpenAI 风格客户端使用 `/v1` 或 `/v1/chat/completions`；Gemini 风格客户端使用 `/v1beta`。

### 2.3 官方账号切换与额度查看

- 官方账号在对应 CLI Tab 的官方账号区域管理，可登录、刷新、切换、删除。
- 额度查询是只读操作，不会替你刷新令牌或自动换号。
- 不同 CLI 的额度窗口不同（5 小时、周、月等）；页面展示的是供应商实际返回的结果，失败时保持未知，不伪造余额。
- 切换账号后，已经启动的会话可能需要重启才能读取新凭据。

### 2.4 Skills 中央仓库与 Universal 同步

- Skills 的事实源是「中央仓库」，不是某个工具的运行时目录。默认在应用数据目录下的 `skills/`，可在 `Skills` 设置里修改。
- Universal（`shared_agents`）对应 `~/.agents/skills`，是 agentskills.io 风格的跨工具公共目录；Cline、Warp、Kimi 等工具也会读取它。
- 安装 Skill 后，在卡片底部的工具图标处点击即可同步/取消同步；`+` 菜单可添加更多工具。
- 同步模式由 AI Toolbox 自动选择：优先符号链接，Windows 无权限时退回目录接合点，Cursor / Antigravity CLI / Antigravity Global 等不支持链接的工具强制复制。
- 复制模式的目标不会自动跟随中央仓库更新。中央仓库内容更新后，需要让 AI Toolbox 重新同步该工具（更新链路会自动重拷已启用工具）。
- 自动发现每 60 秒扫描一次配置的来源目录，只把「本轮新纳管」的 Skill 分发到目标工具；不会把用户手动取消的工具绑定重新加回来。
- 分组、备注、标签是 AI Toolbox 内部元数据，不写入 `SKILL.md`，也不触发工具同步。
- 内置技能 `ai-toolbox` 随应用发布，不可删除、不可编辑，但可以自由取消 Universal 或任意工具的同步。

### 2.5 MCP 管理

- 在 `MCP` Tab 添加 Server，可选择传输方式（stdio / HTTP / SSE）和启动命令或 URL。
- 保存后按工具同步；各工具的 MCP 配置文件和字段名不同，AI Toolbox 负责转换。
- 导入/导出和分组只作用于管理元数据，不会删除工具目录里的外部 MCP 配置。

### 2.6 WSL / SSH 同步

- WSL 同步由 `skills-changed` 等事件触发，把中央仓库内容同步到 WSL 侧的 `~/.ai-toolbox/skills`，再按工具策略维护副本或链接。
- SSH 同步是独立链路，不走文件映射；它把配置、MCP 和 Skills 同步到远端统一目录。
- 对于已经在 WSL 内直接运行的 CLI（WSL Direct），不要再用 Windows 侧路径重复写入其 Skills 目录。
- 远端目标清理失败时，AI Toolbox 会保留中央条目并记录 warning，下次同步重试。

### 2.7 备份与恢复

- 备份入口在 `Settings` → 备份；支持本地目录、WebDAV、GitHub/Gitee 私有仓库。
- 可以选择备份项、排除敏感文件，并设置加密口令。加密口令保存在系统凭据库，不写进备份文件。
- 备份是「配置与元数据」备份，不是所有工具运行时目录的完整镜像。
- 恢复后建议重启应用，让托盘、WSL/SSH 和网关重新读取配置。
- 跨机器恢复时，路径会尽量按 `~/...`、`%APPDATA%/...` 这类可迁移形式解析；旧机器的绝对用户路径不保证可用。

## 3. 常见问题排查

### 3.1 网关启动失败或端口占用

- 先确认 `Gateway` 页面显示的监听端口是否被其他进程占用。Windows 可用 `netstat -ano | findstr :37123`，macOS/Linux 可用 `lsof -i :37123`。
- 如果端口被占用，修改 Gateway 监听端口后重启网关。
- 检查是否有防火墙/安全软件拦截 `127.0.0.1` 本地监听。
- 看 Gateway 的运行日志和请求明细；后台的 usage 记录事件只刷新数据，不代表失败。
- 接管后 CLI 报 401/403 时，先确认对应供应商的 Key 和 Base URL，再确认网关是否真的启动。

### 3.2 插件里的模型连不通

- 在插件中心打开对应 Provider，先确认账号状态为已登录。
- 点「检查 / 连通性测试」，确认模型列表能拉到。
- 如果 Provider 插件声明的模型 `npm` 与上游协议不匹配，网关会按声明的协议转发；检查 `npm` 映射是否正确：
  - `@ai-sdk/anthropic` → Anthropic Messages
  - `@ai-sdk/openai` → OpenAI Responses
  - `@ai-sdk/openai-compatible` / `@ai-sdk/openrouter` / `@ai-sdk/deepseek` / `@ai-sdk/xai` → OpenAI Chat
  - `@ai-sdk/google` / `@ai-sdk/google-vertex` → Gemini Native
- OAuth 账号过期时，先让插件后台续期或重新登录；不要手工改 `plugin-auth.json`。
- 只有「添加到 CLI」后仍不通，检查该 CLI 是否已被网关接管，以及它是否读取了正确的配置文件。

### 3.3 Windows 路径与 WSL Direct

- Windows 上路径比较不区分大小写；配置里统一使用正斜杠，运行时再转换。
- 当某个 CLI 的配置根被设置成 `\\wsl.localhost\...` 时，它的 Skills 目标也会解析为 WSL UNC 路径，但同步源仍然是中央仓库。
- 「WSL Direct」表示该 CLI 直接在 WSL 内运行，AI Toolbox 不应再通过 Windows 侧路径维护同一个目录。
- 遇到 `Path does not exist` 或 UNC 写入失败，先确认 WSL 发行版正在运行、路径存在，并且当前用户有权限。

### 3.4 配置路径自定义

- 每个 CLI 的根目录可以在对应 Tab 修改；Skills 的中央仓库路径在 `Skills` 设置里修改。
- 修改前先确认新路径可读写；路径迁移会做源/目标重叠校验，不允许把仓库复制到自身内部。
- 配置路径变更后，依赖它的 Skills/MCP 目标会重新解析；旧目标可能留下 warning，不会静默删除用户文件。
- 不要把中央仓库设置成某个工具的运行时目录，除非你明确知道两者是同一个物理目录。

### 3.5 备份与恢复的边界

- 备份不包含：正在运行的进程状态、系统凭据库明文、临时缓存、Git 克隆缓存。
- 恢复不会自动改写第三方工具的运行时目录；它恢复的是 AI Toolbox 管理的配置和元数据。
- 恢复后如果某个 Skill 显示来源 warning，按提示手动恢复源目录或重新安装，AI Toolbox 不会自动重建用户内容。
- 加密备份必须使用同一口令恢复；口令丢失无法找回。

## 4. 给 AI Agent 的操作原则

- 先读对应模块的 `AGENTS.md` 和当前源码，再改配置；不要凭记忆猜字段。
- 任何写工具目录的操作都要先确认「源是中央仓库 / 配置源」，不要把工具运行时目录当成源。
- 不要读取、打印或提交用户的 API Key、OAuth token、`plugin-auth.json` 明文。
- 修改配置前先备份目标文件；优先使用各 CLI 的原生写入 helper，保留未知字段、注释和顺序。
- 遇到不确定的行为，先在 UI 里复现并查看 Gateway 日志/请求明细，再定位到后端模块。

## 5. 参考文档

- 需要安装、使用或编写插件时：读 `references/plugins.md`。
- 插件类型、清单、Bridge API、中间件沙箱、Provider 宿主、市场 `registry.json` 和本地热调试都在该文件中。

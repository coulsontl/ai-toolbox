# 会话详情工作台重构计划

## 背景

本计划基于两部分源码研究：

- 参考项目：`D:\GitHub\claude-code-history-viewer`
- 当前项目：`D:\GitHub\ai-toolbox`

参考项目的会话详情体验是偏 IDE / Command Center 的工作台风格。你明确要求不要左侧 `ProjectTree`，所以本计划只迁移会话详情相关体验：

- 不新增、不模仿左侧 `ProjectTree`。
- 保留 AI Toolbox 现有会话列表、工具页和入口。
- 重构共享会话详情页，以及为了支撑该详情页所需的后端结构化消息契约。

当前 AI Toolbox 的会话管理详情由共享模块承载，覆盖 Claude Code、Codex、Gemini CLI、OpenCode、OpenClaw：

- `web/features/coding/shared/sessionManager/SessionManagerPanel.tsx`
- `web/features/coding/shared/sessionManager/types.ts`
- `web/features/coding/shared/sessionManager/sessionManagerApi.ts`
- `tauri/src/coding/session_manager/mod.rs`
- `tauri/src/coding/session_manager/claude_code.rs`
- `tauri/src/coding/session_manager/codex.rs`
- `tauri/src/coding/session_manager/gemini_cli.rs`
- `tauri/src/coding/session_manager/open_code.rs`
- `tauri/src/coding/session_manager/open_claw.rs`

因此实现入口应优先放在 shared session manager，而不是分别改五个工具页面。

## 目标

1. 把所有工具的会话详情页改成紧凑工作台风格：
   - 顶部命令栏
   - 中央消息查看器
   - 右侧消息导航器
   - sticky 搜索和过滤
   - 高密度消息时间线
   - 底部状态信息

2. 后端从纯文本消息升级为结构化消息：
   - 保留现有 `role`、`content`、`ts`
   - 增加可选字段：消息 id、父消息 id、消息类型、模型、usage、工具调用、工具结果、thinking、summary、元数据
   - 老导出文件继续可导入
   - 不破坏原生 snapshot 的恢复语义

3. 保留现有会话管理能力：
   - 列表
   - 详情
   - 导入
   - 导出
   - 重命名
   - 删除
   - 复制 resume command
   - `sourcePath` 身份语义
   - KeepAlive 可见性保护

4. 控制实现复杂度：
   - 不迁移到 Tailwind / Radix
   - 不照搬参考项目深色硬编码配色
   - 第一版不新增虚拟滚动依赖，除非真实大数据场景证明必须引入
   - 使用 AI Toolbox 当前 CSS 变量、Ant Design、lucide 图标约定

## 非目标

1. 不做左侧 `ProjectTree`。

2. 不替换当前会话列表页。

3. 不新增一套全局设计系统。

4. 不把详情页强制做成深色主题。

5. 不要求五个 provider parser 一次性达到完整功能等价。结构先打通，provider 能力逐步补齐。

6. 不破坏 `ai-toolbox.session-export.v2`。

7. 不把截图导出、消息选择模式、subagent 图谱导航作为第一版阻塞项。

## 目标交互结构

### 详情容器

继续使用现有详情 Modal 作为容器，但替换 Modal 内部结构。

桌面端推荐结构：

```text
+----------------------------------------------------------------+
| 顶部命令栏：标题、路径、搜索、过滤、重命名、导出、删除          |
+---------------------------------------------+------------------+
| 中央消息查看器                                | 右侧消息导航器   |
| - 日期分割线                                  | - 用户轮次       |
| - 结构化消息卡片                              | - assistant 回复 |
| - 工具调用卡片                                | - 工具调用       |
| - copy / expand 控制                          | - 搜索命中       |
+---------------------------------------------+------------------+
| 底部状态栏：总消息数、可见消息数、sourcePath、最后活跃时间      |
+----------------------------------------------------------------+
```

响应式规则：

- `>= 1100px`：中央消息查看器 + 右侧消息导航器。
- `< 1100px`：右侧导航器收起为 Drawer / Popover。
- `< 720px`：顶部命令栏允许换成两行；按钮尽量使用图标；危险操作保留清晰文字。

### 顶部命令栏

命令栏应包含：

- 关闭 / 返回按钮
- 会话标题
- 短 session id
- 项目路径
- 详情内搜索输入框
- 上一条 / 下一条搜索命中
- role 过滤
- content type 过滤
- 复制 resume command
- 重命名
- 导出
- 删除

实现要求：

- 常用轻操作优先用图标按钮。
- 有歧义或危险的动作保留文字，例如删除、导出。
- 搜索框和过滤器应 sticky，滚动消息时仍可操作。

### 搜索与过滤

新增详情内状态：

- `detailQuery`
- `activeMatchIndex`
- `messageRoleFilter`
- `messageContentFilter`
- `showNavigator`

role 过滤第一版支持：

- 全部
- user
- assistant
- system
- tool

content type 过滤第一版支持：

- 全部
- text
- tool call
- tool result
- thinking
- summary

搜索匹配范围：

- `content`
- text block
- summary block
- tool name
- tool input
- tool output

搜索行为：

- 搜索命中高亮。
- 显示命中数量。
- 上一条 / 下一条跳转到对应消息。
- 没有命中时，在消息区域显示紧凑 empty state，不整页空白。

### 中央消息查看器

消息区改成 timeline 风格：

- 跨日期插入日期分割线。
- user 消息桌面端右对齐。
- assistant / system / tool 消息左对齐。
- 消息头显示 role、时间、模型、usage、费用、耗时等可用元数据。
- 消息正文按 block 渲染。
- 长消息支持展开 / 折叠。
- 每条消息支持复制。
- 右下角提供滚动到顶部 / 底部按钮。

第一版 block renderer 必须支持：

- text
- thinking
- redacted_thinking
- tool_call
- tool_result
- tool_execution
- summary
- system
- command
- image
- document
- unknown fallback

消息展示规则：

- user：
  - 桌面端右对齐。
  - 使用强调色气泡，但必须来自当前主题 token，不硬编码参考项目的 accent 色。
  - 最大宽度约 `85%`，避免横跨整行。
  - 默认显示 3 行预览，长消息显示展开 / 收起。
  - hover 时显示复制按钮。
  - 搜索时禁用 markdown 渲染，改用纯文本高亮，避免 highlight 被 markdown tree 切碎。

- assistant：
  - 左对齐。
  - 使用次级 surface 气泡或轻边框容器。
  - 最大宽度大于 user，桌面端约 `min(780px, 95%)`。
  - 支持 Markdown + GFM，包含列表、表格、代码块。
  - 表格默认最多显示 2 行 body，超出后用“显示更多行”按钮展开。
  - 长文本默认显示 3 行预览，搜索时自动展开匹配内容。
  - hover 时显示复制按钮。

- system：
  - 使用低强调中性色或 warning/info 语义色。
  - 不用聊天气泡，使用横向提示条 / renderer card。
  - 如果 system content 包含 command 标签，应交给 command renderer。

- summary：
  - 使用 compact summary card。
  - 标题固定为“会话摘要”或 provider 原始 summary label。
  - 内容参与 navigator 和搜索。

- thinking：
  - 默认折叠。
  - header 包含 Bot / Brain 类图标、标题、首行预览。
  - 命中搜索时自动展开。
  - 使用专门的 thinking surface，不与普通 assistant 文本混在同一个气泡里。

- redacted_thinking：
  - 使用 neutral card。
  - 显示“推理内容已隐藏 / redacted”说明。
  - 如果有 redacted 数据，只显示短预览，不完整展开敏感/不可读 payload。

- command：
  - 解析 `<command-name>`、`<command-message>`、`<command-args>`、`<local-command-stdout>`、`<local-command-caveat>`、`stdout` / `stderr` 类标签。
  - command name 作为卡片标题。
  - args 和 message 作为折叠内容。
  - stdout 使用 success/result 样式。
  - stderr 使用 error 样式。
  - caveat 使用 info 样式并默认折叠。

- image：
  - 如果 content 是图片 URL 或 base64 data URL，渲染图片预览。
  - 图片必须设置最大宽度、圆角和 alt。
  - 图片后剩余文本继续走文本 renderer。

- unknown fallback：
  - 不丢数据。
  - 对 string 用可折叠文本 / markdown fallback。
  - 对 object 用 JSON preview，默认折叠。

### 工具调用卡片

工具调用和结果必须尽量统一成一张“动作 + 目标 + 结果”的卡片，而不是把 tool call 和 tool result 分散成两条普通消息。

统一卡片结构：

- 紧凑 header
- 工具图标
- 工具名称
- 状态
- 可选耗时
- 可折叠 body
- monospaced input / output
- tool id badge
- result status badge
- error 状态边框和标题语义

通用尺寸和层级：

- renderer card 外层半径建议 `6px` 到 `8px`。
- header 最小高度约 `32px`，但移动端点击区不小于 `44px`。
- header padding 约 `10px 6px` 或等效 AntD token。
- body padding 约 `10px`。
- 图标统一 `16px`，辅助状态图标 `12px`。
- 标题、正文、meta 文本保持 `12px` 到 `13px`，不得低于 12px。
- code block 最大高度第一版建议 `256px`；内容型结果最大高度建议 `384px`。
- 卡片之间垂直间距保持 `6px` 到 `8px`，避免松散。
- 所有颜色映射到 AI Toolbox 主题变量，不能直接复制 `bg-tool-*` 这套 Tailwind 类。

建议在 AI Toolbox 中实现 renderer variant class：

- `terminal`：Bash / shell / command execution
- `code`：Read / Edit / MultiEdit / ApplyPatch / NotebookEdit
- `file`：Glob / file list / file content
- `search`：Grep / search result
- `task`：TodoWrite / update_plan / Task / Agent
- `web`：WebFetch / WebSearch
- `mcp`：MCP tool use / result
- `document`：Document / citation / file-like rich result
- `system`：system result / code execution result
- `thinking`：thinking block
- `success` / `warning` / `error` / `neutral`：状态类结果

这些 variant 只定义语义色和 surface 层级，不引入新的全局 palette。

### 逐工具展示规格

第一版必须按下面的规格实现，而不是简单打印 JSON。

#### Bash / Terminal

参考文件：

- `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\BashToolRenderer.tsx`
- `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\BashCard.tsx`

展示规则：

- variant：`terminal`
- icon：`Terminal`
- title：终端 / Terminal
- header 右侧：
  - `run_in_background` 显示 background badge
  - `timeout` 显示秒数，例如 `30s`
  - 有结果时显示 success / warning / error status badge
- body：
  - `description` 在 command 上方用 muted text 显示。
  - command 使用终端代码块，不展示为普通 JSON。
  - command block 顶部可加一条小 header：Play 图标 + “Command”。
  - 使用 monospace，保留换行和横向滚动。
- result：
  - 如果有 `stdout`，用 output block 显示，支持 ANSI。
  - 如果有 `stderr`，用 error/warning block 显示，支持 ANSI。
  - 如果有 `return_code`，header 或 result 右侧显示 exit code badge。
  - 没有输出时显示 muted italic 的“无输出 / no output”。

#### Read

展示规则：

- variant：`code`
- icon：`FileText`
- title：读取文件 / Read
- body：
  - `file_path` 是主视觉对象，用 path card 显示。
  - `offset` / `limit` 显示为行范围或 meta chip。
  - path 必须 `break-all`，避免长路径撑破布局。
- result：
  - 如果结果是 file content，显示文件内容 renderer。
  - 文件内容 header 显示文件名、短路径、起始行、展示行数。
  - 长内容折叠，搜索命中自动展开。

#### Write

展示规则：

- variant：优先 `success`，有错误时切到 `error`
- icon：`FileText` / `FilePlus`
- title：写入文件 / Write
- body：
  - `file_path` path card。
  - `content` 默认折叠，只在 summary 中显示行数，例如 `128 lines`。
  - 展开后显示 code block。
- result：
  - 成功显示 success status。
  - 失败显示 error card，不能只在正文里显示字符串。

#### Edit / MultiEdit

展示规则：

- variant：`code`
- icon：`Edit` / `FileEdit`
- title：
  - Edit：编辑文件
  - MultiEdit：批量编辑
- body：
  - `file_path` path card。
  - MultiEdit header 右侧显示 edit count。
  - 每个 edit 使用 diff renderer，不用纯 JSON。
  - `old_string` / `new_string` 以 before/after 或 diff 形式展示。
  - `replace_all` 显示为 replace all badge 或 meta row。
- result：
  - 如果 provider 返回 `originalFile` / `userModified`，显示 edit type 和 user modified 两个小 meta block。
  - 有完整文件内容时可折叠显示原文件或变更后文件。

#### ApplyPatch

展示规则：

- variant：`code`
- icon：`FileCode2` / `PencilLine`
- title：文件补丁 / Patch
- header 右侧显示 patch 行数。
- body：
  - `patch` 使用 diff syntax code block。
  - max height 比普通 command 略高，建议 `28rem` 上限。
  - 长 patch 可滚动，不撑高整个 Modal。

#### NotebookEdit

展示规则：

- variant：`code`
- icon：`BookOpen`
- title：Notebook Edit
- header 右侧：
  - `edit_mode` badge
  - `cell_type` badge
- body：
  - `notebook_path` path card。
  - `cell_number`、`cell_id` 显示为 meta row。
  - `new_source` 根据 `cell_type` 用 markdown 或 python code block。

#### Grep

展示规则：

- variant：`search`
- icon：`FileSearch` / `Search`
- title：搜索文本 / Grep
- header 右侧：
  - `output_mode` badge
  - result status badge
- body：
  - `pattern` 是主字段，必须突出显示。
  - `path`、`glob`、`type` 显示为 property row。
  - `-i`、`-n`、`-A`、`-B`、`-C`、`multiline` 显示为 flags chips。
  - `head_limit` 显示为 limit property。
- result：
  - 搜索结果如果是文本，用 markdown/string renderer。
  - 如果结果能解析成文件列表或匹配列表，按文件/行分组显示。

#### Glob

展示规则：

- variant：`file`
- icon：`FolderSearch`
- title：查找文件 / Glob
- body：
  - `pattern` property row。
  - `path` property row。
- result：
  - 文件列表 renderer。
  - 每项第一行显示文件名，第二行显示目录。
  - 文件名和目录都参与搜索高亮。

#### WebFetch

展示规则：

- variant：`web`
- icon：`Globe`
- title：网页读取 / WebFetch
- body：
  - `url` 使用 URL card，带 ExternalLink 图标。
  - `prompt` 单独显示为 prompt card，保留换行。
- result：
  - 网页内容走 markdown/string renderer。
  - URL 必须允许换行或 `break-all`，不能撑破 navigator 或 message viewer。

#### WebSearch

展示规则：

- variant：`web`
- icon：`Search`
- title：网页搜索 / WebSearch
- body：
  - `query` 是主字段，用 medium text 显示。
  - `allowed_domains` 显示 success/info chips。
  - `blocked_domains` 显示 danger chips。
- result：
  - 每条结果应显示标题、URL、摘要。
  - 如果数据不足，fallback 到 markdown/string renderer。

#### TodoWrite

展示规则：

- variant：`task`
- icon：`ListTodo`
- title：任务列表 / TodoWrite
- header 右侧显示 item count。
- body：
  - 每个 todo 一行 compact card。
  - `completed` 用 CheckCircle。
  - `in_progress` 用 Loader / progress icon。
  - 其他状态用 Circle。
  - `priority=high` 用 danger badge。
  - `priority=medium` 默认不强调。

#### update_plan

展示规则：

- variant：`task`
- icon：`ListChecks`
- title：计划更新 / Update Plan
- header 右侧显示 task count。
- body：
  - `explanation` 作为顶部说明 card。
  - `plan` 每一步单独一行 / 小卡。
  - status 显示图标和文本：
    - pending
    - in_progress
    - completed
    - deleted
  - 当前进行中的 step 应比 pending 更醒目，但不要用整块强色背景。

#### Task / Agent / Subagent

展示规则：

- variant：`task`，如果存在 `subagent_type`，允许按 subagent 类型增加轻微 tint。
- icon：`MessageSquare` / `Bot`
- title：Task / Agent
- header 右侧：
  - `subagent_type` badge
  - `run_in_background` badge
  - `model` monospace meta
  - `isolation` badge
- body：
  - `description` 是醒目的摘要块。
  - `prompt` 默认折叠，折叠时显示首行预览。
  - 展开后 prompt 按 markdown 渲染。
- result：
  - result markdown 单独显示。
  - 如果能定位子会话，提供“查看子会话”入口；第一版可以先不做子会话图谱。

#### MCP Tool Use / Result

展示规则：

- variant：`mcp`
- icon：`Server`
- title：MCP Tool
- body：
  - 显示 `serverName / toolName`，前面放 `Wrench` 图标。
  - input 默认折叠。
  - 展开 input 后显示 formatted JSON。
- result：
  - error：error variant，展示错误文本。
  - text：pre-wrap 文本块。
  - image：图片预览。
  - resource：显示 resource URI。
  - object fallback：formatted JSON。

#### Code Execution Result / Terminal Result

展示规则：

- variant：成功用 `system` 或 `success`，错误用 `error`。
- header：
  - 成功显示 CheckCircle。
  - 警告/失败显示 AlertCircle。
  - `return_code` 显示 exit code badge。
- body：
  - `stdout` 单独输出块。
  - `stderr` 单独错误块。
  - 支持 ANSI。
  - 无输出显示 muted italic。

#### File List Result

展示规则：

- variant：`file`
- header 显示文件数量。
- body：
  - 每个文件是 compact row。
  - 第一行文件名。
  - 第二行目录。
  - 长路径截断但提供 title。
  - 搜索命中高亮。

#### String / Markdown Result

展示规则：

- 如果内容像文件树或含 ANSI，使用 monospace pre-wrap。
- 否则走 Markdown + GFM。
- 超过 15 行默认折叠。
- 折叠时显示“还剩 N 行”入口。
- 搜索命中时自动展开。

#### Unknown / Default Tool

展示规则：

- 根据 tool name 映射最接近的 variant。
- header 显示工具名、图标、状态、tool id。
- input 用 `<details>` 或自渲染 collapse，summary 显示 input keys。
- 展开后 formatted JSON。
- result 走通用 result router。

### 工具调用与结果配对语义

后端和前端都要支持配对：

- 优先把同一个 `tool_id` 的 `tool_call` 和 `tool_result` 组合成一个 `tool_execution` block。
- 如果 parser 无法安全合并，则保留 `tool_call` 和 `tool_result` 两个 block，但二者必须共享 `toolId`。
- 前端渲染时先按 `toolId` 尝试合并，再 fallback 分开显示。
- 合并后卡片标题表达动作，body 表达输入目标，result 表达执行结果。
- 如果只有 tool call 没有 result，显示 pending。
- 如果只有 result 没有 tool call，显示 result-only card，不丢内容。

这条是实现参考项目风格的关键点。否则 Bash / Read / Grep 等工具会变成两段割裂内容，视觉上不像参考项目。

### 右侧消息导航器

右侧 navigator 是本次替代 `ProjectTree` 的核心导航结构。

导航条目来源：

- user 消息
- assistant 消息
- tool call
- summary
- 搜索命中

预览内容优先级：

1. summary block
2. 第一段 text block
3. tool name + primary argument
4. fallback `content`

交互：

- 点击条目滚动到消息。
- 当前视口附近消息高亮。
- role / content filter 同步影响 navigator。
- 搜索命中显示 badge。

### 导出 / 捕获

参考项目有 capture mode。AI Toolbox 第一版不把它作为必做项。

第一版保留：

- 当前 JSON 导出
- 复制单条消息
- 复制 resume command

第二版可选：

- 选择多条消息
- 复制选中消息为 Markdown
- 截图 / 图片导出

## 后端数据契约

### 当前问题

当前后端 `SessionMessage` 过于扁平：

```rust
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub ts: Option<i64>,
}
```

这会导致前端无法稳定区分：

- 工具调用
- 工具结果
- thinking
- summary
- usage
- model
- cost
- duration
- parent / child 关系

所以后端需要一起改。

### 推荐 Rust 类型

保留现有字段，同时添加可选结构化字段。

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub ts: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_type: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<SessionMessageBlock>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionMessageUsage>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_sidechain: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageBlock {
    pub kind: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_tool_name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
}
```

关键点：

- `role/content/ts` 不删，保证旧前端逻辑和导出可读性。
- 新字段全部可选，老数据不会出错。
- `blocks` 默认空数组。
- `content` 仍然是 flatten 后的人类可读文本，用于搜索、预览、导出和 fallback。
- `kind` 是结构语义，不是视觉类名；推荐值包括 `text`、`thinking`、`redacted_thinking`、`summary`、`system`、`command`、`tool_call`、`tool_result`、`tool_execution`、`image`、`document`、`unknown`。
- `variant` 只是 display hint；前端可以忽略或重新映射，推荐值包括 `terminal`、`code`、`file`、`search`、`task`、`web`、`mcp`、`document`、`system`、`thinking`、`success`、`warning`、`error`、`neutral`。
- `language` 用于 code/diff/markdown/bash/python/json 等渲染提示。
- `tool_name` 保留 provider 原始工具名，用于展示和排查。
- `normalized_tool_name` 是跨 CLI 的稳定路由键，用于把 `bash` / `shell` / `execute_command` 这类 provider 差异归一到同一个 renderer。
- `is_error` 用于 error border、error icon 和 status badge。
- `input` 保留工具调用参数。
- `output` 保留工具结果或合并后的结果。
- `metadata` 只存无法标准化但对调试/渲染有帮助的信息，不把所有 raw payload 全量塞进去。

`metadata` 中允许的高价值字段示例：

- `timeout`
- `runInBackground`
- `returnCode`
- `stdout`
- `stderr`
- `filePath`
- `lineCount`
- `offset`
- `limit`
- `outputMode`
- `allowedDomains`
- `blockedDomains`
- `subagentType`
- `model`
- `isolation`

不建议把完整 provider raw event 全量塞进 `metadata`。如果确实需要排查，优先只放和渲染/调试有关的小字段。

### 推荐 TypeScript 类型

```ts
export interface SessionMessageUsage {
  inputTokens?: number;
  outputTokens?: number;
  cacheCreationInputTokens?: number;
  cacheReadInputTokens?: number;
}

export interface SessionMessageBlock {
  kind: string;
  text?: string;
  title?: string;
  variant?: string;
  language?: string;
  toolId?: string;
  toolName?: string;
  normalizedToolName?: string;
  status?: string;
  isError?: boolean;
  input?: unknown;
  output?: unknown;
  metadata?: unknown;
}

export interface SessionMessage {
  role: string;
  content: string;
  ts?: number;
  id?: string;
  parentId?: string;
  messageType?: string;
  blocks?: SessionMessageBlock[];
  model?: string;
  usage?: SessionMessageUsage;
  durationMs?: number;
  costUsd?: number;
  isSidechain?: boolean;
  metadata?: unknown;
}
```

### 导出兼容策略

第一版继续使用 `ai-toolbox.session-export.v2`。

理由：

- 这次是给 `normalizedMessages` 添加可选字段，不是破坏性语义变化。
- 旧 v2 文件没有新字段，`#[serde(default)]` 可以正常导入。
- 新 v2 文件包含结构化字段，也仍然是 normalized message 的扩展。
- 真正恢复会话时，native snapshot 仍是权威来源。

只有当后续必须改变导出语义时，才升级到 v3。本计划不需要升级 v3。

## 后端 Provider Parser 策略

### 共享 helper

新增小型 helper，位置可选：

- `tauri/src/coding/session_manager/mod.rs`
- 或 `tauri/src/coding/session_manager/message_blocks.rs`

建议 helper：

- `text_block(text)`
- `tool_call_block(tool_id, tool_name, input)`
- `tool_result_block(tool_id, tool_name, output, status)`
- `tool_execution_block(tool_id, tool_name, input, output, status)`
- `thinking_block(text)`
- `summary_block(text)`
- `flatten_blocks_for_content(blocks)`
- `pair_tool_blocks(blocks)`

原则：

- helper 保持 provider-neutral。
- 不提前抽象复杂 renderer。
- 每个 parser 仍能按自身数据结构做最小转换。
- tool pairing helper 只按明确 `tool_id` 合并，不猜测不相邻、无 id 或语义不明的结果。

### Claude Code

文件：

- `tauri/src/coding/session_manager/claude_code.rs`

任务：

- 保留 uuid / message id。
- 保留 parent uuid。
- 解析 content array：
  - `text` -> text block
  - `thinking` -> thinking block
  - `tool_use` -> tool_call block
  - `tool_result` -> tool_result block
  - unknown -> fallback block
- 保留 model、usage、stop reason、cost、duration 等字段。
- `content` 继续生成可读 flatten 文本。

### Codex

文件：

- `tauri/src/coding/session_manager/codex.rs`

任务：

- 保留 event / item id。
- user / assistant 文本保持 text block。
- function call / tool call 转成 `tool_call` block。
- function output / tool output 转成 `tool_result` block。
- 如果 payload 有 model / usage，写入标准字段。
- 现有 `[Tool: name]` 文案只作为 fallback，不作为主结构。

### Gemini CLI

文件：

- `tauri/src/coding/session_manager/gemini_cli.rs`

任务：

- 保留现有 array content 顺序。
- `functionCall` 转成 `tool_call` block。
- `functionResponse` 转成 `tool_result` block。
- 有 model / usage 时写入标准字段。
- 当前已有 tool call 测试，应扩展断言 blocks。

### OpenCode

文件：

- `tauri/src/coding/session_manager/open_code.rs`

任务：

- JSON storage 路径：
  - 解析 message parts
  - 尽量生成 text/tool blocks
  - 保留官方 export 相关字段

- SQLite storage 路径：
  - 将 message rows 映射到同一结构化 block shape
  - 保持现有 official export 行为

- 不修改 OpenCode native import/export 语义，除非测试证明必须改。

### OpenClaw

文件：

- `tauri/src/coding/session_manager/open_claw.rs`

任务：

- 保留现有文本行为。
- 至少生成 text block。
- 只有当源数据明确暴露工具结构时，再生成 tool block。

## 前端实现计划

### 步骤 1：扩展类型和纯工具函数

文件：

- `web/features/coding/shared/sessionManager/types.ts`
- `web/features/coding/shared/sessionManager/utils.ts`

任务：

- 新增 `SessionMessageBlock`。
- 新增 `SessionMessageUsage`。
- 扩展 `SessionMessage`。
- 新增工具函数：
  - `getMessageBlocks(message)`
  - `getMessageKind(message)`
  - `getMessageSearchText(message)`
  - `getMessagePreview(message)`
  - `getMessagePrimaryToolLabel(message)`
  - `buildNavigatorEntries(messages, filters, query)`
  - `groupMessagesWithDateDividers(messages)`
- 暂时保留现有 `buildSessionTocItems`，等 navigator 完全接管后再移除。

### 步骤 2：拆分详情组件

新增目录：

- `web/features/coding/shared/sessionManager/detail/`

建议组件：

- `SessionDetailWorkbench.tsx`
- `SessionDetailCommandBar.tsx`
- `SessionMessageViewer.tsx`
- `SessionMessageCard.tsx`
- `SessionMessageBlockRenderer.tsx`
- `SessionRendererCard.tsx`
- `SessionToolExecutionCard.tsx`
- `SessionToolResultBlock.tsx`
- `SessionMessageNavigator.tsx`
- `SessionDetailStatusBar.tsx`
- `SessionSearchHighlight.tsx`

建议 renderer 子目录：

- `detail/renderers/MessageTextBlock.tsx`
- `detail/renderers/ThinkingBlock.tsx`
- `detail/renderers/CommandBlock.tsx`
- `detail/renderers/ToolExecutionBlock.tsx`
- `detail/renderers/ToolResultRouter.tsx`
- `detail/renderers/tools/BashToolCard.tsx`
- `detail/renderers/tools/ReadToolCard.tsx`
- `detail/renderers/tools/WriteToolCard.tsx`
- `detail/renderers/tools/EditToolCard.tsx`
- `detail/renderers/tools/SearchToolCard.tsx`
- `detail/renderers/tools/WebToolCard.tsx`
- `detail/renderers/tools/TodoToolCard.tsx`
- `detail/renderers/tools/TaskToolCard.tsx`
- `detail/renderers/tools/McpToolCard.tsx`
- `detail/renderers/tools/DefaultToolCard.tsx`

原则：

- 保持组件职责清晰。
- 不在 `SessionManagerPanel.tsx` 里继续堆大块 JSX。
- 可以做一个轻量 `SessionRendererCard` 统一外壳、header、collapse、status badge、variant class。
- 不做过度泛化的 renderer 框架；具体工具卡片先按上面的展示矩阵实现。
- `SessionToolExecutionCard` 负责按 tool name 分发到具体工具卡片。
- `ToolResultRouter` 负责 string、file list、terminal stdout/stderr、MCP result、unknown JSON 的 result 展示。
- `SessionSearchHighlight` 负责全局一致的搜索高亮和 current match 标记，不要每个 renderer 自己写一套。

### 步骤 3：替换详情 Modal 内容

文件：

- `web/features/coding/shared/sessionManager/SessionManagerPanel.tsx`

任务：

- 保持列表区域逻辑不变。
- 保持详情加载逻辑不变。
- 保持导入、导出、重命名、删除、复制 resume command 的 handler。
- 将当前 inline detail modal body 替换为 `SessionDetailWorkbench`。
- 保持 Modal open/close 生命周期。
- 保持 request id 和 KeepAlive visible context guard。
- 重命名后继续刷新 detail 和 list。

### 步骤 4：样式

文件：

- `web/features/coding/shared/sessionManager/SessionManagerPanel.module.less`
- 或新增 `web/features/coding/shared/sessionManager/detail/SessionDetailWorkbench.module.less`

任务：

- 如果样式继续膨胀，优先新建 detail 专属 module。
- Modal chrome 不做大面积重写；主要控制 body 内部 workbench。
- 详情 workbench 采用稳定 grid：
  - 顶部 command bar 固定高度或允许双行 wrap。
  - 中央 viewer `min-width: 0`。
  - 右侧 navigator 固定宽度，建议 `260px` 到 `300px`。
  - 底部 status bar 高度稳定。
- 全部颜色使用：
  - `var(--color-bg-container)`
  - `var(--color-bg-elevated)`
  - `var(--color-border)`
  - `var(--color-text-primary)`
  - `var(--color-text-secondary)`
  - `var(--color-text-tertiary)`
- 少量使用已有风格里的 `color-mix()`。
- 不硬编码白/黑/深 slate 背景。
- 不使用负 letter spacing。
- 不做 card 套 card。
- 右侧 navigator 固定宽度，避免内容变化导致布局抖动。
- 消息 action button 使用固定尺寸图标按钮。
- 增加响应式断点。
- renderer card 视觉要求：
  - 外层 border 明确但低对比。
  - header 和 body 背景同属一个 surface，不做断裂的嵌套卡。
  - hover 只增强可点击区域，不改变尺寸。
  - collapse chevron 旋转，不移动标题。
  - status badge、tool id badge 高度稳定。
  - 代码块使用 monospace，横向滚动，不让长命令撑宽 Modal。
  - stdout/stderr/error 块要有清晰语义差异，不能只靠文字。
- 消息气泡视觉要求：
  - user / assistant 宽度、对齐、背景必须明显不同。
  - user 气泡不能过亮到破坏暗色主题对比。
  - assistant markdown 的 code、blockquote、table 要有可读边界。
  - copy 按钮 hover 出现，但 keyboard focus 时也必须可见。
- navigator 视觉要求：
  - 当前消息高亮使用左侧细条或背景，不只改文字颜色。
  - 搜索命中显示小 badge。
  - 条目高度稳定，长预览两行内截断。
  - tool call 条目显示工具图标和 tool name。
- 动效要求：
  - collapse/expand 150-220ms。
  - chevron transform/opacity 过渡即可，不动画 height 到导致卡顿。
  - 遵守 `prefers-reduced-motion`。

### 步骤 4.1：视觉验收清单

实现后必须逐项检查：

- Bash 命令不是 JSON，而是终端命令卡片。
- Bash stdout/stderr 分区显示，stderr 不和 stdout 混在一起。
- Read 显示路径和行范围，不只是 `{"file_path": ...}`。
- Edit/MultiEdit 显示 diff，不是 old/new 字段列表。
- Grep 显示 pattern、scope、flags、output mode。
- Glob result 显示文件名 + 目录的列表。
- WebFetch 显示 URL card 和 prompt card。
- WebSearch 显示 query 和 allow/block domain chips。
- TodoWrite 显示状态图标和 priority。
- update_plan 显示每个 step 的状态。
- MCP 显示 `server/tool`，input 默认折叠。
- Unknown tool 才 fallback JSON。
- 亮色和暗色主题下，所有 card border、badge、文字对比可读。
- 375px 宽度下命令栏不遮挡、不横向滚动。
- 长 path、长 URL、长 command 不撑破 Modal。

### 步骤 5：i18n

先搜索现有 key，再补缺失 key。

可能新增：

- `sessionManager.searchInDetail`
- `sessionManager.previousMatch`
- `sessionManager.nextMatch`
- `sessionManager.messageNavigator`
- `sessionManager.filterAll`
- `sessionManager.filterUser`
- `sessionManager.filterAssistant`
- `sessionManager.filterSystem`
- `sessionManager.filterTool`
- `sessionManager.filterText`
- `sessionManager.filterToolCalls`
- `sessionManager.filterToolResults`
- `sessionManager.filterThinking`
- `sessionManager.filterSummary`
- `sessionManager.visibleMessages`
- `sessionManager.totalMessages`
- `sessionManager.noMatchingMessages`
- `sessionManager.scrollToTop`
- `sessionManager.scrollToBottom`
- `sessionManager.copyMessage`
- `sessionManager.copyBlock`
- `sessionManager.toolInput`
- `sessionManager.toolOutput`
- `sessionManager.pendingResult`
- `sessionManager.exitCode`
- `sessionManager.stdout`
- `sessionManager.stderr`
- `sessionManager.command`
- `sessionManager.filePath`
- `sessionManager.pattern`
- `sessionManager.flags`
- `sessionManager.allowDomains`
- `sessionManager.blockDomains`
- `sessionManager.todoCount`
- `sessionManager.planStepCount`
- `sessionManager.showInput`
- `sessionManager.hideInput`
- `sessionManager.showMoreLines`
- `sessionManager.collapse`
- `sessionManager.expand`

要求：

- 中文和英文 locale 都要补。
- 不只改当前显示语言。

## 后端实现计划

### 步骤 1：扩展 Rust 结构体

文件：

- `tauri/src/coding/session_manager/mod.rs`

任务：

- 新增 `SessionMessageBlock`。
- 新增 `SessionMessageUsage`。
- 扩展 `SessionMessage`。
- 给新增字段添加 `#[serde(default)]` 和 `skip_serializing_if`。
- 修复所有手动构造 `SessionMessage` 的编译错误。

### 步骤 2：新增 parser helper

文件：

- `tauri/src/coding/session_manager/mod.rs`
- 或 `tauri/src/coding/session_manager/message_blocks.rs`

任务：

- 添加 text/tool/thinking/summary block helper。
- 添加 flatten helper。
- helper API 保持小而明确。

### 步骤 3：逐个 provider parser 升级

推荐顺序：

1. Claude Code
2. Codex
3. Gemini CLI
4. OpenCode
5. OpenClaw

理由：

- Claude Code 和 Codex 的结构化消息收益最高。
- Gemini CLI 当前已有工具调用相关测试，适合快速扩展。
- OpenCode 的 native import/export 更敏感，需要在前面结构稳定后谨慎处理。
- OpenClaw 可以先做 text block，工具结构后补。

### 步骤 4：导出 / 导入兼容

文件：

- `tauri/src/coding/session_manager/mod.rs`
- provider native snapshot 相关模块

任务：

- 保持 schema v2。
- 验证旧 v2 导出文件无新字段时仍可导入。
- 验证新 v2 导出文件包含 structured `normalizedMessages`。
- 不改 native snapshot payload，除非 provider 恢复链路要求。
- OpenCode 需要重点跑 official export / raw snapshot 相关测试。

### 步骤 5：后端测试

新增或扩展测试：

- Claude Code parser 能保留 text、tool_use、tool_result blocks。
- Claude Code parser 能把同 id 的 tool_use/tool_result 配成 `tool_execution` 或可被前端配对的数据。
- Codex parser 能保留 function call、function output blocks。
- Gemini parser 能保留 functionCall、functionResponse blocks。
- OpenCode 原有 export/import 测试继续通过。
- 旧 v2 exported file 缺少新字段仍可导入。
- 新 v2 exported file 带 blocks 后仍可导入。

## 前端测试计划

新增或扩展测试：

- `getMessageSearchText` 包含 text、tool input、tool output。
- navigator entries 包含 user turn 和 tool call。
- role filter 能正确隐藏 / 显示消息。
- content filter 能正确隐藏 / 显示 block 类型。
- unknown block 走 fallback renderer。
- 没有 blocks 的旧消息仍按 `content` 正常渲染。
- tool pairing 能把相同 `toolId` 的 call/result 合并成一个 execution item。
- Bash block 选择 Bash renderer，而不是 default JSON renderer。
- Read/Edit/Grep/Web/Todo/MCP 至少各有一个 renderer routing 测试。
- 搜索命中能覆盖 tool input 和 tool output。

测试文件应放到 `web/test/` 下，并镜像功能目录。

视觉测试以手动 smoke 为主，自动化先覆盖 routing 和纯函数。若后续引入 Playwright，再补详情页截图回归。

## 执行顺序

1. 确认本计划。

2. 后端数据模型：
   - 扩展 Rust struct
   - 添加 helper
   - 修复 struct 初始化编译错误

3. 后端 parser：
   - Claude Code
   - Codex
   - Gemini CLI
   - OpenCode
   - OpenClaw

4. 后端测试：
   - parser targeted tests
   - session manager export/import targeted tests

5. 前端类型和工具函数：
   - 扩展 TS types
   - 添加 message/navigator utilities
   - 添加纯函数测试

6. 前端组件：
   - 创建 detail workbench components
   - 接入 `SessionManagerPanel`
   - 保持列表和批量操作不变

7. 样式：
   - workbench shell
   - command bar
   - message cards
   - tool cards
   - navigator
   - responsive states

8. i18n：
   - 补中文和英文 key
   - 检查文案和按钮是否溢出

9. 自动验证：
   - 先跑 targeted Rust tests
   - 再跑 targeted frontend tests
   - 跑 `pnpm test`
   - 跑 `cd tauri && cargo test`
   - 跑 `pnpm exec tsc --noEmit`
   - 如果前端入口、共享组件或构建链路受影响，再跑 `pnpm build`

10. 手动 smoke check：
   - Claude Code 会话详情
   - Codex 会话详情
   - Gemini CLI 会话详情
   - OpenCode 会话详情
   - OpenClaw 会话详情
   - 详情内重命名
   - 详情内导出
   - 详情内删除
   - 导入后打开详情
   - 搜索 / 过滤 / navigator
   - 亮色和暗色主题
   - 窄屏布局

## 风险点

1. 导出兼容

   新字段必须全部可选，否则旧导出文件会导入失败。

2. OpenCode native export/import

   OpenCode 有 official export 和 raw snapshot fallback，不要为了 UI 展示破坏恢复语义。

3. `content` fallback

   即使有 blocks，`content` 仍必须可读。搜索、预览、导出、旧 UI fallback 都依赖它。

4. KeepAlive 可见性

   详情刷新、重命名、删除、导入、导出都必须保留当前 visible-context 保护。隐藏页面不能向全局 UI 吐旧提示。

5. 大会话性能

   第一版先不新增虚拟化依赖。如果真实大会话卡顿，再做专门的 message virtual list。

6. 视觉照搬风险

   参考项目是深色、Tailwind-heavy 风格。AI Toolbox 只能迁移布局和交互模型，不能硬搬配色。

7. 范围膨胀

   capture mode、图片导出、provider-specific rich renderer、subagent 图谱导航都不应该阻塞第一版。

## 验收标准

1. 五个工具页面都通过现有会话管理入口打开新版详情工作台。

2. 新版详情中不存在左侧 `ProjectTree`。

3. 详情页包含：
   - 顶部命令栏
   - 中央消息查看器
   - 桌面端右侧 navigator
   - 窄屏 navigator fallback
   - 搜索命中数量和上一条 / 下一条
   - role 过滤
   - content type 过滤
   - 日期分割线
   - copy 操作
   - 现有重命名 / 导出 / 删除 / resume 操作
   - 逐工具 renderer，不把常见工具全部退化为 JSON

4. 后端在源数据支持时返回结构化消息字段。

5. 老的纯文本消息仍能正常显示。

6. 旧导出文件仍能导入。

7. 新导出文件能保留 structured `normalizedMessages`。

8. 现有 list/detail/import/export/rename/delete 行为不退化。

9. UI 在亮色和暗色主题下都可读，不依赖硬编码背景/文字颜色。

10. Bash、Read、Write、Edit、MultiEdit、ApplyPatch、Grep、Glob、WebFetch、WebSearch、TodoWrite、update_plan、Task/Agent、MCP、unknown fallback 的展示方式符合本文逐工具规格。

11. 交付时明确报告跑过哪些验证命令、哪些通过、哪些失败、失败是否与本轮相关。

## 建议提交边界

如果需要拆 commit，建议分成两个：

1. 后端结构化消息和测试。
2. 前端工作台 UI 和测试。

如果最终只做一个 commit，也应按后端优先、前端随后组织改动，避免混入无关清理。

## Review 补充

本节是对上面计划再次 review 后补充的遗漏点。后续实现时这些点不应被忽略。

### 1. 工具图标和工具类型映射

逐工具 renderer 不能只靠 `toolName === "Bash"` 这类精确匹配。参考项目同时支持标准 Claude 工具名、大小写变体、MCP/custom/fuzzy tool name。

实现要求：

- 前端增加 `getSessionToolVariant(toolName)`。
- 前端增加 `SessionToolIcon`。
- 常见映射：
  - Read / Write / Edit / MultiEdit / NotebookEdit -> `code`
  - Glob / LS / file list -> `file`
  - Grep / search -> `search`
  - Bash / shell / command / execute -> `terminal`
  - TodoWrite / update_plan / Task / Agent -> `task`
  - WebFetch / WebSearch / http -> `web`
  - MCP / server tool -> `mcp`
  - PDF / document / citation -> `document`
  - unknown -> `neutral`
- 给映射补纯函数测试，避免后续新增 provider 后全部掉到 neutral。

### 2. 状态徽标

当前计划已经要求 status badge，但还需要明确状态来源。

状态推导顺序：

1. `block.isError === true` -> error
2. `block.status` 明确为 error / failed / interrupted -> error 或 warning
3. result 里存在 `stderr` 且非空 -> warning/error
4. result 里存在 `returnCode` 且不是 0 -> warning/error
5. tool result 缺失 -> pending
6. 其他有 result -> success

视觉要求：

- pending 使用 muted badge。
- success 使用 success badge。
- warning 使用 warning badge。
- error 使用 danger badge，并让卡片边框进入 error variant。
- 状态不能只靠颜色，badge 内要有文字或图标。

### 3. Tool Result Router

只做 tool call 卡片不够，结果展示也需要 router。否则 Bash、Grep、Read 的结果会被打回普通 JSON 或纯文本。

第一版 result router 至少支持：

- string result
- error string
- stdout/stderr/returnCode object
- filenames/numFiles file list
- file content object
- file edit object
- structured patch object
- content array
- MCP text/image/resource result
- web search result
- todo update result
- unknown object fallback

### 4. Content Array

Claude 和部分 provider 的 content 不是单一字符串，而是 array。后端 parser 和前端 renderer 都要保留顺序。

必须支持：

- text block
- image block
- document block
- citation block
- tool_call block
- tool_result block
- thinking / redacted_thinking block
- unknown block fallback

### 5. 日期和滚动定位

计划已有日期分割线，但还缺少滚动定位细节。

实现要求：

- 每条可定位消息都有稳定 DOM id，例如 `session-message-${message.id || index}`。
- 日期分割线不参与 message count，但参与滚动视觉。
- 搜索上一条 / 下一条要滚动到消息，并把当前 match 标记成 active。
- navigator 点击和搜索跳转使用同一套 scroll helper。
- 当前视口消息高亮可以先用 IntersectionObserver；如果实现成本高，第一版可在点击/搜索跳转时更新 active id。
- 浮动日期 overlay 第一版可选，但日期分割线必须做。

### 6. 虚拟化策略

参考项目使用虚拟化。当前项目第一版不建议直接引入新依赖，但要留好边界。

第一版要求：

- message viewer 内部是独立滚动容器。
- 每条 message 有稳定 key。
- 长代码块和长结果自身滚动，避免撑高整个列表。
- 工具函数和 renderer 不依赖真实 DOM 高度。

后续如果大会话卡顿，再补：

- message row virtualization
- height estimation
- scroll restoration
- virtual row + date divider flattening

### 7. Capture Mode 不是第一版，但结构要避免冲突

参考项目有 capture mode、隐藏 block、截图预览。第一版不做，但组件结构不要阻碍后续扩展。

预留点：

- message card 接受 `selectionMode` / `selected` / `hidden` 可选 props，但第一版可以不用。
- block renderer 不直接读取全局 DOM；后续截图时能复用。
- copy selected markdown 和 screenshot 可以作为第二阶段。

### 8. Provider 差异和 fallback

不同工具源数据精度不同。不能为了让 UI 好看而伪造字段。

实现原则：

- 有结构就结构化。
- 没结构就 text block。
- 不能确定 tool result 对应关系时，不合并，只共享可用 id。
- `content` 永远保持可读。
- `metadata` 只放渲染必要或调试必要字段，不塞完整 raw event。

### 9. 当前项目约束

实现时还必须回头检查这些项目约束：

- 修改 `tauri/src/coding/session_manager/**` 前先读该目录 `AGENTS.md`。
- 修改 `web/features/coding/shared/**` 前先读 shared `AGENTS.md`。
- `SessionManagerPanel` 依赖 `tool + sourcePath` 契约，不能把 `sourcePath` 当展示字段随意改。
- KeepAlive 隐藏页不能吐旧的全局成功/失败提示。
- 改动影响 list/detail/import/export/rename/delete 时，必须一起验证这些链路。
- 结构化消息如果成为长期设计决策，实施时应同步更新对应模块 `AGENTS.md`。

## Review 补充 2：多 CLI 复用、中间层和模块化

本轮 review 后需要把架构边界再收紧：这次改造不能做成“五个 CLI 各一套详情页 + 各一套 parser + 各一套 renderer”。正确方向是让 Claude Code、Codex、Gemini CLI、OpenCode、OpenClaw 都落到同一套 normalized session domain，差异只留在 provider adapter / parser 边缘。

### 1. 总体分层

推荐分层如下：

```text
Provider raw files / sqlite / export snapshot
        |
        v
Provider parser adapter
  - 读取各 CLI 原始格式
  - 保留原始 id、timestamp、role、tool id
  - 把 raw content 转为 draft blocks
        |
        v
Session normalization layer
  - 统一 block kind
  - 统一 normalizedToolName
  - 统一 status / isError
  - 统一 tool call/result pairing
  - 统一 content flatten
  - 统一 search/navigator text source
        |
        v
Shared frontend session detail domain
  - message/block helpers
  - tool catalog
  - search/filter/navigation
  - renderer routing
        |
        v
Shared detail workbench UI
```

关键要求：

- provider parser 只处理“这个 CLI 原始数据长什么样”。
- normalization layer 处理“多个 CLI 如何变成同一个会话详情协议”。
- frontend renderer 只吃统一 block，不直接理解 Claude/Codex/Gemini/OpenCode/OpenClaw 的原始事件结构。
- `SessionManagerPanel` 继续只作为共享入口，不把五个工具页面拆出五份详情实现。

### 2. 后端中间层设计

建议新增两个小模块，避免把 `mod.rs` 继续变大：

- `tauri/src/coding/session_manager/message_blocks.rs`
- `tauri/src/coding/session_manager/tool_normalizer.rs`

`message_blocks.rs` 职责：

- 放 `SessionMessageBlock` / `SessionMessageUsage` 相关 helper，结构体本身可以继续在 `mod.rs` 或迁入该文件后 `pub use`。
- 提供 `text_block`、`summary_block`、`thinking_block`、`command_block`、`tool_call_block`、`tool_result_block`、`unknown_block`。
- 提供 `flatten_blocks_for_content(blocks)`。
- 提供 `pair_tool_blocks(blocks)`，只按明确 `tool_id` 合并。
- 提供 `message_from_blocks(role, ts, blocks)` 或等效 builder，减少五个 parser 重复写 `content` fallback。

`tool_normalizer.rs` 职责：

- 提供 `normalize_tool_name(raw_tool_name)`，返回稳定 `normalized_tool_name`。
- 提供 `infer_tool_variant(normalized_tool_name, raw_tool_name)`。
- 提供 `infer_tool_status(block)`，统一 pending/success/warning/error。
- 提供 `extract_tool_primary_label(normalized_tool_name, input, metadata)`，供后端 flatten 或前端 navigator 参考。

KISS 约束：

- 第一版不需要做大型 trait object / plugin framework。
- 不要求每个 provider 实现一个复杂 trait；当前每个 provider 已经有独立 `load_messages`，让它们调用同一套 helper 即可。
- 如果后续新增第六个 CLI，再考虑把 provider adapter trait 化。当前先保持函数式 helper，更容易维护。

### 3. 统一工具目录

需要明确一份跨 CLI 的 canonical tool catalog。后端输出 `normalized_tool_name`，前端可以二次兜底归一，但不能每个 renderer 自己猜。

| normalized_tool_name | 典型原始名 / alias | variant | renderer |
|---|---|---|---|
| `bash` | Bash, bash, shell, terminal, execute_command, command | `terminal` | BashToolCard |
| `read` | Read, read_file, file_read, view_file | `code` | ReadToolCard |
| `write` | Write, write_file, create_file | `code` | WriteToolCard |
| `edit` | Edit, edit_file, replace_in_file | `code` | EditToolCard |
| `multi_edit` | MultiEdit, multi_edit, batch_edit | `code` | EditToolCard |
| `apply_patch` | apply_patch, Patch, file_patch | `code` | PatchToolCard |
| `notebook_edit` | NotebookEdit, notebook_edit | `code` | NotebookToolCard |
| `grep` | Grep, grep, search_text, rg | `search` | SearchToolCard |
| `glob` | Glob, glob, find_files, file_glob | `file` | SearchToolCard / FileList renderer |
| `web_fetch` | WebFetch, web_fetch, fetch_url, browser_fetch | `web` | WebToolCard |
| `web_search` | WebSearch, web_search, search_web | `web` | WebToolCard |
| `todo_write` | TodoWrite, todo_write, todo | `task` | TodoToolCard |
| `update_plan` | update_plan, UpdatePlan, plan | `task` | PlanToolCard |
| `task` | Task, task, subagent_task | `task` | TaskToolCard |
| `agent` | Agent, agent, subagent, delegate | `task` | TaskToolCard |
| `mcp` | mcp__*, MCPTool, server_tool | `mcp` | McpToolCard |
| `unknown` | any unmatched tool | `neutral` | DefaultToolCard |

实现要求：

- `tool_name` 永远保留原始显示名。
- `normalized_tool_name` 只用稳定小写 snake_case。
- 不确定时输出 `unknown`，不要伪造为看起来相似的工具。
- MCP 工具优先识别 `mcp__server__tool`、`serverName/toolName`、provider 暴露的 MCP result shape。
- 前端 `getSessionToolVariant` 优先使用 `block.normalizedToolName`，没有时再对 `block.toolName` 做兜底。

### 4. Provider parser adapter 输出边界

每个 provider parser 的目标不是直接生成最终视觉效果，而是生成统一消息语义。

Claude Code parser：

- raw `content[]` 顺序必须保留。
- `tool_use.name` -> `tool_name`，同时用 `tool_normalizer` 生成 `normalized_tool_name`。
- `tool_result.tool_use_id` 必须进入 `tool_id`。
- usage/model/cost/duration 放到标准字段。

Codex parser：

- `function_call` / `tool_call` -> `tool_call` block。
- `function_call_output` / `tool_output` -> `tool_result` block。
- raw item id 保留到 message/block metadata。
- 现有 `[Tool: name]` 只能作为 fallback 文本，不能作为结构化主来源。

Gemini CLI parser：

- `functionCall.name` -> `tool_name`。
- `functionResponse.name` 或可推断字段 -> `tool_name`。
- 只在 id/part 关系明确时配对；否则保持 call/result 分开。
- array parts 顺序优先于视觉合并，不能因为配对破坏原始文本顺序。

OpenCode parser：

- JSON 和 SQLite 两条读取路径最终都要走相同 normalized helper。
- 对 official export/raw snapshot 保持原语义；normalized blocks 只服务详情展示和 normalized export。
- `source_path` / SQLite source key 不进入 UI 视觉归一层，仍由 OpenCode 专属逻辑处理。

OpenClaw parser：

- 第一版至少统一 text block。
- 如果源数据没有可靠 tool id，不强行合并。
- 能识别的 tool shape 再逐步接入 tool normalizer。

### 5. 前端 domain 层模块化

新增 `web/features/coding/shared/sessionManager/detail/domain/`，把纯逻辑从 React 组件里拿出来。

建议文件：

- `messageBlocks.ts`
- `toolCatalog.ts`
- `toolPairing.ts`
- `messageSearch.ts`
- `messageFilters.ts`
- `messageNavigator.ts`
- `messageFlatten.ts`

职责划分：

- `messageBlocks.ts`：`getMessageBlocks`、旧消息 fallback、block 类型判断。
- `toolCatalog.ts`：`getNormalizedToolName`、`getToolVariant`、`getToolIconName`、`getToolPrimaryLabel`。
- `toolPairing.ts`：前端兜底合并 call/result，输入是 blocks，输出是 display blocks。
- `messageSearch.ts`：搜索文本提取、match ranges、active match。
- `messageFilters.ts`：role/content type filter。
- `messageNavigator.ts`：navigator entry 生成和预览提取。
- `messageFlatten.ts`：日期分割线、可见消息 flatten。

要求：

- 这些文件尽量保持纯函数，方便 `web/test/` 里覆盖。
- React 组件不直接解析 provider raw shape。
- Renderer 不自己做全局搜索/过滤；只消费 domain 层给出的 display block。
- 对旧 `SessionMessage { role, content, ts }` 的 fallback 只在 `messageBlocks.ts` 里做一次，避免多个组件重复判断。

### 6. 前端 UI 组件模块边界

详情 UI 推荐分成四层：

```text
SessionDetailWorkbench
  SessionDetailCommandBar
  SessionMessageViewer
    SessionMessageCard
      SessionMessageBlockRenderer
        tool/text/thinking/result renderers
  SessionMessageNavigator
  SessionDetailStatusBar
```

边界要求：

- `SessionDetailWorkbench` 负责状态组合：搜索、过滤、active message、navigator 开关。
- `SessionDetailCommandBar` 只负责命令和控件，不渲染消息。
- `SessionMessageViewer` 只负责滚动容器、日期分割线、消息列表。
- `SessionMessageCard` 只负责 role 布局、header、copy/expand。
- `SessionMessageBlockRenderer` 只按 block kind 分发。
- `SessionToolExecutionCard` 只按 `normalizedToolName` 分发具体工具卡片。
- `SessionRendererCard` 统一卡片外壳、collapse、status badge、variant class。

不应该出现的实现：

- 在 `SessionManagerPanel.tsx` 里继续追加几百行详情 JSX。
- 在每个 CLI 页面单独维护一份详情页。
- 在每个工具 renderer 里重复写 collapse/header/status badge。
- 在 CSS 里为每个 provider 写独立主题色。

### 7. 共享样式 token 和 variant

样式也要复用，不按 CLI 拆。

建议新增：

- `detail/SessionDetailWorkbench.module.less`
- 或 `detail/detailTokens.module.less`

核心 class：

- `.rendererCard`
- `.rendererHeader`
- `.rendererBody`
- `.variantTerminal`
- `.variantCode`
- `.variantFile`
- `.variantSearch`
- `.variantTask`
- `.variantWeb`
- `.variantMcp`
- `.variantSystem`
- `.variantThinking`
- `.statusPending`
- `.statusSuccess`
- `.statusWarning`
- `.statusError`

要求：

- variant 只表达语义，不表达 provider。
- 所有颜色来自主题变量或 Ant Design token。
- 不使用参考项目 Tailwind palette。
- 高密度但不能牺牲可点击区：桌面紧凑，移动端 header 点击区仍不低于 44px。

### 8. 契约测试优先级

为了保证多 CLI 复用，测试要覆盖 normalized contract，而不是只测某个 CLI 的字符串输出。

后端新增/调整测试：

- `normalize_tool_name`：多 alias 能归一到相同 `normalized_tool_name`。
- `pair_tool_blocks`：同 tool id 合并；无 id 或不确定关系不合并。
- `flatten_blocks_for_content`：text/tool/thinking/result 都能产生可读 fallback。
- Claude/Codex/Gemini/OpenCode/OpenClaw parser 至少各一个 fixture，断言输出 block kind 和 `normalized_tool_name`。
- OpenCode official export/import 测试继续断言 native snapshot 不被 normalized fields 污染。

前端新增/调整测试：

- `toolCatalog.ts` alias routing。
- `toolPairing.ts` call/result 合并和 result-only fallback。
- `messageSearch.ts` 覆盖 tool input/output。
- `messageNavigator.ts` 对 user、assistant、tool、summary 都能生成条目。
- renderer routing：`bash/read/edit/grep/glob/web/todo/plan/task/mcp/unknown` 不掉到错误 renderer。

### 9. 执行顺序微调

原执行顺序需要前置 contract 层：

1. 锁定 normalized message/block contract，补 `normalizedToolName`。
2. 后端新增 `message_blocks.rs` / `tool_normalizer.rs`。
3. 后端先补 tool normalizer 和 pairing/flatten 单元测试。
4. 逐 provider parser 接入 shared helper。
5. 前端先建 `detail/domain/*` 纯函数和测试。
6. 再建 renderer shell 和工具 renderer。
7. 最后接入 `SessionManagerPanel` 的详情 Modal。

这样做的收益：

- provider 差异集中在 parser adapter。
- frontend renderer 不感知 CLI 来源。
- 后续新增 CLI 时，只要接入 parser adapter 和 normalized contract，不需要复制详情页。
- 已有 CLI 新增工具时，优先扩展 tool catalog 和一个 renderer，而不是改五个页面。

### 10. 验收补充

除原验收标准外，新增以下架构验收：

- 五个 CLI 的详情页使用同一个 `SessionDetailWorkbench`。
- 常见工具通过 `normalizedToolName` 路由，而不是 provider-specific `if tool === claudecode` 判断。
- `SessionManagerPanel.tsx` 只负责列表、API handler、Modal 生命周期和把 detail 传给 workbench。
- 后端 provider parser 中没有重复实现大段 tool pairing/status/variant 逻辑。
- 旧纯文本消息 fallback 只通过共享 helper 处理。
- 新增工具 renderer 时不需要修改五个 CLI 页面。
- 新增 provider parser fixture 时能复用同一套 normalized contract 断言。

## 参考代码文件索引

下面文件来自 `D:\GitHub\claude-code-history-viewer`，用于后续实现时对照。不要直接复制 Tailwind/Radix/CSS palette；应复制交互结构、信息层级和 renderer 语义，再适配 AI Toolbox 的主题变量和 Ant Design/lucide 约定。

### 总布局和详情工作台

| 用途 | 参考文件 |
|---|---|
| 整体三栏工作台布局、顶部 Header、右侧 navigator 挂载位置 | `D:\GitHub\claude-code-history-viewer\src\layouts\AppLayout.tsx` |
| 顶部命令区入口、全局按钮密度 | `D:\GitHub\claude-code-history-viewer\src\layouts\Header\Header.tsx` |
| 消息详情主容器、搜索栏、过滤栏、滚动按钮、overlay、虚拟列表挂载 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\MessageViewer.tsx` |
| 过滤工具条 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\components\FilterToolbar.tsx` |
| 捕获模式工具条，第二阶段参考 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\components\CaptureModeToolbar.tsx` |
| 日期分割线 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\components\DateDivider.tsx` |
| 浮动日期 overlay，第二阶段参考 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\components\FloatingDateOverlay.tsx` |
| 虚拟行结构，后续性能优化参考 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\components\VirtualizedMessageRow.tsx` |
| 消息树 flatten、date divider 插入、隐藏块 placeholder | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\helpers\flattenMessageTree.ts` |
| 消息高度估算，后续虚拟化参考 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\helpers\heightEstimation.ts` |
| 搜索状态、deferred query | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\hooks\useSearchState.ts` |
| 滚动到底、搜索跳转、目标消息滚动 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\hooks\useScrollNavigation.ts` |
| 虚拟化 hook，后续参考 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\hooks\useMessageVirtualization.ts` |

### 消息节点和消息正文

| 用途 | 参考文件 |
|---|---|
| 单条消息调度入口：summary/system/default/tool execution | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\components\ClaudeMessageNode.tsx` |
| 消息头：role、时间、metadata 展示 | `D:\GitHub\claude-code-history-viewer\src\components\MessageViewer\components\MessageHeader.tsx` |
| user/assistant 气泡、Markdown、表格折叠、复制按钮 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\MessageContentDisplay.tsx` |
| summary 消息展示 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\SummaryMessageRenderer.tsx` |
| system 消息展示 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\SystemMessageRenderer.tsx` |
| assistant 详情和 usage/model 展示参考 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\AssistantMessageDetails.tsx` |
| command 输出展示 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\CommandOutputDisplay.tsx` |
| progress / queue / file history 特殊消息，后续可选 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\ProgressRenderer.tsx` |
| progress / queue / file history 特殊消息，后续可选 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\QueueOperationRenderer.tsx` |
| progress / queue / file history 特殊消息，后续可选 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\FileHistorySnapshotRenderer.tsx` |

### Renderer 外壳、样式和工具图标

| 用途 | 参考文件 |
|---|---|
| collapsible renderer 外壳、header/content 结构 | `D:\GitHub\claude-code-history-viewer\src\shared\RendererHeader.tsx` |
| compound renderer card | `D:\GitHub\claude-code-history-viewer\src\components\renderers\RendererCard.tsx` |
| renderer variant、尺寸、padding、code max height | `D:\GitHub\claude-code-history-viewer\src\components\renderers\styles.ts` |
| renderer 类型 | `D:\GitHub\claude-code-history-viewer\src\components\renderers\types.ts` |
| renderer hooks：auto-expand on search | `D:\GitHub\claude-code-history-viewer\src\components\renderers\hooks.ts` |
| 工具图标组件 | `D:\GitHub\claude-code-history-viewer\src\components\ToolIcon.tsx` |
| 工具名到 variant 映射 | `D:\GitHub\claude-code-history-viewer\src\utils\toolIconUtils.ts` |
| 工具 variant 测试参考 | `D:\GitHub\claude-code-history-viewer\src\test\toolIconUtils.test.ts` |
| 搜索高亮 | `D:\GitHub\claude-code-history-viewer\src\components\common\HighlightedText.tsx` |
| tooltip icon button | `D:\GitHub\claude-code-history-viewer\src\shared\TooltipButton.tsx` |

### Tool Use 分发和逐工具输入卡片

| 用途 | 参考文件 |
|---|---|
| tool_use 分发总入口 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ToolUseRenderer.tsx` |
| tool use card 统一外壳 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\ToolUseCard.tsx` |
| Bash 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\BashToolRenderer.tsx` |
| Read 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\ReadToolRenderer.tsx` |
| Grep 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\GrepToolRenderer.tsx` |
| Glob 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\GlobToolRenderer.tsx` |
| WebFetch 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\WebFetchToolRenderer.tsx` |
| WebSearch 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\WebSearchToolRenderer.tsx` |
| MultiEdit 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\MultiEditToolRenderer.tsx` |
| ApplyPatch 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\ApplyPatchToolRenderer.tsx` |
| NotebookEdit 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\NotebookEditToolRenderer.tsx` |
| TodoWrite 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\TodoWriteToolRenderer.tsx` |
| update_plan 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\UpdatePlanToolRenderer.tsx` |
| Task 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\TaskToolRenderer.tsx` |
| Agent/Subagent 输入展示 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\AgentToolRenderer.tsx` |
| Task create/update/output，后续可选 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\TaskCreateToolRenderer.tsx` |
| Task create/update/output，后续可选 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\TaskUpdateToolRenderer.tsx` |
| Task create/update/output，后续可选 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\toolUseRenderers\TaskOutputToolRenderer.tsx` |

### Tool Use + Result 合并卡片

| 用途 | 参考文件 |
|---|---|
| 合并 tool_use + tool_result 分发入口 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\UnifiedToolExecutionRenderer.tsx` |
| 合并卡片公共类型和 truncate/error helper | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\shared.ts` |
| 合并卡片状态 badge | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\StatusBadge.tsx` |
| 合并结果渲染入口 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\ResultBlock.tsx` |
| Bash 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\BashCard.tsx` |
| Read 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\ReadCard.tsx` |
| Write 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\WriteCard.tsx` |
| Edit/MultiEdit 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\EditCard.tsx` |
| Grep 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\GrepCard.tsx` |
| Glob 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\GlobCard.tsx` |
| WebFetch 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\WebFetchCard.tsx` |
| WebSearch 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\WebSearchCard.tsx` |
| Agent/Task 合并卡片 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\AgentCard.tsx` |
| unknown 合并卡片 fallback | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\unifiedCards\DefaultCard.tsx` |

### Tool Result 和内容块

| 用途 | 参考文件 |
|---|---|
| tool result 分发总入口 | `D:\GitHub\claude-code-history-viewer\src\components\messageRenderer\ToolExecutionResultRouter.tsx` |
| tool result card 统一外壳 | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ToolResultCard.tsx` |
| terminal stdout/stderr/return code | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\TerminalExecutionResultRenderer.tsx` |
| Bash code execution result | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\BashCodeExecutionToolResultRenderer.tsx` |
| code execution result | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\CodeExecutionToolResultRenderer.tsx` |
| text editor code execution result | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\TextEditorCodeExecutionToolResultRenderer.tsx` |
| command tag renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\CommandRenderer.tsx` |
| thinking renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ThinkingRenderer.tsx` |
| redacted thinking renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\RedactedThinkingRenderer.tsx` |
| Claude content array renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ClaudeContentArrayRenderer.tsx` |
| tool result content array renderer | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\ContentArrayRenderer.tsx` |
| file content renderer | `D:\GitHub\claude-code-history-viewer\src\components\FileContent.tsx` |
| diff/file edit renderer | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\FileEditRenderer.tsx` |
| enhanced diff viewer | `D:\GitHub\claude-code-history-viewer\src\components\EnhancedDiffViewer.tsx` |
| advanced diff viewer | `D:\GitHub\claude-code-history-viewer\src\components\AdvancedTextDiff.tsx` |
| file list result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\FileListRenderer.tsx` |
| string / markdown result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\StringRenderer.tsx` |
| fallback object result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\FallbackRenderer.tsx` |
| error result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\ErrorRenderer.tsx` |
| structured patch result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\StructuredPatchRenderer.tsx` |
| terminal stream result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\TerminalStreamRenderer.tsx` |
| todo update result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\TodoUpdateRenderer.tsx` |
| task result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\TaskResultRenderer.tsx` |
| task status icon/color config | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\taskStatusConfig.ts` |
| MCP result | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\MCPToolResultRenderer.tsx` |
| MCP use | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\MCPToolUseRenderer.tsx` |
| MCP structured renderer | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\MCPRenderer.tsx` |
| WebSearch result | `D:\GitHub\claude-code-history-viewer\src\components\toolResultRenderer\WebSearchRenderer.tsx` |
| WebSearch content result | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\WebSearchResultRenderer.tsx` |
| WebFetch content result | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\WebFetchToolResultRenderer.tsx` |
| Search result | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\SearchResultRenderer.tsx` |
| image renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ImageRenderer.tsx` |
| document renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\DocumentRenderer.tsx` |
| citation renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\CitationRenderer.tsx` |
| container upload renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ContainerUploadRenderer.tsx` |
| server tool use renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ServerToolUseRenderer.tsx` |
| tool search result renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\ToolSearchToolResultRenderer.tsx` |
| OpenCode step renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\OpenCodeStepRenderer.tsx` |
| task notification renderer | `D:\GitHub\claude-code-history-viewer\src\components\contentRenderer\TaskNotificationRenderer.tsx` |

### 右侧消息导航

| 用途 | 参考文件 |
|---|---|
| 右侧 navigator UI、搜索、user-only filter、虚拟滚动 | `D:\GitHub\claude-code-history-viewer\src\components\MessageNavigator\MessageNavigator.tsx` |
| navigator entry 生成、过滤噪声消息、预览提取 | `D:\GitHub\claude-code-history-viewer\src\components\MessageNavigator\useNavigatorEntries.ts` |
| navigator 类型 | `D:\GitHub\claude-code-history-viewer\src\components\MessageNavigator\types.ts` |

### 后端结构化消息和 tool result 合并

| 用途 | 参考文件 |
|---|---|
| 统一消息结构 `ClaudeMessage` | `D:\GitHub\claude-code-history-viewer\src-tauri\src\models\message.rs` |
| 会话元数据结构 | `D:\GitHub\claude-code-history-viewer\src-tauri\src\models\session.rs` |
| 多 provider 读取消息入口 | `D:\GitHub\claude-code-history-viewer\src-tauri\src\commands\multi_provider.rs` |
| tool_use/tool_result 合并逻辑 | `D:\GitHub\claude-code-history-viewer\src-tauri\src\commands\multi_provider.rs` |
| Claude JSONL load parser、mmap/simd-json、分页参考 | `D:\GitHub\claude-code-history-viewer\src-tauri\src\commands\session\load.rs` |
| 会话搜索参考 | `D:\GitHub\claude-code-history-viewer\src-tauri\src\commands\session\search.rs` |
| 后端 message/session snapshot tests | `D:\GitHub\claude-code-history-viewer\src-tauri\src\models\snapshot_tests.rs` |

### AI Toolbox 当前改造入口

| 用途 | 当前项目文件 |
|---|---|
| shared session manager 主入口 | `D:\GitHub\ai-toolbox\web\features\coding\shared\sessionManager\SessionManagerPanel.tsx` |
| shared session manager 类型 | `D:\GitHub\ai-toolbox\web\features\coding\shared\sessionManager\types.ts` |
| shared session manager API | `D:\GitHub\ai-toolbox\web\features\coding\shared\sessionManager\sessionManagerApi.ts` |
| shared session manager 工具函数 | `D:\GitHub\ai-toolbox\web\features\coding\shared\sessionManager\utils.ts` |
| shared session manager 样式 | `D:\GitHub\ai-toolbox\web\features\coding\shared\sessionManager\SessionManagerPanel.module.less` |
| 后端 session manager 主结构和导入导出 | `D:\GitHub\ai-toolbox\tauri\src\coding\session_manager\mod.rs` |
| Claude Code parser | `D:\GitHub\ai-toolbox\tauri\src\coding\session_manager\claude_code.rs` |
| Codex parser | `D:\GitHub\ai-toolbox\tauri\src\coding\session_manager\codex.rs` |
| Gemini CLI parser | `D:\GitHub\ai-toolbox\tauri\src\coding\session_manager\gemini_cli.rs` |
| OpenCode parser/import/export | `D:\GitHub\ai-toolbox\tauri\src\coding\session_manager\open_code.rs` |
| OpenClaw parser/import/export | `D:\GitHub\ai-toolbox\tauri\src\coding\session_manager\open_claw.rs` |
| shared 模块约束 | `D:\GitHub\ai-toolbox\web\features\coding\shared\AGENTS.md` |
| 后端 session manager 模块约束 | `D:\GitHub\ai-toolbox\tauri\src\coding\session_manager\AGENTS.md` |

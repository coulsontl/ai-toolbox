# AI Toolbox 插件开发指南

本文档面向 **AI Toolbox 插件作者**，完整说明当前支持的全部插件类型、清单规范、运行时契约、安全沙箱、持久化存储与本地调试方式。

AI Toolbox 的插件体系兼容 [Magpie](https://github.com/magpie-community) 插件规范及 OpenCode v1 `auth` 插件格式，并在自定义页面方向上提供多页扩展（`magpie.pages`）与轻量级隔离持久化存储（`storage:kv` / `storage:files`）。

> 本指南的核心内容同样内嵌于 AI Toolbox 的内置 **`ai-toolbox` Skill**（Skills Tab 内置项，默认同步到 Universal 目录并随应用版本自动更新），方便 AI Coding Agent 在开发插件时直接查阅。

---

## 1. 插件类型总览

一个插件（npm 包、`github:owner/repo`、本机目录或单个 `*.middleware.js` / `*.middleware.mjs` / `*.gateway.js` / `*.gateway.mjs` 文件）可以提供以下一种或多种能力：

| 类型 | `kind` | 运行引擎 | 启动时机 | 典型用途 |
|---|---|---|---|---|
| **Provider 渠道插件** | `provider` | 按需 Bun 单子进程（`1.4.2` + SHA-256 校验） | 仅在安装/使用 Provider 插件时按需下载并启动 | 接入第三方订阅/OAuth 渠道、多账号额度/签到、自动续期、模型发现与流式转发 |
| **Gateway 中间件插件** | `middleware` | 进程内 `rquickjs`（QuickJS-NG 沙箱） | 零外部依赖，网关热路径按需执行 | 请求改写、参数覆盖、模型重定向、系统提示词注入、敏感词过滤、SSE 帧改写、用量观测 |
| **自定义页面插件** | `page` | 按需本机静态微服务器（`127.0.0.1:0`）+ 沙箱 `<iframe>` | **纯页面插件不启动 Bun**；仅当插件含代码入口（`main`/`exports`）且页面声明 `service` 时按需拉起 Bun | 在「插件中心」左侧菜单注册自定义页面（数据看板、分享卡片、工具页、内嵌 Chat 等） |
| **复合插件** | `both` / 任意类型 + `pages` | 按需组合 | 各自独立按需触发 | Provider 插件顺带提供统计/设置页，或同时提供 Provider 与 Middleware |

> **如何接入 CLI**：Provider 插件登录账号后，用户可在「插件中心 → 渠道与账号 → 添加到 CLI」或任意支持的 CLI Tab 中点击「从插件导入」，一键同步到 **16 个 Coding CLI**（Claude Code、Claude Desktop、Codex、Grok、Kimi、Gemini CLI 走网关接管并支持单账号/自动轮询策略；ZCode、WorkBuddy、CodeBuddy、OpenCode、OpenClaw、Pi、Oh My Pi、OMO Native、Hermes、DSH 走直连插件路由并固定自动轮询；Antigravity CLI 仅支持 Google 官方 OAuth，不支持自定义供应商）。同时本机网关自动暴露：
> - `GET /plugin/<provider_id>/v1/models`、`GET /plugin/<provider_id>/v1beta/models`
> - `POST /plugin/<provider_id>/v1/chat/completions`
> - `POST /plugin/<provider_id>/v1/responses`、`POST /plugin/<provider_id>/v1/responses/compact`
> - `POST /plugin/<provider_id>/v1/messages`、`POST /plugin/<provider_id>/v1/messages/count_tokens`
> - `POST /plugin/<provider_id>/v1beta/...:generateContent` / `:streamGenerateContent`

---

## 2. 安装来源与本地调试

在「插件中心 → 安装插件」弹窗中支持 4 种输入：

1. **npm 包名**：`@scope/pkg`、`@scope/pkg@1.2.3`、`my-plugin`
2. **GitHub 仓库**：`github:owner/repo` 或 `github:owner/repo#branch`
3. **本机插件目录**：绝对路径（如 `D:\dev\my-plugin`、`/Users/me/my-plugin`）或 `file://` URL
   - 本机目录为**原地引用**，AI Toolbox 不会复制或删除你的源码目录；
   - 修改 `package.json` 或页面 HTML/JS 后，点击侧边栏底部的刷新按钮或页面顶栏的「重新加载」即可生效；
   - **纯页面插件与纯中间件插件走本机目录安装时完全不依赖 Bun**。
4. **单文件中间件**：直接传入 `*.middleware.js`、`*.middleware.mjs`、`*.gateway.js` 或 `*.gateway.mjs` 文件路径。

---

## 3. 自定义页面插件（`magpie.page` / `magpie.pages`）

自定义页面插件会在「插件中心」可折叠左侧菜单栏的「**插件页面**」分组中注册入口；右侧区域提供 `42px` 轻上下文栏，下方 **100% 全画布**渲染你的 HTML 页面。

### 3.1 `package.json` 声明

**单页（100% 兼容 Magpie 社区规范）：**

```json
{
  "name": "@your-scope/page-demo",
  "version": "0.1.0",
  "description": "Demo custom page plugin",
  "magpie": {
    "icon": "page/icon.svg",
    "summary": {
      "zh": "示例自定义页面插件",
      "en": "Demo custom page plugin"
    },
    "permissions": [
      "usage:summary",
      "usage:day",
      "providers:list",
      "storage:kv",
      "storage:files"
    ],
    "page": {
      "title": "Demo Page",
      "titles": {
        "zh": "示例页面",
        "en": "Demo Page"
      },
      "file": "page/index.html",
      "icon": "page/icon.svg",
      "keepAlive": false
    }
  }
}
```

**多页扩展（AI Toolbox 支持单个插件在左侧注册多个菜单项）：**

```json
{
  "magpie": {
    "permissions": ["usage:summary", "storage:kv"],
    "pages": [
      {
        "id": "overview",
        "title": "Overview",
        "titles": { "zh": "总览", "en": "Overview" },
        "file": "pages/overview/index.html",
        "icon": "pages/overview/icon.svg",
        "keepAlive": true
      },
      {
        "id": "tools",
        "title": "Tools",
        "titles": { "zh": "实用工具", "en": "Tools" },
        "file": "pages/tools/index.html"
      }
    ]
  }
}
```

#### 字段说明

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | 否 | `magpie.page` 默认为 `"default"`；`magpie.pages` 未填时自动从标题/文件名生成唯一 slug |
| `title` | 建议 | 默认标题（无匹配语言时展示） |
| `titles` | 否 | 按语言码映射（如 `zh` / `zh-CN` / `en`） |
| `file` | **是** | 相对插件根目录的 `.html` / `.htm` 路径，禁止绝对路径或 `..` 逃逸 |
| `icon` | 否 | 相对插件根目录的图标文件（支持 `.svg`/`.png`/`.jpg`/`.jpeg`/`.webp`/`.ico`，最大 `256 KiB`）。不填时自动按品牌名匹配内置图标，再无匹配则在左侧栏渲染**圆角方框 + 菜单首字**（折叠后只留该单图标 + Tooltip） |
| `keepAlive` | 否 | 默认 `false`（切换管理 Tab 或切离 `/plugins` 后保留状态，后台闲置 **10 分钟**自动回收释放内存并吊销静态 Token）；设为 `true` 则持续保活直到插件禁用/卸载 |
| `service` | 否 | 可选 `{ "port": 18080, "option": "port" }`。**仅对包含 `main`/`exports` 宿主入口的插件生效**（纯页面插件会被安全门禁强制忽略，防止任意探测本机端口）；生效时 CSP 额外放行 `http://127.0.0.1:<port>` 与 `ws://127.0.0.1:<port>`，打开页面时按需拉起 Bun 宿主 |

> **安全边界说明**：
> - 每个页面的静态根目录严格限定在 `file` 所在的父目录内；HTML 与 `.svg` 均下发严格 CSP（含显式 `http://127.0.0.1:<port>/_plugin/<token>/` 来源，兼容 macOS WKWebView 与 Windows WebView2）。
> - 页面运行在 `<iframe sandbox="allow-scripts">` 中，无法触碰 `__TAURI__`、文件系统或外网；若 iframe 内部发生自我跳转（如外链导航），宿主会立即检测并断开 Bridge 保护数据安全。
> - 兼容边界：当前**暂未实现** Magpie 的 `magpie.actions` / `panels` / `replaces`（页面中注册的 `magpie.onAction` 不会收到动作事件）。

### 3.2 页面 Bridge API（`_magpie/bridge.js`）

在页面 HTML 中引入宿主虚拟挂载的桥接脚本（`window.aiToolboxPlugin` 为 `window.magpie` 的完整别名）：

```html
<!doctype html>
<html lang="zh">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <script src="_magpie/bridge.js"></script>
  </head>
  <body>
    <main id="app">Loading...</main>
    <script>
      (async () => {
        const theme = await window.magpie.ready;
        const summary = await window.magpie.read("usage:summary", { period: "7d" });

        // 需要声明 "storage:kv" 权限
        const visits = ((await window.magpie.storage.get("visits")) ?? 0) + 1;
        await window.magpie.storage.set("visits", visits);

        window.magpie.onTheme((nextTheme) => {
          console.log("Theme changed:", nextTheme.dark, nextTheme.lang);
        });

        window.magpie.onVisibility((visible) => {
          if (visible) {
            // 页面重新回到前台时刷新数据
          }
        });

        document.getElementById("app").textContent =
          `近 7 天调用 ${summary.calls} 次，访问次数 ${visits}，主题: ${theme.dark ? "暗色" : "亮色"}`;
      })();
    </script>
  </body>
</html>
```

#### 全部 Bridge 方法与属性

| API | 说明 |
|---|---|
| `await magpie.ready` | 等待宿主握手完成（超时 `10000ms`），返回 `{ dark, lang, tokens }` |
| `magpie.theme` / `magpie.onTheme(fn)` | 当前主题快照及亮/暗/系统主题或语言切换时的实时回调 |
| `magpie.visible` / `magpie.onVisibility(fn)` | 当前页面是否前台可见（切 Tab 或切离 `/plugins` 模块时均实时触发） |
| `magpie.context` | 页面上下文 `{ pluginId, pageId }` |
| `magpie.service` | 若含宿主入口且声明了 `service`，为本地服务 URL（如 `http://127.0.0.1:18080/`），否则为 `""` |
| `await magpie.options.get()` / `set(obj)` | 读写当前插件自己的持久化配置对象（纯页面插件保存不会重启 Bun；保存失败会正常 reject） |
| `await magpie.read(name, params?)` | 只读查询宿主数据（需在 `magpie.permissions` 声明，见 3.3） |
| `await magpie.image.copy(png)` | 将 PNG `Blob` / `ArrayBuffer` / `Uint8Array` 或 base64/DataURL（最大 `20 MiB`、`8192×8192`）写入系统剪贴板 |
| `await magpie.image.save(png, filename?)` | 保存 PNG 到系统下载目录（自动清洗文件名、防重名覆盖），返回 `{ path }` |
| `await magpie.openPlugin(packageName)` | 唤起主窗口「安装插件」弹窗并预填包名（需用户确认，绝不静默安装） |
| `await magpie.storage.get(key)` | 读取插件私有 KV 值（不存在返回 `null`；需 `storage:kv` 权限） |
| `await magpie.storage.set(key, value)` | 写入插件私有 KV 值（JSON；需 `storage:kv` 权限） |
| `await magpie.storage.delete(key)` | 删除键并返回布尔值 `boolean`（需 `storage:kv` 权限） |
| `await magpie.storage.list(prefix?)` | 返回按字典序排序的匹配键数组 `string[]`（需 `storage:kv` 权限） |
| `await magpie.storage.clear()` | 清空当前插件全部 KV（需 `storage:kv` 权限） |
| `await magpie.files.readText(path)` | 读取私有目录 UTF-8 文本，返回 `string`（需 `storage:files` 权限） |
| `await magpie.files.writeText(path, text)` | 写入 UTF-8 文本（自动建父目录、原子写），返回 `{ size }`（需 `storage:files` 权限） |
| `await magpie.files.readBytes(path)` | 读取二进制文件，返回 standard base64 `string`（需 `storage:files` 权限） |
| `await magpie.files.writeBytes(path, base64)` | 写入 base64 二进制文件，返回 `{ size }`（需 `storage:files` 权限） |
| `await magpie.files.list(dir?)` | 列出目录条目 `Array<{ name, path, isDir, size, modifiedAt }>`（需 `storage:files` 权限） |
| `await magpie.files.stat(path)` | 返回 `{ exists, isDir, size, modifiedAt }`（需 `storage:files` 权限） |
| `await magpie.files.delete(path)` | 删除文件或子目录，返回 `boolean`（需 `storage:files` 权限） |
| `magpie.error(err)` | 主动上报致命错误触发错误边界（Bridge 自动忽略沙箱内 `localStorage`/`sessionStorage` 的 `SecurityError` 与浏览器扩展脚本错误） |

#### 自动主题变量（`--magpie-*`）

Bridge 自动同步 `document.documentElement.dataset.theme`（`"light"` / `"dark"`）、`color-scheme` 以及以下 CSS 变量，并注入低优先级 `:where()` 基础样式：

- `--magpie-bg`（页面底色）
- `--magpie-fg`（主文本色）
- `--magpie-card`（容器表面色）
- `--magpie-line`（边框分割线）
- `--magpie-muted`（次要文本色）
- `--magpie-accent` / `--magpie-accent-fg`（主强调色与前景色）
- `--magpie-font`（宿主正文字体栈）
- `--magpie-code`（宿主等宽字体栈）

### 3.3 权限白名单与持久化存储配额（`magpie.permissions`）

所有权限均在 Rust 后端强制校验，默认零权限：

| Permission | 说明与契约 |
|---|---|
| `usage:summary` | 参数 `{ period?: "today" \| "7d" \| "30d", days?: number }`；返回 Magpie 标准结构 `{ period, since, calls, errors, input, output, cacheRead, cacheWrite, cost, models: [{ model, calls, input, output, cost }] }`（附 `requests`/`totalTokens` 兼容别名） |
| `usage:day` | 参数 `{ date?: "YYYY-MM-DD", from?: "YYYY-MM-DD" \| "all", tz?: string }`；返回 Magpie 标准 `Day` 结构 `{ from, date, today, calls, errors, input, output, cacheRead, cacheWrite, reasoning, cost, unpriced, cacheUnknown, agents: [...], moreAgents?, days: [...], icons, currency, rate }`（其中 `days` 为截至查询日的 53 周热力图数据） |
| `providers:list` | 返回非敏感摘要 `Array<{ id, name, cliKey, enabled }>`（**绝不暴露任何 API Key / Token / Header / 账号凭据**） |
| `storage:kv` | 隔离存储于 `<app_data>/plugins/data/<slug>-<hash8>/kv.json`；key ≤ `256` 字符，单值 ≤ `1 MiB`，总计 ≤ `20 MiB`，原子写 + `sync_all` |
| `storage:files` | 隔离存储于 `<app_data>/plugins/data/<slug>-<hash8>/files/`；拒绝 `..` / 绝对路径 / `\` / `:` (NTFS ADS) / Windows 保留名 / 符号链接、深度 ≤ `16` 层；单文件 ≤ `20 MiB`，总计 ≤ `200 MiB` |

> **卸载数据策略**：卸载插件时**默认保留** `<app_data>/plugins/data/<slug>-<hash8>/` 防止重装丢数据；用户可在卸载确认弹窗手动勾选「同时删除该插件保存的本地数据（KV 与文件）」一并清理。

---

## 4. Gateway 中间件插件（`magpie.middleware` / `magpie.gateway`）

中间件运行在网关进程内 `rquickjs` 沙箱中，与 Magpie 中间件契约对齐。

### 4.1 声明方式

1. **单文件形式**：命名为 `*.middleware.js`、`*.middleware.mjs`、`*.gateway.js` 或 `*.gateway.mjs` 的 ESM 文件。
2. **包目录形式**：在 `package.json` 中声明 `magpie.middleware`（或别名 `magpie.gateway`）：

```json
{
  "name": "@your-scope/middleware-demo",
  "version": "0.1.0",
  "magpie": {
    "middleware": "./index.mjs",
    "summary": { "zh": "示例网关中间件", "en": "Demo gateway middleware" }
  }
}
```

### 4.2 导出 Hook 与运行契约

```js
// 可选：限定 onEvent 只拦截特定 SSE 事件名
export const events = ["content_block_delta", "message_delta"];

export function onRequest(body, ctx) {
  // ctx.protocol: "anthropic" | "chat" | "responses" | "gemini"
  // ctx.agent: CLI 标识（如 "claude" / "codex" / "grok" / "gemini" 等）
  // ctx.model / ctx.stream / ctx.path / ctx.options / ctx.state（同一次请求内跨 Hook 共享）
  // 支持限流且自动脱敏的 console.log / console.warn / console.error
  // 返回新对象替换 body，或调用 ctx.reject(400, "reason") 直接拒绝请求
  return body;
}

export function onProvider(body, ctx) {
  // 每次尝试发送给上游前触发（ctx.provider: { id, name, account, model }）
  return body;
}

export function onEvent(event, ctx) {
  // SSE 流式帧处理（同一流内复用已编译模块与 ctx.state，不逐帧重编译）
  // 返回新对象替换、返回 null 丢弃、返回 undefined 保持原样
  return event;
}

export function onResponse(body, ctx) {
  // 非流式完整响应体改写
  return body;
}

export function onUsage(row, options) {
  // 异步用量观测回调（1024 容量队列，不阻塞主链路）
  // row 包含 Magpie 标准字段：time, operation, agent, provider, providerAccount,
  // model, requested, served, input, output, cacheRead, cacheWrite, reasoning,
  // ms, ttftMs, status, errType, kind, rejected, tries, cost
}
```

### 4.3 沙箱硬性约束

- **资源上限**：单次调用 `32 MiB` 内存、`512 KiB` 调用栈；模块加载 `1000 ms`、`onRequest`/`onProvider`/`onResponse`/`onUsage` 上限 `250 ms`、`onEvent` 上限 `50 ms`。
- **禁止篡改 `stream`**：`onRequest` / `onProvider` 若新增或修改顶层 `stream` 字段，改动会被立刻回滚并计入错误统计。
- **Fail-open**：除显式 `ctx.reject(400..599, message)` 外，超时、抛错或非法返回值均自动放行原请求。
- **仅允许包内相对 import**：无 `node_modules` 解析与网络 IO，第三方工具库请提前打包进入口文件。

---

## 5. Provider 渠道插件（兼容 Magpie / OpenCode v1 `auth`）

Provider 插件跑在按需启动的 Bun 宿主子进程中（宿主进程环境自动注入 `AI_TOOLBOX_PLUGINS_DATA_DIR` 指向官方插件数据根目录）：

- `package.json` 通过 `main` 或 `exports` 导出插件模块，可在 `magpie.providers` 中声明 provider ID 列表；
- 宿主完整支持：
  - **登录方式**：`authorize`（浏览器 OAuth / Device Code）、`callback`、`apiKey`、`importFiles`；
  - **账号生命周期**：`validate`、`check`、`usage`、`checkin`、`signOut`、`refresh`（支持 `refreshLead` 与 `keepAlive`）；
  - **后台自动化**：
    - **OAuth 后台定时续期**：接入 `auth_refresh` 共享调度器（启动 90 秒首轮 + 每 15 分钟检查），只对临近过期的已启用 OAuth 插件账号按需续期（无临期账号不拉起 Bun）；
    - **可选每日自动签到**：用户在「渠道与账号」为支持 `auth.checkin` 的账号开启「自动签到」后，按自然日（优先遵循 `until`，否则按北京时间次日 0 点）自动执行一次，失败后 30 分钟低频重试；
  - **声明式选项**：`auth.options` 布尔开关会持久化到 SQLite，重启自动回显；
  - **协议映射（`npm` → 网关目标协议）**：
    - `@ai-sdk/anthropic`、`@ai-sdk/google-vertex/anthropic` → Anthropic Messages
    - `@ai-sdk/openai`、`@ai-sdk/azure` → OpenAI Responses
    - `@ai-sdk/openai-compatible`、`@ai-sdk/openrouter`、`@ai-sdk/deepseek`、`@ai-sdk/xai`、`@ai-sdk/github-copilot` → OpenAI Chat
    - `@ai-sdk/google`、`@ai-sdk/google-vertex` → Gemini Native
- **凭据隔离**：账号真实凭据保存在 `<app_data>/plugins/plugin-auth.json`（`0600` 权限，排他锁保护）；网关发送请求前自动剥离占位认证头，仅由 Bun 宿主按选中账号注入真实凭据；stdout/stderr 日志自动脱敏。

---

## 6. 图标声明与内置品牌映射

插件在 `package.json` 或市场 `registry.json` 中通过 `icon`（或 `magpie.page.icon`）声明图标，按以下优先级渲染：

1. **插件包内本地图标**（页面插件 `magpie.page.icon`）：支持 `.svg` / `.png` / `.jpg` / `.jpeg` / `.webp` / `.ico`（<= `256 KiB`）。
2. **远程或内联图片**：`https://...` 或 `data:image/...`。
3. **内置 49 个品牌图标**（按 `name + icon` 自动匹配，含彩色版与自适应亮/暗模式单色版）：
   - **AI / 模型厂商**：`claude` / `anthropic`、`openai` / `chatgpt` / `gpt-` / `codex`、`gemini` / `google`、`deepseek`、`qwen` / `tongyi` / `alibaba`、`kimi` / `moonshot`、`zhipu` / `bigmodel` / `glm`、`zai` / `z.ai` / `zcode`、`grok` / `xai`、`groq`、`mistral`、`meta` / `llama`、`minimax`、`doubao`、`bytedance`、`hunyuan` / `tencent`、`volcengine` / `ark`、`stepfun`、`xiaomi` / `mimo` / `mimocode`、`perplexity`、`cohere`、`nvidia`、`poe`
   - **网关 / 云平台**：`openrouter`、`ollama`、`siliconflow` / `siliconcloud`、`together`、`fireworks`、`bedrock` / `aws`、`vertex` / `vertexai`、`azure` / `azureai`、`huggingface`、`github`、`copilot`
   - **Coding CLI / 工具品牌**：`opencode`、`openclaw`、`antigravity`、`workbuddy`、`codebuddy`、`qoder` / `qwenwork`、`cursor`、`windsurf`、`cline`、`roocode`、`hermes`、`kiro`、`devin`、`trae`、`factory-droid` / `droid`
4. **无图标兜底**：
   - 卡片/市场列表：双字母缩写圆角徽章；
   - 左侧菜单（展开与折叠态）：**圆角方框 + 菜单标题首字**（折叠态仅留单图标 + Tooltip）。

---

## 7. 自定义插件市场源（`registry.json`）

用户可在「插件中心 → 插件市场 → 管理市场源」注册任意 HTTPS JSON 市场地址，格式与 Magpie 市场完全兼容：

```json
{
  "version": 1,
  "plugins": [
    {
      "package": "@your-org/page-usage-card",
      "name": "Usage Card",
      "kind": "page",
      "icon": "https://example.com/icon.svg",
      "community": true,
      "summary": {
        "zh": "生成每日用量分享图并复制到剪贴板",
        "en": "Generate and copy daily usage summary cards"
      }
    }
  ]
}
```

- `kind` 支持：`provider`（缺省默认）、`middleware`、`page`、`both`；
- 多市场源自动按 `package` 合并去重；离线自动回退到本地缓存（内置源 `magpie-community` 另带编译期离线快照）。

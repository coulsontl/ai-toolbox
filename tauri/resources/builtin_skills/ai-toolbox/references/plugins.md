# 插件使用与开发手册

AI Toolbox 的插件体系兼容 [Magpie](https://github.com/magpie-community) 规范与 OpenCode v1 `auth` 插件格式。一个插件包可以只提供一种能力，也可以同时提供多种能力。

## 1. 插件类型

| 类型 | 声明位置 | 运行方式 | 典型用途 |
| --- | --- | --- | --- |
| Provider 渠道插件 | `magpie.providers` + `auth` hook | 按需启动 Bun 子进程 | 第三方订阅 / OAuth 渠道、多账号、额度、签到、模型发现、流式转发 |
| Gateway 中间件插件 | `magpie.middleware` | 网关进程内 `rquickjs` 沙箱 | 请求改写、模型重定向、系统提示词注入、SSE 帧改写、用量观测 |
| 自定义页面插件 | `magpie.page` / `magpie.pages` | 本机静态服务 + 沙箱 iframe | 数据看板、分享卡片、工具页、内嵌 Chat |
| 复合插件 | 上述任意组合 | 按需组合 | Provider + 页面、Provider + 中间件等 |

纯页面插件和纯中间件插件不依赖 Bun。只有 Provider 插件（或声明了 `service` 的页面）才会按需启动 Bun。

## 2. 安装、市场与 CLI 接入

### 2.1 安装来源

在「插件中心 → 安装插件」中支持：

1. npm 包名：`@scope/pkg`、`@scope/pkg@1.2.3`、`my-plugin`
2. GitHub 仓库：`github:owner/repo` 或 `github:owner/repo#branch`
3. 本机插件目录：绝对路径或 `file://` URL。本机目录是原地引用，AI Toolbox 不复制、不删除源码。
4. 单文件中间件：直接传入 `*.middleware.js` 路径。

### 2.2 多市场注册

「插件中心 → 插件市场 → 管理市场源」可以注册任意 HTTPS JSON 地址。市场格式与 Magpie 兼容：

```json
{
  "version": 1,
  "plugins": [
    {
      "package": "@your-org/provider-demo",
      "name": "Demo Provider",
      "kind": "provider",
      "icon": "openai",
      "providers": ["demo"],
      "summary": { "zh": "接入示例渠道", "en": "Connect the demo provider" }
    },
    {
      "package": "@your-org/page-demo",
      "name": "Demo Page",
      "kind": "page",
      "icon": "https://example.com/icon.svg",
      "community": true,
      "summary": { "zh": "示例页面插件", "en": "Demo page plugin" }
    }
  ]
}
```

- `kind` 支持 `provider`、`middleware`、`page`、`both`；缺省按 `provider` 处理。
- 多个市场源包含同一个 `package` 时会合并去重，并记录所有来源。
- 网络失败时回退到各市场源的本地缓存；内置 `magpie-community` 还有编译期离线兜底。

### 2.3 渠道账号与 16 个 CLI

Provider 插件登录后，可以在「插件中心 → 渠道与账号 → 添加到 CLI」同步到以下 16 个 CLI：

Claude Code、Claude Desktop、Codex、Grok、Kimi、Gemini CLI、ZCode、WorkBuddy、CodeBuddy、OpenCode、OpenClaw、Pi、Oh My Pi、OMO Native、Hermes、DSH。

也可以在对应 CLI Tab 中点击「从插件导入」。Antigravity CLI 只支持 Google 官方 OAuth，不支持添加自定义供应商。

账号能力包括：

- 浏览器 OAuth / Device Code、回调、API Key、从本地 CLI 文件导入登录。
- 额度查询、重置倒计时、每日签到。
- OAuth 后台自动续期；续期失败会标记账号，不会静默使用过期凭据。
- 多账号轮询与 429 冷却切换由网关负责，插件只需要提供账号级 `fetch` / 模型 / 用量能力。

### 2.4 外部客户端直连

不需要经过 CLI 接管，外部客户端也可以直接访问本机网关的插件路由：

```text
http://127.0.0.1:<gateway_port>/plugin/<provider_id>/v1
```

同时支持：

```text
/plugin/<provider_id>/v1/chat/completions
/plugin/<provider_id>/v1/responses
/plugin/<provider_id>/v1/messages
/plugin/<provider_id>/v1beta/...
```

网关会自动做跨协议转换，并使用该 Provider 账号的凭据与多账号轮询。

## 3. 编写 Provider 插件

最小可运行结构：

```text
my-provider-plugin/
├── package.json
└── index.js
```

`package.json`：

```json
{
  "name": "my-provider-plugin",
  "version": "0.1.0",
  "type": "module",
  "main": "index.js",
  "magpie": {
    "providers": ["my-provider"],
    "icon": "openai",
    "summary": {
      "zh": "接入 my-provider 的示例插件",
      "en": "Example plugin for my-provider"
    }
  }
}
```

`index.js`：

```js
export default async function plugin(input) {
  return {
    auth: {
      provider: "my-provider",

      // 每个账号请求前执行一次，返回 fetch 需要的连接信息。
      async loader(getAuth) {
        const auth = await getAuth();
        return {
          baseURL: "https://api.example.com/v1",
          apiKey: auth.key ?? auth.access,
          headers: { "x-client": "ai-toolbox" },
        };
      },

      methods: [
        { type: "api", label: "API Key" },
        {
          type: "oauth",
          label: "浏览器登录",
          authorize(inputs, { signal }) {
            return {
              url: "https://example.com/oauth/authorize?client_id=...",
              method: "code",
              instructions: "登录后把回调地址中的 code 粘贴回来。",
              async callback(code, { signal }) {
                // 这里用 code 换取 token。
                return {
                  type: "success",
                  access: "access-token",
                  refresh: "refresh-token",
                  expires: Date.now() + 3600_000,
                };
              },
            };
          },
        },
      ],

      // auth.options 会渲染为「渠道与账号」里的开关并持久化。
      options: [
        {
          key: "fast_mode",
          label: "Fast mode",
          description: "使用低延迟线路",
          default: false,
        },
      ],

      // 可选：额度查询。
      async usage(getAuth, provider) {
        const auth = await getAuth();
        return {
          plan: "Pro",
          windows: [
            {
              name: "Weekly",
              used: 42,
              resetsAt: "2026-12-01T00:00:00Z",
            },
          ],
        };
      },

      // 可选：每日签到。
      async checkin(getAuth, provider) {
        return { outcome: "claimed", credit: 10, message: "签到成功" };
      },
    },

    // Provider hook 负责模型发现。返回 id -> 模型 的对象。
    provider: {
      id: "my-provider",
      async models(models, ctx) {
        return {
          "example-model": {
            name: "Example Model",
            api: { npm: "@ai-sdk/openai-compatible" },
            limit: { context: 128000, output: 8192 },
            capabilities: {
              reasoning: true,
              tool_call: true,
              modalities: { input: ["text"], output: ["text"] },
            },
          },
        };
      },
    },
  };
}
```

要点：

- `auth.loader(getAuth, provider)` 返回的 `baseURL` / `apiKey` / `headers` / `fetch` 决定上游请求怎么发。
- `provider.models(models, ctx)` 返回模型表；`api.npm` 决定目标协议。
- `auth.methods` 的元素类型为 `api` 或 `oauth`。`api` 方法可以只用内置 Key 输入框；声明 `prompts` + `authorize` 时可以自己追问多个字段。
- `magpie.providers` 是插件声明的 provider ID 列表；`auth.provider` 是其中实际负责登录的 ID。
- 不要手工修改 `plugin-auth.json`。凭据以 `0600` 权限保存在应用数据目录，日志会自动脱敏。

### 3.1 `npm` 与协议映射

| 模型声明的 `npm` | 网关目标协议 |
| --- | --- |
| `@ai-sdk/anthropic` | Anthropic Messages |
| `@ai-sdk/openai` | OpenAI Responses |
| `@ai-sdk/openai-compatible`、`@ai-sdk/openrouter`、`@ai-sdk/deepseek`、`@ai-sdk/xai` | OpenAI Chat |
| `@ai-sdk/google`、`@ai-sdk/google-vertex` | Gemini Native |

模型级 `api.npm` 优先于 provider 默认 `npm`。

### 3.2 `auth.options`

每个选项：

```js
{
  key: "fast_mode",          // [a-z0-9_-]{1,40}
  label: "Fast mode",        // 必填
  description: "低延迟线路",  // 可选
  default: false,            // 可选
  confirm: "确认文案",        // 可选
  labels: { "zh": "快速模式" },
  descriptions: { "zh": "使用低延迟线路" },
  confirms: { "zh": "确认文案" },
  filtersModels: true,       // 开启时可能改变模型列表
  placement: "top",          // 或省略，放在模型列表下方
}
```

插件通过 `input.magpie.option(providerId, key)` 读取用户值。

## 4. 编写 Gateway 中间件插件

单文件形式：`redact.middleware.js`。包目录形式：

```json
{
  "name": "@your-scope/middleware-demo",
  "version": "0.1.0",
  "type": "module",
  "magpie": {
    "middleware": "./index.mjs",
    "summary": { "zh": "示例网关中间件", "en": "Demo gateway middleware" }
  }
}
```

`index.mjs`：

```js
// 可选：只让 onEvent 处理这些 SSE 事件名。
export const events = ["content_block_delta", "message_delta"];

export function onRequest(body, ctx) {
  // ctx.info: { protocol, model, stream, path, agent, caller }
  // ctx.options: 该插件保存的配置对象
  if (body.model === "blocked-model") {
    ctx.reject(400, "blocked-model is not allowed");
  }
  return body;
}

export function onProvider(body, ctx) {
  // 每个 attempt 拿到独立副本。
  // ctx.to: { id, name, account, model }
  return body;
}

export function onEvent(data, ctx) {
  // data 是 SSE 帧中的 JSON data 对象。
  // 返回对象：替换；返回 null：丢弃该事件；返回 undefined：保持原样。
  if (data?.type === "content_block_delta" && data.delta?.text) {
    data.delta.text = data.delta.text.replace(/secret/gi, "***");
  }
  return data;
}

export function onResponse(body, ctx) {
  // 修改非流式完整响应体。
  return body;
}

export function onUsage(usage, ctx) {
  // 异步用量观测，不阻塞主链路。
  console.log("usage", usage);
}
```

### 4.1 运行契约

- 可以只导出部分 Hook；未导出的 Hook 在热路径零开销跳过。
- 单次调用内存上限 `32 MiB`，调用栈 `512 KiB`。
- 模块加载上限 `1000 ms`；`onRequest` / `onProvider` / `onResponse` / `onUsage` 上限 `250 ms`；`onEvent` 上限 `50 ms`。
- **禁止篡改顶层 `stream` 字段**。`onRequest` / `onProvider` 新增或修改 `stream` 会被回滚并计入失败。
- 除显式 `ctx.reject(status, message)` 外，抛错、超时或非法返回值都 fail-open，请求继续发往上游。
- 沙箱内没有 `node_modules` 解析和网络 IO，只允许包内相对 import。需要第三方库时先用 esbuild / bun build 打包进入口文件。

### 4.2 协议标识

`ctx.info.protocol` 来自网关路由：

- 经过协议转换的请求常见值为 `anthropic_messages`、`openai_chat`、`openai_responses`、`gemini_native`。
- 部分直通路由会给出 `anthropic`、`gemini` 等 route name。

判断时优先用前缀或包含匹配，不要只假设某一种拼写。Magpie 生态中也常用 `anthropic` / `chat` / `responses` / `gemini` 这组简称。

## 5. 编写自定义页面插件

### 5.1 单页声明

```json
{
  "name": "@your-scope/page-demo",
  "version": "0.1.0",
  "magpie": {
    "icon": "page/icon.svg",
    "summary": { "zh": "示例页面", "en": "Demo page" },
    "permissions": ["usage:summary", "usage:day", "providers:list"],
    "page": {
      "title": "Demo Page",
      "titles": { "zh": "示例页面", "en": "Demo Page" },
      "file": "page/index.html",
      "icon": "page/icon.svg",
      "keepAlive": false
    }
  }
}
```

### 5.2 多页声明

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
        "keepAlive": true
      },
      {
        "id": "tools",
        "title": "Tools",
        "titles": { "zh": "工具", "en": "Tools" },
        "file": "pages/tools/index.html"
      }
    ]
  }
}
```

页面字段：

- `file` 必填，必须是插件根目录内的 `.html` / `.htm`，禁止绝对路径和 `..`。
- `icon` 支持 `.svg` / `.png` / `.jpg` / `.jpeg` / `.webp` / `.ico`，最大 `256 KiB`。
- `keepAlive` 默认 `false`，离开后闲置 10 分钟回收；设为 `true` 时持续保活。
- `service` 可声明 `{ "port": 18080, "option": "port" }`。声明后 CSP 会放行对应 `127.0.0.1` 的 HTTP / WebSocket。
- 静态资源根目录严格限制在入口 HTML 的父目录内。

### 5.3 页面 Bridge

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
        const options = await window.magpie.options.get();
        const summary = await window.magpie.read("usage:summary", { days: 7 });

        window.magpie.onTheme((nextTheme) => {
          console.log("theme", nextTheme.dark, nextTheme.lang);
        });
        window.magpie.onVisibility((visible) => {
          if (visible) {
            // 页面重新回到前台时刷新数据。
          }
        });

        document.getElementById("app").textContent =
          `最近 7 天请求数：${summary.requests}`;
      })();
    </script>
  </body>
</html>
```

`window.aiToolboxPlugin` 是 `window.magpie` 的完整别名。

全部 API：

| API | 说明 |
| --- | --- |
| `await magpie.ready` | 等待握手，返回 `{ dark, lang, tokens }` |
| `magpie.theme` | 当前主题快照 |
| `magpie.onTheme(fn)` | 主题或语言变化 |
| `magpie.visible` / `magpie.onVisibility(fn)` | 页面是否在前台 |
| `magpie.context` | `{ pluginId, pageId }` |
| `magpie.service` | 有本地服务时为 URL，否则为空字符串 |
| `magpie.options.get()` / `options.set(obj)` | 当前插件的持久化配置 |
| `magpie.read(scope, params?)` | 读取宿主数据，需要声明权限 |
| `magpie.image.copy(png)` | 写系统剪贴板，PNG 最大 20 MiB |
| `magpie.image.save(png, filename?)` | 保存到下载目录，返回 `{ path }` |
| `magpie.storage.get/set/delete/list/clear` | 插件私有 KV，需要 `storage:kv` |
| `magpie.files.readText/writeText/readBytes/writeBytes/list/stat/delete` | 插件私有文件树，需要 `storage:files` |
| `magpie.openPlugin(packageName)` | 打开安装弹窗并预填包名，仍需用户确认 |
| `magpie.onAction(fn)` | 面板动作回调 |
| `magpie.error(err)` | 上报致命错误，触发错误边界与「重试」 |

### 5.4 只读权限

| Permission | 返回 |
| --- | --- |
| `usage:summary` | `{ requests, input, output, cacheRead, cacheWrite, totalTokens, cost, days }` |
| `usage:day` | `{ date, requests, input, output, cacheRead, cacheWrite, totalTokens, cost, byModel, byProvider }` |
| `providers:list` | `Array<{ id, name, cliKey, enabled }>`，只返回非敏感摘要 |

`providers:list` 永远不会返回 API Key、Token、Header 或账号凭据。

### 5.5 存储配额

- `storage:kv`：单个值 ≤ 1 MiB，整体 ≤ 20 MiB。
- `storage:files`：单文件 ≤ 20 MiB，总量 ≤ 200 MiB。
- 路径逐段拒绝绝对路径、`..`、反斜杠、NUL、空段、盘符、Windows 保留名、超过 16 层深度和 symlink 组件。
- 页面运行在 `sandbox="allow-scripts"` iframe 中，默认 `connect-src 'none'`，不能访问 `__TAURI__`、文件系统或外网。

### 5.6 主题变量

Bridge 会自动设置：

```css
:root {
  --magpie-bg: ...;
  --magpie-fg: ...;
  --magpie-card: ...;
  --magpie-line: ...;
  --magpie-muted: ...;
  --magpie-accent: ...;
  --magpie-font: ...;
}
```

同时设置 `document.documentElement.dataset.theme` 为 `light` / `dark`，并注入低优先级基础样式。页面应优先使用这些变量，不要硬编码亮色或暗色。

## 6. 本地调试闭环（Agent 优先用 `aitoolbox`）

### 6.1 编写 → 安装 → 热重载 → 验收

```bash
# 1) 写代码：页面/Provider/中间件完成后，确认 package.json 清单
# 2) 安装本地目录（原地引用，不复制源码）
aitoolbox plugin install /absolute/path/to/my-plugin --json

# 3) 改完代码后热重载：重扫清单 → 刷中间件 → 同步页面服务 → 必要时重启 Bun host
aitoolbox plugin reload my-plugin-id --json
# 或只重载全部
aitoolbox plugin reload --json

# 4) 检查宿主状态
aitoolbox plugin list --json
aitoolbox plugin get my-plugin-id --json
aitoolbox plugin logs --limit 200 --json
aitoolbox plugin middleware --json
aitoolbox plugin settings --json
aitoolbox plugin bun-status --json
aitoolbox plugin host-status --json
aitoolbox plugin pages --json

# 5) Provider 插件：检查账号 → 刷新模型 → 添加到 CLI
aitoolbox plugin providers --json
aitoolbox plugin account-check my-provider --json
aitoolbox plugin account-usage my-provider --json
aitoolbox plugin refresh-models --json
aitoolbox plugin link-cli my-provider --json-body ./link-cli.json

# 6) 跑到 CLI 侧验收
aitoolbox provider list --cli codex --json
aitoolbox provider test --cli codex --id <providerId> --json
```

`link-cli.json` 的最小形态（`pluginSpec` 必须与 `aitoolbox plugin providers --json` 里的插件 spec 一致；URL 路径 provider 与 body 的 `providerId` 不一致会 400）：

```json
{
  "cliKey": "codex",
  "pluginSpec": "@acme/ai-plugin",
  "providerId": "my-provider",
  "displayName": "My Provider",
  "accountStrategy": "auto_rotate",
  "modelIds": []
}
```

- 只刷新某一个 Provider 时直接 POST 四段路由（`/v1/plugins/providers/<provider>/refresh-models`）；CLI 的 `plugin refresh-models` 刷新全部。
- `plugin reload` 是 L1 可逆写操作。默认授权模式下会弹窗；设置页可以单独开启「本地插件重载免确认」，但只对本地目录来源生效。
- 页面清单的 `file` 或 `service.port` 改动后，旧页面 token 会被撤销；下次打开页面重新签发，不要沿用旧标签页。
- `plugin install` 本地目录是 L1；`npm` / `github:` 远程来源是 L2，每次都要用户确认。
- 改 `package.json` 的页面清单、权限、图标、Provider 列表后必须 reload，宿主才会重新探测。
- 改 Provider / 中间件代码后，reload 会按需重启对应运行时；如果模块有顶层缓存，禁用再启用或重启宿主更彻底。
- 纯页面插件和纯中间件插件不需要 Bun；页面服务是 `127.0.0.1` 上的按需静态服务。
- 插件全局设置也可用 CLI 维护：`aitoolbox plugin settings` 读取；`--bun-path` / `--clear-bun-path`、`--npm-registry` / `--china-mirror`、`--follow-app-proxy` 做局部更新。只想改一个字段时不要先 GET 再整体 PUT，后端会保留未传字段。
- 渠道级每日自动签到用 `aitoolbox plugin provider-auto-checkin <provider> --enabled true|false`；渠道选项用 `aitoolbox plugin provider-options <provider> [--set '<JSON>']`。
- Bun 宿主需要复位时用 `aitoolbox plugin host-restart`（L1，可逆但有中断）；`aitoolbox plugin bun-probe <path>` 只验证一个可执行文件，不写入设置。

### 6.2 Symptom → Check 故障表

| 现象 | 优先检查 | 修复 |
| --- | --- | --- |
| 插件列表里没有刚写的插件 | `aitoolbox plugin list`；manifest `magpie` 字段是否合法；目录是否含 `package.json` | 修正 manifest 后 `aitoolbox plugin reload` |
| 页面 404 / 白屏 | `aitoolbox plugin logs`；页面 `file` 是否在插件根内、扩展名是否为 `.html` | 修正路径/文件后 reload |
| 页面 Bridge 报权限错误 | `package.json` 的 `permissions` 是否声明对应 scope | 补权限并 reload |
| 中间件完全没生效 | `aitoolbox plugin middleware` 是否显示 running；插件是否启用；`events` 导出是否过滤掉了目标事件 | 启用/reload；检查 hook 导出与事件名 |
| 中间件超时或报错 | `aitoolbox plugin logs`；单次调用是否超过 250ms / 32MiB | 缩减逻辑或打包依赖；失败是 fail-open，不会中断请求 |
| Provider 登录失败 | `aitoolbox plugin account-check <provider>`；OAuth 回调/设备码是否过期 | 重新登录；不要手工改 `plugin-auth.json` |
| Provider 插件提示找不到 Bun | `aitoolbox plugin bun-status`；`bunManualPath` 与实际路径 | `aitoolbox plugin bun-ensure` 或 `plugin settings --bun-path <path>`；先 `plugin bun-probe <path>` 验证 |
| Bun 宿主崩溃 / 请求一直 Pending | `aitoolbox plugin host-status`（`crashes`、`circuitOpen`）与 `plugin logs` | 修复插件代码后 `plugin reload`；必要时 `plugin host-restart` |
| npm 安装插件很慢或超时 | `aitoolbox plugin settings` 的 `npmRegistry` 与 `followAppProxy` | 切国内镜像：`aitoolbox plugin settings --china-mirror true` |
| Provider 模型列表为空 | `aitoolbox plugin refresh-models`；`auth.loader` 的 baseURL/apiKey；`provider.models` 返回值 | 修正 loader 或模型 hook 后 reload |
| 添加到 CLI 后仍不通 | `aitoolbox provider list --cli <key>`；`aitoolbox gateway clis --json` 看目标 CLI 是否已被接管；Base URL 是否指向网关 | 直接 `aitoolbox provider test`（后端用原始凭据）；不要先 `restore`，否则会丢 aggregate 配置 |
| 请求日志里看不到插件流量 | 网关是否启动；客户端是否真的经网关；插件 Provider 是否被选中 | `aitoolbox gateway status`、`aitoolbox gateway logs` |
| 修改代码后行为没变 | 是否执行了 `aitoolbox plugin reload`；Bun host 是否被复用 | reload 指定 id；必要时 disable→enable |

### 6.3 安全边界

- 调试日志、终端输出、聊天记录里都不要出现 API Key / OAuth token / `plugin-auth.json` 明文。
- Agent 不要为了“快速验证”去直接改 `plugin-auth.json`、数据库或网关内部状态；用插件命令和授权弹窗。
- 远程插件安装前先确认来源可信；中间件在沙箱内运行，Provider 插件拥有完整 Node 能力。
- 卸载默认保留插件私有数据；只有显式 `--purge-data` 才删除 `plugins/data/<plugin>`，删除前必须让用户确认。

## 7. 安全与边界

- 不要把 API Key、OAuth token、`plugin-auth.json` 明文写进日志、页面或仓库。
- 页面只能读取声明过的权限；未知 scope 会被后端拒绝。
- Provider 插件运行在 Bun 子进程，拥有完整 Node 能力，安装第三方插件前应确认来源可信。
- 中间件运行在 `rquickjs` 沙箱，不能访问网络、文件系统或宿主 API。
- 插件卸载默认保留私有数据；只有显式选择清除数据才会删除 `plugins/data/<plugin>`。

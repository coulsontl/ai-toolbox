# 插件使用与开发手册

AI Toolbox 的插件体系兼容 [Magpie](https://github.com/magpie-community) 规范与 OpenCode v1 `auth` 插件格式。一个插件包可以只提供一种能力，也可以同时提供多种能力。安装时宿主会探测并持久化一份能力矩阵（`contributions`），其中本版本尚未消费的能力会明确列在 `unsupportedFeatures` 中，不会静默失效。

## 1. 插件类型

| 类型 | 声明位置 | 运行方式 | 典型用途 |
| --- | --- | --- | --- |
| Provider 渠道插件 | `magpie.providers` + `auth` hook | 按需启动 Bun 子进程 | 第三方订阅 / OAuth 渠道、多账号、额度、签到、模型发现、流式转发 |
| Gateway 中间件插件 | `magpie.middleware` | 网关进程内 `rquickjs` 沙箱 | 请求改写、模型重定向、系统提示词注入、SSE 帧改写、`onAccount` 账号过滤、用量观测 |
| Gateway 异步钩子 | `magpie.hooks` 或 `*.hooks.{js,mjs,ts,mts}` | 按需 Bun 宿主内异步钩子 | `onRequest` / `onResponse` 观测与请求改写、跨步骤诊断 |
| 自定义页面插件 | `magpie.page` / `magpie.pages` | 本机静态服务 + 沙箱 iframe | 数据看板、分享卡片、工具页、内嵌 Chat |
| 菜单动作 | `magpie.actions` | 宿主菜单 + 插件页面 Bridge | 从渠道/Agent 菜单打开插件页面并携带动作 |
| 内嵌面板 | `magpie.panels` | 宿主页面插槽 + 沙箱 iframe | 在渠道或设置页里嵌入轻量信息面板 |
| 插件工具（MCP） | Provider 插件入口导出的 `tool` 钩子 | 按需 Bun 宿主 + 网关 MCP 服务端 | 把插件工具通过 Streamable HTTP MCP 提供给任意 Agent |
| 复合插件 | 上述任意组合 | 按需组合 | Provider + 页面、Provider + 中间件 + 工具等 |

纯页面、纯 panel 和只读 `rquickjs` 中间件不依赖 Bun。Provider 插件、异步 `magpie.hooks`、插件 `tool` 钩子，以及声明了 `service` 且包含宿主入口的页面才会按需启动 Bun。

## 2. 安装、市场与 CLI 接入

### 2.1 安装来源

在「插件中心 → 安装插件」中支持：

1. npm 包名：`@scope/pkg`、`@scope/pkg@1.2.3`、`my-plugin`
2. GitHub 仓库：`github:owner/repo` 或 `github:owner/repo#branch`
3. 本机插件目录：绝对路径或 `file://` URL。本机目录是原地引用，AI Toolbox 不复制、不删除源码。
4. 单文件中间件：`*.middleware.js`、`*.middleware.mjs`、`*.gateway.js`、`*.gateway.mjs`，在网关进程内 `rquickjs` 沙箱运行。
5. 单文件异步钩子：`*.hooks.js`、`*.hooks.mjs`、`*.hooks.ts`、`*.hooks.mts`，在 Bun 宿主内运行；后缀与中间件、`magpie.gateway` 绝不重叠。

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
- 多账号轮询、账号级 401 刷新、429 冷却与配额感知选号由网关负责，插件只需要提供账号级 `fetch` / 模型 / 用量能力。
- `auth.single: true` 表示渠道只允许一个账号；界面会禁用再次登录入口，避免创建第二个账号。
- `provider.models` 可以按账号返回不同模型列表（`accounts[].models`）；`Single` 策略下，“添加到 CLI”会按被固定账号的模型列表收窄。
- `auth.rotating: true` 表示 refresh token 是一次性的；宿主不会把该登录态复制到其他机器。

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
| `@ai-sdk/anthropic`、`@ai-sdk/google-vertex/anthropic` | Anthropic Messages |
| `@ai-sdk/openai`、`@ai-sdk/azure` | OpenAI Responses |
| `@ai-sdk/openai-compatible`、`@ai-sdk/openrouter`、`@ai-sdk/deepseek`、`@ai-sdk/xai`、`@ai-sdk/github-copilot` | OpenAI Chat |
| `@ai-sdk/google`、`@ai-sdk/google-vertex` | Gemini Native |

模型级 `api.npm` 优先于 provider 默认 `npm`；模型级 `api.url` 优先于 provider 的 `api` 基址。`npm` 值可以用逗号、分号或空格分隔多个包，宿主取第一个非空项。

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

### 3.3 模型与账号保真字段

`provider.models(models, ctx)` 返回的每个模型除了 `name` / `limit` / `capabilities`，还可以声明以下字段；宿主会透传给网关与「添加到 CLI」链路：

```js
{
  "my-model": {
    name: "My Model",
    api: {
      npm: "@ai-sdk/openai-compatible",
      url: "https://api.example.com/v2", // 模型级基址，覆盖 provider 的 api
      id: "vendor-model-id",             // 厂商模型 id，当与 key 不同时使用
    },
    capabilities: {
      output: ["text"],                  // 声明输出类型；不声明视为可对话
      input: { text: true, image: true },
    },
    limit: { context: 128000, output: 8192 },
  },
}
```

| 字段 | 含义 | 网关行为 |
| --- | --- | --- |
| `api.url` | 模型自己的 API 基址 | 优先于 provider 的 `api` 基址 |
| `api.id` | 厂商自己的模型 id | 与公开 `id` 不同时作为真正发往上游的模型 ID（用户显式模型映射优先） |
| `api.npm` | 模型使用的 AI SDK 包 | 优先于 provider 默认 npm，决定目标协议 |
| `capabilities.output` / `outputsSaid` | 模型输出类型 | 未声明时视为可对话；声明后必须包含 `text` 才会出现在聊天 CLI 的模型列表 |
| `accounts[].models` | 每个账号可用的模型 | `Single` 策略下按固定账号的列表收窄 |
| `auth.single` | 渠道只允许一个账号 | 界面禁用再次登录 |
| `auth.keepAlive` / `refreshLead` | 后台续期与提前量 | 宿主报告 `renews` / `renewLead`，后台调度按提前量续期；缺失时按 15 分钟 |
| `auth.rotating` | refresh token 一次性 | 不跨机器复制登录态 |

`outputsSaid` 是宿主根据插件是否真的声明了 `capabilities.output` 生成的布尔值。**非对话模型不会静默消失**：它们在插件的模型列表里仍可见，只是不会进入聊天 CLI 的模型下拉。

### 3.4 代理、`caller` 与 `session` 透传

- 应用设置 `followAppProxy` 开启时，插件宿主的市场/npm 请求与 Provider 请求会走应用代理。当前实现只透传**自定义 HTTP/HTTPS 代理地址**；应用处于**系统代理**模式时 Bun 不会原生读取，等同直连。`providers` RPC 按 `{ proxies: { [providerId]: { [account]: proxyUrl } } }` 传入；`usage` / `checkin` / `check` / `keepSignedIn` 用单值 `proxy` 字段。
- 网关把调用方身份放进 `ctx.caller`：优先解析入站 `x-magpie-caller-agent` / `x-magpie-caller-name` / `x-magpie-caller-via`（逗号分隔、百分号解码）；没有这些头时合成 `{ agent, name, via: [] }`，`agent` 是 CLI key，`name` 是 route name。
- 插件 `fetch` 会从 `x-magpie-session`、`x-claude-code-session-id`、`x-session-id`、`session_id` 提取 `session`，传给 `chat.headers`；异步 `magpie.hooks` 的 `ctx.session` 使用同一组头。

### 3.5 账号级 401 / 429 与配额感知轮询

- `401` 且响应头 `x-magpie-sign-in: expired`：宿主已判定该账号凭据失效。网关冷却该账号 300 秒；`AutoRotate` 且有其它候选时切号。**不要**把它当成可恢复的缓存陈旧。
- 其它 `401`：网关调用宿主 `reload` RPC（只清该账号的 loader/auth 缓存），同账号重试一次；**绝不重启 Bun 宿主**，否则会中断所有在途请求。
- `429`：解析 `Retry-After`（十进制秒数或 HTTP-date，clamp 5–3600 秒；缺失回落 60 秒）冷却该账号；`AutoRotate` 最多切换 4 次，全部账号都在冷却时把 429 原样返回给客户端。
- 配额感知选号只看已持久化的账号 `quota`：`used` 是 0–100 的百分比，`limit` 是窗口额度数量（不是 `used` 的分母）；任一窗口 `used >= 100` 且未标 `aside: true`、`resetsAt` 未过，或没有窗口时 `balance <= 0`，就把该账号降权。所有账号都耗尽时仍回退全部候选，坏快照不会让账号彻底不可用。

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
  // ctx: { protocol, model, stream, path, agent, caller, options, state }
  // ctx.options: 该插件保存的配置对象
  if (body.model === "blocked-model") {
    ctx.reject(400, "blocked-model is not allowed");
  }
  return body;
}

export function onProvider(body, ctx) {
  // 每个 attempt 拿到独立副本。
  // ctx.provider: { id, name, account, model }
  return body;
}

export function onAccount(account, ctx) {
  // account: { ref, provider, name, kind, model }
  // ref 是每中间件隔离的 HMAC 匿名标识，不是账号 key，也不能反推账号。
  // 只有返回对象且 skip === true 才跳过该账号；其它返回值/抛错/超时都放行。
  if (account.kind === "codex" && account.model === "blocked-model") {
    return { skip: true };
  }
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
- 模块加载上限 `1000 ms`；`onRequest` / `onProvider` / `onAccount` / `onResponse` / `onUsage` 上限 `250 ms`；`onEvent` 上限 `50 ms`。
- **禁止篡改顶层 `stream` 字段**。`onRequest` / `onProvider` 新增或修改 `stream` 会被回滚并计入失败。
- 除显式 `ctx.reject(status, message)` 外，抛错、超时或非法返回值都 fail-open，请求继续发往上游。
- 沙箱内没有 `node_modules` 解析和网络 IO，只允许包内相对 import。需要第三方库时先用 esbuild / bun build 打包进入口文件。

### 4.2 `onAccount` 账号过滤

`onAccount(account, ctx)` 在网关为一次插件请求选出候选账号后、真正发送前执行：

- `account` = `{ ref, provider, name, kind, model }`。`ref` 是 16 字节 hex 的 HMAC-SHA256 匿名标识，按「中间件 + provider + 账号 key」隔离；同一账号在不同中间件里得到不同 `ref`，不能反推账号 key。
- 返回 `{ skip: true }` 才会跳过该账号（网关继续尝试其它候选；全部被跳过时回退原选号）。
- `null` / `undefined` / 抛错 / 超时 / 其它返回值一律 fail-open，该账号照常使用。
- 同一请求的后续钩子可以通过 `ctx.account` 读到这次 `onAccount` 看到的账号对象。
- HMAC 密钥保存在 `<app_data>/plugins/account-ref.key`（32 字节 hex，首次创建、已有绝不覆盖）；读不到时退化为本次运行的随机密钥，重启后旧 `ref` 会变化。

### 4.3 协议标识

`ctx.protocol` 来自网关路由：

- 经过协议转换的请求常见值为 `anthropic_messages`、`openai_chat`、`openai_responses`、`gemini_native`。
- 部分直通路由会给出 `anthropic`、`gemini` 等 route name。

判断时优先用前缀或包含匹配，不要只假设某一种拼写。Magpie 生态中也常用 `anthropic` / `chat` / `responses` / `gemini` 这组简称。

### 4.4 异步 `magpie.hooks`（Bun 宿主）

单文件 `*.hooks.{js,mjs,ts,mts}`，或 `package.json` 的 `magpie.hooks`（字符串入口、`{ entry | path }` 对象、或 `true` 自动探测）。它和 `magpie.middleware` 是两套引擎：

| | `magpie.middleware` | `magpie.hooks` |
| --- | --- | --- |
| 引擎 | 网关进程内 `rquickjs` | 按需 Bun 宿主 |
| 位置 | 请求主链路，同步 | 网关前后异步钩子 |
| 单次预算 | `250 ms`（`onEvent` `50 ms`） | `gatewayTimeoutMs` 100–30000 ms（默认 1000） |
| 整链预算 | — | 2–10 s（单钩子配置更大时按其超时，最多 30 s；不含首次启动 Bun） |
| 失败行为 | fail-open | fail-open |
| 文件后缀 | `*.middleware.js` / `*.gateway.js` | `*.hooks.*` |

```js
export function onRequest(body, ctx) {
  // ctx: { protocol, model, stream, path, agent, session, cwd, query, caller,
  //        options, state, signal, report }
  // 返回对象替换请求体；返回 undefined 保持原样。
  // 顶层 stream / tools / tool_choice / function_call / functions / toolConfig / model 不允许改。
  return body;
}

export function onResponse(reply, ctx) {
  // ctx.success、ctx.request（请求体）。返回值不会被采用。
  // 用于观测、审计或外部记录；不要在这里尝试改写客户端响应。
}
```

- 遵循 Magpie 上游契约，模块必须至少导出 `onRequest` 或 `onSignIn`，否则宿主加载时报 `magpie.hooks exports no onRequest`；仅需 `onResponse` 观测时，请同时导出空操作 `export function onRequest() {}`。
- `magpie.hooks` 入口必须位于插件目录内；相对路径中的反斜杠 `\` 会先规范化为 `/`，随后由 `resolve_hooks_entry` 拒绝绝对路径、`..` 与逃逸符号链接。
- `onRequest` 的整条链预算为 2–10s（有钩子配置更大的 `gatewayTimeoutMs` 时按该钩子超时，最多 30s）；首次启动 Bun（含冷下载）在该预算之外，但会阻塞本次请求直到宿主就绪。`finish` / `close` 在后台执行，绝不阻塞客户端响应。
- 每个钩子可以设置 `options.gatewayTimeoutMs`（100–30000，默认 1000）。整链预算是各钩子超时之和，clamp 2–10s；单个钩子配置超过 10s 时按它自己的超时。
- Rust 侧还会兜底回滚被改写的顶层 `stream`；`tools` / `tool_choice` / `function_call` / `functions` / `toolConfig` / `model` 的改动会被宿主拒绝并计为失败。
- 同时最多 128 个 run，闲置 15 分钟回收；`GatewayHookRun` 的 Drop 会兜底 `close`，早退出不会泄漏 run。
- `ctx.report(code, bytes?)` 只接受固定诊断码（`ready` / `empty` / `unbound` / `unauthorized` / `unavailable` / `invalid` / `skipped`）和 ≤32 KiB 的字节数；异常文本、查询、凭据不会进入诊断。
- `gatewayStates` 由宿主提供，`aitoolbox plugin hooks` 读取 `calls` / `failures` / `avgMillis` / `lastOutcome` / `lastBytes`。Bun 未运行时返回空表，不会为了诊断启动 Bun。
- 纯 `magpie.hooks` 包（无 provider entry）归为 middleware kind，但只会被 Bun 宿主加载；不要把它写成 `magpie.middleware`，也不要指望 `rquickjs` 执行它。

### 4.5 网关请求明细里的插件改写

打开「Gateway → 请求明细」时，插件相关诊断只存在于 JSONL 详情，不进入 SQLite 摘要：

- Body tab 最多展示三段：收到（`request_body`）、实际发出（`upstream_request_body`，整流/协议转换/中间件改写后）、插件最终发出（`plugin_request_body`，Bun 宿主捕获的 `wire_request.body`）。只有与相邻段不同才显示。
- 「请求记录」段列出改写来源：`plugin_middleware_request` / `plugin_middleware_provider` / `plugin_middleware_response`（`rquickjs` 中间件三阶段）；`plugin_provider_fetch`（宿主真实发出的 URL/headers/body 与网关构造的请求有实质差异）；异步 `magpie.hooks` 的请求改写显示为 `magpie.hooks`。
- Headers tab 的「发往上游请求头（已脱敏）」来自 `upstream_request_headers`：写入前会对 `authorization` / `x-api-key` / `x-goog-api-key` / `cookie` / `*_token` / `*-api-key` 等敏感头脱敏。认证头由宿主重新注入，不参与 `plugin_provider_fetch` 的差异判定。

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
| `magpie.onAction(fn)` | 接收 `magpie.actions` 菜单动作：`{ id, provider }` 或 `{ id, agent: { id, name } }`；页面握手完成后才派发 |
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

### 5.7 `magpie.actions` 菜单动作

动作必须由带页面的插件声明（动作打开插件页面）。`package.json`：

```json
{
  "magpie": {
    "page": { "title": "Demo", "file": "page/index.html" },
    "actions": [
      {
        "id": "open-dashboard",
        "menu": "provider",
        "title": "Open dashboard",
        "titles": { "zh": "打开看板" }
      }
    ]
  }
}
```

- `id` 必须匹配 `^[a-z0-9][a-z0-9-]{0,39}$`；重复 id 只保留第一条。
- `menu` 只接受 `provider` / `agent`；最多 8 条。
- 宿主把动作注册到插件页面：`aitoolbox plugin actions` 返回 `{ pluginId, pluginName, pageId, id, menu, title, titles }`。
- 当前宿主在渠道头部派发 `menu: "provider"` 的动作；页面通过 `magpie.onAction(fn)` 接收 `{ id, provider }`（provider 菜单）或 `{ id, agent: { id, name } }`（agent 菜单）。动作在页面完成 Bridge 握手后才派发，页面重新打开不会重放旧动作。

### 5.8 `magpie.panels` 内嵌面板

面板是宿主页面里的沙箱 iframe，不拥有本地服务（CSP `connect-src 'none'`）：

```json
{
  "magpie": {
    "panels": [
      {
        "id": "provider-status",
        "slot": "provider",
        "title": "Status",
        "titles": { "zh": "状态" },
        "file": "panels/status.html"
      }
    ]
  }
}
```

- `slot` 只接受 `provider` / `settings` / `usage` / `agents`；当前宿主挂载 `provider`（渠道头部）与 `settings`（插件运行时设置页）。
- `file` 必须是插件根目录内的 `.html` / `.htm`；相对路径中的反斜杠会先规范化为 `/`，随后拒绝绝对路径、`..` 与逃逸符号链接。
- 显式 `id` 会清洗后使用；未填时生成 `<slot>-<序号>`；最多 8 个面板。
- 面板通过同一个 Bridge 握手，`magpie.context` 包含 `{ pluginId, panelId, slot, ... }`；Bridge 会自动观察文档高度并上报 `size` 事件，宿主把高度限制在 `80–720 px`、默认 `220 px`。
- 面板 token 作用域是 `panel:<panel_id>`；禁用/卸载插件会连同页面一起吊销。

### 5.9 能力矩阵与兼容性提示

安装探测结果保存在已安装记录的 `contributions`：

| 字段 | 含义 | 当前是否消费 |
| --- | --- | --- |
| `hasHooks` / `hooksEntry` | `magpie.hooks` 或 `*.hooks.*` | 是（Bun 宿主） |
| `hasPanels` | `magpie.panels` 非空 | 是 |
| `hasActions` | `magpie.actions` 非空 | 是 |
| `hasReplaces` | `magpie.page(s)[].replaces` 非空 | 否，列入 `unsupportedFeatures` |
| `hasAgent` | `magpie.agent` 非空 | 否，列入 `unsupportedFeatures` |
| `hasPrivacyModels` | `magpie.privacyModels` 非空 | 否，列入 `unsupportedFeatures` |
| `unsupportedFeatures` | 本版本不消费的能力名，固定顺序 `replaces` / `agent` / `privacyModels` | 只读提示 |

- 安装成功但存在 `unsupportedFeatures` 时，插件中心会显示明确 warning；这是提示而非静默失效。
- 如果包**只**声明了未消费的能力、又没有 provider/middleware/hooks/page/panel 入口，安装会失败，并在错误信息里逐项点名这些能力，而不是笼统报「不是插件」。
- `hooks` / `panels` / `actions` 已接入；不要把它们误写成不支持。

## 6. 插件工具与 MCP

Provider 插件的入口可以导出 OpenCode 的 `tool` 钩子；宿主会把它们聚合成一个 Streamable HTTP MCP 服务端，供任意 Agent/CLI 调用。

```js
export default async function plugin(input) {
  return {
    tool: {
      forecast: {
        description: "查询指定城市的天气",
        args: {
          type: "object",
          properties: { city: { type: "string" } },
          required: ["city"],
        },
        async execute(args, ctx) {
          return `Weather in ${args.city}`;
        },
      },
    },
  };
}
```

- `tool` 的名字必须匹配 `^[A-Za-z0-9_-]{1,48}$`；只有 `execute` 是函数的条目会被发布。
- `args` 可以是 zod shape、zod object 或 JSON Schema；zod v3/v4 会被转换成 JSON Schema，读不出的部分降级为 `{}`。
- 宿主为每个工具生成 MCP 名 `<插件短名>_<工具名>`：
  - 短名取包名去掉 scope/版本/git ref，去掉 `middleware-` / `opencode-` / `tools-` / `page-` / `agent-` 前缀和 `-auth` 后缀，清洗到 `[A-Za-z0-9_-]`，最多 24 字节，空则 `plugin`。
  - 全名最多 64 字节，按 UTF-8 字符边界截断；重名追加 `_2` / `_3`。
- MCP 端点：`http://127.0.0.1:<gateway_port>/mcp/plugins`，兼容别名 `/mcp/magpie/tools`。支持 `initialize` / `ping` / `tools/list` / `tools/call`；无 `id` 的 notification 返回 202；`DELETE` 返回 204；其它方法 405；请求体上限 4 MiB。
- 这两个端点只接受本机 loopback 连接：即使网关监听 `0.0.0.0`，局域网请求也返回 `403 {"error":"loopback_only"}`；工具在 Bun 宿主内拥有本机完整能力，不能暴露给局域网。
- `tools/call` 在 Bun 宿主里执行（55s 超时），结果包装为 MCP `content` + `isError`。这条路径不经过 provider，不计 token，也不触发网关的模型路由。
- 没有启用的 Provider 插件时，工具列表为空且不会拉起 Bun；`aitoolbox plugin tools` 可以只读查看端点、工具与 `mcpServerId`。
- `aitoolbox plugin sync-tools-mcp`（L1）会在中央 MCP 管理里创建/刷新 `ai-toolbox-plugins` HTTP server 并触发 `mcp_sync_all`；网关端口变化后重新执行。
- 工具在 Bun 宿主内拥有完整 Node 能力（与 Provider 插件相同），只安装可信插件。

## 7. 本地调试闭环（Agent 优先用 `aitoolbox`）

### 7.1 编写 → 安装 → 热重载 → 验收

```bash
# 1) 写代码：页面/Provider/中间件完成后，确认 package.json 清单
# 2) 安装本地目录（原地引用，不复制源码）
aitoolbox plugin install /absolute/path/to/my-plugin --json

# 3) 改完代码后热重载：重扫清单 → 刷中间件 → 同步页面服务 → 必要时重启 Bun host
aitoolbox plugin reload my-plugin-id --json
# 或只重载全部
aitoolbox plugin reload --json

# 3.1) npm/GitHub 插件查更新或更新（本地插件请继续用 reload）
aitoolbox plugin update --check-only --json
aitoolbox plugin update my-plugin-id --json
aitoolbox plugin update --json

# 4) 检查宿主状态
aitoolbox plugin list --json
aitoolbox plugin get my-plugin-id --json
aitoolbox plugin logs --limit 200 --json
aitoolbox plugin middleware --json
aitoolbox plugin settings --json
aitoolbox plugin bun-status --json
aitoolbox plugin host-status --json
aitoolbox plugin pages --json
aitoolbox plugin tools --json
aitoolbox plugin actions --json
aitoolbox plugin panels --json
aitoolbox plugin hooks --json

# 4.1) 把插件工具暴露给其他 Agent：同步到中央 MCP 管理（L1）
aitoolbox plugin sync-tools-mcp --json

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
- 插件全局设置也可用 CLI 维护：`aitoolbox plugin settings` 读取；`--bun-path` / `--clear-bun-path`、`--npm-registry` / `--china-mirror`、`--follow-app-proxy`、`--auto-update true|false`、`--auto-update-interval <HOURS>` 做局部更新。只想改一个字段时不要先 GET 再整体 PUT，后端会保留未传字段；`lastAutoUpdateCheckAt` 由后端维护，不要手工写。
- `plugin update --check-only` 只查询版本，不写 `lastAutoUpdateCheckAt`；实际更新会下载并执行新版本代码（L2），并保留原 `spec` / `options` / `enabled` / `installedAt` / `marketId`。锁定版本、本地插件、GitHub check-only 会返回 `skipped`；全量 npm 检查包含已停用插件，更新后仍保持停用。
- 渠道级每日自动签到用 `aitoolbox plugin provider-auto-checkin <provider> --enabled true|false`；渠道选项用 `aitoolbox plugin provider-options <provider> [--set '<JSON>']`。
- Bun 宿主需要复位时用 `aitoolbox plugin host-restart`（L1，可逆但有中断）；`aitoolbox plugin bun-probe <path>` 只验证一个可执行文件，不写入设置。

### 7.2 Symptom → Check 故障表

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
| npm 插件有新版本但没自动更新 | `aitoolbox plugin settings` 的 `autoUpdateEnabled` / `autoUpdateIntervalHours` / `lastAutoUpdateCheckAt`；插件是否 enabled；spec 是否锁定版本；Bun 是否可用 | 打开自动更新或 `aitoolbox plugin update`；锁定版本改为未锁定 spec 后重装；先 `plugin bun-ensure` |
| `plugin update` 提示 skipped | 来源是 local/GitHub check-only/锁定版本，或没有 npm 包名 | 本地插件用 `plugin reload`；GitHub 用单插件 `plugin update <id>`；锁定版本需改 spec |
| `plugin tools` 返回空表 | 插件是否有 Provider 入口并导出 `tool` 钩子；`execute` 是否为函数；工具名是否匹配 `^[A-Za-z0-9_-]{1,48}$` | 修正入口/工具定义后 `plugin reload` |
| MCP 客户端看不到插件工具 | 网关是否启动；配置的 URL 是否指向 `/mcp/plugins`；是否执行过 `plugin sync-tools-mcp` | 启动网关并重新 `plugin sync-tools-mcp`；`plugin tools` 核对 `gatewayRunning` 与 `mcpServerId` |
| 工具调用返回 `isError` | `plugin logs`；工具参数是否符合声明的 `args` schema；插件 `execute` 是否抛错或超过 55s | 修正参数/实现后 reload |
| `plugin actions` / `plugin panels` 为空 | 插件是否有页面（actions 依赖页面）；`menu` / `slot` 是否在允许值内；`file` 是否存在且在插件根内 | 修正 manifest 后 reload |
| `plugin hooks` 返回空表 | 是否有启用插件声明 `magpie.hooks`；Bun 宿主是否正在运行 | 先 `plugin host-status`；必要时触发一次请求或 `plugin host-restart`；不要为诊断强行拉起 Bun |

### 7.3 安全边界

- 调试日志、终端输出、聊天记录里都不要出现 API Key / OAuth token / `plugin-auth.json` 明文。
- Agent 不要为了“快速验证”去直接改 `plugin-auth.json`、数据库或网关内部状态；用插件命令和授权弹窗。
- 远程插件安装前先确认来源可信；中间件在沙箱内运行，Provider 插件拥有完整 Node 能力。
- 卸载默认保留插件私有数据；只有显式 `--purge-data` 才删除 `plugins/data/<plugin>`，删除前必须让用户确认。

## 8. 安全与边界

- 不要把 API Key、OAuth token、`plugin-auth.json` 明文写进日志、页面或仓库。
- 页面只能读取声明过的权限；未知 scope 会被后端拒绝。
- Provider 插件与其 `tool` 钩子运行在 Bun 子进程，拥有完整 Node 能力，安装第三方插件前应确认来源可信。
- 中间件运行在 `rquickjs` 沙箱，不能访问网络、文件系统或宿主 API。
- 插件卸载默认保留私有数据；只有显式选择清除数据才会删除 `plugins/data/<plugin>`。

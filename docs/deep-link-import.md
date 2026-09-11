# 供应商分享与跨工具导入

AI Toolbox 可以把当前供应商复用到其他 Coding 工具。分享弹窗提取 API 地址、密钥、实际协议和模型，按目标工具生成配置；可以直接导入本机，也可以复制 `aitoolbox://` 链接在另一台设备上确认导入。

这是一次配置复制。后续修改源供应商的 Key 或模型不会自动同步到已导入的副本。

## 使用流程

1. 在供应商卡片点击「分享」。支持 Claude Code、Claude Desktop、Codex、Grok CLI、Kimi Code、Gemini CLI、OpenCode、OpenClaw、Pi、Oh My Pi、Hermes 和 DeepSeek Harness。
2. 一个模型目录包含不同 URL、协议或 Key 时，先选择「源连接」，同一连接的模型一起带入。
3. 选择目标工具，检查名称、协议、Key、地址、模型列表和默认模型。地址适配后会显示目标工具使用的地址。
4. 选择重名策略：默认跳过已有项，也可以创建自动编号的副本。分享导入没有覆盖操作。
5. 点击「导入到本机」或「复制链接」。外部链接只唤起确认弹窗，点击「导入」才保存。

数据库型工具保存为未应用的供应商；文件型工具加入当前配置文件，保留原有默认模型、其他供应商及未知配置。导入成功后导航到目标工具并刷新页面和托盘。

## 目标与保存语义

| app | 工具 | 保存位置与模型适配 |
|---|---|---|
| claude | Claude Code | 独立供应商记录；环境变量、默认模型和角色模型 |
| claudedesktop | Claude Desktop | 独立供应商记录；Desktop 模型路由和显示名称 |
| codex | Codex | 独立供应商记录；auth、TOML、模型目录；默认模型独立于目录顺序 |
| grok | Grok CLI | 独立供应商记录；defaultModelKey 和含连接信息的模型目录 |
| kimi | Kimi Code | 独立供应商记录；providerConfigs 和模型目录 |
| gemini | Gemini CLI | 独立供应商记录；环境变量和默认模型 |
| opencode | OpenCode | 当前 opencode.json/jsonc；SDK、options、模型字典 |
| openclaw | OpenClaw | 当前 openclaw.json 的模型供应商 |
| pi | Pi | 当前根目录的 models.json |
| omp | Oh My Pi | 当前根目录的 models.yml |
| hermes | Hermes | 当前 config.yaml 的 custom_providers |
| dsh | DeepSeek Harness | 当前 settings.yaml 与 .credentials.yaml |

共享通用连接字段和目标能表达的模型名称、上下文、输出上限、推理、输入能力。权限、MCP、插件、提示词和工具专用高级配置不在分享范围内。

上游协议保持不变。目标只能使用另一种协议时，数据库型工具保留上游元数据并提示需要 Gateway；导入不会自动启用代理。Kimi 的直接客户端配置使用 openai_legacy，其他上游协议交给 Gateway；能力字段映射为 image_in、video_in、thinking。

内置 gatewayProfile 优先映射同一 profile、同一协议的目标 endpoint，保存引用而非兼容参数快照。目标没有对应 endpoint 时，仅在通用协议可用的情况下回退为自定义配置并提示；依赖特定网关适配器的组合会被拒绝。

原生文件型目标只接受静态请求头。DSH/Hermes 不导入自定义请求头或 Anthropic Bearer 认证；DSH 不接收 Gemini Native 路由。这些情况会在预览时说明原因。

OAuth、订阅登录的 access/refresh token，以及环境变量、文件或命令引用，不会被当作可分享的 API Key。需要独立 API Key 的来源会提示补填。OpenCode 官方卡片只读取选中渠道的 API-key 认证；内置元数据从本地模型缓存或 bundled defaults 补齐，不联网刷新，不以收藏历史代替当前配置。

链接包含密钥，只应发给可信接收者。链接超过 8,000 字符时停止复制并提示减少模型数量；直接导入本机不受此 URL 长度限制。

## URL 协议

继续使用 aitoolbox://v1/import。旧的 Claude/Codex/Gemini 链接仍可导入。

```text
aitoolbox://v1/import?resource=provider&sourceApp=opencode&app=codex&name=Relay&category=custom&apiFormat=openai_chat&apiKey=test-key&baseUrl=https%3A%2F%2Frelay.example%2Fv1&model=vendor%2Fmodel
```

参数值使用 URLSearchParams / application/x-www-form-urlencoded 编码；加号、中文、模型 ID 内的斜杠、JSON 和地址查询串都按参数值编码。

| 参数 | 语义 |
|---|---|
| resource | 必须为 provider |
| app | 必填，目标工具 ID，见上表 |
| name | 必填，非空名称 |
| category | 可选，默认 custom；兼容 official、third_party、aggregator 等旧值 |
| sourceApp | 新链接的源工具 ID，用于 SDK 地址适配；旧链接省略时保持原地址语义 |
| baseUrlStyle | 源连接地址为 root 或 versioned；区分同一工具原生 SDK 与 Gateway 配置的版本路径语义 |
| apiFormat | anthropic_messages、openai_chat、openai_responses、gemini_native、ollama/chat |
| apiVersion | 可选的 Gemini API 版本，和 SDK 的 Base URL 分开处理 |
| apiKey / baseUrl | 凭据和 http(s) 地址；新分享需要可解析的 Base URL |
| model | 默认模型的真实上游 ID |
| models | JSON 数组：id、name、contextWindow、maxTokens、reasoning、input |
| modelRoles | JSON 对象：Claude 的 sonnet、opus、fable、haiku 到模型 ID 的映射 |
| headers | JSON 数组：静态 set 或网关支持的 delete/rename/copy 操作 |
| gatewayProfile | JSON 对象：tool、profileId、endpointId |
| providerType / apiKeyField | 明确保存的兼容信息及认证方式，不复制 profile 派生快照 |
| sourceProviderId | 来源标识，用于数据库型目标去重 |
| homepage / notes / icon / iconColor | 描述信息；主页只接受 http(s) |
| config / extra | 仅兼容旧链接的 Base64 配置覆盖，不由新分享生成，不能跨目标工具导入 |

endpoints 仍被明确拒绝，没有未经实现的多地址持久化语义。

## 地址、认证与模型

OpenCode 的 AI SDK Anthropic/Google Base URL 包含版本路径，原生 Anthropic/Google SDK 自行拼接版本。分享保留源地址，在后端预览和保存时执行同一适配：

- OpenCode https://relay.example/anthropic/v1 到 Claude：去掉末尾版本路径，得到 https://relay.example/anthropic。
- Claude https://relay.example/anthropic 到 OpenCode：加上 /v1。
- OpenCode https://relay.example/google/v1alpha 到 Gemini CLI：Base URL 为 https://relay.example/google，另写 GOOGLE_GENAI_API_VERSION=v1alpha。
- 不改 OpenAI 的版本路径；保留代理前缀、查询字符串及明确的 ## 完整 URL 覆盖语义。

Claude 的 ANTHROPIC_API_KEY 和 ANTHROPIC_AUTH_TOKEN 分别表示 API-key 与 Bearer 认证；OpenCode Anthropic 分别写 options.apiKey 和 options.authToken，不能同时写两者。

OpenCode 本地模型 alias 通过模型 id 转成真实上游 ID；provider_id/model_id 只在第一个斜杠处分割。Codex 目录第一项不是用户的默认模型。Claude 的 [1M] 后缀不会附着到其他工具的模型 ID。

Desktop 对 Claude 原生模型保留安全 ID；其他模型用 Claude 可接受的路由 ID 映射到上游，放在 meta.claudeDesktopModelRoutes 并要求 Gateway。

核对来源：

- [AI SDK Anthropic](https://ai-sdk.dev/providers/ai-sdk-providers/anthropic)
- [AI SDK Google](https://ai-sdk.dev/providers/ai-sdk-providers/google)
- [Google GenAI URL 构造](https://github.com/googleapis/js-genai/blob/main/src/_api_client.ts)
- [Gemini CLI Content Generator](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/core/contentGenerator.ts)
- [Kimi ProviderType 与 ModelCapability](https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/llm.py)

## 实现与持久化边界

协议事实源是 tauri/src/coding/deeplink/parser.rs。前端 providerTransfer.ts 从当前供应商快照提取字段，providerShareUrl.ts 负责纯 URL 编码，分享和外链导入复用 ProviderTransferModal。

preview_deeplink_import 与 import_from_deeplink_unified 共用 prepare_import 校验和适配函数。保存入口在共享导入锁内查重和自动命名，再调用各模块原有 create/save 命令。确认之前无持久化。

查重使用同名、数据库来源标识或文件型目标的稳定 ID。文件型新 ID 带 -shared，避免遮蔽同名内置渠道而意外改变当前默认配置；非 ASCII 名称加确定性摘要，避免不同中文名称都变成 provider。

DSH 先快照旧配置和凭据原字节，再写新凭据和供应商。失败恢复两份文件；凭据只新增独立 ref，保留 records 和已有 refs，成功后才通知配置变更。

旧 config/extra 由 parser 容忍式 Base64 解码。Claude/Gemini 的 config 覆盖 settings JSON，Codex 的 config 覆盖 TOML；它们不是加密数据。携带这些字段时，前端锁定目标和连接字段，后端校验原目标，继续复用原保存验证。

## 唤起与验证

系统入口仍由 tauri-plugin-deep-link 汇合到 on_open_url。前端未 ready 时使用 latest-wins pending slot；普通热链接不写 pending。轻量模式重建 WebView 后重新握手，避免丢链接或重复回放。

Windows/Linux 安装版使用系统协议注册，开发模式启动时 register_all()；macOS 需要安装版 .app 的 CFBundleURLTypes。Windows/Linux 第二实例经 single-instance 转发 argv，macOS 使用 RunEvent::Opened。

日志中的 deep-link query 值统一替换为 ***REDACTED***，清除 userinfo 和 fragment。解析、取消和预览都不写配置。

```text
pnpm test
pnpm exec tsc --noEmit
pnpm build
cd tauri
cargo test --lib coding::deeplink
cargo test --test deeplink
cargo test
```

跨工具集成用例使用 MockRuntime、隔离 SQLite 和临时配置目录，调用真实统一导入入口，再读取供应商或文件；覆盖 12 个目标、重复导入、默认模型保留、坏配置和凭据保留。运行时路径缓存是进程级单例，因此使用独立 deeplink 测试二进制，并在配置测试目录后刷新缓存。

GUI 验收检查分享入口、目标切换、错误反馈、未确认不写入和成功刷新；OS 冷启动/第二实例与真实第三方接口连通性属于独立的平台集成检查。

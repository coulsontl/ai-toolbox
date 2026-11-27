# 供应商分享与导入模块

## 一句话职责

- 把当前供应商的通用连接配置适配到目标工具，通过确认入口复用目标模块原有的持久化链路。

## Source of Truth

- URL 协议以 `parser.rs` 为准，与前端 URLSearchParams 使用相同的 form 编码；协议说明在 `docs/deep-link-import.md`。
- 上游协议来自实际配置或当前 profile，目标 CLI 的协议不是上游协议。跨协议请求转换归 Gateway，不在分享模块重做 transformer。
- 供应商继续使用各工具已有数据库或运行时文件，没有统一供应商主库；收藏历史不是当前连接配置。

## 核心设计决策（Why）

- 只分享通用连接、模型和 header 信息，不生成原始 config/extra blob，避免跨工具结构错误和 OS URL 超长。旧 blob 仅在原目标导入。
- 预览与保存共用 `prepare_import`，避免弹窗说明与最终写入不同。外链收到、预览和关闭均无保存副作用。
- profile 按同一 profile、同一协议映射目标 endpoint，保存引用而非派生兼容快照。无法移植的特殊 adapter 必须报错，不能伪装成原生通用渠道。
- 默认跳过重复项，副本自动改名，不覆盖。查重和保存置于共享导入锁内；数据库新增默认未应用，文件新增使用独立 ID，不能因为名字等于内置渠道而改变当前默认配置。

## Gotchas

- OpenCode AI SDK 与原生 Anthropic/Google SDK 的版本语义不同。保留 sourceApp，只适配已知终端版本路径；不能盲目删代理前缀、查询参数或 OpenAI /v1。
- 认证方式属于连接信息，Anthropic API key 与 Bearer 不能互换。OpenCode 不能同时写 apiKey 和 authToken。OAuth access/refresh token 不能冒充共享 API Key。
- 默认模型与列表独立。OpenCode alias 转真实模型 ID，ID 内的斜杠保持原样；Codex 不能把目录第一项当默认模型。
- Codex 无自带 `config` TOML 时，生成的 `model_provider` 和 `[model_providers.<id>]` 统一使用 `custom`，provider 展示名写到 `name`。不要按 `request.name` 生成 slug：slug 会随供应商改名产生新的 provider 桶，恢复旧会话时 Codex 会因 `saved_provider` 不在当前 `[model_providers]` 而拒绝恢复。自带 `config` 时仍逐字使用原 TOML。
- Claude Code 的显示名称是独立 env 变量，不是模型 ID 的别名：四个 tier 写 `ANTHROPIC_DEFAULT_<ROLE>_MODEL_NAME`，custom 写 `ANTHROPIC_CUSTOM_MODEL_OPTION_NAME`，subagent 没有名称变量。导入必须把 `models[].name` 写进这些变量，否则目标回退显示模型 ID。Desktop 目标不写 subagent/custom 绑定。
- Claude Code 的 role 全集是 sonnet/opus/fable/haiku/subagent/custom；Desktop 只认前四个 tier。未知 role 不报错：过滤掉并通过 preview/result 的 `ignored_roles` 告诉用户，避免新版本链接让旧客户端整体导入失败。
- 模型能力按目标 schema 写入：OpenCode limit 只接受完整 context/output 对，不能伪造缺失值；modalities 需要 input/output 两侧。Pi/OMP/OpenClaw/DSH 的 input 只写 text/image，不能把 models.dev 的 audio/video 枚举原样塞入运行时配置。
- Desktop 可见安全 ID 与上游模型可不同；生成 claudeDesktopModelRoutes，原生 Claude 模型不改名，其他模型明确依赖 Gateway。
- 文件读取失败或已有结构非法时终止，不能按空配置处理。DSH 配置和新凭据 ref 必须整体成功或恢复原字节，保留登录 records。
- 内置默认数据只读本地 cache/bundled defaults，不联网刷新。OpenCode 选中渠道只读取 API-key 类型，禁止使用会 fallback OAuth access 的连通性 helper。

## 最小验证

- 新目标或字段覆盖分享提取、URL parse、确认导入、保存、再读取，并检查原有默认配置和未知字段。
- `cargo test --lib coding::deeplink` 与 `cargo test --test deeplink`；前端对应 `web/test/features/shared/deepLink/`。
- 跨工具集成使用独立测试二进制、临时目录和 MockRuntime；进程级 runtime location 缓存要在设定目录后刷新，并在写入前断言解析路径属于测试目录。
- 跨层变更按根规则执行全量前后端测试、类型检查和构建。

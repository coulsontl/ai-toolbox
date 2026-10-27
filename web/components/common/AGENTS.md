# Shared Components Development Guide

## 一句话职责

- 为多个页面提供编辑器和基础交互组件，并保证用户输入规模或内容形态不会阻塞前端主线程。

## 核心设计决策（Why）

- Monaco Monarch tokenizer 在 WebView 主线程执行。字符串规则必须保持线性时间；正则分支不能重叠消费同一字符，否则包含大量转义符的配置行会触发灾难性回溯并冻结整个主窗口。
- 编辑器组件一律 `import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'`（核心 API 入口，不自动注册语言），按需 `import 'monaco-editor/esm/vs/language/json/monaco.contribution'` 只注册 JSON。**不要**改回 `from 'monaco-editor'`（`editor.main` 入口会全量注册 css/html/typescript 语言并拉入对应 worker bundle，~8.7 MB JS 常驻 webview 内存，而这些编辑器从不使用 css/html/ts 语言）。`web/app/monaco.ts` 的 `MonacoEnvironment.getWorker` 也只注册 `editor` 与 `json` 两个 worker；新增语言 worker 时需同步在 workerFactories 里登记，并确认确有编辑器用到该 language。
- Monaco 的剪贴板服务在 `web/app/monaco.ts` 里被全局替换为 Tauri 后端实现（`StandaloneServices.initialize` + arboard 命令，issue #369）：右键菜单 paste 没有原生剪贴板事件可用，只能走 `navigator.clipboard.readText`，该 API 在 WebKitGTK（WSLg 桥）下不可用、在 WebView2 中即使已授权限也仍会失败，而 Monaco 对失败的处理是返回空串静默粘贴。Ctrl+C/V 走浏览器原生剪贴板事件、不受该服务影响，也不要去拦截它们；唯一例外是原生 paste 事件**带空 `clipboardData`** 时的兜底重放（`web/utils/emptyPasteFallback.ts`，issue #384）：只在「原生粘贴什么都没带 + 焦点在 Monaco 编辑器内」时用后端文本重放一次，带文本的粘贴、普通 input/textarea、失焦编辑器一律不碰，不要把它扩大成无条件拦截。`StandaloneServices.initialize` 只在首次调用生效且只覆盖未实例化的服务，因此该初始化必须保持在 `main.tsx` 的 import 图中先于首个编辑器创建执行。新增编辑器组件默认继承该服务，**不要**绕过服务直接调 `navigator.clipboard.readText`；剪贴板命令封装在 `web/services/clipboardApi.ts`（Tauri 后端优先、Web API 兜底）。
- `AutoComplete` 一律用 `ImeSafeAutoComplete`（issue #409）。antd 的 `AutoComplete` 是 rc-select 的 combobox 模式，渲染的是 rc-select 自带的原生 `<input>`（`@rc-component/select/es/SelectInput/Input.js`），**不走 antd 的 `Input`（rc-input）**；而 rc-input 才有 IME 保护（`compositionend` 分支 + `resolveOnChange` 的 clone，绕开 Safari 受限的 `input.value` getter）。rc-select 那条路径在组合过程中就会读 `event.target.value` 并 `triggerChange`（`Select.js` 的 combobox 分支），把受控值改写回 DOM，在 macOS WKWebView 下直接打断组合、让拼音原样上屏。`ImeSafeAutoComplete` 给 AutoComplete 塞一个带 composition 守卫的自定义子 Input（即 `ImeSafeInput`）：组合期间照常镜像 DOM 值（不镜像的话 rc-input 每次 `setValue` 后的重渲染会被 React 写回、同样打断组合），但**不把 preedit 转发给 rc-select**，`compositionend` 时才上报一次。`children` 被这个包装占用，调用方只能用 `options`。它保持与 antd 同款的泛型签名（`OptionType` 由 `options` 反推），否则调用点内联的 `filterOption` 会失去自己的选项类型；`size` 被转给内层 Input（customizeInput 下 antd 忽略 AutoComplete 上的 `size` 并告警），Kimi 模型表格那类密集场景靠它保住 24px 高度。自定义输入是「输入框自己画边框/内边距」的形态（antd `-customize` 会把根节点的 chrome 交给子输入），实测外框高度、字号、文字内缩与裸 AutoComplete 一致。
- 普通 `Input` 只有在**没人从外部回写它的值**时才是 IME 安全的。若某字段的 value 会在每次按键被「清洗/派生 → `form.setFieldsValue` 回写」（Codex 的 base url / api key：`onValuesChange` → `useCodexConfigState` 的 `trim()` → effect 回写），一旦回写值与组合中间态不一致（中文输入法在中文模式下上屏英文时带的那个空格会被 `trim()` 掉），WKWebView 会像 rc-select 那条路径一样打断组合（issue #409 的残留报告）。这类字段改用 `ImeSafeInput`（`web/components/common/ImeSafeInput`，`ImeSafeAutoComplete` 也复用它）：组合期间自己持有渲染值、不把 preedit 交给表单，`compositionend` 才上报一次。**只给这类字段用，其余普通字段仍用 antd `Input`**，不要无脑全局替换。
- Monaco 的 `editorDidMount`（`react-monaco-editor` 的 prop，`lib/editor.js`）只在挂载时调用一次，所以在 `onDidBlurEditorText` / `onDidFocusEditorText` 里注册的回调会**永久冻结**在首次渲染的闭包上；`onChange` / `options` 不受影响（react-monaco-editor 每次渲染都用 ref / effect 同步它们）。挂载期注册的编辑器回调必须在调用点经 ref 取最新 prop：`JsonEditor`（`onBlur` / `onRawBlur`）、`JsoncEditor`、`TomlEditor`、`MarkdownEditor` 均已按此转发（issue #406：OpenCode“其他配置”失焦自动保存拿到的是面板展开那一刻的配置，整份回退掉 MCP 页刚写入的 server）。**新增编辑器或给现有编辑器加挂载期回调时沿用同一模式，不要裸捕获 prop。**

## 易错点与历史坑（Gotchas）

- TOML 双引号字符串的“未闭合”规则中，转义分支 `\\.` 与普通字符分支必须互斥。普通字符分支必须排除反斜杠，使用 `[^"\\]`，不能退回会同时匹配反斜杠的 `[^\"]`。
- 不要只用普通短配置验证 tokenizer。Codex `notify` 等配置会把 JSON 嵌入 TOML 字符串，形成包含大量反斜杠和转义引号的超长单行。
- `FetchModelsModal` 的展示顺序统一按 `sort.ts` 的 owner 分组排序（locale 钉死 `en` 保证确定性）；`priorityOwnedBy` 是可选 prop，消费方（如 Codex 置顶 openai）自选，**不得**把具体厂商偏好写进默认行为。`onSuccess` 的 `orderedModelIds` 是完整列表的显示顺序（含未勾选项），供消费方对齐自身列表/映射顺序；`selectedModels` 必须从**完整列表**的排序结果里过滤（现在 `handleConfirm` 的做法），不能从搜索过滤后的视图取——否则搜索状态下确认会静默丢弃被过滤隐藏的已勾选模型。
- `ModelItem` 的 `extraActions` 是给消费方追加行级操作的插槽，必须和内置的「设为主模型」一样按 hover 显示（复用 `styles.hoverAction`，两种 `appearance` 各自决定是「占位淡入」还是「让出宽度」——见 `shared/AGENTS.md`），并且在选择模式下隐藏；不要在每一行常驻渲染文字按钮，高密度模型列表会被撑爆。
- `FetchModelsModal` 搜索只改变视图，跨搜索的选择必须保留；Ant Design Table 需要 `preserveSelectedRowKeys: true`，否则第二次勾选时就会丢掉隐藏行，确认阶段遍历完整列表也无法恢复。关闭、重新获取或切换连接后应重置选择，旧连接/旧弹窗的异步结果不能覆盖新结果。
- 模型导入弹窗可能一直挂载，不能只在首次 `useState` 初始化 SDK 对应的 API 类型；每次打开或切换 SDK 都要重置为正确的原生/兼容模式。Google Native 的可编辑 URL 必须与后端发现路径一致：无版本时仅在探测 URL 补 `/v1beta`，保留显式版本和用户手改 URL，不改写供应商保存的 Base URL。

- `FetchModelsModal` 的可选 `configValueMode`（Pi 传 `'pi'`、OMP 传 `'omp'`）表示后端会在该工具运行时里解析 apiKey/headers 的配置值语法：此时预览 URL 不得内嵌原始 key（Google native 的 `?key=` 由后端解析后补齐），新增消费方默认不传，保持既有 URL 语义。

## 最小验证

- 修改任一 Monaco 编辑器的失焦/回调转发后，运行 `pnpm test:json-editor-blur`（真实 Monaco + headless Chromium 的本地 fixture，不进 CI）：它把 `JsonEditor` / `JsoncEditor` / `TomlEditor` / `MarkdownEditor` 同时挂起来，逐个断言「消费方重渲染后失焦派发的是最新 `onBlur`，而不是挂载期闭包里的那个」，并校验各自回调的载荷形态。fixture 只用 blur/焦点链路，因此把 JSON 语言贡献替换成 no-op stub，避免在没有 worker factory 的 fixture 里请求 JSON worker。已反向验证：把任一编辑器的 ref 转发改回挂载期捕获，对应检查立刻以 `1 !== 4` 失败。
- 修改 TOML tokenizer 后，运行 `web/test/components/common/TomlEditor/invalidDoubleQuoteStringPattern.test.ts`。
- 修改空粘贴兜底（`web/utils/emptyPasteFallback.ts`）后，运行 `web/test/utils/emptyPasteFallback.test.ts`，覆盖「原生带文本/空 `clipboardData`/无焦点编辑器/后端失败/后端空串」五种分支。
- 修改 `ImeSafeAutoComplete` 或 `ImeSafeInput` 后，运行 `pnpm test:ime-autocomplete`（真实 antd + headless Chromium 的本地 fixture，不进 CI）。Chromium 复现不了 WebKit 的组合中断，所以它守的是我们的契约：旁边的裸 `AutoComplete` 作为对照（mid-composition 的 preedit 会进 `onChange`），断言守卫版「组合期间不上报、DOM 值不被清空、`compositionend` 时上报一次、普通输入照常转发」，另有一组外观断言（外框高度/字号/文字内缩与裸 AutoComplete 一致、placeholder 仍在、`size="small"` 仍比默认矮）。fixture 里还有第二组场景——两个**表单接线完全相同**的字段（`onValuesChange` → 清洗 → effect 回写），只差输入组件：普通 `Input` 一侧断言「回写确实在组合中改写了 DOM 值」（即 WKWebView 上打断组合的那个动作，Chromium 也能看到），守卫一侧断言「组合期间回写不落到 DOM、表单收不到 preedit，`compositionend` 后才提交一次并稳定在被清洗后的值」。反向验证：AutoComplete 那组去掉组合守卫后第二个断言立即失败；字段这组把守卫换回普通 `Input` 后「组合值存活」立即失败（`'https://a.example' !== 'https://a.example '`）。
- 语义覆盖要同时包含：未闭合串、普通 closed 串、真实 Codex `notify` 风格 Windows 路径 closed 串，以及会触发指数回溯的 adversarial closed 串。
- 指数回溯回归必须在可终止的 Worker 中执行（超时即失败），避免危险正则重新出现时把完整测试进程永久卡住。
- 修改 `FetchModelsModal` 排序或 `onSuccess` 契约后，运行 `web/test/components/common/FetchModelsModal/sort.test.ts`；改动 URL/请求契约（如 `configValueMode`）后，同时运行 `web/test/components/common/FetchModelsModal/request.test.ts`，并确认 `onSuccess` 新增字段对所有消费方（OpenCode/Grok/Pi/DSH/Hermes/OhMyPi/OpenClaw 页面与 Codex 表单）是纯增量。
- 搜索选择、协议切换和过期响应用 `pnpm test:codex-model-import` 验证真实共享弹窗（`web/test/features/coding/codex/components/fetchedModelsModalBrowserChecks.mjs`，29 项断言，夹具直接挂载 `FetchModelsModal`）；需要已安装 Chrome/Edge，可传 `--browser` 指定路径。模型接口和保存由隔离桩提供，不写用户配置。它在 `pnpm test` 链上，改本组件时必须保持通过。

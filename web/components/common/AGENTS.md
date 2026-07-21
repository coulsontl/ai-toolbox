# Shared Components Development Guide

## 一句话职责

- 为多个页面提供编辑器和基础交互组件，并保证用户输入规模或内容形态不会阻塞前端主线程。

## 核心设计决策（Why）

- Monaco Monarch tokenizer 在 WebView 主线程执行。字符串规则必须保持线性时间；正则分支不能重叠消费同一字符，否则包含大量转义符的配置行会触发灾难性回溯并冻结整个主窗口。

## 易错点与历史坑（Gotchas）

- TOML 双引号字符串的“未闭合”规则中，转义分支 `\\.` 与普通字符分支必须互斥。普通字符分支必须排除反斜杠，使用 `[^"\\]`，不能退回会同时匹配反斜杠的 `[^\"]`。
- 不要只用普通短配置验证 tokenizer。Codex `notify` 等配置会把 JSON 嵌入 TOML 字符串，形成包含大量反斜杠和转义引号的超长单行。

## 最小验证

- 修改 TOML tokenizer 后，运行 `web/test/components/common/TomlEditor/invalidDoubleQuoteStringPattern.test.ts`。
- 语义覆盖要同时包含：未闭合串、普通 closed 串、真实 Codex `notify` 风格 Windows 路径 closed 串，以及会触发指数回溯的 adversarial closed 串。
- 指数回溯回归必须在可终止的 Worker 中执行（超时即失败），避免危险正则重新出现时把完整测试进程永久卡住。

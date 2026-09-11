# DeepLink 前端模块说明

## 一句话职责

- 统一供应商分享、本机跨工具导入和 aitoolbox:// 外链确认，复用同一个字段编辑与后端预览流程。

## Source of Truth

- URL 协议和合法目标以 tauri/src/coding/deeplink/parser.rs 为准；说明见 docs/deep-link-import.md。前端工具 registry 只维护名称和路由。
- 当前供应商快照是分享来源，独立收藏历史不代替当前配置。缺省字段可使用后端只读 cache/bundled defaults 补齐。
- 唯一写入路径是 import_from_deeplink_unified；复制链接只做纯前端 URL 生成。预览使用后端同一份导入准备逻辑。

## 核心设计决策（Why）

- 链接携带通用连接、协议、认证语义和可移植模型字段，不带源工具 config/extra blob。旧 blob 只能进入原目标，字段锁定，避免错写另一种配置结构。
- 分享和外链确认共用弹窗；显示实际协议、目标地址、Gateway 要求、是否直接写运行时文件，不能把保存和启用混成一个动作。
- 模型目录包含不同 URL、协议或 Key 时按连接分组，先选源连接，不能把所有模型发到第一项地址。
- 长链接停止复制并保留本机导入；用户可减少模型数量。不能静默截断模型或链接。

## Gotchas

- 保留 sourceApp 供后端处理 SDK 版本路径差异。切换目标时不把源协议替换成目标工具默认协议。
- API Key 使用可编辑密码输入。OAuth 和 env/file/command 引用不可移植，不能执行命令、解析任意文件或导出订阅 tokens 来补 Key。
- 旧异步 preview/import 返回时不得覆盖或关闭后来收到的新链接；身份检查覆盖成功、失败和 finally。
- Claude 默认模型沿用角色 fallback 链并剥离 [1M]；Desktop 读取实际上游路由。Codex 默认模型独立于目录顺序。OpenCode alias 转真实模型 ID，只按第一个斜杠分隔 provider/model。
- profile 保存引用，不复制派生兼容快照，Base URL 的用户覆盖继续优先。
- ready 状态只属于当前 WebView 生命周期；轻量模式重建重新挂监听器再握手，pending latest-wins，不重复回放已送达热链接。
- URLSearchParams 与 Rust query_pairs 都按 form 编码处理加号。homepage 只输出 http(s)，避免生成成功而接收端拒绝。
- 被 node:test 直接加载的纯函数保持相对导入，当前 loader 不解析 @/ 别名。
- 同工具的完整无损克隆仍由原「复制」动作承担，分享只保留跨工具连接语义。

## 最小验证

- pnpm test 中的 providerShareUrl 和 providerTransfer 用例覆盖所有来源、默认模型、分组、alias、认证和 URL 字符。
- GUI 检查分享入口、目标/源连接切换、loading/error、取消、复制、重复导入和成功刷新，同时检查亮暗/system 主题、长文本与窄窗口。

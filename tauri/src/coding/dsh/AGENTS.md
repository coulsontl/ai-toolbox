# DeepSeek Harness (dsh) Backend Module 说明

## 一句话职责

- `dsh/` 负责 DeepSeek Harness 的配置目录解析与 `settings.yaml` / `.credentials.yaml` 的可视化管理：provider 增删改、`agent-default-model` 默认模型、Other Settings，以及全局提示词（写入 `<root>/AGENTS.md`）。

## Source of Truth

- dsh 用**单个 namespaced YAML 文件** `settings.yaml`（位于配置目录内），栈内按插件短名分节。本模块负责三类 section：
  1. `llm-pi-ai.providers.<route>`：供应商字典（key 即 route id），对应 Hermes 的 custom_providers。
  2. `agent-default-model`：默认模型 `{ provider, model, reasoningEffort }`。
  3. 其它 section（未知）保留。
- 凭据放在**独立的** `.credentials.yaml`。dsh >= 0.1.1-rc.1 使用**版本化布局**：顶层 `version: 1` 标记 + `refs:` 节点（`REF: secret`，REF 为 POSIX 环境变量名，如 `DEEPSEEK_API_KEY`）+ `records:` 节点（dsh 登录流写入的 api-key/OAuth grant 记录，key 格式 `llm-pi-ai/<providerId>`）。供应商只存 `apiKeyEnv` 引用，key 本体在 `refs:` 下。dsh 启动时会把旧扁平文档自动迁移成版本化布局。
- 配置目录解析优先级：应用 DB `dsh_settings_config` 的 `common.config_dir`（source=`custom`）> 环境变量 `DSH_HOME`（`env`）> shell 配置（`shell`）> 平台默认（`default`）。平台默认：mac/Linux `~/.dsh`，Windows `%USERPROFILE%\.dsh`。
- SQLite 只保存配置目录选择（`common` 记录）与全局提示词预设（`dsh_prompt_config`）；**不要**新增 `dsh_provider` 之类第二套 provider 主数据。
- 配置目录优先级由 `commands.rs` 的同一解析器提供给文件操作与 `runtime_location`；WSL UNC 识别、`module_statuses` 和同步跳过集合统一由 `runtime_location` 产出，不在 dsh 内复制判定。普通目录保留本机同步，UNC 目录视为已直接操作 WSL 文件。

## 核心设计决策

- provider 写入时只 upsert `llm-pi-ai.providers.<route>` 的 exact route；`models` 保持数组（`[{ id, contextWindow?, maxTokens? }]`）。
- 默认模型 `agent-default-model` 采用字符串字段「空串=删除键」语义（同 pi/hermes）。
- Other Settings 编辑器隐藏并保留托管键：`llm-pi-ai`、`agent-default-model`。
- 凭据读写集中在 `commands.rs` 的 `CredentialsDocument`：写入**统一输出版本化布局**（盖 `version: 1` 戳、只改 `refs:` 节点、整体重写保留 `records:` 与键序）；读到旧扁平文档时按 dsh 官方迁移规则把既有条目原样收编进 `refs:` 再改写（等价于 dsh 的 boot migration，避免覆盖丢密钥）。`records:` 归 dsh 登录流所有，本模块永不写入或删除。
- 凭据写盘使用 0600 权限（参照 pi 的 `set_credentials_file_permissions`）。`save_dsh_credential` 传空 value 相当于删除该 ref。
- WSL/SSH 根据配置根目录派生 `dsh-config`（settings.yaml）、`dsh-credentials`（.credentials.yaml）、`dsh-prompt`（AGENTS.md）与 `dsh-mcp`（cordis.patch.yml）四个默认文件映射，模块名 `dsh`。WSL Direct 时跳过重复文件/MCP 同步；SSH 仍允许读取 UNC 源文件后上传。
- 文件式预览由 `read_dsh_runtime_config` 返回三个原始文件内容（`configContent` / `credentialsContent` / `promptContent`），前端按文件 Tab 展示，与 Codex 预览一致。

## Gotchas

- 分享导入同时写供应商和独立凭据 ref 时，使用 `save_dsh_models_provider` 的 credential 输入在同一链路保存；先快照两份文件，任一步失败恢复旧字节或删除原不存在的文件，成功后再发事件。不要改成前端先保存 Key 再独立保存供应商，否则后一步失败会遗留凭据或破坏共享引用。

- 保存或清除配置目录后，必须先刷新 `runtime_location` 的 dsh 缓存，再发 `wsl-config-changed` 和原有配置/自动同步事件。issue #331 曾因映射能解析 UNC、状态接口却没有 dsh，导致 Linux `cp` 收到 `//wsl.localhost/...` 并报 cannot stat。只修路径字符串转换不能解决重复同步。
- provider 视图的凭据回填顺序镜像 pi-ai 运行时解析顺序：先查 `records["llm-pi-ai/<route>"]`（api-key 记录取 `key` 字段回填；grant 或 env-only 记录仅标记已配置、不显示值），无记录才回查 `apiKeyEnv` 指向的 ref。因此经 dsh 官方 UI 登录的渠道在卡片上也能正确显示「已配置」。
- `delete_dsh_credential` 对不存在的 ref 是幂等 no-op（不再报错）：有效凭据可能在 records 里，清空 key 的 UI 流程必须能成功返回。
- 删除 provider 只删 `llm-pi-ai.providers.<route>`/空容器，不回滚 `agent-default-model` 默认选择；本地生效配置只在用户显式切换/应用时改写。
- 前端批量删除应先排除默认/内置渠道，并从删除前的完整 runtime view 构建计划和收藏凭据快照。一个 `apiKeyEnv` 可被多个渠道共享：仍有未删除渠道引用时保留 ref，全部删除时只删一次；各被删渠道的收藏都要保留原密钥，不能用删除上一渠道后返回的 `credentialExists` 决定下一份备份。回归见 `web/test/features/coding/dsh/utils/providerDeletion.test.ts`。
- 删除 prompt 预设只删 SQLite 记录，不改写/清空当前运行时 `AGENTS.md`。
- `settings.yaml` 允许未知 top-level 与 provider 未知字段；读写必须 preserve unknown fields。
- 保存 Other Settings 时不要把托管键（`llm-pi-ai`、`agent-default-model`）带回文件。
- 内置 provider 即使没有写进 `llm-pi-ai.providers`（凭 env/默认可用）也不应显示为 missing；凭据缺失显示为未配置而非 missing。
- dsh MCP 由 `mcp::cordis_patch` 适配器管理 `~/.dsh/cordis.patch.yml`（Cordis patch DSL，format `cordis`）。每个 MCP server 是一行 `insert`，包名固定 `@deepseek-ai/dsh-mcp-client`，`config.serverName` 作 key。本模块（dsh）仍管 `settings.yaml`/`.credentials.yaml`/`AGENTS.md`，不直接写 MCP 配置。dsh 是 developer preview，cordis patch 格式可能迭代；adapter 隔离在 `cordis_patch.rs` 便于后续更新。
- `read_dsh_runtime_config` 返回的 `credentialsContent` 是 `.credentials.yaml` 原始内容，包含真实密钥；前端仅用于只读文件预览，不得把该字段当作可编辑数据回写。
- 启用 agent-instructions（`enable_dsh_agent_instructions`）会同时往 home 级 `cordis.patch.yml` 写 `disabled: false` 和 `config.maxBytes: 262144`（256 KiB），覆盖 bundle 默认 64 KiB 预算，避免项目根 `AGENTS.md` 一超 64 KiB 就把 `~/.dsh/AGENTS.md` 整文件挤出 baseline。`check_dsh_agent_instructions` 仍只按 `disabled` 判定启用；重复启用会幂等覆盖 maxBytes。cordis patch 写字段走 `mcp::cordis_patch::set_plugin_config_field`（合并 config、保留其它字段与行）。

## 最小验证

- `cargo test --test coding wsl_direct_status` 覆盖真实保存命令到 WSL 配置状态读取、UNC 两种前缀、切回本机目录和清除自定义目录后的缓存刷新；WSL MCP 过滤回归另见 `cargo test --lib coding::wsl::mcp_sync::tests`。
- `settings.yaml` 已有 `llm-pi-ai.providers.<route>` 时，编辑该 route 后其它 provider 与未知顶层键保持不变。
- 默认 provider 不在 `llm-pi-ai.providers` 又非内置时，view 应标记 missing，`save`/`delete` 返回明确错误。
- 保存默认模型后，`agent-default-model` 的 `provider`/`model` 正确写入，`reasoningEffort` 空串时被删除。
- Provider 卡片编辑 apiKey 即写 `.credentials.yaml`，且文件权限为 0600（Unix）。
- 对已是版本化布局的凭据文件保存/删除 ref 后，`version: 1` 与 `records:` 原样保留；对旧扁平文档首次写入后整体迁移成版本化布局且既有条目不丢。
- 删除已保存 prompt 后，磁盘 `AGENTS.md` 内容保持不变。

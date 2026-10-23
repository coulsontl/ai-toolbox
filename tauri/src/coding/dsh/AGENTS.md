# DeepSeek Harness (dsh) Backend Module 说明

## 一句话职责

- `dsh/` 负责 DeepSeek Harness 的配置目录解析与 `settings.yaml` / `.credentials.yaml` 的可视化管理：provider 增删改、`agent-default-model` 默认模型、Other Settings，以及全局提示词（写入 `<root>/AGENTS.md`）。

## Source of Truth

- **写入层随 dsh 版本切换，判据是「dsh 自己有没有完成迁移」，不是「有没有 profile 目录」。** dsh >= 0.1.7 不再读 `$DSH_HOME/settings.yaml`：`@deepseek-ai/dsh-settings` 在 Loader 就绪后把它一次性导入当前 profile 的 `profiles/<profile>/cordis.patch.yml`，随后把源文件改名为 `settings.yaml.imported`（见上游 `packages/settings/settings/src/index.ts` 的 `importLegacyDocument`）。**`profiles/web/` 是 dsh 0.1.5 安装时就会创建的**，它的存在只说明 dsh 启动过这个 profile，不说明供应商数据在哪一层。因此 `commands.rs` 里的 `DshConfigLayer` 是唯一分层入口：
  - `settings.yaml` 仍存在、且没有 `settings.yaml.imported` → 写 `SettingsYaml`。这是 dsh < 0.1.7 的常态，也是 0.1.7+ 尚未启动过的状态；此时 `profiles/<name>/cordis.patch.yml` 里通常只有 `webserver` 之类，硬读它会得到一个**空供应商列表**（真实故障：0.1.5-rc.3 机器上整页空白，而配置完整躺在 `settings.yaml` 里）。
  - 迁移标记 `settings.yaml.imported` 存在 → 写 `ProfilePatch`：`llm-pi-ai` / `agent-default-model` 各是一行 `{ id, name, config }` patch entry，其余插件行原样保留。
  - 既无 `settings.yaml` 也无标记（全新安装且从未写 legacy 文件）→ 有 profile 就走 `ProfilePatch`，否则 `SettingsYaml`。
  - profile 解析顺序：DB 固定值 → `desktop` → `web` → mtime 最新的 profile（`profile_patch::detect_profile`）。desktop 端固定启动 `profiles/desktop`（`apps/desktop/src/paths.ts`），`dsh web` 初始化 `profiles/web`。
  - patch 的 `config` 是**整行替换**语义（`applyEntryPatches` 做 `target[key] = value`），所以读写都按完整对象处理；section 为空时必须删除整行，不能写空对象。
  - 回归：`commands::tests::an_unmigrated_settings_yaml_wins_over_an_existing_profile`（有 profile 但无标记 → 仍读 legacy）与 `the_migration_marker_hands_the_layer_to_the_profile_patch`（标记一出现 → 立刻切到 profile 层）。
- 旧版 `settings.yaml` 的语义（legacy 层仍在用）是**单个 namespaced YAML 文件**，栈内按插件短名分节。本模块负责三类 section：
  1. `llm-pi-ai.providers.<route>`：供应商字典（key 即 route id），对应 Hermes 的 custom_providers。
  2. `agent-default-model`：默认模型 `{ provider, model, reasoningEffort }`。
  3. 其它 section（未知）保留。
- 凭据放在**独立的** `.credentials.yaml`。dsh >= 0.1.1-rc.1 使用**版本化布局**：顶层 `version: 1` 标记 + `refs:` 节点（`REF: secret`，REF 为 POSIX 环境变量名，如 `DEEPSEEK_API_KEY`）+ `records:` 节点（dsh 登录流写入的 api-key/OAuth grant 记录，key 格式 `llm-pi-ai/<providerId>`）。供应商只存 `apiKeyEnv` 引用，key 本体在 `refs:` 下。dsh 启动时会把旧扁平文档自动迁移成版本化布局。
- 配置目录解析优先级：应用 DB `dsh_settings_config` 的 `common.config_dir`（source=`custom`）> 环境变量 `DSH_HOME`（`env`）> shell 配置（`shell`）> 平台默认（`default`）。平台默认：mac/Linux `~/.dsh`，Windows `%USERPROFILE%\.dsh`。
- 同一 DB 记录里的 `profile` 只决定**编辑哪个 profile 的补丁**（`write` 与 WSL/SSH 映射共用 `resolve_active_profile_async`）；它不改 `DSH_HOME`，也不影响 dsh 实际启动的 profile（desktop 端恒为 `desktop`）。
- SQLite 只保存配置目录选择（`common` 记录）与全局提示词预设（`dsh_prompt_config`）；**不要**新增 `dsh_provider` 之类第二套 provider 主数据。
- 配置目录优先级由 `commands.rs` 的同一解析器提供给文件操作与 `runtime_location`；WSL UNC 识别、`module_statuses` 和同步跳过集合统一由 `runtime_location` 产出，不在 dsh 内复制判定。普通目录保留本机同步，UNC 目录视为已直接操作 WSL 文件。
- 会话产物的磁盘布局事实（含命名与代际选择）唯一来源是 `session_artifact.rs`：`<sessions root>/<project key>/<encoded session id>/session[.v<N>].jsonl[.zstd]`，`session.jsonl` 即格式 v0，`session.v<N>.jsonl` 为第 N 代；**同一会话目录可并存多代**（迁移只新增文件、不改写旧文件），**数值最大的一代才是活跃产物**。`session_manager`（浏览）与 `proxy_gateway`（用量导入）都必须走这里，不得各自复制判定规则。

## 核心设计决策

- provider 写入时只 upsert `llm-pi-ai.providers.<route>` 的 exact route；`models` 保持数组（`[{ id, contextWindow?, maxTokens? }]`）。
- 模型条目的合法字段由上游 `packages/llm/llm-pi-ai/src/config.ts` 的 `modelFields` 决定：`id`（必填）、`name`、`contextWindow`、`maxTokens`、`input`（`text`/`image`）、`reasoningEfforts`、`compat`。`input` 与 `compat` 是上游 schema 字段，前端白名单必须包含；`reasoning` / `cost` 不是上游字段，仅因 schemastery 对未知键透传而保留为展示层数据。
- `DSH_BUILTIN_PROVIDERS` 必须与已安装 pi-ai 的 `catalogProviderIds()` 对齐（`llm-pi-ai/src/catalog.ts` 的 `servesChat`：至少有 chat 模型，或 catalog 完全不列模型）。`@earendil-works/pi-ai@1.1.0` 下共 41 条，`typesafe` 因为只有非 chat 模型而被排除。名单只负责显示名与“内置不可删除”标记，不决定配置是否存在。
- 默认模型 `agent-default-model` 采用字符串字段「空串=删除键」语义（同 pi/hermes）；`provider` 与 `model` 任一为空时整节删除（profile 层即删除该 patch row）。
- Other Settings 编辑器隐藏并保留托管键：`llm-pi-ai`、`agent-default-model`。**profile 层上该编辑器整体冻结**（`read_dsh_runtime_config` 返回 `otherSettings: {}`，`save_dsh_other_settings` 直接拒绝）：patch 里是 dsh 全部插件的行，自由 JSON 编辑会破坏无关插件，这些配置归 dsh 自己的设置页所有。
- 凭据读写集中在 `commands.rs` 的 `CredentialsDocument`：写入**统一输出版本化布局**（盖 `version: 1` 戳、只改 `refs:` 节点、整体重写保留 `records:` 与键序）；读到旧扁平文档时按 dsh 官方迁移规则把既有条目原样收编进 `refs:` 再改写（等价于 dsh 的 boot migration，避免覆盖丢密钥）。`records:` 归 dsh 登录流所有，本模块永不写入或删除。
- 凭据写盘使用 0600 权限（参照 pi 的 `set_credentials_file_permissions`）。`save_dsh_credential` 传空 value 相当于删除该 ref。
- WSL/SSH 根据配置根目录派生 `dsh-config`（settings.yaml）、`dsh-credentials`（.credentials.yaml）、`dsh-prompt`（AGENTS.md）、`dsh-mcp`（home 级 cordis.patch.yml）与 `dsh-profile-patch`（`profiles/<active>/cordis.patch.yml`）五个默认文件映射，模块名 `dsh`。后两条都参与 MCP 专用同步（`is_mapped_mcp_config_file`）；`dsh-profile-patch` 的路径按当前活动 profile 动态解析，回退固定为 `profiles/desktop`。WSL Direct 时跳过重复文件/MCP 同步；SSH 仍允许读取 UNC 源文件后上传。
- 文件式预览由 `read_dsh_runtime_config` 返回原始文件内容（`configContent` / `credentialsContent` / `promptContent` / `cordisPatchContent`），`configContent` 就是当前写入层（profile patch 或 legacy settings.yaml），前端按 `profile` 字段决定 Tab 标题。`profile` / `profiles` / `migratedFromSettings` / `settingsYamlPath` 用于 profile 选择器与迁移提示。

## Gotchas

- **`!!js` 表达式往返必须保真。** profile/home patch 由 js-yaml 读取，`!!js` 标量序列化成 `{ __jsExpr: "<src>" }`；而 serde_yaml 会把标准 `!!js` 解析掉（退化成普通字符串），只有本地写法 `!js` 会保留为 `Tagged`。`yaml_sync::rewrite_unquoted_tag` 在读写前互换两种拼写（引号内文本不动），`yaml_to_json_lossless` / `json_to_yaml_lossless` 负责 `__jsExpr` 标记与 tag 的互转。`profile_patch`、`mcp::cordis_patch` 与 `mcp::command_normalize::process_cordis_patch_yaml` 三处写路径都必须走这套；直接 `serde_json::to_value` 会把表达式写成字符串，dsh 启动时就变成字面量。
- 修改 patch 文件时只动自己的行：`read_patch_entries` 读整个数组、`set_entry_config` / `remove_entry` 按 `id` 定位，其它行（含 `insert` 行）逐行保留。不要把 patch 当 `settings.yaml` 那样合并顶层键。
- `save_dsh_settings_config` 的 `profile` 是三态：省略=保留现有固定值；`""`=清除固定回到自动检测；具体名字=固定该 profile。前端选择器依赖这个语义，改成 `Option` 空值即清空会破坏「只改目录」的调用。
- 备份/恢复会额外打包活动 profile 的 `profiles/<profile>/cordis.patch.yml`（`settings::backup::utils::get_dsh_profile_patch_path_from_db_async`）。只打包 `settings.yaml` 会恢复出一份已过期的旧文件，而真正生效的供应商/默认模型全丢；归档内保留 `profiles/<name>/` 相对路径，恢复侧按普通 dsh 外部配置写回。
- 分享导入同时写供应商和独立凭据 ref 时，使用 `save_dsh_models_provider` 的 credential 输入在同一链路保存；先快照两份文件，任一步失败恢复旧字节或删除原不存在的文件，成功后再发事件。不要改成前端先保存 Key 再独立保存供应商，否则后一步失败会遗留凭据或破坏共享引用。

- 保存或清除配置目录后，必须先刷新 `runtime_location` 的 dsh 缓存，再发 `wsl-config-changed` 和原有配置/自动同步事件。issue #331 曾因映射能解析 UNC、状态接口却没有 dsh，导致 Linux `cp` 收到 `//wsl.localhost/...` 并报 cannot stat。只修路径字符串转换不能解决重复同步。
- provider 视图的凭据回填顺序镜像 pi-ai 运行时解析顺序：先查 `records["llm-pi-ai/<route>"]`（api-key 记录取 `key` 字段回填；grant 或 env-only 记录仅标记已配置、不显示值），无记录才回查 `apiKeyEnv` 指向的 ref。因此经 dsh 官方 UI 登录的渠道在卡片上也能正确显示「已配置」。
- `delete_dsh_credential` 对不存在的 ref 是幂等 no-op（不再报错）：有效凭据可能在 records 里，清空 key 的 UI 流程必须能成功返回。
- 删除 provider 只删 `llm-pi-ai.providers.<route>`/空容器，不回滚 `agent-default-model` 默认选择；本地生效配置只在用户显式切换/应用时改写。
- 前端批量删除应先排除默认/内置渠道，并从删除前的完整 runtime view 构建计划和收藏凭据快照。一个 `apiKeyEnv` 可被多个渠道共享：仍有未删除渠道引用时保留 ref，全部删除时只删一次；各被删渠道的收藏都要保留原密钥，不能用删除上一渠道后返回的 `credentialExists` 决定下一份备份。回归见 `web/test/features/coding/dsh/utils/providerDeletion.test.ts`。
- 删除 prompt 预设只删 SQLite 记录，不改写/清空当前运行时 `AGENTS.md`。
- legacy `settings.yaml` 允许未知 top-level 与 provider 未知字段；读写必须 preserve unknown fields。profile 层同理：patch 的其它行、未知字段一律原样保留。
- 保存 Other Settings 时不要把托管键（`llm-pi-ai`、`agent-default-model`）带回文件；profile 层根本不允许保存（见上）。
- 内置 provider 即使没有写进 `llm-pi-ai.providers`（凭 env/默认可用）也不应显示为 missing；凭据缺失显示为未配置而非 missing。
- 内置模型目录（`resources/dsh_builtin_models.json`）来自 **pi-ai 1.1.0** 的 `dist/providers/data/deepseek.json`：`deepseek-flash`（V4.1 Flash，text+image）与 `deepseek-v4-pro`（text）。`deepseek-v4-flash` 与 `deepseek-v4-flash-vision-exp` 在 0.87.1 起已不存在于 catalog；`deepseek-v4-flash-vision-exp` 的能力被并入 `deepseek-flash`。compat 新增 `supportsStrictMode`（两条）与 `supportsMidConvoSystemMessages`（仅 pro）。升级 pi-ai 时按 `tauri/resources/AGENTS.md` 的提取规则刷新，不要凭记忆补字段。
- 上游 0.2.1 起 `webserver.host` 拒绝通配符地址（`isWildcardHost`：`0.0.0.0`、`::` 等），只接受具体 IPv4/IPv6 字面量；`webRuntime` 服务已被 `webStartup` 取代。这两条属于用户 profile 配置，本模块不代为改写，但排障时要能指出原因。
- agent-instructions 在 0.2.1-alpha.2 移除了 `dshHome` 配置字段，用户全局指令根为 `$DSH_HOME/AGENTS.md` 与 `$DSH_AGENTS_HOME`（默认 `~/.agents`）/AGENTS.md。本模块的启用开关仍只写 home 级 patch 的 `disabled` 与 `maxBytes`，不写 `dshHome`。
- dsh MCP 由 `mcp::cordis_patch` 适配器管理 **home 级** `~/.dsh/cordis.patch.yml`（Cordis patch DSL，format `cordis`）。0.1.7+ 的 live 配置在 profile 补丁里，本模块只在 `llm-pi-ai` / `agent-default-model` 两行上写 profile 补丁，MCP 行仍留在 home 级（home patch 在 profile patch 之后叠加，见 `readProfilePatches` 的 layer 顺序）。每个 MCP server 是一行 `insert`，包名固定 `@deepseek-ai/dsh-mcp-client`，`config.serverName` 作 key，且该 key 受上游约束 `^[A-Za-z0-9_-]{1,32}$`（写入前在 `sync_server_to_cordis` 校验，详见 `mcp/AGENTS.md`）。本模块（dsh）仍管 `settings.yaml`/`.credentials.yaml`/`AGENTS.md`，不直接写 MCP 配置。dsh 是 developer preview，cordis patch 格式可能迭代；adapter 隔离在 `cordis_patch.rs` 便于后续更新。
- `read_dsh_runtime_config` 返回的 `credentialsContent` 是 `.credentials.yaml` 原始内容，包含真实密钥；前端仅用于只读文件预览，不得把该字段当作可编辑数据回写。
- 启用 agent-instructions（`enable_dsh_agent_instructions`）会同时往 home 级 `cordis.patch.yml` 写 `disabled: false` 和 `config.maxBytes: 262144`（256 KiB），覆盖 base bundle 默认的 64 KiB 预算，避免项目根 `AGENTS.md` 一超 64 KiB 就把 `~/.dsh/AGENTS.md` 整文件挤出 baseline。`check_dsh_agent_instructions` 仍只按 `disabled` 判定启用；重复启用会幂等覆盖 maxBytes。cordis patch 写字段走 `mcp::cordis_patch::set_plugin_config_field`（合并 config、保留其它字段与行）。
- **该开关只治理 base 系 profile（tui / headless 等）。** dsh 0.1.x 起把 agent plane 移到 per-session **agent preset**：web bundle 里 `agent-instructions` 那行是 `disabled: true` 的占位墓碑，真正生效的行由每个会话的预设挂载（`maxBytes: 65536`），home 级 `cordis.patch.yml` 够不到它。因此 Web 会话既不受这个开关控制，也拿不到 256 KiB；Web 面唯一杠杆是用户自建预设 `${DSH_HOME}/.agent-presets/<id>/`（不得改随包安装的那份）。UI 文案必须如实说明这个作用域，不要再写成无条件的「全局提示词不会生效」。
- Web UI 端口唯一事实是 `dsh web` 的组合默认 **3080**（上游没有任何端口环境变量）。我们启动时不传 `--port`，让用户自己的 `webserver` 配置生效，所以探测/打开一律按 3080；用户自定义端口不被探测。将来若要支持，必须启动时传 `--port` 且探测与打开共用同一个值，绝不重新引入 env 读取。

## 最小验证

- `cargo test --test coding wsl_direct_status` 覆盖真实保存命令到 WSL 配置状态读取、UNC 两种前缀、切回本机目录和清除自定义目录后的缓存刷新；WSL MCP 过滤回归另见 `cargo test --lib coding::wsl::mcp_sync::tests`。
- `settings.yaml` 已有 `llm-pi-ai.providers.<route>` 时，编辑该 route 后其它 provider 与未知顶层键保持不变。
- profile 层：保存 provider/默认模型后，`profiles/<profile>/cordis.patch.yml` 的其它行（含 `!!js` 表达式）逐字未变；清空默认模型后 `agent-default-model` 行整体消失（回归见 `cargo test --lib coding::dsh::commands` 的 `provider_write_preserves_other_patch_rows_and_js_expressions` / `empty_managed_sections_remove_their_patch_rows`）。
- `profile_patch::read_patch_entries` / `write_patch_entries` 的 `!!js` 往返测试见同模块 `js_expressions_survive_a_round_trip`。
- 默认 provider 不在 `llm-pi-ai.providers` 又非内置时，view 应标记 missing，`save`/`delete` 返回明确错误。
- 保存默认模型后，`agent-default-model` 的 `provider`/`model` 正确写入，`reasoningEffort` 空串时被删除。
- Provider 卡片编辑 apiKey 即写 `.credentials.yaml`，且文件权限为 0600（Unix）。
- 对已是版本化布局的凭据文件保存/删除 ref 后，`version: 1` 与 `records:` 原样保留；对旧扁平文档首次写入后整体迁移成版本化布局且既有条目不丢。
- 删除已保存 prompt 后，磁盘 `AGENTS.md` 内容保持不变。

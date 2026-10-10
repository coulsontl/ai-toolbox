# Oh My Pi 后端模块说明

## 一句话职责

- `oh_my_pi/` 负责 OMP(Oh My Pi)运行时根目录、`models.yml` provider 配置与 `config.yml` 设置的可视化管理;`config.yml` 与认证数据库继续由 OMP 自己管理。

## Source of Truth

- OMP provider 的事实源是当前运行时根目录的 `models.yml`(YAML),自定义根目录存 `oh_my_pi_settings_config` 表的 `common` 记录。
- OMP 设置的事实源是当前运行时根目录的 `config.yml`(YAML,点分 camelCase 键)。
- OMP MCP server 主数据仍属于全局 MCP 模块,派生文件是当前运行时根目录的 `mcp.json`。
- 全局提示词预设存 `oh_my_pi_prompt_config` 表,写入运行时根目录的 `AGENTS.md`。
- 文件式预览由 `read_omp_runtime_config` 返回原始文件内容(`configContent`/`modelsContent`/`mcpContent`/`promptContent`),前端按文件 Tab 展示,与 Codex 一致。
- **OpenAI Codex 订阅是原生 OAuth,不是 `models.yml` provider**: 凭据在运行时根目录的 `agent.db`(`auth_credentials` 表,`provider='openai-codex'`),账号写入/切换/额度在 `accounts.rs`,模型目录来自 `omp models openai-codex --json`。`build_provider_views` 无条件插入 `openai-codex` 这个 key,使它在没有 YAML provider 时也可见。
- Codex 模型目录由 `refresh_omp_codex_catalog` 命令(前端订阅卡片的刷新按钮与页面首次加载)跑 CLI 取得,结果缓存在进程内的 `CODEX_CATALOG_CACHE`(按运行时根路径分键,失败结果也缓存)。`read_omp_runtime_config` **只读缓存,不跑 CLI**。刷新调用先核对页面的根路径；缓存发布按请求代次过滤迟到结果，防止旧超时清掉新目录。前端独立加载目录，不让首次联网发现阻塞本地配置 —— 详见下方 Gotchas 的 `omp models` 副作用。
- **subagent / roles 集中配置(OMP 侧「Subagents 集中配置」)**: 多套方案存 `oh_my_pi_agents_config` 表，分为**核心模型角色(modelRoles)**与**自定义 subagents(agents)**两层。
  1. 核心模型角色(`model_roles`): 对应 OMP 原生内置角色(`default`, `plan`, `task`, `advisor`, `commit`, `tiny`, `smol`, `slow`, `vision`)。apply 时写入运行时 `config.yml` 的 `modelRoles` 映射(`provider/modelId:thinkingLevel`)，`default` 的思考等级同步更新 `defaultThinkingLevel`。**只接管这 9 个核心角色**:`config.yml` 里方案之外的自定义 role(上游 `getKnownRoleIds` 允许任意 role 名)原样保留,方案里没写的核心角色视为用户清空。
  2. 自定义 subagents(`agents`): 对应扩展的委托代理，apply 时渲染为 `<agentDir>/agents/*.md`。方案外的自定义文件由 apply / clear applied 清理(目录 = 当前方案)。
  空库时 `__local__` 桥接态同时读取本地 `config.yml` 的 `modelRoles` 与 `agents/*.md`。
- `agents/*.md` 作为 WSL/SSH 的**目录映射** `omp-agents-dir`(`~/.omp/agent/agents`)参与同步:目录链路是整体镜像,apply 后推新文件、clear applied 后本机目录为空即清掉远端残留,不依赖"本机文件不存在就跳过"的单文件删除语义。注意 `omp-agents` 是 AGENTS.md(全局提示词),两条映射不要混用。

## 与 Pi 的差异

- OMP 没有 `auth.json`/`models.json`/`settings.json`;凭据(apiKey)直接写在 `models.yml` 的 provider 配置里,默认模型用 `modelRoles.default`(格式 `provider/modelId`)表达,思考级别用 `defaultThinkingLevel`。
- OMP 扩展是 `omp plugin` 系统(plugins),不是 Pi 的 `extensions` 命令;本地扩展目录是 `<root>/extensions`。
- OMP 的 skills 由 native 能力(priority 100)从 `<agentDir>/skills`(即 `~/.omp/agent/skills`)发现,应用把 skills 同步到该目录;不是 agents 能力(priority 70,可被 `skills.enableAgentsUser` 关闭)的 `~/.agents/skills`。
- OMP 与 Pi 都识别 `PI_CODING_AGENT_DIR`,但应用内自定义根目录分别保存。
- OMP 的 `models.yml` 配置值语法与 Pi 不同:provider `apiKey` / header 值是「先按**精确大小写**当环境变量名查,查不到就当字面量」,或以 `!` 开头的 shell 命令(10s 超时,stdout trim,进程内缓存成功结果)。命令失败/超时/空输出、或 header 解析为空时**省略该值**,不是报错——上游实现是 `packages/coding-agent/src/config/model-config-values.ts`(精确大小写查找见 `packages/utils/src/env.ts` 的 `$envExact`),不要套用 Pi 的 `$ENV_VAR` 插值规则。

## Gotchas

- `models.yml` 允许 override-only provider 和未知字段。按 provider key 写入时必须保留其他 provider 及未知字段。
- 写入 `modelRoles.default` 时必须是 `provider/modelId`(OMP `parseModelString` 按首个 `/` 拆分),裸 provider 无效。
- 不要接管 `config.yml` 全部字段;其他设置编辑器隐藏并保留 `modelRoles`/`defaultThinkingLevel`/`extensions`/`enabledModels` 等受管键。
- `defaultThinkingLevel` 的“清除”由 `OmpModelSettingsInput.clear_thinking_level` 显式驱动；前端空字符串不代表清除，避免用户在切换 provider/model 时误删全局思考级别。
- OMP 的 `thinking.mode` 是其 schema 的必填字段(`ThinkingControlModeSchema`:effort/budget/google-level/anthropic-adaptive/anthropic-budget-effort)。生成带 `thinking` 块的模型时若缺 mode,整个 models.yml 校验失败、所有自定义 provider 被禁用。前端 `buildOmpThinkingFromPreset(variants, api)` 按 api 推断 mode(google 系→google-level、anthropic-messages/bedrock→anthropic-adaptive、其余→effort);后端 `normalize_omp_provider_for_omptype` 对旧数据/手写 JSON 缺 mode 时同样兜底补上。
- provider `api` 的合法词表是 omp `ApiSchema` 的 9 个值(见前端 `web/features/coding/oh_my_pi/utils/ompApiOptions.ts`,镜像 oh-my-pi `models-config-schema-bundle.ts`);未知 `api` 值会让整个 models.yml 校验失败、禁用所有自定义 provider。但 omp 的 `Api` 类型对扩展开放(可注册自定义 API),因此前后端都不对 `api` 做枚举硬校验,保持字符串透传;provider 表单 Select 只提供词表选项,自定义值走原始 JSON 编辑。
- WSL 场景下选中 `~/.omp` 目录且其 `agent` 子目录为有效运行时布局时,归一化为 `~/.omp/agent`。
- 「获取模型」/「连通性测试」必须先按 OMP 语义解析 `apiKey`/headers 再发请求,即前端 OMP 页面传可选 `configValueMode: "omp"`,后端实现在 `tauri/src/coding/omp_config_value.rs`,本机/WSL Direct 的 host 选择共用 `tauri/src/coding/config_value_host.rs`。命令 shell 与 OMP 一致:Windows 走 `cmd /C`(Node `execSync` 默认 shell),Unix/WSL 走 `/bin/sh -c`。`!command` 解析不到时凭证被省略,且只要 provider 配置了 `apiKey`,共享命令就不再回退到 OpenCode `auth.json`(否则会把另一个工具的密钥发到该端点);只有完全没配 `apiKey` 的 provider 才保留既有的 provider 兜底。排查「OMP 没配 key 却带上 Bearer」时先看这里。
- OMP 的持久化端点与共享诊断端点分开：Anthropic 在诊断时补 `/v1`，Gemini 补版本路径，不能反写 `models.yml`。`openai-codex-responses` 诊断显式携带 apiFormat，使用 Codex 请求/终态契约；Azure、Bedrock、Gemini CLI、Vertex 及自定义 API 暂无对应诊断适配，界面禁用并说明，不能降成 Chat Completions。模型连接一致时可用模型覆盖值；不同连接混用时不再禁用，而是退回供应商级 `api`/`baseUrl` 作为目录（获取模型）端点，连通性测试则按每个模型自己的 `api`/`baseUrl` 逐个发起（协议不在连通性词表内的模型不进入测试列表）。连通性的可用性以逐模型为准：有模型时只要还有一个模型的连接在词表内就放行，全部不可测则禁用（否则会出现“按钮可用但测试列表为空”）；没有模型时才回落到代表性连接。
- 新建供应商的默认地址由表单记录自动填值来源，API 切换只更新仍由表单自动填入的地址；用户编辑或主动清空后停止自动修改。编辑/复制现有供应商不自动填地址，重新打开新建弹窗才重置自动填值状态。

- subagent 方案(agents)是数据库 profile,删除方案不会删正在运行的文件;`clear applied` 重置方案接管的 9 个核心 `modelRoles`(方案外的自定义 role 保留)并清空 `agents/*.md` 目录。`main` / `sub` 是 OMP 会话 sentinel agentName,自定义 agent 名不得使用;文件名只允许字母/数字/`-`/`_`/`.`。
- 渲染整份方案到目录时**先整体校验再写**——任一 agent 的 frontmatter 非法(缺 description、model/tools 类型错误、保留名、非法文件名)则整份拒绝,避免写到一半留下脏目录。校验镜像上游 `parseAgentFields`,所以:description 对**所有** agent(含内置名覆盖)都必填;保留名比对大小写不敏感(上游 `name.trim().toLowerCase()`);`blocking`/`readSummarize` 同时接受布尔与 `"true"`/`"false"` 字符串(上游 `parseBoolean`)。内置名(`task`/`sonic`/`scout`/`reviewer`/`security-reviewer`)与核心 role 同名不冲突:agent 名与 `modelRoles` 是两个命名空间,`agents: {task: ...}` 是合法的 bundled 覆盖,不能被前端过滤掉。
- `modelRoles` 值里的 `:level` 后缀只有落在思考级别词表(`off`/`auto` + `minimal..max`)内才当级别拆;否则整串按字面 model id 处理(Ollama tag `ollama/qwen2.5:14b`、OpenRouter `:free`)。大小写不符同样不拆——与上游 selector map 精确查表一致,避免截断 model id。后端 `parse_role_string`、前端 `parseOmpModelRoleEntry` 必须保持同一条规则。
- `defaultThinkingLevel` 是全局键:apply 时方案里 `default` 有合法词表值就写入,没有(或值非法)就删除——只增不减会让清空后的旧值继续作用于默认模型。
- 禁用「已应用」方案会撤回运行目录(清空 `agents/*.md` + 重置核心 `modelRoles`),前端必须二次确认后再调用 `toggle_omp_agents_config_disabled`;后端在"已应用且被禁用"时复用 clear applied 流程,不留"已禁用但仍应用到运行目录"的悬挂状态。
- apply/已应用方案 update 必须先对整份 `agents` 做无副作用 render/validate,再写 `config.yml` 或 `agents/` 目录;否则非法 agent 会先改 `modelRoles` 再失败,而 update 又会把 re-apply 错误吞掉。
- 备份恢复的 re-apply 编排除了全局提示词,还要按 `get_applied_omp_agents_config_id` 重新渲染 subagent 方案(用 `apply_omp_agents_config_internal_without_events`,不得在恢复期间 emit 事件)。
- 文件级 agent 编辑命令(`list_omp_agents` / `save_omp_agent` / `delete_omp_agent` 与前端同名 API 封装)是**给后续"agents/*.md 文件编辑器"铺的地基,当前没有 UI 入口**;不要当成死代码删掉,也不要误以为前端已经在用。
- **`omp models` 不是只读命令,绝不能放进 `read_omp_runtime_config`**。实测(OMP 18.8.7):它会初始化/迁移 `agent.db`、改写 `models.db`(102400→126976 字节),冷缓存时还会联网拉目录。而 `read_omp_runtime_config` 被每个 save 命令、托盘菜单和 deeplink 导入调用,所以放在那里等于每次改模型都付一次子进程 + 数据库写入,托盘还可能阻塞在网络上。目录因此改为按需刷新 + 进程内缓存,详见 Source of Truth。
- **`omp` 命令行必须带 `--profile default`**。上游 `ict()`/`v$()` 的解析顺序是 profile 环境变量优先于 `PI_CODING_AGENT_DIR`,而 `default` 是唯一表示「无 profile」的取值(上游返回 `undefined` 并清掉 `OMP_PROFILE`/`PI_PROFILE`)。实测 `OMP_PROFILE=hijack` 时 `PI_CODING_AGENT_DIR` 被完全忽略,数据库写进 `~/.omp/profiles/hijack/agent/`,而我们指定的目录保持为空——用户会看到「登录成功但状态仍是未登录」。`omp_login_command` 的本地与 WSL 两条分支都要带。
- **WSL/UNC 根目录下不得打开 `agent.db`**。`\\wsl.localhost\` 路径会经 Windows 网络重定向器访问活的 Linux WAL 数据库,它不实现 POSIX 所需的锁语义,只读打开也足以损坏或阻塞。`accounts.rs` 的 `checked_root` 已拒绝这类根,`commands.rs` 的 `read_codex_oauth_status` 同样必须在任何文件系统调用之前拒绝(WSL Direct 模式与 WSL UNC 路径都返回 `Unavailable`,不是 `Missing`)。

## 最小验证

- 新增、修改、删除一个 subagent 方案后,apply 后 `<agentDir>/agents/*.md` 目录与方案完全一致(含清理方案外自定义文件),clear applied 后目录为空且数据库 applied 标记取消。
- subagent 方案含非法 agent(缺 description / 非法文件名 / 保留名 / `model` 类型错误)时 apply 整体失败,目录保持不变。
- 手工在 `config.yml` 写一个自定义 role(如 `my-role`),apply 一个只配了核心角色的方案后,该 role 仍在;`modelRoles.default` 的思考级别清空后 `defaultThinkingLevel` 一并消失。
- `modelRoles` 写成字面 model id(如 `ollama/qwen2.5:14b`)时,打开方案弹窗再保存、apply 后该值逐字不变。
- WSL 同步下 apply / clear applied 一个方案,`~/.omp/agent/agents/` 与 Windows 侧目录一致(clear 后远端同样为空)。
- 新增、修改、删除一个 provider 后,其他 provider 和未知字段保持不变。
- `apiKey` 写成环境变量名时「获取模型」/「连通性测试」用变量值;写成 `$ENV_VAR`(非 `!` 开头)时按字面量发送(OMP 不插值);写成 `!cmd` 时执行并 trim;命令失败/空输出时不带该凭证,且不应报解析错误。
- 保存默认模型后 `config.yml` 的 `modelRoles.default` 为 `provider/modelId`。
- 安装 `omp` 后运行 `omp plugin list --json` 可列出插件。
- 打开 OMP 页面后,连续保存若干次配置(改模型、改 provider)期间 `models.db` 与 `agent.db` 的修改时间不再变化——`read_omp_runtime_config` 不再触发 CLI;只有点订阅卡片的刷新按钮才重新跑 `omp models`。
- 在导出了 `OMP_PROFILE=<任意名>` 的 shell 里,把页面给出的登录命令粘贴执行,凭据落在页面显示的那个运行目录里(而不是 `~/.omp/profiles/<名>/agent/`)。
- 运行目录选在 WSL 下时,Codex 卡片显示凭据状态「不可用」而不是「未登录」,且不产生任何对 `agent.db` 的访问。

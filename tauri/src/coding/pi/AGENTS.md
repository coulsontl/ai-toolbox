# Pi 后端模块说明

## 一句话职责

- `pi/` 负责 Pi CLI 全局 root、`settings.json`、`auth.json`、`models.json`、全局 prompt、扩展和页面 runtime view。

## Source of Truth

- Pi provider 的事实源是 Pi runtime 文件，不是 AI Toolbox 数据库 provider 表。
- `auth.json.<providerKey>` 是 API key / OAuth credential entry。
- `models.json.providers.<providerKey>` 是 custom provider 或 built-in provider override。
- `settings.json.defaultProvider/defaultModel/defaultThinkingLevel` 只表示默认启动选择，不表示唯一生效 provider。
- Pi extensions 的事实源是 Pi CLI 输出和当前 runtime root 下的 `extensions/` 目录，不是 AI Toolbox 数据库。
- `settings.json.packages` 属于 Pi 扩展/包管理链路，不属于 Other Configuration；Other Configuration 读取时隐藏它，保存时保留现有值。
- Pi MCP 配置由 `pi-mcp-adapter` 扩展消费，文件位于当前 Pi runtime root 下的 `mcp.json`；MCP server 主数据仍属于全局 MCP 模块。
- SQLite 只保存 Pi root 选择和 prompt presets；不要新增 `pi_provider`、`pi_extension` 或类似第二套主数据。
- 文件式预览由 `read_pi_runtime_config` 返回原始文件内容（`settingsContent`/`authContent`/`modelsContent`/`promptContent`），前端按文件 Tab 展示，与 Codex 一致。

## 核心设计决策

- Pi 原生支持多 provider / model，产品形态按 OpenCode 的“运行时配置可视化”处理。
- 保存 provider 时只 upsert 当前 exact runtime key；如果 key 是 `anthropic`、`openrouter` 等官方内置 key，也是在原 key 上覆盖/补充，不生成 `ai-toolbox-*` 包装 provider。
- `defaultModel` 写 Pi 官方 settings 的裸 model id。model id 本身可能包含 `/`，不要拼成 OpenCode 风格的 `provider_id/model_id`。
- 扩展管理优先通过 Pi CLI 执行 `list/install/remove/update`，并优先附带 `--no-approve`，避免非交互环境下 project trust 提示卡住；若用户 Pi CLI 过旧或不识别该 flag（例如 `Unknown option --no-approve for "list"`），必须降级重试一次不带该 flag。本地 `.ts` 文件扩展只扫描当前 runtime root 派生的 `extensions/` 目录。
- `list_pi_extensions` 会在本地 `currentVersion` 之上，对未 pin 的 `npm:` 包并发查询 registry `dist-tags.latest`，填充 `latestVersion` / `updateAvailable`。查询失败必须静默降级，不能让整个 list 失败。git/local/pin 源首版不检测。
- 扩展启用/禁用**不走 CLI**（`pi config` 只有 TUI、`pi list` 没有 `--json`），由 `set_pi_extension_enabled` 直接读改写 `settings.json`：包 → `packages` 条目写成对象 + 四类 `[]`；本地 `.ts` / 目录 → 顶层 `extensions` 写 `-extensions/xxx`(目录取 `index.ts`)。行的 `enabled` 由 `list_pi_extensions` 从 `settings.json` 反解，不依赖 `pi list` 的 `(filtered)` 文本。

## Gotchas

- 删除 prompt preset 只删 SQLite 记录，不改写/清空当前 runtime prompt 文件。产品语义是“删除已保存的提示词记录”，不是“清空本地 runtime 提示词”。
- 内置 provider 即使没有写入 `auth.json` 或 `models.json`，也可能通过环境变量或 Pi `/login` 可用；不要显示为 missing。
- `auth.json` OAuth token 是 Pi runtime-owned。AI Toolbox 可以识别和保留，但首版不编辑 token、不发起 `/login`。
- 前端 runtime view 的 `credential` 保持 `unknown`，收藏备份必须复用页面 `upsertPiFavoriteProvider` 的对象归一化，保留合法 API key/OAuth 对象及未知字段。批量入口不能把 `unknown` 直接传给仅接收 record 的配置构造函数，也不要用类型断言跳过运行时形态检查。
- `models.json` 允许 unknown top-level 和 provider/model unknown fields。读写必须 preserve unknown fields。
- Fetch Models 前端用 `findPresetModelById` 补全能力时必须保留上游返回的 model id 原文（含大小写）。preset 匹配是大小写不敏感的，只能拿 context/cost/reasoning/input 等元数据，不能把 `minimax-m3` 改写成 preset 的 `MiniMax-M3`；实现位于 `web/features/coding/pi/utils/piFetchedModels.ts`。
- Pi 0.99+ 原生支持 MCP，内置实现读取 `<runtime-root>/mcp.json`（项目级为 `.pi/mcp.json`）；`pi-mcp-adapter` 6.0.0 读取同一个文件。两者只能生效一个：扩展注册 `/mcp` 时会替换内置实现，adapter 首次启动还会自动往 `settings.json` 写 `-builtin:mcp`。MCP 页面把 Pi 作为同步目标时不需要区分实现，因为文件与 `mcpServers` 格式相同。内置实现不支持 SSE（adapter 支持），同步层只在 `packages` 里的 `pi-mcp-adapter` **实际生效**（未被本页开关写成四类 `[]`）时写入 SSE 条目，否则把该错误记进同步结果。
- 不要硬编码 `~/.pi/agent/extensions`。Pi root 可能来自应用内 custom root、`PI_CODING_AGENT_DIR`、shell 配置、默认路径或 WSL Direct，扩展目录必须从当前 runtime location 派生。
- Windows 文件夹选择器在 WSL UNC 下可能只能选到 `~/.pi`，但末段名为 `.pi` 的目录也可能是合法 custom root。Pi 设置保存和 runtime cache 刷新只有在当前目录没有 Pi runtime 数据、且其 `agent` 子目录已存在 Pi runtime 布局时，才归一化并回写为 `~/.pi/agent`；不能只凭目录名迁移。
- WSL Direct 扩展命令需要把 mise/asdf/bun/Volta/fnm/用户 npm bin 前置到原 WSL `$PATH`。动态 root 和扩展 source 必须保持为独立进程参数，不能拼进 shell 命令字符串；否则含空格路径会拆参，Shell 元字符还会改变命令结构。
- 本机 `pi` CLI 解析走共享 `cli_resolver`。Windows 上用 `bun install -g` 安装的 `pi` 默认在 `%USERPROFILE%\.bun\bin`（或 `$BUN_INSTALL\bin`）；GUI 启动时不一定继承终端 PATH，必须把 bun 全局 bin 纳入候选路径，不能只查 nvm/volta/fnm/npm。用 mise/asdf 管理 `pi` 时（常见 `mise use -g npm:earendil-works/pi-coding-agent`）真实 bin 路径含包名无法泛化，需扫 mise/asdf shims 目录命中；shim 又依赖 mise/asdf 本体在子进程 PATH，详见共享 `tauri/src/coding/AGENTS.md` 的 mise/asdf 候选与 PATH 补齐规则。
- 多路径 `pi` 时**不会**比版本选最新：先用进程 PATH 的 `which`/`where` 结果（非 Windows 取第一条，Windows 按 `.exe`/`.cmd`/… 扩展名优先级），PATH 查不到再按候选顺序（`~/.local/bin` → `/opt/homebrew/bin` → `/usr/local/bin` → node/bun 全局 bin）。扩展 list/install/remove/update 失败错误必须附带解析到的 `pi_cli=` 路径；list 成功响应应带 `cliPath`/`cliVersion` 方便对照终端里的 `where pi`。
- `pi-deck-*` 和 `ai-toolbox-*` 本地扩展按内置/受保护处理，页面不要提供直接删除入口（同理不给启用/禁用开关）。
- `pi list` 对对象形式（带 filter）的条目打印 ` (filtered)` 后缀，`parse_list_output` 必须剥掉：否则 source 带后缀会让卸载/更新用错 source、推荐列表把已安装判成未安装。
- 行级开关是**整包语义**：关 = 四类资源都写 `[]`；开 = **只摘掉本开关写的空数组**，手写/TUI 写的非空 filter 原样保留（否则会静默复活用户排除的文件；pi 自己的 TUI 也只删它切换的那一个 pattern）。`autoload` 等未知键始终保留，只剩 `source` 时退回字符串形式。
- **禁用只能用 `[]`，不能改用 `!**` 这类排除模式**（2026-10-07 用真实 `DefaultPackageManager.resolve` 实测）：`applyPatterns` 的强制包含（`+path`）在排除**之后**执行，用户只要写了一条 `+path`，`!**` 就挡不住它——包照常加载而界面显示"已禁用"。`applyPackageFilter` 对空数组走的是无条件全禁分支（官方 `docs/packages.md` 也写明 `[]` = "load none of that type"），所以 `[]` 压得过 `+path`。
- 覆盖成 `[]` 会抹掉用户自己的逐文件 filter，所以禁用时先把非空的那几类挪进包条目里的旁路键 `aiToolboxDisabledFilters`，启用时原样搬回。pi 读设置不校验未知键、`addSourceToSettings` 只改它要改的字段，所以这个键能安全留存（实测过）。回归测试：`disabling_a_package_beats_a_force_include`。
- `enabled = false` 的判定：包 = 条目四类资源键**全部存在且都为空数组**；本地 = 顶层数组里存在匹配的 `-`（精确）或 `!`（glob）。顺序与上游 `isEnabledByOverrides` 一致：`!` → `+` → `-`，后者覆盖前者。**注意 pi 只对条目上出现的类型应用 `[]`，没出现的类型仍走 `collectDefaultResources` 正常加载**——所以 `{"extensions": []}` 单独出现时该包仍在加载 skills/prompts/themes，不能判成禁用（已用 SDK 实测：四类全空 = 全禁用；只有 extensions 空 = skills 仍 `[x]`）。因此用 `pi config` TUI 手工把某包资源逐个取消勾选（写的是 `-path` 而非 `[]`）时，本页行开关仍显示"启用"；要判定"是否一个都不剩"必须枚举包内资源，当前不做。
- 实测边界（不要当成 bug 去"修"）：`packages` 里若同时存在指向同一文件的本地路径条目（如 `pi install <file>`），`resolveLocalExtensionSource` 对**文件**固定 `enabled: true`，且 package-origin 先于 auto-discovery 入表 → 顶层 `-extensions/x.ts` 对它无效，本页会显示"已禁用"而 pi 仍加载；目录型本地路径条目才会真的被 `[]` 禁用。
- 项目级（`.pi/settings.json`）条目没有开关：本页只编辑当前 runtime root 的 `settings.json`，传给后端也会因"未在 settings.json 中列出"而失败。内置扩展平时不渲染开关，但**已处于禁用态时渲染一个可用的"启用"开关**，否则手写/外部工具写下的禁用没有恢复入口。
- 顶层 `extensions` 已被加入 `PI_OTHER_SETTINGS_PROTECTED_KEYS`：开关写的就是这个键，而「其他配置」编辑器持有的是页面加载时的旧切片，不保护就会被一次 blur 静默覆盖。改这个键的写入方必须同时确认它仍受保护。
- `!` 过滤走 pi 的 `matchesAnyPattern`（minimatch，`*`/`?` 不跨 `/`），`+`/`-` 走 `matchesAnyExactPattern`（精确、会剥前导 `./`、并按 `toPosixPath` 把 `\` 归一成 `/`——Windows 上 pi 自己写出的 pattern 带反斜杠，不归一就读不出来也删不掉）。本地实现见 `glob_matches` / `normalize_pi_exact_pattern`。
- 顶层 `extensions` 不是数组时（手改出的形状）开关命令直接报错，不覆盖也不删除用户原值。
- `autoload: false` 的条目**不归本开关管**：它只是项目级 delta，基座在 `.pi/settings.json`，在这里写 `[]` 既关不掉任何东西、又会让行显示一个它控制不了的状态，所以读取侧直接按"启用"处理（不渲染假开关）。pi 侧 `applyPackageDeltaFilter` 在没有任何资源数组时确实一个都不加载——两者语义不同是有意的。
- `pi list` 每个 settings 条目打一行，同一 source 可以出现两次（普通条目 + 带 filter 的条目），`merge_extensions` 必须给重复 id 加 `#n` 后缀；否则前端 React key 重复、按 id 打补丁会同时翻转两行。
- **重复条目的读写都必须取第一条**（`effective_package_entry`）：pi 的 `dedupePackages` 对同 scope 的同身份包只保留第一条、后面的直接丢弃。实测 `["npm:x", {"source":"npm:x","extensions":[]…}]` 时 pi 仍 `[x]` 全加载；若读/写取到被丢弃的那条，就会写出 pi 根本不读的 filter，行显示"已禁用"而包照常加载。
- **`switch_supported`**：本地目录若其 `package.json` 的 `pi.extensions` 非空，pi 走 `resolveExtensionEntries` 加载的是 manifest 里那些文件（不是 `index.ts`），我们只能写 `-extensions/<dir>/index.ts` → 排除无效。这种行必须 `switch_supported: false`（不渲染开关），否则又是一个假状态。已实测：同样的 `-…/index.ts` 对「只有 index.ts 的目录」排除生效（`[ ]`），对「manifest 目录」无效（`[x]`，仍加载 `src/main.ts`）。`autoload: false` 的包条目同样置 false（基座在 `.pi/settings.json`）。
- `<root>/extensions` 的扫描走 `run_blocking_fs_operation` + `PI_EXTENSION_SCAN_TIMEOUT`（WSL UNC 下 `read_dir`/`is_file` 可能长时间阻塞；见 `coding/AGENTS.md` 的 UNC 读约定）。`read_package_current_version` 仍是同步读，属于既有问题。
- 保存 Other Configuration 时不要清空或覆盖 `settings.json.packages`；扩展管理区已经负责 package 安装、列表和卸载入口。

- `models.json` 的 provider `apiKey` 与自定义 header 值遵循 Pi 的配置值语法（`$ENV_VAR` / `${ENV_VAR}` 插值、`!command`、`$$` / `$!` 转义）。「拉取模型」和「连通性测试」必须先按 Pi 语义解析再发请求，否则会把 `$MY_API_KEY` 当字面量 Bearer 发出去（issue #355）。解析器镜像上游 `packages/coding-agent/src/core/resolve-config-value.ts`，实现在共享层 `tauri/src/coding/pi_config_value.rs`，由 `fetch_provider_models` / `test_provider_model_connectivity` 的可选 `configValueMode: "pi"` 显式开启；其它工具不传该字段，行为不变。
  - 解析失败（变量缺失、命令非 0 退出或超时）直接让该次请求失败并报出字段名/变量名，不回退成字面量——继续发送只会以 401 的形式掩盖配置错误。
  - WSL Direct 下 `$VAR` / `!cmd` 在目标发行版内解析（`wsl -d <distro> --exec printenv NAME` / `bash -c`），本机则用宿主进程环境与 Git Bash（缺失时 `cmd /C`）。本机/WSL 的 host 选择与 `printenv` 共用实现在 `tauri/src/coding/config_value_host.rs`，Pi 特有语法保留在 `pi_config_value.rs`。已知边界：本机取 GUI 进程环境、WSL 取发行版默认环境，两者都不含只写在 shell rc/profile 里、未进入进程环境的变量；未命中会报出缺失变量名而不是回退成字面量，因此是可见的失败而不是静默 401。不要为此改成 `bash -lc`/`bash -ic` 取环境——交互式 rc 的 stdout 会污染凭证。
  - Google native（`api: google-generative-ai`）把 key 放在查询串里，弹窗无法预填运行时解析结果：Pi 模式下前端不再把 key 拼进预览 URL，后端解析后用 `?key=` 补齐。

## 最小验证

- 改动 Pi 配置值解析（`tauri/src/coding/pi_config_value.rs`）或共享命令的 `configValueMode` 后，运行 `cargo test --lib pi_config_value` 与 `cargo test --lib models_api`；前端契约改动同时跑 `node --test web/test/components/common/FetchModelsModal/request.test.ts` 与 `node --test web/test/features/coding/shared/providerConnectivity/batchTest.test.ts`。
- `settings.defaultProvider = "anthropic"` 且 `auth.json`/`models.json` 没有 `anthropic` 时，provider view 应标记 built-in/default，不是 missing。
- 同一个 key 同时存在 `auth.json` 和 `models.json.providers` 时，应合并成一条 provider view。
- 保存 `models.json.providers.<key>` 只覆盖该 key，其他 providers 和 unknown top-level 字段原样保留。
- 自定义 Pi root 下的扩展列表应扫描 `<custom-root>/extensions`，不是默认 home 目录。
- WSL Direct Pi root 或扩展 source 含空格和 Shell 元字符时，`list/install/remove/update` 的命令参数边界必须保持不变，且补充 shim 后仍保留原 WSL `$PATH`。
- `pi list`（优先带 `--no-approve`，不支持时回退）返回的 package 扩展和 `extensions/*.ts` / `extensions/<dir>/index.ts` 本地扩展应合并展示。
- 扩展命令遇到 `Unknown option --no-approve` 时不能把该错误直接当作最终失败；应去掉 flag 重试，并在 UI/错误里仍允许后续提示用户升级官方 Pi CLI。
- 扩展 CLI 失败文案应包含 `pi_cli=<resolved path or wsl -d <distro> -- pi>`；list 成功时 meta 区应能看到同一路径和 `pi --version` 探测结果（探测失败可省略版本）。
- 关闭一个包：`settings.json` 该条目变成对象 + 四类 `[]`，`pi list` 出现 `(filtered)`，而页面行名不带后缀；`pi config` TUI 里该包资源全部未勾选。再打开：条目退回字符串形式、TUI 恢复勾选。
- 关闭本地扩展：顶层 `extensions` 出现 `-extensions/foo.ts`；目录扩展写的是 `-extensions/<dir>/index.ts`（不是目录名）。
- 手工写 `{"source":"npm:x","extensions":[]}`（只一类为空）时该行必须仍显示"已启用"，且 `pi` 确实还在加载该包的 skills/prompts/themes。
- `packages` 里同一 source 写两次（`["npm:x", {"source":"npm:x","extensions":[]…}]`）时该行仍显示"已启用"（pi 只认第一条）；点"禁用"必须改到第一条上，第二条原样不动。
- 手写 `!extensions/*.ts` 或 `-extensions\foo.ts`（反斜杠）后打开页面：对应行显示"已禁用"；点开关"启用"只删自己写的 `-`，用户手写的 `!`/`+` 条目逐字保留。
- 把 `settings.json` 顶层 `extensions` 改成非数组后打开页面：扩展列表照常加载（不是整块失败），只有该行开关点击时报出"不是数组"。
- 项目级（`.pi/settings.json`）的扩展行带"项目"标签、无开关；用户级行不带标签。项目级行的"已启用"只表示"本页没读到它的 filter"，不代表已验证加载状态。
- 同一 source 在 `packages` 里出现两次（普通 + 带 filter）时页面显示两行、各自独立切换，不会互相带动。
- `<root>/extensions/<dir>/package.json` 里写 `{"pi":{"extensions":["src/main.ts"]}}` 时，该行**不出现开关**（写 `-…/index.ts` 排不掉它）；把 package.json 删掉后开关恢复。
- 扩展列表在 WSL 发行版停摆时应在 `PI_EXTENSION_SCAN_TIMEOUT` 内报超时错误，而不是让面板一直 loading。
- `settings.json` 中已有 `packages` 时，`read_pi_runtime_config().other_settings` 不返回该字段，`save_pi_other_settings` 也不会删除或覆盖它。
- Pi 0.80.6 起共享 thinking ladder 是 `off/minimal/low/medium/high/xhigh/max`。AI Toolbox 的前端选项、preset `thinkingLevelMap` 转换和后端校验白名单必须同步维护这七档；缺省的标准档 `off/minimal/low/medium/high` 按 identity mapping 支持，扩展档 `xhigh/max` 必须由模型显式提供非 `null` 映射才算支持。`ultra` 属于 Codex 的主动多智能体档位，不是 Pi thinking level，不能加入 Pi settings 或模型映射。
- Fetch Models 命中大小写不同的 preset 时（例如上游 `minimax-m3` / preset `MiniMax-M3`），写入 `models.json` 的 model id 必须仍是上游原文；可运行 `pnpm test:web -- web/test/features/coding/pi/utils/piFetchedModels.test.ts`。

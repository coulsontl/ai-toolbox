# `aitb` Live E2E 用例清单

配套规则见 [`AGENTS.md`](AGENTS.md) 的「Live E2E 验证 SOP」。本文件只提供可复用的用例模板：每个用例执行前先填「现状 / 预期 / 验证 / 回滚」，填不出来就不要执行写命令。

## 0. 每次必做的前置检查

| 检查 | 做法 |
| --- | --- |
| 应用与 CLI 版本 | `aitb status --json` 确认 `enabled`、`running`、`version`；安装的是当前构建的 `aitb` |
| 网关状态 | `aitb gateway clis --json` 保存快照；确认哪些 CLI 处于 single/failover/aggregate |
| CLI 能力 | `aitb cli list --json` 保存快照；只测该 CLI 声明支持的能力 |
| 基线数据 | 保存将要触碰的 provider/model/config/prompt 的 `--json` 输出 |
| 文件基线 | 对将触碰的配置文件算 SHA-256，并复制到备份目录 |
| 数据库基线 | 复制 SQLite 主文件及 `-wal`/`-shm`；记录文件大小与修改时间 |
| 备份目录 | 项目外的时间戳目录，例如 `%TEMP%\aitb-e2e-20261128-1530\` |
| 授权模式 | 确认当前 `approvalMode`；只在本轮需要时临时调整，收尾必须恢复 `always_ask` |

> **禁止用 `claudecode` 做任何写测试。** 只读用例（`provider list/get`、`model list`、`config get`、`cli paths`）可以执行。原因见 SOP：Paseo 的 Claude Code 子代理依赖这份配置。
>
> 推荐的安全写测试对象：`zcode`、`pi`、`oh_my_pi`、`hermes`、`dsh`、`opencode`、`openclaw`（文件/运行时型），以及非 `claudecode` 的网关型 CLI（`codex` / `grok` / `kimi` / `geminicli` / `claudedesktop`）。

## 1. 通用用例模板

每个用例按下面五列记录；「操作」列写完整命令（Key 用 `--api-key-env`，不要写明文）。

| 用例 / 操作 | 现状（执行前） | 预期（成功与失败） | 验证（不只退出码） | 回滚（不符时） |
| --- | --- | --- | --- | --- |
| 示例：`aitb provider create ... --json-body <file>` | CLI 未接管；provider 数 N；config 文件哈希 H0 | 退出 0；DB 多一行；文件不变；无 `config-changed` 之外的副作用 | `aitb provider get --json`；`provider list` 计数 N+1；文件哈希 H0 | 删除合成 provider；恢复 DB/文件备份；核对 `provider list` 回到 N |
| 示例：同上的拒绝路径 | 授权弹窗出现 | 退出 4；`error.code=approval_denied`；无任何写入 | 退出码 + `--json` body；provider 数仍为 N；文件哈希 H0 | 无需回滚；确认无残留 |

## 2. 只读与授权（所有域共用）

| 用例 / 操作 | 现状 | 预期 | 验证 | 回滚 |
| --- | --- | --- | --- | --- |
| `aitb status --json` | 应用运行、控制面开启 | 退出 0；含 `enabled/running/url/discoveryFile/approvalMode`；无 token | 输出字段；`grep` 不到 token/明文 Key | 无需 |
| `aitb cli list --json` | — | 17 个 CLI；能力矩阵与 Skill §3 一致 | 对比 `providerCrud/modelCrud/config/prompts/apply/test` | 无需 |
| `aitb cli paths --cli <cli>` | 已知配置路径 | 返回真实路径；文件存在性符合现状 | 对比文件系统 | 无需 |
| 拒绝授权（任一 L1 写） | `approvalMode=always_ask` | 退出 4；`approval_denied`；弹窗关闭；无写入 | 退出码/JSON；文件哈希与 DB 快照 | 无需 |
| 等待超时 | `approvalTimeoutSecs` 调小到 15 | 退出 4；`approval_timeout`；弹窗变为过期 | 退出码/JSON；前端无残留审批卡片 | 恢复原超时 |
| `read_only` 模式写操作 | 设置切 `read_only` | 退出 4；`error.code=read_only`；后端未执行 | 退出码/JSON；文件与 DB 未变 | 恢复原模式 |
| 不支持能力 | 如 `antigravity provider create` | 退出 4 或 5；`unsupported`（400），不是 `read_only` | `--json` 的 code/status | 无需 |

## 3. 插件域

| 用例 / 操作 | 现状 | 预期 | 验证 | 回滚 |
| --- | --- | --- | --- | --- |
| `plugin list` / `get` / `providers` / `accounts` | 记录插件数与 provider 数 | 只读返回；凭据为 `***`；`hasApiKey` 保留 | 输出中无真实 Key；计数不变 | 无需 |
| `plugin install <本地目录>`（L2） | 记录插件目录与 DB | 退出 0；插件出现；`plugin list` 计数 +1；reload 事件 | `plugin get`；插件目录存在；前端列表更新 | `plugin remove`；删除安装目录；恢复 DB 备份 |
| `plugin enable/disable <id>` | 记录启用状态 | 状态翻转；reload 后页面路由 token 重新签发 | `plugin get` 状态；页面可打开/不可打开 | 反向命令；恢复状态 |
| `plugin options --set` | 记录原 options | 合并/替换符合 `--set`；非法 JSON 400 且不改 | `plugin options` 读回；DB 快照 | 回写原 options 或恢复 DB |
| `plugin logs` / `clear-logs` | 有日志 | 读取分页；clear 后为空；不影响插件运行 | 再读 logs；插件仍 enabled | 无需（日志不可恢复，接受） |
| `plugin link-cli` | 记录目标 CLI provider | URL provider 与 body `providerId` 不一致时 400；一致时创建链接 | `provider get`；CLI 列表出现插件 provider | 删除链接 provider；恢复 DB |
| `plugin remove --purge-data` | 记录插件数据目录 | 插件与数据都删除；未带 flag 时保留数据 | 目录存在性；`plugin list` | 从备份恢复目录与 DB |

## 4. Provider 域

| 用例 / 操作 | 现状 | 预期 | 验证 | 回滚 |
| --- | --- | --- | --- | --- |
| `provider list/get` | 记录 N 与字段 | 只读；`settingsConfig` 为对象；Key 为 `***`；`hasApiKey` 正确 | 输出对比；无明文 | 无需 |
| `provider create`（canonical 字段） | CLI 未接管；文件哈希 H0 | 退出 0；新 provider 的 settingsConfig 落到该 CLI 的类型化结构；文件不变（未 apply） | `provider get --json` 检查 `env`/`auth`/`config`/`modelCatalog`；文件哈希 H0 | 删除新 provider；恢复 DB/文件 |
| `provider create`（`--json-body` 原生结构） | 同上 | 原生结构原样保存；canonical 顶层字段覆盖 JSON 同名项 | `provider get`；必要时 `provider apply` 后看文件 | 同上 |
| `provider update`（不传 Key） | 记录原 Key 指纹（勿打印） | Key 保持不变；普通字段更新；`***` 不会写入 | `provider get` 的 `hasApiKey`；实际 apply 后上游仍可用（或文件中的 hash 不变） | 恢复 provider 原值/DB |
| `provider update`（传 `apiKey=***`） | 同上 | 400；不写入 | 退出码/JSON；文件哈希与 DB 未变 | 无需 |
| `provider apply` | 记录当前 applied provider 与文件哈希 H0 | 退出 0；目标 CLI 文件被该 provider 覆盖；applied 标记唯一 | `provider list` 的 `isApplied`；文件 diff；`gateway clis` 状态 | `apply` 回原 provider；或 restore 备份 |
| `provider apply`（网关接管中） | CLI 处于 single/failover/aggregate；记录清单 | 后端 restore→apply→re-engage；清单/别名/`[agents]` 不变；`gateway clis` 模式不变 | `gateway clis --json` 对比；配置文件最终仍是网关地址；manifest 哈希不变 | 重新 `gateway engage` 原模式；恢复 manifest/文件 |
| `provider apply`（插件 provider） | 记录插件 provider | 走网关 provider 路径；`customUrl` 跨 origin 时 400 且不带本机凭据 | `gateway clis`；请求日志无凭据 | 恢复 provider/网关 |
| `provider test` | 记录 provider 与 model | 成功返回延迟/模型；失败返回稳定错误；网关型 CLI 优先走网关测试 | 输出字段；网关日志出现测试请求 | 无需（只读网络） |
| `provider fetch-models` | 记录原模型列表 | 返回上游模型；不自动写入 provider（除非命令语义明确） | 对比 provider 模型列表 | 如写入则恢复原列表 |
| `provider delete`（L2） | 记录 provider、其模型、是否 applied | 删除 DB 行与其模型；applied 时行为与桌面一致；不误删他人 provider | `provider list`；`model list`；`gateway clis` | 用备份重建 provider/模型；恢复 DB |
| Antigravity 渠道 CRUD | — | `unsupported`（400），不是只读 | code/status | 无需 |

## 5. Model 域

| 用例 / 操作 | 现状 | 预期 | 验证 | 回滚 |
| --- | --- | --- | --- | --- |
| `model list` | 记录模型列表 | 只读；Codex 的默认模型出现在列表 | 输出对比 | 无需 |
| `model add` | 记录列表长度 L0 | 新模型出现；`--name` 生效；不改变 applied 状态 | `model list`；`provider get` 的 modelCatalog | `model delete`；恢复 DB |
| `model delete` | 记录目标模型是否默认 | 删除成功；默认模型回退规则与桌面一致 | `model list`；`provider get` 的 default | 重新 `model add`；恢复 DB |
| `model default` | 记录原默认模型 | 默认切换；Codex 同时写 `[chat].model`（若存在）；不破坏其它模型 | `provider get` 的 default；`model list` | 切回原默认；恢复 DB |
| 网关接管中执行 add/delete/default | CLI 已接管；记录清单 | 走 `with_gateway_reattach`；清单不变；最终仍接管 | `gateway clis`；manifest 哈希；配置文件最终地址 | 重新 engage 原清单 |

## 6. Config 域（format × mode 矩阵）

| 用例 / 操作 | 现状 | 预期 | 验证 | 回滚 |
| --- | --- | --- | --- | --- |
| `config get`（JSON） | 记录文件/DB 与哈希 | 返回对象；secret 为 `***`；无明文 | 输出；`grep` 真实 Key 不存在 | 无需 |
| `config get`（TOML/YAML） | 同上 | 返回原文文本；secret 被结构化替换为 `***`；注释/未知字段保留 | 文本对比；真实 Key 不存在 | 无需 |
| `config get --scope cli/omo` | ZCode/OMO 特有存储 | 读到对应存储；默认 scope 不受影响 | 对比两套存储 | 无需 |
| `config set` patch（JSON） | 记录原配置 | 递归合并；未知字段保留；`***`/null secret 保留旧值 | 读回；文件/db 哈希变化符合预期 | 恢复文件/DB 备份 |
| `config set` replace（JSON） | 同上 | 整份替换；`***` 从旧树回填；未传字段删除 | 读回；旧 secret 仍在 | 恢复备份 |
| `config set` patch（TOML） | 记录注释与 inline table | 递归合并；注释/未知字段/inline table 兄弟键保留；`***` 400 | 文件 diff；读回 | 恢复文件备份 |
| `config set` replace（TOML） | 同上 | 整份替换；`***` 400 | 文件内容；退出码 | 恢复备份 |
| `config set` patch（YAML） | 记录未知字段 | Mapping 递归合并；非 mapping patch 400；`***` 400 | 读回；文件 diff | 恢复备份 |
| `config set` replace（YAML） | 同上 | 整份替换；`***` 400 | 文件内容 | 恢复备份 |
| `config set --file x.yml`（OMP/Hermes/DSH） | 记录原配置 | 字符串按 YAML 解析后合并；非法 YAML 400 | 读回；退出码 | 恢复备份 |
| `config set`（CodeBuddy） | — | `read_only` 403；不写 | 退出码/JSON；文件哈希不变 | 无需 |
| `config set`（WorkBuddy） | — | `unsupported` 400；不写 | 退出码/JSON | 无需 |
| `config extract`（支持列表内） | 记录 DB 公共配置与文件哈希 | 读当前文件→保存；返回等价 `config get` | 读回对比；文件未被改（除后端重写） | 恢复 DB/文件 |
| `config extract`（网关接管中） | CLI 已接管 | 先 restore 再读用户直连配置，最后 re-engage；DB 不出现网关地址 | `config get` 的 baseUrl 不是网关地址；`gateway clis` 恢复 | 恢复 DB；重新 engage |
| `config extract`（不支持 CLI） | — | 405 `method_not_allowed`；不返回成功空对象 | 退出码/JSON | 无需 |

## 7. Prompt 域

| 用例 / 操作 | 现状 | 预期 | 验证 | 回滚 |
| --- | --- | --- | --- | --- |
| `prompt list` | 记录数量 | 只读；`isApplied` 唯一 | 输出对比 | 无需 |
| `prompt create` | 记录数量 | 新方案出现；内容来自 `--content-file`；不自动 apply | `prompt list`；内容读回 | `prompt delete`；恢复 DB |
| `prompt update`（只改名称） | 记录原内容 | 名称变；内容不变 | 读回 | 回写原名/内容 |
| `prompt update`（只改内容） | 记录原名称 | 内容变；名称不变 | 读回 | 回写 |
| `prompt apply` | 记录当前 applied | 目标变 applied；CLI 配置中的 prompt 文件写入正确 | `prompt list`；目标 CLI 配置文件/提示词文件 | `prompt apply` 原方案或 `disable`；恢复文件 |
| `prompt disable`（无 `--id`） | 有 applied | 当前 applied 被移除；其它方案保留 | `prompt list`；CLI 配置 | 重新 apply 原方案 |
| `prompt delete` | 记录方案 | 删除；applied 时行为与桌面一致；不误删 | 列表；CLI 配置 | 重建方案/恢复 DB |
| OpenClaw/WorkBuddy prompt | — | `unsupported` | code/status | 无需 |

## 8. Gateway 域

| 用例 / 操作 | 现状 | 预期 | 验证 | 回滚 |
| --- | --- | --- | --- | --- |
| `gateway status` / `health` / `logs` / `clis` | 记录状态 | 只读；status 含 running/port；logs 分页过滤正确；clis 含 mode/primary/aggregate | 输出对比；无明文凭据 | 无需 |
| `gateway start` | 已停止 | 启动成功；端口监听；`aitb status` 正常 | `gateway status`；端口；日志 | `gateway stop` |
| `gateway stop`（有接管） | 至少一个 CLI 被接管 | 后端拒绝；接管状态不变 | 退出码/JSON；`gateway clis` | 无需；先 restore 再 stop |
| `gateway restart` | 运行中 | 热重启；端口/日志重置；接管保持 | `gateway status`；`gateway clis` | 再 restart；必要时 stop/start |
| `gateway reset-health` | 有冷却条目 | 冷却清空；路由恢复 | `gateway health` | 无需（可重新触发） |
| `gateway engage single`（L2） | CLI direct；记录 provider | CLI 文件改为网关地址；manifest 生成 single；`gateway clis` 显示 single | 文件 diff；manifest；`gateway clis`；请求日志 | `gateway restore`；恢复文件/manifest 备份 |
| `gateway engage failover` | CLI direct；记录候选 | failover 清单写入；主 provider 正确；配置文件指向网关 | `gateway clis` 的 aggregate/failover 详情；manifest | `restore`；恢复备份 |
| `gateway engage aggregate`（省略字段） | 已有 aggregate 草稿/清单 | 省略字段沿用草稿；不会重置别名/站点/`[agents]`；至少一个 provider | `gateway clis`；manifest 哈希；`config get` | 重新 engage 原参数；恢复 manifest |
| `gateway switch`（已接管） | 记录 primary 与清单 | 主 provider 切换；模式/清单/别名不变；文件仍指向网关 | `gateway clis`；manifest 哈希；文件 | `switch` 回原 primary；恢复 manifest |
| `gateway switch`（未接管） | CLI direct | 与桌面一致地拒绝或按实现返回错误；不产生半接管 | 退出码/JSON；`gateway clis` 仍 direct | 无需 |
| `gateway restore`（L2） | CLI 已接管 | 文件恢复为用户直连配置；manifest 清理；`gateway clis` direct | 文件 diff；manifest；`gateway clis` | 重新 engage 原清单；恢复文件备份 |
| `gateway` 写操作拒绝授权 | 弹窗出现 | 退出 4；无状态变化 | 退出码/JSON；`gateway clis`/文件快照 | 无需 |

## 9. 收尾核对表

- [ ] 所有被测文件已按备份恢复，SHA-256 与测试前一致（预期会变的文件除外，且已说明原因）。
- [ ] SQLite 已恢复或差异已逐条解释；`-wal`/`-shm` 未遗留异常。
- [ ] 合成 provider/model/prompt/插件/网关清单已删除。
- [ ] `aitb gateway clis --json` 与测试前快照一致（模式、primary、aggregate）。
- [ ] `approval_mode` 已恢复 `always_ask`。
- [ ] 应用重启后 `aitb status --json` 与只读命令正常。
- [ ] 记录已执行/通过/未验证的用例，以及备份目录位置。
- [ ] 有价值的新场景已补成 Rust/CLI/脚本回归测试。

# 数据库表合并方案（Schema Consolidation）

> 状态：**已落地**（v33/v34 在 `feat/relay-schema-consolidation`；v35 功能域合并、v36 计价基准与上下文分档字段预留已实现并通过测试）。
> 最后更新：2026-11-05

## 一、问题

当前 `DbTable` 共 **86 张 JSONB 表**，其中 42 张是「每 CLI 一整套」：

```
Provider         12 张   claude_provider / codex_provider / ...
OfficialAccount   6 张   codex_official_account / grok_official_account / ...
CommonConfig      9 张   claude_common_config / codex_common_config / ...
PromptConfig     14 张   claude_prompt_config / codex_prompt_config / ...
SettingsConfig    6 张   workbuddy_settings_config / pi_settings_config / ...
AgentsConfig      3 张   oh_my_pi_agents_config / omo_native_agents_config / ...
GlobalConfig      2 张   oh_my_openagent_global_config / oh_my_opencode_slim_global_config
```

**后果**：每接入一个新 CLI，要改 `DbTable` 三处（enum / `ALL_TABLES` / `name()`）+ 写一个 `migrate_vNN` + 建 2~4 张表。

## 二、核对结论（实测，非推断）

### 2.1 Provider 表：字段完全同构

10 张 provider 表对应的 Rust struct，字段**逐字相同**：

```
id, name, category, settings_config, source_provider_id, website_url,
notes, icon, icon_color, sort_index, meta, is_applied, is_disabled,
created_at, updated_at
```

差异只有 `sort_index` 的 `Option<i32>` vs `i64`，可统一为 `i64`。

CLI 之间的真实差异全部装在 `settings_config: String` 这个不透明字符串里，**表结构本身没有任何 CLI 特有字段**。

### 2.2 OfficialAccount 表：字段高度重合

6 张表共有 `id, provider_id, name, kind, email, auth_snapshot, account_id, plan_type, last_refresh, last_error`，差异字段（`limit_5h_text`、`credits` 等）只出现在部分 CLI，用 JSONB 天然容纳。

### 2.3 Config 表：三类可精确映射，一类是自由 JSON

| 子类 | 张数 | 实测字段 | 结论 |
|---|---|---|---|
| PromptConfig | 14 | `id, name, content, is_applied, sort_index, created_at, updated_at` —— **14 张逐字相同** | 合 1 张 |
| CommonConfig | 9 | `config, root_dir, updated_at`；仅 Zcode 多 `official_account_index` | 合 1 张 |
| SettingsConfig | 6 | 两族：`config_dir, updated_at`（workbuddy/codebuddy/hermes）与 `root_dir, updated_at`（pi/oh_my_pi/omo_native）；Dsh 多 `profile` | 合 1 张 |
| AgentsConfig | 3 | Omp: `model_roles`；Omo: `categories + model_profiles`；Slim: `agents + council + fallback` | 自由 JSON，单列 |
| GlobalConfig | 2 | `sisyphus_agent` / `council` 等各自不同的整块 JSON | 自由 JSON，单列 |

## 三、目标结构（实际落地）

### 3.1 三张 CLI 表（55 → 3）

```sql
CREATE TABLE cli_provider (
    id TEXT NOT NULL, cli_key TEXT NOT NULL,
    data BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    PRIMARY KEY (cli_key, id)
);

CREATE TABLE cli_official_account (
    id TEXT NOT NULL, cli_key TEXT NOT NULL,
    data BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    PRIMARY KEY (cli_key, id)
);

CREATE TABLE cli_config (
    id TEXT NOT NULL, cli_key TEXT NOT NULL, kind TEXT NOT NULL,
    data BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    PRIMARY KEY (cli_key, kind, id)
);
```

`kind` 的取值：`common` / `prompt` / `settings` / `agents` / `global`。

**原方案把 `cli_config` 拆成 `cli_config` + `cli_profile` 两张，已取消**：拆分理由是「前 29 张有固定列、后 8 张是整块 JSON」，但这些表**全是 JSONB**，物理列只有 `id/data/created_at/updated_at`，没有一列是类型化的。理由不成立，合并为一张。

### 3.2 三张 relay 表（8 → 3）

```sql
CREATE TABLE relay_entity (          -- 9 种 kind 共用
    id TEXT NOT NULL, kind TEXT NOT NULL,
    data BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    PRIMARY KEY (kind, id)
);
CREATE TABLE relay_quota_snapshot (...);  -- 每次轮询重写
CREATE TABLE relay_quota_point   (...);   -- 每次轮询追加
```

`relay_entity` 的 9 个 `kind`：
`channel` / `model` / `plugin` / `plugin_provider` / `plugin_account` / `access_key` / `checkin_schedule` / `checkin_record` / `claim_record`

### 3.3 作用域由 `DbTable` 变体携带

```rust
DbTable::CliProvider(Cli::Codex)                     // cli_provider, cli_key='codex'
DbTable::CliConfig(Cli::Kimi, CliConfigKind::Prompt) // cli_config, cli_key='kimi' AND kind='prompt'
DbTable::RelayEntity(RelayKind::Channel)             // relay_entity, kind='channel'
```

所有 helper 自动加作用域谓词，不提供裸表入口。见第五节。

## 四、主键策略

**不做 `{cli_key}:{id}` 前缀拼接**。

实测依据：
- 现有 provider id 全部是 `uuid::Uuid::new_v4().simple().to_string()`（32 位 hex）
- 本机真实库：providers 24 行 → 24 个唯一 id，0 碰撞；official accounts 8 行 → 8 个唯一 id，0 碰撞
- 代码中已有 `db_clean_id()` 会剥离第一个 `:` 之前的内容，**拼前缀反而会被静默吞掉**

唯一性由 `cli_key` 列保证，所有读写强制带 `cli_key = ?`。

## 五、读写层约束

新增作用域包装，**不暴露裸表入口**：

```rust
pub struct CliScope { pub table: CliTable, pub cli_key: &'static str }
pub struct RelayScope { pub kind: &'static str }
```

漏掉 `cli_key` / `kind` 过滤的后果是**跨 CLI 读到别人的数据**，这类 bug 极难发现。所以：

- `db_list` / `db_get` / `db_put` / `db_delete` 的 CLI/relay 版本只接受 `CliScope` / `RelayScope`
- 不给裸 `DbTable` 传参的入口

## 六、迁移

### 6.1 前置清理（已执行）

本机真实库 `%APPDATA%/com.ai-toolbox/ai-toolbox.db`（原为 8 张空 `api2api_*`）：

```sql
DROP TABLE IF EXISTS api2api_channel;
DROP TABLE IF EXISTS api2api_model;
DROP TABLE IF EXISTS api2api_plugin;
DROP TABLE IF EXISTS api2api_plugin_provider;
DROP TABLE IF EXISTS api2api_plugin_account;
DROP TABLE IF EXISTS api2api_access_key;
DROP TABLE IF EXISTS api2api_checkin_schedule;
DROP TABLE IF EXISTS api2api_checkin_record;
PRAGMA user_version = 32;
```

依据：8 张表**全部 0 行**（已实测），删除不丢数据。清理后版本号退回 32，v33 可复用。

### 6.2 v33 迁移内容

1. 建 4 张 CLI 表 + 3 张 relay 表
2. 把 42 张旧表的数据搬入（`cli_key` 常量由表名推导）
3. **删旧表**（用户选定「一步到位」）
4. `TARGET_SCHEMA_VERSION` 33

### 6.3 版本号成本对比

| 操作 | 合并前 | 合并后 |
|---|---|---|
| 新增一个 CLI | 改 3 处 + 写 `migrate_vNN` + 建 2~4 张表 | 只加一个 `cli_key` 常量 |
| 给 provider 加字段 | 不影响（JSONB） | 不影响（JSONB） |
| 给 relay 加字段 | 不影响（JSONB） | 不影响（JSONB） |

## 七、最终表数量

```
合并前不同物理表名                 79   （86 个 DbTable 变体，relay 8 个变体指向同一张表）
- Provider       10 → 1          -9
- OfficialAccount 6 → 1          -5
- Config         39 → 1          -38
- api2api/relay   1 → 3 (拆成 entity + 2 张 quota 表)  +2
──────────────────────────────────────
合并后                             31   （28 张业务 JSONB 表 + model_pricing / proxy_request_logs / usage_daily_rollups）
```

注：
- `opencode_favorite_plugin` / `opencode_favorite_provider` 在 v33 时保持独立——它们是用户收藏列表，当时没有对应的 `kind`。**v34 已新增 `favorite_plugin` / `favorite_provider` kind 并入 `cli_config`**，见第十二节。
- `ssh_sync_config` / `wsl_sync_config` 与 CLI 无关；v33 时保持独立，v35 已并入 `sync_entity`（见第十三节）。
- `model_pricing` / `proxy_request_logs` / `usage_daily_rollups` 是列式表，从来不在 `DbTable` 内。

## 八、relay 功能字段（对应「模型列表 / 余额 / 领取 / 平台 / Token 续期 / LLM 请求 / 签到」）

| 需求 | 落点 | 关键字段 |
|---|---|---|
| 模型列表 | `relay_entity kind=model` | `source`(upstream/manual)、`last_synced_at`、`delisted`、`context_window`、`max_output_tokens`、`supports_*`、`cost_*`、`aliases` |
| 余额查询 | `relay_quota_snapshot` | `plan`、`user`、`balance`、`balance_unit`、`currency`、`windows_json`、`resets_json`、`status`、`error`、**`read_at` 与 `last_success_at` 分开** |
| 余额趋势 | `relay_quota_point` | `plugin_provider_id`、`account_key`、`window`、`at`、`left`、`used`、`resets_at` |
| 领取 | `relay_entity kind=claim_record` | `kind`(plan/reward/onboarding/reset_card)、`amount`、`unit`、`outcome`、`valid_until` |
| 平台 | `relay_entity`(channel/model) | `platform`：`""` / `cn` / `intl` / `global`，由插件填 |
| Token 自动续期 | `relay_entity kind=plugin_account` | `expires_at`、`refresh_expires_at`、`refresh_lead_ms`、`last_refresh_at`、`refresh_failures`、`status`、`refused_reason` |
| LLM 请求 | 复用 `proxy_request_logs` | `app_type='relay'`；`GatewayUsageTool` **必须**加 `Relay` 变体（漏登记会 HTTP 200 但统计全空） |
| 签到 | `relay_entity kind=checkin_record` | 补 `reason`、`error`、`tries`、`offline`、`valid_until`（毫秒，非字符串） |

### `relay_quota_snapshot` 的关键规则

**`read_at` 与 `last_success_at` 必须分开**：查询失败时 `status` 置 `unknown`，但 `balance` 保留上次成功值。否则会出现「查询失败 → 判定无限 → 放行 → 上游 402」。这是 agent2api 记录过的真实坑。

## 九、风险

| 风险 | 缓解 |
|---|---|
| 漏带 `cli_key` 导致跨 CLI 串数据 | 只暴露 `CliScope`，不提供裸表入口 |
| 合并后查询变慢 | `(cli_key)` 与 `(cli_key, kind)` 索引；现有数据量下差异可忽略 |
| 一步到位删旧表无法回滚 | 迁移前有自动备份；迁移在单个 SAVEPOINT 内 |
| 42 张表搬迁遗漏 | 迁移后用行数对账：旧表总行数 == 新表该 cli_key 行数 |
| `relay_entity` 混入高频表导致页竞争 | 高频的 quota 两类已独立成表 |

## 十、执行顺序

1. **前置清理**（DBA 操作，用户已批准）
2. 目录与类型改名 `api2api` → `relay`
3. `DbTable` 收敛：86 → 31（作用域直接放进变体，不另加 `CliScope`/`RelayScope` 参数）
4. 写 `migrate_v33`：建 6 张新表 + 搬迁 + 删旧表
5. helper 层自动施加作用域（`db_get`/`db_list`/`db_put`/`db_delete`/`db_count`/`db_patch_*`/`db_update_applied_status`）
6. 改所有调用点（655 处 `DbTable::` 引用、60 个文件）
7. 验证：`cargo test`、`cargo test --test sqlite_jsonb`、`cargo check --all-targets`

**本次未做（独立任务）**：relay 功能字段补全（模型列表/余额/领取/平台/Token 续期/签到）、`GatewayUsageTool::Relay` 登记。这两项与表合并正交——JSONB 加字段不需要迁移，合并完成后再做不会触发 schema 变更。

## 十一、验证结果

### 落地前（真实库，`%APPDATA%/com.ai-toolbox/ai-toolbox.db`）

| 项目 | 结果 |
|---|---|
| 迁移前备份 | `ai-toolbox.pre-consolidation-20261101.db`，121.5 MB，`integrity_check: ok` |
| 迁移前状态 | `user_version=32`，81 张表，待合并的 55 张表共 51 行 |
| 演练 | 用备份副本跑真实 `run_all`：81 → 34 张表，51 行逐条核对全部落位且内容不变 |
| 旧表清理 | 53 张旧表全部 `DROP`，无残留 |
| 索引 | 20 个作用域前缀索引全部建立 |

### 测试

| 命令 | 结果 |
|---|---|
| `cargo test` | 3445 passed, 0 failed（含 14 ignored） |
| `cargo test --test sqlite_jsonb` | 43 passed（其中 7 条为本次新增的作用域回归用例） |

新增回归用例锁死的失败模式：`db_list` 串读、`db_get` 跨 CLI 命中、`db_delete`/`db_count` 越界、`db_update_applied_status` 清空别人的 applied、字面量 id `common`/`global` 互相覆盖、重写记录产生重复行或丢 `created_at`、`relay_entity` 的 kind 串读。

## 十二、v34 收尾（2026-11-04）

v33 之后仍剩 3 张 CLI 独立表与 1 张死表，v34 一并处理：

| 旧表 | 行数（本机） | 新位置 |
|---|---:|---|
| `codex_plugin_workspace_roots` | 0 | `cli_config(cli_key='codex', kind='workspace_roots', id='settings')` |
| `opencode_favorite_plugin` | 4 | `cli_config(cli_key='opencode', kind='favorite_plugin', id=<插件名>)` |
| `opencode_favorite_provider` | 78 | `cli_config(cli_key='opencode', kind='favorite_provider', id=<provider_id>)` |
| `app_migration` | 2 | 直接删除（SurrealDB 时代的一次性数据迁移台账，已无读写点） |

- `CliConfigKind` 新增 `favorite_plugin` / `favorite_provider` / `workspace_roots` 三个 kind；不新增物理表、不新增索引。
- v33 时曾以「收藏列表映射不到任何 kind」为由保持独立；v34 的判断是：`cli_config` 的 kind 本就是开放集合，新增 kind 即可容纳，独立表才是特例。
- `usage_stats::load_opencode_provider_names` 的原生 SQL 已同步改为带 `cli_key` / `kind` 谓词查 `cli_config`。
- 迁移在单 SAVEPOINT 内「先搬后删」，失败整体回滚；新库走 v1 建旧表 → v34 搬空 → 删除，最终结构与升级库一致。
- 完成后本机物理表 34 → 30（JSONB 表 31 → 27）。

## 十三、v35 功能域表合并（2026-11-04）

v33/v34 消掉了「每 CLI 一套」的重复；v35 处理剩下的「每个功能实体一张表」。这 17 张表共享同一 JSONB 行形状与生命周期，按功能域收成 4 张 `(kind, id)` 作用域表：

| 域 | 旧表 | 新表 | 新 kind |
|---|---|---|---|
| Skill | `skill` / `skill_group` / `skill_repo` / `skill_preferences` / `skill_settings` | `skill_entity` | `skill` / `group` / `repo` / `preferences` / `settings` |
| MCP | `mcp_server` / `mcp_preferences` / `favorite_mcp` / `mcp_group` | `mcp_entity` | `server` / `preferences` / `favorite` / `group` |
| 远程同步 | `wsl_sync_config` / `wsl_file_mapping` / `ssh_sync_config` / `ssh_connection` / `ssh_file_mapping` | `sync_entity` | `wsl_config` / `wsl_defaults_version` / `wsl_file_mapping` / `ssh_config` / `ssh_defaults_version` / `ssh_connection` / `ssh_file_mapping` |
| Image | `image_channel` / `image_job` / `image_asset` | `image_entity` | `channel` / `job` / `asset` |

- 四张新表的结构与 `relay_entity` 完全同构：`(id, kind, data, created_at, updated_at)`，主键 `(kind, id)`。
- `sync_entity` 的 WSL / SSH 必须使用不同 kind：本机真实库中两边的 file mapping 有 62 个同名 id，合并 kind 会静默覆盖；`config` 与 `defaults_version` 也拆成不同 kind，否则 `db_list` 会把版本行当配置读。
- `custom_tool` 保持独立：它是 Skills 与 MCP 共用的跨域工具注册表，不属于任一域。
- `settings` / `proxy_gateway_settings` 保持独立：合并只净减 1 张表，却会碰启动与备份路径，单独评估。
- `usage_session_state`（v36 由 `gateway_session_usage_state` 改名）、`relay_quota_snapshot` / `relay_quota_point` 保持独立：高频状态 / 轮询表，和低频配置同表只会放大页竞争。
- 索引只保留与真实查询表达式一致的 9 个（name / sort / created_at / module）；`kind` 单列索引由主键 `(kind, id)` 的自动索引覆盖，`created_at` 按 JSON 表达式建索引以匹配 store 的 `ORDER BY`。
- 完成后 JSONB 表 27 → 14，物理表 30 → 17。

## 十四、计价基准与上下文分档（参考 cc-switch / models.dev）

cc-switch 的 `model_pricing` 与本项目 v36 之前逐列相同（6 列），**没有可直接抄的冗余字段**；它真正的差异是在用量侧记录计价基准：

- `proxy_request_logs.pricing_model`：写入时实际用于查价的模型 id（`pricing_model_source` 解析后的结果）。
- `usage_daily_rollups.request_model` + `pricing_model`：进主键，保证明细被 retention 清理后仍能区分「上游回显模型 / 客户端请求模型 / 实际计价模型」。

v36 之前：只有 `pricing_model_source`（配置值），没有 `pricing_model`（实际命中行）；rollup 主键是 `(date, app_type, provider_id, model, project_path)`，明细 prune 后无法还原计价维度。由于代理请求的成本在写入时已固化，这**不影响已有成本数字**，只影响历史审计与未来「缺价补价」能力。

**2026-11-05 已落地（v36，仅字段与结构预留）**：

- `proxy_request_logs` 新增 `pricing_model TEXT`（可空，与 cc-switch 一致）与 `pricing_tier TEXT`（可空，实际命中档位的阈值，`''` / `NULL` 表示基础档）。
- `usage_daily_rollups` 重建，新增 `request_model` / `pricing_model` / `pricing_tier`（均 `TEXT NOT NULL DEFAULT ''`），主键扩展为 `(date, app_type, provider_id, model, project_path, request_model, pricing_model, pricing_tier)`；历史行三列填 `''`。
- `model_pricing` 新增 `context_tiers TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(context_tiers))`，仍是一模型一行；数组元素形如
  `{"over_tokens":200000,"input_cost_per_million":"10","output_cost_per_million":"37.5","cache_read_cost_per_million":"1","cache_creation_cost_per_million":"12.5"}`，
  只存高于基础价的档位，按 `over_tokens` 升序；`over_tokens` 采用“严格大于”语义（200000 表示 >200K 生效），缺省的价格字段继承基础列。该形状同时覆盖 models.dev 的 `cost.context_over_200k` 与 `cost.tiers[{tier:{type:"context",size}}]`，200K、1M 及更多阈值都能表达；接入时需把 models.dev 的 `input` / `output` / `cache_read` / `cache_write` 映射为本表的四个列名。档位判断使用 prompt token 数（`input` + `cache_read` + `cache_creation`，不含 `output`），与 models.dev 消费端的口径一致；个别上游按 `>=` 计费时再单独确认。
- `gateway_session_usage_state` 改名为 `usage_session_state`：它是唯一使用 `gateway_` 前缀的表，同一用量领域的 `usage_daily_rollups` / `proxy_request_logs` 分别用 `usage_` / `proxy_`；名字也最长。v36 用 `ALTER TABLE ... RENAME TO` 保留全部行，冻结迁移仍创建旧名。
- **没有接入查价功能**：查价函数不读 `context_tiers`，写入不填 `pricing_tier`；除账本表名外，rollup / 读取 / 会话导入对账 SQL 的逻辑保持原样，行为与 v35 一致。新列与 PK 维度的默认值让 `INSERT OR REPLACE` 仍按原逻辑命中同一行。
- 后续要真正利用这些字段，仍需：让查价函数按请求的上下文 token 数选择档位，并返回命中的定价行 id + 档位；写入日志时填 `pricing_model` / `pricing_tier`；rollup 聚合按三个维度分组并把它们加入 `LEFT JOIN` 键；会话导入对账键同步扩展；把 models.dev 的 `context_over_200k` / `tiers` 转进 `model_pricing.json` 与远端同步。
- 不建议给 `model_pricing` 加 `provider` / `context_window` / `aliases`：这些是 models.dev 元数据，cc-switch 也没有放进价格表；`context_tiers` 只承载价格阈值，不承载上下文窗口上限。

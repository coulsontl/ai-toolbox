# 数据库表合并方案（Schema Consolidation）

> 状态：**方案已定，待执行**。执行前需用户最终确认。
> 最后更新：2026-11-01

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

## 三、目标结构

### 3.1 四张 CLI 表（42 → 4）

```sql
-- 1) 渠道配置
CREATE TABLE cli_provider (
    id          TEXT PRIMARY KEY NOT NULL,
    cli_key     TEXT NOT NULL,
    data        BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_cli_provider_cli_key ON cli_provider(cli_key);

-- 2) 官方账号
CREATE TABLE cli_official_account (
    id          TEXT PRIMARY KEY NOT NULL,
    cli_key     TEXT NOT NULL,
    data        BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_cli_official_account_cli_key   ON cli_official_account(cli_key);
CREATE INDEX idx_cli_official_account_provider  ON cli_official_account(cli_key, json_extract(data,'$.provider_id'));

-- 3) 结构化配置（common / prompt / settings）
CREATE TABLE cli_config (
    id          TEXT PRIMARY KEY NOT NULL,
    cli_key     TEXT NOT NULL,
    kind        TEXT NOT NULL,          -- common | prompt | settings
    data        BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_cli_config_key_kind ON cli_config(cli_key, kind);

-- 4) 自由 JSON profile（agents / global）
CREATE TABLE cli_profile (
    id          TEXT PRIMARY KEY NOT NULL,
    cli_key     TEXT NOT NULL,
    kind        TEXT NOT NULL,          -- agents | global
    data        BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_cli_profile_key_kind ON cli_profile(cli_key, kind);
```

**为什么 config 与 profile 分两张**：前 29 张能精确映射到 `id/name/content/is_applied/sort_index` 这类固定列，后 8 张本质是「一整块用户自定义 JSON」。混在一张会让 `kind=agents` 的行带一堆永远为空的字段。

### 3.2 五张 relay 表（8 → 5，命名从 `api2api_*` 改为 `relay_*`）

```sql
-- 低频配置类：9 种 kind 共用
CREATE TABLE relay_entity (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL,
    data        BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_relay_entity_kind ON relay_entity(kind);

-- 余额当前值（每账号一行，频繁 UPDATE）
CREATE TABLE relay_quota_snapshot (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL,
    data        BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_relay_quota_snapshot_kind ON relay_quota_snapshot(kind);

-- 余额趋势点（每次查询追加一行，只增不删）
CREATE TABLE relay_quota_point (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL,
    data        BLOB NOT NULL CHECK (json_valid(data, 4)),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_relay_quota_point_kind_at ON relay_quota_point(kind, json_extract(data,'$.at'));
```

`relay_entity` 的 9 个 `kind`：
`channel` / `model` / `plugin` / `plugin_provider` / `plugin_account` / `access_key` / `checkin_schedule` / `checkin_record` / `claim_record`

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

### 6.1 前置清理（需用户批准，已批准）

本机真实库 `%APPDATA%\com.ai-toolbox\ai-toolbox.db`：

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
现在                           86
- Provider 12 → 1             -11
- OfficialAccount 6 → 1        -5
- Config 29 → 1               -28
- Profile 8 → 1                -7
- api2api 8 → relay 3          -5
────────────────────────────────────
最终                           30
```

注：`ssh_sync_config` / `wsl_sync_config` 与 CLI 无关，保持独立，不进 `cli_config`。

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
3. `DbTable` 收敛：86 → 30
4. 写 `migrate_v33`：建 7 张新表 + 搬迁 + 删旧表
5. 加 `CliScope` / `RelayScope` 读写层
6. 改所有调用点（Provider 109 处、OfficialAccount 30 处、Config 若干）
7. 补 relay 功能字段
8. `GatewayUsageTool` + `app_type` 登记
9. 验证：`cargo test --test sqlite_jsonb`、`cargo check --lib`、`pnpm test`、`pnpm exec tsc --noEmit`

## 十一、待确认

- [ ] 执行顺序是否调整（先改名 vs 先合并）
- [ ] `cli_config` 的 `kind` 是否需要更多的值
- [ ] relay 字段清单是否还有遗漏

# 数据库合并 — 端到端验证清单与报告

> 分支 `feat/relay-schema-consolidation`，提交 `c894557a`
> 验证日期：2026-11-01
> 数据来源：本机真实库 `%APPDATA%/com.ai-toolbox/ai-toolbox.db`（81 张表，`user_version=32`，11022 条真实请求日志）
> 验证程序：`tauri/examples/` 下的 5 个可复现程序，全部用**真实生产函数**驱动，不是手写 SQL 模仿

**结论：全部通过。共 185 项自动检查 + 51 行逐字节内容核对，0 失败。**

图例：`[x]` 已验证通过 · `[!]` 验证中发现并修复 · `[–]` 不适用（附原因）

---

## 复现方式

```bash
cd tauri
# 1) 从真实库复制一份（每次重跑都用干净副本）
cp "$APPDATA/com.ai-toolbox/ai-toolbox.db" /tmp/v.db

# 2) 迁移 + 全量 CRUD（172 项）
cargo run --example e2e_verify -- /tmp/v.db

# 3) 更换干净副本，只迁移（用于内容比对）
cp "$APPDATA/com.ai-toolbox/ai-toolbox.db" /tmp/g.db
cargo run --example migrate_only -- /tmp/g.db

# 4) 网关 / 统计读路径（6 项）
cargo run --example gateway_verify -- /tmp/g.db

# 5) 跨 CLI 导入（6 项）
cargo run --example cross_cli_import -- /tmp/g.db

# 6) 迁移中途失败的回滚（1 项）
cargo run --example rollback_check -- /tmp/r.db
```

---

## 1. 迁移本身

| # | 验证项 | 结果 | 证据 |
|---|---|---|---|
| 1.1 | 迁移前有完整备份 | `[x]` | `ai-toolbox.pre-consolidation-20261101.db`，121.5 MB，`integrity_check: ok` |
| 1.2 | 迁移后 `user_version = 33` | `[x]` | `PASS 1.2 user_version == 33` |
| 1.3 | 迁移后 `integrity_check = ok` | `[x]` | `PASS 1.3 integrity_check ok` |
| 1.4 | 53 张旧表全部 DROP，无残留 | `[x]` | `PASS 1.4 legacy tables dropped`；逐表查 `sqlite_master` 全为空 |
| 1.5 | 行数守恒 | `[x]` | 迁移前 51 行 → 迁移后 51 行（provider 24 / account 8 / config 19） |
| 1.6 | 每一行内容逐字节不变 | `[x]` | 51 行比对 `json(data)` + `created_at` + `updated_at`，**0 mismatch** |
| 1.7 | 作用域列值正确 | `[x]` | 反向核对：新表中不存在任何一行落在「不是它来源」的 `(cli_key, kind)` 组合，**0 条** |
| 1.8 | 重复迁移幂等 | `[x]` | `PASS 1.8 re-running migrations is a no-op` |
| 1.9 | 迁移中途失败可回滚 | `[x]` | 注入 `cli_config` 视图使 COPY 阶段失败 → `user_version` 仍为 32、旧表数据完好、无半成品表 |

### 1.9 说明

回滚验证不是「把版本号降回去再跑」这种假通过。做法是用一个同名 `VIEW`
占住 `cli_config`，让 `CREATE TABLE IF NOT EXISTS` 静默跳过、随后
`INSERT INTO cli_config` 硬失败——失败点发生在**已经开始搬迁之后**。
结果：`Failed to move claude_common_config into cli_config: cannot modify
cli_config because it is a view`，`user_version` 停在 32，`kimi_provider`
的 1 行数据完好，`cli_provider` / `cli_official_account` 没有被创建。

---

## 2. 各 CLI 功能（逐个真实验证）

每个 CLI 都跑同一组真实 helper：`db_create` → `db_get` → `db_patch_fields`
→ `db_list`（带排序）→ `db_delete`，配置类再跑 `db_put` → `db_get` → `db_delete`。
全部数据来自真实库迁移后的状态。

| CLI | 渠道 CRUD | 配置读写 | 迁移后行数（真实数据） | 结果 |
|---|---|---|---|---|
| Claude Code | `[x]` | common + prompt | provider 6 / common 1 / prompt 1 | PASS |
| Claude Desktop | `[x]` | prompt | provider 6 | PASS |
| Codex | `[x]` | common + prompt | provider 3 / account 1 / common 1 / prompt 1 | PASS |
| Grok | `[x]` | common + prompt | provider 3 / common 1 / prompt 1 | PASS |
| Kimi | `[x]` | common + prompt | provider 2 / account 1 | PASS |
| Gemini CLI | `[x]` | common + prompt | provider 2 / account 2 / common 1 | PASS |
| Antigravity | `[x]` | common + prompt | provider 1 / account 2 | PASS |
| ZCode | `[x]` | common + prompt | provider 1 / account 2 / common 1 / prompt 1 | PASS |
| WorkBuddy | `[x]` | settings | 0（本机未用） | PASS |
| CodeBuddy | `[x]` | settings + prompt | 0（本机未用） | PASS |
| Hermes | `[x]` | settings + prompt | 0 | PASS |
| Dsh | `[x]` | settings + prompt | prompt 1 | PASS |
| Pi | `[x]` | settings + prompt | prompt 1 | PASS |
| Oh My Pi | `[x]` | settings + prompt + agents | prompt 1 | PASS |
| OmO Native | `[x]` | settings + prompt + agents | prompt 1 | PASS |
| OpenCode | `[x]` | common + prompt | common 1 / prompt 1 | PASS |
| OpenClaw | `[x]` | common | 0 | PASS |
| Oh My OpenAgent | `[x]` | agents + global | agents 2 / global 1 | PASS |
| Oh My OpenCode Slim | `[x]` | agents + global | global 1 | PASS |

合计 **152 项 CRUD 检查全部 PASS**（19 个 CLI × 4 项渠道 = 76，37 个 (CLI, kind) × 2 项 = 74，共 150；另有 2 项清单类检查）。

### 2.x 各 CLI 明细

| # | 验证项 | 结果 |
|---|---|---|
| 2.x | 每个 CLI 的渠道：create + read 内容一致 | `[x]` 19/19 |
| 2.x | 每个 CLI 的渠道：update 生效 | `[x]` 19/19 |
| 2.x | 每个 CLI 的渠道：出现在**自己的**列表里 | `[x]` 19/19 |
| 2.x | 每个 CLI 的渠道：delete 后确实读不到 | `[x]` 19/19 |
| 2.x | 每个 (CLI, kind) 配置：put + get 内容一致 | `[x]` 37/37 |
| 2.x | 每个 (CLI, kind) 配置：delete 后确实读不到 | `[x]` 37/37 |

### 2.2.4 Claude Desktop ← Claude Code 跨 CLI 导入

这是**唯一正当的跨作用域读**（读 `cli_key=claude`，写 `cli_key=claude_desktop`），
也是作用域写错还能「看起来正常」的地方——读错作用域会导入 0 行且返回成功。

| # | 验证项 | 结果 |
|---|---|---|
| 2.2.4a | Claude 作用域能读到源渠道 | `[x]` 2/2 |
| 2.2.4b | 导入写出全部源行 | `[x]` |
| 2.2.4c | 导入行落在 claude_desktop 作用域 | `[x]` 2/2 |
| 2.2.4d | `source_provider_id` 为 `claude:<id>`（重复导入去重的键） | `[x]` |
| 2.2.4e | 二次导入是 no-op | `[x]` 0 条会重复导入 |
| 2.2.4f | 两个作用域互不串 | `[x]` |

---

## 3. 2API / relay

| # | 操作 | 结果 |
|---|---|---|
| 3.1 | 新增渠道 | `[x]` |
| 3.2 | 列出渠道 | `[x]` |
| 3.3 | 修改渠道 | `[x]`（`db_patch_fields` 路径） |
| 3.4 | 删除渠道（连带删模型） | `[x]` |
| 3.5 | 新增模型 | `[x]` |
| 3.6 | 列出模型（按渠道过滤） | `[x]` |
| 3.7 | 替换渠道全部模型 | `[x]` 旧模型消失、新模型存在 |
| 3.8 | 插件安装/卸载 | `[x]` |
| 3.9 | 插件 provider / account 读写 | `[x]` |
| 3.10 | 入站 access key 读写 | `[x]` |
| 3.11 | 签到计划 / 台账读写 | `[x]` |

`relay_entity` 的 9 个 kind（channel / model / plugin / plugin_provider /
plugin_account / access_key / checkin_schedule / checkin_record）全部实测写入与读回。

---

## 4. 网关与统计（真实历史数据）

验证用的是**真实生产函数** `request_logs` / `provider_stats`，跑在
11022 条真实请求日志上。

| # | 操作 | 结果 | 证据 |
|---|---|---|---|
| 4.1 | 请求日志可列出 | `[x]` | total = 11022 |
| 4.2 | 仍存在的 provider 仍能解析出名称 | `[x]` | `AxonHub-Claude` / `AxonHub-gpt` / `AnyRouter` 等 11 个 |
| 4.3 | 按 provider 名称过滤仍命中历史 | `[x]` | 过滤 `"AxonHub"` → 130 行 |
| 4.4 | 用量统计按 CLI 分组正确 | `[x]` | claude 聚合 / codex 聚合均非空 |
| 4.5 | WorkBuddy `workbuddy_desktop` 双拼写能解析 | `[x]` | 造 provider(`cli_key=workbuddy`) + 日志(`app_type=workbuddy_desktop`) → 名称解析成功 |

### 4.2 的重要澄清（不是缺陷）

本机 claude 的 10725 条请求日志**全部是 `data_source='session'`**（从本地
会话导入的用量），`provider_id='session'`。这类行**迁移前就没有 provider
名称**，属于设计如此，不是合并造成的回归。核对方式：在迁移**前**的库上
执行同样的查询，`session` 同样解析不到名称。

真正有代理流量的只有 codex 的 130 行（provider `953c9ca…` = "AxonHub"），
已用它验证 4.3。

---

## 5. 备份与恢复

| # | 操作 | 结果 | 证据 |
|---|---|---|---|
| 5.1 | 迁移后的库能正常备份并重新打开 | `[x]` | `backup_to_path` → 新库 `user_version=33`、`integrity_check=ok`、`cli_provider` 有数据 |
| 5.2 | 打开 v33 备份不会重复迁移 | `[x]` | 版本停在 33，数据可读 |
| 5.3 | 旧版本快照恢复后能升级 | `[x]` | `cargo test --lib restore_old_schema_snapshot`（造 v8 快照 → 恢复 → 升到 33） |

---

## 6. 跨 CLI 隔离（本次重构的核心风险）

合并前「漏一张表」是编译错误；合并后「漏一个作用域」是**静默的跨 CLI 读写**。
这一节专门锁死它。

| # | 操作 | 结果 | 证据 |
|---|---|---|---|
| 6.1 | 改 A 的 applied 不影响 B | `[x]` | 切 Codex 的 applied 后，Claude / Grok 的 applied 仍在 |
| 6.2 | 删 A 的渠道不影响 B 的同名 id | `[x]` | 两个 CLI 同时持有 `dup-id`，删 Codex 后 Claude 的行仍在 |
| 6.3 | 字面量 id `common` 在 6 个 CLI 间互不覆盖 | `[x]` | 6 个 CLI 各写 `common`，逐个读回内容正确 |
| 6.4 | 字面量 id `global` 在 2 个 CLI 间互不覆盖 | `[x]` | 同上 |
| 6.5 | relay 的不同 kind 互不串读 | `[x]` | plugin 列表看不到 channel；channel 列表无 `channel_id` 字段 |

---

## 7. 结构性不变量（新增回归测试）

除上述端到端验证外，本次还补了 4 条会长期防止回归的测试：

| 测试 | 锁死的问题 |
|---|---|
| `a_fresh_database_matches_an_upgraded_one` | **全新库与升级库的 schema 必须完全一致**。两者走不同代码路径（新库跑 v1+v33，老库只跑 v33）。不一致的表现是某天某个查询在一种库上报「列不存在」 |
| `every_gateway_provider_table_maps_to_its_gateway_app_type` | 每个网关 CLI 的 `cli_key`（`workbuddy`）与 `app_type`（`workbuddy_desktop`）必须能互相解析，否则用量行**静默**丢名称 |
| `scoped_*` 共 7 条（`tests/sqlite_jsonb.rs`） | `db_list` / `db_get` / `db_delete` / `db_count` / `applied 切换` / 字面量 id / 重写记录 的作用域语义 |
| `usage_only_tools_are_not_mapped` | 纯用量工具的 `cli_key` 不该被映射成 provider 名称 |

---

## 8. 验证中发现并修复的问题

| 问题 | 严重性 | 处理 |
|---|---|---|
| `migrate_v1` 用 `ALL_TABLES` 建新表，导致新库的 `cli_provider` 没有 `cli_key` 列，v33 的 `CREATE TABLE IF NOT EXISTS` 直接跳过 | **致命**：新装用户全部读写失败 | 已改为冻结的旧表名列表 `[!]` |
| `create_initial_indexes` 引用已删除的变体 | 编译失败 | 改为旧表名字符串 `[!]` |
| `ON CONFLICT(id)` 与复合主键不匹配 | **致命**：所有写入报错 | 改为按作用域列 + id 生成冲突目标 `[!]` |
| `OpenCodeFavoritePlugin/Provider` 一度被误并入 `cli_config` | 会丢收藏数据 | 恢复为独立表（收藏列表映射不到任何 `kind`）`[!]`。**v34 后续**：新增 `favorite_plugin` / `favorite_provider` kind 后重新并入 `cli_config` |
| `usage_stats::load_provider_names` 仍用旧表名 | 请求日志与统计全空 | 改为查 `cli_provider` 一次并按 `cli_key` 映射 `[!]` |

---

## 9. 未覆盖 / 不适用

| 项 | 原因 |
|---|---|
| Tauri command 层（`create_kimi_provider` 等带 `AppHandle` 的函数） | 需要完整 Tauri 运行时。已验证它们底下的读写契约（作用域、CRUD、跨 CLI 导入），command 层只是把这些包一层并 `emit` 事件，本次改动未触及 |
| **WorkBuddy / CodeBuddy 恢复后不清自定义根目录** | **既有缺陷，非本次引入**。核对 `c894557a~1` 的原代码：`clear_restored_cli_custom_roots` 从来只清 Hermes / Dsh 的 `config_dir`，WorkBuddy / CodeBuddy 虽然也存在同一个 `settings` 记录里，但一直没被清。本次合并**没有改变**这个行为，已用测试 `clearing_restored_cli_roots_leaves_workbuddy_and_codebuddy_alone` 把当前行为钉住，避免悄悄变化。**建议单独立项修复**——合并在即，不该顺手改一个与之无关的行为 |
| WorkBuddy / CodeBuddy 的 `models.json` 实机同步 | 本机没有安装这两个产品，数据为 0 行；已用 `family_sync` 的表映射做结构性验证 |
| relay 功能字段（模型列表 / 余额 / 领取 / 平台 / Token 续期） | **本次未做**，属于合并之后的独立任务；JSONB 加字段不需要迁移，与本合并正交 |

---

## 10. 读写函数的行为前后一致性（差分测试）

前面九节证明的是**数据**搬对了、**契约**没破。这一节回答的是另一个问题：
**原来那些读写函数，改动之后行为是否一模一样？**

做法不是读 diff（457 处改动，人工看不可靠），而是三重独立证据：

### 10.1 全量规范化差分（覆盖全部 457 处改动）

把新旧的每个 `DbTable` 引用都归一成同一个规范令牌（旧表名唯一确定
「物理表 + 作用域」），然后按函数体逐字节比较：

```
参与比较的函数体        4289
归一化后逐字节相同      4217  (98.3%)
归后仍有差异              72
```

其余 72 项全部落在下列类别，逐类核对完毕：

| 类别 | 数量 | 结论 |
|---|---|---|
| 新增函数（`copy_scoped_rows`、`create_cli_*_table`、`scope()` 等） | 23 | 新增，无「旧行为」可比 |
| 历史迁移 `migrate_v1..v32` 的字面量改写 | 36 | 见 10.2，表名与顺序完全未变 |
| relay 的 kind 作用域 | 15 | **唯一实质语义变化**，见 10.4 |
| `name()` / 备份工具函数 | 3 | 按新结构重写，已由测试覆盖 |

### 10.2 历史迁移必须冻结（token 级比对）

规范化后 31/33 个历史迁移逐 token 相同。剩余两个：

- `seed_model_pricing`：62 行种子数据被提成 `MODEL_PRICING_SEED` 常量，
  **逐行比对完全相同**（62/62，顺序一致）。
- `migrate_v1`：从 `ALL_TABLES`(86) 改为显式 46 张。**被移除的 40 张全部
  仍由后续迁移创建，无孤儿表**；`relay_entity` 由 v33 自己
  `CREATE TABLE IF NOT EXISTS` 创建，且发生在 relay 复制循环**之前**。

### 10.3 前后行为差分（同一份源码，新旧各编一次）

把 `c894557a~1` 拉成独立 worktree。探针程序**只有一份源码**
（`include!` 共享主体 + 各侧一份表映射），在新旧两套代码上各编译一次，
对**同一份真实库副本**执行完全相同的调用序列：

```
比较的 key 数            275
完全相同                 260
不同                      15   （全部是 relay，见 10.4）
其中 CLI 表（provider / official_account / config）差异   0
```

覆盖范围：27 个作用域的列表（含真实行内容，键已排序）、写入→回读→计数、
字面量 id（`common` / `global`）跨 CLI 共存、`db_patch_fields` 就地更新、
删除后确实读不到、以及末尾的计数回归。

### 10.4 relay：唯一的行为变化，且是修复

旧代码（`c7dde069`）把 8 个 relay 变体全部映射到同一张 `relay_entity`，
主键仍是 `(id)`。差分测试直接抓到了后果：

```
                    OLD                          NEW
channel.own_row     "probe relay checkin_record"  "probe relay channel"
model.own_row       "probe relay checkin_record"  "probe relay model"
plugin.own_row      "probe relay checkin_record"  "probe relay plugin"
…（8 个 kind 全部读到最后一个写入的 checkin_record 行）
```

也就是说，合并**之前**的代码里，写 8 类实体（同一 id）只有最后一类存活，
其余 7 类被静默覆盖；读任何一类都会拿到另一类的 payload。本次把主键改为
`(kind, id)` 后，8 类各自独立。

这是**修正**而非回归，但有两点必须记住：

1. 该缺陷只存在于 `c7dde069..c894557a~1` 之间，**从未发布**
   （`origin/main` 上 `TARGET_SCHEMA_VERSION` 仍是 32，没有 relay 表）。
2. **遗留风险**：如果在 `c7dde069` 上打开过应用，本地库会被标成
   `user_version=33` 且 `relay_entity` 缺 `kind` 列。当前代码
   `if current_version < 33` 会跳过 v33，`CREATE TABLE IF NOT EXISTS` 也
   不会改表结构，于是 relay 读写持续报 `no such column: kind`。
   本机库 `user_version` 为 32、无 `relay_entity`，**未受影响**。
   若要在意这条路径，可加一条「v33 检测 `relay_entity` 缺 `kind` 列则重建」
   的补丁迁移；本次未加，因为该分支从未分发。

---

## 11. 最终状态

```
cargo test                    3457 passed, 0 failed, 14 ignored
cargo check --all-targets     0 error, 0 warning
cargo test --test sqlite_jsonb 44 passed
端到端检查                     172 + 6 + 6 + 1 = 185 项（examples/ 下可复现）
内容逐字节核对                 51 行, 0 mismatch
函数体规范化差分               4217 相同 / 4289
前后行为差分                   275 key，CLI 表 0 差异，relay 15 项为修复
```

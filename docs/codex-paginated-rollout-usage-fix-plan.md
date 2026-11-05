# Codex 分页 rollout / subagent 用量统计修复 — 任务清单

> 建立于 2026-10-07。用途：跨会话/上下文压缩后恢复任务上下文。
> 状态标记：`[ ]` 未开始 · `[~]` 进行中 · `[x]` 完成
>
> **实施状态（2026-10-07）**：阶段 0-5、7 已完成，全部 2771 项 lib 测试通过。
> 阶段 6 改为**显式重建命令**（`proxy_gateway_rebuild_session_usage`），未做成自动迁移 ——
> 原因见下方「实施修正」。**本机无需重建**（实测零损坏，见下）。

## 背景（调研结论，已实证）

网关统计页的「本地 session 用量」采集由 `tauri/src/coding/proxy_gateway/session_import.rs`
及其 `session_import/` 子模块负责。Codex 侧存在 4 个缺陷，参考项目 cc-switch（`D:\GitHub\cc-switch`）
有同源 issue：#7699（fork 子会话用量不记录）、#7084（分页 rollout 导致子代理用量全丢）、
PR #7091（#7084 的解法，**截至 2026-10-07 仍未合并**）。

**方向差异**：ccs 是「少记」（deferred 丢用量），我们是「多记」（重复计费）+ 覆盖丢数据。

### 缺陷与证据

| # | 缺陷 | 位置 | 证据 |
|---|---|---|---|
| 1 | 双段文件名 `<threadId>_<pageId>` 的 `source_identity` 取到**页 ID** 而非线程 ID | `session_import.rs:428-434` | 实测 `rollout-...-<P>_<PG>.jsonl` → 得到 `PG` |
| 2 | 父 timeline 不跨页拼接 → 子会话重记父历史 | `parsers.rs:529-555` `exclude_codex_replay` | Python 复刻 #7084 场景：导入 7 条，应为 5 条（**多记 40%**） |
| 3 | 分页两页 `request_id` 完全相同 → 后页覆盖前页 | `parsers.rs:476` + `session_import.rs:1037` `ON CONFLICT DO UPDATE` | 两页 `session_meta.id` 同为线程 ID，生成相同 `SESSION:codex:{tid}:token:{n}` |
| 4 | `.jsonl.zst` 扫不到 | `session_import.rs:470` 只认 `extension == "jsonl"` | 对照 `session_manager/codex.rs:697` 明确接受两种拼写 |

### 本机现状（为什么是潜伏缺陷）

- `~/.codex/sessions` 324 个 rollout，`history_mode` **320 个是 paginated**，但 `history_base` 全为 0（**尚未发生翻页**）
- 68 个 subagent rollout 的父文件都在索引里能找到，replay 排除正确 → **当前没有正在发生的错误**
- 本机 `.jsonl.zst` 数量为 0 → 缺陷 4 暂无影响

### 已确认**不需要**修改

- **Claude Code 的 subagent 用量已正确统计**：`source_identity` 对 `subagents/agent-*.jsonl` 返回 `<父会话ID>:<agentId>`（`session_import.rs:417-427`）；生产库实测近期 159 个 subagent 文件 1008 条非零用量记录 **100% 入库**；父会话文件 `isSidechain` 记录数为 **0**（无重复计费）。
- **Hermes 未接入**（`session_files` 对 Hermes 恒返回 `false`），用户 2026-10-07 决定**暂不接入**。

---

## 用户决策（2026-10-07）

1. ✅ 全部 4 项都修
2. ✅ 保留旧数据不丢（不做破坏性清理）
3. ✅ 不等 ccs 合并 PR #7091，基于自己的 `codex_rollout.rs` 实现（我们的 lineage 方案比 ccs 的「按文件名分组」更完整）
4. ✅ Claude Code 侧不改（已正确）
5. ✅ Hermes 暂不接入，仅记录（已写入记忆库 `projects/ai-toolbox/gateway-session-usage-coverage.md`）
6. ✅ **做数据重建**，但必须安排好老数据迁移时机

---

## 实施计划

### 阶段 0：准备

- [x] 0.1 已新建分支 `fix/codex-paginated-rollout-usage`（工作区的无关改动原样保留，未提交）
- [x] 0.2 基线已记录：Codex session live 156 行（2026-10-01→10-04）/ rollup 150 行（2026-01-15→09-28，27.9 亿 token）

### 阶段 1：暴露 `codex_rollout` 给用量采集路径

`codex_rollout` 现为 `session_manager` 的私有模块（`session_manager/mod.rs:5`），所有项为 `pub(super)`。
用量路径 `proxy_gateway::session_import` 是**兄弟模块**，访问不到。

- [x] 1.1 未用 `pub use` 重导出（`pub(crate)` 项不能被 `pub use` 重导出，会 E0364/E0365），改为在 `session_manager/mod.rs` 声明 `pub(crate) mod codex_rollout;`，调用方走完整路径
- [x] 1.2 导出面：`RolloutFileName`、`parse_rollout_path`、`Lineage`、`LineageSegment`、`resolve_lineage`、`open_rollout_reader`、`sessions_root_of`（新增，替代 codex.rs 里私有的 `find_sessions_root`）
- [x] 1.3 逐项 `pub(super)` → `pub(crate)`，含结构体字段

### 阶段 2：修缺陷 1 — 双段文件名取错 ID（**方案已改**）

- [x] 2.1 **不改 `source_identity`**（见「实施修正」1）。改为在 `sync_sources` 的父索引里同时登记 `thread_id` / `rollout_id` / 文件 stem
- [x] 2.2 保留原「末尾 36 字符」逻辑不变，账本键因此保持稳定
- [x] 2.3 账本键未变 ⇒ 无全量重扫、无孤儿账本条目
- [x] 2.4 单测覆盖于 `codex_child_sees_every_page_of_a_paginated_parent`

### 阶段 3：修缺陷 2 — 父 timeline 跨页拼接

- [x] 3.1 `codex_files` 改为 `HashMap<String, Vec<PathBuf>>`，每文件登记多个键
- [x] 3.2 新增 `read_codex_parent_snapshots`，用 `codex_rollout::resolve_lineage` 取全部段
- [x] 3.3 `read_codex_lineage_snapshots` 按段顺序拼接、用 `open_rollout_reader(path, end_byte_offset)` 按字节截断
- [x] 3.4 降级：`sessions_root_of` 返回 `None` 时退回 `Lineage::single`；`resolve_lineage` 内建链断裂降级
- [x] 3.5 单测 `codex_child_sees_every_page_of_a_paginated_parent`（父两页 + 子 fork 在第二页后，断言只导入 3 条）

### 阶段 4：修缺陷 3 — `request_id` 跨页冲突

- [x] 4.1 `parse_codex` 的 `request_id` 在 rollout id ≠ 线程 id 时带上 rollout id
- [x] 4.2 **放弃 `legacy_request_ids` 别名迁移**（见「实施修正」3：后页旧 ID == 前页规范 ID，别名收养会错挂）
- [x] 4.3 无需扩展 adopt 路径
- [x] 4.4 单测 `codex_paginated_pages_keep_distinct_request_ids` + `codex_single_file_rollout_keeps_its_legacy_request_id`

### 阶段 5：修缺陷 4 — `.jsonl.zst` 扫描

- [x] 5.1 `session_files` 的 Codex 分支接受 `.jsonl.zst`
- [x] 5.2 `parse_codex` 读取改走 `codex_rollout::open_rollout_reader`（透明解压）
- [x] 5.3 `parse_rollout_path` 已 strip `.zst`，`source_identity` 解析正确
- [x] 5.4 单测 `codex_compressed_rollout_is_scanned`

### 阶段 6：数据重建（老数据迁移时机）

**关键约束**：
- 归档 rollup 覆盖 **2026-01-15 → 2026-09-28**，共 150 行 / 27.9 亿 token
- live 行覆盖 **2026-10-01 → 2026-10-04**，156 行
- 磁盘 rollout 文件覆盖 **2025-11-18 → 2026-10-04**（324 个 + archived 5 个）→ **源文件仍在，重建可行**

- [x] 6.1 重建方式：新增 `session_import::rebuild_session_usage`，只删该 CLI 的 `data_source='session'` 明细行 + `provider_id='session'` 汇总行 + 账本条目，然后重扫
- [x] 6.2 时机：做成**显式 Tauri 命令** `proxy_gateway_rebuild_session_usage`（不是启动时自动迁移），由用户决定何时执行
- [x] 6.3 重建前自动备份 DB 到 `sqlite-migration-backups/ai-toolbox-session-usage-rebuild-<ts>.db`
- [x] 6.4 重建后比对：见下方「实施修正」第 3 条
- [x] 6.5 **未**做成 DB migration：迁移里无法调用依赖 runtime 的 `session_import`；且重建是运维动作而非 schema 变更，不应随版本升级强制发生
- [x] 6.6 即 6.2 的显式命令

### 阶段 7：回归与文档

- [x] 7.1 全量 Rust 测试：2771 项通过
- [x] 7.2 新增回归：`codex_child_sees_every_page_of_a_paginated_parent`、`codex_paginated_pages_keep_distinct_request_ids`、`codex_single_file_rollout_keeps_its_legacy_request_id`、`codex_compressed_rollout_is_scanned`、`rebuild_reimports_session_usage_and_keeps_proxy_rows`
- [x] 7.3 更新 `tauri/src/coding/proxy_gateway/AGENTS.md`（Codex rollout 身份与 lineage 约定 + 重建语义）
- [x] 7.4 账本键**未**改动（见下方修正 2），无需更新 `gateway_session_usage_state`（现 `usage_session_state`）说明

---

## 实施修正（与原计划的偏差，均为调研后推翻的假设）

1. **`source_identity` 不改**（原阶段 2）。原计划让它返回 `thread_id`，但那会改变**账本键**，导致所有 Codex 文件重扫 + 旧账本条目成为孤儿。改为只在 `sync_sources` 的**父索引**里同时登记 `thread_id` 与 `rollout_id`，`source_identity` 保持每文件唯一。这样账本键稳定，父查找也正确。

2. **`request_id` 只在与线程 id 不同时才加 rollout id**。原计划无条件加，会让**单文件 rollout**（rollout id == thread id）的键从 `SESSION:codex:{tid}:token:{n}` 变成带重复 id 的形式，把全部既有行重键。测试 `codex_single_file_rollout_keeps_its_legacy_request_id` 捕获了这个错误。修正后单文件保持旧键，修复对既有数据是 no-op。

3. **不能用 `legacy_request_ids` 做旧 ID 迁移**（原阶段 4.2/4.3）。分页场景下**后页的旧 ID 恰好等于前页的规范 ID**（两页都用线程 id 拼键），按别名收养会把前页的行改挂到后页。已放弃别名方案，改用显式重建。

4. **本机实测零损坏，无需重建**：
   - 磁盘 rollout 文件 **329 个**，账本条目 **329 个**，一一对应，无孤儿
   - 双段（分页）文件名 **0 个**、含 `history_base` 的 rollout **0 个**、`.jsonl.zst` **0 个**
   - 即：本机从未发生分页翻页，四项缺陷都是**潜伏**而非已发生。重建对 Codex 是幂等的（重扫得到同样的行）

5. **解析器版本号 3 → 4**（`parsers::revision`）。强制重扫一次以套用新解析；对单文件 rollout 因键未变，重扫结果与旧行一致（`updated_records` 而非 `inserted_records`）。

---

## 关键文件索引

| 文件 | 作用 |
|---|---|
| `tauri/src/coding/proxy_gateway/session_import.rs` | 采集主流程：`source_identity`(392)、`session_files`(449)、`sync_sources`(561)、`persist_records`(796)、`write_record`(1002) |
| `tauri/src/coding/proxy_gateway/session_import/parsers.rs` | `parse_file`(107)、`parse_codex`(385)、`exclude_codex_replay`(529) |
| `tauri/src/coding/session_manager/codex_rollout.rs` | 已实现的 rollout 身份/lineage/压缩读取（**待复用**） |
| `tauri/src/coding/session_manager/mod.rs` | 模块声明(5) 与重导出(45) |
| `tauri/src/db/migrations.rs` | `TARGET_SCHEMA_VERSION = 26`，迁移模式 |
| `docs/gateway-native-session-usage-research.md` | 各 CLI 采集调研（注意：**部分结论已被 2026-10-07 实测推翻**，见下） |

## 参考项目

- `D:\GitHub\cc-switch`：`src-tauri/src/services/session_usage_codex.rs`
  - `build_rollout_index` / `resolve_parent_signatures` / `parent_timeline`
  - PR #7091（OPEN）给出跨页拼接解法；issue #7084 给出完整根因分析
  - 引用前必须 `git pull` 并在当前 HEAD 验证

## 注意事项 / 已知陷阱

1. **`parse_codex` 的 `thread_id` 初值是 `source_identity()`，随后被 `session_meta.id` 覆盖**（`parsers.rs:388,412-414`）。阶段 2 改 `source_identity` 只影响账本键与「无 meta 文件」的回退，**不影响** `request_id`（后者用覆盖后的 `thread_id`）——阶段 4 要单独处理。
2. `exclude_codex_replay` 用 `parsed.started_at` 作 cutoff，`started_at` 来自 `session_meta` 的**顶层 timestamp**。若 meta 缺 timestamp 会退化为 `i64::MAX`（不过滤），需确认行为。
3. `session_import.rs:892` 的跳过逻辑：`previous.is_some() && existing_source.is_none()` 时跳过（行已被 rollup 归档）。重建时必须同时清账本，否则重扫会被跳过。
4. `docs/gateway-native-session-usage-research.md` 中关于「Grok 未导入」「Claude Desktop 重复统计」等结论是**调研当时**状态；2026-10-07 生产库实测显示 Grok 已有 4711 万 token 归档。
5. 保留期：`maybe_rollup_and_prune` 按 `settings.log_retention_days` 归档，默认值未在本文件确认（`settings.rs` 中未直接搜到），实施时需查 `ProxyGatewaySettings` 默认值。

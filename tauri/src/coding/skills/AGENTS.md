# Skills 模块架构文档

## 零、迁移期高优先级补充规则

- Skills 的唯一源目录始终是中央仓库 `central_repo_path`。Claude/Codex/OpenCode/OpenClaw 或任何自定义工具当前运行时的 skills 目录都只是目标目录，不是同步源。
- `skills_sync_to_tool` 的语义始终是“中央仓库 -> 工具运行时 skills 目录”。如果工具当前配置落在 WSL，该目标目录可能解析成 `\\\\wsl.localhost\\...` UNC 路径，但源目录仍不变。
- `management_enabled=false` 是后端必须维护的同步 invariant，不只是前端展示状态。任何会写工具目录的入口（`skills_sync_to_tool`、tray toggle、全量 resync、Inventory apply）都必须先确认 Skill 已启用；禁用 Skill 的唯一恢复路径是先 enable，再走明确的工具恢复/同步流程。
- WSL skills 同步和 SSH skills 同步都不是复用普通 file mappings；它们是独立链路，但源端仍然是中央仓库。
- 对已经 `is_wsl_direct` 的内置工具，处理 WSL skills 同步时要优先判断“目标目录是否已直接在 WSL 内”，而不是只看当前 Windows 侧是否存在 UNC 显示路径。

## 一、模块概述

Skills 模块提供 AI 编程工具技能的统一管理功能。用户可以从本地文件夹或 Git 仓库安装技能，并同步到多个 AI 编程工具（Claude Code、Cursor、Codex、OpenCode 等）。模块还支持技能发现、导入现有技能、系统托盘快捷菜单等功能。

## 二、文件结构

### Rust 后端 (tauri/src/coding/skills/)

| 文件 | 职责 |
|------|------|
| mod.rs | 模块导出 |
| types.rs | 核心数据结构和 DTO 定义 |
| adapter.rs | 数据库记录与 Rust 结构体的转换 |
| skill_store.rs | SurrealDB 增删改查操作 |
| commands.rs | Tauri 命令（前端 API 接口） |
| installer.rs | 技能安装逻辑（本地/Git） |
| sync_engine.rs | 文件同步引擎（符号链接/接合点/复制） |
| tool_adapters.rs | 工具检测和路径解析 |
| onboarding.rs | 技能发现（扫描已安装工具） |
| central_repo.rs | 中央仓库管理 |
| git_fetcher.rs | Git 克隆/拉取操作 |
| cache_cleanup.rs | Git 缓存清理 |
| content_hash.rs | 目录内容哈希计算 |
| tray_support.rs | 系统托盘菜单集成 |

### 前端 (web/features/coding/skills/)

| 目录/文件 | 职责 |
|----------|------|
| services/skillsApi.ts | Tauri invoke 封装 |
| stores/skillsStore.ts | Zustand 状态管理 |
| hooks/useSkills.ts | 技能数据 Hook |
| hooks/useSkillActions.ts | 操作处理 Hook |
| hooks/useToolStatus.ts | 工具检测 Hook |
| components/SkillsModal.tsx | 主模态框 |
| components/SkillsList.tsx | 技能列表（支持拖拽排序） |
| components/modals/ | 子模态框（添加、导入、设置等） |
| utils/errorHandlers.ts | 错误解析和对话框 |
| utils/syncHelpers.ts | 同步操作辅助函数 |

## 三、数据库表结构

数据存储在 SurrealDB 中，采用宽表模式减少表关联。

### 3.1 skill 表（技能主表）

| 字段 | 类型 | 说明 |
|------|------|------|
| id | string | UUID，主键 |
| name | string | 技能名称 |
| source_type | string | 来源类型：local / git / import |
| source_ref | string? | 来源引用（本地路径或 Git URL） |
| source_revision | string? | Git 版本号 |
| central_path | string | 中央仓库中的绝对路径 |
| content_hash | string? | 内容哈希（用于变更检测） |
| created_at | i64 | 创建时间戳（毫秒） |
| updated_at | i64 | 更新时间戳（毫秒） |
| last_sync_at | i64? | 最后同步时间戳 |
| status | string | 状态：ok / error |
| sort_index | i32 | 排序索引（拖拽排序用） |
| user_group | string? | 旧版/兼容分组名称字段；新逻辑以 `group_id` 为准，写入时同步回填名称便于兼容读取 |
| group_id | string? | 指向 `skill_group` 的稳定内部分组 ID；重命名 group 不迁移 skill，只更新 group 记录本身 |
| user_note | string? | AI Toolbox 内部自定义备注，不写入 SKILL.md，不参与内容哈希 |
| management_enabled | bool | AI Toolbox 管理启用状态；不是 `status` 健康状态，false 表示 UX 禁用并从当前工具取消同步 |
| disabled_previous_tools | array | 禁用前记录的工具绑定 key；重新启用时用于默认恢复勾选 |
| enabled_tools | array | 已启用的工具列表，如 ["claude_code", "codex"] |
| sync_details | object? | 每个工具的同步详情（嵌入式 JSON） |

**sync_details 嵌入字段：**

| 字段 | 类型 | 说明 |
|------|------|------|
| target_path | string | 工具目录中的目标路径 |
| mode | string | 同步模式：symlink / junction / copy |
| status | string | 同步状态：ok / error |
| synced_at | i64? | 同步时间戳 |
| error_message | string? | 错误信息 |

### 3.2 skill_preferences 表（偏好设置，单例）

| 字段 | 类型 | 说明 |
|------|------|------|
| id | string | 固定为 "default" |
| central_repo_path | string | 中央仓库路径，默认为应用数据目录/skills |
| preferred_tools | array? | 首选工具列表 |
| default_view_mode | string | 进入 Skills 页面时的默认 UI 视图：`flat` / `grouped`，非法或缺失时回退 `flat` |
| git_cache_cleanup_days | i32 | Git 缓存清理天数，默认 30 |
| git_cache_ttl_secs | i32 | Git 缓存 TTL 秒数，默认 60 |
| known_tool_versions | object? | 已知工具版本信息 |
| installed_tools | array? | 已检测到的已安装工具 |
| show_skills_in_tray | bool | 是否在托盘菜单显示技能 |
| updated_at | i64 | 更新时间戳 |

### 3.3 skill_repo 表（Git 仓库源）

| 字段 | 类型 | 说明 |
|------|------|------|
| id | string | 格式：owner/name |
| owner | string | 仓库所有者 |
| name | string | 仓库名称 |
| branch | string | 分支，默认 main |
| enabled | bool | 是否启用 |
| created_at | i64 | 创建时间戳 |

### 3.4 custom_tool 表（自定义工具）

| 字段 | 类型 | 说明 |
|------|------|------|
| id/key | string | 唯一标识符（字母数字下划线） |
| display_name | string | 显示名称 |
| relative_skills_dir | string | Skills 目录相对路径（相对于 HOME） |
| relative_detect_dir | string | 检测目录相对路径（用于判断是否安装） |
| created_at | i64 | 创建时间戳 |

### 3.5 skill_group 表（Skill 手动分组）

`skill_group` 是 first-class 分组 registry，用于保存手动分组本身的元数据。它不是工具同步 Profile，也不改变中央仓库目录结构或任何工具目标路径。

| 字段 | 类型 | 说明 |
|------|------|------|
| id | string | UUID，稳定内部主键；Inventory JSON 不暴露该字段 |
| name | string | 分组名称；应用内唯一，Inventory JSON 通过该名称引用分组 |
| note | string? | 分组备注，用于记录分组用途 |
| sort_index | i32 | 分组排序 |
| created_at | i64 | 创建时间戳 |
| updated_at | i64 | 更新时间戳 |

旧数据迁移：读取列表或分组时会将非空 `skill.user_group` 自动迁移为 `skill_group` 记录，并把对应 skill 写入稳定 `group_id`。迁移后业务归属以 `group_id` 为准，`user_group` 只作为兼容展示/旧数据回填字段。

## 四、详细流程说明

### 4.1 技能发现流程

技能发现用于扫描用户已安装的 AI 工具，找出已存在的技能并提供导入选项。

**触发时机：** 打开 ImportModal 时调用 `skills_get_onboarding_plan`

**处理流程：**

1. **获取中央仓库路径**
   - 读取 skill_preferences 中的 central_repo_path
   - 默认为应用数据目录/skills（如 Windows: `%APPDATA%/com.ai-toolbox/skills`）

2. **获取已管理的目标路径**
   - 查询所有 skill 记录的 sync_details
   - 提取每个工具的 target_path 构建排除列表

3. **遍历所有工具适配器**
   - 包括 14 个内置工具 + 用户自定义工具
   - 检查每个工具的 relative_detect_dir 是否存在（判断是否安装）

4. **扫描已安装工具的 skills 目录**
   - 读取 relative_skills_dir 下的所有子目录
   - 跳过特殊目录（如 Codex 的 .system）
   - 检测是否为符号链接/接合点，记录 link_target

5. **过滤已管理的技能**
   - 排除 link_target 指向中央仓库的技能
   - 排除已在 sync_details 中记录的目标路径

6. **计算内容哈希**
   - 对每个发现的技能目录计算 SHA256 哈希
   - 用于检测不同工具中同名技能是否内容一致

7. **按技能名称分组**
   - 同名技能归为一组
   - 比较组内各变体的 fingerprint
   - 如果存在不同哈希值，标记 has_conflict = true
   - 记录每个变体的 conflicting_tools 列表

8. **返回 OnboardingPlan**
   - total_tools_scanned: 扫描的工具数量
   - total_skills_found: 发现的技能总数
   - groups: 分组后的技能列表

### 4.2 本地安装流程

从本地文件夹安装技能到中央仓库。

**入口函数：** `install_local_skill`

**处理流程：**

1. **验证源路径**
   - 检查路径是否存在
   - 如果不存在，返回错误

2. **确定技能名称**
   - 检查源目录下是否有 SKILL.md
   - 如果有，解析 YAML frontmatter 中的 name 字段
   - 如果没有或解析失败，使用目录名作为技能名

3. **准备中央仓库**
   - 解析 central_repo_path（支持 ~ 展开）
   - 确保目录存在，不存在则创建

4. **检查目标是否已存在**
   - 目标路径：central_repo_path/{name}
   - 如果已存在且 overwrite=false，返回 `SKILL_EXISTS|{name}` 错误
   - 如果已存在且 overwrite=true，删除现有目录

5. **复制技能内容**
   - 调用 copy_skill_dir 复制目录
   - 跳过 .git 目录
   - 解析顶层符号链接，复制实际内容
   - 处理 Windows Git 的文本符号链接

6. **计算内容哈希**
   - 遍历目录所有文件
   - 计算整体 SHA256 哈希

7. **创建数据库记录**
   - 生成新的 UUID 作为 skill_id
   - 获取当前最大 sort_index + 1
   - 写入 skill 表

8. **返回 InstallResult**
   - skill_id: 新技能 ID
   - name: 技能名称
   - central_path: 中央仓库路径
   - content_hash: 内容哈希

### 4.3 Git 安装流程

从 Git 仓库安装技能。

**入口函数：** `install_git_skill`

**处理流程：**

1. **初始化代理设置**
   - 从应用设置读取代理配置
   - 设置 git_fetcher 的代理

2. **解析 Git URL**
   - 支持多种格式：
     - 完整 URL: `https://github.com/owner/repo`
     - 带分支: `https://github.com/owner/repo/tree/main`
     - 带子路径: `https://github.com/owner/repo/tree/main/path/to/skill`
     - 简写: `owner/repo`
   - 提取：clone_url、branch、subpath

3. **克隆或更新缓存**
   - 计算缓存 Key: SHA256(clone_url + branch)
   - 缓存目录: ~/.cache/ai-toolbox/skills-git-cache/{key}
   - 检查缓存是否存在且未过期（TTL 检查）
   - 如果需要更新，执行 git clone 或 git pull
   - 记录 HEAD revision

4. **确定复制源**
   - 如果 URL 包含 subpath，使用 repo_dir/subpath
   - 否则扫描仓库查找 SKILL.md：
     - 先检查根目录
     - 递归扫描子目录（跳过 .git）
     - 如果找到多个，返回 `MULTI_SKILLS|` 错误
     - 如果找到一个，使用该目录
     - 如果没找到，使用仓库根目录

5. **确定技能名称**
   - 从 copy_src 目录的 SKILL.md 读取 name
   - 如果没有，从仓库 URL 提取仓库名

6. **复制到中央仓库**
   - 同本地安装流程步骤 3-6

7. **构建完整 source_ref**
   - 如果使用子目录，构建 tree URL
   - 格式：`https://github.com/owner/repo/tree/branch/subpath`
   - 用于后续更新时定位正确目录

8. **创建数据库记录**
   - source_type = "git"
   - source_ref = 完整 URL（含分支和子路径）
   - source_revision = Git HEAD revision

9. **返回 InstallResult**

### 4.4 多技能仓库处理流程

当仓库包含多个技能时的处理流程。

**触发条件：** `install_git_skill` 发现多个 SKILL.md

**处理流程：**

1. **后端返回错误**
   - 返回 `MULTI_SKILLS|` 前缀错误

2. **前端捕获错误**
   - 调用 `skills_list_git_skills` 获取候选列表
   - 弹出 GitPickModal 让用户选择

3. **用户选择后安装**
   - 调用 `skills_install_git_selection`
   - 传入 repoUrl、subpath、branch
   - 后端直接使用指定 subpath 安装

4. **批量选择处理**
   - 用户可多选
   - 循环调用 install_git_selection
   - 遇到 SKILL_EXISTS 错误时提供覆盖选项
   - 支持"全部覆盖"选项

### 4.5 Git 缓存机制

Git 仓库的本地缓存策略。

**缓存位置：** `~/.cache/ai-toolbox/skills-git-cache/`

**缓存结构：**
```
skills-git-cache/
├── {sha256_hash_1}/           # 仓库缓存目录
│   ├── .git/                  # Git 元数据
│   ├── .skills-cache.json     # 缓存元信息
│   └── ...                    # 仓库内容
└── {sha256_hash_2}/
    └── ...
```

**缓存 Key 计算：**
- 输入：clone_url + "\n" + branch
- 算法：SHA256
- 结果：64 字符十六进制字符串

**缓存元信息 (.skills-cache.json)：**
| 字段 | 说明 |
|------|------|
| last_fetched_ms | 上次拉取时间戳（毫秒） |
| head | 当前 HEAD commit hash |

**TTL 检查逻辑：**
1. 检查 .git 目录是否存在
2. 读取 .skills-cache.json
3. 计算时间差：now - last_fetched_ms
4. 如果小于 git_cache_ttl_secs × 1000，使用缓存
5. 否则执行 git pull 更新

**缓存清理：**
- 定时任务：根据 git_cache_cleanup_days 清理过期缓存
- 手动清理：调用 `skills_clear_git_cache` 立即清空
- 损坏恢复：如果 clone/pull 失败，删除缓存目录后重试

**并发控制：**
- 使用 `OnceLock<Mutex<()>>` 全局锁
- 防止多个请求同时操作同一缓存目录

### 4.6 技能更新流程

从源重新拉取技能内容并更新。

**入口函数：** `update_managed_skill_from_source`

**处理流程：**

1. **获取技能记录**
   - 根据 skill_id 查询 skill 表
   - 获取 source_type、source_ref、central_path

2. **验证中央仓库路径**
   - 确认 central_path 存在
   - 获取父目录用于临时目录

3. **创建临时目录**
   - 目录名：`.skills-update-{uuid}`
   - 用于构建新内容，避免更新失败导致数据丢失

4. **根据 source_type 拉取内容**

   **Git 类型：**
   - 解析 source_ref URL
   - 调用 clone_to_cache 更新缓存
   - 记录新的 revision
   - 复制到临时目录

   **Local 类型：**
   - 读取 source_ref 路径
   - 验证源路径仍然存在
   - 复制到临时目录

5. **替换中央仓库内容**
   - 删除原 central_path 目录
   - 尝试 rename 临时目录到 central_path
   - 如果 rename 失败（跨分区），使用 copy + delete

6. **计算新的内容哈希**
   - 对更新后的目录计算哈希
   - 用于检测内容是否真正变化

7. **更新数据库记录**
   - 更新 content_hash
   - 更新 source_revision（Git 类型）
   - 更新 updated_at 时间戳

8. **重新同步 copy 类型的目标**
   - 遍历 sync_details 中所有目标
   - 跳过未安装的工具
   - 对于 mode=copy 或 tool=cursor 的目标：
     - 重新执行 copy 操作
     - 更新 synced_at 时间戳
   - symlink/junction 自动指向新内容，无需处理

9. **返回 UpdateResult**
   - skill_id: 技能 ID
   - name: 技能名称
   - content_hash: 新哈希
   - source_revision: 新版本号
   - updated_targets: 重新同步的工具列表

### 4.7 工具同步流程

将技能同步到指定工具。

**入口函数：** `skills_sync_to_tool`

**先记住这个约束：**
- Skills 的源目录始终来自中央仓库 `central_repo_path`。
- Claude/Codex/OpenCode/OpenClaw 的运行时 skills 目录只是同步目标，不是源。
- 当这些工具的配置目录改成 WSL 路径时，目标目录可能会被解析成 `\\\\wsl.localhost\\...`，但同步源仍然是中央仓库。

**处理流程：**

1. **获取工具适配器**
   - 先查找内置工具
   - 再查找自定义工具
   - 未找到返回错误

2. **检查工具安装状态**
   - 自定义工具跳过检查
   - 内置工具检查 relative_detect_dir 是否存在
   - 未安装返回 `TOOL_NOT_INSTALLED|{key}|{path}` 错误

3. **解析目标路径**
   - 获取工具的 relative_skills_dir
   - 拼接 HOME 目录得到绝对路径
   - 目标：tool_skills_dir/{skill_name}

4. **检查目标是否存在**
   - 如果存在且 overwrite=false，返回 `TARGET_EXISTS|{path}` 错误
   - 如果存在且 overwrite=true，删除后继续

5. **选择同步模式并执行**

   **Cursor 工具：**
   - 强制使用 copy 模式
   - 调用 sync_dir_copy_with_overwrite

   **其他工具（混合模式）：**
   - 首先尝试 symlink
     - Unix: `std::os::unix::fs::symlink`
     - Windows: `std::os::windows::fs::symlink_dir`
   - 如果失败（权限不足），Windows 上尝试 junction
     - 使用 junction crate
   - 如果仍失败，回退到 copy
     - 递归复制目录内容
     - 跳过 .git 目录

6. **记录同步结果**
   - 更新 skill 表的 sync_details
   - 添加工具到 enabled_tools 数组
   - 设置同步时间戳

7. **返回 SyncResult**
   - mode_used: 实际使用的同步模式
   - target_path: 目标路径

**和配置路径联动的特殊点：**
- `runtime_location::get_tool_skills_path_async` 会根据模块当前配置目录，动态解析 Claude/Codex/OpenCode/OpenClaw 的 skills 目标目录。
- 如果模块配置目录是 WSL UNC 路径，目标目录也会变成对应的 WSL UNC 路径。
- 各模块保存配置时会先记录 `previous_skills_path`，再在保存后调用 `resync_all_skills_if_tool_path_changed`。这一步的目的不是改中央仓库，而是把所有已管理 skill 重新同步到新的工具目标目录，并清理旧目标记录。

### 4.8 取消同步流程

从工具中移除技能。

**入口函数：** `skills_unsync_from_tool`

**处理流程：**

1. **检查工具安装状态**
   - 如果工具未安装，直接返回成功

2. **获取同步目标信息**
   - 从 sync_details 中读取该工具的记录
   - 如果不存在，直接返回成功

3. **删除目标路径**
   - 根据 mode 选择删除方式：
     - symlink: 使用 remove_file（Unix）或 remove_dir（Windows junction）
     - junction: 使用 remove_dir
     - copy: 使用 remove_dir_all
   - 处理路径不存在的情况（静默成功）

4. **更新数据库**
    - 从 sync_details 中移除该工具记录
    - 从 enabled_tools 数组中移除该工具

### 4.9 Skill 管理禁用/恢复流程

`management_enabled` 是 AI Toolbox 内部管理状态，不能复用 `status`。`status` 仍表示 skill 内容或同步健康状态（如 ok/error）。

**禁用入口：** `skills_set_management_enabled(skillId, false)` 或 Inventory apply 中 `enabled=false`。

**处理流程：**

1. 先读取当前 `enabled_tools` / `sync_details`。
2. 将当前工具绑定保存到 `disabled_previous_tools`；如果当前没有绑定但已有历史记录，则保留历史记录。
3. 删除每个当前同步目标路径。
4. 将 `management_enabled=false`，并清空 `enabled_tools` / `sync_details`。
5. 不改变 `group_id` / `user_group`，不删除中央仓库内容，不改写 `SKILL.md`。

**恢复入口：** `skills_set_management_enabled(skillId, true)` 只恢复管理启用状态并返回历史工具列表；前端必须让用户确认要恢复哪些历史工具，再复用已有 `skills_sync_to_tool` 链路恢复同步。不要在 Inventory 导入阶段新增工具可用性阻断逻辑；未安装工具仍由现有 `TOOL_NOT_INSTALLED|...` 语义处理。

### 4.10 Skill Inventory JSON 导入导出流程

Inventory JSON 是完整管理清单，用于重排 AI Toolbox 元数据，不是 skill 内容备份，也不是局部 patch。

**导出入口：** `skills_export_inventory_file`（主 UI 使用）/ `skills_export_inventory`（兼容旧的字符串返回）

- 导出全部 skill，包括 `management_enabled=false` 或当前 UI 筛选隐藏的 skill。
- 主 UI 不再把完整 JSON 塞进 textarea 或剪贴板，而是直接写入用户目录下的 `~/skill-group-{timestamp}.json`，避免大清单在界面里卡顿或难以复制。
- groups 只包含 `name/note/order`，不包含内部 `group_id`。
- skills 包含 `id/name/group/user_note/order/enabled/enabled_tools/previous_enabled_tools/source_type/source_ref/central_path/content_hash`。
- 不导出 `description`。description 仅用于 UI 展示，来自中央仓库 `SKILL.md` frontmatter 缓存解析。

**预览/应用入口：** `skills_preview_inventory_import_file` / `skills_apply_inventory_import_file`（主 UI 使用）/ 字符串版本命令（兼容旧调用）

- `schema_version` 当前为 1；不兼容版本应返回明确错误。
- group name 必须非空且唯一；重复名称会导致 skill 的 group 引用不可唯一解析，必须阻断 apply。
- skill 匹配策略：`id` 优先；否则用 `name + source_ref/central_path` 兜底；`content_hash` 只用于内容变化提示，不能作为唯一匹配键。
- apply 以 JSON groups 重建 `skill_group` registry，并在后端映射到新的内部 `group_id`。
- 本地存在但 JSON 未匹配到的 skill 不物理删除；完整清单语义下默认禁用，并在预览/确认中展示 `default_disable_count`。
- apply 是 desired-state 语义：matched 且 `enabled=true` 的 skill 必须按 JSON 的 `enabled_tools` 对当前工具绑定做增删对齐；matched 且 `enabled=false` 的 skill 必须取消当前同步并记录历史工具；本地存在但 JSON 未匹配到的 skill 默认禁用时还必须清空 `group_id/user_group`，避免完整重建 group registry 后留下孤儿分组引用。
- 整理 prompt 应引导 agent 使用文件读写工具读取导出的 JSON 文件并另存/写回可导入文件，不要求用户手动粘贴大段 JSON。

### 4.11 SKILL.md description 展示缓存

`ManagedSkillDto.description` 来自中央仓库 `SKILL.md` frontmatter 的 `description` 字段，不持久化到 `skill` 表，也不进入 Inventory JSON。缓存 key 应至少包含 `central_path` 与 `content_hash`；如果后续新增更细粒度失效策略，可再引入 `SKILL.md` mtime，但不要把 description 写成 DB 事实源。

### 4.12 技能删除流程

完全删除一个管理的技能。

**入口函数：** `skills_delete_managed`

**处理流程：**

1. **获取所有同步目标**
   - 读取 sync_details 中所有工具的 target_path

2. **删除所有同步目标**
   - 遍历每个目标路径
   - 调用 remove_path 删除
   - 记录删除失败的路径（不中断流程）

3. **删除中央仓库内容**
   - 获取 central_path
   - 删除整个目录

4. **删除数据库记录**
   - 从 skill 表删除记录

5. **返回结果**
   - 如果有删除失败的目标，返回警告信息
   - 列出无法清理的路径

## 五、功能模块详解

### 5.1 工具适配器 (tool_adapters.rs)

内置工具清单由共享模块 `crate::coding::tools::BUILTIN_TOOLS` 统一提供，Skills 不允许再维护一份平行的内置工具枚举或 key 映射。

当前内置支持的 Skills 工具如下：

| 工具 Key | 显示名称 | Skills 目录 | 检测目录 |
|----------|----------|-------------|----------|
| cursor | Cursor | ~/.cursor/skills | ~/.cursor |
| claude_code | Claude Code | ~/.claude/skills | ~/.claude |
| codex | Codex | ~/.codex/skills | ~/.codex |
| opencode | OpenCode | ~/.config/opencode/skills | ~/.config/opencode |
| antigravity | Antigravity | ~/.gemini/antigravity/skills | ~/.gemini/antigravity |
| amp | Amp | ~/.config/agents/skills | ~/.config/agents |
| kilo_code | Kilo Code | ~/.kilocode/skills | ~/.kilocode |
| roo_code | Roo Code | ~/.roo/skills | ~/.roo |
| goose | Goose | ~/.config/goose/skills | ~/.config/goose |
| gemini_cli | Gemini CLI | ~/.gemini/skills | ~/.gemini |
| github_copilot | GitHub Copilot | ~/.copilot/skills | ~/.copilot |
| openclaw | OpenClaw | ~/.openclaw/skills | ~/.openclaw |
| qoder_work | QoderWork | ~/.qoderwork/skills | ~/.qoderwork |
| qoder | Qoder | ~/.qoder/skills | %APPDATA%/Qoder |
| droid | Droid | ~/.factory/skills | ~/.factory |
| windsurf | Windsurf | ~/.codeium/windsurf/skills | ~/.codeium/windsurf |

工具检测逻辑：检测目录存在即认为工具已安装。

### 5.2 同步引擎 (sync_engine.rs)

同步模式选择逻辑：

1. 如果是 Cursor → 强制使用 copy（Cursor 不支持符号链接）
2. 尝试 symlink（Unix 或 Windows 管理员权限）
3. Windows 回退到 junction（目录接合点，无需管理员）
4. 最终回退到 copy（完整复制目录）

特殊处理：
- 复制时跳过 .git 目录
- 顶层符号链接会被解析后复制实际内容
- Windows 上 Git 存储的文本符号链接也会被正确处理

### 5.3 托盘支持 (tray_support.rs)

菜单结构：
```
──── Skills ────
  my-skill-1  ▸  [✓ Claude Code, ✓ Codex, ○ OpenCode]
  my-skill-2  ▸  [✓ Claude Code, ○ Codex]
```

事件处理：
- 事件 ID 格式：`skill_tool_{skill_id}_{tool_key}`
- 点击后切换同步状态（已同步 → 取消同步，未同步 → 同步）
- 同步使用 overwrite=true 直接覆盖

托盘双向同步：
- 托盘 → 前端：Rust 调用 `app.emit("skills-changed", "tray")`，前端监听并刷新
- 前端 → 托盘：前端操作完成后调用 `refreshTrayMenu()` 刷新托盘

## 六、错误处理

### 特殊错误前缀

前端通过解析错误消息前缀来触发特定 UI 流程：

| 前缀 | 含义 | 前端处理 |
|------|------|----------|
| SKILL_EXISTS\|name | 技能已存在于中央仓库 | 弹出覆盖确认 |
| TARGET_EXISTS\|path | 技能已存在于工具目录 | 弹出覆盖确认 |
| TOOL_NOT_INSTALLED\|key\|path | 工具未安装 | 显示安装提示 |
| MULTI_SKILLS\| | 仓库包含多个技能 | 弹出选择器 |

### Git 错误解析

gitErrorParser.ts 提取具体的 Git 错误类型：
- 网络错误（代理问题、DNS、超时）
- 认证错误
- 仓库不存在
- 权限拒绝

## 七、SKILL.md 格式

技能通过 SKILL.md 文件声明元数据：

```yaml
---
name: "技能名称"
description: "可选的描述"
---

[Markdown 内容...]
```

如果没有 SKILL.md 或没有 name 字段，则使用目录名作为技能名称。

## 八、重要注意事项

### 8.1 路径处理

- 配置中使用正斜杠（跨平台兼容）
- 运行时转换为系统原生分隔符
- Windows 路径比较不区分大小写

### 8.2 Cursor 限制

- Cursor 不支持符号链接和接合点
- 始终使用复制模式
- 更新技能后需要重新同步（不会自动更新）

### 8.3 中央仓库

- 默认路径：应用数据目录/skills（如 `%APPDATA%/com.ai-toolbox/skills`）
- 可在设置中自定义
- 存储技能的原始内容
- 工具目录通过链接或复制引用
- WSL/SSH 自动同步时，源目录仍然是中央仓库；不要把工具运行时目录误判成同步源
- 用户备注、手动分组、管理启用状态都属于 AI Toolbox 管理元数据，不属于中央仓库文件内容。分组事实源分拆为 `skill_group` + `skill.group_id`，`user_group` 仅为兼容名称；更新、同步、WSL/SSH 后续链路不能因为这些元数据变化而改写 `SKILL.md` 或发起技能内容同步。

### 8.4 代理支持

- 使用应用全局代理设置
- Git 操作前自动设置代理
- 支持 HTTP/HTTPS 代理

### 8.5 新工具检测

- 每次获取工具状态时比较 installed_tools
- 新发现的工具会触发 NewToolsModal
- 提示用户同步现有技能到新工具

### 8.6 同步模式的影响

| 模式 | 更新行为 | 磁盘占用 | 权限要求 |
|------|----------|----------|----------|
| symlink | 自动同步 | 无额外占用 | Unix: 无 / Windows: 管理员 |
| junction | 自动同步 | 无额外占用 | 无 |
| copy | 需手动重新同步 | 完整副本 | 无 |

### 8.7 WSL 与 SSH 的 Skills 链路分工

- 本地 `skills_sync_to_tool`
  - 负责把中央仓库同步到每个工具当前运行时目录。
  - 目标路径由 `runtime_location::get_tool_skills_path_async` 动态解析，可能是本机路径，也可能是 `\\\\wsl.localhost\\...`。

- WSL 自动同步
  - 监听 `skills-changed`。
  - 负责把中央仓库内容同步到 WSL 侧统一中央仓库 `~/.ai-toolbox/skills`，再给各工具目录建立符号链接。
  - 对于已经 `is_wsl_direct` 的内置工具，应跳过该工具目录的额外链接维护，避免和该工具已经直接运行在 WSL 内的目录重复写入。
  - Claude Code 本机自定义根目录只改变本机运行时 skills 目标，例如 `<custom-root>/skills`；普通 WSL 侧工具目录仍按 Claude 默认 `~/.claude/skills` 维护。只有 Claude 当前运行时本身是 WSL Direct 自定义根目录时，目标才跟随该 Linux 根目录。

- SSH 自动同步
  - 也监听 `skills-changed`，但不走 file mappings。
  - 负责把中央仓库内容同步到 SSH 远端的统一中央仓库 `~/.ai-toolbox/skills`，再给远端工具目录建立符号链接。
  - 排查 SSH 问题时，不要只看 SSH 同步设置页展示的 file mappings，因为 skills 根本不是走那条链路。

- 最常见的误判
  - 误把工具运行时 skills 目录当成源目录。
  - 误以为 SSH/WSL skills 同步复用了 file mappings。
  - 只看 UI 展示路径就判断同步真实读写路径，没有继续核对 `central_repo_path`、运行时目标目录和远端目标目录三者是否一致。

### 8.8 4 个内置工具在 WSL 自定义配置下对 Skills 的影响

- 这里的 4 个工具是：OpenCode、Claude Code、Codex、OpenClaw。
- `runtime_location` 会先解析每个工具当前的**生效配置路径**，然后再判断它是不是 `\\\\wsl.localhost\\...`。
- 一旦某个工具被判定为 `is_wsl_direct=true`：
  - 该工具的运行时 skills 目录也会跟着切到对应的 WSL UNC 路径。
  - `skills_sync_to_tool` 仍然从中央仓库取源，只是同步目标改成了这份 WSL UNC 路径。
  - WSL 自动同步在给 WSL 统一中央仓库建工具链接时，应跳过这个工具，避免对已经直接运行在 WSL 内的工具目录重复维护。

- 4 个工具对“生效配置路径”的解析方式不同：
  - OpenCode、OpenClaw：以配置文件路径为核心，再从该文件位置派生 skills 目录。
  - Claude Code、Codex：以根目录为核心，再从根目录派生配置文件、prompt、skills 等路径。

- 所以排查 “为什么 Skills 目标目录变成 WSL” 时，不要只看 Skills 模块本身，要回头确认对应 tab 的配置路径是否已经被 `runtime_location` 判定为 WSL direct。

## 九、前后端通信

### Tauri 命令命名规范

命令名：`skills_` 前缀 + 下划线分隔
参数名：camelCase（前端兼容）

### 事件系统

- 事件名：`skills-changed`
- 负载：字符串标识来源（如 "tray"）
- 用途：托盘操作通知前端刷新

### API 列表

| 命令 | 说明 |
|------|------|
| skills_get_tool_status | 获取工具安装状态 |
| skills_get_central_repo_path | 获取中央仓库路径 |
| skills_set_central_repo_path | 设置中央仓库路径 |
| skills_get_managed_skills | 获取所有管理的技能 |
| skills_install_local | 从本地安装技能 |
| skills_install_git | 从 Git 安装技能 |
| skills_list_git_skills | 列出 Git 仓库中的技能 |
| skills_install_git_selection | 安装 Git 仓库中的指定技能 |
| skills_sync_to_tool | 同步技能到工具 |
| skills_unsync_from_tool | 取消同步 |
| skills_update_managed | 更新技能（从源重新拉取） |
| skills_delete_managed | 删除技能 |
| skills_get_onboarding_plan | 获取技能发现计划 |
| skills_import_existing | 导入现有技能 |
| skills_get_groups | 获取 first-class skill 分组 |
| skills_save_group | 新增或更新 skill 分组，要求名称唯一 |
| skills_delete_group | 删除分组，并将组内 skill 移到未分组；不改变管理启用状态 |
| skills_batch_update_group | 批量移动 skill 到指定 group_id |
| skills_set_management_enabled | 设置管理启用/禁用状态；禁用时记录历史工具并取消同步 |
| skills_export_inventory | 导出完整 Skill Inventory JSON |
| skills_export_inventory_file | 导出完整 Skill Inventory JSON 到 `~/skill-group-{timestamp}.json` |
| skills_preview_inventory_import | 预览 Inventory JSON 导入影响 |
| skills_preview_inventory_import_file | 从 JSON 文件预览 Inventory 导入影响 |
| skills_apply_inventory_import | 应用 Inventory JSON 完整清单导入 |
| skills_apply_inventory_import_file | 从 JSON 文件应用 Inventory 完整清单导入 |
| skills_get_preferred_tools | 获取首选工具 |
| skills_set_preferred_tools | 设置首选工具 |
| skills_get_show_in_tray | 获取托盘显示设置 |
| skills_set_show_in_tray | 设置托盘显示 |
| skills_reorder | 重新排序技能 |
| skills_get_repos | 获取仓库列表 |
| skills_add_repo | 添加仓库 |
| skills_remove_repo | 删除仓库 |
| skills_get_custom_tools | 获取自定义工具 |
| skills_add_custom_tool | 添加自定义工具 |
| skills_remove_custom_tool | 删除自定义工具 |
| skills_get_git_cache_cleanup_days | 获取缓存清理天数 |
| skills_set_git_cache_cleanup_days | 设置缓存清理天数 |
| skills_get_git_cache_ttl_secs | 获取缓存 TTL |
| skills_clear_git_cache | 清空 Git 缓存 |
| skills_get_git_cache_path | 获取缓存路径 |

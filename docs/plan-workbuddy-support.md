# WorkBuddy 系列集成方案（桌面版 + CodeBuddy Code CLI）

> 关联 issue：[#450 能否添加 workbuddy 智能体？](https://github.com/coulsontl/ai-toolbox/issues/450)
> 状态：实施中
> 最近更新：2026-10-30

## 1. 产品概述

腾讯的 WorkBuddy 是**两条共用同一引擎**的产品线：

| 产品 | 形态 | 配置根 | 本机状态 |
|---|---|---|---|
| **WorkBuddy 桌面版** | Electron GUI + 内嵌 `codebuddy` sidecar | `~/.workbuddy` | ✅ 已装 v5.8.3，有真实数据 |
| **CodeBuddy Code CLI** | 标准终端 CLI（`codebuddy` / `cbc`） | `~/.codebuddy` | ❌ 未安装 |

两者共用 `models.json` 自定义模型契约，但**其余配置语义不同**：

| 文件 | 桌面版 `~/.workbuddy` | CLI `~/.codebuddy` |
|---|---|---|
| `models.json` | 自定义模型（**同一 schema**） | 自定义模型（**同一 schema**） |
| `settings.json` | 桌面 sandbox 规则（*不是*模型配置） | 真正的用户设置（`model`/`permissions`/`env`…） |
| `mcp.json` | user scope MCP | user scope MCP（**`.mcp.json` 优先级更高**） |
| 全局提示词 | 身份文件 `user-<uid>-personal/{IDENTITY,USER,SOUL,BOOTSTRAP}.md` | `CODEBUDDY.md` |
| 会话 | `projects/<cwd编码>/<sessionId>.jsonl` + `workbuddy.db` | `projects/<cwd编码>/<sessionId>.jsonl` |

决策：**两个独立 tab**（`workbuddy` / `codebuddy`），共享 `coding/workbuddy_family/` 里的 `models.json` 与转录解析逻辑。

## 2. 范围与边界

### 2.1 本次做

| 能力 | 桌面版 | CLI |
|---|---|---|
| 自定义模型渠道管理 | ✅ | ✅ |
| Skills / MCP 联动 | ✅ | ✅ |
| 会话管理 | ✅ | ✅ |
| Gateway 接管（改写 `models.json` 的 `url`） | ✅ | ✅ |
| 身份文件 / `CODEBUDDY.md` 提示词管理 | ✅ 身份文件 | ✅ `CODEBUDDY.md` |

### 2.2 不做

- **国际版**（`~/.workbuddy-ai` / `~/.codebuddy-ai`）：本机无数据，无法实测。
- **官方账号 / OAuth**：两条产线的登录都由本体管理（桌面版走浏览器，CLI 走 `codebuddy` 交互登录），没有可复用的凭据快照 API。
- **插件市场**：`~/.workbuddy/plugins/` 由腾讯云端下发，写入会被覆盖。

## 3. 已实测的硬事实（`models.json` 契约）

从 WorkBuddy v5.8.3 的 `app.asar` 内部实现读出的校验器（`isValidLocalCustomModel` / `getLocalCustomModelRequiredFieldIssues`）：

```jsonc
{
  "models": [{
    "id": "必填，非空唯一",                 // ← 必填
    "url": "完整到 /chat/completions",      // ← 必填
    "apiKey": "sk-…",                      // ← 必填，loopback 地址可省
    "name": "显示名",
    "vendor": "供应商名",
    "maxInputTokens": 200000,
    "maxOutputTokens": 8192,
    "temperature": 1,
    "supportsToolCall": true,
    "supportsImages": true,
    "supportsReasoning": true,
    "onlyReasoning": false,
    "useCustomProtocol": false             // true = 跳过 URL 规范化
  }],
  "availableModels": ["id1", "id2"]        // ⚠️ 见下
}
```

**四条必须遵守的语义：**

1. **`availableModels` 为空/缺失 = 显示全部**（含 59 个腾讯内置模型）；**非空 = 只显示列出的**。
   → AI Toolbox 写入时**绝不**把它收窄成只含自定义模型，否则用户的内置模型会整体消失。
2. **URL 自动规范化**：app 把 `<base>/v1` 补成 `<base>/v1/chat/completions`；`useCustomProtocol: true` 跳过。
   → 网关接管时写完整路径即可，不需要依赖这个开关。
3. **热重载 + 原子写**：1 秒防抖 file watcher；写入走临时文件 + rename，且 UI 保存会保留未知字段。
   → AI Toolbox 必须**先读后合并再原子写**，不能整文件重建。
4. **顶层两种形态都接受**：`{models,availableModels}` 或**裸数组**。读时必须两种都认，写时保留原形态。

## 4. 后端模块设计

### 4.1 目录

```
tauri/src/coding/
├── workbuddy_family/         # 共享：models.json 契约、身份文件、转录解析
│   ├── mod.rs
│   ├── models_json.rs        # 读写/合并/原子写/URL 规范化
│   ├── identity.rs           # user-<uid>-personal 定位与身份文件读写
│   └── transcripts.rs        # projects/** 扫描（复用 session_import 的解析）
├── workbuddy/                # 桌面版 tab
│   ├── mod.rs / constants.rs / types.rs / adapter.rs / commands.rs
│   ├── paths.rs              # ~/.workbuddy 定位（含 WORKBUDDY_CONFIG_DIR）
│   └── tray_support.rs
└── codebuddy/                # CLI tab
    ├── mod.rs / constants.rs / types.rs / adapter.rs / commands.rs
    ├── paths.rs              # ~/.codebuddy 定位（含 CODEBUDDY_CONFIG_DIR）
    └── tray_support.rs
```

### 4.2 数据库表

沿用 `id + data(JSONB) + created_at + updated_at`：

| 表 | 用途 |
|---|---|
| `workbuddy_provider` | 渠道记录（一个渠道 = 一组同 `url`+`apiKey` 的模型） |
| `workbuddy_prompt_config` | 身份文件模板（`kind` 区分 IDENTITY/USER/SOUL/BOOTSTRAP） |
| `codebuddy_provider` | 同上 |
| `codebuddy_prompt_config` | `CODEBUDDY.md` 模板 |

> **`models.json` 是运行时事实源，数据库只是「渠道分组 + 备注 + 排序」的投影**，与 Pi 一致：读页面先读文件，数据库只补 UI 元数据。这样用户在 app 里直接加模型不会被 AI Toolbox 覆盖。

### 4.3 渠道聚合规则

`models.json` 是**扁平的模型列表**，没有 provider 概念。页面上的「渠道卡片」由聚合得到：

```
分组键 = (url 去掉 /chat/completions, apiKey)
渠道名 = vendor（同组内取第一个非空）
成员   = 该组下所有模型条目
```

保存时反向展开：一张卡片 → N 条模型条目，逐条 upsert 回 `models`，**不属于本渠道的条目原样保留**。

### 4.4 身份文件定位

桌面版身份文件是**按账号**存放的：

```
~/.workbuddy/storage/skeleton/account-snapshot.json   → primary.uid
~/.workbuddy/user-<uid>-personal/IDENTITY.md
                                /USER.md
                                /SOUL.md
                                /BOOTSTRAP.md
                                /workspace-state.json
```

`uid` 缺失时（未登录）该能力整体不可用，页面显示原因而不是报错。

> 目录名规则已从 app 代码确认：`user-${uid}-${enterpriseId || 'personal'}`。

## 5. Gateway 接管

WorkBuddy 的内置模型走腾讯云端、配置文件改不到，所以**只有自定义模型可被接管**：

- **engage**：遍历 `models.json` 的 `models[]`，把 `url` 改写为 `http://127.0.0.1:<port>/workbuddy/v1/chat/completions`，`apiKey` 改写为网关 key。
- **manifest**：逐条记录原始 `id`/`url`/`apiKey`，恢复时字段级还原。
- **restore**：按 manifest 还原；条目在接管期间被用户删掉的，跳过而不是重建。
- **入站路由**：`/workbuddy/v1/chat/completions`、`/codebuddy/v1/chat/completions`（两条独立前缀，避免与现有 `/openai` 冲突）。

新增 `GatewayCliKey::Workbuddy` / `GatewayCliKey::Codebuddy`。

> ⚠️ **与 usage tool 的撞名风险**：`GatewayUsageTool` 里已有 `Workbuddy` / `Codebuddy`（字符串 `"workbuddy"` / `"codebuddy"`）。`cli_key_from_app_type` 用 `GatewayUsageTool::as_str()` 反查，两边同名会让**采集来源与接管来源不可区分**。处理方式：`GatewayCliKey` 用不同的 `app_type` 字符串（`"workbuddy_desktop"` / `"codebuddy_cli"`），或在 `data_source` 里区分。落地时以「请求列表能正确分组」为准，并在 `usage_stats` 补回归。

## 6. 会话管理

`projects/<cwd编码>/<sessionId>.jsonl` 是 Claude Code 形状，条目类型：

```
session-meta / message / reasoning / function_call / function_call_result
/ credit-usage / file-history-snapshot / ai-title
```

复用 `proxy_gateway/session_import/workbuddy_family.rs` 的解析器读取用量；`session_manager` 只需要：

- `scan_sessions(projects_root)` —— 目录名反解 cwd，`ai-title` 取标题，`session-meta` 取 id/时间
- `load_messages(path)` —— `message` / `reasoning` / `function_call` / `function_call_result` 映射到现有 `MessageBlock`

桌面版另有 `workbuddy.db` 的 `sessions` 表（`title`/`model`/`created_at`），可作为标题与模型的补充来源；CLI 没有这个库。

## 7. Skills / MCP 联动

- `builtin.rs` 已有 `workbuddy` / `workbuddy_ai`；本次**新增 `codebuddy`**。
- **修复既有缺陷**：CodeBuddy CLI 的 user-scope MCP 优先级是 `~/.codebuddy/.mcp.json` > `~/.codebuddy/mcp.json`。`codebuddy` 必须指向 `.mcp.json`；`workbuddy`（桌面版）保持 `mcp.json`（app 自己写的就是这个）。
- Skills 目录：桌面 `~/.workbuddy/skills`，CLI `~/.codebuddy/skills`。

## 8. 前端

两个页面都按 §0.1.1 判定为**多渠道路由** → `OpenCodeStyleCard`（列表布局）/ `ProviderCompactCard`（卡片布局）：

- 列表卡片陈述「渠道名 / ID / endpoint」，模型列表折叠在卡片内
- 卡片视图的格子只陈述「渠道名 / endpoint / 模型数」，模型管理进侧栏（常开）；两种布局是**密度差异，不是能力差异**
- 默认模型**不做**（没有可靠文件指针，`preferences.json` 的 `defaultModelId` 在桌面版未被写入）
- 模型行支持增删改，不提供「设为默认」
- 渠道级「编辑」（2026-10-31 改：**做了**）＝把新的端点 / key / vendor 写回该渠道的每一条模型；文件里没有渠道记录，这是唯一能移动渠道的方式
- **不把列表卡片塞进网格格**：它的工具栏有五个文字按钮，350px 的格放不下，「模型列表」会竖排成单字一行（2026-10-31 用户报障，两个页面的浏览器夹具已守）

页面区块（按 §4.0.2 并集核对后取）：

```
页头（配置根路径 + 布局切换）
├── 供应商列表（卡片 / 卡片视图）
├── 全局提示词（桌面：4 个身份文件；CLI：CODEBUDDY.md）
├── 其他配置（CLI 才有：settings.json 只读预览）
└── 会话管理
```

## 9. Allowlist 注册清单（最高风险）

两个 tab key：`workbuddy`、`codebuddy`。逐项：

- [ ] `web/constants/modules.tsx` `MODULES` subTabs
- [ ] `web/features/settings/pages/GeneralSettingsPage.tsx` `CODING_TABS`
- [ ] `web/services/settingsApi.ts` `SIDEBAR_PAGE_KEYS` + `createDefaultSidebarHiddenByPage` + `defaultSettings.visible_tabs`
- [ ] `tauri/src/settings/types.rs` `AppSettings::default()` + `default_sidebar_hidden_by_page()`
- [ ] `tauri/src/settings/adapter.rs` `CURRENT_DEFAULT_VISIBLE_TABS` + `PRE_*` 基线 + 迁移测试
- [ ] `tauri/src/coding/runtime_location.rs` `MODULE_KEYS` + `get_tool_skills_path_async` / `get_tool_mcp_config_path_async`
- [ ] `tauri/src/coding/reapply_applied_runtime.rs` `ALL_WSL_FILE_MODULES`
- [ ] `tauri/src/settings/backup/utils.rs` `OPTIONAL_BACKUP_CLI_TOOLS` + `tool_prefixes` + 打包段
- [ ] `tauri/src/settings/backup/restore.rs` 解压分支
- [ ] `tauri/src/coding/ssh/skills_sync.rs` `SKILLS_TARGETS_FROM_RUNTIME_LOCATION`
- [ ] `tauri/src/tray.rs` section builders
- [ ] `web/features/settings/hooks/useWSLSync.ts` / `useSSHSync.ts` `TAB_TO_MODULE`
- [ ] `web/features/settings/components/WSLSyncModal.tsx` / `SSHSyncModal.tsx` `MODULE_TO_TAB`
- [ ] `web/features/settings/components/FileMappingModal.tsx` / `SSHFileMappingModal.tsx`
- [ ] `web/features/settings/utils/syncMessageTranslator.ts`
- [ ] `web/components/layout/MainLayout/index.tsx` 图标分支
- [ ] `web/features/coding/shared/toolIcon/ToolIcon.tsx`（已有 `workbuddy`，补 `codebuddy`）
- [ ] Gateway：`GatewayCliKey`、`routes.rs`、`connectivity_test.rs`、`usage_stats.rs` 的 `load_provider_names`
- [ ] 前端 Gateway 列表：`proxyGatewayApi.ts`、`GatewaySettingsPanel.tsx`、`ModelPricingModal.tsx`、`providerProfiles.ts`

## 10. 实施路线图

状态截至 2026-10-30。

| 阶段 | 内容 | 验收 | 状态 |
|---|---|---|---|
| P0 | 方案文档 | 本文 | ✅ |
| P1 | `workbuddy_family` 共享层 + 单测 | `cargo test workbuddy_family` | ✅ 20 项 |
| P2 | `workbuddy/` 后端 + 数据库表 + 命令 | `cargo test workbuddy` | ✅ |
| P3 | `codebuddy/` 后端 | `cargo test codebuddy` | ✅ 11 项 |
| P4 | Allowlist 全量注册（两个 key） | 全仓 `rg` 逐个确认 | ✅ |
| P5 | 前端两个页面 + 卡片映射 | `pnpm tsc --noEmit` + 浏览器断言 | ✅ 编译、布局守卫、页面节奏（12 项浏览器断言）通过；**未做真机视觉核对**（见 §10.2） |
| P0–P6 收尾 | 复查 §4.0.2 A–H / §12.1 机械清单 | 逐条跑命令 | ✅ 期间修掉 6 处静默缺陷（外壳/预览原文/i18n 前缀/备份对账/身份路径/改名丢键），已回写 SOP #158–#163 |
| P6 | 身份文件 / `CODEBUDDY.md` 提示词 | 往返测试 | ✅ |
| P7 | 会话管理 | `cargo test --lib session_manager` | ⬜ 未做 |
| P8 | Gateway 接管 | `cargo test proxy_gateway` | ⬜ 未做（usage 采集早已存在且独立） |
| P9 | Skills / MCP + 托盘 + 备份 + WSL/SSH | `pnpm test` + `cargo test` | 🟡 Skills/MCP 路径、备份、WSL/SSH **已做**；**托盘未做** |

### 10.1 P7 / P8 / P9-托盘 为何暂缓

- **P7 会话管理**：`proxy_gateway/session_import` 已有 WorkBuddy / CodeBuddy 的 JSONL 解析器，但那属于网关用量采集，不是本应用的会话浏览页。会话浏览需要新的扫描/详情/重命名/导入导出链路，量级等同一次 P5，单独一轮做。
- **P8 Gateway 接管**：接管要求改写 `models.json` 里每条模型的 `url`/`apiKey` 并保留原始值以便恢复。之所以单独一轮：它必须同时解决 `app_type` 撞名（`GatewayUsageTool::Workbuddy` 序列化后就是 `"workbuddy"`，与 tab key 相同，`usage_stats::cli_key_from_app_type` 会分不清用量行与接管行）。
- **P9 托盘**：托盘的语义是「在多个已应用 provider/模型之间切换」。这两个产品既没有单一 active provider，也没有可靠的默认自定义模型指针（见 §8），托盘上没有可切换的对象，因此不做——这不是遗漏，是形态不匹配。若将来 P8 落地、接管了多条渠道，再按当时的实际可切换项补。

### 10.1.1 §4.0.2 H「能力核对」逐项结论

SOP §4.0.2 的 H 节要求「参照 CLI 有的能力本 CLI 也要有，找不到的写明有意不做 + 理由」。本轮结论：

| 能力 | 结论 | 理由 |
|---|---|---|
| 选择预设模型 | **不做** | `models.json` 的模型条目是「endpoint + key + 开关」的扁平记录，没有 Codex / ZCode 那种带推理档位、限量、模态的模型目录；预设在这里没有可填的地方 |
| 「获取模型」（从上游拉取） | **做了（2026-10-31 改）** | 走仓库共享的 `fetch_provider_models`。原判「拉回来的只有 id 无处可填」不成立：新 id 就是一条继承渠道 endpoint/key 的模型条目，正是本页模型的最小形态。**含 `useCustomProtocol` 的渠道禁用**——这类条目 URL 不一定是 `chat/completions` 形式，自动拼 `{base}/models` 会拼错 |
| 模型级连通性测试 | **做了（2026-10-31 改）** | 走共享 `test_provider_model_connectivity`（`@ai-sdk/openai-compatible` → `{base}/chat/completions`，与 `models.json` 的 URL 规则一致）。原判「本模块没有网关侧探针链路」不成立：不需要网关，共享命令直连上游 |
| 供应商级连通性测试 / 批量测试 | **未做** | 批量测试（`onBatchTest`）仍不接：它按 provider 各测一个模型，而这里同一渠道内的模型共用 endpoint/key，逐渠道批量测的收益低于界面复杂度 |
| 「默认渠道」删除置灰并排除出批量选择 | **不适用** | 判据是「默认指针是否写在 provider 记录之外」。这里**根本没有默认指针**（见 §8），所以没有需要保护的默认项；批量删除覆盖全部渠道是正确的 |
| 模型行「设为主模型」 | **不做** | `models.json` 无默认模型指针。唯一候选 `availableModels` 是**白名单**，写它会隐藏产品自带的全部内置模型，与「选默认」正好相反。`ModelDisplayData.isPrimary` 因此始终省略 |
| 「复制模型」 | **做了（2026-10-31）** | 预填 `<id>_copy` 并清空 `previousId`，保存走新增分支。不清空 `previousId` 会退化成把源条目改名到自身 |
| 模型级批量删除 | **做了（2026-10-31）** | 一次只允许一个渠道处于选择模式（`modelSelectionChannelId` 是单值），确认弹窗在页面层 |
| 模型拖拽排序 | **不做** | `models.json` 是扁平数组，产品不读回模型顺序 |
| 渠道级「编辑」 | **做了（2026-10-31）** | 与「添加供应商」同一个弹窗的编辑态：改端点 / key / vendor 会逐条写回该渠道的全部模型，每条自己的 id / name / 能力开关原样带上（`upsert_model_renaming` 按 id 合并）。编辑态隐藏「首条模型 ID」 |
| 「导入我使用过的供应商」/ 收藏 provider | **不做** | favorite provider 的语义是「历史库 + 诊断缓存」，其 key/payload 约定要求该 CLI 有稳定的 provider 标识。渠道 id 是 `(url, apiKey)` 的哈希、从不落盘，导入后也无法与「当前配置里的同一条」对齐 |
| 全局提示词 | **做了** | CodeBuddy：`CODEBUDDY.md` 走标准预案；WorkBuddy：四个身份文件**复用同一套提示词卡片**（`GlobalPromptConfigCard` + `GlobalPromptConfigModal`），但集合固定——只能编辑内容，不能新增 / 删除 / 拖拽 / 应用（2026-10-31 用户指定）。结构性理由：产品只读这四个文件名；删除等于清空助手运行的文档（保存空内容才是可逆的清空）；文件本身就是记录，顺序无处可写 |
| 会话管理 | **暂缓** | 见 §10.1 |
| 预览配置 / 打开文件夹 / 刷新配置 | **做了** | 预览读文件原文：WorkBuddy 是 `models.json`；CodeBuddy 是 `models.json` + `settings.json` 两个 Tab（`read_workbuddy_models_file` / `read_codebuddy_models_file` / `read_codebuddy_settings_file`） |

### 10.2 视觉核对：做了什么、没做什么

SOP §12.3 要求「在界面上逐项指认」，并明确「若无法运行界面，必须在提交说明里写明『未做视觉核对』」。本轮的状态：

**已用真实浏览器（headless）量过的**

- `pnpm run test:page-rhythm`：12 项断言全部通过，其中 WorkBuddy / CodeBuddy 各 2 项，量的是 `CodingPageHeader` 底边 → 第一个区块顶边 = 16px，以及相邻区块之间 = 16px。这两个页面已加入 `PageRhythmFixture` 的 `PAGES`，此后改动会被持续守住。
- `test:provider-card-layout` / `test:form-modal-layout` / `test:css-module-classes`：17 / 18 个卡片与表单文件、CSS Modules 类名双向核对，均通过（0 个孤儿）。

**没有做的**

- **没有在真实 Tauri 窗口里跑过这两个页面**。`pnpm dev:debug` + `pnpm inspect` 那条链路本轮未执行。
- 完整 `pnpm test` 在本机无法稳定跑完：浏览器夹具会随机在某一步卡在 `Runtime.evaluate` 超时（`Browser exceptions: []`，即页面没抛错、是渲染进程不再响应）。**这不是本轮改动引入的**——在干净 `main`（`867506ea`）上跑同一条命令，同样在第 5 步 `test:provider-card-view` 以完全相同的形式失败；而把同一批夹具**逐条串行**执行时，各自单独运行都能通过。判据是「换一个夹具名 / 换一次运行就换一个失败点」，且失败点与本轮 diff 无因果关系。
- 因此以下**尚未在真机确认**：亮/暗/system 三套主题下的实际观感、真实数据（含真实渠道、真实模型名、长路径）下的折行与截断、空态与加载态、`SectionSidebarLayout` 侧栏入口的滚动定位、`RootDirectoryModal` 的实际回填行为、身份文件面板在未登录/已登录两态下的呈现。
- 合并前如需补齐，执行：`pnpm dev:debug` 起带 CDP 的窗口，另开终端 `pnpm inspect wait` / `snapshot` / `box` 逐个量；驱动方式见根 `AGENTS.md` 的「Live Page Inspection」（注意冷启动约 14s、`location.hash` 不会挂载路由、查询必须过滤 `offsetParent`）。

## 11. 风险

| 风险 | 缓解 |
|---|---|
| `availableModels` 收窄导致内置模型消失 | 写入时若原值为空/缺失，保持不写；只在用户显式管理时写 |
| 整文件重写丢掉 UI 写入的未知字段 | 读-合并-原子写；回归测试用「含未知字段的文件」往返 |
| `useCustomProtocol` / URL 规范化差异 | 网关接管写完整 `/chat/completions` 路径 |
| usage tool 与 takeover key 撞名 | 用不同 `app_type` 字符串，`data_source` 分组回归 |
| CodeBuddy CLI 本机未装，无法实测 | 全部走文件级 fixture；文档标注「未在真机验证」 |
| 桌面版身份文件按账号隔离 | uid 缺失时禁用能力并说明原因；`uid` / `enterpriseId` 按不可信输入校验（含分隔符/`..`/空即整体不可用），回归 `workbuddy_family::identity::tests::a_uid_that_could_escape_the_config_root_is_refused` |
| 改名丢掉产品自有键、并把模型挪到列表末尾 | 改走 `upsert_model_renaming`；两侧各一条回归（把实现改回「先删后插」会从 2 行变 3 行） |
| **schema 版本号与 main 撞车**（已发生并修复） | 本轮从 `b61f7b9d` 分叉时基线是 v30，把三张新表写成 `migrate_v31`；分叉后 main 的 `0ca25b58` 也占用了 v31（`native_tools_used`）。老库 `user_version=31` → 永不建表；新库则永久缺 main 的列。已把 v31 逐字还原为 main 原样，新表顺延到 `migrate_v32`，`TARGET_SCHEMA_VERSION = 32`。回归 `v32_migration_adds_workbuddy_tables_to_an_already_v31_database` 从「版本已是 31、表却缺失」起步。**合并前必须再查一次 main 有没有继续往前推版本号** |
| 自定义 `visible_tabs` 用户看不到新 tab | 不是缺陷：`adapter.rs` 故意只对「仍是某个历史默认值」的用户做 full-replace，自定义顺序的用户不被强插（否则每次重开都会把他们关掉的 tab 加回来）。向用户说明入口在「设置 → 通用 → 模块显示」，并先排除「跑的是旧构建」与「迁移没跑到」两层 |
| 备份打包 ↔ 恢复分支集合漂移 | 打包段按工具直写字面量路径（不用 `format!`），§12.1 的 grep 对账才能看见；输出必须为空 |

# 新增 CLI 工具接入 SOP

> **用途**：为 AI Toolbox 接入一个新的 CLI coding 工具（ZCode、Kimi 这类）时，按阶段执行的检查清单。
>
> **定位**：操作手册。设计规则与约束的原文以根 `AGENTS.md` 和各模块级 `AGENTS.md` 为准；本文只做整合、排序与索引，不重复规则原文。两者冲突时以规则文档为准。
>
> **依据来源**：
> - 根 `AGENTS.md`：「Tab / Page-Key Allowlist Rules」「Implementation Checklist for New Tray Integration」「Lightweight Mode」「4 Tabs WSL Direct Notes」
> - `docs/plan-kimi-code-cli.md`：§14 实施路线图 + §16 偏差与踩坑记录（目前最完整的单工具方案模板）
> - ZCode 集成实证（2026-10-06 合并）：10 个提交、63 个文件，含 3 轮「漏注册」修复
> - 记忆库 `projects/ai-toolbox/tab-allowlist-misses-new-tabs.md`（头号复发坑）
>
> **最近更新**：2026-10-23（**迁移收尾三件事：孤儿 CSS 反向守卫、账号区块折叠、弹窗说明不换行**（非迁移轮的回修）。用户连报三条：① OMP 页头与区块之间**完全没有间距**——那是我上一轮改名的中间态（`gap` 已删、`.ompSection` 尚未写回），已修；但它证明「间距这类结论只能量」。② OMP 的 OpenAI Codex **也想参与排序**——用户明确选择「保持现状（Codex 固定第一）」，因为纯 OAuth 渠道在 `models.yml` 里没有条目，给它建档会让它变成「yaml override」（名字加后缀、OMP 可能按自定义 provider 处理）。**拒绝一个请求也要记录理由**，否则下一个会话会再问一遍。③ OMP 账号列表**要能折叠**——`OfficialAccountsSection` 只在宿主传 `collapsed` / `onToggleCollapsed` 时才画三角，照 Codex 的 `renderOfficialAccounts({ fold })` 模式补上（列表卡折叠、详情侧栏常开），并在 `test:migrated-provider-cards` 加两条（有折叠按钮 + 点开真的展开），已验证「去掉 fold 会红」。④ **CSS Modules 守卫补上反向扫描，并把孤儿全部清零**：`scripts/verify-css-module-classes.mjs` 原先只查「引用→定义」，查不出迁移留下的孤儿类；现按 less 文件汇总全部导入者再判（否则假报 2655 条）。用户随后要求「迁移留下大量孤儿 CSS，全删了」，于是先修掉解析器的三类假阳性（Less **mixin** 定义/调用、`:global { }` **块形式**里的 `.ant-*`、`@import` 的文件扩展名），再把 25 个样式表的 69 个真实孤儿删净——`KNOWN_ORPHAN_COUNTS` 归空。**顺带修好一类真 bug**：`.listActions .ant-btn` 没包 `:global`，CSS Modules 把 `.ant-btn` 也哈希化，这条圆角规则从写下起就没生效过。四个迁移页面另有 79 个孤儿（21+22+21+15）在本轮更早一次删除。⑤ **其他 CLI 的孤儿也一并删净**：用户追问「之前迁移的其他 CLI 的孤儿 css 也删了」时，守卫报的是 0——**但它的盲区里藏着残留**。① 动态键曾让整份样式表退出检查（`SessionDetailWorkbench` 174 类里 7 个真孤儿）；② 没人导入的样式表整个漏掉（`github-markdown-overrides.module.less` 是 `MarkdownPreview/index.module.less` 的旧副本、全仓零导入）。补齐两条盲区后再删 8 个类 + 1 个文件，**反向检查从此不再对任何样式表开天窗**。教训：守卫报 0 时先问它扫了什么。⑥ **OMP 供应商弹窗的协议说明丢了换行**：旧内联实现的 `.apiOptionDescription` 带 `white-space: normal`，搬进弹窗后变成 `<Text>`，而 antd 选项行是 `nowrap`，40+ 字说明被省略成一行——**迁移「搬了内容、丢了让它生效的样式」**，是比孤儿类更隐蔽的一类。上一轮：2026-10-22（**页面容器间距 + CSS Modules 类名：两条「静态检查全绿但界面是错的」缺陷，补成规则与守卫**（非迁移轮的回修）。用户报「今天新迁移的 3 个 CLI 顶部和下面的模块之间的间距有点大，其他的 CLI 没有这个问题，之前也犯过这个问题，sop 里面没写吗？」——**SOP 里确实没写**：OpenCode 2026-10-19 修过同一个 32px，但那条结论只记在 §4.3 的**表格单元格**里（opencode 那一行），照 §4.3 当「迁移状态表」读的人根本不会读到它。四条后果与修正：① **§4.3.1 新增**「页面容器不许有 `gap`」——`CodingPageHeader` 自带 16px，容器再 `gap:16px` 就叠成 32px；正确形状是容器 `display: block` + 区块自带 `margin-bottom: 16px`（`block` 让 margin 折叠，`flex` 不折叠，所以某个区块根节点自带 16px 时 flex 会把间距翻倍）。pi / oh_my_pi / dsh / hermes 四页同改。② **§4.3.2 新增**「迁移后查 CSS Modules 类名」——`OhMyPiPage` 页面写 `styles.ompSection`、less 写 `.piSection`，**6 个区块长期渲染成 `class=""`**，连同 `scroll-margin-top` 一起静默失效；CSS Modules 少一个键既不报错也不警告，取到 `undefined` 而已。③ **两条机械守卫**：`pnpm run test:page-rhythm`（真浏览器量四个页面 × 页头→首区块 / 区块→区块，都必须是 16px，已验证「重放 bug 会红」）与 `pnpm run test:css-module-classes`（源码级，扫全仓 `*.tsx` 的 `styles.x` 是否在对应 `.module.less` 里有定义；3 个历史遗留登记为已知项，只减不增）。**守卫自己踩的坑也记进 §4.3.2**：第一版只剥 `/* */` 注释，而那句「// was `.ompSection`」的注释把缺失的类名「定义」上了，对真缺陷报绿——写源码守卫必须拿已知违规文件跑一次。④ **SOP 自省规则的反例**：本轮暴露的不是「漏补一条教训」，而是「教训写在了错误的位置」——进度表单元格不是规则正文，读不到等于没写。已同步 `web/features/coding/{pi,oh_my_pi,dsh,hermes}/AGENTS.md`。上一轮：2026-10-22（**pi / oh_my_pi / dsh 迁到共享卡片：旧卡正式退役 + 三个后端补齐 reorder**（迁移轮，最终三家）。三家都是「per-provider 模型目录 + 没有 provider 级 应用」（默认指针在 `modelSettings` / `modelRoles.default` / `agent-default-model`），按 §0.1.1 全部用 OpenCode 式，头部无主操作。① **映射层 + 弹窗抽文件**：新建 `PiProviderCard` / `OmpProviderCard` / `DshProviderCard` 三个薄映射层，以及 `PiProviderFormModal` / `OmpProviderFormModal` / `DshProviderFormModal`（横向布局）——这三家此前的供应商弹窗是**内联在页面里**的 `layout="vertical"` 网格，`verify-form-modal-layout` 按文件名扫描，一直没管过它们（§4.2.10 的那条 ⚠️ 说的就是这回事）。守卫文件数 15 → 18。② **拖拽要把后端一起做完**：Hermes 那轮的结论再次应验——读取端按字母序重建列表，拖了也白拖。三家的 `build_provider_views` 原本都用 `BTreeSet` 收口 key（pi 混了 `auth.json` / `models.json` / `settings.json` 三个来源，omp 还先把 `openai-codex` 排到最前，dsh 遍历 providers 字典），本轮改为**跟随存储文件的键序**，并各补一条 `reorder_*_providers` 命令（`reorder_pi_providers` / `reorder_omp_providers` / `reorder_dsh_providers`）。**判据：拖拽是「把手 + 行为 + 存取」三件套，缺一件就表现成坏掉**（13.1 模式二十五）；只有把手没有 reorder 命令时，唯一诚实的做法是把把手关掉。写盘仍按序列化文本比较变化（`preserve_order` 下 `Value` 相等与键序无关，见上一轮）。③ **旧卡退役**：`components/common/ProviderCard/index.tsx`（534 行）全仓已无 import（只有 `types.ts` 的 `ModelDisplayData` 等类型仍被共享 `ModelListSection` / `ModelItem` 使用），旧卡组件本体删除，`scripts/i18n-keys.mjs` 对应前缀登记一并移除。**「迁完」的判据是旧写法残留数为 0**（13.1 模式二十八），不是新写法出现了——这道守卫的第四条规则（禁止 import 旧卡）正是为此保留。④ **目录 ≠ 文件**：pi / oh_my_pi / dsh（以及先前的 hermes / omo_native）页头显示的是**配置目录**，而 `CodingPageHeader` 固定用 `common.configPath`（「配置文件路径」）。此前 §4.1 记着「迁移时再加 `configPathLabel` 之类的 prop」——本轮按那个方向落地成 `configPathKind: 'file' | 'directory'`（**不是自由文案覆盖**，两份文案都在 `common.*`：`common.configPath` / 新增的 `common.configDir`），调用方只声明「我这个路径是哪种东西」。⑤ 回归：`test:migrated-provider-cards` 62 → 78 项（Pi 端点/协议、无应用、凭据不当端点、把手、默认模型行、内置源标签与不伪造端点；OMP 端点/把手/订阅状态、dsh 端点/sdkName/凭据 ref 标签、目录型 dsh 无编辑与添加模型且无把手），`ompProviderForm.test.ts` 由「抓页面源码」改写为 8 条纯函数断言（页面里那两个内联 handler 已随弹窗抽取消失）。上一轮：2026-10-22（**多选模式：不可选的行也要有复选框，置灰并说明原因**（非迁移轮）。用户报「供应商列表多选时正在使用的那个前面没有多选按钮……应该是有多选按钮但是置灰，鼠标移动上去之后有说明，这样用户才知道为什么不能多选，不是以为出 BUG 了，其他 cli 也要这样」。① **缺失的控件与禁用的控件是两种信息**：多选只给可选行画复选框，被排除的行左槽直接空掉，用户读到的是「按钮丢了」；改为每行都画，不可选的 `disabled` 并挂 tooltip。这与 #112「被有意禁用的能力必须说明原因」是同一条规则，只是这次缺失的是控件本身。② **判定与文案分两层**：页面知道自家记录里哪个字段意味着「默认 / 本地 / 官方」，共享层只知道「为什么不能选」这句话怎么写。新增 `ProviderSelectionBlockReason`（`default | local | official | officialAccount | readOnly | builtinSource`）+ `providerSelectionDisabledReason(t, reason)`（`shared/providerList/selectionDisabledReasons.ts`，文案在 `common.batch.selectDisabled*`），页面只补一个 `batchBlockReason(provider)`。每个 CLI 各写一遍措辞，就是当年同一句「默认渠道」出现三种说法的来路。③ **`Tooltip` 的宿主必须是 `<span>`**：禁用的 `<input>` 不派发 hover，直接包 checkbox 会让原因永远不出现——浏览器断言里用真悬停验证过。④ 三处渲染点同时改：`CardShell`（Claude / Codex / OpenCode 式）、`ProviderCompactCard`（卡片视图）、以及 `components/common/ProviderCard`（pi / oh_my_pi / dsh 尚未迁移的三家）。`selectDisabledReason` 有值即视为「选择模式」，拖拽把手让位；全选语义不变，仍只覆盖可选行。回归：`test:migrated-provider-cards` 55 → 62 项（可选行活复选框、不可选行置灰、悬停出原因、只读行同样置灰、两种情况都不再出现把手）。
>
> 上一轮：2026-10-22（**拖拽排序的两处「写不进去、读不回来」+ 测试空转**（非迁移轮）。用户报「openclaw 排序不生效，Hermes 连排序的小把手都没有」。两条根因都在**持久化链路**，不在控件：① **OpenClaw：写盘 diff 把「只换序」判成无变化**。拖拽只改 `models.providers` 的**键序**，`roundtrip::apply_root_section_diff` 用 `serde_json::Value` 相等判定变化，而本 crate 开了 `preserve_order` —— map 相等**与键序无关**，于是 `save()` 命中「无变化」分支跳过写盘，`loadConfig()` 读回旧顺序。改为比较**序列化文本**；回归 `apply_root_section_diff_writes_a_provider_reorder` 直接断言写出的键序（用 `serde_json::Value` 比较的断言会假绿）。**判据：凡是「只有顺序是数据」的写入，都不能用集合相等做变更检测。** ② **Hermes：读取端按字母序重建，拖了也白拖**。`build_provider_views` 用 `BTreeSet` 收口所有 provider key，顺序永远字母序；且当时**没有 reorder 命令**，所以上一轮把把手关掉是对的。本轮补齐：读取改为 `custom_providers` 序列顺序优先（只读 `providers:` 字典与悬空默认渠道紧随其后）、新增 `reorder_hermes_providers`（只重写 `custom_providers`，payload 里没提到的 key 保留在原相对顺序之后，绝不丢弃）、前端恢复 `DndContext` + 把手（只读渠道无把手，搜索/非 `custom` 排序禁用拖拽），乐观更新只替换**可写项**的位置、不把只读项挤掉。③ **测试空转**：`test:provider-list-drag` 的 `sortMode=created` 断言期望 `A,B,C`，而 fixture 的 `createdAt` 恰好也让排序结果是 `A,B,C` —— 这条断言**无法失败**；已改成与存储顺序相反并新增三条「真的点排序菜单」的断言（重排、落库、不折叠区块）。**排序/拖拽这类缺陷必须用真事件验收，且断言必须能区分「排序生效」与「没生效」。** 回归：`cargo test --lib` 2970 项、`test:provider-list-drag` 7 → 10 项、`test:migrated-provider-cards` 53 → 55 项、`hermesFetchedModels.test.ts` 补预设名断言。
>
> 上一轮：2026-10-21（**OpenClaw 与 Hermes 迁到共享卡片：五个配置型 CLI 收尾**（迁移轮）。两家都是「per-provider 模型目录 + 没有 provider 级 应用」，因此都用 OpenCode 式，头部无主操作。① **迁移层不再允许 import 旧卡**：`verify-provider-card-layout.mjs` 新增第四条规则（映射层不得 `from '@/components/common/ProviderCard'`）——旧规则只查「自建 chrome」，而旧卡是**共享组件**，所以 `OpenClawProviderCard` 包着它几个月、带着旧的内边距与头部，守卫一句话都没说。② **OpenClaw**：`apiToNpm` 抽到 `utils/apiToNpm.ts`（页面 / 映射层 / 两个弹窗共用），删除确认下沉到页面 handler 并令 `deleteConfirm: false`（旧卡的 `Popconfirm` 只保护列表，格子菜单与详情侧栏是绕行入口）。③ **Hermes**：新建 `HermesProviderCard`；只读渠道不渲染编辑/复制/删除与模型增改，并用 `nameTags` 画「只读」——此前这条事实被拼进 Base URL 兜底文本（伪造 URL）。**没有 provider 拖拽**（后端无 reorder 命令），模型行的假把手一并去掉。**复制按钮顺手修好**：此前打开空白新增弹窗，现在预填源渠道、Key 加 `_copy`。④ **探针位置变化**：两家的「模型测试 / 获取模型」从旧卡头部移到模型区工具栏（与 Codex / ZCode 一致）。回归：`test:migrated-provider-cards` 36 → 53 项，`apiToNpm.test.ts` 3 项。上一轮：2026-10-21（**获取模型补预设 + 元信息行不换行 + 官方渠道的应用门控**（非迁移轮）。① **获取模型的预设补全**：Grok 的「获取模型」此前只写 `key/model/displayName/baseUrl/apiBackend`，与 Codex/dsh/OMP/Hermes/OpenClaw/Pi/OpenCode/ZCode/OmO 全部不一致——新条目一律经 `findPresetModelById`（Grok 为 `findGrokPresetForModel`：xAI 组 → 协议组 → 全组回退）补 `contextWindow`/思考菜单/`modalities`。抽出 `mergeFetchedGrokCatalogModels`（`grok/utils/grokFetchedModels.ts`），上游 id 原样保留，已有条目只刷新渠道级字段。**「获取模型」是共享语义，不是各 CLI 的自由发挥**：新 CLI 的这条路径必须先查预设。② **元信息行不换行**：`ClaudeStyleCard` / `CodexStyleCard` 的 `metaEntries` 行改为 `nowrap` + 条目省略 + `inlineActions` 不收缩（用户报 Claude Desktop 卡片的「模型测试」被长端点挤到第二行）。判据：**动作属于这一行，事实可以被压**。③ **官方渠道的应用门控进共享层**：`primaryAction.disabledReason` 只在 `disabled` 时渲染（`tooltip` 是「做什么」，不能兼任「为什么不能点」）；Grok 官方渠道在没有任何账号时锁定「应用」并说明原因，三处（列表 / 格子 / 详情侧栏）因共用 `renderProviderPrimaryAction` 而只接线一次。④ **没有账号后端的 CLI 不要照搬账号区块**：Claude Desktop 的 1P 登录由本体管理，官方卡片改为第二行说明「应用会切回 1P、移除 3P profile」，并去掉永远 disabled 的「模型测试」。回归：`test:migrated-provider-cards` 30 → 36 项，`grokFetchedModels.test.ts` 5 项。上一轮：2026-10-21（**Claude Desktop：官方行成为固定列表成员 + 修掉「需网关却被判成直连」**（非迁移轮）。① **官方渠道不再是新建选项**：后端 `list_claude_desktop_providers` 首次列出时 `ensure_official_provider_row` 保证恰好一条 `category=official` 行（复用已有第一条，否则 seed `claude-desktop-official`、`sort_index=-1`）；表单新建时下拉只有自定义 / 内置网关渠道，编辑官方行时渠道只读、**名称可编辑**。此前「渠道=官方 + 名称输入框 `disabled` 且 `required`」让官方保存永远报「操作失败」。② **官方行是固定列表成员**：卡片标题 `🔗 官方账号`（共享 `OfficialAccountHeadingIcon`），删掉重复的「官方订阅」标签；整页按列表顺序取**第一条** official 行做删除保护（不进批量选择、三个删除入口都不渲染），复制/历史产生的后续 official 行仍可删——前端 `officialChannelProviderId` 与后端 `delete_claude_desktop_provider` 用同一判据。③ **「需不需要网关」前后端对齐**：新增 `hasClaudeDesktopRoutingModels`（镜像后端 `has_routing_models`），「任一 route 的 `model` ≠ `route_id`」即需网关，覆盖 `claude-sonnet-5 → claude-opus-5` 这类两边都是 claude-safe 名的映射（AnyRouter）；只判 `hasNonClaudeModelIds` 会让它显示「应用」，点下去后端静默写网关 profile 却没接管网关。后端 `apply_claude_desktop_provider` 同时补上 native 守卫（对齐 Claude Code），托盘改走 `provider_switch::apply_or_switch_provider` 并按同一谓词置灰。回归：`test:migrated-provider-cards` 30 项（新增官方标题/无删除/映射渠道三态）、`providerProtocol.test.ts` 新增镜像用例、Rust `claude_desktop` 36 项。上一轮：2026-10-20（**最后三个 PENDING CLI 迁入共享组件（迁移轮，非教训轮）**：claudedesktop / geminicli / grok 全面接入——卡片变体（`ClaudeStyleCard` ×2 + `CodexStyleCard`）、页头 `CodingPageHeader`、列表外壳 `ProviderListSection`、卡片视图 `ProviderCompactCard`；`omo_native` 补上卡片视图。共享层三处配套改动：① `ClaudeStyleCard` 补 `namePrefix`、真 `Tag`（`kind: 'tag'`）与跨满整宽的 `footer`（从头部左栏移出），`inlineActions` 不再要求有 `metaEntries`；② `OfficialAccountsSection` 新增自由插槽 `renderExtraActions`——Grok 的「刷新 Token」不再需要手写账号行；③ `providerCardMenu` 尊重 `deleteDisabledReason`（卡片视图的「更多 → 删除」此前绕过默认渠道保护）。`PENDING_MIGRATION` 两道棘轮均已归零/只剩 antigravity；新增 `test:migrated-provider-cards`（27 项真浏览器断言，组件级夹具）。审阅轮（同日）补了三处「官方分支吞掉用户备注」的回归：Gemini / Grok 的官方卡片第二行在切换说明之后仍追加 `provider.notes`，Claude Desktop 在没有角色绑定时把备注留在第二行（有绑定时才进 footer）；同时确认 Grok 的连通性入口只有模型工具栏一处（头部不再重复「模型测试」）。`test:migrated-provider-cards` 因此 24 → 27 项。同一轮还删掉了零消费方的 `actions.extraActions` 与已无引用的 `ClaudeDesktopPage.module.less`，并按实测重写了附录 B.1 的消费方列（原表把 kimi 漏出网关/主操作、把 `extraActions` 记成有消费方、把 Gemini 的状态点说成「入口在别处」——它根本没有连通性测试）。同步更新 §4.2.1 / §4.2.2.1 / §4.2.9 / §4.2.10 / §4.3 / 附录 B.1。上一轮：2026-10-19（**OpenCode 页面迁入共享组件（迁移轮，非教训轮）**：页头 `CodingPageHeader`、供应商区 `ProviderListSection`、卡片 `OpenCodeStyleCard` + `ProviderCompactCard`（薄映射层 `OpenCodeProviderCard`）、编辑弹窗 `OpenCodeProviderFormModal` 全部接入；官方模型只读目录走卡片 `footer`（不进可编辑的模型列表），「定位」指向默认模型所在渠道，批量删除的确认挪进 handler。`ProviderFormSections` 新增可选 `showNotes`——OpenCode 的 provider 记录没有 `notes` 字段，传 `false` 而不是渲染一个写了也不落盘的假字段。同步更新 §4.2.10 / §4.3 的迁移进度、§4.2.6 与附录 B.1 的 props 台账、附录 B 各消费方列。`components/common/ProviderFormModal` 已无消费者并删除，`scripts/i18n-keys.mjs` 的对应动态前缀登记一并移除。上一轮：2026-10-15（**不需要「应用」的渠道：格子用自己的控件占住那个动作位**：新增 13 节 #154 与 13.1 模式一百零四。① `ProviderCompactCard` 新增 **`cellControl` 插槽**——「这张格子自己的控件」，落在动作槽里、主操作之前，也就是别的 CLI 放 应用 / 网关代理 的位置；ZCode 的 provider 永不被应用（`设为默认` 在模型行上），格子第二行原本什么都不说，而它唯一的 per-provider 控件（启用开关）折在「更多」菜单里（用户 2026-10-15：「**针对 zcode 这种不需要应用的渠道，把它在列表模式下的开关按钮放出来，放到现在应用的地方**」）。② **那颗开关先收成一个组件再上槽**：列表卡头部（`OpenCodeStyleCard`）、「更多」菜单、格子动作槽三处共用 `ProviderEnabledSwitch`（`providerCardActions.tsx`；`checked` 说的是**启用**而记录里存 `isDisabled`，全仓唯一的取反就在这一处；菜单那份带 `tooltip={false}`，因为它自己的行上已经写了「启用」）。③ **上了槽就从菜单撤掉**那一条（`cellControl ? undefined : onToggleDisabled`）：菜单收起开关的理由是「旁边有 应用 会争抢」，格子那行没有主操作，理由不成立，留着就是同一张卡片说两遍。④ **位置类需求横向比**（#154 / 模式一百零四）：判据是「这个位置在邻居那里是什么」，不是「本 CLI 有没有那个按钮」。⑤ 回归：`test:zcode-card-view` 42 → 49 项——真悬停读 tooltip、真点一下禁用（核对回写的 `{providerId, isDisabled}`）、卡片随即陈述「已禁用」、回来的路仍在同一行、点它**不**打开侧栏、菜单里不再有第二条。上一轮：2026-10-14（**官方渠道格子的「当前登录」行改成任何状态都说话，ZCode 一并办**：新增 13 节 #153 与 13.1 模式一百零二～一百零三。① 那一行的取值从「只认 `isApplied` 的行」放宽成 `find(isApplied) ?? find(isVirtual) ?? 「未登录」`（新 key `common.officialAccount.currentLoginNone`）——起因是用户在**两条闲置渠道**上问「**为什么 codex 和 kimi 的卡片下面没有显示当前的账号啊**」：Codex 应用的是 AxonHub、官方渠道 `is_applied=0`，Kimi 两个渠道都没应用，于是那一行没有可选值、整个不渲染；而它的位置无论说不说都被共享卡片占着（`.bodyWithAction` 的 30px），**留空读起来是坏卡片，不是「无话可说」**（#153 / 模式一百零二）。口径由用户定：他选的是**「只写真正在用的登录」**——闲置态写「未登录」，不把闲置的账号说成当前登录；Codex 那份仍在 `showOfficialRuntimeState` 门控内（网关代理/接管时 `auth.json` 装的是网关的凭据）。**ZCode 同一条规则**（用户同轮指定「zcode 也改成始终展示当前正在登录的账号」）。② **诊断靠真机数据而不是读代码**：应用库 `ai-toolbox.db` 的 `data` 列是 SQLite **JSONB**——`sqlite3 -readonly <db> "select json_extract(data,'$.is_applied') from …"` 直接问（先按 msgpack 解字节对不上，白绕一圈）；CLI 侧读磁盘真文件：`~/.codex/auth.json` 是 `auth_mode=apikey` 却仍留着 marujunyy@qq.com 的过期官方 token（截图上 `(2)` = 1 保存 + 1 本地就是这么来的）、`~/.kimi-code/config.toml` 是未登录的默认桩（#153 / 模式一百零三）。③ 夹具加 `accounts=virtual` 与 `accounts=idle` 两个模式，**三态各自断言**（Codex 的 `virtual` 特意做成用户那台机器的真实形状：**一份保存但没在用的账号 + 磁盘上那份登录并存**，格子必须报「在用」的那份、不报闲置的）：`test:codex-card-view` 76 → 81 项、`test:kimi-card-view` 34 → 37 项、`test:zcode-card-view` 40 → 42 项。上一轮：2026-10-14（**「格子 + 它自己的详情抽屉」收进共享卡片，官方渠道的格子改陈述「当前登录」**：新增 13 节 #151–152 与 13.1 模式一百～一百零一。① **抽屉跟面板一起归共享卡片**：`ProviderCompactCard` 收 `detail = { title, statusTags, primaryActions, secondaryActions, sections }`，自己持有开合状态、自己渲染 `<Drawer size="min(60vw, 760px)" destroyOnHidden closable={false}>`；Claude / Codex / ZCode / Kimi 四个映射层此前各写了一份**逐字相同**的抽屉（唯一差异是面板内容），现在四家都不再 import `Drawer` / `ProviderDetailPanel`，也不再各自持有 `detailOpen`（#151 / 模式一百。用户原话：「**样式都封装到通用组件啊，不要每个单独改啊，sop 里完善一下**」）。② **卡片模式下三家的官方渠道格子逐字同形**：格子第二行只说一件事——「当前登录: <账号>」（共享 `common.officialAccount.currentLogin`，值取该账号在自己那行的 label，Codex / Kimi / ZCode 一致）；账号**管理器**（一行一个账号、每行各有动作）进**该格子自己的详情侧栏**，且**常开、无 caret**（用户：「kimi和codex的官方账号卡片上也显示当前登录账号，不要显示主模型了」）。③ 把手也是控件：`handleCardClick` 的忽略列表加 `[data-provider-card-handle]`（它是 `<span>`，只按标签名过滤会让「按住把手拖一下」顺带打开侧栏），并补反向断言「拖它不会打开侧栏」。④ **一条结论被推翻后三处正文没清**（#148 / 模式九十八的「跨整行」动作、13.1 里讲已整份回滚的 `ProviderCardGrid.rowItem` 的孤儿段落、`providerCardView/AGENTS.md` 里与同文件第 34 条自相矛盾的那条）——起因是用户问「**sop 里面没写吗？**」，实际是「写过了，但读者先撞上的是旧答案」（#152 / 模式一百零一）。⑤ 套件：`test:zcode-card-view` 32 → 40 项、`test:kimi-card-view` 33 → 34 项、`test:codex-card-view` 75 → 76 项。上一轮：2026-10-13（**另外两个 CLI 接入卡片布局：ZCode / Kimi**：新增 13 节 #148–150 与 13.1 模式九十七～九十九（#150 为后续一轮：**卡片模式下 ZCode 的官方账号那块内容与邻居不同形**——先保留了列表外壳、又改成裸内容，两版都被用户退回；定稿是「穿宿主里其他成员穿的那件」：同一个 `ProviderCompactCard` + `renderBody`）。① 两个映射层各加 `viewMode`（`'card'` 分支渲染 `ProviderCompactCard` + 自持详情 `Drawer`），页面接 `useProviderListViewPrefs` / `ProviderCardGrid` / `rectSortingStrategy` + `restrictToWindowEdges`，卡片 props 收进一份 `*ProviderCardProps(provider)`（两种布局同一份）。② **格子只陈述「默认模型」**（Kimi 用 `defaultModelKey`、ZCode 用 `models[].isDefault`），连接信息与模型目录（`appearance="detail"`，常开）进侧栏；Kimi 的官方账号块由 `renderOfficialAccounts({ fold })` 按宿主给形态——**列表卡片那侧逐字未动**（#147 的纪律）。③ **ZCode 的官方账号卡片是列表成员却不是 provider**：两种布局的成员顺序来自同一个 `listMembers` 表达式（#148）；网格里它穿宿主里其他成员穿的形状（同一个 `ProviderCompactCard`，与邻居同宽同高，**当时把管理器整个塞进了格子**——第二天的 #151 把管理器移进它自己的侧栏），位置那一步之后不再有「跨整行」。④ 两条新浏览器套件：`test:zcode-card-view`（当时 32 项，现 40）、`test:kimi-card-view`（当时 33 项，现 34）。⑤ 测试侧教训：抽屉关闭是异步的，`waitFor(侧栏存在)` 会被**正在关闭的那个侧栏**满足，后续断言全在读上一个 provider 的面板（#149 / 模式九十七）。上一轮：2026-10-13（**「明细的需求」与「列表的现状」：一轮收尾，加一次作用域回退**：新增 13 节 #144–147 与 13.1 模式九十三～九十六。① **明细侧栏**的模型行：静止时文本独占整行（把手与带文字的动作 `display: none` 真正让出宽度，不用 `visibility`），模型名与「模型列表」标题左对齐；图标三连与开关保持常驻（#143 的续集，同一处几何用成对断言钉住：静止 vs 悬停）。② **明细里**的账号列表不再折叠（`OfficialAccountsSection` 的 `collapsible` 仍由「有没有传 `onToggleCollapsed`」决定，ZCode / Kimi 的卡片保留折叠）；抽屉统一为 `size="min(60vw, 760px)"`，与 Skills / MCP 一致。③ **Codex 卡片把最后一个动作挂到副标题行**（`actionPlacement: 'footer' | 'subtitle'`，同一份动作节点择一处放置）。④ **`ModelItem` 的 `useSortable` 从布尔 `disabled` 改成对象**——dnd-kit 把 `disabled: true` 读作「不能拖但仍可被放下」，模型列表一展开就让供应商卡片拖不动（#146 / 模式九十五，由 Kimi 的官方卡片拖拽检查发现）。⑤ **① 和 ② 一开始被做成了共享组件的全局形态，列表布局跟着一起变**——用户 2026-10-13 报「**列表的样式不要变啊，列表样式的模型列表怎么也没有折叠了**」；两个区块改为按宿主给形态（`ModelListSection.appearance`：`'card'` 折叠 + 卡片口径的行，`'detail'` 常开 + 侧栏口径的行；`renderOfficialAccounts(foldable)` 同构），列表那侧**逐字还原**（#147 / 模式九十六），并补了「列表那一侧没变」的四条断言：仍折叠、点开有行、行是 13px、把手在、带文字的动作占位。上一轮：2026-10-12（**两处布局缺陷：整行高度与「隐形按钮吃宽度」**：新增 13 节 #142–143 与 13.1 模式九十一～九十二。① 用户报「**副标题离下面的分隔线太远**」——网格把整行拉到最高那张，没有模型行的卡片把 17px 差额堆在分隔线上方；`.body` 按自身尺寸预留该行（`min-height: 23px`，含下内边距）（#142 / 模式九十一）。② 用户报「**明细里的模型列表展示有问题**」——`ModelItem` 的动作区 `visibility: hidden` 仍占 362px，478px 的侧栏行里模型名只剩 60px、被折成四行；文本改为 `flex: 1 1 220px` + 单行省略，动作放不下时整组换行（#143 / 模式九十二）。两处都补了成对几何断言（有/无模型行等高、同行距相等；文本不被截断且 ≥200px），Codex 套件 55 → 60 项）上一轮：2026-10-12（**Codex 接入卡片视图 + 「断言之外的第三条证据」**：新增 13 节 #139–141 与 13.1 模式八十八～九十。① 侧栏里 `common.model.title`（模型列表）同时被当作模型绑定区的专名 → 一个侧栏两个同名标题，新增 `codex.model.mappingTitle`（模型配置）（#139 / 模式八十八）。② 夹具的命令桩 `list_codex_official_accounts` **忽略 `providerId` 参数**，让自定义 provider 也长出账号块与「历史官方账号」提示——53 项断言全绿，靠截图发现；修桩 + 补一条反向断言「自定义 provider 没有账号块」（#140 / 模式八十九）。③ 把 `modelSection.*` 的渲染点搬进共享 `ProviderModelSection.tsx` 后，`verify-provider-card-layout.mjs` 把 7 个**活着**的 prop 报成死的——守卫的作用域清单必须与代码搬迁同改（#141 / 模式九十）。附录 A.1 的卡片视图行补 codex，`ModelListSection` 行改为经 `ProviderModelSection` 触达。接入细节：映射层加 `viewMode` + 页面换上 `ProviderCardGrid`，格子只留主模型、目录与官方账号块进侧栏（55 项断言，`pnpm run test:codex-card-view`））上一轮：2026-10-12（**卡片视图的类型尺度与「同一状态一个说法」**：新增 13 节 #135–138 与 13.1 模式八十四～八十七。① 用户报「**卡片上的已应用标识没了**」——卡片视图过去只用 accent 边框表达已应用，而边框不是陈述；`nameTags` 拆成共用的 `stateTags` + 各布局自己的前缀（#135 / 模式八十四）。② 用户报「布局切换按钮和通用配置的 icon 一样」——lucide `LayoutGrid` 与 antd `AppstoreOutlined` 都是 2×2 方块；换成 antd `LayoutOutlined` / `UnorderedListOutlined`，**并补上文字**，位置移到「一键测试」之前（#136 / 模式八十五）。③ 「API Key 一行、中间省略」不能用 CSS 做 → `utils/middleEllipsis.ts` 纯函数 + 单测，完整值走 `title` 与复制按钮（#137 / 模式八十六）。④ 共享按钮渲染函数里的内联 `fontSize` 挡住了「卡片按钮变小」——密度归宿主（#138 / 模式八十七）。同轮：卡片三段式字号对齐 Skills（标题 14/600、副标题 11、footer 与标签 11/10）、副标题条目间加 `|` 分隔、标题变成来源链接（悬停给域名、点击 `open_url`）、详情侧栏新增「API 格式」行与 Base URL/API Key 复制按钮、key 不再打码；浏览器套件 52 项。详见 `web/features/coding/shared/providerCardView/AGENTS.md` 与 `web/features/coding/claudecode/AGENTS.md`）上一轮：2026-10-11（**供应商卡片视图（issue #399）**：新增 13 节 #132–134 与 13.1 模式八十一～八十三。① 新写的共享组件 `ProviderCompactCard.tsx` 差点被 `verify-provider-card-layout.mjs` 按后缀 `*ProviderCard.tsx` 误扫成「CLI 映射层自建 chrome」——**名字是契约的一部分**（#132 / 模式八十一）。② 卡片视图是「同一份数据的另一种呈现密度」，不是 `providerCardVariants/` 的第四种样式；两者共用面抽成 `cardChrome.ts`（accent）与 `providerCardMenu.tsx`（「更多」菜单契约）（#133 / 模式八十二）。③ 新增布局偏好接进既有的 `providerListStateCache` 单飞缓存，而不是另起一个自带读取的 hook（#134 / 模式八十三）。附录 A.1 加 `providerCardView` 一行，附录 B.1 的 `ProviderListSection` props 台账加 `viewMode` / `onViewModeChange`）上一轮：2026-10-11（**补上「官方账号区块」的接入步骤那一节**：新增 §4.2.9「官方账号区块 `officialAccounts`」（两步接入：先判宿主、再写映射；字段表、props 表、三条后端不变量、最小验证），附录 A.1 的共享组件对照表补上这一行，附录 B.1 补该组件的 props 台账；另新增 13 节 131 与 13.1 模式八十。起因是用户问「**sop 里加上官方账号的处理方法了吗**」——上一轮把教训（#120–#130）、模式（六十九～七十九）、4.0.2 的清单 bullet、附录 B.2 的区块段、守卫表行都写了，**唯独方案里承诺的那一节没写**（§4.2 下根本没有它，附录 A.1 也没有这一行）。**教训 / 模式 / 清单 / 接入步骤是四种产物，一种顶不了另一种**：回答「我要接一个新 CLI，第一步做什么」的是正文章节，不是历史坑表；收尾时把方案里承诺的产物逐条 `grep` 定位，别凭记忆说「我写了」）」上一轮：2026-10-10（**官方账号区块三端统一 + 「位置类需求」的验收**：新增 13 节 126–129 与 13.1 模式七十五～七十八。① 「把卡片头部那两行换成 `🔗 官方账号 (2)` + 说明」被我做成了改**区块自己的标题行**，用户回「改错了啊」——区块挂在卡片内部时「区块的标题行」≠「卡片那行」，两行候选**先用预览问清**，改完用**坐标**钉住。② 三端统一成一整套形状：标题行（🔗 + 官方账号 + 计数）、说明行、列表行（▾ 账号列表 + 登录入口）、账号行 + 折叠能力；措辞收进 `common.officialAccount.*`（每个 CLI 仍自己传已翻译串，但传同一批 key），图标与计数收成两个共享导出，官方卡片去掉 `官方订阅` 类别标记，Kimi 的卡片标题不再显示「Kimi Official」。③ ZCode 的官方账号卡片一直自己写 `<Card>` + `useSortable` + 把手——`verify-provider-card-layout.mjs` 的作用域是**文件名后缀** `*ProviderCard.tsx`，这个 `*OfficialAccountCard.tsx` 从未被扫过（它的 `EXEMPT_FILES` 豁免也从来没被读到），于是把手丢了悬停反馈、还被放进标题行内部，正文贴到卡片左边缘（用户圈出「为什么下面的内容左边距没了」）。作用域改成后缀集合，卡片外壳一律走 `CardShell`。④ 登录按钮从卡片头部挪到**列表行右端**（用户：「登录按钮应该是放到账号列表标题的右侧啊」）——按钮的落点看「它改动谁」。⑤ 追加（同日晚些）：说明句在 ZCode 上比另外两家深一档——同一句话走了两条渲染路径（antd `secondary` alpha 0.45 vs CSS 变量 0.65），用户贴图问「颜色和字体大小和另外 2 个的不一样吗？」。收成共享组件 `OfficialAccountHint`（`ProviderCardMetaEntry.value` 随之从 `string` 放宽为 `ReactNode`），并**新建 ZCode 真浏览器套件** `pnpm run test:zcode-official-account`（12 项，含 `getComputedStyle` 颜色断言）——此前 ZCode 这一侧没有任何几何/样式兜底。上一轮：**共享区块的装饰取并集 + 源码守卫的假绿**：新增 13 节 124–125 与 13.1 模式七十三～七十四。① 抽 `shared/officialAccounts` 时按 Codex 的形状抄了标题行，而 Codex 恰好把 🔗 画在登录按钮上 → 标题行没有图标；Kimi（迁移前 `KimiOfficialAccountCard.tsx:125`）与 ZCode 本来都有。用户对照 ZCode 截图说「都改成 zcode 这种样式」。图标已进共享组件（ZCode 的 `leadingAction` 里那份随之删除），Codex 补上此前缺的副标题（文案随 `isOfficialProvider` 门控，非官方卡片不显示）。**抽组件前要把每份原实现的装饰取并集**，不是照其中一份抄。② `verify-official-accounts-shared.mjs` 的 `FORBIDDEN` 匹配的是 `anticon-swap` / `anticon-check`——**antd 渲染后的 DOM class**，源码里不存在，于是 geminicli / grok / antigravity 三份手写区块从未被扫出，每次报告「0 待迁移」的假绿。改为源码级信号（自己 `map` 账号行）后三份现形并登记进 `PENDING_MIGRATION`。**源码守卫只能匹配源码里真实存在的字符串，写完先拿已知违规文件试一次。** 上一轮：**账号行的身份**：新增 13 节 123 与 13.1 模式七十二。Kimi 的账号行原本只能显示「登录时间」，理由是「Kimi 的 OAuth 不发身份」——**这条是错的**：access token 是 JWT（`sub` = user_id），2.x CLI 还有 `GET {base}/me`（`nickname` / `user_level_name` / `email`）。用户一句「列表里都不知道是哪个账号啊」直接推翻。改为：显示名取 `/me` 的 `nickname`，`user_id` 存 `subject` 并**优先按它判重**（重登换 token、身份不换），凭据指纹降级为回退。**上一轮：官方账号区块抽成共享组件 + 两个不变量**：新增 13 节 120–122 与 13.1 模式六十九～七十一。① 官方账号列表原本在 Codex / Kimi / ZCode **三处各写一份**，差异全是无意分叉（同动作三种叫法、同句说明一处副标题一处脚注、切换按钮一处隐藏一处置灰）→ 抽 `shared/officialAccounts`：区块本体共享，各 CLI 只写「自家记录 → `OfficialAccountRowView`」映射；**登录入口是 `ReactNode` 插槽而不是布尔开关**（否则共享组件被迫知道一个 CLI 有几个官方 provider），宿主由调用方挑（`embedded` 卡片内 / `standalone` 自持卡片）。守卫 `pnpm run test:official-accounts-shared`（源码棘轮）。② **「登录 / 导入 / 应用」成功之前不许落任何行**——Kimi 的登录在设备授权前先建 provider 行且取消不回滚，于是每次「点登录又关掉」都留一张空壳卡片（用户截图圈出）。改为后端在 token 交换成功时才 `ensure_official_provider`，`start_*_device_auth` 随之**不再接收 `provider_id`**。③ **凭据文件名与显示名必须是两个字段**——Kimi 拿 `account.name` 当文件名、而真实 CLI 只认 `credentials/kimi-code.json`，于是账号表按 provider 去重、永远只能有一个账号；解耦后文件恒为固定键、`name` 当显示名，磁盘一份 live 凭据 + DB 多行快照。Kimi 的官方渠道行现在与 Codex 同形：只由「手工新建」或「provider 表为空时导入本地登录」产生，账号嵌在官方卡片内部。**上一轮：「定位当前已应用供应商」+ 时间维度交互的验收**：新增 13 节 118–119 与 13.1 模式六十八。供应商列表标题右侧加「定位」按钮：滚到当前已应用的卡片并闪一下（共享外壳 `ProviderListSection` + 卡片 `data-provider-id`，5 个已迁移 tab 各传一行 `locateProviderId`）。① 闪光必须在**卡片停稳之后**才开始——点击即闪 + 平滑滚动 = 动画在屏幕外跑完，用户到达时看到的是一张毫无标记的卡片；且 keyframes 要「立刻绘满 3px 环再淡出」（0% 写 spread 0 等于没画），断言点因此在「到达那一刻」且要求环的 spread ≠ 0。② 浏览器夹具必须包 `<main>`（`App.css` 把 `html/body/#root` 锁 `overflow:hidden`，真正滚动的是 `MainLayout` 的 `main`），否则位置类断言跑在「一个不可能滚动的页面」上；夹具 query 自带 `&` 时还必须 `encodeURIComponent(runId)`，否则表现为 10s 超时。③ 「定位」对**没有「单一已应用供应商」概念**的 CLI 不成立（omo_native 多个 provider 同时生效）→ 共享外壳按「传了才渲染」自然缺席，不为它造目标。回归：`pnpm run test:provider-list-locate`（11 项，含两条「答不了就说原因」，以及一条「`__local__` 桥接记录被后端标了 applied 但卡片不显示已应用标签，因此不算定位目标」）。⚠️ **上一条声称「新增 13 节 118–119」，但表尾当时停在 117**——本轮用的 118–119 才是真实新增的两条。**上一轮：共享组件「统一」掉的两个原值 + 先检索再设计**：13.1 模式六十五～六十七。① `CardShell` 的 body padding 抄了另一套老卡片的值（`8px 12px`），而所有 provider 卡片都是 16 → 每个迁移过的 tab 静默少了 8px 上下；Claude 式卡片的标题/副标题间距原本是「栈 4 + 行 4 = 8」、Codex 式是 4，共享组件压成了 4。对策：迁移时把原值当契约逐个抄（`git show <迁移前>:<原文件>`），共享组件内部允许各变体不同值，并把数值落成浏览器断言 `pnpm run test:provider-card-spacing`。② 同一种产品问题在别的 CLI 已有实现时（Kimi 的官方账号卡片 = ZCode 三周前已落地并写进规范的 `ZcodeOfficialAccountCard`），**先检索再设计**；用户说「参考 XX」就是检索信号。③ 「启动时迁移存量数据」要接进**已有的迁移序列**（同一把锁、同一时机），不是新开路径——Kimi 的「收编 CLI 已有登录」接在 `migrate_legacy_credential_names` 之后。**上一轮：区块说明是「副标题」不是「脚注」**：新增 13 节 117 与 13.1 模式六十四——Kimi 官方账号卡片（照抄 ZCode）把说明文字挂在账号行**之后**，空态时中间还夹着一整块空插画，用户圈出来说「副标题应该放在标题下面啊」。改为「标题行 → 说明 → 内容」，**Kimi 与 ZCode 两张同构卡片一起改**；判据是「遮住内容区只留第一屏，这句说明还看得出在讲这个区块吗」。搬运的通用坑是**只加没删**（同一句渲染两次），给夹具加通用断言「卡片里没有任何一句话被渲染两次」拦住。**卡片的「区」必须跨满整宽 + 「空了一块」只能靠量几何发现**：新增 13 节 116 与 13.1 模式六十三——`CodexStyleCard` 的 `footer`（官方账号区）被渲染进头部两栏的**左栏内部**，账号行的「切换 / 删除」右边缘比模型区工具栏短 160px（正好是右上「应用 / ⋮」的宽度），用户报成「账号列表右侧空了一大块」；类型检查、静态守卫全通过，**唯有 `getBoundingClientRect` 能发现**。修法与守卫：`footer` 移出两栏、与模型区并列，`pnpm run test:codex-official-accounts` 断言「账号行右边缘 == 卡片内容右边缘 == 头部动作右边缘」。判别顺序：① 量两个元素的边，再读代码；② 别只看「这一个区块」，要和它上下相邻的同类区块比——卡片里每个「区」都该跨满整宽。**「外壳插槽」之前先判身份 + 文案的跨层共用**：新增 13 节 114「『外壳缺位置』判断错了对象——官方账号区根本不是外壳的一部分，而是**列表成员**（该与供应商卡片并排、可拖动排序）」与 115「同一句文案被两层语义共用，改措辞会改错另一层」（Codex/Kimi 的账号行「切换」与渠道「应用」共用 key，直接改会把渠道按钮一起改掉；对策是按层拆 key，改文案前先 `grep` 全部渲染点）；#106 标注**结论已被 114 推翻**（规则仍成立，实例判错），Kimi 的落点改为 `children` + `alwaysVisible` 双传 + `officialAccountIndex` 持久化，`toolbarExtra` 随最后一个消费方删除。新增 13.1 模式六十一「判『外壳缺插槽』之前先判这个东西在列表里的身份——判别顺序：能否拖动/并排 → 是否整列表动作 → 是否只是下方补充信息；并先 `grep` 同形状的既有实现（ZCode 的官方账号卡片早就是成员卡片，#42/#61 已写成规范）」与模式六十二「一句文案被两层语义共用时改措辞会改错另一层——两个渲染点会同屏出现且语义不同 → 必须拆 key」。§12.3 迁移核对清单的「官方账号区块位置」一条改写为「必须是列表成员 + 真的拖动它 + 空态仍在」。**上一轮：「默认渠道不可删」进能力清单**：§4.0.2-H 新增一条勾选项（判据＝默认指针是不是写在 provider 记录之外），13 节补 113——OmO Native 从 Pi 抄了卡片的按钮，却没抄这条门控，而全仓另外六个页面都有。**交互验收要用真事件 + 禁用态要说明原因**：新增 13 节 111–112 与 13.1 模式五十九「功能被有意关闭时，界面必须说明原因——否则用户读到的就是『坏了』」（Codex 拖拽被有意禁用，而用户的 DB 里 `provider_sort_modes.codex = "created"`，于是「拖不动」被报成 bug；给排序控件加 tooltip 说明恢复方式）与模式六十「交互类缺陷不能用『读代码』验收——仓库里已经有真浏览器，用它」（拖拽被『读链路』修好两次都没修好，改用真指针事件一轮定位；落成 `pnpm run test:provider-list-drag`，含夹具要挂真实页面、必须从 `@/app/routes` 进入 import 环、宿主全局要在第一个 import 里赋值、拖拽要发多步 `mouseMoved` 四条要点）。§12.3.1 第三遍补「『点』必须是真事件」与「被有意禁用的能力也要点（不可用 + 说明了原因两条）」。上一轮为**「规则本身写错了」这一类的修正 + 小颗粒能力集**：§4.0.2-I **推翻重写**——原版「默认三区块，除此外不许有」是**错的规则**，导致 OmO 少了「其他配置」「官方认证渠道」两块（全仓分别有 7 / 5 个页面有），改为「先取并集再逐块删」并给出可复现的 grep 命令；新增 13.1 模式二十六「SOP 自己给出过强规则」、模式二十七「区块是大颗粒、卡片能力集是小颗粒，后者没人枚举过」、模式二十八「迁移做了一半就宣布完成——判据是旧写法残留数为 0，不是新写法出现了」与模式二十九「照搬了区块的『形』，没照搬它的『筛选』」、模式三十「『保存失败』的报告往往其实是『读取端看不见』」、模式三十一「把『安全最佳实践』套到不该套的场景」、模式三十二「同源工具不等于同语法」。13 节补 85–95：SOP 区块清单本身错、手工内建名单与引擎脱节（48 条并加三条回归测试）、预览标签只写文件名导致空块显示成「配置文件是空的」、保留 id/名字散在 **33 个后端文件（51 处字面量）** 各写各的（抽 `local_bridge.rs` / `shared/localConfig.ts`）、「抽成常量」只做了一半就宣布完成（旧写法残留数必须为 0）、卡片能力集靠记忆补（ZCode 缺复制 / OmO 缺批量删除）、**同一缺陷在别的页面也存在只是没人报**（按新清单全仓扫出 OpenClaw 供应商与模型**两个复制都缺**，已补）、**只读区块把全量都列了出来没筛掉不可用的**（内建渠道列了 49 个、48 个未配置凭据；改为照 OpenCode 只列已配置的，新增 13.1 模式二十九）、**写入端与读取端用了不同的字段导致界面永远显示空**（密钥存 `models.json` 的 `apiKey` 而 `has_key` 只查 `auth.json`；统一到 `auth.json` 并迁移旧值、**界面回填明文密钥**，新增 13.1 模式三十 / 三十一）、**「获取模型」没传密钥导致 401 且用错了 config value 模式**（OmO 的 `apiKey` 支持 `$ENV_VAR` 插值而 OMP 不支持，传 `"omp"` 会把 `$MY_KEY` 当字面量；新增 `ConfigValueMode::Omo` 复用 Pi 解析器 + 新增 13.1 模式三十二）。上一轮为共享组件的样式归属 + 交互三件套：13 节补 80–84 与 13.1 模式二十四「插槽只共享文案，样式留给调用方手抄」/ 模式二十五「拖拽是把手 + 行为 + 存取三件套，缺一件就表现成坏掉」。再上一轮为同源引擎字段核对：13 节补 76–79 与 13.1 模式二十三「抄了『名字像的』而不是『同引擎的』兄弟」——**字段形状以引擎自己的 `docs/models.md` 为准**）

---

## ⚠️ 本文件的自省规则（最高优先级，先读这条）

> **用户每指出一个问题，就说明本文件存在一处缺陷**——要么漏写了这一条，要么写错了。用户不需要明确指出「去改 SOP」；**只要他提出问题，就必须主动回来检查本文件并强化修正**。

**为什么**：本文件是**跨对话交接的唯一操作手册**。下一个接入新 CLI 的对话没有本次的上下文，它只会照本文件执行。本文件漏一条，下一个 CLI 就漏一项；本文件写错一条，下一个 CLI 就照着错的做。**用户亲自发现问题，说明本文件本该拦住它的那道关卡不存在。**

**发现任何问题（用户指出 / 自己排查 / 收尾复盘）后，除了修代码，必须做这四件事**：

1. **定位本该拦截它的关卡**——是 4.0.2 的某个核对项没写？是第 3 节的某个接入点清单漏了？是 13.1 缺一个模式？还是 §12 的验收项不够？
2. **补上或改正那一节**，并在**第 13 节**加一条编号教训（说明：现象 → 错因 → 正确做法）。
3. **若能归纳成模式**，在 **13.1** 补一个「模式 N」（说明：什么样的代码结构会静默失败 + 对策）。
4. **若该问题能被机械检查出来**，把检查命令写进对应的核对清单（4.0.2 / §12.1 / §12.3），让它下次能自动被发现。

**用户说「把之前的问题都反思一遍」时**：逐条回溯会话（及能查到的历史）中用户提过的每个问题，对照本文件现状列出「已覆盖 / 未覆盖 / 覆盖但不够具体」，把未覆盖的补上。**不许只补最后一条**。

**判断补在哪里的速查表**：

| 问题性质 | 补在哪里 |
|---|---|
| 某个具体控件/字段的形态不对 | 4.0.2 的 A–H 对应小节 |
| 某个能力整体缺失 | 4.0.2-H（能力核对）或附录 B.3 |
| 某个页面上「和列表并列」的区块 | 附录 B.2（页面级区块） |
| 某块共享组件「**怎么接入**」（有官方登录的 CLI） | §4.2.9（官方账号区块：判宿主 → 写映射 → 最小验证）+ 附录 B.1 的 props 台账 |
| **按方案补文档时只写了教训、没写那一节** | 13.1 模式八十：教训 / 模式 / 清单 / 接入步骤是四种产物——流程进 4.x 正文，坑进 13 节，收尾逐条 `grep` 对账 |
| 后端漏注册某处 | 第 3 节的下游清单 + 3.6 |
| 一类会静默失败的结构 | 13.1 新模式 |
| 编译通过但形态不对的返工 | 4.0.2 的核对项（**这条最容易被漏**） |
| **一条硬规则被绕过、且没人发现** | 把它变成**机械守卫**（源码扫描脚本 / 测试），挂进 `pnpm test`；进度类规则用「棘轮」——未迁移的登记在脚本里，只减不增。见 13.1 模式四十六 |
| **迁移共享组件时比错了参照物** | 4.1 / 4.2 对应小节 + 12.1 的 key 比对命令。见 13.1 模式四十七 |
| 跨模块铁律 | 同时写回根 `AGENTS.md` |

---

## 0. 前置决策

### 0.1 先定性：配置文件型 vs 根目录型

这个选择决定后续所有路径语义，必须先定，且不能混写（见 `tauri/src/coding/AGENTS.md`）。

| 类型 | 保存对象 | 现有工具 | 路径派生方式 |
|------|---------|---------|-------------|
| **配置文件型** | 配置文件路径 | opencode、openclaw、pi、omp、hermes、dsh | 改 `config_path`，派生目录在其旁 |
| **根目录型** | 配置根目录 | claudecode、codex、grok、geminicli、antigravity、kimi、zcode | 改 `root_dir`，其余路径全部由根派生 |

判断依据：上游 CLI 是否提供「一个根目录 + 固定内部结构」（根目录型），还是「直接指向某个配置文件」（配置文件型）。

### 0.1.1 再定性：单一 active provider vs 多渠道路由

**这个定性决定三件事：卡片样式、「应用」按钮存不存在、默认模型在哪里选。** 在 0.1 之后、写代码之前必须定。

| | 单一 active provider | 多渠道路由 |
|---|---|---|
| 语义 | 同一时刻只有**一个** provider 生效，切走即停用其他 | 多个 provider **同时可用**，「用哪个」由某个指针（默认模型/默认选择）表达 |
| 现有工具 | claudecode、codex、grok、kimi、geminicli、antigravity、claudedesktop | opencode、pi、oh_my_pi、dsh、hermes、**zcode** |
| 卡片样式 | `ClaudeStyleCard` / `CodexStyleCard` | `OpenCodeStyleCard`（见 §4.2.1） |
| 卡片上的「应用」按钮 | **有**（语义 = 切到这一个） | **没有**——会误导用户以为其他渠道被关掉 |
| 默认模型入口 | 卡片头部「应用」 | **模型行**上的「设为默认」 |

**判断依据**：问一句「同时配两个 provider，上游是都用还是只用一个？」都用 → 多渠道路由。

> **多渠道路由的 CLI 有一个必查项**：「谁是默认」这个状态**存在哪里**。它通常同时存在于两处——CLI 自己目录内的偏好（如 ZCode 的 `settings.models[].isDefault`）和运行时实际读的指针（如 `defaultModelSelection`）。**两处都要写，缺一就会出现「界面显示默认、运行时不用」**（见 13.1 模式十五 / #55）。

### 0.2 再定范围：做哪些能力

| 能力 | 优先级 | 参考实现 |
|------|--------|---------|
| 后端模块 + provider 管理 + 页面 | 必需 | 任意现有模块 |
| runtime_location 注册 | 必需 | `tauri/src/coding/runtime_location.rs` |
| **Allowlist 全量注册** | **必需（最高风险）** | 见第 3 节 |
| 托盘菜单 | 建议 | `tauri/src/tray.rs` + 模块 `tray_support.rs` |
| WSL/SSH 文件同步 | 建议 | `web/features/settings/hooks/useWSLSync.ts` |
| Skills / MCP 同步 | 按需 | `tauri/src/coding/tools/builtin.rs` |
| 备份 / 恢复 | 建议 | `tauri/src/settings/backup/` |
| Gateway 接管 | 可选 | claudecode / codex / kimi |
| 会话管理 | 可选 | `tauri/src/coding/session_manager/` |
| cli_resolver + 「更多选项」 | 有 CLI 调用则必需 | `tauri/src/coding/cli_resolver.rs` |

> **注意**：ZCode 集成实证表明，即使不做 Gateway 接管，`usage_stats.rs` 的读路径映射也可能需要注册（见 3.6）。

---

## 1. 阶段一：上游调研

写代码前先确认并记录以下事实，后续所有路径都从这里派生：

- [ ] CLI 名称 / npm 包名 / 可执行文件名
- [ ] 配置根目录默认值与环境变量覆盖（如 `KIMI_CODE_HOME`）
- [ ] 配置文件格式（TOML / JSON / JSONC）与主配置文件相对路径
- [ ] 是否支持自定义 provider / 官方 OAuth
- [ ] 会话存储位置与格式（决定第 10 节是否可做）
- [ ] Skills / MCP 支持情况与目录结构
- [ ] 非交互命令能力（`--version`、`session list` 等，决定 cli_resolver 探测方式）

产出格式参考 `docs/plan-kimi-code-cli.md` §1「产品概述」的事实表。

**建议**：为本工具写一份 `docs/plan-<tool>-support.md`，仿照 kimi plan 的 16 节结构。ZCode 跳过此步，结果是在收尾轮才靠排查发现多处漏注册。

---

## 2. 阶段二：后端模块骨架

新建 `tauri/src/coding/<tool>/`：

```
<tool>/
├── mod.rs            # 导出 + Tauri command 注册清单
├── types.rs          # 数据结构
├── adapter.rs        # SQLite JSONB 读写
├── constants.rs      # 默认路径、文件名常量
├── commands.rs       # apply_config_internal + Tauri commands
├── tray_support.rs   # 托盘数据获取 + apply 函数
├── projection.rs     # 配置投影（按需）
├── templates.rs      # 默认模板（按需）
├── official_accounts.rs  # 账号快照 / 切换 / 去重 / Tauri commands（按需）
├── oauth_login.rs    # 登录流程本身：发起 → 浏览器 → 轮询（按需）
└── credential_cipher.rs  # 凭证解密，仅供展示身份（按需）
```

> **官方账号不是一个文件。** 参考实现（ZCode，2026-10-06）拆成三件，因为三件事的失败代价不同：**搬运**（`official_accounts.rs`，必须字节等价，不许失败）、**登录**（`oauth_login.rs`，可失败、有超时/取消）、**展示**（`credential_cipher.rs`，失败只损失一个显示名）。混在一个文件里最容易犯的错就是拿展示用的解析结果去驱动搬运逻辑（见 13.1 模式六）。

- [ ] `tauri/src/coding/mod.rs` 加 `pub mod <tool>;`
- [ ] `tauri/src/db/schema.rs` 的 `DbTable` 枚举加表名（`<tool>_provider` / `_common_config` / `_prompt_config` / `_official_account`，按需）
- [ ] `tauri/src/db/migrations.rs` 注册建表
- [ ] 表结构遵循 `id + data(JSONB) + created_at + updated_at`，业务字段放 JSONB（普通字段增删无需 migration）
- [ ] `apply_config_internal` 带 `from_tray` 参数（托盘与主窗口共用）
- [ ] emit `config-changed` 事件，payload 区分 `"window"` / `"tray"`

### 2.1 行 id 约定：**不要含冒号**

`adapter.rs` 的读取函数默认调 `crate::coding::db_id::db_extract_id`，它会把**第一个 `:` 之前的内容当作遗留的 `table:id` 前缀剥掉**：

```rust
db_extract_id(&json!({"id": "custom:axonhub-deepseek"}))  // → "axonhub-deepseek"  ← 剥错了
```

后果是**写库成功、读回被篡改**：返回给前端的 id 与真实行 id 不一致，后续按 id 的每一次调用（投影、删除、应用、编辑）都会报 `not found`，而数据明明在表里。

ZCode 是唯一一个**业务 id 合法含冒号**的模块——它的托管供应商 id 必须以 `custom:` 开头（`ZCODE_MANAGED_PROVIDER_ID_PREFIX`），且这个 id 同时就是行 id。所以它不能用共享的 `db_extract_id`，必须在自己的 adapter 里读原始值。

**规则**：

| 情况 | 做法 |
|------|------|
| 行 id 是不含冒号的 UUID / slug | 用 `db_extract_id`（默认，保持不变） |
| 行 id 是**业务 id 且业务 id 允许含冒号** | 自己读 `value["id"]` 原始字符串，**不要**走 `db_clean_id` |
| 能让行 id 与业务 id 分离（行 id 用 UUID） | 首选。这样既保留 `db_extract_id`，又不把业务约束泄漏到主键 |

> 若选了第三种（UUID 行 id），业务 id 只存 JSONB 里，`db_extract_id` 可正常工作。ZCode 采用第二种是因为历史实现已把两者合一。
>
> **配套**：无论选哪种，都要加一条「往返」回归测试——用真实的含冒号 id 走一遍 `from_db_value_*`，断言 id 原样返回。这类 bug 单看代码读不出来。

---

## 3. 阶段三：Allowlist 全量注册（最高风险）

> **这是本 SOP 存在的核心理由。** 硬编码清单散落前后端十余处，漏一处就**静默失效**（无报错、无日志），已复发 3 次（详见根 `AGENTS.md` 与记忆库记录）。

### 3.1 权威来源（先改这里）

| 来源 | 位置 | 说明 |
|------|------|------|
| `SIDEBAR_PAGE_KEYS` | `web/services/settingsApi.ts` | 侧栏专用 key（15 个，仅 coding 工具） |
| `CURRENT_DEFAULT_VISIBLE_TABS` | `tauri/src/settings/adapter.rs` | visible_tabs 全量基线（19 个，含 gateway/image/ssh/wsl） |

### 3.1.1 设置页「模块显示」的两个列表

**设置页 → 通用设置 → 模块显示**（`web/features/settings/pages/GeneralSettingsPage.tsx`）有两行模块 chip，新增 CLI **必须加进左侧那行**：

| 常量 | 对应 UI | 内容 |
|------|---------|------|
| `CODING_TABS` | **左侧**一行 | 全部 coding 工具，**新增 CLI 加这里** |
| `OTHER_TABS` | 右侧一行 | `miniBrowser` / `gateway` / `image` / `ssh` /（Windows 再加 `wsl`） |

- `CODING_TABS` 的顺序就是 chip 默认顺序，也是拖拽排序的基准（`codingTabOrder` 的初始值由它派生）。
- 漏加的表现：新 CLI **在设置页的模块显隐列表里完全不出现**，用户无法从设置页开关或排序它（kimi 集成时踩过）。
- 右侧 `OTHER_TABS` 只在新增**非 coding** 模块时才需要动；新增 CLI 不要碰它。

#### 3.1.2 「装好了却看不见」的三层排查顺序

新 CLI 上架后用户报「设置里没有入口 / 侧边栏没有 tab」，**不要先怀疑注册清单**——按这三层查，越靠前越常见：

| 层 | 判据 | 检查方式 |
|---|---|---|
| ① 跑的是不是旧构建 | 装的是已发布包，不是本轮 worktree | 先问清「你怎么启动的」。已安装的二进制不含本轮改动，任何注册都是徒劳 |
| ② 迁移没跑到（**最隐蔽**） | `PRAGMA user_version` 已等于目标版本，但新表不存在 | 见下方命令块；命中即 `TARGET_SCHEMA_VERSION` 撞号，按模式一百一十四修 |
| ③ `visible_tabs` 是自定义顺序 | 已存值不等于任何 `PRE_*_DEFAULT_VISIBLE_TABS` 基线 | `adapter.rs` **故意**不给自定义顺序的用户强插新 tab（否则会把用户关掉的 tab 每次重开）；这类用户只能去设置页手动打开 |

```bash
# 只读打开，确认「版本已到位、结构却缺失」的形状
python - <<'PY'
import os, sqlite3
p = os.path.join(os.environ["APPDATA"], "com.ai-toolbox", "ai-toolbox.db")
c = sqlite3.connect("file:" + p.replace("\\", "/") + "?mode=ro", uri=True)
print("user_version =", c.execute("PRAGMA user_version").fetchone()[0])
print("json =", c.execute("SELECT json(data) FROM settings WHERE id='app'")
        .fetchone()[0].__class__)
tabs = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table'")]
print("新模块表 =", [t for t in tabs if "<新模块关键词>" in t])
PY
```

> 第 ③ 层不是缺陷而是既有设计（根 `AGENTS.md`：自定义顺序用户**不**被强插新 tab，新 tab 只通过 full-replace 基线浮出）。向用户解释时要说明「去 设置 → 通用 → 模块显示 打开」，并确认该 chip 确实存在（即前两层已排除）。
> **自查顺序反过来也是对的**：注册清单这层要能一次 grep 干净（§3.2 就是为此），所以它反而不该是第一个被怀疑的对象——真正会静默失效的是版本号和自定义顺序。

### 3.2 下游清单（逐一检查）

- [ ] `web/constants/modules.tsx` 的 `MODULES` subTabs —— **侧边栏显示的唯一入口**，漏了 tab 静默不显示（kimi 踩过）
- [ ] `web/features/settings/pages/GeneralSettingsPage.tsx` 的 `CODING_TABS` —— 设置页「模块显示」**左侧**列表（见 3.1.1），漏了在设置页静默消失（kimi 踩过）
- [ ] `tauri/src/settings/types.rs` 的 `AppSettings::default()`（visible_tabs）+ `default_sidebar_hidden_by_page()`
- [ ] `tauri/src/settings/adapter.rs` 的 `CURRENT_DEFAULT_VISIBLE_TABS` + 新增 `PRE_<TOOL>_DEFAULT_VISIBLE_TABS` 基线 + 匹配判断分支
- [ ] `web/services/settingsApi.ts` 的 `createDefaultSidebarHiddenByPage()` + `defaultSettings.visible_tabs`
- [ ] `tauri/src/coding/runtime_location.rs` 的 `MODULE_KEYS`（当前 14 个）+ `get_tool_skills_path_async` / `get_tool_mcp_config_path_async` 的 arm
- [ ] `tauri/src/coding/reapply_applied_runtime.rs` 的 `ALL_WSL_FILE_MODULES` + `wsl_module_for_reapply_label`
- [ ] `tauri/src/settings/backup/utils.rs` 的 `ALWAYS_BACKUP_CLI_TOOLS` / `OPTIONAL_BACKUP_CLI_TOOLS` + `tool_prefixes` + `get_custom_root_dir_path_info` + **`write_external_configs_to_backup_zip` 的打包段**
- [ ] `tauri/src/settings/backup/restore.rs` 的 **解压分支 + root-dir override 读取 + restore dir 解析**（三处，漏一处数据就丢；见 8.2-C）
- [ ] `tauri/src/coding/ssh/skills_sync.rs` 的 `SKILLS_TARGETS_FROM_RUNTIME_LOCATION`（必须 = `get_tool_skills_path_async` 的 arm 集 − hermes；见 7.2.1）
- [ ] `tauri/src/tray.rs` 的 section builders + `is_tab_visible("<tab>")` 门控
- [ ] `web/features/settings/hooks/useWSLSync.ts` / `useSSHSync.ts` 的 `TAB_TO_MODULE` + `ALL_CODING_MODULES`
- [ ] `web/features/settings/components/WSLSyncModal.tsx` / `SSHSyncModal.tsx` 的 `MODULE_TO_TAB` + `ALL_MODULE_KEYS`
- [ ] `web/features/settings/components/FileMappingModal.tsx` / `SSHFileMappingModal.tsx` 模块下拉
- [ ] `web/features/settings/utils/syncMessageTranslator.ts` 同步消息翻译
- [ ] `web/components/layout/MainLayout/index.tsx` tab 图标分支
- [ ] `web/features/coding/shared/toolIcon/ToolIcon.tsx` 图标映射

### 3.3 迁移基线规则

新增**默认可见**的 tab 时：

- 全量替换 `CURRENT_DEFAULT_VISIBLE_TABS`
- 新增 `PRE_<TOOL>_DEFAULT_VISIBLE_TABS` 快照（旧基线），让仍在使用旧默认的用户通过全量替换获得新 tab
- 自定义排序用户**有意不强制插入**新 tab
- 同步更新 `visible_tabs_*` 迁移测试期望

### 3.4 兜底验证（必做）

```bash
# 把 <tool> 换成实际 tab key，全局搜字符串，逐个确认是"有意排除"还是"漏了"
rg -n "<tool>" web/ tauri/src/ --glob '!node_modules' --glob '!*.test.*'
```

漏掉的清单不会报错——**只能靠搜**。

### 3.5 回归测试要求

这类 bug 的回归测试必须断言「新增 key 能通过**完整读链路**读回其存储值」，而不是只测默认值存在。

### 3.6 Gateway 相关清单（做接管时）

- [ ] `tauri/src/coding/proxy_gateway/types.rs` 的 `GatewayCliKey` 枚举
- [ ] `web/services/proxyGatewayApi.ts` 的 `GATEWAY_USAGE_TOOLS`（统计页筛选、请求筛选的数据源）
- [ ] `web/features/settings/pages/GatewaySettingsPanel.tsx` 的 `CLI_OPTIONS`
- [ ] `web/features/coding/gateway/components/ModelPricingModal.tsx` 的 `pricingCliKeys`
- [ ] `web/features/coding/shared/gateway/providerProfiles.ts` 的 `normalizeGatewayProviderTool`
- [ ] `tauri/src/coding/proxy_gateway/usage_stats.rs` 的 `load_provider_names`（漏注册会让已落库的请求行在列表/统计中静默丢弃）

---

## 4. 阶段四：前端页面

新建 `web/features/coding/<tool>/`（pages / components / utils），配套：

- [ ] `web/services/<tool>Api.ts`（+ `<tool>PromptApi.ts` 按需）
- [ ] `web/types/<tool>.ts`
- [ ] `web/features/coding/index.ts` 加 export
- [ ] `web/app/routeConfig.ts` 注册路由（含会话详情子路由）
- [ ] `web/i18n/locales/zh-CN.json` + `en-US.json`，用 `pnpm i18n:set-key` 生成 key
- [ ] **先做 §4.0 的「选参照 CLI + 逐项核对形态」** —— 这是本阶段最容易返工的环节，不要跳过
- [ ] **页面头部用共享组件 `CodingPageHeader`**（见 4.1）
- [ ] **供应商列表用共享组件**（见 4.2）：`ProviderListSection` 外壳 + `ProviderCard` 卡片；有模型目录的再加 `ModelListSection`
- [ ] **模型编辑弹窗用 `ModelFormModal`**，按 CLI 能力传 `show*` 开关 + `toolName`；只有字段语义/示例确实不同才用 `messageOverrides`（见 4.2.4）
- [ ] ⚠️ **模型弹窗必须有「选择预设模型」入口**——最重要的能力之一，**不许漏**（见 4.2.4）
- [ ] **供应商编辑弹窗的分区用 `ProviderFormSections`**（见 4.2.6）；有协议下拉时接网关支持门控（见 4.2.7）
- [ ] **全局提示词区块用 `GlobalPromptSettings`**，传 `promptFileName`（见 4.2.8）
- [ ] UI 遵循 `DESIGN.md`（改任何可见 UI 前必须先完整阅读）

### 4.0 迁移前必做：选参照 CLI + 逐项核对形态

> **这一节是本 SOP 最重要的一节。** 以下是实际踩过的教训：迁移共享组件时，最容易犯的错误是把「换成共享组件」理解成「替换组件引用」。正确理解是「**对齐参照 CLI 的形态**」——组件只是承载形态的容器。只换组件、不对齐形态，结果是「用了共享组件但长得像另一个产品」。

#### 4.0.1 第一步：为每类界面指定一个参照 CLI

不要凭印象，先明确「照着谁改」：

| 界面 | 参照 | 理由 |
|------|------|------|
| 供应商列表 / 卡片 | **Codex** | 功能最全：模型目录 + 网关门控 + 批量操作 |
| 供应商编辑弹窗 | **Codex** | 含渠道行、协议门控、完整分区 |
| 模型编辑弹窗 | **Pi / Hermes** | 可选字段最多，`show*` 覆盖面最广 |
| 页面头部 | **Codex** | 首个消费方，无遗留覆盖 prop |
| 模型列表工具栏 | **Codex** | 传全了全部 handler |
| **模型/供应商字段形状** | **同引擎的兄弟** | ⚠️ 不是「名字像的」。先确认运行时同源（读同一个文件？同一个二进制？），再抄它的字段；最终**以该引擎自己的 `docs/models.md` 为准**——兄弟页面也可能是错的（13.1 模式二十三） |

#### 4.0.2 第二步：逐项核对（机械清单，不许凭印象）

打开参照文件逐行读，**不是**「我记得应该差不多」。ZCode 迁移时漏掉的每一项都在下面：

**A. 外层容器（共享组件管不到的部分）**

- [ ] `layout` / `labelCol` / `wrapperCol` —— **共享组件只管内部，外层布局仍由调用方写**。
  > ZCode 保留了 `layout="vertical"`（标签在输入框上方），而其他 5 个 CLI 全是 `layout="horizontal"` + `labelCol` / `wrapperCol`。弹窗看起来像另一个产品。
  >
  > **同一个错误在 ZCode 犯了两次**：先是供应商弹窗，后是模型弹窗。**每一个 `<Form>` 都要单独核对**——供应商弹窗改对了不代表模型弹窗也对。
  >
  > `labelCol` 的 span 各 CLI 不同（shared 4/6、codex 6/8、grok 5/7、kimi 7/9、openclaw 5/7），**按本 CLI 最长的标签选**，不是照抄。**标签改了就要重算**：ZCode 原用 7/9，是因为最长标签是「推理等级（从低到高）」；标签简化为「推理等级」后改成 6/8。
- [ ] `width` / `title` / `okText` / `cancelText` / `destroyOnHidden`
- [ ] **弹窗的「进行中」状态用正文 `Spin` 表达，不要用 `okButtonProps={{ loading: true }}`。**
  > Ant Design 的 `loading` 会给按钮加 `pointer-events: none`，**同时锁死取消按钮**——用户看到的是一个变灰、点不动的「取消」，登录/提交卡住时无法退出（ZCode OAuth 登录弹窗踩过）。
  >
  > 正确做法：`okButtonProps` 只设 `disabled`，把进行中的视觉反馈放进弹窗正文（`<Spin>` 或局部 loading），保证「取消」始终可点。**任何有长耗时操作的弹窗都要单独验一次「操作进行中点取消」**。
- [ ] 表单字段的**顺序与数量**：逐字段列出「参照有我没有」「我有参照没有」，确认每一处差异都是**有意的**
- [ ] **分组子字段**（如「一组三态下拉」）：横向布局下内层 `Form.Item` 若带 `label` 会继承外层 labelCol 百分比，布局错乱。改成「纯文本子标签 + 无 label 的 `Form.Item`（`labelCol={{span:0}}` / `wrapperCol={{span:24}}`）」，既保持分组又保留校验错误显示。
  > **何时可以用 `noStyle`**：分组内**没有**逐字段校验时（如「模型能力」勾选组——布尔字段永远有值，不存在「漏填」）用 `noStyle` 是正确的，它让勾选框自然横向排列。**有** `rules` 的字段必须留出 `wrapperCol`，否则校验错误无处渲染，手动模式漏填会「点了保存没反应」（13.1 模式三）。判断依据是**该字段有没有 rules**，不是「在不在分组里」。

**B. 第一行放什么**

- [ ] Codex 供应商表单**第一行是「渠道」**：左边渠道选择器 + 右边格式选择器，**同一行**（grid 两列），下方跟 hint。
  > ZCode 把渠道放在第 3、4 个字段且分成两行，用户第一眼看到的是「名称」而不是「选渠道」。

**C. 每个字段的交互细节**

- [ ] API Key 是否有**显示/隐藏按钮**。
  > ⚠️ **antd 6 的 `Input.Password` 自带显示/隐藏按钮**（`visibilityToggle` 默认 `true`，见 `antd/es/input/Password.js`），所以「用了 `Input.Password`」本身就满足这条，不需要额外 `addonAfter`。这条核对的是**结果**（用户能不能切换明文），不是写法。若某处 `visibilityToggle={false}` 或有自定义 `iconRender`，才需要单独确认。
- [ ] 是否该用 `ImeSafeInput` / `ImeSafeAutoComplete`（IME 组合输入安全）
- [ ] hint 走 `help` 还是 `extra`，字号是 11 还是 12
- [ ] **说明性文字放在它所说明的控件下方**（输入框 / 编辑器之后），不是上方。
  > 放在上方时它读起来像一条需要先关掉的横幅；放在下方才是「这个框里该写什么」的说明。ZCode 通用配置的 `description` 原本在 `JsonEditor` **上方**，参照 Codex 挪到下方后才读得通。
  >
  > 通用判据：**「解释这个控件」的文字跟着控件走**（下方）；**「警告/阻断」类信息才置顶**（错误、不兼容提示）。核对方法：把弹窗从上往下读一遍，每段文字问一次「它在解释谁」——解释不到任何控件的，就是放错了位置。
- [ ] **含义不自明的字段，标题后有问号，hover 显示说明**——用 `components/common/FieldHelp`（**通用组件，不要各 CLI 重写**）。
  > 判据：**遮住说明、只看标签，用户能填对吗？** 「上下文窗口」「名称」能；「推理参数映射」「模型能力」「输入类型」不能。给每个字段都挂问号会让界面变吵。
  >
  > **文案优先逐字取自上游**的对应帮助文本（ZCode 取自 `settings.modelProvider.help.*`）——自己重写一遍会与官方文档产生第二套说法，用户对照官方文档时对不上。
  >
  > ⚠️ **文案是纯字符串，不能直接丢给 `Tooltip title`**：`\n` 不换行、`**粗体**` 会原样显示星号，整段挤成一行。`FieldHelp` 已处理分段/列表/粗体/行内代码（见其模块 `AGENTS.md`）。
- [ ] **长文本输入用多行编辑器**，不是单行 `Input`。
  > 判据：**内容里会不会出现换行或嵌套括号？** 会 → `Input.TextArea`（`autoSize`）。JSON / CEL / 表达式类字段一律多行；单行只用于确实是一行的值（URL、key、名称）。
  >
  > ZCode 的「推理参数映射」是带嵌套花括号的 CEL 表达式，单行输入框里读不成句、改不动。

**D. 下拉的选项列表**

- [ ] 是否有**显式的「自定义」选项**，还是靠 placeholder 暗示。
  > **不要用 `allowClear` + placeholder 表达「不选」。** Codex 给「自定义」一个具名选项（`CUSTOM_PROVIDER_ENDPOINT_KEY`），语义明确；ZCode 用 `allowClear` + 「不使用模板」placeholder，用户看不出「留空 = 自定义」。
- [ ] 选项数据源、`showSearch` / `allowClear` 是否与参照一致

**E. 卡片的详情区**

- [ ] Codex 是**一行**：`baseUrl` + 格式 Tag + API Key + 备注，用 `|` 分隔。
  > ZCode 渲染了**三行**（id / baseUrl / 备注）。注意参照不显示 provider id —— 对自动生成的 id 它只是名称的 slug 副本。
- [ ] **操作按钮的位置：贴着它作用的那个对象**，不要集中放在卡片顶部。
  > 一个按钮作用在哪一行/哪个条目，就渲染在那一行/那个条目的操作区里。放在卡片顶部时，用户要先读按钮文案才知道它作用于谁，条件渲染（「只在这一行满足条件时显示」）也会让顶部多出一个含义不明的按钮。
  >
  > ZCode 的「保存当前登录」原本在卡片顶部，而它只对 `isVirtual`（未保存的 live 登录）那一条有效——用户看不出它作用于哪一行。移到那一行的操作区后，**「它作用的条目就是携带它的条目」**，不需要解释。
  >
  > 判据：**按钮的作用域如果是「某一行」，它就该在那一行里**；只有作用域是「整张卡片」的操作（编辑、删除、应用）才放头部。
- [ ] 操作按钮的数量、**样式**（`type="link"` / `type="text"` / default）、图标、禁用条件
  > ZCode 的「应用」用了 default 按钮（带边框），其他 CLI 全是 `type="link"`（蓝色文字）。
  >
  > **卡片内的次要操作一律无边框**（`type="text"` + `style={{ fontSize: 12 }}`），与模型列表右侧一致；带边框的按钮在一排图标里视觉过重，会抢走名称的注意力。**这一排的密度是统一判据**：数一数参照卡片上每个按钮的边框，多一个带框的按钮就是不一致。
  >
  > **同一个动作在所有 CLI 上必须同款**：如果「删除」在某 CLI 是图标、在另一个是文字链，用户就要重新学一遍。跨 CLI 的一致性优先于单页美观——这也是 §4.2.1 把三种卡片样式固定的原因之一。

**F. 三层「可选项」台账（最容易漏的一类）**

共享组件全部用「**传了才渲染**」组织 UI：漏传一个 prop → 少一个按钮 / 分区 / 插槽，**没有任何报错**，看起来像设计如此（13.1 模式二）。ZCode 在这三层**各漏过**，且都在界面上「看起来正常」。

**核对方法**：打开 **附录 B** 的三张表，**每一行**都在 PR 描述里写「传 / 有意不传 + 理由」。不许整表跳过——跳过的行下次就会被当成「设计如此」。

- [ ] **F-1 组件可选 prop**（附录 B.1）：`ProviderListSection` / `ModelListSection` / `CodingPageHeader` / `ProviderFormSections` / `ModelFormModal` 的**每一个**可选 prop
- [ ] **F-2 页面级区块**（附录 B.2）：不属于任何组件、靠对照参照页面逐块点的部分（提示块、导入入口、页面级 Alert…）
- [ ] **F-3 能力**（见 H）：组件 prop 之外的功能缺口

> **ZCode 实证（三处，2026-10-06 全部补齐）**：
>
> | 漏传 | 界面表现 |
> |------|---------|
> | `ModelListSection.onCopyModel` | 模型行没有「复制」按钮，其余 7 个 CLI 都有 |
> | `CodingPageHeader.onPreviewConfig` | 页面没有「预览配置」；§4.1 早就记过这条，迁移后仍未补 |
> | `ProviderListSection.hint` / `footer` | 提示块与「导入我使用过的供应商」整体缺失 |
>
> 三处的共同点：**组件本身没坏、页面也没报错**，只有把参照 CLI 打开逐项指认才会发现。它们是在**做完附录 B 台账之后**才被一起发现的——说明这份台账不能等到收尾再填。

**G. 状态与空态**

- [ ] 空态 / 搜索空态 / 加载态 / 禁用态文案是否与参照一致

**H. 能力核对（不是样式，样式核对抓不到）**

逐项确认参照 CLI 有的**能力**本 CLI 也有。漏能力不会让界面「长得不对」，只会让界面「少了功能」，所以必须单独列：

- [ ] **模型编辑弹窗有「选择预设模型」入口** —— ⚠️ **最重要的能力之一，不许漏**（见 4.2.4）
- [ ] 模型列表有「获取模型」（从上游 API 拉取），**且应用时按 model id 匹配预设自动填充**（见 4.2.5）
  > ⚠️ **这条在 OmO Native 上重犯过一次**（2026-10-07）：入口有、能跑通、不报错，只是「应用」后拿到的是裸 id——与 ZCode 完全同一个缺陷。**入口存在 ≠ 能力完成**，判据是「应用后那一行有没有带上参数」
- [ ] 模型列表有连通性测试
- [ ] 供应商有连通性测试 / 批量测试
- [ ] **「默认渠道」的删除按钮置灰，并排除出批量选择**
  > 判据：**默认指针是不是写在 provider 记录之外**（settings/config 里的 `defaultProvider` / `model`）？是 → 适用（pi / ohMyPi / dsh / hermes / openclaw / opencode / omo_native）。Claude 式（claudecode / codex / gemini / grok / kimi）是「应用」语义，不适用。
  >
  > **OmO Native 2026-10-08 才补上**（第 113 条）：规则在 Pi 上早就有，但本清单从没列过它——卡片能力集没枚举，新建的卡片就静默少一条（模式二十七 / 90 的又一例）。只置灰单个按钮不算完成：`batchSelectableIds` 也要把它排除，否则「全选 → 批量删除」能绕过。
- [ ] 有「导入我使用过的供应商」或等价导入入口
- [ ] 有全局提示词区块
- [ ] 有会话管理面板（若该 CLI 有会话概念）
- [ ] 有「预览配置」「打开文件夹」「刷新配置」

> 对照方法：打开参照 CLI 的页面**逐个点一遍**，列出它有的入口；回到本 CLI 页面逐个找。找不到的要么补上，要么在提交说明里写明「有意不做 + 理由」。

**I. 页面区块清单（先「取并集」再「逐块删」）**

> ⚠️ **这一节在 2026-10-07 被推翻重写过一次。** 原版写的是「默认三区块，除此外不许有别的」——
> 那条规则**本身是错的**，而且它导致了一个更严重的后果：按它收敛完 OmO Native，
> 用户立刻指出缺了「其他配置」和「官方认证渠道」，而这两块在 **7 个已有页面**上都有。
> 详见 13.1 模式二十六。

**做法：从「全部已有页面实际有什么」取并集，再逐块决定留不留。**

先跑这条命令，把**现状**列出来（不要凭印象，也不要只读一个参照页）：

```bash
# 每个 coding 页面的区块 id（去掉 CLI 前缀，只看功能名）
for f in web/features/coding/*/pages/*.tsx; do
  echo "== $(basename $f .tsx)"
  grep -oE "id: '[a-z-]+'" "$f" | sed "s/id: '//;s/'//" | sort -u
done
```

**截至 2026-10-07 的全仓并集**（这才是「标准区块」的事实来源）：

| 区块 | 有它的页面数 | 判定 |
|---|---|---|
| 供应商列表 | 15 | **必备** |
| 全局提示词 | 15 | **必备** |
| 会话管理 | 13 | 必备（有会话概念就有） |
| **其他配置** | **7**（opencode / pi / oh_my_pi / dsh / hermes / openclaw / kimi） | **配置型 CLI 基本都要**——它承载「本页不专门管理、但属于同一份配置」的键 |
| **官方认证渠道** | **5**（opencode / codex / grok / geminicli / antigravity） | 有官方 OAuth/账号体系的 CLI 都要 |
| 插件 / 扩展 | 5 | 该 CLI 有插件体系时 |
| 配置目录 / 设置 | 6 | 见 §11 |
| 记忆 / agents / 工具 / env | 各 1–3 | 单页专属，不要推广 |

- [ ] **先跑上面的命令，把并集列出来**。**不许只对照一个「参照 CLI」就下结论**——参照 CLI 自己也可能缺块（Codex 就没有「其他配置」，但那不代表新 CLI 不该有）。
- [ ] **逐块回答「本 CLI 有没有这类数据」**，而不是「别的 tab 有没有」：
      「其他配置」问的是**这份配置文件里有没有本页不管理的键**；「官方认证渠道」问的是**这个 CLI 有没有官方登录/OAuth**。有数据就加，没有才不加——**并写进模块 `AGENTS.md` 说明为什么不加**。
- [ ] 只有**确实回答不了上面两问**的区块才删，并在提交说明里写明理由（13.1 模式二十一）。
- [ ] 侧栏 `sections` 标记与页面区块**一一对应**，没有指向已删区块的残留项。
- [ ] 撤下某个区块时**只删 UI**：命令、DB 表、托盘接线、备份恢复链路全部保留，并在模块 `AGENTS.md` 写明「无 UI 入口但后端保留」（判据见 §12.1 的死封装审计）。

> **实证（正反两面）**：
> - **该删的**：OmO Native 曾有「生效配置」（与页头「预览配置」重复）、「更多选项」（⋯ 弹窗里已有同一份）、「MCP 与 Skills」（只是一段指向别处的指引）——这三块删得对。
> - **不该删的**：同一轮把「其他配置」和「官方认证渠道」也一并当成「Native 专属」删掉了，而这两块在 7 / 5 个已有页面上都有。用户看到别的 tab 有、OmO 没有，立刻指了出来。**「这个区块只有本 CLI 有」不等于「本 CLI 不需要它」**——前者是独特性，后者是缺失，两者在页面上长得一模一样。

#### 4.0.3 第三步：核对共享组件的**前置条件**

共享组件不是无条件适用的，用之前先确认它的前提：

- [ ] **分区可能只对部分 CLI 有效。** `ProviderFormSections` 的**计费 / 自定义请求头 / 模型改写三块只由本地网关消费**（`inject_custom_headers` / `resolve_upstream_model_id` 是唯一读者）。CLI 不在 `GatewayCliKey::supported_mvp()` 里就必须传 `show*=false`。
  > ZCode 不在网关支持列表，三个分区填了没有任何代码读；更糟的是表单还把值写进 `meta`，制造了「配置已生效」的假象。
  >
  > **推论**：看到共享组件有 `meta` 合并 helper（`merge*IntoMeta`），先确认「谁读这个 key」，再决定是否调用。

- [ ] **组件可能暗示了不属于本 CLI 的概念。** 例如 billing 意味着「该 CLI 的请求会经过网关计费」。

#### 4.0.4 第四步：验证方式

**「编译通过」不等于「形态对了」。** 编译只保证类型正确，不保证长得对。

- [ ] 对照参照 CLI **打开实际界面比对**（截图对比最有效）
- [ ] 逐条走 4.0.2 的清单，每项在界面上指认一次
- [ ] 若无法运行界面，**在 PR/提交说明里明确写出「未做视觉核对」**，不要默认通过

> ZCode 这一轮累计返工 8 次（布局 / API Key 按钮 / 渠道行位置 / 自定义选项 / 网关门控 / 卡片详情行 / 工具栏按钮 / 静默错误），全部属于「编译通过但形态不对」——若第一步就做了逐项核对，这些都不会发生。

### 4.1 页面头部标准（`CodingPageHeader`）

**所有 coding tab 的页面头部必须用共享组件**，不要照抄隔壁页面。组件在 `web/features/coding/shared/CodingPageHeader.tsx`，由 Codex 页（2026-10-06）作为首个消费方验证；Claude Code 页 2026-10-07 接入。

> **迁移一个页面时，先核对它现在用的 key 是否与 `common.*` 同义**。Claude Code 页迁入时核了 5 组，中文 5/5 完全相同，英文有 3 处措辞差异（`Documentation` → `Official Docs`、`Configuration File Path` → `Config Path`、`Customize Root Directory` → `Customize Config Dir`）——按 Codex 的先例统一取 `common.*`，并把变化写进提交说明。**不要为了保住旧措辞而给组件加覆盖 prop**：那正是 §4.1 记录的、已被删除的 5 个 prop 的老路。
>
> 迁完删掉孤儿 key（`viewDocs` / `configPath` / `openFolder` / `refreshConfig` / `rootPathSource.customize`）。**注意 `rootPathSource.modal.*` 不在此列**——`RootDirectoryModal` 与各 CLI 的 plugins 面板仍在用，误删会让弹窗渲染出字面量 key。

**为什么有这个组件**：此前 14 个页面各写各的头部，结构一致但代码分散，新工具接入只能靠"复制隔壁 + 手改"，容易漏项（zcode 漏了「预览配置」、claudedesktop 文案硬编码中文未走 i18n）。

> **ZCode 的教训（已修 2026-10-06）**：迁到 `CodingPageHeader` 之后，头部仍只传了 7 个 prop，`onPreviewConfig` 一直是 `undefined` → 页面没有「预览配置」，直到做附录 B 台账时才被发现。**迁到共享组件不等于补齐了缺失的能力**——组件换对了，漏传的 prop 依然静默。

**标准形态**：

```
┌──────────────────────────────────────────────────────────────┐
│ <标题>  🔗官方文档  👁预览配置              ⋯ 更多选项        │
│ 配置文件路径: [<path>] ✎自定义配置目录 📁打开文件夹 ⟳刷新配置  │
└──────────────────────────────────────────────────────────────┘
```

**最小用法**（新工具接入时直接照抄，改 3 处即可）：

```tsx
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';

<CodingPageHeader
  title={t('<tool>.title')}
  docsUrl="https://<上游文档地址>"
  configPath={configPath || '~/.<tool>/config.toml'}  // 换成该 CLI 的默认路径
  onPreviewConfig={appliedProviderId ? handlePreviewCurrentConfig : undefined}
  onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
  onOpenFolder={handleOpenFolder}
  onRefresh={handleRefreshPage}
  onMoreOptions={() => setSettingsModalOpen(true)}
/>
```

**props 说明**：

| prop | 说明 |
|------|------|
| `title` | 页面标题，传 `t('<tool>.title')` |
| `docsUrl` | 官方文档地址；**不传则不显示**该链接 |
| `onPreviewConfig` | 传了才显示「预览配置」；无 provider 应用态时可传 `undefined` |
| `configPath` | 路径字符串，**调用方自己给兜底值** |
| `configPathKind` | `'file'`（默认）或 `'directory'`——决定标签取 `common.configPath` 还是 `common.configDir`。**管理根目录而非单个文件的 CLI 要传 `'directory'`**（pi / oh_my_pi / dsh / hermes / omo_native） |
| `onCustomizeConfig` | 传了才显示「自定义配置目录」 |
| `customizeConfigDisabled` / `customizeConfigDisabledReason` | 置灰「自定义配置目录」；**两个一起传**——禁用态按钮不接收指针事件，组件用外层 `<span>` 承载 Tooltip，只置灰不传原因会读成坏按钮（opencode 的 V2 迁移场景） |
| `onOpenFolder` | 传了才显示「打开文件夹」 |
| `onRefresh` | 传了才显示「刷新配置」 |
| `onMoreOptions` | 传了才显示「更多选项」 |
| `extraActions` | 追加额外文字按钮（openclaw 的「打开 Web UI」、opencode 的「同步模型」） |

> **没有 `hint` prop**（2026-10-07 删除）。页面级提示块统一放 `ProviderListSection` 的 `hint`，与 Codex / ZCode / Claude Code 同位置；原先头部这个 `hint` 只有 omo_native 一个调用方，且组件裸渲染它、把样式留给调用方手抄，照抄的那份漏了样式。

**文案一律走 `common.*`，没有 per-CLI 覆盖 prop**：`viewDocs` / `configPath` / `customizeConfigDir` / `openFolder` / `refreshConfig` / `previewConfig` / `moreOptions`。

> **为什么不做覆盖 prop（踩过的坑）**
>
> 组件最初提供了 `docsText` / `configPathLabel` / `customizeConfigText` / `openFolderText` / `refreshText` 五个覆盖项，理由是要"保持既有中英文文案不变"。实际统计 14 个页面后发现这些"差异"全是同义异写，没有一处是真实语义差别：
>
> | 文案 | 实际分布 |
> |------|---------|
> | 打开文件夹 | **14/14 完全相同** |
> | 刷新配置 | 12 个「刷新配置」，zcode/geminicli 是「刷新」 |
> | 官方文档 | 「官方文档」 vs 「查看文档」 |
> | 自定义配置目录 | 「自定义配置目录」 vs zcode「自定义根目录」 |
>
> 覆盖 prop 不但没保住什么，反而让 zcode 的「刷新」这种不一致固化了下来。**已全部删除**，组件只用 `common.*`；codex/zcode 的 8 个孤儿 key 一并 prune。
>
> **唯一保留的例外（已落地，2026-10-22）**：`pi` / `ohMyPi` / `dsh` / `hermes` / `omo_native` 显示的是**目录**（`rootPathInfo.path`）而非配置文件路径。这是真实语义差异，迁移时没有抹平，也没有加自由文案覆盖，而是加了一个**枚举** prop：`configPathKind?: 'file' | 'directory'`（默认 `'file'`）。两份标签都在 `common.*`（`common.configPath`「配置文件路径」/ `common.configDir`「配置目录路径」），**调用方只声明「我这个路径是哪种东西」**——自由文案 prop 是当年被删掉的那五个（见上），枚举则不会重新长出一堆同义异写。
>
> **教训**：不要为了"迁移时不动文案"而预先加覆盖 prop。先把各页面的值统计出来，同义的直接统一，只有真正不同的才参数化。

### 4.2 供应商列表标准（`ProviderListSection` / `providerCardVariants` / `ModelListSection`）

供应商列表按 CLI 能力分**两种形态**，但共用同一套外壳。

| 形态 | 代表 | 特征 |
|------|------|------|
| **不支持自定义模型** | Claude Code | 卡片只有字段区（Haiku/Sonnet/Opus 等），无模型列表 |
| **支持自定义模型** | Codex | 卡片内多一个「模型列表 (N)」区（常开，不是折叠区） |

#### 4.2.1 卡片样式：固定三种，不再新增（Hard Rule 14）

**新建 CLI 时从下面三种里选一种，不要新写 `*ProviderCard.tsx` 布局。** 选型依据是 provider 模型形状（§0.1.1 的两个定性），不是个人偏好：

| 样式组件 | 何时用 | 判定问题 | 第二行 | 头部主操作 | 已接入 |
|---|---|---|---|---|---|
| `ClaudeStyleCard` | 没有模型目录（模型写在 provider 配置里） | CLI 是否管理一个模型清单？否 → 此样式 | 带标签的绑定（`默认: …` / `Haiku: …`） | 文字链「应用」 | claudecode、geminicli、claudedesktop |
| `CodexStyleCard` | 有模型目录，且有单一 active provider | 是，且「应用」有意义 → 此样式 | 自由格式（端点/模型/masked key/备注） | 文字链「应用」 | codex、kimi、grok |
| `OpenCodeStyleCard` | 有模型目录，且**没有**单一 active provider | 是，且「启用哪个渠道」不成立 → 此样式 | 固定顺序 `ID • SDK • 端点` | **无**（默认在模型行上选） | zcode、omo_native、opencode |

> **「已接入」这一列在 2026-10-07 之前是空的**：三种样式里只有 `OpenCodeStyleCard` 有消费方，而 `ClaudeStyleCard` / `CodexStyleCard` 零消费方——它们各自最典型的对应 CLI（Claude Code / Codex）反而持 776 / 1134 行的 bespoke 卡片。规则写在 Hard Rule 14 里，**没有任何检查能发现规则被绕过**。现已由 `scripts/verify-provider-card-layout.mjs` 机械守护（见下）。

**关键区分：OpenCode 式没有头部「应用」按钮**，因为该 CLI 里多个渠道同时可用，「应用」会误导用户以为其他渠道被关掉了。这类 CLI 的「用哪个」表达在**模型行**的「设为默认」上。

三种样式都在 `web/features/coding/shared/providerCardVariants/`，共用一个 `CardShell`（拖拽注册、选中复选框、卡片外框）。**通用行为改动（间距、拖拽、选中态、按钮样式）改在变体组件里**，一处生效于所有使用该样式的 CLI；只在某个 CLI 的卡片里改会让样式重新分叉，这正是本轮重构要消除的问题。

`CardShell` 的两个约束：`useSortable` 每次渲染都必须执行（无 id 时传占位符并 `disabled`，不要条件调用 hook）；`setNodeRef` **无条件**挂载（dnd-kit 需要测量节点，未注册的节点在父级重新启用拖拽时无法拖动）。

**守卫（必跑）**：`pnpm run test:provider-card-layout`。它扫 `web/features/coding/<cli>/components/*ProviderCard.tsx` **与 `*OfficialAccountCard.tsx`**（作用域是**后缀集合**，不是单个后缀——见 13.1 模式七十七：ZCode 的官方账号卡片同为列表成员，却因为名字不匹配从未被扫到），禁止映射层自己 `useSortable` / 自己渲染 `<Card>` / 自己写 `ManagementCheckbox`。`PENDING_MIGRATION` **已于 2026-10-20 清空**（最后三家 claudedesktop / geminicli / grok 迁完），此后新出现的未迁移卡片直接报错，不再有登记缓冲。**2026-10-21 补上第四条规则**：受管映射层不得从 `@/components/common/ProviderCard` 导入组件——前三条规则查的是「自建 chrome」，而旧共享卡既不是 chrome 也不触发任何一条，`OpenClawProviderCard` 因此包着它数月未被发现。类型仍可从 `@/components/common/ProviderCard/types` 导入。脚本的 `EXEMPT_FILES` 只留一个「名字像但不是列表卡片」的例外（`AntigravityProviderCard`，理由写在条目里）；**读不到的豁免是假豁免**——`ZcodeOfficialAccountCard` 曾有一条（理由也早已过期），因为后缀不匹配从来没被读过。

**样式能力不够时怎么办**：先确认**至少两个**调用方需要这个能力，再给 `ProviderCardVariantProps` 加 prop。实例：网关接管按钮（`actions.gatewayActions`）的处理器在 claudecode / codex / grok / kimi / claudedesktop **五张卡片里逐字重复**（实测 diff 只有 `'claude'` vs `'codex'` 一行不同），补这个插槽是本轮唯一改共享组件的地方。反之，只有一个调用方需要的能力留在调用方自己的映射层里，不要塞进共享 props。

> **例外：外壳的「插槽完备性」不适用这个门槛**（2026-10-08 补）。`ProviderListSection` / `CodingPageHeader` 这类**外壳**组件，判据不是「几个调用方要这个能力」，而是「**去掉这个插槽，调用方要不要把整段外壳抄回来？**」要 → 单消费方也该加。外壳的插槽不产生行为，只是让已有行为有地方待着；缺一个位置就会逼出一次分叉，正是外壳要消除的东西。
>
> **但先判身份，再选插槽**（2026-10-08 二次修正）。这条例外只回答「外壳该不该留这个位置」，不回答「这个东西本来就该在那吗」。Kimi 的官方账号区当时按这条例外新增了 `toolbarExtra`，事后发现**判错了身份**：它不是工具栏动作、也不是页底区块，而是**列表成员**——与供应商卡片平级、要能拖动排序。正确落点是调用方的 `children` + `alwaysVisible`，参照同仓库已有的 ZCode 官方账号卡片（#42 / #61）。`toolbarExtra` 随最后消费方一起删除。判据：**「用户会不会期望它能被拖动 / 和供应商卡片并排？」** 会 → 它是列表成员，外壳插槽都是错的答案。见 13.1 模式五十五。

**三个组件**（均在 `web/features/coding/shared/` 或 `web/components/common/`）：

| 组件 | 路径 | 职责 |
|------|------|------|
| `ProviderListSection` | `shared/ProviderListSection.tsx` | 区域外壳：标题 + 工具栏 + 提示 + 空态 + 底部导入 |
| `ProviderCard` | `components/common/ProviderCard/` | **旧卡片，已于 2026-10-22 随 pi / oh_my_pi / dsh 迁完而删除**。目录里只剩 `types.ts`（`ModelDisplayData` 等共享类型仍被 `ModelListSection` / `ModelItem` 使用）。已迁移的映射层禁止 import 旧卡，见 §4.2.1 的守卫 |
| `ModelListSection` | `shared/ModelListSection.tsx` | 模型列表区，两种形态按 `appearance` 选（列表卡片折叠 / 明细侧栏常开，见 §4.2.3；仅"支持自定义模型"形态用） |

#### 4.2.1 区域外壳 `ProviderListSection`

**标准形态**：

```
┌────────────────────────────────────────────────────────────────────┐
│ 📋 供应商列表  [Gateway 胶囊]  ☑多选 🔍搜索 ⇅排序 ◎定位 ⚡一键测试 品通用配置 +添加供应商 │
├────────────────────────────────────────────────────────────────────┤
│ ▎可添加多套供应商配置并通过「应用」或系统托盘快捷菜单快速切换。…      │  ← hint
│ ▎注意：配置存储在应用数据库中，请勿手动修改本地配置文件，…           │
│                                                                    │
│  [ 供应商卡片 … ]                                                   │
│                                                                    │
│  [从 CC Switch 导入] [导入我使用过的供应商] [从 All API Hub 导入]     │  ← footer
└────────────────────────────────────────────────────────────────────┘
```

**最小用法**：

```tsx
import ProviderListSection from '@/features/coding/shared/ProviderListSection';

<ProviderListSection
  sectionId="<tool>-providers"
  collapsed={providerListCollapsed}
  onCollapsedChange={setProviderListCollapsed}
  loading={loading}
  providerCount={providers.length}
  visibleCount={visibleProviders.length}
  batch={providerBatch}                      // useProviderBatchSelection 的返回值
  batchSelectableIds={batchSelectableIds}
  keyword={providerKeyword}
  onKeywordChange={setProviderKeyword}
  sortMode={sortMode}
  sortModes={PROVIDER_SORT_MODES}
  onSortModeChange={setSortMode}
  viewMode={viewMode}                        // 可选：与 onViewModeChange 成对，两个都传才渲染布局切换
  onViewModeChange={setViewMode}
  locateProviderId={appliedProviderId}       // 传了才渲染「定位」；无「单一已应用供应商」的 CLI 不传
  onBatchTest={handleBatchTestProviders}
  batchTesting={batchTestingProviders}
  onOpenCommonConfig={() => setCommonConfigModalOpen(true)}
  onAddProvider={handleAddProvider}
  headerExtra={<GatewayFailoverButton ... />} // 可选：Gateway 胶囊等
  emptyTextHint={t('<tool>.importFromX')}      // 可选：仅当能从此处导入时
  hint={<><div>…第一行提示…</div><div>…第二行…</div></>}  // 只传文案，样式在组件里
  footer={<Space wrap>…三个导入按钮…</Space>}  // 可选
>
  {/* 卡片列表（含 DndContext） */}
</ProviderListSection>
```

**关键 props**：

| prop | 说明 |
|------|------|
| `sectionId` | sidebar 锚点 id，如 `<tool>-providers` |
| `batch` / `batchSelectableIds` | 供应商级多选状态（`useProviderBatchSelection`） |
| `headerExtra` | 插槽：渲染在标题旁（Gateway 的 Failover / Aggregate 胶囊走这里） |
| `emptyTextHint` | 追加在通用空态文案**下方**的一句补充，仅用于说明导入来源 |
| `hint` / `footer` | 提示块 / 底部导入按钮；**只传文案，组件不硬编码**。⚠️ `hint` 的**样式由组件负责**（12px / 次要色 / 左侧竖线），调用方**不要**自带 `style`——手抄会漏（13.1 模式二十四） |
| `onBatchTest` / `onOpenCommonConfig` | 传了才渲染对应按钮 |
| `locateProviderId` | 当前已应用供应商的 id → 渲染标题右侧的「定位」按钮：滚到那张卡片并闪光（卡片侧契约是 `CardShell` 的 `data-provider-id`）。传 `''`（当前没有已应用）仍渲染按钮，点击说明原因；**不传则完全没有这个按钮**——没有「单一已应用供应商」概念的 CLI（omo_native：多个 provider 同时生效）就该不传 |
| `viewMode` / `onViewModeChange` | 布局切换（列表 / 卡片）。**两个都传才渲染**：卡片形态是 `shared/providerCardView` 提供的（接入方式见该模块 `AGENTS.md`），没接的 CLI 不传，免得切到一个不存在的布局。控件是**一个「图标 + 文字」按钮**，放在**一键测试之前**（与定位/搜索/排序同组：那组管「列表本身怎么摆」，右边那组是「对 provider 做什么」，2026-10-12 用户指定）；`data-provider-view-toggle` 带当前模式，**图标与文字都表示「点了会切到哪种布局」**（文字是目标布局名），tooltip 与 `aria-label` 用整句说同一件事，不是分段控件。图标必须避开「通用配置」的 `AppstoreOutlined`——antd 的 `LayoutOutlined` / `UnorderedListOutlined`，**不要用 lucide 的 `LayoutGrid`（2×2 方块，同形）** |

> ⚠️ **`hint` 与 `footer` 是最容易整体漏掉的两个插槽**：ZCode 两个都没传，于是提示块和「导入我使用过的供应商」**整块消失**，而界面看起来完全正常；已迁移的两个试点（claudecode / codex）都传了。`hint` 是两行（`pageHint` + `pageWarning`），`footer` 含三个导入源。
>
> **接 `footer` 时先确认本 CLI 有没有 favorites 存储的接入**——ZCode 完全没有，于是连带「删除前备份」这个共享 helper 也被传了个空 writer（`async () => undefined`），**批量删除实际不备份**，而 `backupFailed` 提示永远不会触发。这类「helper 传了但传的是空实现」比漏传更难发现：调用点看起来是齐的。
>
> 13 个 coding 页面里只有 10 个有「导入我使用过的供应商」入口，kimi / geminicli / antigravity / omo_native 仍没有。**新增 CLI 默认应该有**，不做要在提交说明里写明理由。

**文案一律走 `common.provider.*`，没有 `i18nPrefix`**：标题「供应商列表」、按钮「添加供应商」/「通用配置」、空态「暂无供应商配置，点击上方按钮添加」。组件不再按 `<tool>.provider.*` 取词——那套 per-CLI key 层级混乱（有的在顶层、有的在 `provider` 下），是空态渲染出字面量 key 的根因。claudecode / codex 迁移时产生的孤儿 key 已 prune。

> 各 CLI 的空态文案内容确实有差异（claudecode 要提「或从 OpenCode 导入」），但**差异只在补充说明**，主体是同一句。所以拆成「通用基座 + 可选 `emptyTextHint`」，而不是让每个 CLI 传整句。

**组件负责的固定项**：Collapse 骨架、`多选/退出` 切换、批量工具栏、搜索框、排序下拉、`定位`（传了 `locateProviderId` 时）、`一键测试`、`通用配置`、`添加供应商`、空态、搜索空态、`data-sidebar-section` 标记。

#### 4.2.2 卡片 `ProviderCard`

`web/components/common/ProviderCard/` 曾是**旧卡片**（拖拽手柄 / 多选复选框、名称、SDK 标签、baseUrl、编辑/复制/分享/删除按钮 + `extraActions`）。2026-10-22 最后三家 `pi` / `oh_my_pi` / `dsh` 迁完后，**组件本体已删除**（目录只剩 `types.ts`，`ModelDisplayData` 等共享类型仍在使用）。**已迁移的 CLI 不得再 import 旧卡**（`verify-provider-card-layout.mjs` 的第四条规则机械拦截），新 CLI 一律用 `providerCardVariants` 的三种样式之一。

**Claude / Codex 不直接用它**——两者各自有模块内的 `ClaudeProviderCard` / `CodexProviderCard`，字段区差异太大（Claude 是 Haiku/Sonnet/Opus 映射行，Codex 是自动审批行 + 完整模型目录），强行参数化只会得到一个塞满条件分支的组件。

**给新 CLI 的取舍建议**：

| 情况 | 做法 |
|------|------|
| 卡片就是"名称 + 地址 + 几个按钮" | 直接用 `ProviderCard` |
| 有 CLI 专属字段区 | 参照 `CodexProviderCard` 写模块内卡片，**模型目录部分**复用 `ModelListSection` |
| 需要状态标签 / 应用按钮 / 更多菜单 | 写进模块内卡片；等**第二个** CLI 也需要同样结构时再上提到共享组件 |

> 教训：不要预先给共享组件加"以后可能会用到"的插槽。曾经给 `ProviderCard` 加了 5 个零消费者的插槽并写进文档，代码审查时被认定为误导性契约，已全部删除。

#### 4.2.2.1 卡片视图（列表之外的第二种呈现）

工具栏最右的「列表 / 卡片」切换把同一批供应商画成紧凑卡片网格。它是 `shared/providerCardView` 提供的**呈现密度**，**不是 `providerCardVariants` 的第四种样式**（§4.2.1 的三种样式按 provider 模型形状选型，这个是按用户偏好切换，见 13 节 #133 / 模式八十二）。已接入：claudecode（2026-10-11）、codex（2026-10-12）、kimi / zcode（2026-10-13）、opencode（2026-10-19）、claudedesktop / geminicli / grok / omo_native（2026-10-20，这批之后 9 个 coding 页全部具备两种布局）。

**接入三步，只动前端**：

| 步骤 | 改哪 | 做什么 |
|---|---|---|
| ① 映射层 | `<cli>/components/*ProviderCard.tsx` | 加 `viewMode?: ProviderViewMode` 一个 prop；`'card'` 分支渲染共享 `ProviderCompactCard`，**复用同一份** `providerState` / `actions` / `nameTags`（不要为卡片分支另算一套按钮条件），再把它**侧栏的内容**交给卡片的 `detail` prop——**抽屉本身（`Drawer`：宽度 `min(60vw, 760px)`、`destroyOnHidden`、开合状态）由共享卡片渲染**，映射层不 import `Drawer` / `ProviderDetailPanel`（2026-10-14；四个映射层曾各写一份逐字相同的抽屉，见 13 节 #151 / 模式一百）。列表卡片上**行内**的动作（连通性测试）走 `extraMenuItems` 进「更多」菜单——位置变了，入口不能丢 |
| ② 页面 | `<cli>/pages/*Page.tsx` | `useProviderListViewPrefs('<module>')`；在 `return` 之前写一份 `*ProviderCardProps(provider)`，两种布局都渲染它（各写一份就是两份会分叉的 handler）；`ProviderListSection` 传 `viewMode` / `onViewModeChange`；列表容器在卡片模式换成 `ProviderCardGrid`；`SortableContext` 的 strategy 切 `rectSortingStrategy`、`DndContext` 的 modifiers 加 `restrictToWindowEdges`——**一个 `DndContext` 服务两种视图**，别为网格再建一个（两套传感器会对「什么时候禁用拖拽」各说各话） |
| ③ 后端 | — | **不用改**。偏好存在既有的 `provider_view_modes` 嵌套键，command 是 `save_provider_view_mode`，module key 传本 CLI 的 |

**格子里放什么由本 CLI 决定，但只有三样**：标题行、第二行（**只说一件事**——Claude 是全部角色模型，Codex 是主模型 `主模型: gpt-5 (high)`，**官方渠道是「当前登录: <账号>」**，没配就不出这一行；OpenCode 是裸 npm 包 `@ai-sdk/…`（**无 label**，`subtitle` 的 `label` 可省），有默认模型时再追加 `当前主模型: <model>`——npm 包每个 provider 都有，所以这一行不会空；OmO Native 同形）、两个主按钮 + **这张格子自己的那个控件（`cellControl`）**。

> **`cellControl` 是「provider 永不被应用」的 CLI 用来占住那个动作位的插槽**：ZCode 没有 provider 级 应用（`设为默认` 在模型行上），格子第二行本来空着，而它的 per-provider 控件是**启用开关**——列表卡把同一颗开关画在头部，于是格子把它放在别的 CLI 放 应用 / 网关代理 的位置（共享的 `ProviderEnabledSwitch`，2026-10-15 用户指定）。**插槽而不是布尔开关**（卡片不决定状态控件是什么），且传了就**从「更多」菜单里撤掉那一条**：同一张卡片上同一个控件不说两遍（理由与做法见 `providerCardView/AGENTS.md`、`providerCardVariants/AGENTS.md`）。**其余事实与动作全部进侧栏**：`sections` 由映射层算（`{ key, title, fields, content }`，字段行靠 `emptyText` 有无决定「缺值时整行消失」还是显示「未配置」）。

> **搬进侧栏的目录 / 账号块必须渲染同一个共享组件**（`providerCardVariants/ProviderModelSection`、`shared/officialAccounts/OfficialAccountsSection`）：模型目录在侧栏里仍是**能用的管理器**（批量删除 / 模型测试 / 获取模型 / 添加模型）。**不要写一个「侧栏版目录」**——卡片视图下的模型管理会变成残缺的一套。
>
> **侧栏分区标题要用专名**：Codex 的模型绑定区叫「模型配置」（`codex.model.mappingTitle`），**不是** `common.model.title`（模型列表）——后者是它下面模型目录区自己的标题，同一个侧栏里两个「模型列表」会把「一个绑定」读成「一份清单」（13 节 #139 / 模式八十八）。

**最小验证**

```bash
pnpm run test:provider-card-view      # Claude 宿主（53 项，真 Chromium 挂真实 ClaudeCodePage）
pnpm run test:codex-card-view         # Codex 宿主（81 项，真 Chromium 挂真实 CodexPage）
pnpm run test:kimi-card-view          # Kimi 宿主（37 项）
pnpm run test:zcode-card-view         # ZCode 宿主（49 项，含格子的启用开关与官方账号格子）
pnpm run test:migrated-provider-cards # 2026-10-21 六家（claudedesktop / geminicli / grok / omo_native / openclaw / hermes，53 项，组件级夹具）
pnpm run test:provider-card-layout    # 映射层守卫：卡片分支不得自建 chrome
pnpm run test:provider-list-drag      # 列表布局的拖拽未被改动
pnpm run test:provider-list-locate    # data-provider-id 契约在两种布局下都成立
```

新宿主照**最接近的那一套**加同款套件（Claude 的差异点是角色绑定与 CLI 启动；Codex 的差异点是模型目录与官方账号块）。夹具与断言的要点：

- 命令表**用共享的 `codexPageStubs.js` 这类桩**，不要复制一份（页面加一个新 command，复制的桩会静默渲染出另一个页面）；
- **桩必须按参数作答**（`args.providerId` 过滤），否则夹具会渲染出一个真实应用不会产生的页面（13 节 #140 / 模式八十九）；
- 每条「有 X」的断言配一条「**不该有 X**」的反向断言（同上）；**已改成常开的区块**（模型列表、明细里的账号列表）不再需要「先点开再断言」，但仍要等**行**渲染出来再量几何：卡片元素早于区块存在，过早测量会量到空盒子并误判「对齐正常」；
- 跑完**看截图**：断言是抽样，截图是全貌，这几轮的实际缺陷大多是这样发现的；
- **高度与间距类断言要在「同一行里形态不同的两张卡片之间」比**：网格会把整行拉到最高那张，所以「卡片高度是它自己内容的函数」这件事只有在有/无第二行、名字长短不一的卡片并排时才看得出来（13 节 #142 / 模式九十一）。

**相关**：§13 教训 #132–#141、§13.1 模式八十一～九十、附录 A.1（迁移进度表）、附录 B.1（`ProviderListSection` 的 `viewMode` 台账）、模块 `web/features/coding/shared/providerCardView/AGENTS.md`。

#### 4.2.3 模型列表 `ModelListSection`（仅"支持自定义模型"形态）

**两种形态**，由 `appearance` 选。数据、工具栏节点、行级操作都是同一份，差别只有折叠与行的画法：

| `appearance` | 谁在用 | 折叠 | 行（`ModelItem` 的 `appearance`） |
|---|---|---|---|
| `'card'`（默认） | 供应商列表卡片（Codex / OpenCode 式） | **折叠**，工具栏就是折叠头，默认收起；行缩进 18px 让开 caret | 卡片口径：把手常驻、带文字的动作保留占位并淡入（`visibility`）、字号 13/12px |
| `'detail'` | 明细侧栏（卡片视图） | **常开**（2026-10-12 用户指定） | 侧栏口径：把手与带文字的动作静止时不在流里（`display: none`）、字号 12/11px |

> ⚠️ **「明细」的需求不等于「列表」的需求。** 2026-10-12 的这几条（模型列表常开、账号列表常开、行内动作让出宽度、字号变小）都是**站在明细侧栏上**提的；把共享组件直接改成一种形态，结果是列表布局跟着一起变——用户 2026-10-13 报「**列表的样式不要变啊，列表样式的模型列表怎么也没有折叠了**」。加外观参数前先数调用点（13.1 模式九十六）。

明细侧栏的形态（常开）：

```
模型列表 (6)                    🗑批量删除  🧪模型测试  ☁获取模型  +添加模型
自动审批模型: deepseek-v4.1-flash  清除自动审批模型          ← aboveList
┌────────────────────────────────────────────────────────────┐
│ DeepSeek V4.1 Flash (deepseek-v4.1-flash | 400K) ·当前主模型   │ ✏️ 📋 🗑 │  ← 静止
└────────────────────────────────────────────────────────────┘
   ↑ 与上面的「模型列表」标题左对齐
┌────────────────────────────────────────────────────────────┐
│ ⠿ DeepSeek … │ [设为主模型] ✏️ 📋 🗑 │                        │  ← 指针在行上
└────────────────────────────────────────────────────────────┘
```

**模型行是一行**（`ModelItem`，全 CLI 共用）：名称 + `(id | 上下文限制)` + `· 当前默认` + 右侧操作。
- **上下文限制写进括号，不另起一行**，也不显示完整数字——用 `200K` / `1M` 这种可扫读的形式（`formatModelLimit`，`web/utils/modelLimits.ts`）。
- **输出限制不在卡片上展示**（编辑弹窗里照常可改）。它的值仍参与校验与保存，只是不再占列表的一行。
- **侧栏口径下，把手与带文字的动作只在悬停时出现，且用 `display: none` 真正让出宽度**：静止时文本独占整行、模型名与区块标题左对齐。用 `visibility` / `opacity` 会保留盒子，把 478px 侧栏行里的文本挤到 60px（2026-10-12 用户报「明细里的模型列表展示有问题」）。**卡片口径照旧用 `visibility` 保留占位**——卡片够宽，而且列表布局本来就不在这次改动范围内。图标三连与开关在两种口径下都**始终可见**。
- ⚠️ **`ModelItem` 的 `useSortable` 必须传对象 `{ draggable, droppable }`，不能传布尔值**：dnd-kit 把 `disabled: true` 读作「不能拖、但仍可被放下」，不可重排的模型行于是成了上层 `DndContext` 的投放目标，拖**供应商卡片**时 `over` 解析成模型行 id，卡片列表的 `indexOf` 找不到而静默放弃——症状是「模型列表一展开，卡片就拖不动了」。判据：**这个 sortable 的 id 在它外层 `SortableContext` 的 items 里吗？** 不在（模型行）→ 关掉 `droppable`；在（`CardShell` / `ProviderCompactCard`，两者保留布尔形式）→ 留作投放目标无害。

**关键 props**：

| prop | 说明 |
|------|------|
| `models` / `rowKeyOf` | 行数据 + 行标识解析器（默认 `model.id`） |
| `sectionKey` | 区块标识（多供应商共存时必须唯一）；折叠形态下是 Collapse 的 key，明细形态下同时发布成 `data-model-list-section`——夹具靠它找到侧栏那份（卡片那份没有这个属性，它的锚点是自己的 `data-provider-id`） |
| `appearance` | `'card'`（默认）/ `'detail'`，见上表。**明细侧栏唯一要改的就是这一个 prop** |
| `selectionMode` / `selectedIds` / `onToggleSelection` | 批量删除选择态 |
| `onToggleBatchDeleteMode` / `onBatchDelete` | 批量删除入口与执行 |
| `onTest` / `onFetchModels` / `onAddModel` | 工具栏三按钮；**传了才渲染** |
| `onEditModel` / **`onCopyModel`** / `onDeleteModel` / `onSetPrimaryModel` | 行级操作；**四个一起传**，漏一个就少一个按钮 |

> ⚠️ **`onCopyModel` 是最常漏的一个**：ZCode 传了 edit / delete / setPrimary，唯独没传 copy → 模型行没有「复制」，而其余 7 处调用方都有（2026-10-06 已补）。与工具栏「只传 1 个 handler」是同一类错（13.1 模式二）。**逐个指认行级按钮数量**能发现，只读代码不能。
>
> **复制 ≠ 编辑**：行 id 就是业务 id 的 CLI（ZCode 按 model id 索引目录）**不能**照抄 Codex 的「同 id 加名字后缀」，那会造出重复 id。正确做法是走**新增**流程并预填副本（`modelIndex: null` + `prefill`），让用户自己起新 id。
| `renderModelExtraActions` | 行级额外操作（Codex 的「设为自动审批模型」） |
| `aboveList` | 工具栏下方、列表上方的内容（Codex 的自动审批行） |
| `className` / `bodyStyle` / `transparentRows` | 样式适配（Codex 用透明背景）。**缩进不要写在这里**：让开 caret 的 18px 是**折叠形态的一部分**，由区块自己按 `appearance` 施加——两个宿主渲染的是同一个 `bodyStyle` 对象，差异写进对象就必然有一侧是错的 |

> **⚠️ 行标识陷阱（Codex 实证）**：`rowKeyOf` 默认用 `model.id`，但当**同一上游模型以多个菜单名出现**时，行标识必须包含 displayName。Codex 的 `display.name` 在上游无 `displayName` 时会**回退成上游 id**，因此**不能**从 display 反推 key。正确做法是在构造 `models` 时建立 `Map<display 对象, rowKey>` 并按对象身份查回（见 `CodexProviderCard` 的 `rowKeyByDisplay`）。

#### 4.2.4 模型编辑弹窗（`ModelFormModal`）

`web/components/common/ModelFormModal/` 已实现，**按 CLI 能力动态裁剪字段**：

| props | 效果 |
|-------|------|
| `showOptions` / `showVariants` / `showModalities` | OpenCode 系字段 |
| `showInputTypes` / `showApi` / `showReasoning` / `showCompat` / `showThinkingLevelMap` | Pi 系字段 |
| `showThinkingLevel` + `thinkingLevelOptions` | Hermes 系字段 |
| `showOmpThinking` | OMP 系字段 |
| `showCost` / `showExtraParams` | 成本 / 额外参数 JSON |
| `limitRequired` / `requireCompleteLimitPair` / `nameRequired` | 校验强度 |
| `npmType` | 预设模型下拉的数据源 |
| `toolName` | 插值进 `capabilitiesHint` / `inputTypesHint`（"支持 **Pi** 的 extended thinking"） |
| `messageOverrides` | 见下方 |

**文案策略**：默认全部走 `common.model.*`（约 55 个 key）。只有**真正因工具而异**的才用 `messageOverrides` 覆盖：

```tsx
<ModelFormModal
  toolName="Pi"
  messageOverrides={{
    idPlaceholder: t('pi.model.idPlaceholder'),          // 示例模型名不同
    contextLimit: t('pi.model.contextLimit'),            // 字段叫法不同
    thinkingLevelHint: t('pi.model.thinkingLevelHint'),  // 字段语义不同
  }}
/>
```

> **`messageOverrides` 的值必须是 `t(...)` 的结果，不能是 key 字符串。**
>
> 最初写成 `idPlaceholder: 'pi.model.idPlaceholder'`（传 key，组件再 `t()`），结果 `i18n:prune` 把 `pi.model.idPlaceholder` 判定为"无人使用"并删除——静态分析看不见对象字面量里的字符串。改成 `t(...)` 后：值就是最终文本，组件用 `??` 短路，工具链也能正常追踪。
>
> 改这个语义时顺带暴露了两个**早就存在的坏 key**（`i18n:check` 现在能看见了）：
> - `omoNative.model.*` 整组不存在（页面一直渲染字面量 key 名）
> - `hermes` 借用 `pi` 前缀，`hermes.model.thinkingLevelHint` 并不存在
>
> 教训：**传 key 字符串给组件做延迟翻译，会同时骗过 i18n 检查工具和未来读代码的人。**

**哪些 key 需要覆盖**（各 CLI 的真实差异）：

| key | 差异性质 |
|-----|---------|
| `idPlaceholder` | 示例模型名（`deepseek-chat` / `gpt-4o` / `claude-sonnet-4.5`） |
| `name` / `namePlaceholder` / `nameOptionalPlaceholder` | 字段叫法（"模型名称" vs "显示名称"） |
| `contextLimit` / `outputLimit` (+Placeholder) | 叫法与示例（"上下文限制" vs "上下文"） |
| `reasoning` | "推理" vs "支持思考" |
| `costHint` / `extraParamsHint` | 是否提示"留空则删除" |
| `thinkingLevelHint` / `thinkingLevelMapHint` | 写入位置不同（如 `agent.reasoning_overrides`） |

其余（`addModel` / `editModel` / `id` / `api*` / `compat*` / `cost*` / `inputTypes*` / `variants*` / 各类校验消息）**都是同一句话，直接共用 `common.model.*`**。

**⚠️ 「选择预设模型」是模型编辑弹窗最重要的能力之一，不许漏。**

它让用户从 models.dev 数据里一键带出模型的全部参数（名称、上下文/输出限制、成本、模态、能力、variants），而不是靠记忆手填十来个字段。**新 CLI 的模型弹窗若没有它，等于把一个高价值能力降级成纯手工表单**——这是不可接受的缺失，不是「可选增强」。

> **实际教训**：ZCode 的模型弹窗从始至终没有预设入口，而同一批次的其他 4 个自建模型弹窗、以及共享的 `ModelFormModal` 都有。迁移时只核对了布局，没核对该有的能力，于是这个缺口一直没被发现，直到被指出来。
>
> **所以「有没有预设模型」必须单独进核对清单**（见 4.0.2-H）——它不属于「样式」，样式核对抓不到它。

**四个必须保证的交互**：

1. **「选择预设模型」入口在模型 ID 输入框右侧**（`ModelFormModal` 的 ID 字段 flex 布局内）——**没有它就不算完成**
2. **编辑态 ID 不可编辑**（`disabled={isEdit}`），选其他预设**只更新其余字段**，不覆盖 ID
3. **新增态选预设**：填充**完整参数**（含 ID、名称、上下文/输出限制、options、variants、模态、能力）
4. **预设数据源按本 CLI 的 SDK 类型取**：`npmType` → `PRESET_MODELS[npmType]`。协议可变的 CLI 由当前协议推导 npmType（如 Codex：`anthropic_messages` → `@ai-sdk/anthropic`，其余 → `@ai-sdk/openai`）

**预设字段 → 本 CLI 字段的映射**（各 CLI 字段名不同，需要显式转换，不是照搬）：

| 预设字段 | 常见落点 |
|---------|---------|
| `id` / `name` | 模型 ID / 显示名 |
| `contextLimit` / `outputLimit` | 上下文 / 输出限制 |
| `modalities.input`（`text`/`image`/`pdf`/`video`/`audio`） | 输入模态开关 |
| `tool_call` / `attachment` / `reasoning` / `temperature` | 能力开关 |
| `cost.*` | 成本字段（若本 CLI 支持） |
| `variants` / `options` | 高级字段（若本 CLI 支持） |

> 本 CLI 没有对应字段的，**留空/继承**，不要硬塞。有对应字段但预设没给的，也保持继承态。

#### 4.2.4.1 自建模型弹窗：字段集合必须来自上游的「可编辑白名单」

> **只有当本 CLI 的模型字段语义与共享 `ModelFormModal` 差异过大、必须自建弹窗时才读这一节。** 能复用共享组件的，字段集合已由它的 `show*` 开关决定。

**问题**：共享组件有 `show*` 开关决定字段集合；自建弹窗没有这个约束，于是**字段集合变成「实现者认为该有哪些」**。结果是想当然地加上上游根本不开放编辑的字段——用户能填、能保存、看起来正常，但写出的键官方编辑器永远不产生，而且**同一字段在不同模式下可编辑性还不一致**（上游 schema 允许与否）。

**做法：先找上游的「可编辑 schema」，以它为准，不要凭直觉增减字段。**

每个 CLI 都可能有一份「用户可编辑字段」的独立 schema，**它与完整的存储 schema 不是同一份**。ZCode 的例子：

```ts
// packages/provider/src/config/manual-model-config.ts
// 手动模式只冻结产品明确开放的叶子；新增系统字段默认不属于个人手动配置。
export const manualModelConfigSchema = completeModelConfigDataSchema
  .pick({ enabled: true })
  .extend({
    properties: complete.properties.pick({
      contextWindow: true,
      supportsJsonSchemaOutput: true,
      supportsNativeWebSearch: true,
      supportsMidConversationSystem: true,
    }).extend({ inputFormat: ...pick({ supportsImage, supportsVideo, supportsPdf }) }),
    optionSpecs: complete.optionSpecs.pick({ reasoningLevel: true }).extend({ maxOutputTokens: ...pick({ max }) }),
  });
```

**核对步骤**（每个自建模型弹窗都要走一遍）：

1. **在上游源码里找可编辑 schema**——搜 `manual*`、`editable*`、`userConfig*`，或看官方设置界面的字段渲染组件（ZCode 是 `ProviderModelMetadataDialog.tsx` 里硬编码的三个能力项）。
2. **逐字段对照**：本弹窗的每个控件，在可编辑 schema 里吗？
   - **不在 → 删掉控件**。多一个控件就是多一个「能填但官方不认」的字段。
   - **在但弹窗没有 → 补上**（除非有明确理由，写进模块 `AGENTS.md`）。
3. **被删掉的字段要「原样保留」**：上游自己写的行带着这些键，重新保存**不能静默删除**。写一个 `pick<上游系统字段>()` 只复制「存在的」键——**不要补默认值**，因为「键缺失 = 让 CLI 目录决定」与「显式 false = 覆盖」语义不同。
4. **预设构造器同样不能写系统字段**，即使预设目录声称有该能力——上游也是由 CLI 而非预设决定。
5. **加回归测试**：断言 (a) 预设不写系统字段；(b) 系统字段经保存后仍在；(c) 键缺失时保持缺失。
6. **每个字段还要问一句「官方把它放在哪一层」**——字段存在不等于放对位置。同一个开关可能在官方是**行级**（列表行上的 `Switch`）、而我们放进了编辑弹窗。
   > ZCode 的「启用」：官方在模型列表行右侧（`ProviderFormControls.tsx` 的 `Switch`），编辑弹窗里**没有**；我们原本放在弹窗里，而 `ModelItem` 当时根本没有行级开关——照官方改会直接丢掉「禁用单个模型」的能力。
   >
   > 判据：**这个状态作用于整张卡片，还是某一行？** 整张卡 → 卡片头部；某一行 → 那一行上。放错层级的表现是「能用，但找不到」或「在那里，但说不清作用于谁」。

**ZCode 实证**（2026-10-06）：弹窗多了「工具调用」「MFJS 工具 Schema」「最大输出参数映射」三个控件。前两个官方 UI 完全不渲染（`ProviderModelMetadata.ts` 注释：「系统字段不由编辑草稿产生」），第三个官方只保留、不提供编辑入口。而 `manual` 模式下前两个又被代码隐藏——**同一字段两种行为**。

> **判据一句话**：**「这个字段上游让用户改吗？」** 答案不在本仓库的代码里，在上游源码里。**没查过上游就加字段，等于猜。**

#### 4.2.5 「获取模型」的预设匹配

`FetchModelsModal` 的 `onSuccess` 回调里，新增的模型要用 **model id 去预设模型匹配**，命中则自动填充其余参数。Codex 的实现是 `importModelsIntoCatalog(..., (modelId) => findPresetModelById(modelId, sdkType))`，配套 `fillCodexCatalogModelFromPreset`。

> **ZCode 的教训**：`handleFetchModelsApply` 里只写了 `modelId` + 名称，**没有做预设匹配**——用户点「应用」拿到的是一串裸 id，每个参数还得自己查。这个缺口藏得很深：入口存在、能跑通、不报错，只是白拿不到数据。
>
> 判据是「**用户在弹窗里点了应用之后，模型列表里那一行是否已经带上了参数**」，不是「有没有这个入口」。预设在这种路径上是**两个**消费点——模型弹窗的标签选择器，和获取模型的应用回调——**只做一个不算完成**。

**两条 `FetchModelsApplyResult` 的义务**（每个 CLI 都要处理，不是可选项）：

| 字段 | 义务 |
|------|------|
| `removedModelIds` | 用户在弹窗里勾了「移除已不存在的模型」才非空。**忽略它 = 开关点了没反应**（13.1 静默失效）。ZCode 曾漏掉这一条 |
| `orderedModelIds` | 按弹窗显示顺序整理列表（含未勾选的 id）。是否重排按本 CLI 是否有手动排序决定——**有手动拖拽排序的 CLI 直接重排会覆盖用户的操作**，需权衡 |

#### 4.2.6 编辑供应商弹窗的分区（`ProviderFormSections`）

供应商编辑弹窗在**顶部通用字段**（渠道 / 名称 / Base URL / API Key）之后，按固定顺序排列以下分区：

```
[模型映射]        ← 可选：只有走"角色→上游模型"映射的 CLI 才有（Claude 有，Codex 没有）
[高级设置]        ← 调用方自己传：JSON（Claude）还是 TOML（Codex）
[计费配置]
[自定义请求头]
[模型改写]
[备注]
```

用共享组件 `web/features/coding/shared/providerConfig/ProviderFormSections.tsx` 编排：

```tsx
import ProviderFormSections from '@/features/coding/shared/providerConfig/ProviderFormSections';

<ProviderFormSections
  editable={!isOfficialMode}          // 官方模式下只保留备注，其余分区隐藏
  modelMapping={renderModelMappingSection()}  // 不传则整个分区不渲染
  advancedSettings={<Form.Item wrapperCol={...}>…JSON 或 TOML 编辑器…</Form.Item>}
  billing={billingConfig} onBillingChange={setBillingConfig}
  customHeaders={customHeaders} onCustomHeadersChange={setCustomHeaders}
  modelRewrites={modelRewrites} onModelRewritesChange={setModelRewrites}
  notesRows={3}
  notesResetKey={notesCollapseResetKey}
/>
```

备注区文案走 `common.provider.notes` / `common.provider.notesPlaceholder`（各 CLI 5/5 相同，无需覆盖）。

**设计要点**：

| 点 | 说明 |
|----|------|
| `modelMapping` 可选 | **有映射表的 CLI 传，没有的不传**。Codex 的模型目录在卡片列表上，弹窗里就没有映射表——这是有意差异，不是缺失 |
| `advancedSettings` 由调用方传 | 编辑器类型随 CLI 变（JSON vs TOML），校验规则也各不同，不适合塞进共享组件 |
| 顶部通用字段**不共享** | 各 CLI 的字段、校验和渠道联动差异太大（`providerEndpointKey` / `apiFormat` / `configToml`…），共享收益低于成本 |
| `editable=false` | 官方模式：计费/请求头/改写全部隐藏，只留备注 |
| `showNotes={false}` | 该 CLI 的 provider 记录**没有 `notes` 字段**时传：渲染一个写了也不落盘的备注区，等于在表单里放一个假字段。OpenCode 是首个消费方（2026-10-19） |

**注意**：导入模式（从 provider 导入）的分支同样走这个组件，传 `editable={false}` + `modelMapping`。

#### 4.2.7 协议下拉的网关支持门控

供应商表单里若有**协议/格式下拉**（`apiFormat`），它的可用性依赖"该 CLI 能否被网关接管"：

```tsx
import { useGatewaySupportedCliKeys } from '@/features/coding/shared/gateway/useGatewaySupportedCliKeys';

const { isGatewaySupported } = useGatewaySupportedCliKeys();
// `undefined` = 列表还在加载；只有明确 false 才置灰，避免加载瞬间锁死用户
const gatewaySupportsThisCli = isGatewaySupported('<cliKey>') !== false;

<Select
  options={apiFormatOptions}
  disabled={!gatewaySupportsThisCli || !selectedIsCustomProviderProfile}
/>
```

**为什么**：某个 CLI 若不能被网关接管，它就没有协议转换能力，上游格式由渠道固定决定，这个下拉形同虚设。

**判定源**：后端 `GatewayCliKey::supported_mvp()`，经命令 `proxy_gateway_supported_cli_keys` 暴露。**不要在前端再维护一份镜像列表**——该列表当前为 `claude / claude_desktop / codex / grok / kimi / gemini / antigravity`。

**与 `GATEWAY_USAGE_TOOLS` 的区别**（容易混）：

| 列表 | 含义 | 数量 |
|------|------|------|
| `GatewayCliKey::supported_mvp()` | 网关**能接管**（可改配置、可转协议） | 7 |
| `GATEWAY_USAGE_TOOLS` | 只是**统计收集**，不意味着能接管 | 15 |

**当前状态**：有格式下拉的 4 个 CLI（claudecode/codex/geminicli/grok）**全部在支持列表内**，所以这个门控今天不改变任何行为。它的价值在于：将来接入**不在支持列表**的 CLI 时，格式下拉会自动置灰，无需再改代码。

#### 4.2.8 全局提示词区块

**三个组件已全部共享**，位于 `web/features/coding/shared/prompt/`：

| 组件 | 职责 |
|------|------|
| `GlobalPromptSettings` | 外壳：Collapse + 「添加全局提示词」+ 提示块 + 列表 + 拖拽排序 |
| `GlobalPromptConfigCard` | 卡片：拖拽手柄 + 展开/收起 + 应用 + 更多菜单（编辑/禁用/删除） |
| `GlobalPromptConfigModal` | 编辑弹窗（名称 + Markdown 内容） |

**接入方式**（新 CLI 只需三行）：

```tsx
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';

<GlobalPromptSettings
  key={`<tool>-prompt-${promptExpandNonce}`}
  toolName="Grok"                     // 插值进提示块与名称占位符
  promptFileName="AGENTS.md"          // 该 CLI 的运行时提示词文件名
  service={<tool>PromptApi}
  collapseKey="<tool>-prompt"
  refreshKey={promptRefreshKey}
/>
```

**文案一律走 `common.prompt.*`，没有 `translationKeyPrefix`**。所有 22 个 key（标题、应用/禁用/删除确认、空态、名称与内容校验、占位符……）都通用化了，13 个 CLI 的 `<prefix>.prompt.*` 副本已全部 prune（每个 22 个，共 286 个）。

**两个插值 prop**：

| prop | 用途 | 不传时 |
|------|------|--------|
| `toolName` | 提示块（"预设多套 **Grok** 全局提示词方案"）与名称占位符（"默认 **Grok** 助手"） | 必传 |
| `promptFileName` | 警告句、本地文件提示、内容占位符里的文件名 | 回退为字面量 `prompt` |

**为什么这样设计**：13 个 CLI 的提示词文案**只有工具名和文件名不同**，句子结构完全一致（`namePlaceholder` 是「默认开发助手」vs「默认 Grok 助手」，`contentPlaceholder` 只差写入哪个文件）。与其维护 13 份重复文案，不如共享模板 + 两个插值参数。

> **`toolName` 的取值**：用各页原本 `namePlaceholder` 里的工具名——`Grok` / `Pi` / `Oh My Pi` / `ZCode` / `Claude Code` 等。原本写「默认开发助手」的 5 个页面（claudecode / claudedesktop / codex / opencode / zcode）现在会显示各自的工具名，这是有意的改进。
>
> **`promptFileName` 不传的两个页面**：geminicli（指向「Gemini CLI 全局提示词文件」，非具名文件）、antigravity（引用绝对路径）。它们现在走通用句子 + 字面量 `prompt`，**文案比原来弱**——如果要恢复精确表述，应给组件加一个 `promptFileLabel` prop，而不是把 22 个 key 再拆回去。

**提示词文件名的事实源**：后端各模块 `constants.rs`（如 `KIMI_PROMPT_FILE` / `HERMES_PROMPT_FILE` / `DEFAULT_GEMINI_CLI_PROMPT_FILE`）；Claude Code 的在 `claude_code/commands.rs` 内联（`CLAUDE.md`）。前端目前是**手写字符串**传入，改动时需与后端常量保持一致。

#### 4.2.9 官方账号区块 `officialAccounts`（有官方 OAuth 登录的 CLI）

CLI 有官方订阅登录（OAuth 授权码 / 设备码）时，页面上要有一块：**登录入口 + 账号列表 + 切换 + 保存当前登录**。消费方：codex、kimi、zcode、geminicli、grok。**不要手写这块**——`web/features/coding/shared/officialAccounts`（2026-10-09 抽成）之前，Codex / Kimi / ZCode 各写一份，差异全是无意分叉：同一个动作三种叫法、同一句说明一处是副标题一处是脚注、切换按钮一处隐藏一处置灰（#120 / 模式六十九）。

**接入是两步：先定宿主，再写映射。**

**第一步：判宿主（`variant`）——判据是「这条官方渠道在 provider 表里有没有行」**，不是观感（#114 / 模式六十一）：

| 官方登录的形态 | `variant` | 区块画在哪 | 标题行谁画 | 参照实现 |
|---|---|---|---|---|
| 官方渠道**就是**一条 provider 行（`category=official`） | `embedded` | 那张卡片的 `footer` 槽 | **卡片**的 name row：`namePrefix={<OfficialAccountHeadingIcon />}`，`<OfficialAccountCount count={n} />` 进 `nameTags` | `CodexProviderCard` / `KimiProviderCard` / `GeminiCliProviderCard` / `GrokProviderCard` |
| 官方登录在凭据文件里、provider 表里没有对应行 | `standalone` | **自持一张卡片**，且是**列表成员**（可拖、可排序、位置持久化） | 区块自己画（`headingTitle`） | `ZcodeOfficialAccountCard` |

> **`standalone` 的那种必须在列表里能排序**：它与供应商卡片视觉同级（同款外壳、同一容器），用户就会期望能一起拖。占一个 sortable id、共用同一个 `DndContext` / `SortableContext`，位置按「它上面有几张供应商卡」记（不是列表下标——下标会随搜索/排序模式变）。外壳一律走 `CardShell`：自己写 `<Card>` + `useSortable` + 把手的写法会被 `pnpm run test:provider-card-layout` 拦下（#128 / 模式七十七）。

**卡片布局下（2026-10-14 起）这两行的答案各再分一层**——一个列表成员在网格里的形状是**格子**，所以问题不止「宿主是谁」，还有「格子说哪件事、管理器去哪」：

| 问题 | 列表布局 | 卡片布局 |
|---|---|---|
| 这一块**陈述什么** | 卡片自己的那一行（Codex / Kimi 的 meta 行；ZCode 由区块画标题行） | 该成员的第二行**只说一件事**：官方渠道说「当前登录: <账号>」（共享 key `common.officialAccount.currentLogin`，值取该账号在自己那行的 label），普通 provider 说模型。**三家一致**。**且这一行在任何状态下都说话**：取值顺序＝`find(isApplied) ?? find(isVirtual) ?? 「未登录」`（`common.officialAccount.currentLoginNone`）——闲置渠道不留空行（#153 / 模式一百零二） |
| 账号行（**管理器**）画在哪 | `embedded`：卡片的 `footer`；`standalone`：自持卡片内部 | **这张格子自己的详情侧栏**，且**常开、无 caret**——与 Codex / Kimi 卡片模式完全同形。账号行是一行一个账号、每行各有动作的管理器，一个格子放不下，也不该放下 |
| 🔗 与 `(n)` **谁画** | 卡片名称行（`embedded`）/ 区块标题行（`standalone`） | 格子的标题行（`namePrefix` + `nameTags`），用**同一对** `OfficialAccountHeadingIcon` / `OfficialAccountCount`；侧栏里是**面板的标题行**——区块在侧栏那份**不再**画标题行，否则「官方账号」在同一个抽屉里出现两次 |
| **抽屉本身谁画** | —— | **共享卡片** `ProviderCompactCard` 的 `detail` prop（映射层只给 `title` / `statusTags` / `primaryActions` / `secondaryActions` / `sections`，不 import `Drawer` / `ProviderDetailPanel`）。四个映射层曾各自手写一份逐字相同的 `<Drawer size="min(60vw, 760px)">` + 面板——2026-10-14 收进共享卡片（#151 / 模式一百） |

**第二步：写映射（自家账号记录 → `OfficialAccountRowView`）**

| 字段 | 填什么 | 取法 / 注意 |
|---|---|---|
| `id` | 行唯一键 | 用自家账号行 id，**不要拿 provider id**（多账号会撞） |
| `label` | 显示名 | **身份优先**：`/me` 的 nickname → email → subject → 回退文案。**不是登录时间**（#123 / 模式七十二：Kimi 曾以「OAuth 不发身份」为由只显示登录时间，那条结论是错的） |
| `kindTag` | 「本地」/「OAuth」等短标签 | 已在 CLI 侧翻译 |
| `metaLines` | 套餐、额度、重置时间 | **已在 CLI 侧格式化**——共享组件不认识任何 CLI 的字段形状（同 `providerCardVariants` 的边界） |
| `isApplied` | CLI 当前正在用的那条 | 有「接管期间隐藏运行态」的 CLI 要**先 AND 掉**再传（Codex 在网关接管期间传 false）。卡片布局的第二行取它当**首选**，缺位时落到 `isVirtual` |
| `isVirtual` | 磁盘上活着、库里没有快照的那条登录 | 只能「保存」，不能切换 / 删除——它**就是** live 的那份。卡片布局的第二行拿它当**次选**（「只有磁盘上那份登录」也是一条真实在跑的登录），两者都没有才写「未登录」 |
| `lastError` | 已格式化的刷新失败文案 | 有值时**替换** `metaLines`，不是追加 |

**props 的选择**（全文与理由见模块 `AGENTS.md`）：

| prop | 说明 |
|---|---|
| `variant` | 第一步的结果，必传 |
| `headingTitle` / `listTitle` | 标题措辞。`headingTitle` 只有 `standalone` 传；`listTitle`（账号列表）**每个宿主都传**——列表始终有自己的标题行 |
| `hint` / `applyHint` / `emptyText` | `hint` 由 `standalone` 传给区块（画在标题下方当副标题，不是脚注，#117 / 模式六十四）；`embedded` 宿主把**同一句**渲染在卡片第二行，两条路径都走 `OfficialAccountHint`——**别再抄一份 style**：两处的颜色来源不同（antd `secondary` alpha 0.45 vs `--color-text-secondary` alpha 0.65），ZCode 那句因此深一档（#130 / 模式七十九） |
| `accounts` | 映射结果 |
| `loginAction: ReactNode` | 登录入口，画在**账号列表标题行右端**（它加的是这个列表里的一行）。插槽而不是布尔开关：Codex / Kimi 传按钮、ZCode 传下拉，共享组件不必知道一个 CLI 有几个官方 provider（#120、#129 / 模式七十八） |
| `collapsed` / `onToggleCollapsed` | 折叠能力，**两个都不传 = 不可折叠**。折叠的是**列表**，所以箭头的家在 `账号列表` 那行，不在标题行（#126 / 模式七十五） |
| `pending` / `actionsDisabled` | `pending` 带**动作种类**（两行可以同时各有动作在飞，转圈要落在真按下的那个按钮上）；登录进行中要置 `actionsDisabled`——登录与行动作改的是同一份凭据文件 |
| `onApply` / `onSaveLocal` / `onRefresh` / `onViewDetails` / `onDelete` | 语义固定的五个，**传哪个渲染哪个**；`isVirtual` 行不给删除、`isApplied` 行不给切换 |
| `renderExtraActions: (account) => ReactNode` | **CLI 专属的行内动作**，渲染在「刷新」之后、「详情」之前；按钮形态与 loading 由调用方自带，按行 id 回查原始记录，返回 `null` 即该行不渲染。现有唯一消费方是 Grok 的「刷新 Token」（重新签发 OAuth token）——它不属于五个公认语义中的任何一个，所以走自由插槽而不是给共享类型加具名动作（2026-10-20） |

**措辞与图标收在共享导出里**，CLI 只传已翻译串：`common.officialAccount.headingTitle`（官方账号）/ `.listTitle`（账号列表），加块内动作（`default` / `refresh` / `details` / `saveCurrent` / `switch`）。**官方卡片不要再画「官方订阅」这类类别标记**——标题已经写着「官方账号」，再标一次是同一句话说两遍（#127 / 模式七十六）。

**三条后端不变量（违反了只能在后端修，前端怎么写都补不回来）**

1. **登录在成功之前不落任何行**：取消 / 失败 / 超时全程不建 provider、不建账号（#121）。「点一次登录又关掉，就多一张空壳卡片」就是这条被违反。
2. **凭据文件名 = 固定常量，账号显示名是另一个字段**（#122）。真实 CLI 只认固定文件名（如 `credentials/kimi-code.json`）；拿显示名当文件名 → 账号表按 provider 去重、永远只能有一个账号。正确形状：**磁盘一份 live 凭据 + DB N 行快照**，切换 = 把选中那份写回磁盘。
3. **判重按身份**（refresh_token → account_id → email），不是按 provider。

> 后端**拆三个文件**，因为三件事的失败代价不同：搬运（`official_accounts.rs`，必须字节等价、不许失败）/ 登录（`oauth_login.rs`，可失败、有超时取消）/ 展示（`credential_cipher.rs`，失败只损失一个显示名）。混在一个文件里最容易犯的错是拿展示用的解析结果去驱动搬运（§2 目录结构说明、模式六）。

**最小验证**

```bash
pnpm run test:official-accounts-shared    # 源码棘轮：禁止再手写行级按钮/标题/行动作；PENDING_MIGRATION 只剩 antigravity（只减不增）
pnpm run test:provider-card-layout        # 官方卡片作为列表成员时，外壳必须走 CardShell
```

三个宿主各有一套**真浏览器**套件，各自钉住本宿主的差异点；**新宿主照最接近的那套加一条同款**：

| 套件 | 宿主 | 钉住什么 |
|---|---|---|
| `pnpm run test:codex-official-accounts`（7 项） | embedded（Codex） | 账号行右边缘 == 卡片内容右边缘 == 头部动作右边缘；第二行讲的是切换，不是它已经没有的 endpoint |
| `pnpm run test:official-account-card`（26 项） | embedded（Kimi） | 标题 + 🔗 + 计数、列表标题 + 折叠、登录按钮几何（在标题行右端、与标题垂直居中）、**真点一下**折叠 |
| `pnpm run test:zcode-official-account`（12 项） | standalone（ZCode） | 卡片在列表中的位置与把手、账号行左边缘与列表标题对齐、**`getComputedStyle` 断言说明句的颜色与字号**、真点折叠 |
| `pnpm run test:migrated-provider-cards`（53 项，组件级夹具） | 六家映射层（claudedesktop / geminicli / grok / omo_native / openclaw / hermes，列表 + 卡片布局） | Claude Desktop 角色 `Role: model` + `tierAlias` 真 Tag + 备注 footer 位置 + 无绑定时的备注 + dragDisabled 无把手 + **官方行标题 `🔗 官方账号` 且菜单无删除** + **映射渠道只有「应用并代理」**；Gemini `🔗 官方账号 (n)` 标题、第二行切换说明、账号行跨满卡片内容宽、virtual 行只给「保存当前登录」、已应用行给「默认」；Grok 官方卡片标题与备注、第六个动作经 `renderExtraActions` 出现在「刷新」与「详情」之间、自定义渠道没有账号块且「模型测试」全卡只出现一次、模型工具栏顺序；OmO 格子的 npm 包与默认渠道、菜单删除保护；OpenClaw 的 id / npm 包 / 端点、无「应用」、模型工具栏四项齐全、默认渠道删除置灰且保留 hover 宿主；Hermes 的端点与 API 模式、无「应用」、名称行不重复默认徽章、模型目录行的 `当前主模型` 与 `设为主模型`、模型行无假把手、只读渠道只画「只读」且没有编辑/复制/删除、格子说包名 + 默认模型；暗色主题 |
| `pnpm run test:zcode-card-view` / `test:codex-card-view` / `test:kimi-card-view`（49 / 81 / 37 项） | 卡片布局（三端） | 格子的第二行是「当前登录: <账号>」（Codex / Kimi 的官方格子、ZCode 的官方账号格），**且三态都有输出**（`accounts=virtual` → 磁盘上那份登录（Codex 那份还并存一份没在用的保存账号，须报在用的、不报闲置的）；`accounts=idle` → 有账号但没在用 → 「当前登录: 未登录」），**管理器不在格子里而在该格子自己的侧栏**（ZCode 那份还断言抽屉里那份**常开无 caret**、且**拖它不会打开侧栏**）；三端各自的 `data-provider-id`、拖拽回写、切回列表那侧逐字未变 |

> 三种断言形状值得照抄：① **位置类**用 `getBoundingClientRect`，不读代码（#116 / 模式六十三）；② **样式类**用 `getComputedStyle`——颜色 / 字号漂移类型检查与源码守卫都看不见（#130）；③ **折叠 / 切换**要发真事件再验（13.1 模式六十）。

**相关**：§13 教训 #120–#130、§13.1 模式六十九～七十九、§12.3 的「官方账号区块在列表里的位置」核对项、附录 B.1（prop 台账）、附录 B.2（页面级区块清单）、模块 `web/features/coding/shared/officialAccounts/AGENTS.md`。

#### 4.2.10 迁移进度（2026-10-22 起已全部迁移）

三个共享组件各自独立迁移，**不要把它们当成一件事**——一个页面可能列表外壳迁完了、卡片还在自己手里：

| 关注点 | 已迁移 | 未迁移 |
|------|------|------|
| `ProviderListSection`（列表外壳） | 全部 14 个有供应商列表的 coding 页：claudecode、claudedesktop、codex、geminicli、grok、zcode、omo_native、kimi、opencode（2026-10-20）、pi / oh_my_pi / dsh（2026-10-22）、openclaw / hermes（2026-10-21） | **无**（antigravity 没有供应商列表，只有官方账号面板，不适用） |
| **卡片变体**（`providerCardVariants`） | 同上 14 个 | **无**（`PENDING_MIGRATION` 已清空；旧卡 `components/common/ProviderCard/index.tsx` 已删除） |
| **卡片视图**（`providerCardView`，含详情侧栏 `ProviderDetailPanel`） | claudecode（2026-10-11）、codex（2026-10-12）、zcode / kimi（2026-10-13）、opencode（2026-10-19）、claudedesktop / geminicli / grok / omo_native（2026-10-20）、openclaw / hermes（2026-10-21）、pi / oh_my_pi / dsh（2026-10-22） | **无**。接入方式见该模块 `AGENTS.md`（映射层加 `viewMode`，页面换列表容器，抽屉与面板由共享卡片给——映射层只提供 `detail = { title, statusTags, sections, … }` 的内容；非 provider 的列表成员在网格里就是**一个普通格子**，与邻居同宽同高，管理器进它自己的侧栏） |
| `ModelListSection`（模型列表区；两个样式卡片里折叠，卡片视图侧栏里常开） | codex、dsh、grok、hermes、kimi、oh_my_pi、omo_native、openclaw、opencode、pi、zcode（**均经 `providerCardVariants/ProviderModelSection` 间接触达**，两个样式卡片与卡片视图侧栏共用这一份映射，侧栏那处传 `appearance="detail"`） | — |

> ⚠️ **卡片变体是三者里唯一有机械守卫的**（`pnpm run test:provider-card-layout`）。列表外壳与模型列表区**没有**守卫，迁移进度只能靠这张表——**改这张表时先跑一遍**：
> ```bash
> grep -rln "shared/ProviderListSection" web/features/coding --include=*.tsx | sed 's|web/features/coding/||;s|/.*||' | sort -u
> grep -rln "providerCardVariants" web/features/coding --include=*.tsx | grep -v shared/ | sed 's|web/features/coding/||;s|/.*||' | sort -u
> grep -rln "shared/ModelListSection" web/features/coding --include=*.tsx | sed 's|web/features/coding/||;s|/.*||' | sort -u
> ```

> ⚠️ **守卫扫不到「内联弹窗」**：`scripts/verify-form-modal-layout.mjs` 只检查文件名匹配 `*ProviderFormModal.tsx` / `*ModelFormModal.tsx` 的文件。把供应商/模型弹窗**内联写在页面或 section 里**（`layout="vertical"` + 网格）就完全逃过这道守卫。**迁移时先把弹窗抽成独立文件**，再改布局——否则改完无人拦回归（教训 #83）。历史：omo_native 2026-10-07 抽过一次（13 → 14）；pi / oh_my_pi / dsh 2026-10-22 抽到 `PiProviderFormModal` / `OmpProviderFormModal` / `DshProviderFormModal`（15 → 18）。**现只剩 `hermes` 的供应商弹窗仍是内联的**，它迁移时应照同一顺序做。

### 4.3 页面头部的迁移状态

各页改用 `CodingPageHeader`（见 4.1）的状态一览——**只剩 antigravity 未迁移**：

| 页面 | 差异点 |
|------|--------|
| codex | ✅ 已迁移（2026-10-06，首个消费方） |
| zcode | ✅ 已迁移（2026-10-06，含搜索/排序/多选/拖拽/一键测试/通用配置） |
| claudecode | ✅ 已迁移（2026-10-07；5 组 key 中文全同、英文 3 处措辞统一取 `common.*`） |
| omo_native | ✅ 已迁移（2026-10-07） |
| kimi | ✅ 已迁移（2026-10-08；5 组 key 中文 2 处不同仍统一取 `common.*`，见下） |
| opencode | ✅ 已迁移（2026-10-19；V1→V2 迁移开关未定稿时 `customizeConfigDisabled` 置灰「自定义配置目录」并配 `customizeConfigDisabledReason` 悬停说明，`extraActions` 放「同步预设模型」；页头 6 组 key 统一取 `common.*`，中文措辞未变；页头外层不再自带 `marginBottom`，间距由 `CodingPageHeader` 自己那 16px 提供，否则会叠成 32px） |
| claudedesktop | ✅ 已迁移（2026-10-20；**不传 `onCustomizeConfig`**——本模块没有自定义根目录能力；平台不支持时在页头下方补一行 `claudedesktop.unsupportedPlatform`，页头与工具栏文案统一取 `common.*`） |
| geminicli | ✅ 已迁移（2026-10-20；`onPreviewConfig` 仅在已应用时传） |
| grok | ✅ 已迁移（2026-10-20） |
| openclaw | ✅ 已迁移（2026-10-21）。页头这一层没有覆盖 prop；`openclaw.previewConfig` 仍在用，但那是**预览弹窗自己的标题**（`FileConfigPreviewModal` 的 `title`），不是页头的「预览配置」链接 |
| hermes | ✅ 已迁移（2026-10-21） |
| pi | ✅ 已迁移（2026-10-22）。`configPathKind="directory"`（页头显示配置目录）；`extraActions` 放「同步预设模型」 |
| oh_my_pi | ✅ 已迁移（2026-10-22）。同上：`configPathKind="directory"` + `extraActions` 放「同步预设模型」 |
| dsh | ✅ 已迁移（2026-10-22）。`configPathKind="directory"`；`extraActions` 放「同步模型」「打开 Web UI」与**配置 Profile 选择器**（`profiles.length > 0` 才渲染）——它属于「这一页的全局开关」，放页头而不进供应商工具栏 |
| antigravity | ❌ 未迁移（手写页头，见模块 `AGENTS.md` 第 8 条；它没有供应商列表，迁移时只换页头这一件） |

#### 4.3.1 页面容器的间距：**区块自带 margin，容器不许有 `gap`**

`CodingPageHeader` 自己带 `marginBottom: 16`。如果页面容器（`.pageContent`）同时又写 `display: flex; flex-direction: column; gap: 16px`，两段间距会**相加**：页头到第一个区块变成 32px，而全仓其他页面是 16px。用户能一眼看出来（2026-10-22：「今天新迁移的 3 个 CLI 顶部和下面的模块之间的间距有点大」），但 `tsc`、i18n 检查、组件级夹具**全都看不见**——间距取决于页面容器的计算布局，而没有任何夹具渲染页面容器。

**正确形状**（Codex / Claude Code / OpenCode 都是这个）：

```less
.pageContent { display: block; }        // 不要 flex + gap
.piSection   { margin-bottom: 16px; }   // 区块自带间距，不依赖邻居
```

- **为什么是 `block` 而不是 `flex column` 且不写 gap**：`block` 下相邻区块的 `margin-bottom` 与下一个区块自身（或其首个子元素）的 `margin-top` 会**折叠**；`flex` 容器不折叠，所以只要某个区块的根节点自身带 16px（`ProviderListSection` 的 `Collapse`、`SessionManagerPanel`、`GlobalPromptSettings` 都带），间距就会翻倍成 32px。这是 §13.1 模式十「间距借自邻居」的页面级版本。
- **页头下方的提示行**（hermes 的 `pageHint`、openclaw 的 `configFileHint`、opencode 的 hint + V2 状态）用 `marginTop: -12px` 把自己拉到页头 16px 下边距里面（视觉上属于页头），然后自己补 `marginBottom: 16px` 顶住第一个区块。
- **Alert / 通知条**同理：它是「关于页头的说明」，与页头同属一个视觉块，用 12px 收尾即可，不要用 16px 去和区块抢节奏。
- 机械守卫：`pnpm run test:page-rhythm`（本节由此而来，8 项断言：四个页面 × 页头→首区块 / 区块→区块，都必须量到 16px）。

> **验收方法**：这类间距**只能量**。把页头底边和第一个区块顶边的 `getBoundingClientRect()` 相减，16 才对；读 less 看到 `gap: 16px` 会觉得「也是 16」——这正是它两次逃过检查的原因。

#### 4.3.2 迁移时必查：CSS Modules 的类名还对不对

`styles.xxx` 在 CSS Modules 里是普通对象取值。**类名写错不报错**：取到 `undefined`，React 渲染成 `class=""`，元素、事件、文本全在，只是**样式全丢**。

`OhMyPiPage` 就是这样：页面里 6 个区块都写 `styles.ompSection`，而 `OhMyPiPage.module.less` 定义的是 `.piSection`——页头的 `scroll-margin-top` 与区块间距一起静默失效，直到 2026-10-22 才被「间距看起来不对」发现。

**两个方向都要查，一条命令**：

```bash
pnpm run test:css-module-classes
```

- **引用了但没定义** → 元素渲染成 `class=""`，样式全丢。按文件精确报出，**不许**加进白名单绕过。
- **定义了但没人引用**（孤儿 CSS）→ 迁移把旧结构搬走后，`.pageHeader` / `.providerCard` / `.modalSection` 这类类名整段留在文件里，仍然读起来像「活着的样式」，下一次改动会去改它、然后看不到任何效果。三个迁移页面各有 21–22 个。

> **孤儿检查必须按 less 文件汇总所有导入者**。一个 `.module.less` 常被同目录多个 `.tsx` 共用（`GatewayStatisticsView.module.less` 被统计页与两个图表组件共用），按「单个导入者」判会报出 2655 条假孤儿。用 `:global(...)` 包住的 `.ant-*` 选择器也要先剥掉——它们本来就不是模块键。
>
> **基准已清零（2026-10-23）**：`KNOWN_ORPHAN_COUNTS` 现在是空表。首次上线时登记的 28 个文件 / 112 个类（Codex / Grok 供应商弹窗从 Claude Code 复制过来却没删的整套 `officialMode*`、image / settings 页的旧按钮系统等）已按用户要求**全部删除**。**新增孤儿一律失败**，不许再加回基线——那个表只留给「某个新 CLI 的迁移确实有正当理由暂时留一个」的情况。
>
> **删类之前先过三关**，否则删的是活代码：① Less **mixin** 定义（`.buttonBase() { … }`）与调用（`.buttonBase();`）长得像类名，`buttonSystem.less` 那套被 20 多处调用，误删直接构建失败——解析器现在靠「有没有空参数列表」区分；② `:global { … }` **块形式**里的 `.ant-*` 是全局选择器，不是模块键（本仓库两种 `:global` 写法都在用）；③ `@import '.../x.less'` 的文件扩展名不是类名。这三关都是守卫自己踩过的假阳性，**用全仓数据跑一遍看报的是不是真问题**，再决定删。
>
> 顺带修好一类真 bug：`.listActions .ant-btn { … }` 这类**没包 `:global` 的 Ant Design 选择器**，CSS Modules 会把 `.ant-btn` 也哈希化，规则从写下那天起就没生效过。守卫把它报成孤儿，正是发现它的方式。见到这类写法一律补 `:global(...)`，而不是删掉。
>
> **守卫的第一版还有两处盲区，是「之前迁移的其他 CLI」的残留藏在里面的原因**（2026-10-23 补齐）：
>
> 1. **动态键一票否决**。`styles[`messageRole${role}`]` 曾让整份样式表退出反向检查，于是 `SessionDetailWorkbench.module.less`（14 个组件共用、174 个类）里 7 个真孤儿谁也看不见。现在动态键只登记**前缀**（`level` / `messageRole`），凡以它开头的类视为可能被用到，**其余照查**。
> 2. **没人导入的样式表整个漏掉**。`github-markdown-overrides.module.less` 是 `MarkdownPreview/index.module.less` 的旧副本，全仓零导入，却因为反向检查只看「有导入者的文件」而一直躺着。现在 `*.module.less` 无导入即整份报死。
> 3. **按单个导入者判孤儿**（更早修过）。一个样式表常被同目录七八个组件共用，必须汇总全部导入者的 `styles.x` 再判。
>
> 三条盲区都验证过「造一个假孤儿会红」。**判别标准：守卫报 0 不等于没有残留，先问它到底扫了什么。**
>
> 守卫自己犯过一次错并因此加强：第一版只剥 `/* */` 注释，而 Less 里普遍用 `//`；一句 `// was `.ompSection`` 的注释就把缺失的类名「定义」上了，守卫于是对真正的缺陷报绿。**写源码守卫时，先拿已知违规文件跑一次**（§13.1 模式四十六的推论）。同一轮里它还因为「按单个导入者算孤儿」产生了 2655 条假报——**守卫上线前必须用全仓数据跑一遍，看它报的是不是真问题**。

---

## 5. 阶段五：系统托盘快捷菜单

托盘不是「每个 CLI 注册一次」，而是「一个巨型 `tray.rs` + 每模块一个 `tray_support.rs`」。

### 5.1 模块侧：`tauri/src/coding/<tool>/tray_support.rs`

参照 `zcode/tray_support.rs`（151 行），必须导出：

| 导出 | 说明 |
|------|------|
| `TrayProviderItem { id, display_name, is_selected, is_disabled }` | 单个供应商行 |
| `TrayProviderData { title, current_display, items }` | 供应商子菜单数据 |
| `TrayPromptItem` / `TrayPromptData` | 全局提示词子菜单数据 |
| `async fn is_enabled_for_tray(app) -> bool` | ZCode 恒 `true`；**可见性由 `tray.rs` 的 `is_tab_visible("<tab>")` 决定**，不要在这里判 |
| `async fn get_<tool>_tray_data(app)` | 返回当前选择 |
| `async fn apply_<tool>_provider(app, row_id)` | 处理托盘点击 |
| `async fn get_<tool>_prompt_tray_data` / `apply_<tool>_prompt_config` | 提示词侧 |

**托盘区块必须与页面区块一一对应**：页面上有的能力，托盘里也要有入口（2026-10-07 用户指出 OmO 托盘缺「默认模型」「全局提示词」）。核对方法：把页面 sidebar 的区块列表抄下来，逐个问「托盘里点得到吗」。**不要凭印象判断某个能力「托盘里没有意义」**——OmO 的 `tray_support.rs` 曾写着「Native 的默认模型由 model_profile 与 agents 共同决定，没有单一的全局默认模型字段」，这句话是错的（`settings.json` 的 `defaultProvider` / `defaultModel` 就是），直接导致两个区块缺席。

**子菜单排序规则（硬规则，2026-10-07 统一）**：**主选择项在前，全局提示词紧随其后**。

| 区块形态 | 顺序 |
|---|---|
| 只有供应商（Claude Code / Codex / Gemini CLI / Antigravity / ZCode） | 供应商 → 全局提示词 |
| 只有默认模型（Pi / OMP / OmO / Hermes / Dsh） | 默认模型 → 全局提示词 |
| 模型组 + 供应商（Grok / Kimi） | 模型 → 供应商 → 全局提示词 |
| 主模型 + 小模型（OpenCode） | 主模型 → 小模型 → 全局提示词 |

核对方法：把所有区块的 append 顺序抽出来并排打印（一条正则），逐行对表——**单看一个区块永远觉得「挺合理」**（见 13.1 模式四十）。

**托盘的空态 ≠ 页面的空态**：两边常调同一个列表命令，但托盘可能额外过滤掉「桥接记录」（`__local__`）。过滤前先问「这条记录能不能被应用」，并把结论写进模块 `AGENTS.md`（见 13.1 模式四十一）。

**如果某 CLI 的模型列表要铺平展示，优先复用现成 builder**：`build_named_prompt_submenu(app, prefix, data, texts)` 是泛型的，任何实现了 `NamedPromptTrayData` / `NamedPromptTrayItem` 的 `TrayPromptData` 都能直接用。模型子菜单没有泛型版（`build_pi_model_submenu` / `build_omp_model_submenu` 各自一份），新 CLI 照抄一份即可——它们做的是「按 `<provider>/<model>` 的斜杠前半段分组，一个渠道一个二级子菜单」。

**关键陷阱（ZCode 踩过）**：托盘回传的是 **DB 行 id**，但很多 CLI 内部按自己的 providerId 索引。ZCode 在 `tray_support.rs:96-119` 做了一次 row_id → providerId 的解析（查 DB 行 → 解析 settingsConfig → 调 `select_*_internal_without_events`）。**直接拿 row_id 当 providerId 用会选错或选不中。**

**刷新机制**：`apply_*` 函数本身不刷新菜单；由 `tray.rs` 的 dispatch 分支在 await 之后调 `refresh_tray_menus`。另有全局链路：任何 `app.emit("config-changed", ...)` → `lib.rs` 的全局 listener → `tray::refresh_tray_menus`。

### 5.2 `tauri/src/tray.rs`：9 个必改点

以 zcode 为例（行号为当前工作区）：

| # | 位置 | 内容 |
|---|------|------|
| 1 | `:17-35` | `use crate::coding::<tool>::tray_support as <tool>_tray;` |
| 2 | `:44-80` | `struct TrayTexts` 加 `<tool>_header: &'static str` |
| 3 | `:169`(en) / `:204`(zh) | **两处**都要给值（漏一处 → 该语言下标题为空） |
| 4 | `:467-488` | dispatch 分支：`event_id.strip_prefix("<tool>_provider_")` + `"<tool>_prompt_"`，spawn → `apply_*` → `refresh_tray_menus` |
| 5 | `:848` | `let <tool>_enabled = is_tab_visible("<tab>") && <tool>_tray::is_enabled_for_tray(app).await;` |
| 6 | `:1018-1038` | 取数据 + **else 分支构造空 `TrayProviderData` 并覆写 `.title`** |
| 7 | `:1509-1526` | `<tool>_has_items` / `<tool>_has_prompt_items` / `<tool>_has_section` |
| 8 | `:1567-1576`、`:1877-1896` | 构造 `<tool>_prompt_submenu`、`<tool>_header`（`MenuItem::with_id`）、`<tool>_provider_submenu` |
| 9 | `:2139-2151` + `:3084-3108` + `:3390-3416` | ① section append 进 `menu`；② `impl NamedPromptTrayItem/Data`；③ `impl NamedProviderTrayItem/Data` |

**好消息**：第 9 点的两个 trait impl 漏了**编译不过**（不是静默失效）。真正的静默失效是 `tray.rs` 根本没 import `tray_support`——ZCode 的 142 行全是死代码，端到端接线后才生效。

**MCP / Skills 区不用改 `tray.rs`**：它们的工具列表来自 `get_mcp_runtime_tools` / `get_all_tool_adapters`，注册进 `BUILTIN_TOOLS` 后自动出现（见第 7 节）。

### 5.3 Lightweight 模式

新增**需要主窗口**的入口必须先判断 `lightweight::is_lightweight_mode()`（详见根 `AGENTS.md`「Lightweight Mode」）。托盘自身的 `show` / `lightweight_mode` 项已覆盖，但新 CLI 若加「打开页面」类菜单项必须自己处理。

### 5.4 其他

- **事件 id 前缀必须唯一**：dispatch 用 `strip_prefix` 顺序匹配，若新 CLI key 是另一个 key 的前缀会误命中。
- `tray.rs:796-809` 的 fallback `visible_tabs` 数组（`get_settings` 读失败时兜底）也建议加上新 tab。

---

## 6. 阶段六：WSL / SSH 同步

### 6.1 路径解析：单一事实源

所有同步路径从 `runtime_location.rs` 派生，**不在同步 helper 里临时查 DB/环境变量**。

| 位置 | 符号 |
|------|------|
| `runtime_location.rs:16` | `const MODULE_KEYS: [&str; 14]`（zcode 在 `:25`）—— WSL Direct 状态、缓存刷新、`get_wsl_direct_status_map_async` 的唯一来源 |
| `:199` | `normalize_module_key()` match（`"zcode" \| "zcode_cli" => Some("zcode")`） |
| `:272` | `refresh_runtime_location_cache_for_module_async()` dispatch match |
| `:1242` | **`get_<tool>_wsl_target_path_async(db, file_name)`**（zcode 范本） |
| `:1922` / `:2025` | `get_tool_skills_path_sync` / `_async` —— **SSH Skills 名单要对齐这个函数的 arm**（§7.2.1） |
| `:2249` / `:2278` | `get_tool_mcp_config_path_sync` / `_async` |
| `:1149` / `:1457` | `resolve_<tool>_root_dir_without_db()` / `resolve_<tool>_path_without_db()`（给 backup restore 用的无 DB 入口；**restore 的 fallback 目录必须与后者一致**） |

`get_<tool>_wsl_target_path_async` 就是「Windows 路径 → WSL 内路径」的映射器，范式：

```rust
match get_<tool>_runtime_location_async(db).await {
    Ok(location) => location.wsl
        .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
        .unwrap_or_else(|| format!("~/.<tool>/{file_name}")),
    Err(_) => format!("~/.<tool>/{file_name}"),
}
```

### 6.2 默认文件映射表

**WSL**（`tauri/src/coding/wsl/commands.rs`）：

| 位置 | 符号 | 说明 |
|------|------|------|
| `:957` | `const CURRENT_DEFAULTS_VERSION: u64` | **每次新增默认映射必须 +1** |
| `:992` | `const DEFAULT_MAPPING_IDS_ADDED_IN_V<N>: &[&str]` | 本次新增的 mapping id |
| `:1077` | `should_backfill_versioned_mapping(...)` | 老用户回填的判定调用 |
| `:2475` | `default_mappings()` 的 `FileMapping` 结构体 | `id` / `name` / `module` / `windows_path` / `wsl_path` / `is_directory` |
| `:1476` | `resolve_dynamic_paths_with_db()` match arm | 把默认 `~/.<tool>/...` 换成**实际解析出的**路径（自定义根目录时关键） |

ZCode 的 4 条映射：

```
<tool>-provider-config  module=<tool>  ~/.<tool>/v2/provider_config.json  (file)
<tool>-prompt           module=<tool>  ~/.<tool>/AGENTS.md                (file)
<tool>-cli-config       module=<tool>  ~/.<tool>/cli/config.json          (file)
<tool>-skills           module=<tool>  ~/.<tool>/skills                   (dir)
```

> ZCode 的 `credentials.json` **有意排除**：AES-GCM key 由平台 + home + 用户名派生，跨机无法解密。代码里有注释。

**SSH**（`tauri/src/coding/ssh/commands.rs`）：结构同构，但 `CURRENT_DEFAULTS_VERSION` 是**独立编号**（`:1148`），字段名是 `local_path` / `remote_path`，另有 `zcode_remote_target_path_from_location()`（`:1844`）。

### 6.3 MCP 同步（两条链路，各两个点）

| 位置 | 符号 | 漏改后果 |
|------|------|---------|
| `wsl/mcp_sync.rs:324` | `is_mapped_mcp_config_file()` 白名单 | 该 CLI 的 MCP 配置**完全不参与 WSL 同步**（静默） |
| `wsl/mcp_sync.rs:367` | `strip_cmd_c_from_wsl_mcp_file()` 的 `let processed = match module` | **注册了 id 但没 arm → `_ => return Ok(())`，Windows `cmd /c` wrapper 原样复制到 Linux，WSL 里 server 起不来**（ZCode 实证） |
| `ssh/mcp_sync.rs:332` / `:366` | SSH 版同上（`strip_cmd_c_from_remote_mcp_file`） | 同上 |

> ⚠️ **这两处是「两文件 × 两位置」共 4 个点，漏任意一个都会静默失效。** 2026-10-07 复查时发现 OmO Native **4 个点全漏**：`omo-native-mcp` 既不在 WSL/SSH 的白名单里，也不在两个 `strip_cmd_c` 的 match 里。后果是 MCP 页写得进 `~/.omo/agent/mcp.json`，同步到 WSL 后 `cmd /c` wrapper 原样带过去，server 起不来。
>
> **自查命令**（把 `<tool>` 换成模块 key，四处都应命中）：
> ```bash
> grep -rn '<tool>' tauri/src/coding/wsl/mcp_sync.rs tauri/src/coding/ssh/mcp_sync.rs
> ```
> 预期至少 4 行：两处白名单 + 两处 match arm。**少于 4 行就是漏了**。
>
> 另一个易漏点：**模块的映射 id 未必叫 `<tool>-mcp`**。ZCode 的 MCP 文件是 `zcode-cli-config`（MCP 与 CLI 配置同一个文件），OmO/Pi 是 `<tool>-mcp`。查映射 id 要去 `default_mappings()` 里按 `module` 字段找，别按名字猜。
| `mcp/command_normalize.rs` | 按需新增 `process_<tool>_json` | 无专用 processor 就用通用形状，CLI 可能读不懂 |
| `mcp/format_configs.rs:80` | `get_format_config()` match | `_ => None` → 用通用 key 写入，CLI 只认自己的 key 就**静默丢值**（zcode 的 `timeoutMs` vs `timeout` 实证） |

> **`get_format_config()` 返回 `None` 不一定是缺陷**——`None` 表示「这个 CLI 就是标准 Claude 形状」（`{"type":"stdio","command","args","env"}` / `{"type","url","headers"}`）。判定方法是拿它上游的 schema 对一遍：**字段名和形状都对得上，就不需要加条目**；加了反而可能把 `type` 写成别的值。OmO Native / Pi / Kimi 都走这条路（它们都是 Claude 形状的 `mcpServers`），只有字段名或形状真的不同才建常量（`ZCODE_FORMAT` 的 `timeoutMs`、`GEMINI_LIKE_FORMAT` 的 `httpUrl` 就是这类）。
>
> 反过来说：**新增 CLI 时这里不用动**，只有当你在 WSL/SSH 侧发现「写进去的 key 上游不认」时才回来补。

### 6.4 事件、reapply、状态

| 位置 | 符号 | 漏改后果 |
|------|------|---------|
| `lib.rs:1320` | `app.listen("wsl-sync-request-<tool>", ...)` 块（**当前 14 个**，对照补） | 保存配置后**不自动同步**（静默） |
| `lib.rs:1688` / `:1706` | `mcp-changed` / `skills-changed` 监听器 | 全局各一个，**新增 CLI 不用动**——它们调 `sync_mcp_to_wsl` / `sync_skills_to_wsl`，链路内部按映射 id / `BUILTIN_TOOLS` 遍历 |
| `reapply_applied_runtime.rs:234` | `wsl_module_for_reapply_label()` match | **漏 arm → 该模块不进 `changed_modules` → 被 `unchanged_wsl_modules` 判为「未变」→ 恢复后 WSL sync 静默跳过它** |
| `reapply_applied_runtime.rs:264` | `const ALL_WSL_FILE_MODULES: &[&str]` | 漏项 → 该模块永远不被 skip（可能误覆盖本机运行时文件） |
| `reapply_applied_runtime.rs:105` | `reapply_cli(&mut summary, "<tool>", ...)` 调用点 | 恢复后不 re-apply |

### 6.5 前端注册（全部会静默失效）

| 文件 | 符号 | 漏改后果 |
|------|------|---------|
| `useWSLSync.ts:56` / `:72` | `TAB_TO_MODULE` / `ALL_CODING_MODULES` | 映射为 `undefined` → `.filter(Boolean)` 丢弃 → 塞进 `skipModules` → **文件永不参与 WSL 同步**（复发 2 次的头号坑） |
| `useSSHSync.ts:23` / `:40` | 同上 | 同上（SSH） |
| `WSLSyncModal.tsx:74` / `:82` | `MODULE_TO_TAB` / `ALL_MODULE_KEYS` | 模块 tab 在同步弹窗里**看不见** |
| `SSHSyncModal.tsx:104` / `:112` | 同上 | 同上 |
| `FileMappingModal.tsx:250` | `<Select.Option value="<tool>">` | 手动加映射时选不到该模块 |
| `SSHFileMappingModal.tsx:153` | 同上 | 同上 |
| `syncMessageTranslator.ts:182` | `BUILTIN_MAPPING_LABELS` 每个 mapping id 一条 | 同步结果消息里显示原始 id 而非本地化名 |

### 6.6 本节的静默失效陷阱汇总

1. **`default_mappings()` 加了结构体但没加进 `DEFAULT_MAPPING_IDS_ADDED_IN_V<N>` / 没 bump `CURRENT_DEFAULTS_VERSION`**：老用户永远拿不到新映射，**只有全新安装才有**。无任何提示。
2. **`resolve_dynamic_paths_with_db` 漏 arm**：映射保留字面 `~/.<tool>/...`，用户设了自定义根目录时同步的是**错误路径**（通常表现为 skipped 而非 error）。
3. **`TAB_TO_MODULE` 漏项**：复发 3 次的头号坑（`AGENTS.md:1111-1113` 有完整记录）。
4. **`runtime_location::MODULE_KEYS` 漏项**（issue #331）：模块不在 Direct 状态列表里 → 后端不知道它已是 WSL Direct → 把 Windows UNC 路径 `//wsl.localhost/...` 交给 Linux `cp`。
5. **`is_mapped_mcp_config_file` 注册了但 `strip_cmd_c` 没 arm**：`cmd /c` 原样进 Linux（ZCode 实证）。
6. **`wsl_module_for_reapply_label` 漏 arm**：恢复后 WSL sync 把该模块当「未变更」跳过。
7. **`SKILLS_TARGETS_FROM_RUNTIME_LOCATION` 漏 key**（`ssh/skills_sync.rs:113`）：WSL Direct 自定义根目录在 SSH 远端静默退回默认目录，同步「成功」但写到 CLI 不读的位置（见 §7.2.1）。**已由源码扫描型测试钉住**。

---

## 7. 阶段七：Skills / MCP 管理

### 7.1 核心事实：工具枚举只有一个权威来源

`tauri/src/coding/tools/builtin.rs:14` 的 `BUILTIN_TOOLS: &[BuiltinTool]` 是**唯一**的工具枚举常量表：

```rust
pub struct BuiltinTool {
    pub key: &'static str,
    pub display_name: &'static str,
    pub relative_skills_dir: Option<&'static str>,
    pub relative_detect_dir: Option<&'static str>,
    pub mcp_config_path: Option<&'static str>,
    pub mcp_config_format: Option<&'static str>,  // "json"|"toml"|"jsonc"|"yaml"|"cordis"
    pub mcp_field: Option<&'static str>,
}
```

zcode 的 entry（`builtin.rs:384-392`）：

```rust
BuiltinTool {
    key: "zcode", display_name: "ZCode",
    relative_skills_dir: Some("~/.zcode/skills"),
    relative_detect_dir: Some("~/.zcode"),
    mcp_config_path: Some("~/.zcode/cli/config.json"),
    mcp_config_format: Some("json"),
    mcp_field: Some("mcp.servers"),   // 支持点分嵌套路径
},
```

**加一条 entry 就够**：`get_all_builtin_tools()` / `get_skills_builtin_tools()` / `get_mcp_builtin_tools()` / `get_all_tool_adapters()` / `get_mcp_runtime_tools()` 全是派生函数，MCP 页、Skills 页、托盘 MCP 区、托盘 Skills 区、以及它们的 WSL/SSH 同步**全部自动生效**。

**前端零改动**（除图标）：`useMcpTools.ts` 和 `SkillsSettingsModal.tsx` 的数据都来自后端命令。

> **路径必须与模块 Source of Truth 一致**：kimi 曾误写 `~/.kimi/*`，实际是 `~/.kimi-code/*`。
>
> **强制复制**：不支持 symlink 的 CLI 要在 `builtin_tool_forces_skill_copy()`（`builtin.rs:436`）加 key（当前只有 `cursor` / `antigravity_cli`）。

### 7.2 但路径解析有 6 处独立于 `BUILTIN_TOOLS` 的 match

这些是真正的「漏改静默失效」区，文件 `tauri/src/coding/tools/detection.rs`：

| 函数 | 行号 | 说明 |
|------|------|------|
| `resolve_special_mcp_config_path` | `:48` | OS 特殊路径（opencode / github_copilot_intellij / claude_desktop / hermes / dsh） |
| `is_tool_installed` 的 special 列表 | `:94` | 同上 5 个 key |
| `resolve_special_mcp_config_path_with_db` | `:172` | 按需查 DB 的**特例**（hermes / dsh） |
| `resolve_mcp_config_path_with_db` | `:190` | DB 优先解析 → 特殊 → `runtime_location` → 静态兜底 |
| `resolve_mcp_config_path_with_db_async` | `:201` | 同上（async） |
| `resolve_special_skills_path_with_db` | `:216` | 同上（hermes） |
| `resolve_skills_path_with_db` | `:228` | 同上 |
| `resolve_skills_path_with_db_async` | `:237` | 同上 |

#### 7.2.1 SSH Skills 目标解析的独立白名单（第 9 处）

`tauri/src/coding/ssh/skills_sync.rs` 的 `SKILLS_TARGETS_FROM_RUNTIME_LOCATION`（`:113`）是**又一份独立于 `runtime_location` 的白名单**：

```rust
const SKILLS_TARGETS_FROM_RUNTIME_LOCATION: &[&str] = &[
    "claude_code", "codex", "grok", "kimi", "opencode", "openclaw",
    "pi", "oh_my_pi", "gemini_cli", "zcode", "omo_native",
];
```

**语义**：名单内的工具，远端 Skills 目录跟随 `runtime_location::get_tool_skills_path_async` 解析（WSL Direct 自定义根目录时用 UNC 解析出的 Linux 路径）；名单外的退回 `get_remote_tool_skills_dir()` 的 `BUILTIN_TOOLS.relative_skills_dir` 字面值。

**漏项的后果**：WSL Direct 自定义根目录在 SSH 远端**静默退回默认目录**，同步「成功」，但文件写到 CLI 不读的位置。

**维护规则**：这份名单必须 = `runtime_location::get_tool_skills_path_async`（`:2025`）的所有 arm，**减去 hermes**（hermes 是文档化例外：文件映射固定写 `~/.hermes/*`，Skills 也固定写 `~/.hermes/skills`，见 `ssh/AGENTS.md:56`）。

> 2026-10-07 复查发现 `zcode` 与 `omo_native` 两个 key 都漏了——两者在 `get_tool_skills_path_async` 里都有 arm（`:2082` / `:2146`），在 `BUILTIN_TOOLS` 里也都有 `relative_skills_dir`，所以 `get_all_skill_tool_keys()` 会遍历到它们，但目标路径永远走回退分支。
>
> **已加源码扫描型回归测试**（`ssh/skills_sync.rs` 的 `runtime_location_skills_tools_are_all_covered`）：测试直接 `include_str!("../runtime_location.rs")` 解析 `get_tool_skills_path_async` 的函数体，抽出所有 `"<key>" =>` arm，与本名单断言相等（预期集 = 名单 + `hermes`）。**名单再漂移，测试立刻失败**，不需要人肉 `grep`。改 `get_tool_skills_path_async` 的 arm 时记得同步这个测试的预期。

配套的 backend resolver 在 `runtime_location.rs` 的 `get_tool_mcp_config_path_sync/_async`（`:2249` / `:2278`）和 `get_tool_skills_path_sync/_async`（`:1922` / `:2025`）。

> **历史上的坑（已结构性修复，2026-10-06）**：`*_with_db*` 原本是**白名单 match**，每个 CLI 在 `detection.rs` 里抄一份自己的 key，zcode 不在其中 → `_ =>` 走静态路径。后果：**用户在「自定义配置目录」里改了 root_dir 后，MCP 页读写的仍是默认路径——无报错、无日志。**
>
> **修法不是「把 zcode 加进白名单」，而是把白名单反过来**：
>
> ```rust
> pub fn resolve_mcp_config_path_with_db(db, tool) -> Option<PathBuf> {
>     resolve_special_mcp_config_path_with_db(db, tool)   // 只剩 hermes/dsh 这类真特例
>         .or_else(|| runtime_location::get_tool_mcp_config_path_sync(db, &tool.key))
>         .or_else(|| resolve_mcp_config_path(tool))      // 静态默认值兜底
> }
> ```
>
> 共享 resolver 自己处理未知 key（返回 `None` → 走兜底），调用侧不维护副本列表——这样**以后新增 CLI 不需要改这里**。这是 13.1 模式一的标准对策，新增 CLI 时若发现某处仍是白名单 match，**优先按这个方向改，而不是补一行 key**。

### 7.3 前端图标（MCP 与 Skills 共用）

`web/features/coding/shared/toolIcon/ToolIcon.tsx` 有 5 级解析顺序（见 `skills/AGENTS.md:76`）：

| 优先级 | 位置 | 形态 |
|--------|------|------|
| 0 | `:191` | `claude_code` / `pi` / `oh_my_pi` / `openclaw` 硬编码分支 |
| 1 | `:122` `RAW_SVG_MARKS` | `?raw` 内联 SVG |
| 2 | `:140` `PNG_ICON_URLS` | PNG `<img>` |
| 3 | `:84` `TOOL_ICON_RENDERERS` | LobeHub 组件（zcode: `Zhipu.Color`） |
| 4 | `:232` | 自定义工具的 `iconUrl` |
| 5 | `:257` | 两字母兜底徽标 |

**漏加 = 静默降级成兜底徽标，不报错。**

### 7.4 MCP 显示名覆盖

`tauri/src/coding/mcp/mod.rs:22` 的 `mcp_tool_display_name()` 只在需要改名时动（当前只有 `github_copilot`）。

---

## 8. 阶段八：备份 / 恢复

### 8.1 管线（三渠道共用）

```
generate.rs::generate_backup_file
  └─ utils.rs::create_backup_zip
       └─ utils.rs::write_backup_zip_contents
            └─ utils.rs::write_external_configs_to_backup_zip   ← 打包：改这里

restore.rs::restore_from_archive                                 ← 恢复：改这里
  ↑ local.rs / webdav.rs / repository.rs 全部只调用它
```

**重要**：`local.rs` / `webdav.rs` / `repository.rs` **没有** per-tool 的 restore 分支，三渠道只做薄包装。新增 CLI **不需要**动这三个文件。

### 8.2 必改的代码位置

#### A. 分类清单（决定「开关关闭时是否仍进包/恢复」）

| 位置 | 符号 | 说明 |
|------|------|------|
| `utils.rs:1494` | `const ALWAYS_BACKUP_CLI_TOOLS`（7 个） | 运行时文件为真源、不可 re-apply → 开关关闭也进包 |
| `utils.rs:1504` | `const OPTIONAL_BACKUP_CLI_TOOLS`（8 个） | SQLite 为真源、可 re-apply → 受 `backup_cli_config_files_enabled` 门控 |
| `utils.rs:1515` / `:1519` | `is_always_backup_cli_tool()` / `is_optional_backup_cli_tool()` | |
| `utils.rs:1610` | `wsl_module_for_external_config_tool()` match | **漏 arm → 恢复出的文件永远不会被 post-restore WSL sync 传播**（拿不到 module 就静默 return） |
| `utils.rs:1595` | `record_restored_external_config_wsl_module()` | 依赖上面那个 match |

> 这两个常量是 **§8.3 第 1 条陷阱的另一半**：`should_skip_external_config_on_restore()` 的兜底是 `_ => true`（跳过）。新增工具若只写了打包段、忘了登记进这两个常量之一，**开关关闭的机器上恢复时它的文件被静默跳过**。

#### B. 打包写入（`write_external_configs_to_backup_zip`，`utils.rs:3021`）

每个工具一段，标准形态是「写 `external-configs/<tool>/` 目录 entry + 写 `root-dir.txt` + 逐个文件调 `add_external_config_file_to_zip`」。

配套 helper：
- `utils.rs:2928` `add_external_config_file_to_zip(...)`
- `utils.rs:2954` `add_external_config_directory_contents_to_zip(...)`（目录递归）
- `utils.rs:390` `harden_restored_sensitive_file()`（敏感文件 0600）
- 每个工具的 `get_<tool>_*_path_from_db()`
- **无 DB 的 restore 期 fallback**：`utils.rs:425` → 调 `runtime_location::resolve_<tool>_root_dir_without_db()`

> ⚠️ `zip::ZipWriter` 不允许重复 entry。目录 entry 必须走 `add_directory_to_zip_once`（带 `added_zip_directories: &mut HashSet<String>` 幂等），否则自定义根目录 + 配置文件同时存在时报 `Duplicate filename`。

#### C. 恢复解压（`restore.rs`）

`restore.rs` 里是一长串 `else if file_name.starts_with("external-configs/<tool>/")`，**当前 15 个分支**。新增工具要加三处：

1. **root-dir override 读取**（`restore.rs:328` 是 opencode 范本；`should_use_root_override_for_tool` 用**工具名**不是模块名，如 `"geminicli"` / `"omo_native"`）
2. **解析 restore dir**（`restore.rs:413` 是范本；fallback 目录要与 `runtime_location::resolve_<tool>_path_without_db` 的默认值一致）
3. **解压分支**（`restore.rs:854` 是 zcode 范本）。分支内必须：
   - 跳过空/目录/`root-dir.txt`（**漏了会在 CLI 数据根目录留下垃圾文件**）
   - `should_filter_external_config_entry(&filter_rules, "<tool>", relative_path)`
   - `resolve_external_config_restore_output_path(&<tool>_restore_dir, restore_relative_path)?`（**安全 helper，禁止直接 `join`**）
   - `record_restored_external_config_wsl_module(&mut restored_wsl_modules, "<tool>")`
   - 敏感文件（`auth.json` / `.credentials.yaml` / 含 key 的 `config.toml`）调 `harden_restored_sensitive_file(&outpath)?`

#### D. 其余备份相关清单

| 位置 | 符号 | 漏改后果 |
|------|------|---------|
| `utils.rs:1035` | `backup_filter_option_path()` match | 过滤规则下拉显示裸相对路径而非 `~/.<tool>/...` |
| `utils.rs:1112` | `list_backup_file_filter_path_options()` | 「文件过滤规则」里没有该工具的任何可选项 |
| `utils.rs:2845` | `normalize_backup_filter_rule_path()` 的 `tool_prefixes` match | 用户写的 `~/.<tool>/AGENTS.md` **归一化不到** `AGENTS.md`，规则静默不匹配（**安全影响**） |
| `utils.rs:1893` | `get_custom_root_dir_path_info()` match | `_ => None` → 归档里不写 `root-dir.txt`，自定义根目录不被备份 |
| `utils.rs:1654` | `clear_restored_cli_custom_roots()` | `skip_cli_custom_roots=true` 时旧机器路径残留 |
| `restore.rs:1289` | `write_post_restore_flags(...)` | 恢复后不触发 WSL resync |
| `db/schema.rs` / `db/migrations.rs` | `DbTable` 枚举 + 建表 | **编译器会抓**（exhaustive match），非静默 |
| `BackupSettingsModal.tsx:72` | `TOOL_ORDER` | 该工具不出现在备份设置的排序里 |
| i18n ×2 | `settings.backupSettings.cliConfigFilesDesc` / `restoreWillReapply` / `fileFilterRules.disabledByCliConfigFiles` | **三段文案各自点名工具列表**（always 组 + DB 型组），中英各一份 = 6 处。2026-10-07 复查时这三段都只列了 3 个 always 工具（实际 7 个）、optional 组还漏了 zcode/antigravity/claude_desktop。**改 `ALWAYS_`/`OPTIONAL_BACKUP_CLI_TOOLS` 时必须同步这 6 处**，否则用户读到的是过期的规则说明 |

### 8.3 备份侧的静默失效陷阱

1. **`should_skip_external_config_on_restore()` 的兜底是 `_ => true`（跳过）**。新增工具若**只写了打包段**、忘了登记进 `ALWAYS_`/`OPTIONAL_BACKUP_CLI_TOOLS`，那么在开关关闭的机器上恢复时，它的文件被**静默跳过**，恢复结果 `success=true`、0 warning。
2. **`wsl_module_for_external_config_tool` 漏 arm**：恢复出的文件永远不会被 post-restore WSL sync 传播。无日志。
3. **`restore.rs` 漏分支**：`else if` 链走到底什么都不做，entry 被丢弃，恢复「成功」。**这是本组里最严重的一条**——打包侧完整、恢复侧静默丢弃，用户换机后整个 CLI 的配置全没了却看到「恢复成功」。
   > 2026-10-07 实证：`omo_native` 是当时唯一「在打包段里、却不在恢复分支里」的模块（其余 14 个都有分支）。**已加回归测试**（`restore.rs` 的 `coverage_tests::every_backed_up_external_config_tool_has_a_restore_branch`）：扫描自身源码里所有 `file_name.starts_with("external-configs/` 标记，抽出工具名集合，对 `ALWAYS_` + `OPTIONAL_BACKUP_CLI_TOOLS` 逐个断言命中。新增工具若只改打包段，测试立刻失败。
   >
   > **自查命令**（两条输出应一一对应）：
   > ```bash
   > grep -on '"external-configs/[a-z_]*' tauri/src/settings/backup/utils.rs | sed 's/.*external-configs\///' | sort -u
   > grep -on 'external-configs/[a-z_]*' tauri/src/settings/backup/restore.rs | sed 's/.*external-configs\///' | sort -u
   > ```
4. **打包段漏写但没漏 `root-dir.txt`**：`root-dir.txt` 被当普通 entry 解压到 `<restore_dir>/root-dir.txt`，留下垃圾文件。
5. **归档路径扁平化**：zcode 曾把 `v2/provider_config.json` 存成 `provider_config.json`，恢复后文件落在 CLI **不读取**的位置（commit `312529b7` 修复，回归测试 `utils.rs:4560`）。**归档 entry 的相对路径必须保留 data-root 下的子目录层级。**
6. **`tool_prefixes` 漏项**：过滤规则「存在即生效」，归一化失败 = 用户以为排除了敏感文件，实际照打进去。

---

## 9. 阶段九：Gateway 接管（可选）

- [ ] `GatewayCliKey` 加枚举值
- [ ] cli_proxy manifest（受管字段、入站路由）
- [ ] `ensure_<tool>_gateway_direct` 直连拒绝
- [ ] reapply 锁定重灌
- [ ] 前端接管入口 + 状态胶囊
- [ ] 3.6 全部清单

> Kimi 踩坑：受管字段写死 `[providers."managed:kimi-code"]`，自定义 provider 流量绕过网关导致统计恒 0。应按 `default_model → models.<key>.provider` 动态解析。

---

## 10. 阶段十：会话管理（可选）

- [ ] `tauri/src/coding/session_manager/mod.rs` 的 `SessionTool` 枚举加值
- [ ] `session_manager/<tool>.rs` 实现解析
- [ ] 导入快照路径必须走 `join_safe_relative`（kimi review 轮发现路径遍历）
- [ ] 前端 `shared/sessionManager/` 接入

---

## 11. 阶段十一：cli_resolver 与「更多选项」

- [ ] `tauri/src/coding/cli_resolver.rs` 加 `resolve_local_<tool>_program()`
- [ ] 全局 bin 候选覆盖（nvm/volta/fnm/bun/mise/asdf）
- [ ] Windows spawn 加 `CREATE_NO_WINDOW`
- [ ] `web/components/common/CliManualPathSetting.tsx` 入口
- [ ] 版本探测：`--version`/`-v`/`version`，async command + 有界超时

---

## 12. 验收

### 12.1 构建与测试

- [ ] `pnpm exec tsc --noEmit` + `pnpm i18n:check` + `pnpm test:web` 全绿
- [ ] **`pnpm run test:provider-list-drag` 通过**（13.1 模式六十）——供应商列表的拖拽用真指针事件验证；只要这轮碰了卡片、列表外壳或排序/搜索，就跑它
- [ ] **`pnpm run test:provider-card-layout` 通过**（13.1 模式四十六）——卡片必须是 `providerCardVariants` 的薄映射层；迁完一个 CLI 要**同步从脚本的 `PENDING_MIGRATION` 删掉它的条目**，守卫是棘轮，两个方向都会报错
  ```bash
  # 名单里的文件是否真的还没迁完？（输出应为空；有输出说明条目该删了）
  node scripts/verify-provider-card-layout.mjs
  ```
- [ ] **迁移共享组件时，先比对旧 key 与共享 key 的中英文**（13.1 模式四十七）——中文相同就统一取共享 key，把英文措辞变化写进提交说明，然后删掉孤儿 key
  ```bash
  # 待迁 key 与目标共享 key 是否同义（zh 相同 → 应该统一）
  node -e "
  const zh = require('./web/i18n/locales/zh-CN.json');
  const en = require('./web/i18n/locales/en-US.json');
  const get = (d, k) => k.split('.').reduce((a, p) => a?.[p], d);
  for (const [from, to] of [['claudecode.viewDocs', 'common.viewDocs']]) {
    console.log(from, 'zh:', get(zh, from) === get(zh, to), 'en:', get(en, from) === get(en, to));
  }
  "
  ```
  > 删孤儿 key 时**只删真正无引用的**：`grep -rn "t('<tool>.<key>'" web` 输出为空才删。相邻的 key 可能仍被别的入口用（如 `claudecode.rootPathSource.customize` 可删，`claudecode.rootPathSource.modal.*` 必须留）。
- [ ] **共享组件声明的可选 prop 都有渲染点**（13.1 模式二 / #70、#97）：给 `ProviderCardVariantProps` 之类加了新 prop 后，确认**每个样式组件里都指认了一次它渲染在哪**——类型里有、组件里没渲染 = 传了等于没传
- [ ] `cargo check`（或 `cargo test`）通过
- [ ] `pnpm build` 成功
- [ ] 至少一条「表单提交 → 持久化 → 再读取」的往返用例
- [ ] **测试夹具里没有开发者的真实信息**（13.1 模式三十八）——本机用户名、真实工作目录、真实渠道 key、真实实例名，以及**输入框 placeholder**（最容易漏，它不在测试文件里）
  ```bash
  # 逐条替换成通用名（tester / sample-workspace / example-provider），输出应为空
  grep -rn "<本机用户名>" --include=*.rs --include=*.ts --include=*.tsx tauri/src web
  grep -rn "<真实工作目录名>\|<真实渠道 key>" --include=*.rs --include=*.ts --include=*.tsx tauri/src web
  ```
  > **别误伤产品数据**：`tauri/resources/*.json`（`preset_models.json` / `dsh_builtin_models.json`）里打包的模型名是公开数据，测试里可以照用；注释里引用**公开开源项目**的 commit 也保留——先确认那个项目是公开的。
- [ ] 3.4 的全局 grep 兜底通过
- [ ] **动态 i18n 前缀已登记进工具的保护表**（13.1 模式二十二）：只要页面用了会拼 key 的共享组件（`useRootDirectoryConfig`、`GlobalPromptSettings` 的 `translationKeyPrefix`、`ModelFormModal` 的 `i18nPrefix`…），就把该模块的前缀加进 `scripts/i18n-keys.mjs` 的 `DEFAULT_DYNAMIC_IDENTIFIER_VALUES_BY_FILE`。**漏登记 = 那一整块文案会被 `i18n:prune` 判为未使用并删除**
  ```bash
  # 每个用 translationKeyPrefix 的页面，其前缀都应出现在工具的保护表里
  for p in $(grep -rho "translationKeyPrefix: '[^']*'" web/features/coding/*/pages/*.tsx | sed "s/.*'\(.*\)'/\1/" | sort -u); do
    grep -q "'$p'" scripts/i18n-keys.mjs || echo "UNREGISTERED PREFIX: $p"
  done
  ```
  输出应为空。**补完保护表后必须重跑 `i18n:check`**——它会把之前被掩盖的真实缺口一次性暴露（缺 key 的模块会立刻报错），逐条补齐，不要用「加白名单消音」收场。
- [ ] **自动清理前先抽查**：跑 `i18n:prune --write` 或任何批量删除之前，手工确认几条被判「未使用」的 key 真没人用（尤其名字带 `Hint` / `Placeholder` / `Success` 的）。工具的保护表过期时，它会主动给出**错误**的删除建议（13.1 模式二十二）
- [ ] **模型/供应商字段形状与引擎核对**（13.1 模式二十三）：本 CLI 与哪个已有 CLI **运行时同源**（读同一个文件？同一个二进制）？照着它核对每个写入键，并**以该引擎自己的 `docs/models.md`（或等价 schema）为准**——兄弟页面的代码也可能是错的
  ```bash
  # 1. 找出本 CLI 实际写的模型键。
  #    两种写法都要抓：直接赋值 `nextModel.foo = ...`，以及经 helper 传字符串键名
  #    的 `setOptionalStringField(nextModel, 'foo', ...)`——后者用点号模式抓不到。
  grep -rho "nextModel\.[a-zA-Z]*" web/features/coding/<tool>/ | sed 's/nextModel\.//' > /tmp/written.txt
  grep -rho "nextModel, '[a-zA-Z]*'" web/features/coding/<tool>/ | sed "s/nextModel, '//;s/'//" >> /tmp/written.txt
  sort -u /tmp/written.txt
  # 2. 逐个在上游字段表里找得到吗？（以 OmO Native 为例，文档随引擎安装）
  ls ~/.omo/binary-runtime/*/docs/models.md
  # 3. 对同一个真实文件列出引擎自己写的键，与第 1 步求差集
  node -e "const j=require(process.env.USERPROFILE+'/.omo/agent/models.json');const s=new Set();for(const p of Object.values(j.providers||{}))for(const m of(p.models||[]))Object.keys(m).forEach(k=>s.add(k));console.log([...s].sort().join('\n'))"
  ```
  > 第 3 步的样本文件只包含**用户已经用到的**字段——差集非空**不一定是错**。正确做法是拿第 1 步的每个键去**上游字段表**（第 2 步的 `docs/models.md`）核对：表里有 → 合规的可选字段；表里没有 → 抄错了兄弟。
  **差集里只应有「本 CLI 有意支持的可选字段」**；出现引擎从不写的键（如 `thinking`）就是抄错了兄弟。同理，**共享弹窗交回的 JSON 字符串必须 parse 后再落盘**（数组/对象），不能就地赋值——`web/test/features/coding/omo_native/utils/omoNativeProviders.test.ts` 是可抄的回归测试样例
- [ ] **死封装核对**（13.1 模式十四）：新 CLI 的每个前端 API 封装、每个后端 command，都要有 UI 调用点。只有定义没有调用的，要么接上、要么删掉——**留着会被后来者当成「功能已完成」**
  ```bash
  # 对每个新封装名跑一次；除 services/ 下的定义外，应至少有一个调用点
  for api in $(grep -o "^export const [a-zA-Z]*" web/services/<tool>Api.ts | awk '{print $3}'); do
    n=$(grep -rn "\b$api\b" web/ --include=*.ts --include=*.tsx | grep -v "services/" | wc -l)
    [ "$n" -eq 0 ] && echo "UNUSED API: $api"
  done
  ```
  输出应为空（或每一项都有「有意保留」的理由）。
- [ ] **读写键同名核对**（13.1 模式十六）：新页面里每一对「读一个 key / 写一个 key」都同名——侧栏折叠键、`visible_tabs`、模块名、事件名。从别的 CLI 复制页面时，读侧最容易残留来源 CLI 的键
  ```bash
  for f in web/features/coding/*/pages/*.tsx; do
    r=$(grep -o "sidebarHiddenByPage\.[A-Za-z_]*" "$f" | head -1 | cut -d. -f2)
    w=$(grep -o "setSidebarHidden('[a-z_]*'" "$f" | head -1 | sed "s/setSidebarHidden('//;s/'//")
    [ -n "$r" ] && [ -n "$w" ] && [ "$r" != "$w" ] && echo "MISMATCH $f: reads=$r writes=$w"
  done
  ```
  输出应为空。
- [ ] **集合对账：备份打包段 ↔ 恢复分支**（13.1 模式四十四）。两条命令的输出必须**完全相等**，缺一个工具就是「备份里有、恢复时静默丢弃」——这是备份链路唯一会让用户**真丢数据**的漏法
  ```bash
  diff <(grep -on '"external-configs/[a-z_]*' tauri/src/settings/backup/utils.rs | sed 's/.*external-configs\///' | sort -u) \
       <(grep -on 'external-configs/[a-z_]*' tauri/src/settings/backup/restore.rs | sed 's/.*external-configs\///' | sort -u)
  ```
  输出应为空。`restore.rs` 的 `coverage_tests::every_backed_up_external_config_tool_has_a_restore_branch` 已把这条不变量固化成测试，跑测试也能发现。
- [ ] **集合对账：SSH Skills 名单 ↔ `get_tool_skills_path_async`**（13.1 模式四十五，见 §7.2.1）。已由 `ssh/skills_sync.rs` 的 `runtime_location_skills_tools_are_all_covered` 固化——**新增 CLI 若在 `get_tool_skills_path_async` 里加了 arm，这个测试会立刻失败**，按提示同步名单（或按 §7.2 的方向把白名单反过来）
- [ ] **模块级 `AGENTS.md` 已建，且已登记进根 `AGENTS.md` 的 Index 表**（13.1 模式十二）
  ```bash
  # 前端：每个 feature 模块
  for d in web/features/coding/*/; do [ -f "$d/AGENTS.md" ] || echo "MISSING $d"; done
  # 后端：编码域 + 非编码域（db / settings / resources 都要覆盖）
  for d in tauri/src/*/ tauri/src/coding/*/; do
    case "$d" in *fixtures*|*transformer/*|*runtime/*) continue;; esac
    [ -f "$d/AGENTS.md" ] || echo "MISSING $d"
  done
  ```
  **两条命令都要跑**，输出应为空（有正当例外的目录要在本 SOP 里显式登记，见下方说明）；再确认新模块在根 `AGENTS.md` 的 Index 表里有一行。

  已知例外（不要求建文档）：`web/features/coding/claude/` 是占位 stub（未接入路由）；`web/services/` 是扁平 API 封装目录，不是模块。

  > **教训 #51**：只跑前端那条命令会漏掉后端。`tauri/src/db`、`tauri/src/settings`、`tauri/src/coding/zcode` 三个模块就是这样被漏掉的。审计必须前后端各一遍。

### 12.2 端到端行为（逐条对照前面各节）

- [ ] **本机**：页面能加载配置、增删改供应商、应用默认、模型 CRUD
- [ ] **WSL Direct**：路径解析正确、CLI 调用走 `wsl -d <distro> --exec`（6.1）
- [ ] **托盘**：新 CLI 的供应商/提示词子菜单**真实出现**且点击生效；中英两种语言标题都不为空（5.2 第 3 点）
- [ ] **托盘区块与页面区块一一对应**：把页面 sidebar 的区块名抄下来，逐个在托盘菜单里找入口。**页面上有的能力，托盘里必须有对应项**（2026-10-07：OmO 托盘缺「默认模型」「全局提示词」，与 Pi 不一致）
- [ ] **托盘的「当前值」回显正确**：子菜单标题括号里显示的是当前生效项，且点击后刷新（不是点完还显示旧值）
- [ ] **WSL/SSH 同步**：新映射在**老库**上也生效（`CURRENT_DEFAULTS_VERSION` 已 bump，6.2）；同步结果消息显示本地化名而非裸 id
- [ ] **MCP 页 / Skills 页**：新工具出现、图标正确、读写路径在自定义根目录下也对（7.2 的白名单）
- [ ] **备份/恢复**：打包 → 换机恢复 → 文件落在 CLI 能读到的位置；恢复后 WSL 自动 resync（8.3）
- [ ] 主窗口保存、托盘刷新、WSL 设置页状态三者一致
- [ ] 新 tab 的侧栏开关重启后保持
- [ ] 新 tab 的配置文件在 WSL/SSH 同步中**不被静默跳过**（`TAB_TO_MODULE` / `ALL_CODING_MODULES`）

### 12.3 视觉核对（**必做，不可用「编译通过」替代**）

**编译只保证类型正确，不保证形态正确。** 逐项在界面上指认，对照 §4.0.1 指定的参照 CLI：

- [ ] **页面上每一个区块都能折叠/展开，且形态与参照 CLI 一致**——「模型设置」这类区块在 Pi / OpenCode 里是**不可折叠的卡片**（`modelCard` + `Title level={5}`），别用 `Collapse`（见 13.1 模式三十九）
- [ ] **页面上每个展示位，用户都能自己产生那份数据**。逐个问：「我（新用户）能从哪里让这里出现内容？」答不上来的展示位要么删掉、要么写清它是只读的（见 13.1 模式三十五）
- [ ] **预览弹窗显示的是文件全文**，不是本页拥有的那一片段。带注释的文件要用 `jsonc` 语言，否则满屏红波浪线（见 13.1 模式三十六）
- [ ] **每个「保存」按钮的 loading 只覆盖写入，不覆盖刷新**：点保存 → 应立刻关闭/停止转圈，而不是等重查列表（见 13.1 模式三十七）
- [ ] 供应商弹窗：标签在左（`layout="horizontal"`）、第一行是渠道、API Key 有显示/隐藏按钮
- [ ] 供应商弹窗：**没有**该 CLI 用不上的分区（非网关 CLI 不应出现计费/请求头/改写，见 4.0.3）
- [ ] 供应商卡片：详情**一行**、应用按钮是蓝色文字（`type="link"`）
- [ ] **卡片内的次要操作全部无边框**（`type="text"` + `fontSize: 12`）；逐个摸一遍边框，多一个带框的按钮就是不一致（见 4.0.2-E / #59）
- [ ] **同款动作跨 CLI 同款**：把参照 CLI 和本 CLI 的同一动作（删除、编辑、复制、分享、测试）并排看，样式与图标一致
- [ ] 模型列表工具栏：按钮**数量与参照一致**（逐个指着数，见 4.0.2-F）
- [ ] **模型行**：编辑 / 复制 / 删除 / 设为主模型**四个按钮都在**（附录 B.1）
- [ ] 页面头部：「预览配置」「自定义配置目录」「打开文件夹」「刷新配置」「更多选项」**五个都在**（附录 B.1）
- [ ] 供应商列表：提示块在、底部导入按钮在（附录 B.1）
- [ ] **官方账号区块在列表里的位置**：它**不是** provider，但用户会期望它和供应商卡片并排——所以它必须是列表成员（走调用方的 `children` + `alwaysVisible`，自带 `marginBottom`，位置自己持久化），**不要**塞进工具栏或 `footer` 就算完（见 13.1 模式六十一、#114）。**真的拖动它**，确认能参与排序、位置被记住、且**空列表/搜索无结果时它仍然在**（附录 B.2）。**卡片布局下它还得是一个普通格子**：与邻居同宽同高、第二行只说「当前登录: <账号>」——**并且逐个状态看一遍：官方渠道在跑 / 只有磁盘上那份登录 / 两者都没有（未登录）**，三态都不许留空行（把凭据文件改掉或用 `accounts=idle` 夹具复现；§4.2.9 的卡片模式表、模式九十九～一百零二），账号管理器进它自己的详情侧栏（常开无 caret），**拖它不能顺带打开侧栏**
- [ ] 空态 / 搜索空态 / 加载态文案与参照一致
- [ ] **文案里的断言仍然成立**：逐个读区块说明、按钮 tooltip、空态措辞，确认没有过期断言（见 13.1 模式三十四）。重点词：`只读` / `不可编辑` / `不支持` / `不能` / `仅展示`，以及 en-US 的 `read-only` / `display only` / `cannot be edited`
- [ ] **把列表里的卡片随机调换顺序**，每两张之间都还有间距（见 13.1 模式十）
- [ ] **每个筛选/下拉都试一遍**，选中后列表立刻跟着变（见 13.1 模式十一的对立面与 #49）
- [ ] 失败路径有可见反馈，不是静默空列表（见 13.1 模式三）
- [ ] **每个弹窗读一遍从上到下**：每段说明文字问一次「它在解释谁」——解释不到任何控件的，位置就错了（见 4.0.2-C / #58）
- [ ] **每个有长耗时操作的弹窗**：在操作进行中**点取消**，确认取消按钮可点、能退出（见 4.0.2-A / #57）

> **核对的单位是「界面上的块」，不是「代码里有没有」。** 漏传一个 prop 在代码里毫无痕迹，只有把参照 CLI 和本 CLI 并排打开、一块一块数，才能发现。附录 B 的台账是这个动作的记录载体。

> 若本次无法运行界面，**必须在提交说明里写明「未做视觉核对」**。ZCode 这一轮 8 次返工全部出在这一步——每一处都是「编译通过但形态不对」。

#### 12.3.1 迁移类改动：三遍核对（功能项 / 形态 / 行为）

**迁移（bespoke → 共享组件）与「新写一个页面」是两类改动，核对方式不同。** 迁移的验收标准是**「和迁移前一模一样」**——不是「编译通过」，也不是「看起来差不多」。必须按下面三遍走，**每一遍都拿迁移前的实现当参照物**（`git show <迁移前的 commit>:<原文件>`）：

**第一遍：功能项对照（有哪些可点的东西）。** 把两边所有交互元素列成清单，逐项点名，而不是凭印象扫一眼。可点的东西包括 `onClick` / `onChange`、菜单项、开关、拖拽把手、复选框——**不只按钮**。
> ```bash
> # 迁移前 / 后各跑一次，diff 两份输出；只该剩下写法差异（如 `() => void f()` vs `f`）
> git show <before>:<file> | grep -oP 'onClick=\{[^}]*\}|onChange=\{[^}]*\}' | sort -u
> grep -oP 'onClick=\{[^}]*\}|onChange=\{[^}]*\}' <file> | sort -u
> ```
> 同时对照 i18n key 清单（`t('<key>')` 去重排序后 diff）——key 从卡片消失**只有两种合法原因**：搬进了共享组件，或有意统一到 `common.*`。两者都要能指着新代码说出落点。

**第二遍：形态对照（每个东西长什么样）。** 见 §12.3 主清单。重点是**菜单与列表的呈现细节**：图标、分隔线、副标题、`type`（`link` / `text` / 默认）、`danger`、tooltip、顺序。

**第三遍：行为对照（点了会发生什么）——最容易漏，必须真的点。** 前两遍都过了，功能仍可能是死的：**回调被接上了，但传的值不对**。
> 例（#104）：`onToggleDisabled` 从「antd 的 `onChange(checked)`」被收敛成「无参 `() => void`」，映射层只好自己重算新值，算成了**当前值**——开关看着正常、点下去没有任何反应，类型检查全通过。用户点了一下就发现了。
>
> **判别方法**：对每个有状态的交互（开关、复选框、拖拽、下拉选择），**点下去，然后确认界面与磁盘状态都变了**；对每个出网动作（测试连通性、获取模型、应用、网关接管），**点下去并确认成功提示或错误提示出现**。逐项在提交说明里写出「点了 X → 观察到 Y」。
>
> **传值的核对法**：一个回调把值**传进来**还是让调用方**自己算**？凡是「让调用方自己算」的契约（无参回调、只有 id 没有新值），都要检查调用方算的方向对不对——这类 bug 不报错，只表现为「点了没反应」。**优先让共享组件把值透传下去**（`onChange={onToggleDisabled}` 而不是 `onChange={() => onToggleDisabled()}`）。

**三遍都做完才算迁移完成。** 只做第一遍 = 东西都在但长得不对（#102）；只做前两遍 = 长得对但点不动（#104）。

> **第三遍的「点」必须是真事件。** 读一遍调用链确认「每层都按契约传参」**不算**做了第三遍——它能验证传参，不能验证这一串传参最终产生了用户能看到的结果。本轮的拖拽就是被这样「修好」了两次，第三次改用真指针事件才定位（#112、13.1 模式六十）。涉及**指针 / 键盘 / 焦点 / 滚动 / 拖放 / 悬停**的交互，走浏览器夹具：
>
> ```bash
> pnpm run test:provider-list-drag   # 真 Chromium + 真指针拖拽 + 落库回写核对
> ```
>
> 若确实无法运行界面（无浏览器 / 无头环境不可用），**必须在提交说明里写明「未做视觉核对」并列出待用户核对的清单**——不要用「链路已核对」充当已验收。

**第三遍的补充：被有意禁用的能力也要「点」。** 交互在某个状态下被有意关闭时（拖拽在非默认排序下禁用、按钮在锁定态禁用），核对清单里要**同时包含「该状态下它确实不可用」和「界面上说明了原因」**两条。前者防止「关不掉」，后者防止用户把正常行为报成 bug（#111、13.1 模式五十九）。

### 12.4 排查前先确认「跑的是当前构建」

UI 表现异常时，**先排除进程 stale**，再怀疑代码：

- [ ] 进程启动时间 vs `ai-toolbox.exe` 编译时间（进程更早 = 跑的是旧二进制）
- [ ] `PRAGMA user_version` vs `TARGET_SCHEMA_VERSION`（不相等 = 迁移没跑）

> 曾出现：前端 Vite 热更新到最新代码，后端进程却是 14 小时前启动的，DB 停在旧 schema。表现为「后端命令报错」，实际代码根本没生效，排查绕了很远。

### 12.5 建议加回归测试的位置

| 场景 | 参考 |
|------|------|
| 归档路径保留子目录层级 | `settings/backup/utils.rs:4560`（zcode 回归测试） |
| **打包段 ↔ 恢复分支一一对应** | `settings/backup/restore.rs` 的 `coverage_tests::every_backed_up_external_config_tool_has_a_restore_branch`（2026-10-07 补，源码扫描型） |
| **SSH Skills 名单 ↔ `get_tool_skills_path_async`** | `ssh/skills_sync.rs` 的 `runtime_location_skills_tools_are_all_covered`（2026-10-07 补，源码扫描型） |
| 业务 id 含冒号的往返读取 | `zcode/adapter.rs` 的 `managed_provider_id_survives_the_db_round_trip`（见 2.1） |
| 预设字段 → 本 CLI 字段的映射 | `web/test/features/coding/zcode/utils/zcodeModelFields.test.ts`（对着 `preset_models.json` 断言，数据漂移即失败） |
| 自定义根目录下 MCP/Skills 路径解析 | `tauri/tests/coding/tools/detection_paths.rs`（2026-10-06 补）。#13 这个 bug 拖了几轮没被发现，就是因为没有这个测试 |
| 外部来源导入（All API Hub / CC Switch）→ 本 CLI 供应商 | `web/test/features/coding/zcode/utils/zcodeImportMapping.test.ts`（两个 extractor 的字段映射，含「无 baseUrl 也无 key → null」） |
| 老库 backfill 默认映射 | `wsl/commands.rs` 的 versioned mapping 测试 |
| **供应商卡片必须是薄映射层** | `scripts/verify-provider-card-layout.mjs`（2026-10-07 补，源码扫描型棘轮；`pnpm run test:provider-card-layout`，已挂进 `pnpm test`）。作用域是**后缀集合** `*ProviderCard.tsx` + `*OfficialAccountCard.tsx`（2026-10-10 加，见 13.1 模式七十七：`ZcodeOfficialAccountCard` 同为列表成员却因名字不匹配从未被扫到）；同时校验「变体 prop 形状上声明的每个 prop 都在某个 style card 里有渲染点」 |
| 表单弹窗必须横向布局 | `scripts/verify-form-modal-layout.mjs`（同类脚本，`pnpm run test:form-modal-layout`） |
| 供应商列表拖拽**真的能拖** | `scripts/verify-provider-list-drag.mjs`（2026-10-08 补，**真 Chromium + 真指针事件**；`pnpm run test:provider-list-drag`，已挂进 `pnpm test`）。挂真实 `CodexPage`，断言：默认排序下拖拽改变渲染顺序**且**回写后端、非默认排序下无把手、排序控件说明原因 |
| **官方账号区块走共享组件**（不许再手写行级按钮） | `scripts/verify-official-accounts-shared.mjs`（2026-10-09 补，`pnpm run test:official-accounts-shared`，已挂进 `pnpm test`）。源码棘轮：扫 `*OfficialAccount*.tsx` 与 `*ProviderCard.tsx`，凡自己 `map` 出账号行（`officialAccounts.map(` 一类）者必须 import `shared/officialAccounts`；`PENDING_MIGRATION` 只减不增（当前 1：antigravity；geminicli / grok 已于 2026-10-20 迁入）。⚠️ 初版匹配的是 `anticon-swap` / `anticon-check`——**antd 渲染后的 class，源码里根本不存在**，于是三份手写实现全部漏检、报告「0 待迁移」的假绿（见 13.1 模式七十三） |
| **Kimi 官方账号区在官方卡片内部**（行几何 + 区块形状 + 折叠交互 + 虚拟行 + 拖拽） | `scripts/verify-kimi-official-account-card.mjs`（2026-10-10 扩充，`pnpm run test:official-account-card`）。挂真实 `KimiPage`，25 项：官方行是普通列表成员且带把手、账号行画在官方卡片内、**账号行右边缘 == 卡片内容右边缘**、**标题行是统一的 `🔗 官方账号 (n)`（带 glyph 与计数）**、**列表行保留 `账号列表` 与折叠开关**、**登录按钮在列表行右侧（`login.top > heading.bottom` 且 `login.left >= listTitle.right` 且中线差 ≤ 8px）**、**真的点一下折叠开关能收起再展开**、已应用行不画切换（画「默认」）、虚拟行只给「保存当前登录」、保存后变普通行、真指针拖动后 reorder 载荷只含 provider id、没有官方渠道时不渲染官方卡片。卡片一律按 `data-provider-id` 定位（标题文本已经不等于数据里的名字） |
| **ZCode 官方账号卡片**（standalone 宿主：形状 + 说明句样式 + 行缩进 + 折叠） | `scripts/verify-zcode-official-account-card.mjs`（2026-10-10 新增，`pnpm run test:zcode-official-account`，已挂进 `pnpm test`）。挂真实 `ZcodePage`，12 项：卡片按 `officialAccountIndex` 插在列表成员中间且带把手、标题行 `🔗 官方账号 (n)`、列表行 `账号列表` + 折叠开关、说明句在标题行与列表行之间、**说明句的 `getComputedStyle` 颜色 == `rgba(0, 0, 0, 0.45)`（antd secondary，与另外两家一致）且字号 12px**、**每个账号行左边缘 == 列表标题文本左边缘（容差 4px，箭头宽度来自图标字体）**、真的点折叠能收起再展开。见 13 节 130 |
| **卡片内各区右边缘对齐**（「右侧空了一块」类报障） | `scripts/verify-codex-official-accounts.mjs`（2026-10-10 扩充，`pnpm run test:codex-official-accounts`，已挂进 `pnpm test`）。挂真实 `CodexPage`，断言账号行右边缘 == 卡片内容右边缘 == 头部动作右边缘（防止区块被渲染进头部两栏的左栏里，见 13.1 模式六十三），以及 Codex 这一端的接线：标题行是共享标题（带 🔗 与计数）、第二行是切换说明（`auth.json`）而不是 endpoint、登录按钮在列表行右侧 |
| **卡片间距与内边距的原值**（「比之前小了」类报障） | `scripts/verify-provider-card-spacing.mjs`（2026-10-08 补，`pnpm run test:provider-card-spacing`，已挂进 `pnpm test`）。**只挂共享卡片组件本身**（不挂页面、无 IPC），断言 Claude 式标题/副标题 8px、Codex 式 4px。判据见 13.1 模式六十五：统一掉的两个值如果原本就是两个，用户会逐个发现 |

> **测试位置**：Rust 集成测试放 `tauri/tests/coding/<tool>/`（不存在的目录要新建并挂进 `tauri/tests/coding.rs`）；纯逻辑单测可直接放模块内 `mod tests`。前端测试放 `web/test/features/coding/<tool>/`，`pnpm test:web` 自动收集 `*.test.ts`。

---

## 13. 历史踩坑汇编

| # | 坑 | 后果 | 修复 |
|---|-----|------|------|
| 1 | `get_sidebar_hidden_by_page` 7-key 白名单 | 新 tab 侧栏开关重启还原 | 改为遍历 DB 全 key |
| 2 | `TAB_TO_MODULE` 只有 7 项 | 新 tab 文件永不参与 WSL/SSH 同步 | 补全映射 + `ALL_*` |
| 3 | dsh/Hermes 未登记 runtime_location | Windows UNC 路径传给 Linux `cp`（issue #331） | 注册模块 + 保存后刷新缓存 |
| 4 | kimi：`MODULES` subTabs 漏注册 | 侧边栏静默不显示 | 补 `MODULES` |
| 5 | kimi：Gateway 统计/定价/normalize 漏注册 | 统计页看不到 kimi | 三处补注册 |
| 6 | kimi：`usage_stats.rs` 漏 `cli_key_from_app_type` | 已落库请求行被静默丢弃 | 补映射 + 回归测试 |
| 7 | kimi：受管字段写死 managed provider | 自定义 provider 流量绕过网关 | 动态解析当前生效 provider |
| 8 | zcode：`tray_support.rs` 未接入 `tray.rs` | 托盘支持是死代码 | 端到端接线 |
| 9 | zcode：`is_mapped_mcp_config_file` 有映射但后处理无 arm | `cmd /c` wrapper 复制到 Linux | 加专用 processor |
| 10 | zcode：备份归档扁平化路径 | 恢复后文件在 CLI 不读取的位置 | 保留 data-root 子目录层级 |
| 11 | zcode：`select_*_provider` 不写 `is_applied` | 默认徽章无值、启动 reapply 找不到 provider | 写入后镜像 flag |
| 12 | kimi：导入快照路径未走 `join_safe_relative` | 路径遍历漏洞 | 修复 + 恶意路径回归测试 |
| 13 | zcode：`detection.rs` 4 个 `*_with_db*` 白名单漏注册 | 自定义根目录下 MCP/Skills 页读写默认路径（静默） | 白名单**反转**为「共享 resolver 处理未知 key」（见 7.2） |
| 14 | zcode：备份 `root-dir.txt` 被当普通 entry 解压 | CLI 数据根目录留垃圾文件 | 分支内显式跳过 |
| 15 | zcode：`wsl_module_for_external_config_tool` 漏 arm | 恢复出的文件不被 post-restore WSL sync 传播 | 补 arm |
| 16 | zcode：`format_configs.rs` 无专用格式 | `timeoutMs` 写成通用 `timeout`，CLI 静默丢值 | 加 `ZCODE_FORMAT` |
| 17 | 共享组件：`omoNative.model.*` / `hermes.model.thinkingLevelHint` 整组键不存在 | 页面渲染字面键名 | 改为 `common.model.*` |
| 18 | 共享组件：`ModelFormModal` 覆盖值存 i18n key | `i18n:prune` 把 key 判为未使用并删除 | 覆盖值存 `t(key)` 的结果（已是终态文本） |
| 19 | zcode：迁移共享组件时保留 `layout="vertical"` | 标签在输入框上方，与其余 5 个 CLI 全不同 | 改 `horizontal` + `labelCol`/`wrapperCol`（见 4.0.2-A） |
| 20 | zcode：API Key 用朴素 `Input.Password` | 当时判断为「缺显示/隐藏按钮，4/5 的 CLI 都有」 | 改 `masked input + addonAfter`（见 4.0.2-C）。**后注（2026-10-07）**：antd 6 的 `Input.Password` 其实**自带**切换按钮（`visibilityToggle` 默认开），这条的**结论对、理由不准**——真正要核对的是「用户能不能切到明文」，不是「有没有写 `addonAfter`」。两种写法都合规，不必统一 |
| 21 | zcode：渠道字段排在第 3、4 位且分两行 | 用户第一眼看到「名称」而非「选渠道」 | 提为第一行，左渠道 + 右格式（见 4.0.2-B） |
| 22 | zcode：用 `allowClear` + placeholder 表达「不选」 | 用户看不出「留空 = 自定义」 | 加显式「自定义」选项（见 4.0.2-D） |
| 23 | zcode：给非网关 CLI 加计费/请求头/改写分区 | 填了没有代码读，且写进 `meta` 造成"已生效"假象 | `show*=false` + 不合并 meta（见 4.0.3） |
| 24 | zcode：卡片详情渲染三行（含 provider id） | 与 Codex 的一行形态不符；id 是名称的 slug 副本 | 合并为一行并去掉 id（见 4.0.2-E） |
| 25 | zcode：卡片的「应用」用 default 按钮 | 其他 CLI 全是 `type="link"` | 改 `type="link"` + `CheckOutlined`（见 4.0.2-E） |
| 26 | zcode：`ModelListSection` 只传 1 个 handler | 工具栏只有「添加模型」，看起来像设计如此 | 补齐 test / fetch / batchDelete（见 4.0.2-F） |
| 27 | zcode：`db_clean_id` 剥离业务 ID 的 `custom:` 前缀 | 写库成功但读回 ID 被篡改，后续操作全报 not found | adapter 读原始 id + 回归测试（见 2.1） |
| 28 | zcode：模板加载 `.catch()` 只 `console.error` | 后端调用失败表现为「暂无数据」，排查绕远路 | 改成可见错误提示 |
| 29 | zcode：模型弹窗也保留 `layout="vertical"` | 同上，**同一错误在第二个 `<Form>` 上重犯** | 改 horizontal；4 个自建模型弹窗全是 horizontal（见 4.0.2-A） |
| 30 | 共享组件：`ModelFormModal` 的 `messageOverrides` JSDoc 说「传 i18n key」 | 与实现（按已翻译文本处理）矛盾，误导调用方 | 改 JSDoc + 删过时注释（见 4.2.4） |
| 31 | zcode：`handleFetchModelsApply` 不查预设 | 点「获取模型 → 应用」只得到裸 id，参数要手填 | 接 `findPresetModelById` + 共享映射（见 4.2.5） |
| 32 | zcode：`removedModelIds` 被忽略 | 弹窗里「移除已不存在的模型」开关点了没反应（9 个 CLI 里只有它没处理） | 先删后加，无变化则不写盘（见 4.2.5） |
| 33 | zcode：`ModelListSection.onCopyModel` 未传 | 模型行没有「复制」按钮，看起来像设计如此 | 补 `handleCopyModel`（预填副本、新 id），见附录 B.1 |
| 34 | zcode：`CodingPageHeader.onPreviewConfig` 未传 | 页面没有「预览配置」；§4.1 记过一轮，迁移后仍未补 | 接 `JsonPreviewModal` + `readZcodeSettings`（见 4.1） |
| 35 | zcode：`ProviderListSection.hint` / `footer` 未传 | 提示块与导入入口整块消失 | 补 hint + 三个导入源；顺带修好「删除不备份」（见 4.2.1） |
| 36 | 「预设模型选择器」在弹窗里有、在获取模型路径里没有 | 同一能力两个消费点只做了一个 | 抽成共享映射模块，一处定义（见 4.2.5） |
| 37 | 用正则批量改 TSX 源文件 | 跨行多匹配吃掉整个 `Form.Item`，文件无法编译 | **不要对 TSX 用正则批量改写**；用 AST 或逐处 Edit |
| 38 | zcode：官方账号切换还原了 `config.json` | 新一代安装上该文件已无人读取，恢复动作「成功」但无效果 | 用与运行时**同一个**代际判断（`provider_config.json` 是否存在）gate 住写入；见 13.1 模式五 |
| 39 | zcode：官方凭证是 `enc:v1:` 密文 | 不解密就拿不到邮箱，账号只能按供应商命名，重复登录无法识别 | 实现解密（`credential_cipher.rs`）；**但切换本身不解密**——快照整块原样复制（见 13.1 模式六） |
| 40 | 账户去重只按解密出的身份 | 密钥不匹配（快照来自另一台机器）时身份为空 → 每次「保存当前登录」都新增一行 | 先按快照字节比对，再退到身份比对（见 13.1 模式六） |
| 41 | zcode：「预览配置」只读 1 个文件 | ZCode 实际读 3 个（`provider_config.json` / `cli/config.json` / `setting.json`），只看第一个会以为其余不存在 | 新增 `get_zcode_preview` 一次返回全部 + `FileConfigPreviewModal` 分页签 |
| 42 | zcode：官方账号卡片挂在 `ProviderListSection` 的 children 里 | 供应商为空或搜索无结果时，`ProviderListSection` 用 `Empty` **替换掉整个 children**，卡片整块消失 | 新增 `alwaysVisible` 插槽，空态也渲染（见 13.1 模式七） |
| 43 | zcode：「通用配置」弹窗编辑的是 `cli/config.json` | 该文件是 MCP / hooks / 插件 / 权限，**已有专门页签**；通用配置该管的是 `provider_config.json` 的共享部分 | 改读 `get_zcode_common_config`，走 `JsonEditor`；`cli/config.json` 交回 MCP 页签（见 13.1 模式八） |
| 44 | 网关本地 session 统计：`session_files()` 以 `root.is_dir()` 开头 | ZCode 的「根」是 `cli/db/db.sqlite` **一个文件**，walk 静默返回空 → 该 CLI 永远统计不到任何用量 | 在函数开头为「根是文件」的工具加分支（见 13.1 模式九） |
| 45 | 网关本地 session 统计：新增一个 CLI 要改 7 处 | `GatewayUsageTool` 枚举 / `all()` / `as_str()` / `GatewaySessionImportCli` / `import_cli_keys` / `default_session_roots` / `session_files` + 前端 `GATEWAY_USAGE_TOOLS` 与 i18n | 见 13.1 模式九的清单；漏任何一处都不报错 |
| 46 | zcode：卡片间距借自「下一张卡」 | 把官方账号卡片插到中间/调换顺序后，两张卡贴在一起（谁都没有 margin） | **每张卡自己带 `marginBottom`**，间距不依赖邻居；见 13.1 模式十 |
| 47 | zcode：`is_applied` 只由切换命令写入 | 「保存当前登录」和「在 ZCode 里直接登录」都没走切换命令 → 已生效的账号不显示「默认」标签 | 列表接口以**磁盘上的 `credentials.json` 为准**回填 `is_applied`；见 13.1 模式十一 |
| 48 | zcode：「保存当前登录」保存后按钮仍在 | 再次点击什么都不会发生，用户以为坏了 | 虚拟条目（`isVirtual`）只在「当前登录尚未保存」时存在，用它作为按钮的显示条件 |
| 49 | 网关请求页：下拉筛选必须点搜索图标才生效 | 选了 CLI 后列表不动，看起来像筛选坏了；顶栏两个开关却是即时的 | 下拉（CLI/日期预设/来源/状态）改为**选中即生效**；文本输入保留草稿（见 `web/features/coding/gateway/AGENTS.md`） |
| 50 | 模块级 `AGENTS.md` 缺失 | 根 `AGENTS.md` 第 11–31 行要求每个模块目录有 `AGENTS.md` 并登记进 Index；web 侧 zcode/pi/oh_my_pi/dsh/claudedesktop 一直缺 | 新增模块时**同一任务内**建文档 + 登记 Index；见 13.1 模式十二 |
| 51 | 缺失的 `AGENTS.md` 不只在 web 侧 | 审计只跑了 `web/features/coding/*/`，漏掉了后端 `tauri/src/db`、`tauri/src/settings`、`tauri/src/coding/zcode` | 审计命令必须**前后端各跑一次**且覆盖 `tauri/src/*/`，见 12.1；根因同 13.1 模式十二 |
| 52 | 通用配置是**死存储**：写进去，没人读 | 「通用配置」把 JSON 存进 DB 的 `config` 字段，但**没有任何代码把它合并进 `provider_config.json`**。上游 schema 是 `.strict()`，只接受 `providerConfigRules` / `modelConfigRules` / `providerOrder` / `defaultModelSelection` 四个键，写别的会被 ZCode 直接拒绝。用户以为改了配置，实际运行时完全没变 | 实现「通用配置 → 落盘合并」或**移除该功能**；本轮选择**移除 UI 入口**（保留 `saveZcodeCommonConfig` 命令，根目录弹窗仍复用它）。见 13.1 模式十三 |
| 53 | 供应商卡片样式各写各的，逐页分叉 | 11 个 CLI 有 9 份 bespoke `*ProviderCard.tsx`（最长的 1148 行），同样的间距/按钮/拖拽/选中态逻辑抄了 9 遍；改一处通用行为要改 9 个文件，且很容易漏 | 固定为**三种样式**（`shared/providerCardVariants/`）：按 provider 模型形状选用，见根 `AGENTS.md` Hard Rule 14 与 §4.2 选型表 |
| 54 | 前端 API 封装存在但**全仓无调用** | `toggleZcodeProviderDisabled` 在 `zcodeApi.ts` 有封装、后端命令也注册了，但没有任何 UI 调用它——「禁用」开关从未接上。这类死封装看起来像功能已完成 | 接新 CLI 时对每个 API 封装跑一次 `rg "<apiName>" web/ \| grep -v services/`，确认除定义外还有调用点；本轮已在迁移卡片时接上 |
| 55 | 两个「默认」标记语义脱节 | ZCode 的 `settings.models[].isDefault`（provider 目录内的偏好）与注册表的 `defaultModelSelection`（运行时实际用哪个）是两个东西，前端 `onSetPrimaryModel` 只写前者 → 卡片显示「当前默认」但 ZCode 从不使用 | 设默认必须**两处都写**，见 `handleSetPrimaryModel`；判断「谁是默认」以运行时事实源（`defaultModelSelection`）为准 |
| 56 | 侧栏开关**读一个键、写另一个键** | `OhMyPiPage` 读 `sidebarHiddenByPage.pi`（Pi 的槽位）却写 `setSidebarHidden('oh_my_pi')` → 开关静默无效，翻转的是本页不显示的槽位；同时显示的是 Pi 页的开关状态。`pi` 与 `oh_my_pi` 在 `SIDEBAR_PAGE_KEYS` 里是两个独立键 | 每个 CLI 只碰自己那个键，读与写必须同名。见 13.1 模式十六；审计：`for f in web/features/coding/*/pages/*.tsx; do r=$(grep -o "sidebarHiddenByPage\.\w*" "$f"\|head -1\|cut -d. -f2); w=$(grep -o "setSidebarHidden('[a-z_]*'" "$f"\|head -1\|sed "s/setSidebarHidden('//;s/'//"); [ -n "$r" ] && [ -n "$w" ] && [ "$r" != "$w" ] && echo "MISMATCH $f: $r vs $w"; done` |
| 57 | 弹窗用 `okButtonProps={{ loading: true }}` 表达进行中 | Ant Design 的 `loading` 会给按钮加 `pointer-events: none`，**把取消按钮一起锁死**——登录/提交卡住时用户无法退出 | 进行中改用正文 `Spin`；`okButtonProps` 只设 `disabled`。任何长耗时弹窗都要验「操作中点取消」（见 4.0.2-A） |
| 58 | 说明性文字放在它说明的控件**上方** | 读起来像需要先关掉的横幅，而不是「这个框里该写什么」的说明 | 解释控件的文字跟控件走（下方）；只有警告/阻断类才置顶（见 4.0.2-C） |
| 59 | 卡片内的次要操作用了带边框的 default 按钮 | 在一排无边框图标里视觉过重，抢走供应商名称的注意力；与其他 CLI 同款动作样式不一致 | 卡片内次要操作一律 `type="text"` + `fontSize: 12`；同款动作跨 CLI 同款（见 4.0.2-E） |
| 60 | 作用域是「某一行」的按钮放在卡片顶部 | ZCode「保存当前登录」在卡片头部，但它只对未保存的那一条有效；用户看不出它作用于谁，条件渲染也让顶部多出含义不明的按钮 | 按钮放进它所作用那一行的操作区（见 4.0.2-E）；判据：作用域是「某一行」→ 放行内，是「整张卡」→ 放头部 |
| 61 | 官方账号卡片固定置顶，不能参与排序 | 它与供应商卡片视觉同级（同款外壳、同一列表容器），用户期望能一起拖动；固定置顶让「位置」这个用户可见的状态无法表达 | 作为列表成员参与排序，位置存 `official_account_index`（按「上面有几张供应商卡」计，不用列表下标）；见附录 B.2 |
| 62 | 多渠道路由的 CLI 卡片上仍有「应用」按钮 | ZCode 没有单一 active provider，多个渠道同时可用；「应用」暗示其他渠道被关掉，且把「选哪个模型」误表达成「启用哪个渠道」 | 改用 `OpenCodeStyleCard`（无头部应用按钮），默认在模型行上选；见 §4.2.1 选型表 |
| 63 | 模型弹窗的字段集合是**猜的**，不是查上游得来的 | 多了「工具调用」「MFJS 工具 Schema」「最大输出参数映射」三个控件。前两个官方 UI 完全不渲染（系统字段），第三个官方只保留不给编辑入口；且 `manual` 模式下前两个又被隐藏 → **同一字段两种行为**，还写出了官方编辑器永不产生的键 | 字段集合以上游**可编辑 schema** 为准（ZCode 是 `manualModelConfigSchema` 的 `pick`）；删掉多余控件，被删字段**原样保留**。见 §4.2.4.1 与 13.1 模式十八 |
| 64 | 含义不自明的字段没有说明入口 | 「推理参数映射」「模型能力」等标签本身说不清字段含义（允许什么值、为什么、注意什么），用户只能猜 | 标题后加问号 + hover 说明（`FieldHelp`）；文案逐字取自上游 `help.*`。见 4.0.2-C |
| 65 | 推理参数映射用单行 `Input` | 内容是带嵌套花括号的 CEL/JSON 表达式，单行输入框里读不成句、改不动 | 改 `Input.TextArea`（`autoSize`），对齐官方的 textarea。见 4.0.2-C |
| 66 | 帮助文案直接丢给 `Tooltip title={字符串}` | `\n` 不换行、`**粗体**` 原样显示星号——整段说明挤成一行，比没有说明更难读 | 走 `components/common/FieldHelp`（纯字符串 → 段落/列表/粗体/代码）。见 4.0.2-C 与 `FieldHelp/AGENTS.md` |
| 67 | 字段开关放错层级（弹窗里 vs 列表行上） | 「启用」官方放在**模型列表行**的 `Switch` 上，我们放进了编辑弹窗；而我们的 `ModelItem` 当时**根本没有**这个开关——照做会丢能力 | 先查官方把它放在哪一层：行级状态放行上、卡片级状态放头部。见 §4.2.4.1 第 6 步 |
| 68 | 模型行排了两行，第二行整行重复 | `上下文限制: 400,000 \| 输出限制: 128,000` 每个模型都占一行，而这一行携带的信息（量级）一眼就能从名称旁读完；列表被撑高一倍 | 合并成一行：`(id \| 400K)`；数字用 `formatModelLimit` 压成 `200K`/`1M`。见 §4.2.3 与 13.1 模式二十 |
| 69 | 页面挂着 4 个只有本 CLI 有的区块 | OmO Native 有「生效配置」「Agent·Category 方案」「MCP 与 Skills」「更多选项」四块，与其余 tab 的形态完全不同；其中「生效配置」与页头「预览配置」重复、「更多选项」的内容在 ⋯ 弹窗里已有同一份 | 收敛成标准三区块（供应商 / 全局提示词 / 会话管理）；**撤下 UI 但保留后端**——先例见 `oh_my_pi/AGENTS.md` 对 `listOmpAgents` 的说明。见 13.1 模式二十一 |
| 70 | `OpenCodeStyleCard` 声明了 `actions.extraActions` 却从不渲染 | 类型里有这个 prop、调用方也传了（连通性测试 / 获取模型两个按钮），但样式组件里没有渲染点 → **两个按钮静默消失**，且类型检查完全通过 | 类型里出现的可选 prop，必须在组件里指认一次它的渲染位置。同 13.1 模式二，但**发生在共享组件自己的 props 上**——比调用方漏传更隐蔽 |
| 71 | `useRootDirectoryConfig` 的动态前缀白名单只列了 3 个模块 | 该 hook 用 `t(\`${translationKeyPrefix}.rootPathSource.modal.*\`)` 拼 12 个模块的 key，但 `scripts/i18n-keys.mjs` 的 `DEFAULT_DYNAMIC_IDENTIFIER_VALUES_BY_FILE` 只登记了 claudecode/codex/geminicli → 其余 9 个模块的整块文案被判「未使用」，`i18n:prune` 会删掉运行时仍在读的 key | 白名单必须与**实际调用方**同步（12 个前缀全部登记）。新增 CLI 时把前缀加进去 |
| 72 | 嵌套动态表达式永不展开 | `${translationKeyPrefix}.rootPathSource.modal.${pathInfo.source}Hint` 有两层占位符，工具的 `expandDynamicExpression` 只做单层展开 → `envHint`/`shellHint` 永远算「未使用」 | 每个占位符都要有对应的 identifier 值表：加 `'pathInfo.source': ['env', 'shell']` |
| 73 | 6 个模块的 `envHint` / `shellHint` 根本没写 | 补完白名单（#71）后 `i18n:check` 立刻报出 claudecode / codex / geminicli / grok / ohMyPi / pi 六个模块缺这两条——它们的根目录决议**都能返回 `env` / `shell` 来源**，走到那条分支时界面显示的是字面 key | 白名单补全不只是「消音」，它会把一直存在的缺口暴露出来。逐条补齐（用各自真实的环境变量名：`CODEX_HOME` / `CLAUDE_CONFIG_DIR` / `GROK_HOME` / `PI_CODING_AGENT_DIR` / `GEMINI_CLI_HOME`） |
| 74 | 共享 `ModelItem` 的开关文案 key 不存在 | `common.model.disabled` / `enabled` / `toggleEnabled` 三条被新代码引用但从未创建 → 开关的 hover 提示显示字面 key。`i18n:check` 抓到了（静态 `t('...')`），`tsc` 抓不到 | 加 UI 的同时加 key；把 `pnpm i18n:check` 当作 UI 改动的**必跑项**（见 §12.1） |
| 75 | 根 `AGENTS.md` 的 Index 缺 `omo_native` 两行 | 模块建了、模块级 `AGENTS.md` 也建了，但根 Index 里 `tauri/src/coding/omo_native/` 与 `web/features/coding/omo_native/` 两行都没有——违反了 Hard Rule 6，且没有任何机制会报错 | 新增模块时**同一任务内**补根 Index；见 13.1 模式十二 |
| 76 | 模型弹窗传 `showOmpThinking`，把 OMP 的 `thinking` 写进 senpi 的 `models.json` | OmO Native 与 Pi 同用 **senpi** 引擎、同读 `models.json`（OMP 读 `models.yml`）。但字段开关照 OMP 抄了 `showOmpThinking` → 写出 `thinking: { efforts, defaultLevel }`。上游 `docs/models.md` 的模型字段表里**根本没有** `thinking`，只有 `thinkingLevelMap`/`defaultThinkingLevel` → 用户编辑思考级别，引擎完全不读；而真实文件里的 `thinkingLevelMap` 被静默覆盖 | 先确认**引擎同源**再决定抄谁：同引擎 → 抄同引擎的兄弟（Pi），别抄名字像的（OMP）。字段集合以该引擎的 `docs/models.md` 为准；见 13.1 模式二十三 |
| 77 | `values.inputTypes` 是 JSON **字符串**，却直接赋给 `nextModel.input` | `ModelFormModal:761` 产出的是 `JSON.stringify(inputModalities)`；上游要的是 `input: ["text","image"]` **数组**。直接赋值写出 `"input": "[\"text\"]"`——引擎读不到，且把原有数组覆盖成字符串 | 弹窗交回的 JSON 字符串**一律 parse** 再落盘（`parseInputTypes` / `parseJsonRecord`）；反向用 `stringifyInputTypes`。回归测试断言 `Array.isArray(model.input)` |
| 78 | 「获取模型 → 应用」不做预设匹配 | SOP §4.2.5 明令，Pi/OMP/Codex 全部接了 `findPresetModelById`；Native 漏了 → 用户点应用只得到裸 id，参数全要手填（= ZCode 教训 #31 的复现） | 每个 CLI 的 fetch 回调都要接预设匹配；同引擎的兄弟是最好的抄写对象（Native 与 Pi 的 builder 现在同构） |
| 79 | 页面标题 `OmO Native（Agents 与 Categories）` 指向已删区块 | 标题里点名的能力在页面收敛后已经不存在——用户按标题找区块，找不到 | 撤区块时**同一任务内**检查标题 / hint / 空态文案里有没有点名它；见 13.1 模式二十一 |
| 80 | `ProviderListSection` / `CodingPageHeader` 的 `hint` 插槽**裸渲染**，视觉契约留给调用方手抄 | 三个已迁移页面各自复制同一段 `style={{fontSize:12, color:secondary, borderLeft:'2px solid', paddingLeft:8, marginBottom:12}}`；第四个（omo_native）漏抄 → 提示块渲染成**正文大小、全黑**，看起来像一段说明段落而不是提示 | 共享组件的插槽只让调用方传**文案**，样式（连同它的常量）进组件；见 13.1 模式二十四 |
| 81 | 卡片渲染了拖拽把手，却没有 `DndContext` | 复制了 ZCode 的 `draggable: !selectable`，但没复制它的 `DndContext` → 把手在、光标是 `grab`、**拖不动**。用户报「排序拖动不了」 | 把手只在**真的能拖**时渲染：`draggable: !selectable && !dragDisabled`；`dragDisabled` 要覆盖「非 custom 排序 / 有搜索词」。见 13.1 模式二十五 |
| 82 | 顺序**存进去读不出来**：`list_*` 结尾 `sort_by(key)` | 就算补上 DndContext + reorder 命令，读取端仍按 key 字母序返回 → 拖完一刷新跳回原样，表现为「拖了没用」 | 顺序的**写入点与读取点必须成对核对**：存哪就读哪（这里是 `preserve_order` 的键序）。见 13.1 模式二十五 |
| 83 | 供应商弹窗内联在 `*ProvidersSection.tsx` 里，`layout="vertical"` | 与其余 CLI 的弹窗形态不同；且因为**文件名不匹配**，`scripts/verify-form-modal-layout.mjs` 扫不到它（守卫只扫 `*ProviderFormModal.tsx` / `*ModelFormModal.tsx`）→ 布局回归无人拦 | 把弹窗**抽成 `*ProviderFormModal.tsx`** 再改布局：既统一形态，也让守卫覆盖到。守卫的文件数会 +1（13 → 14） |
| 84 | `__local__` 桥接项的 `name` 各写各的 | 三类写法并存：`default`（6 个 CLI）/ `Local <文件名>`（5 个，含 omo_native）/ 裸文件名（ZCode）。用户看到 `Local AGENTS.md` 而别处是 `default`，问「为什么多了个 local」 | 新增 CLI 用 `default`（多数派）；顺带查卡片副标题是否已在说同一件事（omo_native 的副标题本来就写「来自本地 AGENTS.md」，名字里再来一次是重复） |
| 85 | **SOP 的区块清单本身写错了**（「只许三块」），导致 OmO Native 少了两块全仓 7/5 个页面都有的区块 | §4.0.2-I 原版把「默认三区块」写成硬规则，并让执行者用「别的 tab 为什么不需要它」自证——诱导否定的问法，任何区块都能被问倒。用户随后指出缺「其他配置」「官方认证渠道」 | §4.0.2-I 重写为「先取并集再逐块删」，并给出可执行命令；13.1 模式二十六。**规则本身要拿现有代码反查**：按这条规则现存哪些东西违规？若「7 个页面都违规」，违规的是规则 |
| 86 | `OMO_NATIVE_BUILTIN_PROVIDERS` 手工副本与引擎脱节 | 漏了 `anthropic-subscription` / `cursor-cli-oauth`（于是被当自定义 provider 列进可编辑列表），还留着 `bai` / `ollama` / `typesafe`（早已不是内建）。**同一份名单还另抄了一份给 OAuth**，两处各自过期 | 按引擎实际输出重建（48 条）；加三条回归测试（有序无重复 / OAuth 是内建子集 / kebab-case）。⚠️ `--list-models` **不列「目录按账号发现」的 provider**（如原生 `cursor`），要对着 `docs/providers.md` 补回——别只看一条命令 |
| 87 | 预览标签写文件名，实际只显示一个块 | 标签是 `omo.jsonc`，内容却只有 `[native]` 块。文件里只有 `[opencode]` 块时（常见），预览显示 `{}` —— 用户以为「配置文件是空的」，实际文件有 3KB | 标签写明 `omo.jsonc → [native]`；空块时给出解释文案（说明本页尚未应用过方案、文件可能仍有 `[opencode]` 块、要看整份文件请用「打开文件夹」） |
| 88 | 同一个保留 id / 名字，**33 个后端文件 + 约 14 个前端文件各写各的字面量** | `"__local__"` 后端实测 **51 处**（`commands.rs` 里构造桥接记录、`if config_id ==` 判定、`tray_support.rs` 过滤、各模块 `constants.rs` 的 const 别名）；前端另有 4 个 `utils/localProvider.ts` 是逐字复制的副本。改名要全仓 grep 手工对齐，漏一处就出现「同一个桥接项在不同页面名字不同」（教训 #84 的直接成因） | 抽成**单一来源**：后端 `coding/local_bridge.rs`（`LOCAL_CONFIG_ID` / `LOCAL_CONFIG_NAME`），前端 `features/coding/shared/localConfig.ts`；旧路径留薄 re-export 兼容既有 import。**保留字面量出现在 >3 个文件里就该抽**；抽完加断言测试钉住取值 |
| 89 | **「抽成常量」做了一半就宣布完成** | 第一轮只迁移了 9 个「构造桥接记录」的文件就报「已统一」，剩下 33 个文件（判定、过滤、const 别名）仍写死字面量——**同一份名单在仓库里存在两种表达**，比全用字面量更难发现（grep 一次看到的是一半结果） | 迁移类改动的完成判据是**旧写法的残留数为 0**，不是「新的写对了」。用 `grep -rn '<旧字面量>' --include=*.rs` 收尾核对（注释里的提及也要一起改，否则下次 grep 又看到它） |
| 90 | 供应商卡片 / 模型列表的**能力集没被枚举**，于是新建的卡片静默缺功能 | ZCode 卡片少「复制供应商」、OmO 模型列表少「批量删除」。这些功能在别的页面都有，但 SOP 从没列过「一张卡片至少要有什么」——照着旧文档从零写，缺哪个全凭记忆 | 建卡片/列表前先**横向 diff 三张已有卡片**，把按钮列出来再逐项决定「要 / 不要 / 为什么不要」；不要的写明理由（如「内建渠道只读，故无编辑」）。见 §4.2.3 与 13.1 模式二十七 |
| 91 | 同一个缺陷**在另一个页面也存在，只是没人报** | 用户报 ZCode 卡片缺「复制供应商」后按新清单全仓扫了一遍，发现 **OpenClaw 才是缺得最狠的**：供应商卡片没有复制、模型行也没有复制（`ProviderCard` 支持 `onCopy` / `onCopyModel`，OpenClaw 一个都没传），而它是全仓唯一这样的页面 | 用户报「少了 X」时，**不要只修他指的那一处**——把「X 这个能力」在全仓做一次横向审计（`for f in */components/*ProviderCard*.tsx; do grep -q onCopy $f || echo $f; done`），一次修完。同一个模板复制出来的页面，缺陷往往也是复制出来的 |
| 92 | **「只读区块」把全量都列了出来，没筛掉不可用的** | OmO 的「引擎内建渠道」列了 49 个内建 provider，其中 48 个未配置凭据——用户在页面上看到一屏自己用不了的东西。OpenCode 的同类区块从 `auth.json` 的键派生，只显示配好的那几个 | 照搬一个已有区块时，**要连它的筛选条件一起照搬**，不只是样式和数据源。判断「这个区块该显示哪些条目」时先问：**用户在这个区块里能对哪些条目做操作 / 哪些条目对他有意义**？无意义的条目就是噪音。见 13.1 模式二十九 |
| 93 | **写入端与读取端用了不同的字段，界面永远显示空** | OmO 的编辑弹窗把 key 写进 `auth.json`，`has_key` 也只查 `auth.json`；但用户的 `shangtang` 是更早（或引擎侧）写在 `models.json` 的 `apiKey` 里的。弹窗打开时 `apiKey: ''` 硬编码清空、placeholder 也是空的 → 用户看到「没保存成功」。**数据其实好着**，`omo auth print-api-key` 正常返回 | ① 「保存成功了吗」这类问题**先查磁盘事实**（文件内容 + 引擎自己的读取命令），不要从界面推断——界面为空不等于没存。② **写和读必须是同一处**：有多个合法存储位置时（OmO 是 `auth.json` 优先于 `models.json` 的 `apiKey`），要么统一到一处并迁移旧值，要么读取端两处都认。③ **密钥字段要回填明文**（见教训 #94）。见 13.1 模式三十 |
| 94 | **自作聪明地把密钥藏起来，制造了「看不见」的 bug** | 修 #93 时把弹窗改成「不回填明文 + `••••••••` placeholder + 『已保存密钥』文案」，自以为符合安全最佳实践。用户当场否掉：「这个应用就是管理配置的，前端都可以展示所有的秘钥，不要再出现这种看不到回退的情况了」。同仓 Claude Code / Codex / Kimi 的弹窗**一直**都是直接回填明文的 | **本项目定位是「配置管理器」，密钥是它管理的对象之一，明文展示是产品要求不是缺陷**。给密钥字段做「防肩窥」处理是**反模式**：它让「存了 key」和「没存 key」在界面上无法区分，直接制造误报。要展示明文就展示——包括列表接口直接返回 `apiKey`、预览弹窗列出 `auth.json`。见记忆库 `projects/ai-toolbox/ai-toolbox-shows-plaintext-credentials.md` 与 13.1 模式三十一 |
| 95 | **「获取模型」没传密钥 → 401**，且**用错了 config value 模式** | 卡片把 `baseUrl` / `headers` / `configValueMode` 传给了 `FetchModelsModal`，唯独没传 `apiKey` → 请求不带 `Authorization`。更要命的是 `configValueMode` 传的是 `"omp"`：OmO 与 OMP 都是 `oh-my-*`，但 **OmO 的 `apiKey` 支持 `$ENV_VAR` 插值、OMP 不支持**（实测确认），传错模式会把 `$MY_KEY` 当字面量发出去 | ① 「拉取模型 / 连通性测试」这类出网动作，**逐个核对四个字段**：`baseUrl` / `apiKey` / `headers` / `configValueMode`——漏传 `apiKey` 是最常见的一种。② **同源工具 ≠ 同语法**：`oh-my-*` 家族里 OmO 与 OMP 的凭据语法不同，选 config value 模式要按**引擎**而不是按名字像。新增模式时把「为什么不是隔壁那个」写进注释。见 13.1 模式三十二 |
| 96 | 硬规则写了「三种样式，不再新增」，但**两种样式零消费方** | `providerCardVariants/` 落地时只有 `OpenCodeStyleCard` 有调用方；`ClaudeStyleCard` / `CodexStyleCard` 从建立起就没有消费者，而它们各自最典型的对应 CLI（Claude Code 776 行 / Codex 1134 行）仍在持 bespoke 卡片。规则本身正确，**没有任何检查能发现它被绕过**——迁移进度只存在于一句文档描述里 | 补源码扫描型守卫 `scripts/verify-provider-card-layout.mjs`（禁止映射层自己 `useSortable` / `<Card>` / `ManagementCheckbox`），未迁移的登记在 `PENDING_MIGRATION` 里**只减不增**。见 13.1 模式四十六 |
| 97 | 共享组件的新 prop **声明了却没渲染** | `ProviderCardModels` 里的 `modelSourceTag`、`onToggleModelDisabled` 等由调用方传入，但某个样式组件里没有渲染点 → 传了等于没传，类型检查全通过 | 同 #70。类型里出现的每个可选 prop，**在组件里指认一次它的渲染位置**；加 prop 时同步补一条守卫或至少在 PR 描述里写「渲染在 X 行」 |
| 98 | 迁移时**照搬了参照页面的旧 key，而不是共享 key** | 页面迁到 `CodingPageHeader` 后仍传 `<tool>.viewDocs` 这类 per-CLI key——它们在中文下与 `common.*` 逐字相同，所以看不出问题；但英文措辞会两套并存（`Documentation` vs `Official Docs`），且这些 key 永远无法 prune | 迁移前把待迁的 key 与 `common.*` **逐条比对中英文**（命令见 §12.1），同义的统一取 `common.*`，把英文措辞变化写进提交说明；**不要为保住旧措辞加覆盖 prop** |
| 99 | 迁移卡片时**漏搬了「状态高亮」** | 四张 bespoke 卡片各自抄了一份「批量选中 > 网关 P0 > 已应用」的边框/底色优先级；`CardShell` 只实现了「批量选中」那一条 → 迁完之后**已应用与 P0 的视觉标记全部消失**，卡片一律灰边框。界面不报错，只是用户看不出哪个渠道生效了 | 迁移前把原卡片的 `style` 逐条对照目标组件的渲染点（不只是 props 对照）；本轮补 `providerState.accent` 到 `CardShell`。见 13.1 模式四十八 |
| 100 | 标签与它的值被拆成**两个** `metaEntries` 项 | Claude Code 的绑定行把 `Haiku:` 与模型 id 分成两个条目 → 行的 `gap: 16px` 插到标签和值之间（读起来像两列），且 `alignItems` 按顶边对齐，`<code>` 的 padding 一撑标签就偏高。**用户一眼看出「没有居中对齐」** | 标签随值走：`{ kind: 'code', label: 'Haiku:', value: '…' }`，组件渲染成同一个 flex 项。见 13.1 模式四十九 |
| 101 | 迁移时漏 import 了 CLI 自己的 `.less`，白色标题条露出来 | `CodexProviderCard.less` 的 `.codex-model-list-collapse` 负责把模型列表区的六层 antd Collapse 背景刷成透明；改写卡片时没带这行 import → **已应用卡片（有底色）上的「模型列表 (N)」标题变成白条**。只在卡片有 accent 时才可见，无底色时完全看不出 | 透明规则改由共享 `ModelListSection` 的 `transparentRows` 统一施加（`ModelListSection.module.less`），CLI 侧不再各存一份。见 13.1 模式五十 |
| 102 | 迁移时把菜单**简化成了等价的数据**，丢掉了图标 / 分隔线 / 副标题 | 原实现是 `MenuProps['items']`（每项带 `icon`，删除前有 `{type:'divider'}`，启用项有 `配置已启用` 副标题）；重写成 `{ key, label }[]` 后**类型全通过、菜单照常弹出**，只是长得像另一个产品。用户逐项对照后说「不要丢东西了」 | 菜单的呈现契约写进组件文档（`providerCardVariants/AGENTS.md`），结构由共享组件持有，调用方只给 handler 与文案。见 13.1 模式五十一 |
| 103 | 迁移时**顺手提升了某个动作的位置** | 「编辑」原本在「更多」菜单里，迁到共享组件后被提到了头部显眼的 `primaryAction` 位——功能没丢，但用户按肌肉记忆去菜单里找，找不到，问「现在跑外面来了」 | **迁移的验收标准是「和迁移前一模一样」**，不是「更合理」。想调位置就单独提出来当一次显式改动。见 13.1 模式五十二 |
| 104 | 回调**契约收窄**，值算反了：开关点了没反应 | `onToggleDisabled` 原本是 antd 的 `(checked) => void`；迁到共享组件时被收窄成无参 `() => void`，映射层只好用当前状态重算新值（`!provider.isDisabled`），再交给一个内部又会取反的处理器 → **写回的就是原值**。开关渲染正常、点了毫无反应，类型检查全通过。用户点了一下就发现 | 共享组件**透传**值而不是让调用方重算：`onChange={onToggleDisabled}`，类型 `(enabled: boolean) => void`。见 13.1 模式五十三、§12.3.1 第三遍 |
| 105 | 卡片级拖拽属性挂在了**模型区**的 props 下，无模型区的样式读不到 | `draggable` / `sortableId` 定义在 `ProviderCardModels` 里（因为最早的消费方都带模型列表）。Claude 式没有模型区 → 迁移后 `useSortable` 从未被喂 id、拖拽把手整个消失。**同一个类型里的 prop，不代表所有样式都会读** | 提到 `providerState`（卡片级 chrome 的归属处），三个样式都在 `CardShell` 上渲染它。见 13.1 模式五十四 |
| 106 | **共享外壳缺一个插槽，调用方就只能整块复制**（**结论已被 #114 推翻**，留作追溯） | `ProviderListSection` 的工具栏没有「额外按钮」位置，Kimi 的「官方账号登录」按钮就无处安放——要么留在页面里继续复制整段工具栏，要么删掉这个入口。**外壳的插槽完备性不是「新能力」**，不能套用「至少两个消费方」的门槛 | 当时新增了 `toolbarExtra`（单消费方，Kimi）。规则本身仍成立（判据写进 §4.2.1 与附录 B.1：**外壳缺位置本身就是缺陷**），但**这条实例的判断是错的**：那个按钮根本不该在工具栏。见 13.1 模式五十五 |
| 107 | `alwaysVisible` 与 `footer` **选错，内容在有数据时整块消失** | Kimi 的官方账号列表原本无条件显示（在列表下方）。迁到 `ProviderListSection` 时放进 `alwaysVisible`——该插槽**只在列表为空/搜索无结果时渲染**，于是有供应商时账号区完全不见。没有报错、没有类型错误，只是内容没了 | 判据：**「列表非空时它还需要显示吗？」** 需要 → `footer`（无条件）；只有「空态时也要能看到」才用 `alwaysVisible`。**第三选项常被忘掉：#114 的列表成员要 `children` + `alwaysVisible` 两个都传。** 见 13.1 模式五十六 |
| 108 | 固定菜单遇到**工具专属菜单项**时没有出口 | Kimi 是唯一把「连通性测试」放在「更多」菜单里的卡片。共享菜单是固定契约（启用→编辑→复制→分享→分隔线→删除），不接纳额外项——迁移者要么删掉（漏功能）要么破坏契约 | 额外项迁到**语义相符的插槽**：连通性测试属于「对模型目录的探测」，落到卡片第二行（`inlineActions`）+ 模型区工具栏（`onTestModels`），与 Codex 卡一致。**位置变化要写进提交说明**。见 13.1 模式五十七 |
| 109 | 共享模型行比 bespoke 行**多出按钮，但页面没有对应 handler** | Kimi 的模型行只有「删除」+ 点名字编辑；共享 `ModelItem` 渲染 编辑/复制/设为主模型/删除 四个。直接迁移会得到两个**点了没反应**的按钮——或者干脆不渲染，等于功能比预期少 | 迁移到共享列表时**逐个确认新增按钮的 handler 在页面侧存在**；不存在就实现（Kimi 补了 `handleCopyModel` / `handleSetPrimaryModel`）。见 13.1 模式五十八 |
| 110 | 迁移时**按钮的图标被换成了「语义相近」的另一个** | 网关代理按钮原本是 `ApiOutlined`（「接管代理」的语义），迁移后写成了 `CheckOutlined`——因为同一排的其他网关按钮（应用并代理 / 切换主渠道 / 锁定应用）都用 `CheckOutlined`，顺手就统一了。**功能完全正常，只是图标换了**，任何测试都发现不了 | 逐按钮对照 `icon={<...>}`：把原实现的图标清单与迁移后**逐个**比对，不只比数量。图标是「这个按钮是什么」的一部分，不是装饰。见 13.1 模式五十一 |
| 111 | 用户报「拖不动」，而**真实原因是排序模式**：功能被有意关掉了，但界面没说 | Codex 页的拖拽在「非默认排序 / 有搜索词」下被有意禁用（`providerDragDisabled`），代码完全正确。用户的 DB 里 `provider_sort_modes.codex = "created"`——于是他看到的是**没有把手的卡片**，读到的结论是「拖拽把手没用」。排查时先怀疑了四层实现（`CardShell` 注册、`useSortable`、页面传感器、`dragDisabled` 传参），全部正确 | 功能性禁用**必须在界面上留下原因**，否则用户无法区分「坏了」和「关掉了」。给 `ProviderSortDropdown` 加 `dragDisabledBySort` + tooltip 说明「拖拽仅在默认排序下可用」。**判别方法：任何让用户可见能力消失的状态，都要能回答「用户怎么知道为什么」。** 见 13.1 模式五十九 |
| 112 | 排查交互 bug 时**只读代码**，把「代码看起来对」当成「行为对」 | 本轮的拖拽「修」了两遍（`draggable` 归属 + `dragDisabled` 门控），每次都是读链路后宣布修好，每次都还没修好。真正定位问题只用了一次**在浏览器里真的拖一下**：仓库里已有 `scripts/lib/browser-fixture.mjs`（真 Chromium + CDP），却一直没被用来做交互验证 | 交互类改动**优先用浏览器夹具验证**，而不是读链路。本轮把 `ProviderListDragFixture.jsx` + `providerListDragBrowserChecks.mjs` 落成 `pnpm run test:provider-list-drag`（7 项，含真指针拖拽、落库回写、禁用态无把手、tooltip 文案）。见 13.1 模式六十 |
| 113 | 用户点名「参考 Pi」补的**默认渠道不可删**，OmO Native 当时没有：pi / ohMyPi / dsh / hermes / openclaw / opencode **六个**页面都做了，只有 OmO 静默少一条 | 规则本体是「默认指针（`settings.json` 的 `defaultProvider`）指向的渠道不能删」；Pi 用 `actions.deleteDisabledReason` 置灰按钮 + `canBatchDeleteProvider` 排除批量。OmO 建卡片时逐项对着 Pi 抄了按钮，**唯独没抄这条门控**——又是模式 27 / 第 90 条那类「卡片能力集没人枚举」 | 分组判据落到 §4.0.2-H：**默认指针是否写在 provider 记录之外**——是（pi / ohMyPi / dsh / hermes / openclaw / opencode / omo_native）→ 必须置灰删除按钮**并**排除出 `batchSelectableIds`；Claude 式「应用」语义不适用。**只置灰单个按钮不算完成**：漏掉批量那条，全选就能删掉它 |
| 114 | （**结论已被 120/121 二次推翻**，留作追溯：官方账号区最终按 Codex 形状落成「官方渠道卡片内的一段」，不是列表成员）**「外壳缺位置」判断错了对象：那个区块根本不是外壳的一部分，而是列表成员** | Kimi 迁移时把官方账号区拆成「工具栏里的登录按钮（新增 `toolbarExtra`）+ 页底的账号列表（`footer`）」，理由是「外壳缺位置就得加」（#106）。用户看出来的却是：**它该是一张与供应商卡片平级、能拖动排序的卡片**，登录按钮在它右上角。参照物同仓库早就有——ZCode 的 `ZcodeOfficialAccountCard`，#42 / #61 已把「官方账号卡片必须是列表成员、空态也显示、可排序」写成了规范，**迁移时没人去检索它** | 落点改为 `children` + `alwaysVisible` 双传（两个分支互斥），位置存 `officialAccountIndex`（走 `save_kimi_official_account_index`，与 ZCode 同构）；`toolbarExtra` 随最后一个消费方删除。**判据：落到外壳插槽之前先答「用户会不会期望它能被拖动 / 和卡片并排？」**——会 → 它是列表成员。见 13.1 模式五十五、六十一 |
| 115 | **同一句文案被两层语义共用**，改其中一层的措辞会把另一层也改错 | Codex 的「官方账号行·切换」按钮与「供应商卡片·应用」按钮**共用 `codex.provider.apply`**。用户的口径是「多个官方账号之间是切换，只有渠道的切换才叫应用」——直接改这个 key 会把渠道按钮一起变成「切换」。同理 `codex.provider.applied` 同时是卡片高亮徽章与账号行的「当前」徽章 | 按层拆 key：账号行新增 `officialAccountSwitch` / `officialAccountApplied`，`apply` / `applied` 留给渠道。Kimi 侧同理（`kimi.account.apply` 改「切换」、`applied` 改「默认」；卡片自己的 `kimi.provider.applied` 不动）。**改任何 i18n 文案前先 `grep` 这个 key 的所有渲染点**，看看有几层语义在共用它 |
| 116 | **一个区块被渲染进了卡片头部两栏的「左栏」里**，右边缘静默短一截 | `CodexStyleCard` 的头部是「左内容 / 右动作链接」两栏 flex。`footer`（Codex 官方账号区）被写在**左栏内部**，于是账号行的「切换 / 删除」右边缘比下方模型区工具栏短了 **160px**——正好是右上「应用 / ⋮」那组链接的宽度。用户报成「账号列表右侧空了一大块」。类型检查、静态守卫、快照全通过 | `footer` 移出两栏、与模型区并列**跨满卡片整宽**（组件文档本来就写着「头部下方、模型区上方」，是实现与文档不符）。**判据只能靠几何量**：新增 `pnpm run test:codex-official-accounts`，断言「账号行右边缘 == 卡片内容右边缘 == 头部动作右边缘」。见 13.1 模式六十三 |
| 117 | 区块的说明文字被挂在**区块最底部**，成了「脚注」而不是「副标题」 | Kimi 官方账号卡片（照抄 ZCode）把「切换会把该账号的登录凭据写入 Kimi 配置…」放在账号行**之后**。有账号时它离标题很远；**空态时中间还夹着一整块空插画**，那句话看起来像页面底部的残留文字。用户直接圈出来：「副标题应该放在标题下面啊」 | 说明文字贴它说明的**对象**走：区块自己的说明 = **标题的副标题**，紧贴标题下方（标题行 → 说明 → 内容）。**判据：遮住内容区，只留第一屏——那句说明还看得出是在讲这个区块吗？** 看不出就是放错了。Kimi 与 ZCode 两张卡一起改（同构才有意义）；新增浏览器断言「卡片里没有任何一句话被渲染两次」，防止「搬到新位置却没删旧位置」。见 13.1 模式六十四 |
| 118 | 「定位」的闪光**在点击那一刻就跑完了**，用户滚到位时卡片上什么都看不到 | 「定位当前已应用供应商」= 滚到卡片 + 闪光。实现写成「点击 → `scrollIntoView({behavior:'smooth'})` → 立刻加 flash 类」。平滑滚动要几百毫秒，动画总共 1.4s——用户看到卡片时动画已过半甚至结束。截图里卡片确实滚到位了，只是**没有任何高亮**：功能「成功」了，用户得到的却是零反馈 | 闪光**等卡片停稳再开始**（`flashWhenArrived`：rAF 轮询卡片 `top`，连续两帧不动，或 1.2s 兜底——兜底同时覆盖「本来就在视口内」，那种情况没有滚动、会立刻稳定）。顺带修掉第二层不可见：原 keyframes 从 spread 0 长到 4px，前 ~200ms 等于没画，改为**立刻绘满 3px 环再淡出**。断言也改成「**到达那一刻**环的 spread ≠ 0」，而不是「有 box-shadow 就算过」（spread 0 的环是有值、无画面）。回归：`pnpm run test:provider-list-locate`。见 13.1 模式六十八 |
| 119 | 浏览器夹具**没包 `main`**，于是「滚到某张卡片」这类断言在一个**不可能滚动**的页面上通过 | 应用把 `html/body/#root` 锁成 `overflow: hidden`（`web/App.css`），真正滚动的是 `MainLayout` 的 `main`。夹具只挂 `<CodexPage/>` 时页面被裁切、`scrollIntoView` 无处可滚：位置类断言要么永远失败，要么在「页面根本不能滚」的前提下"通过"。同一轮还踩到第二个夹具坑：夹具 query 本身带 `&` 时，`runId` 不做 `encodeURIComponent` 会被 `URLSearchParams` 拆成额外参数，夹具永远等不到自己的 run id（表现为 10s 超时，看着像页面没渲染） | 夹具把页面包进 `<main style={{height:'100%',overflowY:'auto'}}>`，并注释写明「这是应用真实的滚动容器」。**凡是要验位置/滚动/吸附的夹具，先确认夹具里存在与应用一致的滚动容器**；同理，任何自带 `&` 的 query 都必须 encode 后再拼 run id。见 13.1 模式六十八 |
| 120 | **同一个产品区块在三处各写一份**，改一处就分叉一处 | 官方账号列表在 Codex 的供应商卡片里、Kimi 的独立卡片里、ZCode 的独立卡片里各写一份。三份的差异全是无意造成的：同一个动作三种叫法、同一句说明在一处是副标题另一处是脚注、切换按钮一处隐藏一处画成置灰。**没有任何一处报错**——只是每次改文案要改三遍，漏一遍就永久分叉 | 抽 `web/features/coding/shared/officialAccounts`：区块本体共享，各 CLI 只写「自家记录 → `OfficialAccountRowView`」的映射。**登录入口做成插槽而不是布尔开关**（Codex/Kimi 是按钮、ZCode 是下拉选 provider，共享组件不该知道一个 CLI 有几个官方 provider）；宿主由调用方挑（`embedded` / `standalone`），因为「挂在供应商卡片内部」与「自持一张卡片」是真实差异。守卫：`pnpm run test:official-accounts-shared`（源码棘轮，禁止再手写行级按钮）。见 13.1 模式六十九 |
| 121 | **登录动作会先落一行**，取消后那行永远留着 | Kimi 的「登录」在设备授权**之前**就先创建 official provider 行（前端 `handleStartOfficialAccountAuth`），而终态回滚是缺的：成功才回调，失败只弹提示，取消是静默终态。于是每次「点登录又关掉」都在供应商列表里留下一张空壳卡片（无凭据、无账号、配置是空模板）。用户在截图里圈出它问「为什么页面上还有一条这个记录」 | **登录必须是纯动作：成功之前不写任何行。** 后端在 token 交换成功那一刻才 `ensure_official_provider`（有就复用、没有才建），前端不再预创建；`start_*_device_auth` 也就不再需要 `provider_id` 参数。Codex / ZCode 本来就是这个形状——**判据：拿一个 CLI 的登录流程与另一个对照，看「取消」之后库里多了什么**。见 13.1 模式七十 |
| 122 | **凭据文件名与账号名是同一个字段**，于是永远只能有一个账号 | Kimi 的 `write_credential_file` 拿 `account.name` 当文件名，而真实 CLI 只认 `credentials/kimi-code.json` 这一个键——`migrate_legacy_credential_names` 反过来把所有账号**名**改成 `kimi-code` 来迁就文件名。两件事被绑死：多一个账号就要多一个文件名，可文件名只有一个，于是账号表按 `provider_id` 去重、永远最多一行 | **文件名与显示名解耦**：文件恒为 `credentials/<固定键>.json`（用常量，不读行上的 `name`），`name` 腾出来当显示名。磁盘只有一份 live 凭据，DB 里 N 行快照，切换时把选中的那份写回去——这正是 Codex 的形状（一份 `auth.json` + N 行账号）。**判据：账号行上的任何字段，先问「它是不是被当成文件名/键在用」**。见 13.1 模式七十一 |
| 123 | **把「这个授权没有身份」当成了事实，其实只是没去解码 token** | Kimi 的账号行只能显示「登录时间」，理由是「Kimi 的 OAuth 不发身份（CLI 的 `OAuthToken` 只有 token + 过期时间，也没有 userinfo 端点）」。用户一句「列表里都不知道是哪个账号啊」把这条推翻——**access token 是 JWT，payload 里就有 `user_id`/`sub`**，而 2.x CLI 还有 `GET {base}/me` 返回 `nickname`/`user_level_name`/`email`。我当时只读了 CLI 的数据结构就下了结论 | 账号行的显示名改用 `/me` 的 `nickname`，`user_id` 存进 `subject` 并**优先按它判重**（重登会换整套 token，只有身份不变），凭据指纹降级为回退。**判据：说「拿不到 X」之前，先把能拿到的东西解开看一眼**——token 是 JWT 就解码 payload，有同名接口就先请求一次（同模式七十二）。见 13.1 模式七十二 |

| 124 | **抽共享组件时，按「其中一个原实现」的形状抄，丢掉了其余两份共有的装饰** | `shared/officialAccounts` 的标题行是照 Codex 写的——Codex 恰好把 🔗 画在登录按钮上，于是标题行没图标；而 Kimi 与 ZCode 的标题行**本来都有** 🔗（Kimi 迁移前 `KimiOfficialAccountCard.tsx:125`）。抽完后 Codex 与 Kimi 的标题变成「只有加粗文字」，用户对照 ZCode 截图说「都改成 zcode 这种样式」。**图标不是功能，没有任何检查会报它丢了** | 抽共享组件时，把**每一份原实现的装饰逐个列出来取并集**（图标、数量徽章、副标题、分隔线、折叠箭头），再决定哪些是真实差异（进 `variant`）、哪些是共同契约（进组件）。判据：`git show <迁移前>:<每个原文件>` 并排读一遍。见 13.1 模式七十四 |
| 125 | **源码守卫匹配了渲染后的 DOM class，于是永远是绿的** | `verify-official-accounts-shared.mjs` 的 `FORBIDDEN` 写的是 `/anticon-swap/`、`/anticon-check/`——那是 Ant Design 把 `<SwapOutlined />` **渲染成 DOM 之后**的 class 名，`.tsx` 源码里永远不会出现。于是 geminicli / grok / antigravity 三份手写区块从未被扫出，脚本每次报告「0 still pending migration」的**假绿**，而它存在的全部意义就是找出这三份 | 源码守卫只能匹配**源码里真实存在的字符串**：组件名（`SwapOutlined`）、JSX 结构（`officialAccounts.map(`）、import 路径。写完守卫**先拿已知违规的文件试一次**（`node -e` 里直接 `test()` 一下），能命中才算守卫存在。见 13.1 模式七十三 |
| 126 | **把「某区块的样式改掉」做成了改这个区块自己的标题行**（其实是卡片头部那两行） | 用户要「把卡片头部这两行换成 `🔗 官方账号 (2)` + 说明」，我改了共享区块的标题行：区块标题从「账号列表」变成「官方账号」并加上 🔗 与说明，而卡片头部一行未动。用户回「**改错了啊**」并附三张图（头部长什么样 / 应改成什么样 / 列表要保持什么样）。**没有任何检查会报位置改错**：类型对、守卫绿、浏览器几何断言全过——因为每一行都还在，只是**在错的那一行上** | 位置类需求（「把 A 换成 B」「按钮放到 X 的右侧」）先把**候选行**写成 ASCII 预览或截图问清是哪一行，再动手；一个「区块」在卡片里往往有**两行**候选：卡片的头部行、以及区块自己的标题行。归属判据：这句话/这个按钮讲的是**这张卡片**（→ 卡片那两行）还是**这一串行**（→ 区块内部）。验收只能靠**坐标**：`login.top > heading.bottom`、`login.left >= listTitle.right`。见 13.1 模式七十五 |
| 127 | **同一个产品区块在三端各自拼装，样式一版一版地分叉** | Codex / Kimi 的官方卡片头部由各自的 `*ProviderCard.tsx` 拼（名称 + 🔗 + 计数），ZCode 的由它自己的卡片拼；措辞各写各的（Codex 的标题叫「账号列表」、Kimi 叫「官方账号」、卡片名还叫「Kimi Official」），类别标记 `官方订阅` 只有两家画，折叠能力只有 Codex 有。用户连续三条指令把它们收拢：「官方订阅 那个标志可以去掉了」「**Kimi Official 标题都统一使用官方账号**」「zcode 也加上账号列表的折叠能力，也就是官方账号卡片的样式是统一的可复用的」 | 共享的是**一整套形状**：标题行（🔗 + 官方账号 + 计数）、说明行、列表行（▾ 账号列表 + 登录入口）、账号行，加一个折叠能力。措辞收进 `common.officialAccount.*`（每个 CLI 仍自己传已翻译串，但传同一批 key），图标与计数收成两个导出（`OfficialAccountHeadingIcon` / `OfficialAccountCount`），折叠由宿主各持一个 state 但**能力三端都有**。默认状态可以不同（Codex 收起、Kimi/ZCode 展开），要共享的是能力不是初始值。见 13.1 模式七十六 |
| 128 | **守卫的作用域写成了文件名后缀，于是「名字不是那个后缀」的同类文件从未被扫到** | `verify-provider-card-layout.mjs` 管的是「卡片 = 薄映射层」（不许自己 `useSortable`、不许自己 `<Card>`、不许手抄选择框），可作用域是 `endsWith('ProviderCard.tsx')`。ZCode 的官方账号卡片叫 `ZcodeOfficialAccountCard.tsx`——它同样是**列表成员**（可拖拽、同一套外壳），却从未被扫过；它自己写 `<Card>` + `useSortable` + 把手，于是把手丢了悬停反馈、还被放进标题行内部，正文贴到卡片左边缘（用户圈出「**zcode 为什么下面的内容左边距没了，不是一个组件吗？**」）。它的 `EXEMPT_FILES` 里那条豁免理由（「不是列表成员、不可排序」）早已不成立，而且因为后缀不匹配**从来没被读过** | 守卫的**作用域**要和它声称的**语义**一致：写「所有卡片」，就别用单个后缀去筛（改成后缀集合，或按 import/结构判）。检查 `EXEMPT_FILES` 里的每条豁免是否真的会被读到——读不到的豁免是**假豁免**，它掩盖的正是它声称豁免的那类文件。外壳一律走 `CardShell`（边框、把手、悬停、内容列全在里面）。见 13.1 模式七十七 |
| 129 | **「按钮加到某个功能区」被放到了卡片的动作栏**（用户要的是列表标题行右侧） | 「账号区块」的登录按钮，我先按 ZCode 卡片的形状放进卡片头部右侧（`extraActions`），于是它和「应用 / ⋮」挤在一起。用户贴图：「**登录按钮应该是放到账号列表标题的右侧啊**」——它加的是列表里的一行，就该和它改动的那些行在同一行 | 按钮的落点看**它改动谁**：改列表 → 列表标题行；改卡片 → 卡片头部。改完用坐标断言钉住（同一行的中线差 ≤ 8px，且在标题行**下方**）。见 13.1 模式七十八 |
| 130 | **同一句话由两条渲染路径画，样式漂了一档灰度** | 账号区块的说明句：Codex / Kimi 经 `metaEntries` 走 antd `<Text type="secondary">`，ZCode 由区块自己画、用了应用的 `--color-text-secondary` CSS 变量。两个名字看着等价，实际不同——antd 的 secondary = `colorTextDescription` = **alpha 0.45**，CSS 变量是 **0.65**，于是 ZCode 那句比另外两家深一点。用户贴图问「文案的颜色和字体大小和另外2个的不一样吗？」——**类型检查、静态守卫、几何断言全过**，只有 `getComputedStyle` 看得见 | **「同一句话」= 同一个组件**，不是一个字符串加两处手抄样式：说明句收成 `OfficialAccountHint`（挂在共享区块里），两条路径都渲染它；`ProviderCardMetaEntry.value` 从 `string` 放宽成 `ReactNode`，因为**值本身就可以是个共享组件**——style card 只管它放哪，不管它长什么样。验收：真浏览器 `getComputedStyle` 断言颜色与字号（三端各一份套件）。见 13.1 模式七十九 |

| 131 | **计划里承诺的「产物」没逐项交付：教训、模式、清单都写了，那节「接入步骤」没写** | 这一轮的方案里明确写了「新增「官方账号区块」的接入步骤与最小验证（与卡片三样式那节并列）」。收尾时写了 13 节的 #120–#130、13.1 的模式六十九～七十九、4.0.2 的清单 bullet、附录 B.2 的区块段、守卫表行——**唯独那一节本身没写**。直到用户问「**sop 里加上官方账号的处理方法了吗**」才去查：§4.2 下没有这一节（4.2.1–4.2.10 是头部 / 列表 / 卡片 / 模型 / 弹窗…），附录 A.1 的共享组件对照表也没有这一行 | **教训 / 模式 / 清单 / 接入步骤是四种不同产物，一种顶不了另一种**：读者带着「我要接一个新 CLI，第一步做什么」来查时，不会去翻历史坑表（见 13.1 模式八十）。做法：把方案里承诺的每条产物写成清单，收尾时**逐条 `grep` 它在文件里的位置**，不是凭记忆说「我写了」；本次补 §4.2.9（两步接入：判宿主 → 写映射、三条后端不变量、最小验证）、附录 A.1 一行、附录 B.1 的 props 台账 |
| 132 | **新写的共享组件会被既有守卫按「文件名后缀」误扫**：`ProviderCompactCard.tsx` 差点被当成 CLI 映射层 | 新增卡片视图组件时，`scripts/verify-provider-card-layout.mjs` 的作用域是文件名后缀 `*ProviderCard.tsx` / `*OfficialAccountCard.tsx`。这个共享卡片**正是** chrome 的宿主（与 `CardShell` 同级，自己 `useSortable`、自己渲染复选框），按语义它和 `CardShell` 一样合法——但叫 `CompactProviderCard.tsx` 就会被扫出三条「不许做的事」 | **名字是契约的一部分**：改名不是绕过守卫，而是声明「这不是映射层」。命名前先跑一次守卫脚本确认扫不扫得到；真要留在作用域内，就在 `EXEMPT_FILES` 里登记**并写明理由**（该 Map 的注释就是这么要求的）。同一条也适用于「新组件放哪个目录」：`providerCardVariants/` 按语义选型、`providerCardView/` 按显示密度，放错了下一个人会照抄错的先例。见 13.1 模式八十一 |
| 133 | **共享组件的「呈现层」被误当成「第四种变体」**：卡片视图差点进 `providerCardVariants/` | 用户要的「供应商卡片布局」本质是**同一份数据的另一种呈现密度**（由用户偏好切换），而 `providerCardVariants/` 的三种样式是按 **provider 模型形状**（语义）选型的。若把卡片视图塞进去，`index.ts` 那张「选型依据」表就失去意义——「该用哪种样式」变成由显示偏好回答 | 判据：**「这个选择是语义差异还是显示差异？」** 语义差异（有没有模型目录、有没有单一 active provider）→ 进变体；显示差异（密度、排序、折叠）→ 独立模块。两者的共用面（accent 高亮、菜单契约）抽成 `cardChrome.ts` / `providerCardMenu.tsx` 供双方调用，**不要**为了复用把呈现层塞进变体。见 13.1 模式八十二 |
| 134 | **共享状态读取缓存被「另起一个 hook」复制成两份** | `useProviderListSort` 里有一个模块级单飞缓存，让所有 tab 共享一次 `get_provider_list_state` 往返。新增布局偏好时若另写一个自带读取的 hook，每个 tab 会多一次往返，且两份缓存各自漂移（A 页改了布局、B 页看不到） | 新增「每 CLI 一份的供应商列表偏好」必须接**同一个** `providerListStateCache`（读取走 `loadProviderListState`，写入走 `patchCachedProviderListState`）。判据：**「这份状态是不是同一个后端读取返回的？」** 是 → 共用缓存；不是 → 才另起。见 13.1 模式八十三 |
| 135 | **同一个状态在两种布局里「说」的方式不同：一种用标签陈述，另一种只有一个边框** | 卡片视图把「已应用」只用 accent 边框表达，列表卡片则在名字旁画 `AppliedTag`。用户 2026-10-12 报「**卡片上的已应用标识没了**」——边框只说了「这张特殊」，没说「为什么特殊」。类型检查、守卫、几何断言全过：两种表达都「有」，只是其中一种不陈述事实 | **同一状态的措辞必须来自同一棵树**：把 `nameTags` 拆成共用的 `stateTags`（官方/绕过/已应用/代理/P0）+ 各自的前缀（列表多一个地址），两种布局各取所需。判据：**「这条信息在另一种布局里是不是同一个组件画的？」**——若两边各判一次 `isApplied`，第一次改动就会分叉。见 13.1 模式八十四 |
| 136 | **新控件的图标与同排已有控件同形**：布局切换按钮撞上「通用配置」 | 切换按钮用的是 lucide `LayoutGrid`（2×2 方块），而「通用配置」是 antd `AppstoreOutlined`——**同一个 2×2 方块**（两家的 path 数据逐字不同、形状一样）。用户 2026-10-12：「布局切换 icon 换掉，现在的卡片时的按钮和通用配置的 icon 一样」 | 判据：**新增控件前，把候选图标与「同一行已有的每一个」比形状**（2×2 方块、三横线、加号、放大镜是最容易撞的几种通用几何），比不出来就换语文——本轮同时把按钮改成「图标 + 文字」，**文字才是有冗余的那部分**（单图标按钮没有第二处信息可以纠正误读）。见 13.1 模式八十五 |
| 137 | **「中间省略」被当成了 CSS 属性**：`text-overflow` 只能从尾部省 | 用户要「API Key 只显示一行，多了中间显示省略号」——key 的两端都在传达身份（前缀=签发方，尾巴=控制台里显示的那截），尾省略正好藏掉要找的那半。CSS 没有 middle-ellipsis | **展示规则是业务规则，落到 `.ts` 纯函数 + 单测**（`web/utils/middleEllipsis.ts`，Claude 与 Codex 侧栏共用）：浏览器断言只能覆盖「长 key 被省略」这一条路径，**「短 key 必须原样返回」这类边界恰恰是夹具不会带的状态**，只能靠单测。同一条：完整值必须另有出口（`title` 悬停 + 复制按钮），否则省略就是丢信息。见 13.1 模式八十六 |
| 138 | **共享渲染函数里写死了字号**：卡片 footer 想变小，内联 `fontSize` 挡在前面 | `renderProviderLaunchAction` 把 `fontSize: 12` 写在内联 `style` 里，而列表卡片头部与详情侧栏渲染的是**同一个函数**。用户要「卡片底下的按钮字号小一点」时，改宿主 CSS 无效——内联样式优先于样式表 | **共享渲染器只决定结构，密度属于上层面**：字号/内边距由宿主 scope（`.footerRow` / `.statusLine` / `.secondaryActions` 各自 `:global(.ant-btn)`）决定，两个宿主各要各的密度。判据：**「这个数值在另一个宿主里也该一样吗？」** 不一样 → 不许写进共享函数。见 13.1 模式八十七 |

| 139 | **「通用文案」被当成某一层的专名复用**：同一个侧栏里出现两个同名标题 | Codex 详情侧栏的模型绑定区标题用了 `common.model.title`（模型列表），而它下面那个**模型目录**区块自己的折叠标题也是「模型列表 (n)」——于是侧栏里两组不同的东西读起来是同一件事，其中一个还只是**一个**绑定。用户看到的形状是「两份模型列表，上面那份只有一行」 | 判据：**「这段文案会不会在同一个屏幕上出现两次、指两件事？」** 会 → 它在这个位置是**专名**，另起 key（`codex.model.mappingTitle` = 模型配置，与 Claude 的 `模型映射` 平行），不要图省事复用通用 key。收尾时顺手 `grep` 一次该 key 的全部渲染点。见 13.1 模式八十八 |
| 140 | **夹具的命令桩忽略参数，于是渲染出一个真实应用永远不会产生的页面** | `codexPageStubs.js` 的 `list_codex_official_accounts` 桩直接返回 `state.officialAccounts`，不按 `args.providerId` 过滤。页面会为**每个** provider 各调一次这个命令，于是自定义 provider 也拿到了 2 条账号：侧栏多出一个账号块，还带着「此供应商仍有历史官方账号，请先清理」的提示。**当轮 53 项断言全绿**——因为那时只有「官方 provider 有账号块」的正向断言，没有任何一条说「自定义 provider 不该有」 | ① 命令桩必须**按参数作答**（`args.providerId` 过滤），桩的作用是复现后端语义，不是提供一个够用的返回值；② 每条「有 X」的断言都配一条「不该有 X」的反向断言（本轮补「自定义 provider 没有账号块」，它当场就抓住了这个桩的错）；③ **截图是断言之外的第三条证据**——这条是看 `codex-card-view-detail.png` 时发现的。见 13.1 模式八十九 |
| 141 | **把 prop 的渲染点搬进共享文件后，守卫的「渲染点清单」没同步** | `modelSection.*` 的映射原本写在 `CodexStyleCard` / `OpenCodeStyleCard` 里，搬进共享 `ProviderModelSection.tsx` 后，`pnpm run test:provider-card-layout` 立刻报 7 个 prop（`modelSelectionMode` / `selectedModelIds` / `onToggleModelSelection` / `onBatchDeleteModels` / `onTestModels` / `testModelsDisabled` / `testModelsDisabledTooltip`）「没有任何样式渲染」——**它们全都活着**，只是不再出现在被扫描的那几个文件里 | 守卫的**作用域清单**（后缀、文件表、豁免表）与代码搬迁是**同一个改动**：搬渲染点时把新文件加进 `STYLE_CARD_FILES`（该表本来就列着 `providerCardMenu.tsx` / `providerCardActions.tsx` 两个同类共享文件）。判据：**「这个 prop 的渲染点还在被守卫扫描的文件里吗？」** 不在 → 补进去。假红与假绿（#124 / #125）是同一个缺陷的两个方向：清单要么漏了违规者，要么漏了合法者。见 13.1 模式九十 |

| 142 | **卡片高度成了「邻居有多高」的函数**：没有模型行的卡片把差额堆在分隔线上方 | 网格把一行里的卡片拉到最高那张，差额由卡片正文（`flex: 1`）吸收。Codex 官方行没有模型行，于是它比邻居矮 17px，拉伸出来的 17px 正好落在 footer 分隔线上方——用户 2026-10-12 贴图报「副标题离下面的分隔线太远」（他看的是有模型行的那张，间距其实来自整行的高度）。**类型、守卫、既有断言全过**：没有任何一条说过「卡片高度只由自己的内容决定」 | 判据：**「同一行里形态不同的卡片，高度会不会跟着邻居变？」** 会 → 把「有的卡片有、有的没有」的那一行按自身尺寸**预留**掉（`.body { min-height }`），让每张卡片自己就够高。**预留值要连内边距一起算**（`border-box`，本例 17px 文字行 + 6px 下内边距 = 23px，写 22 就差 1px）。回归必须**拿两张形态不同的卡片对比**，不能只断言某一张的数值。见 13.1 模式九十一 |
| 143 | **hover 显隐的按钮在「看不见」的时候仍然吃满宽度**，把同行的文本压到只剩几个字 | `ModelItem` 的动作区用 `visibility: hidden` 隐藏（对的做法，避免 hover 时布局跳动），但它**照旧占着 362px**；Codex 侧栏的模型行只有 478px，文本块只剩 60px，`deepseek-v4.1-flash` 被折成「DeepSee / k V4.1 / Flash (d / eepseek-…」四行——用户 2026-10-12 贴图报「明细里的模型列表展示有问题」。这些行在**列表卡片上**是好的（1176px 宽，文本拿到 642px），所以只有窄宿主才暴露 | **给文本一个最小份额、让动作区整组换行**：`flex: 1 1 220px` + `min-width` 保住可读宽度，行 `flex-wrap: wrap` 让放不下的动作落到第二行。关键是**换行由它们恒定的宽度决定，与 hover 状态无关**——所以指针进出不会重排（保住了 `visibility` 那个选择的初衷）。判据：**「把容器缩到最窄的宿主，这行的文字还剩多少？」** 别只在宽卡片上看。见 13.1 模式九十二 |
| 144 | **可折叠区块改成常开之后，守卫仍然在等一个已经不存在的折叠控件**——超时被读成「区块没渲染」 | 用户 2026-10-12 指定的「模型列表 / 明细里的账号列表不需要折叠」本是**明细侧栏**上的形态（见 #147），当时却把 `ModelListSection` 与 Codex 账号块的 Collapse 整个删掉了。`test:codex-official-accounts` 还在 `waitFor` **列表卡片**里的折叠 caret，于是超时报「账号区没出现」——那个 caret 后来确实该在（列表卡片保留折叠），而当时被读成了「区块没渲染」 | ① 报错指向的是**被删掉的东西**而不是坏掉的东西时，先确认「这个控件该不该被删」——本轮真正的错在 #147，测试只是先喊了出来；② 折叠区上「等 caret」应换成「等**内容**」（账号行、模型行）再加一层：内容才是「区块到达了」的证据，而且它顺带修掉另一个隐患——卡片元素早于区块存在，过早量几何会量到空盒子并误判「对齐正常」；③ 反过来也要查：`openDetailCatalog()` 这类「点开折叠区」的 helper 在常开后变成**空操作**，会静默让后面的断言失去意义；④ 判据——**「我删掉的这个控件，还有谁在等它？」** 改产品行为前先 `grep` 它的选择器。回归：列表卡片那侧重新变成「等 caret → DOM 点开 → 等行出现」。见 13.1 模式九十三 |
| 145 | **同一个共享组件被两个宿主复用，只改了其中一个的调用点，另一个跟着一起变了** | 用户说的是「**明细**里的账号列表不需要折叠」，而 Codex 的 `renderOfficialAccounts()` 同时喂**列表卡片的 footer** 和**详情侧栏**。去掉 `collapsed`/`onToggleCollapsed` 之后两处都不再折叠——卡片那处也跟着变了，而 `test:codex-official-accounts` 正断言卡片上有 caret（本轮 144 的超时就是这个引起的） | 判据：**「这个 prop 的调用点有几个？」** 共享组件的可选 prop 一改，先 `grep` 全部调用方，再决定「两处一起改」还是「按宿主传值」。**本轮一开始选错了**（「两处一起改，因为同一块内容在两个宿主里形态不同是凭空的差异」），用户当天的下一句话就推翻了它——共享组件的形态差异不是凭空，它正是 `OfficialAccountsSection` 的 `collapsible`（由「有没有传 `onToggleCollapsed`」决定）存在的理由：现在 `renderOfficialAccounts(foldable)` 按宿主传，列表卡片保留折叠、明细侧栏常开；ZCode / Kimi 只有一个宿主，照旧保留折叠。见 13.1 模式九十四 |
| 146 | **dnd-kit 的 `disabled: true` 意思是「不能拖，但**仍可被放下**」——不可排序的行成了上层拖拽的投放目标，拖卡片时 `over` 解析成行 id，卡片列表静默放弃重排** | `ModelItem` 对不可重排的模型行传 `disabled: !draggable`（布尔）。dnd-kit 的 `normalizeLocalDisabled` 把布尔 `true` 展开成 `{ draggable: true, droppable: false }`——**这两个字段是「是否禁用」，所以含义是「拖被禁用、放仍开着」：只关掉了一半**。Kimi 的模型列表不重排（`modelsDraggable: false`）且**没有自己的 `DndContext`**，于是这些行注册进了**供应商列表**的上下文：拖官方卡片经过模型列表时，`closestCenter` 选中 `provider-c-model-a`，`handleDragEnd` 的 `indexOf` 返回 -1 而静默返回。症状是「模型列表一展开，供应商就拖不动了」——**从卡片改成常开后必然发生**，而且只有真指针拖拽能发现 | ① `useSortable` 的 `disabled` 一律写对象 `{ draggable, droppable }`，别写布尔；② 判据——**「这个 sortable 的 id 在它外层 `SortableContext` 的 items 里吗？」** 不在 → 它的 `droppable` 必须关掉，否则它就是上层碰撞检测的噪声；③ dnd-kit 自己的 live region（`[aria-live]`）会写出 `Draggable item X was moved over droppable area Y`，**这是定位「拖拽没反应」最快的一条证据**，比读代码快得多。见 13.1 模式九十五 |
| 147 | **同一个共享区块，明细侧栏要的形态被当成了通用形态**：`ModelListSection` 的 Collapse 被整个删掉 | 用户 2026-10-12 在**明细侧栏**上提的四条——模型列表常开、行内动作让出宽度、字号小一点、账号列表常开——都落进了共享组件，于是**列表布局的模型列表也不再折叠**。用户 2026-10-13 报：「**列表的样式不要变啊，列表样式的模型列表怎么也没有折叠了**」。这正是 #145 自己刚写下的判据（「先数调用点」）**隔一天又被违反**：那次数的是账号块，这次连 Collapse 都删了 | **共享区块的形态差异写成参数，不要写成唯一形态**：`ModelListSection` 现在有 `appearance`（`'card'` 默认 = 折叠 + 卡片口径的行；`'detail'` = 常开 + 侧栏口径的行），`ProviderModelSection` 透传，**只有明细侧栏那一个调用点传 `'detail'`**。三处配套：① 让开 caret 的 18px 缩进从 `bodyStyle`（两个宿主共用的同一个对象）搬进区块自己，按 `appearance` 施加；② 行（`ModelItem`）同样加 `appearance`，卡片口径把 #143 的 `visibility` 占位行为**逐字还原**；③ 回归补成对断言——列表卡片「折叠着」+「点开后行在」+「行是 13px、把手在、带文字的动作占位」，明细侧栏保留原有的静止/悬停成对几何。判据：**「这条需求里的『列表』指的是哪个列表？」**——「明细」三个字出现在需求里，就说明另一方不该动。见 13.1 模式九十六 |
| 148 | **列表里的「非 provider 成员」在网格里没有位置**：ZCode 的官方账号卡片既是列表成员（可拖动排序、参与搜索过滤与空态），又不是 provider（它没有 provider 行） | 卡片布局的网格按「一格一个 provider」排布，账号卡片无处安放：**塞进一个格子**会被 350px 压成一条读不了的窄栏（它的内容是一串账号行 + 每行各自的动作），**提到网格之外**则丢掉「它是成员」这件事——它在列表里的槽位（`officialAccountIndex`）就没了着落。当时的列表布局还用 `providerIndex === officialAccountSlot` 单独实现了「插在哪里」 | **成员留在网格里，形状由内容定**：账号卡片跨整行（外层 `div` 上 `gridColumn: '1 / -1'`）并保持槽位，于是它同时是「一个网格项」和「一整行管理器」。两件事一起做实：① 两种布局的成员顺序来自**同一个** `listMembers` 表达式（列表布局原来那份 `providerIndex` 判断随之删除）——「账号卡片放在哪」不能有两个答案；② 断言要量宽度而不只读 CSS：`gridItemBoxes()` 断言它的左右边 == 网格的左右边、且 provider 格子都比它窄（一个「恰好独占一行所以看起来很宽」的卡片骗不过）。见 13.1 模式九十八 ⚠️ **本条的动作（跨整行）已被 #150 / 模式九十九推翻**：位置那一步（成员留在网格里、顺序来自同一个表达式）成立，**形状那一步不成立**——网格里一个成员唯一的形状是**格子**；#151 之后连管理器也移进了它自己的侧栏 |
| 149 | **`waitFor(侧栏存在)` 被「正在关闭的那个侧栏」满足**：抽屉关闭是异步的，面板要等动画结束才卸载 | Kimi 的卡片视图检查在 provider-a 的侧栏里读完，`closeDetail()` + 固定 `delay(300)` 之后点官方卡片、再 `waitFor(detailPanel())`——这一步**立刻通过**，因为上一个面板还在页面上（正在关闭）。于是后面两条断言读的是 provider-a 的面板：「官方侧栏里有账号块」判为 false。**报错指向的是产品（账号块没跟过来），实际是两个面板的时序** | ① 关掉一个异步卸载的东西之后，**先等它消失**再等下一个出现（`waitFor('!fixture.detailPanel()')`）；② 更好的等待条件是**新面板的内容特征**而不是「面板存在」（`detailTextIncludes('账号列表')`），它同时排除了「上一个还开着」和「新的还没到」；③ 固定 `delay` 在这里是猜动画时长——同一条已有定论（见模式九十三：等待条件要选「内容」而不是「控件」）。见 13.1 模式九十七 |
| 150 | **列表里「不是 provider」的成员，形状被两个宿主各答了一次**：卡片模式下 ZCode 的官方账号那块内容与邻居不同形 | `ZcodeOfficialAccountCard` 的列表形态是 `CardShell`（和 provider 卡片同一张描边卡）。接卡片布局时只给它搬了位置（跨整行、保住槽位），于是它仍穿着列表的外壳——一整行宽的描边卡浮在网格里，读起来像第四种卡片样式；用户 2026-10-13 报「卡片模式 zcode 的官方账号怎么还是列表样式」。改法是**不套外壳**，于是又变成一块裸内容摆在格子之间（既不是格子也不是卡片），用户当天再报一次「还是这样啊」 | **形状归宿主布局，而网格里一个成员唯一的形状就是格子**：它走**同一个** `ProviderCompactCard`，与邻居同宽同高（2026-10-14 定稿：它在那张格子自己的详情侧栏里放账号管理器、第二行只说「当前登录: <账号>」，与 Codex / Kimi 的卡片模式同形）。它因此回到普通网格项：没有跨整行、没有「网格之外的特殊路径」，排序/定位/搜索/空态全按格子走。判据：**「这个成员的形状由谁定？」**——宿主定，就把它画成宿主里其他成员的形状，而不是为它发明第三种（跨整行）、也不是让它不穿衣服。回归要**量几何**（它与邻居同宽 ±1px）而不是断言 CSS：第一次错的那版量出来的宽度是对的（跨整行也是「一整行」），错的是形状。见 13.1 模式九十九 |
| 151 | **同一段 chrome 在四个映射层里逐字重复**：`<Drawer size="min(60vw, 760px)" destroyOnHidden closable={false} styles={{body:…}}>` + `ProviderDetailPanel` 各写一份 | 面板（`ProviderDetailPanel`）早已共享，但**装它的抽屉**没有：Claude / Codex / ZCode / Kimi 四份拷贝的 placement、宽度、`destroyOnHidden`、`closable`、body styles 一字不差，唯一不同的是面板内容。用户看到「ZCode 的官方账号块也得进抽屉」时说「**样式都封装到通用组件啊，不要每个单独改啊**」 | **抽屉跟面板一起归共享卡片**：`ProviderCompactCard` 收 `detail = { title, statusTags, primaryActions, secondaryActions, sections }`，自己持有开合状态、自己渲染 `<Drawer>`；四个映射层删掉 `Drawer` / `ProviderDetailPanel` 的 import 与 `detailOpen` state，只算内容。判据：**「这段 chrome 的调用点有几个？」≥2 就搬进共享组件**——n 份拷贝不会同时坏，所以不会有任何一个测试发现它们漂了。附带一条：`handleCardClick` 的忽略列表要加 `[data-provider-card-handle]`——把手是 `<span>`，只按标签名（button/a/input）过滤会让「按住把手拖一下」顺带打开侧栏，补一条反向断言「拖它不会打开侧栏」（#151 实测：不补也能过大部分断言） |
| 152 | **一条结论被推翻后，正文里被推翻的实例没清**：SOP 有三处在讲已经不算数的答案 | #148 / 模式九十八（「留在网格里、跨整行」）被 #150 / 模式九十九推翻，正文里**没有任何标注**；13.1 里还留着一整段**没有标题的孤儿块**，讲的是**已经整份回滚**的 `ProviderCardGrid.rowItem`；`providerCardView/AGENTS.md:60` 仍在说「ZCode 的账号卡片在网格里跨整行」，与同文件第 34 条（「它就是一个普通格子」）**直接冲突**。于是用户问「**sop 里面没写吗？**」——写过了，但读者先撞上的是那三处旧答案，读起来就是「没写」 | **推翻一条结论 = 清理它的全部实例**，四件一起做：① 教训表那一行加「结论已被 #NNN 推翻」（#106 的先例：规则仍成立、实例判错时也要标）；② 13.1 的模式行同样标注；③ 孤儿段落删掉——**回滚一个方案时要把它留下的那一段一起删**（本例 `rowItem` 那段）；④ `grep` 被推翻做法里的**特征标识**（本例 `renderBody`、`rowItem`、`跨整行`、`onOpenDetail`、`bodyCustom`）扫全仓 `*.md`，逐个改掉。判据：**「读者照这份文档做，会做出哪一个？」**——只要正文里还剩一个旧答案，答案就是旧的 |

| 153 | **「没内容可写」被实现成了「不写」**：官方渠道格子的第二行在渠道闲置时整个消失 | 卡片布局里官方格子（Codex / Kimi / ZCode 三端同形）的第二行只说一件事——「当前登录: <账号>」，而取值只认 `isApplied` 的行。用户那台机器上**两家渠道都闲置**（`codex_provider` 里应用的是 AxonHub、官方渠道 `is_applied=0`；Kimi 两个渠道都没应用），于是那行没有可选值 → 不渲染。用户连着问「**为什么 codex 和 kimi 的卡片下面没有显示当前的账号啊**」。**代码没坏、断言全绿**：三条套件都只断言了「有应用中的账号时」那一态——规则太窄，而不是实现错了 | **「说一件事」的行要枚举状态，每条状态都给输出**：取值改为 `find(isApplied) ?? find(isVirtual) ?? 「未登录」`（未登录用 `common.officialAccount.currentLoginNone`），**不再有留空的那一态**——那行的位置无论说不说都被共享卡片占着（`.bodyWithAction`），留空读起来是坏卡片，不是「无话可说」。三件事一起做：① 夹具加两个模式（`accounts=virtual` = 只有磁盘上那份登录、`accounts=idle` = 有账号但没在用），断言把**三态**都钉住——只断言「有值的那一态」等于没断言；② Codex 那份仍在 `showOfficialRuntimeState` 门控内（网关代理/接管时 `auth.json` 装的是网关的凭据，那两行都不是本渠道的当前登录，于是和任何闲置渠道一样说未登录）；③ 取舍口径问用户：他选的是「**只写真正在用的登录**」，所以闲置态写未登录，而不是把它保存的账号说成当前登录。见 13.1 模式一百零二 |

| 154 | **「这个 CLI 没有主操作」被当成了「这张格子不说话」**：ZCode 格子的动作槽一直空着，而它唯一的 per-provider 控件躺在「更多」菜单里 | ZCode 的 provider 永远不会被应用（`设为默认` 在模型行上），所以 `ProviderCompactCard` 的第二行动作槽没人用——而**每个邻居格子都在那个位置说自己的控件**（应用 / 网关代理）。用户 2026-10-15 指出：「**针对 zcode 这种不需要应用的渠道，把它在列表模式下的开关按钮放出来，放到现在应用的地方**」——列表卡片头部本来就画着那颗启用开关，格子这侧却把它折进了菜单 | **「没有主操作」不等于「没有控件」**：给卡片加 `cellControl` 插槽，让 CLI 把**它自己的那个控件**放进动作槽（ZCode 传共享的 `ProviderEnabledSwitch`，2026-10-15 同步把列表头部与菜单那两份收成**同一个组件**——三处各写一份，`checked` 的取反方向或 tooltip 文案迟早有一处漂）。**传了就撤掉菜单里那一条**（`cellControl ? undefined : onToggleDisabled`）：菜单收起它本来就是「旁边有 应用 会争抢」，格子那行没有主操作，理由不成立，留着就是同一张卡片说两遍。判据：**「这个位置别的格子都说了什么？本 CLI 对应的那个控件是什么？」**——不是「本 CLI 有没有 应用」。回归要真点一下（`toggle_zcode_provider_disabled` 的参数、卡片随即陈述「已禁用」、回来的路仍在同一行、点它不打开侧栏、菜单里没有第二条）。见 13.1 模式一百零四 |

| 155 | **先写页面、后读 §4.0.2**：共享组件用对了，形态全错——自建 `Modal` + `Form` 取代 `ModelFormModal`，自建折叠区取代 `ProviderListSection`，页面里散着一堆 `<Alert>` | 接入 WorkBuddy / CodeBuddy 时，先按「能编译、能跑」写出两个页面：`Collapse` 手搓供应商区、`Modal` 手写模型表单、`Alert` 堆提示。用户随后要求「**注意要遵循 SOP**」。读完 §4 才发现 §4.0.2 的 A–H 逐项核对**根本没做**——不是漏了一个 prop，是整层外壳都没接 | ① **§4.0.2 的核对必须在写第一行前端代码之前**：先打开参照 CLI（本节指定 Codex / Pi / Hermes）逐行读，再动手；「编译通过」不是形态正确的证据。② 共享组件在写页面时先 `ls` 一遍 `web/features/coding/shared/`，把 `CodingPageHeader` / `ProviderListSection` / `providerCardVariants` / `ModelFormModal` / `GlobalPromptSettings` 逐个对照「我这个页面哪块该用它」。③ 两个同引擎的 tab 共用一段 UI 时，把它写进 `shared/`（本轮落到 `shared/modelsJson/`），不要让 A 的页面 import B 的组件目录——那会把「共享」伪装成「借用」，下一个改 B 的人不知道自己在改公共件。见 13.1 模式一百零五 |
| 156 | **共享类型写了两遍**：`WorkbuddyModelInput` 与 `CodebuddyModelInput` 逐字段相同，两个页面的表单却各自绑定一份 | 两个产品同引擎、同 `models.json`，类型定义仍各写一份。改其中一处字段，另一处不会报错——共享弹窗组件的 `form` 与 `channel` 类型对不上，于是出现了 `as unknown as` / `as never` 的类型逃生口 | ① **类型不一致时先找是不是重复定义**，`as` 只在该处类型确实应该不同时才用；本轮修法是把三个形状下沉到 `web/types/workbuddyFamily.ts`，两个模块 `export type { X as Y }` 再导出，调用方一行不用改。② 判据：**「这两个 type 的字段列表能否逐行对上？」** 能 → 就是一份，不要两份。③ `as unknown as` / `as never` 出现在 PR 里本身就是信号：它在说「这里有两套不兼容的类型」，先查来源再消音。见 13.1 模式一百零六 |
| 157 | **托盘「其它 tab 都有」被当成「我也要有」**：为两个模型形状不匹配的 tab 硬凑托盘 | 托盘语义是「在多个已应用 provider/模型之间切换」。WorkBuddy / CodeBuddy 既无单一 active provider，也无可靠的默认自定义模型指针（`preferences.json` 的 `defaultModelId` 桌面版不写），托盘上没有可切换的对象 | **接入前先问「托盘在这里切换的是什么」**：答不出具体对象，就不做，并把这个决定**写进计划文档**（本轮是 §10.1 第三条）——否则下一轮 review 会把它当遗漏补上。判据：**「本 CLI 有没有一个『当前生效』指针？」** 没有 → 托盘不适用，不是缺失。见 13.1 模式一百零七 |

| 158 | **页面外壳没接 `SectionSidebarLayout`，行为上「能跑」，形态上和其他 16 个 tab 不是一个产品** | WorkBuddy / CodeBuddy 第一版页面直接 `<Spin><div style={{padding:16}}>` 把内容铺开：没有区块侧栏导航，也没有 `.pageContent` 的 `block` 布局；同时还漏了 `routeConfig` 的 `contentPadding: 'compact'`，内容区退回 24px。三件事都不报错——`tsc` 过、`i18n:check` 过、组件夹具过 | ① **新建 coding 页面先 `grep -c SectionSidebarLayout web/features/coding/*/pages/*.tsx`**：16/16 都有，新页面也必须有一个 `sections` 数组 + `data-<tool>-sidebar-section` 标记 + `getIcon`。② 容器类**只写 `display: block`**，间距由区块自己的 `margin-bottom: 16px` 承担（§4.3.1）。③ `routeConfig` 的每条 coding 路由都要 `chrome: { contentPadding: 'compact' }`，否则内容区内边距与其他页不同（24 vs 8+16）。④ **把新页面加进 `pageRhythmFixture` 的 `PAGES`**，让 `test:page-rhythm` 真的量它的页头→首区块间距——守卫只量名单里的页面，不在名单里的页面等于没被检查。见 13.1 模式一百零八 |
| 159 | **预览弹窗把「解析后的视图」当成「文件内容」**：`models.json` 预览用 `JSON.stringify({providers, availableModels})` 现场拼 | WorkBuddy 第一版「预览配置」把 `FamilyRuntimeConfig` 重新序列化。那份视图**丢掉没有 URL 的条目、重排字段、且不含产品自有键**——用户看到的是一份磁盘上不存在的文件，而预览的全部意义是「我现在到底写进去了什么」 | **预览必须读原文**：后端加一个只 `std::fs::read_to_string` 的只读命令（本轮 `read_workbuddy_models_file`），前端直接渲染字符串。判据：**「这个预览的内容，`read_to_string` 能不能直接得到？」** 能就不要再 JSON 序列化一遍——任何 parse→serialize 往返都会改变用户文件的形状。同一条约束也适用于 `settings.json` 只读面板（`read_codebuddy_settings_file` 早已是这个形状）。见 13.1 模式一百零九 |
| 160 | **补登记 i18n 动态前缀之后，`i18n:check` 立刻炸出 36 个缺失 key**——而在此之前它一直「通过」 | `useRootDirectoryConfig` 的 `translationKeyPrefix` 保护表里没有 `workbuddy` / `codebuddy`。保护表只影响 `prune` 的安全区，`check` 是**按保护表决定扫哪些展开式**的，于是这两个模块的 `rootPathSource.modal.*` 从来没被检查过。补上之后一次性暴露 18 个 key × 2 个工具（每个 key 在两种语言各报一条，共 72 行输出） | ① §12.1 那条「补完保护表后必须重跑 `i18n:check`」是**真的会红**，不是形式主义——它是这类缺口唯一的发现途径。② 新增 tab 用任何会拼 key 的共享组件（`useRootDirectoryConfig` / `GlobalPromptSettings` / `ModelFormModal`）时，**先登记前缀再写页面**。③ 缺失的一律按参照模块的措辞补齐（本轮照 `dsh.rootPathSource.modal.*`），工具名与环境变量名按本 CLI 替换；不要用空串或复制工具名占位。见 13.1 模式一百一十 |
| 161 | **备份打包段用 `format!("external-configs/{tool}/")` 动态拼路径，于是 §12.1 的「打包 ↔ 恢复」集合核对看不见它** | 那条核对命令 grep 的是 `external-configs/<tool>` **字面量**。打包段写成循环 + `format!` 时，grep 在 utils 侧一个都抓不到，diff 的输出变成「restore 多了 workbuddy / codebuddy」——看上去像恢复端多了两个孤儿分支，实际是**打包端确实打包了，只是检查脚本看不见**。危险在于反向也成立：真漏一个工具时，同样的写法会让核对**假通过** | ① **两个集合必须写成字面量**：逐个工具直写 `"external-configs/workbuddy/"`，即使因此重复几行。这是「守卫可读性优先于 DRY」的一处正当例外——守卫看不见的 DRY 就是不可验证的代码。② 改完立刻跑核对命令，**输出必须为空**。③ 判据：**「这条不变量有没有机械守卫？守卫抓的是哪种写法？」** 有守卫就按守卫能看见的写法写，否则守卫只是装饰。④ 同理适用于任何按 `DbTable` / tool 循环构造**路径或 key** 的代码——先看守卫怎么抓。见 13.1 模式一百一十一 |

| 162 | **页面接了 `SectionSidebarLayout`，但漏了 `SidebarSettingsModal`——侧栏隐藏后再也打不开** | 补完页面外壳后，「更多选项」弹窗与侧栏开关都没接。全仓 14 个 coding 页面都有这一对：页头 `onMoreOptions` → `SidebarSettingsModal`（内含「显示侧栏」开关）。漏掉的后果不是少一个入口，而是**不可逆的界面状态**——用户点掉侧栏后没有任何办法恢复 | ① **`grep -c SidebarSettingsModal web/features/coding/*/pages/*.tsx` 必须覆盖全部页面**，和 `SectionSidebarLayout` 一起查。② `sidebarHiddenByPage.<key>` 只被这个弹窗写，页面读；读侧漏了会显示错，写侧漏了会卡死。③ 判据：**「这个开关关掉之后，用户还能从这里打开吗？」** 不能 → 必须有反向入口。 |

| 163 | **两个同引擎的 tab 共用一段文案时，把产品名写进了共享键里** | CodeBuddy 页面复用 `workbuddy.availableModelsFree`，而它的中文是「未限制可用模型：**WorkBuddy** 会显示内置模型…」。页面能跑、`i18n:check` 通过（键确实存在），但 CodeBuddy 用户读到的是「WorkBuddy 会显示…」 | ① 跨模块复用的文案必须下沉到 `common.*` 并**去掉产品名**（本轮 `common.availableModelsFree` / `common.availableModelsRestricted`）；② 判据——**「这句文案里有没有出现某一个模块的名字？」** 有 → 它就不是共享键，要么参数化 `{{tool}}`，要么留在该模块自己的命名空间；③ 迁移后按 SOP 的「只删真正无引用的」清理旧键（`i18n:prune --prefix <exact-key>`，注意 prune 的前缀是**精确键**而不是命名空间，`workbuddy.availableModels` 匹配不到 `workbuddy.availableModelsFree`）。 |

### 13.1 静默失效的模式（归纳）

上表的坑可以归成下面几类，识别出模式就能提前防：

**模式一：白名单 / 映射表漏项。** 用一个手工维护的列表去 gate 行为，新增实体时漏改一处 → 该实体永久静默失效。
> 例：#2 `TAB_TO_MODULE`、#13 `detection.rs` 白名单、#15 `wsl_module_for_reapply_label`、#5 kimi Gateway 注册。
>
> **对策**：优先让共享 resolver 自己处理未知 key（返回 `None` 后走兜底），而不是在调用侧维护副本列表。见 7.2 的结构性修法。

**模式二：可选 prop 决定渲染，漏传看起来像设计如此。** 组件按「传了才渲染」组织 UI，漏传一个 handler → 少一个按钮/分区，且**没有任何报错**。
> 例：#26 `ModelListSection` 工具栏、#23 `ProviderFormSections` 分区。
>
> **对策**：迁移时**枚举组件的全部可选 prop**，逐个决定传/不传并记录理由（4.0.2-F）。

**模式三：错误被吞掉。** `catch` 只打 console，UI 呈现为正常空态。
> 例：#28 模板加载失败显示「暂无数据」。
>
> **对策**：`catch` 必须给用户可见反馈，除非该失败确实无需用户知晓（此时要写明理由）。

**模式四：同一能力有多个消费点，只做了一个。** 一个能力往往不止一处入口——「预设模型」既要在模型弹窗里选，也要在「获取模型」的应用回调里按 id 匹配；「模型目录」既要在卡片上增删改，也要在保存供应商表单时原样带回。做了其中一处，**另一处不会报错，只是白拿不到数据**。
> 例：#31 预设匹配、#36 同一条。
>
> **对策**：实现一个能力时先问「**谁还会消费这份数据**」，把转换逻辑抽成共享模块（如 `utils/zcodeModelFields.ts`），两个调用点共用一个定义。**两处各写一遍必然漂移**——这正是 #31 的成因。

**模式五：写进了运行时不读的文件 / 键。** 目标路径选错不会报错——写盘成功、返回成功、界面打勾，只是那份数据没人看。工具自己换了一代存储、或同一份状态有新旧两个落点时必然出现。
> 例：#38 ZCode 官方账号切换还原 `config.json`；而 `provider_config.json` 一旦存在，运行时**完全不读** `config.json`。
>
> **对策**：写入前用**运行时的判断依据**做 gate，而不是「文件存在就写」。ZCode 的代际判断已有现成的 `provider_config_exists()`——照抄它，不要在调用侧另写一套（同模式一）。

**模式六：把「显示用」和「搬运用」混成一件事。** 快照/搬运类能力应当**整块原样复制**，只有为了展示才去解析内容。混起来要么搬丢数据，要么在不该失败的地方失败。
> 例：#39 官方凭证是 `enc:v1:` 密文，切换**不解密**（原样复制，字节等价），只有取名/去重才解密；#40 去重若只依赖解密结果，密钥不匹配（快照来自另一台机器）时身份为空 → 每次都当新账号。
>
> **对策**：搬运路径与展示路径分开写；去重先比**字节**再退到**语义**（身份），这样解密失败只损失一个显示名，不损失正确性。

**模式七：把内容塞进「空态会替换掉」的插槽。** 列表组件的空态分支（无数据 / 搜索无结果）常常是 `Empty` **取代** `children`，而不是追加在它旁边。凡是「和列表并列存在」的区块塞进 `children`，就会在空态整块消失，且只在**恰好为空**时才复现。
> 例：#42 官方账号卡片；搜索无结果时同样消失。
>
> **对策**：区分「列表内容」与「和列表并列的区块」——后者要单独的插槽（ZCode 的 `alwaysVisible`），并**在空态、搜索无结果态各看一遍**。空态是新区块最容易被漏测的状态：开发时手上总有数据。

**模式八：把「已经归属别人的文件」又编辑了一遍。** 一个 CLI 的配置文件往往各有归属：MCP、Skills、权限都有专门页签，通用配置再打开同一个文件，就有两处在写同一份内容，而界面上看不出谁才是权威。
> 例：#43 ZCode 的通用配置编辑 `cli/config.json`（MCP / hooks / 插件 / 权限），与 MCP 页签撞车；它真正该管的是 `provider_config.json` 的共享部分。
>
> **对策**：动手前先问「**这个文件还有谁在写**」。若已有专门页签，通用配置应退回到「不随实体变化的共享部分」，并在弹窗描述里写明边界——否则用户会以为这里是唯一的入口。

**模式九：新 CLI 的接入点散在多个 match 里，漏一个不报错。** 「本地 session 统计」这类能力要改 7 处后端 + 2 处前端，每一处都是 `match`/数组的一项；漏掉的后果是**该 CLI 静默地永远没有数据**，而其它 CLI 一切正常，很难联想到是漏注册。
> 例：#44 `session_files()` 的 `root.is_dir()` 让 ZCode 的数据库文件永远扫不到；#45 是完整的接入点清单。
>
> **对策**：照抄一个**同类**工具（同为「根是文件/有专门解析器」的：Hermes / Dsh / ClaudeDesktop）逐处对照，而不是从零找。清单见下表。

**「网关本地 session 统计」新增 CLI 的接入点清单**（2026-10-06 实测）：

| # | 位置 | 作用 |
|---|---|---|
| 1 | `proxy_gateway/types.rs` `GatewayUsageTool` 枚举 | 新增变体 |
| 2 | 同文件 `GatewayUsageTool::all()` | 加进全量列表 |
| 3 | 同文件 `as_str()` | 落库的 `app_type` 字符串 |
| 4 | 同文件 `GatewaySessionImportCli` 枚举 | 前端单选值的反序列化目标 |
| 5 | `session_import.rs` `import_cli_keys()` | 单选值 → 工具 |
| 6 | 同文件 `default_session_roots()` | 扫描根（**根可以是文件**，见 #44） |
| 7 | 同文件 `session_files()` / `parsers.rs::parse_file()` | 文件筛选与解析器分派 |
| 8 | `usage_stats.rs` `load_provider_names()` | 供应商名映射（否则列表里只剩 id） |
| 9 | 前端 `proxyGatewayApi.ts` `GATEWAY_USAGE_TOOLS` | 筛选下拉项 |
| 10 | i18n `settings.gateway.cli.<key>` | 显示名 |

**模式十：布局间距借自「邻居」，顺序一变就塌。** 让 A 依赖 B 存在（或依赖 B 的 margin）来产生间距，在列表里是隐形耦合：顺序一换、B 被删、B 被条件隐藏，间距就没了，而**代码里两处都看不出问题**。
> 例：#46 ZCode 供应商卡片自己没有 `marginBottom`，靠下方官方账号卡片的 margin 撑开；两张卡调换顺序后贴在一起。
>
> **对策**：列表项**各自带完整的间距**（自己的 `marginBottom` 或父容器 `gap`），不依赖相邻项。核对方法：把列表里的项**随机调换顺序**看一遍——顺序无关的布局才是对的。

**模式十一：状态标记只在一个写入路径上维护。** 同一个「当前生效的是哪个」的语义，如果有多个写入路径（切换、保存、外部工具直接改），只在一处写标记，另外两条路径就会显示成「没有生效项」。
> 例：#47 ZCode 官方账号的 `is_applied` 只在切换命令里写；「保存当前登录」和「在 ZCode 里直接登录」都不经过它，于是已生效的账号不显示「默认」。
>
> **对策**：这类标记应当**从事实源推导**，而不是当状态存。事实源是磁盘上的配置文件时，列表接口就每次读它回填，别指望每个写入路径都记得更新。

**模式十二：模块建了，文档没建。** 根 `AGENTS.md` 要求每个模块目录有 `AGENTS.md` 并登记进 Index，但**没有任何机制会在缺失时报错**——新模块往往是先写代码、文档「下次再说」，然后就一直缺。缺文档的代价不是「少一份说明」，而是**下一个人（或 agent）会照着错误的心智模型改代码**。
> 例：#50 web 侧 `zcode` / `pi` / `oh_my_pi` / `dsh` / `claudedesktop` 五个目录一直没有 `AGENTS.md`。
>
> **对策**：把「建模块目录」和「建 `AGENTS.md` + 登记 Index」当成**同一个动作**，写进收尾清单（见 4.0.2）。审计方法见 12.1——**前后端各一条命令**（`tauri/src/*/` 也要覆盖，否则 `db` / `settings` 这类非编码域模块会被整片漏掉，见 #51）。

**模式十三：配置存了，但没有任何代码读它。** 界面提供一个「通用配置 / 高级配置」编辑器，保存成功、重新打开还能读回——于是**看起来完全正常**。但没有任何代码把存下的内容合并进真正生效的文件。用户改的是 DB 里的一个字符串，运行时读的是磁盘上的另一份文件，两者永不交汇。这比「保存失败」更危险：失败至少会报错，而它只给一个成功的假象。
> 例：#52 ZCode「通用配置」把 JSON 存进 DB，却从未合并进 `provider_config.json`；而且上游 `storedProviderConfigSchema` 是 `.strict()` 的，只认 `providerConfigRules` / `modelConfigRules` / `providerOrder` / `defaultModelSelection` 四个键——就算真去合并，写任何别的键也会被 ZCode 拒绝。
>
> **对策**：新增任何「配置编辑器」时，必须能指着一条**读回路径**说出「这份内容在哪里被消费」。只写不读的字段应当在实现前就暴露出来，而不是等用户发现「改了没反应」。核对方法：对每个存储键跑一次全仓 `rg`，确认除 get/save 命令外还有**第三方消费者**。
>
> **附带教训**：`.strict()` 的上游 schema 是硬边界。设计「共享/通用配置」这类自由编辑区之前，先读上游 schema，确认它到底允许哪些键——否则功能从第一天起就是死的。

**模式十四：API 封装写好了，UI 从未接上。** 前端 `services/*Api.ts` 里有 invoke 封装、后端命令也注册进了 `lib.rs`——两层都「完成」了，只有第三层（调用它的组件）不存在。读代码时看到封装会以为功能已实现，实际用户永远碰不到。与模式十三的区别：那个是**存了没人读**，这个是**能调没人调**。
> 例：#54 `toggleZcodeProviderDisabled` 全仓只有定义、没有调用；ZCode 的「禁用」开关从未接上。
>
> **对策**：接新 CLI 时，对每个 API 封装跑一次 `rg "<apiName>" web/ --glob '!services/**'`，确认除定义外还有调用点。反过来，删除 UI 入口后也要反向确认封装是否变成了死代码。

**模式十五：同一个概念有两个存储位置，只有一个被写。** 「默认」这类状态常常在两层各存一份：一层是 CLI 自己的目录内偏好，一层是运行时的实际指针。前端只写了其中一层 → 界面显示的状态与运行时行为不一致，而且**两边都不报错**。
> 例：#55 ZCode 的 `settings.models[].isDefault`（provider 目录偏好）与注册表 `defaultModelSelection`（运行时指针）脱节；卡片显示「当前默认」，ZCode 却从不使用那个模型。
>
> **对策**：判断「当前是哪个」时以**运行时事实源**为准，不以 UI 自己写的标记为准。写入时如果两个位置都要改，就封装成一个动作，不要在两个入口各写一半。

**模式十六：读一个键，写另一个键。** 一个状态在组件里出现两次——一次读、一次写——用的是两个不同的标识。因为两边都不报错（读有默认值，写有合法目标），界面看起来完全正常，只是**开关不起作用**。最容易发生在新 CLI 从另一个 CLI 复制页面时：复制来的读侧还指着来源 CLI 的键，写侧已经改成了自己的。
> 例：#56 `OhMyPiPage` 读 `sidebarHiddenByPage.pi`、写 `setSidebarHidden('oh_my_pi')`；`pi` 与 `oh_my_pi` 是两个独立键，开关因此静默无效，还顺带把 Pi 页的开关状态显示到了 OMP 页。
>
> **对策**：任何「一个 key 读、一个 key 写」的配对（侧栏折叠、`visible_tabs`、`runtime_location` 的模块键、WSL 映射的模块名、事件名），复制页面后必须逐对核对读写同名。**命名相近的两个实体**（`pi` / `oh_my_pi`、`claude` / `claudecode` / `claudedesktop`）是重灾区。

**模式十七：视觉/交互一致性问题，任何自动化检查都发现不了。** 前面十六个模式大多能靠 `rg`、编译或测试兜住；这一类不行。文案位置、按钮边框、间距、排序能力、空态措辞——**编译器不管、类型系统不管、i18n 检查不管、测试也不管**。它们的共同特征是「功能都正常，只是看起来/用起来不对」，所以只有**打开参照 CLI 并排看**才会发现。
> 例：#58 说明文字放在控件上方、#59 卡片按钮带边框、官方账号卡片不能排序。三者都不影响功能，都能编译通过，用户一看就觉得不对。
>
> **对策**：
> 1. **视觉核对不可省略**（§12.3）。它是这一整类问题唯一的检出手段——没有跑起来看过，就不算验证过。
> 2. **并排比对，不是「凭印象」**：把参照 CLI 的同一屏和本 CLI 的同一屏放在一起，逐个元素指认。单独看一个页面永远觉得「挺正常的」。
> 3. **把判据写下来**（如「卡片内次要操作一律无边框」「解释性文字在控件下方」「视觉同级的区块要能一起排序」），而不是记住某个具体页面的样子——判据才能迁移到下一个 CLI。
> 4. 新出现的这一类问题，除了记进第 13 节，还要判断它是否该变成 4.0.2 的一个核对项——**能问出来的问题才是可复用的问题**。

**模式十八：字段集合是「实现者认为该有」，不是上游开放的。** 给某个 CLI 自建表单时，字段往往按「这个模型有哪些属性」来定，而不是按「上游允许用户改哪些」。两者不等——上游通常有一份**独立于完整 schema 的「可编辑白名单」**，剩下的字段由 CLI 自己管理。凭空多加一个控件不会报错：能填、能保存、能读回，只是写出的键官方编辑器永远不产生，而且同一字段在不同模式下可编辑性还可能不一致（上游 schema 允许与否）。与模式十三（存了没人读）的区别：那个是**存了没有消费者**，这个是**存了但官方根本不接受用户写**。
> 例：#63 ZCode 模型弹窗的「工具调用」「MFJS 工具 Schema」「最大输出参数映射」。上游 `manual-model-config.ts` 明确写着「手动模式只冻结产品明确开放的叶子；新增系统字段默认不属于个人手动配置」，官方 UI 对这三个字段完全不渲染控件。
>
> **对策**：字段集合**以上游的可编辑 schema 为准**，不凭直觉增减。核对步骤见 §4.2.4.1——找到上游的 `manual*` / `editable*` / 官方设置界面渲染组件，逐字段对照。被删掉的字段要**原样保留**（上游自己写的行带着它们），且「键缺失」与「显式 false」语义不同，不要补默认值。
>
> **判据一句话**：「这个字段上游让用户改吗？」——答案不在本仓库代码里，在上游源码里。**没查过上游就加字段，等于猜。**

**模式十九：功能存在，但放错了层级。** 同一个开关可能属于**行**（列表某一项的属性）也可能属于**卡/弹窗**（整条记录）。放错层级不会报错——功能照常工作，只是**用户找不到它**，或者找到后**说不清它作用于谁**。最容易发生在照抄官方功能清单时：只核对了「有没有这个开关」，没核对「它在哪一层」。
> 例：#67 ZCode 的「启用」官方在**模型列表行**的 `Switch` 上（`ProviderFormControls.tsx`），编辑弹窗里没有；我们放进了弹窗，而行级开关当时根本不存在。
>
> **对策**：字段/开关的核对要**两步**——先问「官方有没有」（模式十八），再问「官方放在哪一层」。判据：**这个状态作用于整张卡片，还是某一行？** 整张卡 → 卡片头部；某一行 → 那一行上。
>
> 同源问题还有：**行级状态必须能一眼扫到**。模型行的「启用」开关刻意不做 hover 隐藏——开关报告状态，只能靠 hover 才看见的状态，用户无法扫列表发现。

**模式二十：行高被「每个字段一行」撑开。** 列表行里逐字段换行（一个字段一行、带完整标签），单看一行很规整，但列表是**重复 n 次**的：每多一行就多一倍高度，而扫读时真正需要的往往只是量级（大小、长短），不是精确值。与模式十七同属「能编译、能跑、只是不好用」，但它的代价是**累积的**——第 1 行看不出问题，第 50 行才发现列表长得没法用。
> 例：#68 模型行原本两行，第二行是 `上下文限制: 400,000 | 输出限制: 128,000`。合并后 `(id | 400K)` 一行说完，且 `400K` 比 `400,000` 更容易比较大小。
>
> **对策**：
> 1. **列表行默认一行**。要加第二个字段时先问「能不能并进已有的括号/分隔里」，而不是直接换行。
> 2. **列表里的数字按「可比较」呈现，不按「精确」呈现**（`200K` / `1M`）。精确值属于编辑弹窗和 tooltip——那里用户是来核对数字的。
> 3. **判断是否该有第二行**：这一行的信息，用户是「扫」还是「读」？扫 → 压进第一行；读 → 它可能不该在列表里，而该进详情。
>
> 反过来也成立：**不要为了压成一行而丢掉比较维度**。如果两个限制都需要看，就并进括号（`id \| 200K / 128K`），而不是删掉一个。

**模式二十一：页面区块是「本 CLI 特有」而不是「本 CLI 需要」。** 每个 CLI 都有自己独有的配置面，于是页面很容易长成一堆专属区块的堆叠——单看每一块都有理由（「Native 确实有 agents 概念」），但**没有一块被问过「它是不是重复了别处的信息」或「别的 tab 为什么不这么放」**。后果不是功能坏了，而是每个 tab 都长得像不同的产品，用户每换一个 tab 就要重新找一遍入口。
> 例：OmO Native 曾有 6 个区块，其中「生效配置」与页头「预览配置」重复、「更多选项」的内容在 ⋯ 弹窗里已有同一份、「MCP 与 Skills」只是一段指向别处的指引。收敛成标准三区块（供应商 / 全局提示词 / 会话管理）后，与 ZCode 完全同构。
>
> **对策**：新 CLI 的页面**默认只放标准区块**（见 §4.0.2-H 的能力清单），任何额外区块都要先回答两个问题——① **它承载的信息在别处有没有**（页头链接 / ⋯ 弹窗 / 顶层独立页面）？② **别的 tab 为什么不需要它**？两个问题都答不上来就不加。真需要加的，写进模块 `AGENTS.md` 说明理由。
>
> ⚠️ **撤下一个区块的 UI ≠ 删除它的后端。** 页面收敛时最容易顺手把「没人用了」的命令、表、托盘接线一起删掉——但那些往往还有**非页面消费者**（托盘菜单、备份恢复、深链导入）。判断依据是 §12.1 的死封装审计命令：`rg "<apiName>" web/ --glob '!services/**'` 为空**只说明前端没入口**，还要再查后端命令是否被 `tray.rs` / `reapply_applied_runtime.rs` / 备份链路调用。查不清就保留，并在模块 `AGENTS.md` 里写明「无 UI 入口但后端保留」（先例：`oh_my_pi/AGENTS.md` 的 `listOmpAgents`）。

**模式二十二：工具的「保护名单」自己过期了。** 静态分析类工具（i18n key 检查、死代码扫描）靠一份**手工维护的标识符取值表**来展开动态调用。那份表是**另一个白名单**——它会随代码增长而过期，而过期的后果是**工具给出错误结论**：把仍在使用的资源判为「未使用」，接着自动清理就把它删了。比工具不报错更危险，因为它主动提供了错误建议。
> 例（本轮连中两处）：`scripts/i18n-keys.mjs` 的 `DEFAULT_DYNAMIC_IDENTIFIER_VALUES_BY_FILE` 里，`useRootDirectoryConfig.ts` 只登记了 3 个 `translationKeyPrefix`（实际 12 个）→ 9 个模块的整块 `rootPathSource.*` 文案被列为可删。同一张表还不支持**嵌套**占位符（`${a}.${b}Hint`），于是 `envHint`/`shellHint` 永远算未使用。
>
> **对策**：① 每次新增「会拼 key 的共享组件调用方」时，**同一任务内**把取值加进工具的保护表（与 §3.1 的 allowlist 同步是同一件事）；② 执行任何自动清理（`i18n:prune --write`、批量删除）**之前**，先手工抽查几条被判「未使用」的 key 是否真没人用——尤其那些名字里带 `Hint` / `Placeholder` / `Success` 的；③ 补全保护表后**必须重跑 `check`**：它会把一直存在的缺口一次性暴露出来（本轮暴露了 6 个模块缺 `envHint`/`shellHint`、共享 `ModelItem` 缺 3 条开关文案）。**「补白名单」的收益不只是消音，更是把被掩盖的真实缺陷翻出来。**

**模式二十三：抄了「名字像的」而不是「同引擎的」兄弟。** 新 CLI 的字段形状总要找个参照抄，而最容易被选中参照的是**名字最像**的那个（`omo_native` 抄 `oh_my_pi`，都带 `omo`/`pi`），不是**运行时真正同源**的那个。两者的差别在界面上完全看不出来：字段能填、能保存、不报错，只是写出的键引擎不读——**用户的编辑静默丢失，而旧值被覆盖**。
> 例（#76）：OmO Native 与 **Pi** 同用 senpi 引擎、同读 `models.json`；OMP 读的是 `models.yml`，字段形状不同。但思考级别的开关照 OMP 抄了 `showOmpThinking`，写出 OMP 的 `thinking: { efforts, defaultLevel }`——上游 `docs/models.md` 的模型字段表里没有这个键，只有 `thinkingLevelMap`/`defaultThinkingLevel`。
>
> **对策**：
> 1. **先确认「同源」再决定抄谁**。判据不是名字，是**运行时事实**：读同一个文件吗？同一个二进制/引擎吗？（`omo` 与 `pi` 都是 senpi；OMP 是另一个实现。）查法：看两边的常量文件名（`models.json` vs `models.yml`）与上游 docs 路径。
> 2. **字段形状以该引擎自己的文档为准**，不以「兄弟页面的代码」为准——兄弟页面也可能是错的（本次就是两边都错，只是 OMP 那边恰好对）。
> 3. **每个字段问一句「引擎读它吗」**：在上游 `docs/models.md`（或对应 schema）的字段表里找得到吗？找不到的键写进去就是死数据。
> 4. **JSON 字符串 ↔ 文件形状的边界要显式**：共享弹窗交回的是**字符串**（`JSON.stringify(...)`），文件里要的是**数组/对象**。转换点集中到 util（`parseInputTypes` / `stringifyInputTypes` / `parseJsonRecord` / `stringifyRecordField`），别在 handler 里就地赋值。
>
> **一句话**：**字段名对不对，不看代码像不像，看引擎读不读。**

**模式二十四：共享组件只共享了「文案」，把「样式」留给了调用方手抄。** 共享组件暴露一个 `ReactNode` 插槽（`hint` / `footer` / `extra`），文档写「文案由调用方传」——但那个插槽在视觉上是**有样式的**（小字、次要色、左侧竖线）。于是每个调用方都要复制同一段 style 对象，而**漏抄的那一个不会报错**：内容在、位置对、字也能读，只是渲染成了正文大小，看起来像设计如此。
> 例（#80）：claudecode / codex / zcode 逐字复制同一段六属性 style；omo_native 没抄 → 提示块变成一段全黑大字。而 SOP §4.2.1 的示例本身就写着 `hint={<div>…两行提示…</div>}`（裸 div），**照着 SOP 抄就会抄错**。
>
> **对策**：
> 1. **区分「文案」与「样式」**：插槽共享的是**文案**，样式属于组件。把 style 连同常量一起放进组件（`HINT_BLOCK_STYLE`），插槽只接受内容。
> 2. **判据**：如果某个插槽的使用方都要写同一段 style，那它就不该是插槽，而该是组件内部的一层。
> 3. **同步改示例**：SOP / AGENTS.md 里的示例代码要跟着改成「只传内容」——示例是别人抄的对象，示例错等于源头错。

**模式二十五：交互能力是「三件套」，缺一件就表现成坏掉。** 拖拽排序看起来是一个功能，实际由三个独立的部分组成：**把手（渲染）→ context/命令（行为）→ 存储的写入端与读取端（持久化）**。任何一件缺失，用户看到的都是同一句话：「拖不动」或「拖了没用」——而三者的修复位置分别在前端组件、后端命令、后端读取函数里，不查全就会只修一半。
> 例（#81/#82）：omo_native 抄了 ZCode 的把手但没有它的 `DndContext`（缺行为）；就算补上前端，`list_*` 结尾的 `sort_by(key)` 还会把顺序抹掉（缺读取端）。
>
> **对策**（顺序照这个查）：
> 1. **把手只在能拖时渲染**：`draggable: !selectable && !dragDisabled`，并让 `dragDisabled` 覆盖所有「展示顺序 ≠ 存储顺序」的情形（非 `custom` 排序、有搜索词）。
> 2. **能力存在也别急着开**：先确认后端有对应的 reorder 命令；没有就先别渲染把手（一个拖不动的把手比没有把手更糟）。
> 3. **写入点与读取点成对核对**：存哪就读哪。「存了 A、读时又按 B 排序」是这类 bug 最隐蔽的一半——它只在**重载之后**才显形。
> 4. **回归测试落在纯逻辑上**：把重排逻辑从 Tauri 命令里抽成纯函数（`reorder_provider_map`）再断言键序，否则 `State`/`AppHandle` 让这条路径无法直测。

**模式二十六：SOP 自己给出了「过强的规则」，于是照着做反而做错。** 前面的模式都是「SOP 漏了某条」，这一条相反：**SOP 写了一条看起来更严格、更省事的规则，而它是错的**。执行者照着做，产出比不做还差——因为错误被 SOP 的权威性掩盖了，没人会去质疑它。
> 例：§4.0.2-I 原版写「页面默认三个区块，除此外不许有别的」，并让执行者「回答两个问题」来决定要不要加第四块。问题在于那两个问题问的是「**别的 tab 为什么不需要它**」——这是个**诱导否定**的问法，任何区块都能被问倒（「别的 tab 没有这个数据」）。结果是：OmO Native 被收敛到 3 块，而全仓并集显示「其他配置」有 7 个页面、「官方认证渠道」有 5 个页面。用户立刻发现缺块。
>
> **根因有三层，缺一不可**：
> 1. **用「参照 CLI」代替「全部页面」**：参照 CLI（Codex）自己就没有「其他配置」，于是这个区块从未进入候选。
> 2. **把「独特」当成了「多余」**：「只有本 CLI 有」是独特性，「本 CLI 缺」是缺失——两者在页面上长得一模一样，但结论相反。
> 3. **规则用了「默认禁止 + 举证例外」的结构**：这种结构在证据不全时会系统性地偏向「删」。
>
> **对策**：
> 1. **先取并集，再做减法**。判断「该有哪些区块」的输入必须是**全部页面的实际现状**（一条 `grep` 命令就能列出来），不是一两个参照页。命令见 §4.0.2-I。
> 2. **提问要中立**。「别的 tab 为什么不需要它」是诱导否定；改成「**本 CLI 有没有这类数据**」——有数据就加，没数据才不加。前者问的是别人，后者问的是自己。
> 3. **写完规则要反查它拦住了什么**。任何「默认禁止」型的清单，都要拿现有代码验一遍：**按这条规则，现存的哪些东西会被判违规？** 如果答案是「7 个页面都违规」，那违规的是规则，不是代码。
> 4. **用户提出的每一个「缺了 X」，都要先问「别的页面有没有 X」**——如果多数页面都有，那就是 SOP 的区块清单漏了 X，不只是这一个 CLI 漏了。

**模式二十七：区块是「大颗粒」，卡片/列表的「能力集」是小颗粒——后者没人枚举过。** 模式二十六解决的是「这个页面该有哪几块」；但**每一块内部有哪些按钮**，SOP 从头到尾没列过。于是即使区块齐了，卡片仍会缺功能：ZCode 的供应商卡片少了「复制」，OmO 的模型列表少了「批量删除」。这两个都是**别的页面有、本页静默没有**——用户得挨个点开才发现。
> 为什么这类缺失特别难自查：区块缺失是**结构性的**，扫一眼页面就知道少一块；按钮缺失是**局部的**，卡片看起来「是完整的」，只是少一个图标。而且它的信息来源是「别的页面」，不是「本页」——只看本页永远看不出来。
>
> **对策**：
> 1. **写卡片前，先横向 diff 三张已有的同类卡片**（供应商卡片挑 3 个不同风格的，模型列表挑 3 个）。把每个按钮列成一张表，逐项标「要 / 不要 / 为什么不要」。
> 2. **「不要」必须写理由**，且理由要落在数据上，不是感觉上。合格：「内建渠道是只读的，没有可写的落盘目标，故无编辑/删除」。不合格：「暂时不需要」。
> 3. **把清单写进模块 `AGENTS.md`**，下次接入时是「对着清单打勾」，而不是「重新想一遍」。
> 4. **用户报「少了 X」时，先查别的页面有没有 X**——同模式二十六第 4 条。若多数页面都有，就是清单漏了，要补进 SOP，不只补这一个页面。
> 5. **修完还要反向扫一遍：哪些页面也缺 X？** 用户报的是 ZCode 缺复制，扫完发现 OpenClaw 连供应商复制带模型复制**两个都缺**。同一个能力在全仓做一次 `grep` 审计（比"逐页打开看"快得多），比只修被指出的那一处更接近"把这类问题解决掉"。

**模式二十八：迁移做了一半就宣布完成——「新的写对了」不等于「旧的清干净了」。** 把 `"__local__"` 抽成共享常量时，第一轮只改了 9 个「构造桥接记录」的文件，就报告「已统一」。实际还有 33 个文件（判定 `if config_id ==`、托盘过滤、各模块 `const` 别名）写死字面量。**这种半迁移比不迁移更难发现**：`grep` 一次只看到一半结果，而两种写法同时存在时，改一处不影响另一处——下一个人要改 id，看到常量以为改完了，字面量那 33 处纹丝不动。
> **对策**：
> 1. **完成判据是「旧写法残留数为 0」，不是「新写法出现了」**。收尾必须跑一次 `grep -rn '<旧字面量>' <源码根>`，把输出当作 checklist 逐条消灭。
> 2. **注释里的提及也要一起改**——`/// Returns a config with id "__local__"` 这类文档注释会让下一次 grep 命中，制造「还没改完」的假象，也会误导读代码的人。
> 3. **别只改「明显的那一类」**。同一个值在代码里有多种角色：构造、判定、过滤、别名常量。构造点最容易看见（它就在你正在写的函数里），判定与过滤点散在 `tray_support.rs` 这种你不常打开的文件里。
> 4. **模块自己的 const 别名保留名字、值指到共享常量**（`const CODEX_LOCAL_PROVIDER_ID: &str = crate::coding::local_bridge::LOCAL_CONFIG_ID;`）——这样调用方一行不用改，单一来源也建立了。

**模式二十九：照搬了区块的「形」，没照搬它的「筛选」。** 一个区块在参照页面之所以好用，往往不只是因为样式和数据源对，而是因为**它只显示该显示的那几条**。照搬时最容易漏的恰恰是这一层——因为筛选条件通常藏在一句不显眼的代码里（OpenCode 是 `read_auth_channels()`，一行），而样式和数据源是看得见的。
> 例：OmO 的「引擎内建渠道」抄了 OpenCode 的卡片、布局、文案，但**数据源直接用了 `omo --list-models` 的全量**——49 个内建 provider 全列出来，其中 48 个未配置凭据。用户看到一屏用不了的东西，问「应该只有配置了 key 的才展示吧」。
>
> **对策**：
> 1. **照搬一个区块前，先回答三个问题**：数据从哪来（✅ 容易想到）、**显示哪些**（❌ 最常漏）、用户能对它做什么。第二个问题的答案通常是一句过滤，而不是数据源本身。
> 2. **判据是「这个条目对用户有没有意义」**，不是「它在数据里存不存在」。未配置凭据的渠道既不能用也不能改，列出来只有噪音。
> 3. **筛选可能拿不到现成的数据源**。OmO 这里要逐个跑 `omo auth check`（引擎没有批量接口），代价 8 秒——**这种代价要主动告诉用户并让他决定放在哪**（页面加载时 / 展开时 / 先全列后收敛），不要自己闷头选一个。
> 4. **筛选后要区分「还没查完」和「一个都没有」**：慢查询下这两者长得一样，但用户看到「空列表」会以为功能坏了。

**模式三十：「保存失败」的报告，往往其实是「读取端看不见」。** 用户说「我写了 api key，重新打开又变成空的，没保存成功」——第一反应是查写入路径。但**先查磁盘**能省掉一整轮排查：OmO 这个案例里两个文件都在用户操作的那一分钟被写过，`omo auth print-api-key` 也正常返回，数据完全没丢。
> 真正的问题有三层，每层单独看都不像 bug：
> 1. **写和读用的不是同一个字段**。写入端写 `auth.json`、读取端也只查 `auth.json`，但用户的记录是写在 `models.json` 的 `apiKey` 里的（引擎两处都认，且优先 `auth.json`）。
> 2. **弹窗把字段硬编码清空**（`apiKey: ''`），placeholder 也是空的 —— 于是「有一条已存的密钥」和「从来没有过」在界面上**完全同形**。
> 3. **没有「已存过」的信号**。密码框显示空白本身不奇怪，奇怪的是它和「真的没有」无法区分。
>
> **对策**：
> 1. **「保存失败」先做三件事，再动代码**：看文件 mtime 与内容、跑一次引擎自己的读取命令、确认写入端与读取端用的是不是同一个位置。界面为空 ≠ 数据没写进去。
> 2. **一个值有多个合法存储位置时，先定一个权威位置**，然后：写入端统一到它 + **迁移旧值**（本案例：保存时顺手把 `models.json` 的 `apiKey` 移进 `auth.json`），读取端在迁移期**两处都认**。只做一半会出现「同一个 key 两处不一致、引擎取的那个不是用户改的那个」。
> 3. **密钥类字段一律「不回填明文 + 显示已存过」**：placeholder 用 `••••••••`，说明文案换成「已保存密钥；留空表示不改动」。这条对密码框是硬要求——因为它天生就是空白的，没有信号就必然被误读。
> 4. **凭据的界面状态要由后端算**（`hasKey`），不要让前端自己推断存储位置——否则存储位置一变，界面就悄悄错。

**模式三十一：把「安全最佳实践」套到不该套的场景，反而制造 bug。** 「密钥不回填明文、只显示占位符」在给别人看的只读界面上是对的；但在**配置管理器**里，用户看自己的密钥是天经地义的，藏起来只会让他分不清「存了」和「没存」。
> 例：修 OmO 的「api key 显示为空」时，我把弹窗改成 `••••••••` placeholder + 「已保存密钥；留空表示不改动」文案。用户当场否掉：「这个应用就是管理配置的，前端都可以展示所有的秘钥，不要再出现这种看不到回退的情况了」。同仓 Claude Code / Codex / Kimi 的弹窗一直就是回填明文的——**既有约定早就摆在那里，是我没去看**。
>
> **对策**：
> 1. **先问「这个产品的定位是什么」**。ai-toolbox 是配置管理器：它的用户就是配置的主人，展示明文是**功能**不是漏洞。同类判断：给一个 Git 客户端做 diff 视图，不该因为「diff 里有密钥」就把它打码。
> 2. **照抄同仓既有做法，别照抄通用最佳实践**。动手前先看**同类字段在别的页面怎么做**（这里 3 个页面的弹窗都是明文回填）——同仓约定优先于行业惯例。
> 3. **「看不见」本身就是缺陷**。任何让用户「分不清有没有」的设计，在这个项目里按 bug 处理。占位符方案的问题不是不够安全，是**它让两种状态同形**。
> 4. **用户否掉一次，就要写成硬规则**（本项目已写进记忆库 `ai-toolbox-shows-plaintext-credentials.md`），不要下次换个模块再犯一遍。

**模式三十二：同源工具不等于同语法——「名字像」的兄弟会误导你。** OmO Native 与 OMP 都是 `oh-my-*` 家族的、都读一个 `models.json`/`models.yml`，但**凭据的 config value 语法不同**：OmO 支持 `$ENV_VAR` 插值（与 Pi 一致），OMP 只认「`!command` 或整值精确匹配环境变量名」。
> 例：给 OmO 的「获取模型」传了 `configValueMode: "omp"`，于是 `$MY_KEY` 不会被插值、被当字面量发出去。这个错误**不会报错**，只会 401——和「没传 key」长得一模一样，所以排查时先怀疑了后者。
>
> **对策**：
> 1. **选实现要按「引擎」不按「名字」**。同源工具（同一作者、同一命名前缀）常常在细节上分叉，抄之前先确认它读的是不是同一个文件、走的是不是同一套语法。
> 2. **拿引擎自己验证一次语法**，别从文档推断。这里用「把值改成 `$VAR` 再跑一次引擎的读取命令」两分钟就确认了。
> 3. **新增枚举值时，把「为什么不是隔壁那个」写进注释**。`ConfigValueMode::Omo` 的文档注释里明确写了「OmO is not OMP despite both being `oh-my-*` tools」——否则下一个人看到两个几乎一样的模式会想合并它们。
> 4. **401 有两个常见根因，要一起查**：没传凭据 / 凭据语法没解析。先看请求有没有 `Authorization` 头，再看值是不是字面量。

**模式三十三：凭据换存储位置时，只改了「写」，没改「读」的另一半。** 把自定义渠道的 key 从 `auth.json` 挪到 `models.json`（因为写进 `auth.json` 会和内建渠道的凭据混淆）本身是对的，但**读取端如果只查新位置，老用户的旧值就凭空消失**——界面上是「空」，磁盘上还在，引擎也还在用。用户会报「我的 key 没了」，而排查会发现两份文件里一份有、一份没有。
> 更麻烦的是**优先级与写入位置的耦合**：引擎的优先级是「`auth.json` 优先于 `models.json`」。所以「写新位置」必须**同时删掉旧位置的同键条目**，否则用户改的新值永远不生效（旧值赢）；而「读新位置为主」又必须**保留读旧位置兜底**，否则没迁移过的用户看不到值。
>
> **对策**：
> 1. **换存储位置是一次三端改动**：写入端（写到哪）、读取端（从哪读，且**新旧都读**）、清理端（旧位置的同键条目要不要删）。缺任何一端都有用户可见的症状。
> 2. **兜底读取要保留至少一个大版本**，并在模块 `AGENTS.md` 写明「主位置 / 兜底位置 / 何时可以撤掉兜底」。
> 3. **优先级由引擎决定，不由界面决定**。写之前先用引擎自己的读取命令确认「两处都有值时它取哪个」，再决定要不要顺手删旧的。
> 4. **删旧条目要限定「同键」**：`auth.json` 里还有内建渠道的条目，整文件覆盖会一起清掉。

**模式三十四：提示文案没跟着功能更新，于是它在骗用户。** 一个区块的说明文字写着「本模块只做展示，不提供编辑能力」——这句在写它的时候是对的，后来这个区块加了「在应用内编辑」的入口，文案却没动。**没有任何检查会发现它**：i18n key 存在、组件正常渲染、功能完全可用，只是**用户读了这句话就不会去找那个按钮**。同类还有按钮 tooltip（「点击打开配置文件」——实际打开的是应用内编辑弹窗，不是系统文件管理器）。
>
> **对策**：
> 1. **给区块加能力时，同一次改动里改文案**。把「说明文字 / tooltip / 空态措辞」列进这类改动的 checklist——它们是**对功能的断言**，功能变了断言就过期了。
> 2. **文案里的否定词是重点排查对象**：「只读」「不可编辑」「不支持」「不能」。对全仓这些词做一次 `grep`，逐个核对当前是否仍然成立（本轮修了两处：OpenCode 与 OmO 的「官方/内建渠道」说明）。
> 3. **区分「模型列表只读」与「渠道凭据可编辑」**——同一个区块里两者可以同时成立。含糊成一句「本模块只读」会连可编辑的部分一起否掉。措辞要落到**具体对象**上（「模型列表只读；渠道凭据可在应用内编辑」）。
> 4. **`grep` 要覆盖两种语言**：只改 zh-CN 会留下 en-US 的旧断言。

**模式三十五：撤了编辑界面，却把它的「空态提示」和「托盘入口」留在原地。** 页面收敛时把一个区块的 UI 拿掉了，但那个区块**独有的数据源**（这里是 `omo.jsonc` 的 `[native]` 块）仍然被两处引用：预览里显示它、托盘里挂它的切换菜单。撤 UI 时只检查了「页面上还看得见吗」，没检查「还有谁在引用这份数据」——于是用户看到一句永远消不掉的提示（「还没有 `[native]` 块」）和一个永远写着「暂无配置」的菜单组，而**页面上根本没有任何入口能产生这份数据**。
> 例（#83/#84）：OmO 页面撤掉「Agent·Category 方案」编辑界面后，`[native]` 块只剩托盘能写；预览里那一栏对它显示「本页尚未应用过任何方案」，托盘的方案组显示「暂无配置」。用户两次追问「我本地已经有2个渠道了啊，这个提示是啥意思？新用户进来也会困惑吧」——他看到的「2 个渠道」是 provider（`models.json`），与 `[native]` 块无关，但界面上没有任何东西解释这个区别。
>
> **对策**：
> 1. **撤 UI 的同一动作里，列出该能力的全部引用点**：预览 / 托盘 / 备份 / reapply / 深链。跑 `rg "<该区块独有的数据键名>"` 而不是只搜组件名。
> 2. **判据是「用户能不能产生这份数据」**，不是「数据是否存在」。没有产生入口 → 它的展示位与切换位都该一起撤（或改成明确的只读说明），否则就是**永久空态**。
> 3. **后端保留 ≠ 前端保留**。按 §12.1 的惯例，后端命令、表、reapply 逻辑照旧保留（备份恢复要用），但**展示层要撤干净**，并在模块 `AGENTS.md` 里写明「无 UI 入口但后端保留」。
> 4. **区分同名不同物的数据源**：用户说「我有 2 个配置」时，先确认他说的是哪一份（provider 还是方案），再解释为什么界面上看不到——**这个解释本身就该写进界面**，而不是只在对话里说一次。

**模式三十六：预览只显示「本页拥有的那一层」，用户以为文件里只有这些。** 一个共享文件（`omo.jsonc` 同时被 OpenCode 插件版和本页写）在预览里只渲染本页负责的那部分（先是 `[native]` 块，后是「共享键」），用户看不到文件的另一半。这类截断**不会报错**，但会让人对「文件里到底有什么」形成错误认知，进而做出错误判断（「我配置怎么没了」）。
> 例（#85）：两轮修正——先是从 `[native]` 块改成共享键（仍是片段），用户直接要求「直接展示完整的 json」。
>
> **对策**：
> 1. **预览文件 = 显示文件的完整内容**。这是「预览」这个词的字面承诺；显示片段要改名叫「XX 视图」并说明范围。
> 2. **共享文件尤其如此**：它被多个模块写，任何一方只显示自己那层，都会让用户误判「另一层不存在」。
> 3. **实现上优先读原文**（`read_file_optional(&config_path)`）而不是 `JSON.stringify(解析后的对象)`——原文保留注释与格式，且不会因为解析器丢了未知字段而少显示内容。注意给编辑器选对语言（`jsonc` 而不是 `json`，否则带注释的文件会满屏报错）。
> 4. **空态文案要跟着改**：显示片段时写的「还没有 XX 块」在改成显示全文后就是错的（文件明明有内容）。

**模式三十七：`await` 一个「刷新」把已完成的保存拖成了长时间 loading。** 保存成功后调刷新回调是常规做法，但刷新**可能很慢**（要重查依赖凭据的列表、跑外部命令）。写成 `await onSave(); await onSaved(); close();` 时，用户看到的是保存按钮一直转圈，**而数据其实早就写完了**——他会以为保存失败或卡住。
> 例（#86）：`AuthConfigModal` 保存后 `await onSaved()`，而 OmO 的 `onSaved` 要重跑 `omo auth check`（48 个渠道约 8 秒）。用户报告「提示操作成功之后一直不消失在 loading，很长时间之后才会消失掉」。
>
> **对策**：
> 1. **关界面的时机是「写入成功」，不是「刷新完成」**。先 `close()`，再把刷新丢进后台：`void Promise.resolve(onSaved()).catch(...)`。
> 2. **刷新失败不能回滚已保存的内容**，但也不能静默——`console.error` 留痕，必要时给一条 toast。
> 3. **判据**：这个 `await` 里的操作，用户需要等它才能继续操作吗？不需要 → 不该 await。
> 4. **共享组件里更要小心**：同一个 `onSaved` 契约，快的调用方（读一个 JSON）和慢的调用方（跑 48 次外部命令）差三个数量级，**慢的那一个会把共享组件的体验定义成「很慢」**。

**模式三十八：测试夹具里照抄了自己机器上的真实信息。** 写测试时从本机跑通的实际数据里复制样本最省事——真实路径、真实渠道名、真实模型名。这些都**不是**隐私意义上的敏感数据，但会：① 把开发者的机器与账号暴露在公开仓库里；② 让测试与某个特定环境耦合；③ 用户看到自己的私有命名出现在代码里会直接要求清理。
> 例（#87）：`C:\Users\<真实用户名>\.claude\plans\...`、`D:\GitHub\<真实仓库名>`、真实 provider key `axonhub-chat`、真实网关实例名 `AxonHub-6 Astra`，以及**输入框的 placeholder**（`placeholder="axonhub-chat"`——最容易漏，因为它不在测试文件里）。
>
> **对策**：
> 1. **判据**：这个名字/路径**只有我这台机器上有**吗？是 → 换掉。判据不是「敏不敏感」。
> 2. **但别误伤产品数据**：先查 `tauri/resources/*.json`（打包进产品的预设清单）。`deepseek-v4-flash`、`glm-5.2` 这类在里面 → 是公开产品数据，可以用。同理，注释里引用**公开开源项目**的 commit（`(AxonHub 7444f537)` 指 `looplj/axonhub`）保留——先搜一下确认那个项目是公开的。
> 3. **替换要保留测试语义**：路径要有盘符与反斜杠（Windows 路径解析测试）、要有空格（引号包裹测试）。换成 `C:\Users\tester\...` / `D:\GitHub\sample-workspace` 这类通用名。
> 4. **自查命令**（改动前后各跑一次，逐条清零）：
>    ```bash
>    grep -rn "<本机用户名>" --include=*.rs --include=*.ts --include=*.tsx tauri/src web
>    grep -rn "<真实工作目录名>\|<真实渠道 key>" --include=*.rs --include=*.ts --include=*.tsx tauri/src web
>    ```
>    **placeholder 与注释最容易漏**——它们不在测试文件里，但同样是「照抄自己机器」的产物。
> 5. **这条是硬规则**：用户 2026-10-07 明确要求「以后都不可以在测试用例里面写我的真实信息」。

**模式三十九：同一个区块在不同页面用了不同的「外壳」——一个可折叠，一个不可折叠。** 区块的内容与数据源都对，只是外层容器不同：这边用 `Collapse`（带展开箭头、默认折叠），参照页面用固定卡片（`modelCard` + `Title level={5}`）。**功能完全正常，所以只有并排看才发现**。它的实际代价是：用户在新 CLI 上要**多点一次**才能看到同样的信息，而且每个 tab 的「哪块默认展开」规则都不一样。
> 例（#88）：OmO 的「模型设置」用了 `Collapse`，而 Pi 与 OpenCode 的同名区块都是不可折叠的卡片。用户要求「改成不可以折叠，和 pi 还有 Opencode 一样」。
>
> **对策**：
> 1. **「模型设置」这类「当前值」区块一律不可折叠**：它只有三个下拉、一行就能放下，折叠省不下版面，只多一次点击。**判据**：这块内容用户是不是每次进页面都要看一眼？是 → 不折叠。
> 2. **照抄参照页面的外壳，不只照抄内容**：搬一个区块时把它的容器（卡片 / Collapse / 内边距 / 标题层级）一起搬。`modelCard` 那三个 class 在各页面的 `.module.less` 里是逐字相同的，可以直接复制一份。
> 3. **横向核对**：把「哪些区块可折叠」列成表，跨页面比对。同类区块的折叠行为应当一致——不一致的地方就是这一条要找的。
> 4. **折叠态默认值也要一致**：`Collapse` 默认展开还是收起，同类型区块应当相同。

**模式四十：同一组菜单项在各区块的**排列顺序**没有统一规则，逐块写下来就漂移了。** `tray.rs` 里每个 CLI 区块都是一串 `if let Some(submenu) = ... { menu.append(...) }`，顺序靠**写代码时的直觉**决定。结果同一个「全局提示词」在 8 个区块里有 3 种位置（第 1 / 第 2 / 第 3），用户扫菜单时找不到规律。
> 例（#89）：Claude Code / Codex / Gemini CLI / Antigravity / Grok / Kimi 把「全局提示词」放在最前，而 Pi / OMP / OmO / Hermes / Dsh / ZCode 放在主选择项之后。用户要求「全部统一放在第二个」。
>
> **根因**：没有一条**可陈述的排序规则**。一旦规则是「凭感觉」，新增 CLI 时就没有可对照的答案，必然再漂移一次。
>
> **对策**：
> 1. **写下规则**：**主选择项在前，全局提示词紧随其后**。「主选择项」= 该区块用户最常改的那个（默认模型 / 供应商 / 主+小模型组）。Claude / Codex 这类只有供应商的，「供应商 → 提示词」；OpenCode 的主选择项是「主模型 + 小模型」一组，「两个模型 → 提示词」。
> 2. **规则要能回答边界情况**：某区块有两个主项时怎么办（→ 主项都排完再排提示词）；某区块没有提示词时怎么办（→ 不显示，不占位）。
> 3. **核对方式**：把 `tray.rs` 里所有区块的 append 顺序抽出来并排打印（一条正则就够），逐行对规则。**这个检查只能靠并排看**——单看一个区块永远觉得「挺合理」。
> 4. **同源提醒**：与模式三十九（区块外壳）是同一类问题——**跨页面的「形态」要一致，不只是「功能」要一致**。新增 CLI 时，除了核对「有没有这一项」，还要核对「它在第几位」。

**模式四十一：托盘的空态与页面的空态不一致，用户以为功能坏了。** 托盘和页面常常调**同一个**后端列表命令，但托盘额外过滤掉「桥接记录」（`__local__`——磁盘上现成内容生成的只读态）。于是当数据库里没有真实记录、只有磁盘文件时：**页面显示一条「来自本地 X 文件」，托盘一片空白**。用户看到的是「页面上明明有，快捷菜单里却没有」。
> 例（#90）：OmO 的 `omo_native_prompt_config` 表 0 条，但 `~/.omo/agent/AGENTS.md` 存在 → 页面有「default（来自本地）」，托盘不显示「全局提示词」整项。
>
> **判断这是不是 bug**：先看**这条记录能不能被应用**。`__local__` 是可应用的（`apply_*_prompt_config_internal` 有专门分支），所以过滤掉它**是有意为之还是遗漏**要问清楚——两种答案都合理（过滤：它点了等于把原文重写一遍，看起来像没反应；保留：用户至少能看到「当前有提示词」）。**不要自己拍板**，这是产品决策。
> 例中用户的答复是「不改，这是预期行为」——但**这个结论要写进 SOP**，否则下一个人会当成 bug 再修一遍。
>
> **对策**：
> 1. **托盘的过滤条件要能说出理由**，并写进模块 `AGENTS.md`。没有理由的过滤就是遗漏。
> 2. **排查这类「托盘为空」时，先数数据库条数**（一条 SQL），再决定是代码问题还是数据问题——**先查数据能省一整轮代码排查**。
> 3. **页面与托盘的空态措辞要能对上**：页面说「来自本地文件」，托盘就不该完全没有这一项——要么都显示，要么托盘给出「暂无预设」的占位（现在用的是 `no_config` 文案，但它只在整块为空时才出现，容易与「这一项没有」混淆）。

**模式四十二：同步链路的「清单」与「实现」各自独立，新增 CLI 时只补了清单。** WSL/SSH 同步在代码里是**一串并列的 match/数组**，每个都要单独加一项。最容易发生的是：加了文件映射（`default_mappings` + 版本回填），就以为接入完了——而 MCP 同步还额外要求「白名单 + 转换 arm」两处。**漏掉的后果是静默的**：文件同步正常，只有 MCP 在 WSL/SSH 里起不来，用户看到的是「server 报错」，很难联想到是同步漏了转换。
> 例（#91）：OmO Native 的 5 条文件映射齐全（V20/V22 版本回填也在），但 MCP 的 4 个点全漏——`omo-native-mcp` 不在 WSL/SSH 的白名单里，也不在两个 `strip_cmd_c` 的 match 里。MCP 页能写 `~/.omo/agent/mcp.json`，同步过去却带着 Windows 的 `cmd /c`。
>
> **根因**：`default_mappings()` 是**一份数据**（看得见、容易想到），而 MCP 的 4 个点是**散在两个文件里的逻辑分支**（看不见，靠记得）。补数据时不会提醒你还漏了逻辑。
>
> **对策**：
> 1. **接入点清单要按「能力」分组，不按「文件」分组**——SOP §6.2/§6.3 就是这么分的，照着逐组打勾，别跳。
> 2. **每组都要有自查命令**，跑一条命令看命中数（见 §6.3 的 `grep` 与预期行数）。**「我觉得加了」不算，命令输出才算。**
> 3. **模块 key ≠ 映射 id**。ZCode 的 MCP 文件是 `zcode-cli-config`（MCP 与 CLI 配置同文件），OmO/Pi 是 `<tool>-mcp`。查 id 要去 `default_mappings()` 里按 `module` 字段找，**别按名字猜**。
> 4. **复查同类模块**：发现一处漏，立刻用同样的 `grep` 扫其余 CLI——同一批接入的模块往往漏在同一处（本次复查同时确认了 ZCode 的 4 个点都在）。

**模式四十三：审查「有没有问题」时，先跑清单，别凭印象。** 用户问「检查 X 和 Y 的同步/备份逻辑有没有问题」时，直接读代码找 bug 会**漏掉整类接入点**——因为你不知道要找什么。正确顺序是：**先把 SOP 里该能力的接入点清单抄出来 → 逐条跑 `grep` 核对 → 命中数对不上就是缺陷**。
> 例（#91）：OmO 的 MCP 缺失就是这样找到的——不是读代码读出来的，是拿 §6.3 的 4 个点逐个 `grep`，发现 OmO 一个都不命中。
>
> **对策**：
> 1. **清单驱动，不凭印象**。SOP 里已有的接入点表就是审查脚本，逐条执行。
> 2. **命中数写下来**。每条检查都记「预期 N 处 / 实际 M 处」，M < N 就是缺陷，M = N 才算过。
> 3. **顺带验证同类模块**（同模式四十二第 4 条）：本次顺带确认了 ZCode 完整、OmO 缺 4 处。
> 4. **把新发现的检查点补回 SOP**——审查过程本身会暴露清单的缺口（如「映射 id 未必叫 `<tool>-mcp`」这条）。

**模式四十四：打包段写了、恢复分支忘了——这是备份链路里唯一「数据真丢」的漏法。** 其余静默失效最多让功能不生效，这一种是：备份包里**完整装着**用户的配置，恢复时 `else if` 链走到底、entry 被丢弃，界面报「恢复成功」。用户换机后打开 CLI，配置全空。
> 例（#91）：`omo_native` 是当时 15 个工具里**唯一**「在 `write_external_configs_to_backup_zip` 里有打包段、在 `restore.rs` 里没有解压分支」的模块。它的 5 个引擎文件与 `root-dir.txt` 都进了包，恢复时全部静默丢弃。
>
> **根因**：打包与恢复是**两个方向相反的函数**，写打包段时脑子在「往里放」，不会自然想到「另一头得取出来」。而且两者在不同文件、行号差几千行，review 时看不到一起。
>
> **对策**：
> 1. **用集合对账，不靠通读**。两侧各 `grep` 出工具名集合，`diff` 一下（§8.3 第 3 条有命令）。**两侧的集合必须完全相等**。
> 2. **加源码扫描型回归测试**（已加，见 §8.3 第 3 条）。这类「A 文件里有一份清单、B 文件里必须有对应处理」的不变量，**测试可以直接读自己的源码来断言**——比人肉 review 可靠，也比「记得检查」可靠。
> 3. **恢复分支里那三行样板别省**：跳过 `root-dir.txt`/目录/空路径、`should_filter_external_config_entry`、`resolve_external_config_restore_output_path`（安全 helper）。漏第 1 条会在 CLI 数据根目录留垃圾文件，漏第 3 条是路径穿越风险。

**模式四十五：「白名单 + 兜底」的组合里，兜底看起来像正常工作，所以漏项永远不报错。** 同步/解析链路大量使用「命中白名单 → 走精确逻辑；否则 → 走默认值」的写法。漏加一个 key 时，代码走的是**设计好的正常分支**（返回默认目录、用通用格式、跳过转换），返回 `Ok`、日志无 warning、界面显示成功。**没有任何信号告诉你「你本该命中却没命中」。**
> 例（#91）：SSH Skills 的 `SKILLS_TARGETS_FROM_RUNTIME_LOCATION` 漏了 `zcode` / `omo_native`。两者在 `BUILTIN_TOOLS` 里有 `relative_skills_dir`，所以 `get_all_skill_tool_keys()` 会遍历到它们、会去同步、会写文件——只是**写到远端默认目录**而不是解析出的真实目录。同步「成功」。
>
> **判别方法**：看到一个 `match key { <白名单> => <精确逻辑>, _ => <默认值> }`，就问一句：**「这个 `_` 分支在正常情况下会返回一个看起来合理的结果吗？」** 会 → 这就是静默失效点，必须有一条清单或测试盯着它。
>
> **对策**：**优先把白名单反过来**（让共享 resolver 处理未知 key，调用侧不维护副本列表，见 §7.2 的历史坑），而不是补一行 key。反过来之后，新增 CLI 不需要改这里，问题从根上消失。只有在「确实存在真例外」时才保留白名单，并且**必须配一条断言对齐的测试**（如 `runtime_location_skills_tools_are_all_covered`）。

**模式四十六：硬规则只写在文档里，没有任何机械守卫，于是它被绕过且无人发现。** Hard Rule 14 写着「不得再新建 per-CLI 卡片布局」，三种样式也确实建好了——但其中两种**从建立起就是零消费方**，而它们各自最典型的对应 CLI 仍在持 776 / 1134 行的 bespoke 卡片。规则没被违反过（没人新建布局），只是**没人执行迁移**；而「迁移到哪了」只存在于一句文档描述里，写错了没有任何东西会报错。
> 例（#96）：`ClaudeStyleCard` / `CodexStyleCard` 零消费方。
>
> **判别方法**：一条硬规则如果**没有对应的检查脚本或测试**，就问「我怎么知道现在还有多少处没遵守？」——答案若是「读文档」或「grep 一下看看」，这条规则迟早会漂移。共享组件的「新东西已经建好」和「旧东西已经换掉」是两件事，规则通常只写了前者。
>
> **对策**：把规则固化成**棘轮（ratchet）**——脚本列出尚未迁移的文件，断言「不在名单里的文件不得违规」+「名单里的文件不得已经合规」。这样名单**只减不增**，且迁完忘了删条目也会报错。参考 `scripts/verify-provider-card-layout.mjs`（另见 `scripts/verify-form-modal-layout.mjs` 的同类做法）。**新建共享组件时同步建这条守卫**，不要等下次审计。

**模式四十七：迁移时比对的是「长得像不像」，而不是「用的 key 是不是同一个」。** 页面从一个内联实现迁到共享组件时，最自然的做法是把原来的 `t('<tool>.xxx')` 原样抄过去——尤其当共享组件的文案 key 在中文下与 per-CLI key **逐字相同**时，界面上完全看不出区别。后果是：英文出现两套措辞（`Documentation` vs `Official Docs`），per-CLI 的那批 key 永远无法 prune，而「这个 key 到底该不该存在」这个问题再也没人回答。
> 例（#98）：Claude Code 页迁 `CodingPageHeader` 时，`claudecode.viewDocs` / `configPath` / `customizeConfigDir` 三处中文与 `common.*` 全同、英文不同。
>
> **判别方法**：迁移前把待迁的 key 与目标组件的 key **逐条比对中英文**，不要只比中文。
> ```bash
> node -e "
> const zh = require('./web/i18n/locales/zh-CN.json');
> const en = require('./web/i18n/locales/en-US.json');
> const get = (d, k) => k.split('.').reduce((a, p) => a?.[p], d);
> for (const [from, to] of [['claudecode.viewDocs', 'common.viewDocs']]) {
>   console.log(from, 'zh:', get(zh, from) === get(zh, to), 'en:', get(en, from) === get(en, to),
>     '\n  ', get(en, from), '\n  ', get(en, to));
> }
> "
> ```
>
> **对策**：中文相同 → 一律取共享 key，并把英文措辞变化**写进提交说明**（这是有意的统一，不是回归）；中文不同 → 说明确有差异，先判断该差异是否真实，再决定统一还是加插值 prop。**永远不要为了保住旧措辞给共享组件加覆盖 prop**——那正是 §4.1 记录的、已被删除的 5 个 prop 的老路。

**模式四十八：迁移「布局」时只对照了 props，没对照原组件的 `style`，于是状态标记静默消失。** 把 bespoke 组件换成共享组件时，注意力全在「传哪些 prop」上；而原组件里**由数据推导出的内联样式**（边框色、背景、透明度、间距）不属于任何 prop，一眼看去像是「渲染细节」，很容易整块丢掉。丢掉的后果通常不是崩溃，而是**某个状态看不见了**——卡片还在、内容还对，只是分不出哪个是当前生效的。
> 例（#99）：四张卡片各自实现的 `cardBorderColor` / `cardBackground`（批量选中 > 网关 P0 > 已应用）没有被 `CardShell` 继承。
>
> **判别方法**：迁移一个组件时，把原文件里的 `style={{` / `className=` 全部列出来，逐条问「这条对应目标组件的哪个渲染点？没有对应 → 我要搬到哪？」。**props 清单对照不够**——状态类样式根本不经过 props。
>
> **对策**：**状态标记属于外壳**。凡是「由数据推导、影响整块外观」的样式，一并搬进共享外壳（本轮是 `CardShell` 的 `accent`），映射层只负责算出这个语义值。留在映射层会导致每个 CLI 重新抄一遍优先级，正是这次重构要消除的分叉。

**模式四十九：数据等价 ≠ 渲染等价——把「一对」拆成「两个」条目。** 一个列表型 prop（`metaEntries`、toolbar 项、菜单项…）里，属于同一视觉单元的片段必须留在**同一个条目**里。拆成两个条目在数据上完全说得通（一个标签、一个值），在渲染上却会被容器自己的 `gap` / `alignItems` 分开——于是出现「标签与值之间空一格」「两者基线不齐」这类**用户一眼就看出、代码却毫无异常**的问题。把标签放进条目的可选字段（`entry.label`），让「它们是一对」这件事编码在类型里。
> 例（#100）：Claude Code 卡片第二行 `Haiku:` 与模型名被拆成两项，`gap: 16px` 插进中间。
>
> **判别方法**：往列表型 prop 里塞数据前，问「这两个片段**能不能分开换行**？」不能 → 它们是一个条目。
>
> **对策**：让条目自己承载它的标签（`{ kind, label?, value }`），而不是让调用方靠「相邻两项」隐式表达配对。相邻关系是渲染层看不见的约定。

**模式五十：把 CLI 专属的样式规则留在 CLI 侧，迁移时随文件一起丢掉。** 一个组件需要的样式，如果写在**调用方**的样式表里（`.codex-model-list-collapse { … }`），那么它和调用方是**隐式耦合**的：迁移、重写、换文件名时它不会跟着走，也不会报错——只是样式失效。更糟的是这类失效**往往只在某个状态下可见**（本例：卡片有 accent 时才看得出白条），所以能潜伏很久。
> 例（#101）：codex 与 grok 各自 `.less` 里逐字重复的 Collapse 透明规则，迁移 codex 卡片时漏 import。
>
> **判别方法**：迁移一个组件时，除了它的 `.tsx`，还要问「**这个组件的样式有没有一部分在别的文件里？**」——按它渲染出的 class 名（`*-collapse`、`*-section`）反查 `.less` / `.css`。
> ```bash
> # 该组件渲染的 class 名，在哪些样式表里被定义？（跨文件 = 隐式耦合）
> grep -rn "<component-class-prefix>" web --include=*.less --include=*.css
> ```
>
> **对策**：**样式跟着组件走**。如果这条规则描述的是「这个组件在自己的容器里该怎么显示」（而不是「这个 CLI 想让它长什么样」），就把它搬进组件的 `*.module.less`，由组件的 prop 触发（如 `transparentRows`）。留在 CLI 侧只会让下一个迁移的人再丢一次。

**模式五十一（补充：图标也会被「顺手统一」掉）。** 同一类规则的另一个入口：迁移时不是丢图标，而是**把图标换成语义相近的另一个**——因为同一排的其他按钮都用那个图标，看着「不一致」就顺手改了。功能不受影响，测试全绿，只是这个按钮从此长得像它的邻居而不像它自己。
> 例（#110）：Kimi 的网关代理按钮从 `ApiOutlined` 变成 `CheckOutlined`。
>
> **判别方法**：对照图标时**逐个按钮比**，不要只比「用了几个图标」——数量相同、位置相同，换掉一个也看不出来。`git show <迁移前>:<文件> | grep -n "icon={<"` 与迁移后逐行对齐。
>
> **对策**：**图标是按钮身份的一部分**，和文案同级。迁移的默认姿势是原样复刻；想统一图标要单独提出来当一次显式改动。

**模式五十一：菜单被「简化」成了等价的数据，丢掉了它的结构。** 把一个 `MenuProps['items']` 菜单重写成更「干净」的 `{ key, label }[]`，类型检查完全通过、菜单也照样弹出、每一项也都点得动——但**图标没了、分隔线没了、某项的副标题退回了通用文案**。菜单项在数据上「等价」，在用户眼里是另一个产品；而这些装饰性细节**没有任何测试会覆盖**。
> 例（#102）：Claude Code / Codex 卡片迁移后，「更多」菜单丢了全部图标、删除前的分隔线，启用开关的副标题从 `配置已启用` 退化成 `已启用`。用户逐项对照后指出「不要丢东西了」。
>
> **判别方法**：改写一个 UI 元素前，先问「**它现在长什么样**」而不只是「它现在有哪些项」。把原实现整个读一遍（`git show <迁移前>:<文件>`），列出**每项的 key + icon + 分隔线 + 文案来源**，逐个确认新实现里有对应物。
>
> **对策**：把菜单的**呈现契约写进组件文档**（见 `providerCardVariants/AGENTS.md` 的「『更多』菜单是契约，不是装饰」），并让共享组件持有菜单结构、调用方只提供 handler 与文案——这样下一个迁移的人**没有机会**简化它。

**模式五十二：迁移顺手「提升」了某个动作的位置，改变了它的可见性。** 共享组件有个显眼的 `primaryAction` 插槽，迁移时就把原来藏在「更多」菜单里的「编辑」提了上去——因为「编辑是常用动作」。功能没丢、点击也有效，只是**卡片上多了一个按钮、菜单里少了一项**：用户按肌肉记忆去菜单里找，找不到。
> 例（#103）：Claude Code / Codex 卡片的「编辑」被从「更多」提到头部；用户问「编辑按钮之前是不是放在更多选项里的啊，现在跑外面来了」。
>
> **判别方法**：迁移一个组件时，对每个动作问一句「**它在原实现里是主操作还是次级操作？**」——共享组件提供了更显眼的插槽，不代表原动作该升级。**位置是设计决定，不是实现细节**。
>
> **对策**：迁移的默认姿势是**逐项原位复刻**；想调整位置就单独提出来当一次显式改动（说明理由、让用户确认），不要混在「迁移」这一次改动里。迁移的验收标准是「**和迁移前一模一样**」，不是「比迁移前更合理」。

**模式五十三：把「传值回调」收窄成「无参回调」，调用方把新值算反了。** 原组件用 antd 的 `onChange={handler}`，antd 把**新值**传进来；迁移到共享组件时把契约简化成 `() => void`（「你帮我翻一下」），于是调用方必须自己从当前状态算出新值。这一步不报错、UI 也正常——只是**算出来的新值等于当前值**，开关变成一个装饰品。这类 bug 的隐蔽之处在于：**回调确实被调用了**，只是参数错了。
> 例（#104）：`onToggleDisabled` 在 Claude Code / Codex 卡片上点了没反应（用户一点就发现）。
>
> **判别方法**：看到一个无参的「请执行 X」回调，就问「**它怎么知道该执行成什么样？**」——若答案是「调用方从当前状态推算」，那么推算方向就是唯一的正确性来源，而它没有任何类型保护。
>
> **对策**：**共享组件透传值，不要让调用方重算**。`onChange={onToggleDisabled}` + `(enabled: boolean) => void`，与 antd 的契约一致——映射层于是变成纯透传，没有可算错的地方。凡是「有状态、可双向」的交互（开关、复选框、步进器），契约里都要带**新值**，不要只带 id。

**模式五十四：把 prop 放在「最早那批消费方都恰好有」的容器下，后来者读不到。** `draggable` / `sortableId` 描述的是**卡片自己**能不能拖，却定义在 `ProviderCardModels`（模型区）里——因为最初的消费方（ZCode / OmO）都带模型列表，作者顺手就放进去了。Claude 式卡片没有模型区，迁移后**整个拖拽把手消失**：prop 从没被读到，类型检查也不会提醒（调用方根本没传它）。
> 例（#105）：Claude Code / Codex 卡片迁移后拖不动。
>
> **判别方法**：给共享 props 加字段时问「**它描述的是哪一层？**」——卡片级（拖拽、选中、禁用、高亮）归 `providerState`；列表级（行、工具栏、批量删除）归 `modelSection`。判断标准是「**没有模型列表的样式还需不需要它**」：需要 → 放卡片级。
>
> **对策**：props 的分组按**语义层级**，不按「当初谁在用」。同一个 interface 里的字段**不代表所有样式都会读**——`ProviderCardVariantProps` 是三个样式共用的扁平结构，每个样式只读自己那部分，**加字段时必须在每个样式的渲染点指认一次**（同 13.1 模式二 / #70）。

**模式五十五：共享外壳缺一个位置，调用方就只能整块复制。** 迁移一个页面到共享外壳时，最难发现的不是「组件能力不够」，而是「**外壳没有地方放我的东西**」：工具栏、标题行、列表底部各有各的插槽，少一个，调用方就只有两条路——把整段外壳抄回来（分叉又回来了），或者把这个入口删掉（漏功能）。而「至少两个消费方才加 prop」的门槛会让人以为单消费方就不该加，于是选了第三条路：**保持 bespoke**，迁移半途而废。
> 例（#106）：`ProviderListSection` 的工具栏没有「额外按钮」位置，Kimi 的「官方账号登录」按钮无处安放。
>
> **判别方法**：给外壳类组件（`ProviderListSection` / `CodingPageHeader` / `ModelListSection`）加插槽前，先问「**去掉这个插槽，调用方要不要复制整段外壳？**」要 → 这是外壳完整性，不是新能力。
>
> **对策**：外壳的**插槽完备性**不适用「至少两个消费方」的门槛。判据是「**有没有一个位置能放下调用方的第 N 个动作**」，不是「有多少人要用这个位置」。插槽本身不产生行为，它只是让已有行为有地方待着。

**模式五十六：两个「显示内容」的插槽长得像、语义相反，选错就静默丢内容。** 共享外壳常有一对插槽：一个「列表为空时也要显示」（`alwaysVisible`），一个「无条件显示」（`footer`）。名字都像「附加内容」，文档也都写着「渲染在列表附近」——选错不会报错，只会**在某一种状态下内容整块消失**，而那种状态（列表非空）恰恰是日常状态，于是问题看起来像「内容不见了」而不是「插槽选错了」。
> 例（#107）：Kimi 的官方账号列表放进 `alwaysVisible`，有供应商时整块消失。
>
> **判别方法**：把两种状态各问一遍——「**列表为空时它要显示吗？**」「**列表非空时它要显示吗？**」两个都「要」→ `footer`；只有前者 → `alwaysVisible`。
>
> **对策**：往共享外壳塞内容前，**把插槽的渲染条件读一遍**（不是读名字）。插槽的名字描述的是位置，渲染条件描述的是语义——后者才决定内容会不会出现。

**模式五十七：「固定契约」的组件遇到调用方的额外项，规则没说该去哪。** 把菜单/工具栏固定成契约是对的（防止各 CLI 长得不一样），但规则只写了「**必须长这样**」，没写「**原来多出来的那一项该去哪**」。迁移者面对一个无处安放的菜单项，只能在「删掉它」（漏功能）和「破例加一项」（破坏契约）之间选。
> 例（#108）：Kimi 把「连通性测试」放在「更多」菜单里，而共享菜单固定为 启用→编辑→复制→分享→分隔线→删除。
>
> **判别方法**：写「固定契约」这类规则时，补一句「**如果原实现有契约外的项，按语义找插槽**」，并给出该契约的插槽清单。
>
> **对策**：额外项迁到**语义相符的插槽**，不是随便找个地方塞。Kimi 的连通性测试本质是「对模型目录的探测」，所以落到模型区工具栏（`onTestModels`）与卡片第二行（`inlineActions`）——与同样有模型目录的 Codex 卡一致。**位置变了就是行为变了，要写进提交说明让用户能找回来。**

**模式五十八：迁到「更完整的」共享组件时，多出来的能力需要页面侧先有 handler。** 共享组件的行级/卡片级契约通常比单个 bespoke 实现**更完整**（四个行级按钮 vs 一个）。迁移后这些按钮要么渲染出来但**点了没反应**（handler 是 `undefined` 时组件仍渲染，取决于组件写法），要么干脆不渲染——两种都是「功能比预期少」。这不是共享组件的缺陷，是**迁移只做了一半**：把 UI 换过去了，没把 UI 需要的能力补上。
> 例（#109）：Kimi 的模型行只有删除，共享 `ModelItem` 有四个按钮；页面侧没有 copy / set-primary 的 handler。
>
> **判别方法**：迁移到共享组件后，**把新组件会渲染的每个按钮列出来**，逐个问「这个按钮点了会调什么？页面里有这个 handler 吗？」
>
> **对策**：缺的 handler **在页面侧实现**，不要为了迁就现状去裁剪共享组件的能力。实现前先确认这个能力在该 CLI 的存储模型里**成立**（Kimi 的 `defaultModelKey` 可写，所以「设为主模型」有意义；若语义不成立，才是应该不传 handler 的场景）。

**模式五十九：功能被有意关闭时，界面必须说明原因——否则用户读到的就是「坏了」。** 拖拽在「非默认排序 / 有搜索词」下被有意禁用，代码与产品意图都正确；但用户看到的只是一个**没有把手的列表**，而他记得上一版有把手。他能得到的唯一结论是「这个功能坏了」。用户报「拖不动」时，实现层可以是完全正确的——**排查的第一问应该是「它是不是被有意关掉了」，而不是「它为什么坏」**。
> 例（#111）：Codex 页 DB 里 `provider_sort_modes.codex = "created"`，拖拽按设计关闭。
>
> **判别方法**：任何「用户可见能力消失」的状态（禁用、隐藏、只读、锁定），都要能回答：**用户怎么知道为什么？** 答不出来就是缺陷——不是「体验欠佳」，而是**用户会把正常行为报成 bug**，团队随后会去「修」一个没坏的东西。
>
> **对策**：在用户视线内的**同一控件**上说明原因（这里：排序按钮的 tooltip 文案），而不是写在文档或提交说明里。文案要指出**怎么恢复**（「选择默认排序」），不能只说「不可用」。

**模式六十：交互类缺陷不能用「读代码」验收——仓库里已经有真浏览器，用它。** 本轮的拖拽被「修好」两次，两次都是把调用链读了一遍、确认每一层都符合预期，然后宣布修好；两次都还没修好。第三次改用**真实指针事件**在真 Chromium 里拖一下，一轮就定位了。读代码能验证「每层是否按契约传参」，**不能验证「这一串传参最后是否真的产生了位移」**——而后者才是用户说的「拖不动」。仓库已有 `scripts/lib/browser-fixture.mjs`（真 Chromium + CDP，含 `Input.dispatchMouseEvent`），此前只被用于表单类夹具。
> 例（#112）：`CardShell` 注册、`useSortable` 参数、页面传感器、`dragDisabled` 传参四处全对，而拖拽确实不动。
>
> **判别方法**：改动涉及**指针 / 键盘 / 焦点 / 滚动 / 拖放 / 悬停**这些「必须有真实事件序列才成立」的行为时，**默认走浏览器夹具**；不要用「我读了链路」代替。
>
> **对策**：把验证落成脚本并注册进 `pnpm test`（`scripts/verify-provider-list-drag.mjs`）。夹具要点：① 挂**真实页面**而不是重画一半（本轮挂真实 `CodexPage`，把页面自身的命令表 stub 掉，未 stub 的命令直接抛错而不是静默返回 `undefined`）；② 页面处在**真实 import 环**里（`CodexPage` → `providerShare` → `ProviderTransferModal` → `deeplinkImportAction` → `app/routes` → `routeConfig` → `CodexPage`），夹具必须从应用入口 `@/app/routes` 进入该环，否则会得到 `Cannot access 'CodexPage' before initialization`——**这是 import 顺序产物，不是产品缺陷**；③ 模块求值期就读的宿主全局（`__TAURI_OS_PLUGIN_INTERNALS__.platform`、`__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener`）必须放在**夹具的第一个 import** 里赋值，写在夹具正文里已经太晚；④ 拖拽要发**多步 `mouseMoved`**——单次跳跃会被当成瞬移，sortable 看不到 over 目标。

**模式六十一：判「外壳缺插槽」之前，先判这个东西在列表里的身份。** #106 那条规则（外壳的插槽完备性不受「至少两个消费方」门槛限制）只回答「外壳该不该留这个位置」，**不回答「它本来就该在那吗」**。Kimi 的官方账号区当时按那条规则加了 `toolbarExtra` + `footer`，规则没错、位置全错：它是**列表成员**，该和供应商卡片并排、能拖动排序。
> 例（#114）：用户一句「供应商列表右侧的官方账号按钮，应该放到官方订阅卡片的右上角吧」就把结论说完了——那东西在用户心里从来就是一张卡片。
>
> **判别顺序**：① 用户会不会期望它**能和供应商卡片并排 / 被拖动**？会 → 列表成员，走调用方的 `children`（槽位内渲染）+ `alwaysVisible`（空列表兜底），位置自己持久化；② 它是不是**整列表的动作**（导入、批量、登录后立刻新增一行）？是 → `headerExtra` / 工具栏；③ 只是**列表下方的补充信息**？→ `footer`。
>
> **同一个功能在别的 CLI 已经解过时，先去看那份实现**（同模式六十一）。Kimi 的官方账号区最终落的形态，就是 ZCode 三周前已经落地并写进规范的 `ZcodeOfficialAccountCard`（含 `official_account_index` 持久化）——迁移时没人去检索它，于是重造了一遍，还被用户指出「副标题位置不对」。

**模式六十六：一个功能「只在别的 CLI 里已经解过」时，先检索再设计。** 同一种产品问题（官方账号怎么展示、默认渠道能不能删、失败怎么回滚）在别的 tab 往往已经有实现 + 规范 + 守卫。**不检索就自己设计**，代价是：① 重新踩一遍已被记录的坑（#42 / #61 早就把「官方账号必须是列表成员、空态也显示、可排序」写成了规范）；② 同构的两份实现随后分叉（Kimi 的卡片是照 ZCode 抄的，于是「副标题位置」要改两处）。
> **动作**：动手前 `grep -rn "<同类名词>" web/features/coding/ | grep -i card`，并读那个 CLI 的 `AGENTS.md`；确认「这份实现存在吗 / 它为什么这么设计 / 有没有守卫」。**用户说「参考 XX」时，这句就是让你去检索的信号。**

**模式六十七：已有的迁移流程里，缺的那一步就是本次要补的步骤。** 「把存量数据收编进新模型」这件事，仓库里通常已经有一两条同类迁移（Kimi 已有 `migrate_legacy_credential_names`，挂在启动刷新循环里）。新需求「启动时迁移一次」的正确落点是**同一个函数序列**（同一把锁、同一个启动时机），不是新开一条路径——否则两条迁移会各自读同一份磁盘状态、各自写库。
> 例：Kimi 的「把 CLI 已有的登录收编成账号」接在 `migrate_legacy_credential_names` 之后、`refresh_applied_kimi_accounts_if_needed` 的同一把 `OAUTH_REFRESH_LOCK` 里，条件写窄（账号表为空 + 已应用 provider 是 official + 凭据文件能解析出 access token），失败不致命。
>
> **先检索同形状的既有实现。** 同一种东西（官方账号）在别的 CLI 已经被解过——ZCode 的 `ZcodeOfficialAccountCard` 早就是成员卡片（#42 / #61 甚至把它写成了规范）。迁移前 `grep` 一下「同类区块」：`grep -rn "OfficialAccountCard\|official_account_index" web tauri/src`。**SOP 里的既有规范要主动去查，而不是等用户指出来。**

**模式六十二：一句文案被两层语义共用时，改措辞会改错另一层。** i18n key 是按「谁最先用到」抽的，不是按语义层级抽的。Codex 的账号行「切换」与渠道卡片「应用」共用 `codex.provider.apply`；`applied` 同时是卡片高亮徽章和账号行的「当前」徽章。
> 例（#115）：用户的口径是「多个官方账号之间是切换，只有渠道的切换才叫应用」——直接改共用 key 会把渠道按钮也一起改掉。
>
> **对策**：改任何文案前先 `grep -rn "<key>" web/`，逐个渲染点判定它属于哪一层；跨层的就**按层拆 key**，不要复用。判据：**两个渲染点会不会在同一屏里同时出现？** 会，且它们的语义不同 → 必须拆。

**模式六十三：「空了一大块」这类报障，读代码永远看不出来——去量几何。** 用户说的是「某个区块右侧空了一块」。代码里没有 `width`、没有空 div、没有多余的 `padding`：那个区块只是被渲染进了**外层两栏布局的左栏**里，于是它的右边缘比卡片少一截——而这个「一截」恰好等于右栏（动作链接）的宽度，屏幕上就表现为一片死白。**没有任何静态检查能发现**：没有报错、没有类型错误、JSX 合法、截图不看数字也说不清差多少。
> 例（#116）：`CodexStyleCard` 的 `footer` 渲染在头部两栏的左栏内。实测：卡片内容右边缘 1251、账号行动作右边缘 1215，差 36（该卡未渲染「应用」）；用户那张卡渲染了「应用 / ⋮」，差约 160px。
>
> **对策**：① 用真浏览器量 `getBoundingClientRect().right`，**把「与谁对齐」写成断言**（这里是「区块右边缘 == 卡片内容右边缘 == 头部动作右边缘」），落成 `pnpm run test:codex-official-accounts`；② 判据本身也要写对——「这一个区块」看着没问题，要和它**上下相邻的同类区块**比：卡片里的每个「区」（模型区、官方账号区）都该跨满整宽，**只跨了头部左栏那一半就是错的**。
>
> **推论**：任何「XX 和 YY 没对齐 / 差一截 / 空了一块 / 挤在一起」的报障，先量两个元素的边，再读代码。

**模式六十四：说明文字贴它说明的对象走——区块的说明是「副标题」，不是「脚注」。** 一个区块自带解释性文字时，它的位置决定了它读起来是什么：紧贴标题下方 = 「这个区块是干什么的」；挂在区块最底部 = 「一段不知道属于谁的补充说明」。空态会把这个差别放大——内容区被一大块空插画占满，底部的说明离标题隔了整整一屏。
> 例（#117）：Kimi 官方账号卡片把「切换会把该账号的登录凭据写入 Kimi 配置…」放在账号行之后（照抄 ZCode 的写法）。用户圈出来说「副标题应该放在标题下面啊」。
>
> **判据**：遮住内容区，只留第一屏——那句说明还看得出是在讲这个区块吗？看不出就是放错了。**顺带检查写法来源**：这段是从 ZCode 拷贝的（用户当初的要求就是「参考 ZCode」），所以**同构的另一份也要一起改**，否则下次谁抄谁都是掷硬币。
>
> **搬运时要成对改**：把一段文字从 A 位置挪到 B 位置，最容易犯的错是**只加没删**——渲染出来是同一句话出现两次，比放错位置更显眼。给浏览器夹具加一条通用断言即可拦住：**「卡片里没有任何一句话被渲染两次」**（只比长度 ≥ 20 的叶子文本，短标签如「切换」本来就该重复）。

**模式六十五：共享组件「统一」掉的两个值，如果原本就是两个，用户会逐个发现。** 迁移把三张卡片的样式收成一份时，凡是**原本就不一致**的数值（内边距、行间距），统一都会改掉其中一部分卡片的外观——而且不会报错、不会破坏任何断言。用户看自己最熟的那个 tab，一眼就能发现「比之前小了」。
> 例（2026-10-08 两次）：① `CardShell` 的 body padding 抄的是 `components/common/ProviderCard` 的 `8px 12px`，而所有 provider 卡片用的是 16（claudecode/codex 迁移前、claudedesktop/geminicli/grok 至今）→ 每个迁移过的 tab 都少了 8px 上下；② Claude 式卡片的标题/副标题间距原本是「栈间距 4 + 行自身 4 = 8」，Codex 式是 4；共享组件把两者压成 4 → Claude Code 卡片少了一半。
>
> **对策**：① 迁移时把「原值」当成契约逐个抄下来（`git show <迁移前>:<原文件>` 里的 `styles={{ body: … }}` / `marginTop` / `gap`），**不要**从另一个共享组件里就近取值；② 共享组件内部**允许各变体不同值**——这不是分叉，是原本就存在的差异，判据是「原实现是不是这个值」；③ 把数值落成浏览器断言（`pnpm run test:provider-card-spacing`：Claude 式 8、Codex 式 4），因为间距**只有量出来才算数**（同模式六十三）。

**模式六十九：同一产品区块在 N 处各写一份时，先抽组件、再改行为。** 这类重复的症状不是报错，而是**同一个东西在不同页面长得不一样**：同一个动作三种叫法、同一句说明一处是副标题一处是脚注、同一个按钮一处隐藏一处画成置灰。判据很简单——**同一段文案/同一组按钮/同一种空态在第二个地方也被写了一遍**，就该抽。
> 例（#120）：官方账号列表在 Codex / Kimi / ZCode 三处各写一份。抽 `shared/officialAccounts` 时**两处判断最容易做错**：① 把「登录入口」做成布尔开关（`showLogin`）而不是 `ReactNode` 插槽——于是共享组件被迫知道「一个 CLI 有几个官方 provider」（ZCode 是下拉、Codex 是单按钮）；② 把「挂在卡片内部」与「自持一张卡片」当成参数变体，而不是两种宿主。**插槽与宿主选择是共享组件的两个正确出口**：前者吸收调用方的形态差异，后者吸收调用方的容器差异。
>
> **动作**：抽之前先 `grep` 出所有实现点（`grep -rn "OfficialAccountCard\|official_account_index" web tauri/src`），逐条对照差异并分类——「真实差异」（宿主、数据形状）留成 prop，「无意分叉」（措辞、位置、禁用态）统一。抽完落一条源码棘轮（同 `verify-provider-card-layout.mjs` 的机制），否则下一次接入又会手写一份。

**模式七十：「登录 / 导入 / 应用」这类动作，成功之前不许落任何行。** 一个动作如果在开始时就写数据、失败路径又不回滚，它就会在用户的列表里留下**空壳**——不是数据损坏，是「看得见的垃圾」，而且只有用户会发现（因为开发者测试时总是走成功路径）。
> 例（#121）：Kimi 的登录在设备授权前先建 provider 行，失败只弹提示、取消是静默终态 → 每次「点登录又关掉」都留一张空壳卡片。**判据：拿这个 CLI 的登录流程与另一个 CLI 对照，问「取消之后库里多了什么」。**
>
> **动作**：把写数据的时机挪到**确认成功的那一刻**（这里是 token 交换成功、写账号之前 `ensure_official_provider`）；入口函数因此可以不再要求调用方预先准备好行（`start_*_device_auth` 去掉 `provider_id` 参数就是这件事的信号）。回归测试要覆盖「取消/失败之后库里没有任何新行」，而不只是「成功之后有一行」。

**模式七十一：一个字段同时当「显示名」和「文件名 / 键」用时，多值能力必然被锁死。** 这类耦合不会报错，它只是让某个能力**在物理上不可能**：要两个账号，就要两个文件名；而文件名是 CLI 认死的，于是「永远只能有一个」。
> 例（#122）：Kimi 的 `write_credential_file` 用 `account.name` 当文件名，`migrate_legacy_credential_names` 反过来把所有账号名改成 `kimi-code` 去迁就它 → 账号表只能按 provider 去重。解耦后：文件恒为固定键，`name` 当显示名，磁盘一份 live 凭据 + DB 多行快照（Codex 的形状）。
>
> **动作**：审查任何「账号 / 渠道 / 配置」记录时，逐个字段问一句「**它是不是被当成文件名、键或唯一标识在用**」。是 → 它就不能同时承担显示职责；把固定键提成常量，显示字段腾出来。

**模式七十二：说「这个接口/凭证没有 X」之前，先把能拿到的东西解开看一眼。** 「没有身份信息」这类结论最容易从**数据结构**上误判：看一眼类型定义、发现没有 email 字段、再发现文档里没有 userinfo 端点，就宣布「拿不到」。可身份往往藏在别处——JWT 的 payload、另一个已存在的接口、响应头。
> 例（#123）：Kimi 的账号行一度只能显示「登录时间」，理由是「Kimi 的 OAuth 不发身份」。实际上 access token 就是 JWT（`sub` = user_id），而 2.x CLI 早就在调 `GET {base}/me` 拿 `nickname` / `user_level_name`。**判据：先解码（JWT base64 payload）、先请求（同名接口打一次看响应），再决定说「没有」。**
>
> **动作**：① 凭证是 JWT → 立刻解码 payload 看 claims；② 想知道「有没有 X 接口」→ 先 `grep` 同源 CLI 的源码里的 URL 常量，再实打一次；③ 由此得到的身份字段要**优先用于判重**（重登换 token，身份不换）。

**模式七十三：源码守卫匹配了「渲染后的 DOM」，于是它永远是绿的。** 写静态守卫时最容易犯的错，是拿**运行时才存在的名字**去 `test()` 源码：antd 的图标在源码里是 `<SwapOutlined />`，渲染成 DOM 之后才是 `anticon-swap`；class 名、`data-*` 属性、CSS module 的哈希名都属于这一类。它们让守卫**每次都以 0 违规通过**——而守卫存在的全部意义就是找出违规。假绿比没有守卫更糟：它让「已检查」这四个字变成一句谎话。
> 例（#125）：`verify-official-accounts-shared.mjs` 的 `FORBIDDEN` 写的是 `/anticon-swap/`、`/anticon-check/`，于是 geminicli / grok / antigravity 三份手写区块从未被扫出，报告「0 still pending migration」。改成源码级信号（自己 `map` 账号行）后，三份立刻现形。
>
> **动作**：① 守卫只匹配源码里真实存在的字符串——组件名、JSX 结构、import 路径；② 写完先拿**已知违规的文件**试一次（哪怕在 `node -e` 里 `test()` 一下），命中了才算守卫存在；③ 守卫输出里的「待迁移数量」应当与手工 `grep` 的结果对得上，对不上就是守卫错了，不是 grep 错了。

**模式七十四：抽共享组件时，按「一个原实现」抄形状，会丢掉其余实现共有的装饰。** 组件抽取的注意力都在**行为**（回调、状态、门控）上，装饰（图标、数量徽章、副标题、分隔线）被默认为「细节」。但原实现有 N 份，随手参照的那一份**恰好没有**某个装饰时，抽出来的组件就没有它——而另外 N-1 份本来有。没有任何检查会报「图标丢了」：类型对、行为对、截图也没人说得出少了什么，直到用户拿另一份的截图对照。
> 例（#124）：`shared/officialAccounts` 的标题行照 Codex 写（Codex 把 🔗 画在登录按钮上），于是标题行没有链接图标；Kimi 迁移前**本来就有**（`KimiOfficialAccountCard.tsx:125`）、ZCode 也有（在自己的 `leadingAction` 里）。用户对照 ZCode 截图说「都改成 zcode 这种样式」。
>
> **动作**：抽组件前，把**每一份原实现的装饰逐个列出来取并集**——`git show <迁移前>:<每个原文件>` 并排读，列图标 / 徽章 / 副标题 / 分隔线 / 折叠箭头；再逐项判定「真实差异（进 `variant`）」还是「共同契约（进组件）」。判据：**任何两份共有的东西，都不该由调用方再传一遍**。
>
> 沿用一次（#124 → #126）：🔗 的归属最终不是「区块自己画」，而是「**标题行**画，而标题行在不同的宿主上是不同的那一行」。抽组件时定的位置规则，遇到「区块挂在卡片内部」这种宿主时会整体位移，规则要跟着宿主写清楚。

**模式七十五：「把这块换成那块」的需求，先问清「这块」是哪一行——一个区块在卡片里通常有两行候选。** 「区块」这个词是模糊的：它既可以是**卡片自己的头部行**（名字 + 标签），也可以是**区块自己的标题行**（列表标题 + 折叠箭头）。两者相距一行说明文字，改错了没有任何检查会报——类型对、守卫绿、几何断言全过，因为每一行都还在，只是**在错的那一行上**。
> 例（#126）：用户要「把卡片头部那两行换成 `🔗 官方账号 (2)` + 说明」，我改成区块自己的标题行；用户回「改错了啊」并附三张图（现状 / 目标 / 必须保持不变的那部分）。
>
> **动作**：① 位置类需求（「把 A 换成 B」「按钮放到 X 的右侧」「上面那块」）先用 **ASCII 预览或截图选项**把候选行摆出来问清是哪一行（一次提问换一轮返工，划算）；② 归属判据是「它讲的是**这张卡片**还是**这一串行**」；③ 改完用**坐标**断言钉住（例：`login.top > heading.bottom && login.left >= listTitle.right && 中线差 ≤ 8px`），视觉话术（「在右侧」）只有几何能验收。

**模式七十六：三端各拼一次同一个产品区块，样式会一版一版分叉——统一要连「措辞、图标、计数、折叠」一起收。** 用户的原话最省事：「官方订阅 那个标志可以去掉了」→「Kimi Official 标题都统一使用官方账号」→「zcode 也加上账号列表的折叠能力，也就是官方账号卡片的样式是统一的可复用的」。三条合起来就是**一整套形状**的收拢。
> 例（#127）：Codex / Kimi 的卡片头部各自拼标题、ZCode 自己拼；措辞三套（「账号列表」/「官方账号」/「Kimi Official」），类别标记 `官方订阅` 只有两家画，折叠只有 Codex 有。
>
> **动作**：统一一个产品区块时逐项过这张表——① **措辞**收进共享 i18n key（各 CLI 仍自己传已翻译串，但传同一批 key）；② **图标与计数**收成共享导出（`OfficialAccountHeadingIcon` / `OfficialAccountCount`），宿主只负责把它们放进自己的行；③ **能力**（折叠、空态、行内动作）三端都要有，**初始状态可以不同**（Codex 收起 / Kimi、ZCode 展开）——共享的是能力不是初值；④ 收完之后**逐个宿主补断言**：共享组件被一个宿主接对，不代表另一个宿主的接线也对。

**模式七十七：守卫的作用域写成「文件名后缀」，覆盖的是文件「叫什么」而不是它「是什么」。** 这类守卫最像「已经检查过了」：脚本在跑、输出有数字、CI 是绿的——漏掉的文件恰恰是命名不符合约定的那一个。
> 例（#128）：`verify-provider-card-layout.mjs` 管「卡片 = 薄映射层」，作用域是 `endsWith('ProviderCard.tsx')`；ZCode 的官方账号卡片叫 `ZcodeOfficialAccountCard.tsx`，同样是列表成员（可拖拽、同一套外壳），却从未被扫过——它自己写 `<Card>` + `useSortable` + 把手，把手丢了悬停反馈、还被放进标题行内部，正文因此贴到卡片左边缘（用户圈出「zcode 为什么下面的内容左边距没了」）。它的 `EXEMPT_FILES` 里那条豁免（「不是列表成员、不可排序」）**因为后缀不匹配从来没被读过**——是假豁免。
>
> **动作**：① 守卫的**作用域**和它声称的**语义**要对齐：写「所有卡片」就用后缀集合（或按 import / 结构判），别用单个后缀；② 审一遍 `EXEMPT_FILES`：**读不到的豁免是假豁免**，它掩盖的正是它声称豁免的那类文件；③ 输出里的扫描文件数要能手工 `ls | grep` 对得上（11 个变了就是 11 个）；④ 外壳（边框、把手、悬停、内容列）一律走 `CardShell`，别手抄——手抄的那份一定会少点东西。

**模式七十八：按钮/控件的落点看「它改动谁」，不看「哪里显眼」。** 一个动作放错栏位不会报错：它照常工作、照常有反应，只是**离它改动的对象隔了一行**——读起来像两件事。
> 例（#129）：账号区块的登录按钮被放到卡片头部右侧（与「应用 / ⋮」并排），用户贴图：「登录按钮应该是放到账号列表标题的右侧啊」——它加的是列表里的一行。
>
> **动作**：① 落点判据——改列表 → 列表标题行；改卡片 → 卡片头部；② 与相邻动作的**同行关系**也要断言（同一个 `Space` / 同一行的中线）；③ 改完在真浏览器里看一眼截图，「隔着说明文字」这种错位读代码读不出来。

**模式七十九：同一句话有两条渲染路径时，样式会漂——「共享文案」要连**组件**一起共享。** 把句子抽成共享 i18n key 只解决了「字一样」，没解决「样子一样」：一处走 antd 的 `<Text type="secondary">`，另一处手写 `style={{ color: 'var(--color-text-secondary)' }}`，两个名字读起来是一回事、渲染出来差一档。**没有任何静态检查看得见**：类型对、源码守卫绿、几何断言也过（位置没变），只有真浏览器里的 `getComputedStyle` 能发现。
> 例（#130）：账号区块的说明句在 Codex / Kimi 上走 `metaEntries`（antd `secondary` = `colorTextDescription` = alpha 0.45），在 ZCode 上由区块自己画（`--color-text-secondary` = alpha 0.65），ZCode 那句比另外两家深。用户贴图问「颜色和字体大小和另外 2 个的不一样吗？」
>
> **动作**：① 判据——**「同一句话」= 同一个组件**（`OfficialAccountHint`），不是「同一个 key + 两处手抄样式」；② 容器/插槽的 value 类型要允许 `ReactNode`（`ProviderCardMetaEntry.value` 已从 `string` 放宽）——样式表只管**放哪**，不管长什么样；③ 验收写 `getComputedStyle(el).color / fontSize`，并且**每个宿主各测一遍**：共享组件被一个宿主接对，不代表另一个宿主的接线也对（这条与模式七十六的动作④同源）；④ 遇到 `--color-text-secondary` 与 antd `type="secondary"` 这类「看起来等价」的名字，先查实际值再选——前者 0.65、后者 0.45，选错就是这种漂。

**模式八十一：源码守卫的「作用域」是文件名/路径，新组件会被误扫或被漏扫——名字是契约的一部分。**
> 例（#132）：`verify-provider-card-layout.mjs` 按后缀 `*ProviderCard.tsx` 扫「CLI 映射层不许自建 chrome」。新增的共享卡片 `ProviderCompactCard.tsx` 自己 `useSortable`、自己渲染复选框——这在映射层是违规，在共享外壳（`CardShell` 同级）是**它的职责**。叫 `CompactProviderCard.tsx` 就会被误报；叫 `ProviderCompactCard.tsx` 天然落在作用域外。反向的坑仓库里也有：ZCode 的 `*OfficialAccountCard.tsx` 曾因作用域只写 `*ProviderCard.tsx` 而**从未被扫到**，躲了三周。
>
> **动作**：新增组件/文件前，先 `grep` 既有守卫脚本的作用域规则，跑一次确认结果符合预期。**用文件名表达身份**（`*ProviderCard.tsx` = 映射层、`ProviderCompactCard.tsx` = 外壳），而不是靠注释声明；确需留在作用域内的合法例外，登记进脚本的 `EXEMPT_FILES` 并写理由。守卫「扫出不该扫的」和「扫不到该扫的」是同一枚硬币的两面。

**模式八十二：把「显示偏好」做成「第四种语义变体」——选型表从此失去意义。**
> 例（#133）：三种卡片样式按 **provider 模型形状**（有没有模型目录、有没有单一 active provider）选型，`index.ts` 那张表就是选型规则。若把「卡片视图」（按用户偏好切换的紧凑呈现）加进去，`index.ts` 那张表就失去意义——「该用哪种样式」变成由显示偏好回答。
>
> **动作**：① 判据——**「这个选择是语义差异还是显示差异？」** 语义差异（provider 模型形状、单/多 active）→ 进变体模块；显示差异（密度、排序、折叠、视图模式）→ 独立模块，两者共用面抽成纯函数/构建器（本例 `cardChrome.ts` / `providerCardMenu.tsx`）。② 独立模块的 `AGENTS.md` 开头就要写「这不是第四种样式」，并交叉引用对方的文档——下一个人先读哪份都能看到这条。

**模式八十三：同一个后端读取被「另起一个 hook」复制成两份缓存——多一次往返，且两份各自漂移。**
> 例（#134）：`useProviderListSort` 有模块级单飞缓存，所有 tab 共享一次 `get_provider_list_state`。新增布局偏好时若另写自带读取的 hook：每个 tab 多一次往返，A 页改了布局 B 页看不到（各自持有快照）。
>
> **动作**：① 判据——**「这份状态是不是同一个后端读取返回的？」** 是 → 接同一个缓存（读取走共享 loader，写入走共享 patcher）；不是 → 才另起。② 抽缓存时把「读取 / 就地写入 / 失败兜底」三件事一起抽，**对外签名保持不变**（本例 `useProviderListSort` 的 11 个消费页面零改动），否则抽取本身就成了回归源。

**模式八十四：一个状态在两种布局里由两段代码各说一次——第二次改动就分叉。**
> 例（#135）：卡片视图的「已应用」只有 accent 边框，列表卡片是 `AppliedTag`；用户看到的是「标识没了」。两处都「实现」了这个状态，所以没有任何检查会失败。
>
> **动作**：① 判据——**「这条信息在另一种布局里是不是同一个组件/同一棵树画的？」** 不是 → 抽出共用的片段（本例 `stateTags`），两种布局只决定**加什么前缀**，不各自判断状态；② 「边框/颜色/动画」这类**非文字**表达不算陈述——用户不会从边框反推出「已应用」，需要陈述的就必须有标签或文案。

**模式八十五：新控件的形状与同排邻居撞车——单图标没有冗余信息可以纠错。**
> 例（#136）：`LayoutGrid`（lucide）与 `AppstoreOutlined`（antd）都是 2×2 方块；同排两个按钮看起来是同一个功能。
>
> **动作**：① 新增控件前把候选图标与**同一行已有的每一个**比形状（2×2 方块、三横线、加号、放大镜、齿轮最容易撞）；② 图标之外补文字——**同排已经有图标语义重叠时，文字才有冗余**，用户可以把误读纠正回来；③ 落成断言：把两个按钮的图标名与文字都读出来比对（本例浏览器套件读 antd 的 `aria-label` 图标名 + 按钮文字）。

**模式八十六：展示规则被当成 CSS 属性——CSS 做不到的那半就没人做。**
> 例（#137）：「中间省略」在 CSS 里不存在（`text-overflow` 只能尾省略），而 key 的两端都在传达身份。
>
> **动作**：① 判据——**「这个规则能不能用一条 CSS 声明表达？」** 不能 → 写成 `.ts` 纯函数（放 `utils/`，与组件同目录树），浏览器断言只能覆盖它被渲染出来的那一支；② **只有一支会被夹具走到**，另一支（短值原样、恰好超一个字符、空值）必须写单测；③ 省略/截断/隐藏都要有**第二出口**（`title`、复制按钮、展开），否则等于丢数据。

**模式八十七：共享渲染函数里写死了密度（字号 / 内边距）。**
> 例（#138）：`renderProviderLaunchAction` 内联 `fontSize: 12`，被列表卡片头部与详情侧栏同时消费；用户要卡片 footer 变小，改宿主 CSS 被内联样式压住。
>
> **动作**：① 判据——**「这个数值在另一个宿主里也该一样吗？」** 不一样 → 不写进共享函数，由宿主 scope 决定（`:global(.ant-btn)` 挂在各自的容器类下）；② 共享函数只决定**结构**（放哪、有没有 tooltip、disabled 时怎么包一层），密度、颜色、字号留给宿主；③ 落成 `getComputedStyle` 断言（本例同时钉住卡片 11px 与侧栏 footer 11px、卡片标题 14px/600、标签 10px）。

**模式九十三：把「可折叠」改成「常开」是一次行为变更，测试与守卫里等那个折叠控件的地方必须同批改。**
> 例（#144）：把 `ModelListSection` 与 Codex 账号块的 Collapse 整个删掉之后（**这本就不该删**，见模式九十六），`test:codex-official-accounts` 仍 `waitFor` 列表卡片里的折叠 caret，超时报「账号区没出现」——测试喊对了，而当时被读成「区块没渲染」。同一批还藏着两个静默项：`openDetailCatalog()` 变成空操作（断言从此失去意义），以及「等 caret」本可以顺带保证的「等内容渲染完」也一起丢了。
>
> **动作**：① 报错指向的是**被删掉的东西**而不是坏掉的东西时，先问「它该被删吗」，再去改测试；② 等待条件从「等控件」换成「等**内容**」（行、文本），内容才是「区块到达」的证据；③ 删任何控件前 `grep` 它的选择器，把「点它 / 等它」的地方逐个处理；④ 判据——**「我删掉的这个控件，还有谁在等它？」**

**模式九十四：共享组件的可选 prop 一改，先数调用点——「改一处」会静默改掉另一处。**
> 例（#145）：用户说的是「**明细**里的账号列表不要折叠」，而 `renderOfficialAccounts()` 同时喂列表卡片的 footer 与详情侧栏；去掉折叠后卡片那处也变了。同一天就露出答案的形状：`OfficialAccountsSection` 的 `collapsed` 由「有没有传 `onToggleCollapsed`」决定（`collapsible`）——ZCode / Kimi 保留折叠、Codex 侧栏没有，**这个差异本来就是组件存在的理由**（#147 把它彻底做实：两个区块都按宿主给不给折叠 props 决定形态）。
>
> **动作**：① 改共享 prop 前 `grep` 全部调用方，明确回答「一起改，还是按宿主传值」；② **「两个宿主形态不同」本身就是最常见的理由**（明细侧栏是「打开来读」的表面，列表卡片是「并排扫读」的表面），判据是**「需求里点名了哪个宿主？」**——点名了就写成参数，两边都没点名才考虑统一；③ 把「谁折叠、谁不折叠」写进文档，否则下一个人会以为是漏改。

**模式九十五：拖拽库的「禁用」通常分两半（拖 / 放），写成一个布尔值会只关掉一半——不可排序的子项会成为上层拖拽的碰撞噪声。**
> 例（#146）：dnd-kit 的 `normalizeLocalDisabled` 把布尔 `disabled: true` 展开成 `{ draggable: true, droppable: false }` ——**这两个字段是「是否禁用」而不是「是否启用」**（`droppable` 那条还标着 "Backwards compatibility"），所以它的意思是**拖被禁用、放仍开着**，只关掉了一半。Kimi 的模型行不重排、又没有自己的 `DndContext`，于是注册进供应商列表的上下文；拖卡片经过模型列表时 `over` 解析成模型行 id，`handleDragEnd` 的 `indexOf` 返回 -1 而静默返回——「模型列表一展开，供应商就拖不动了」。
>
> **动作**：① `disabled` 写对象形式，两个语义各表各的；② 判据——**「这个 sortable 的 id 在它外层 `SortableContext` 的 items 里吗？」** 不在 → 关掉它的 `droppable`；③ **先看库自己的 live region**（dnd-kit 会在 `[aria-live]` 里写出 `Draggable item X was moved over droppable area Y`）——「拖拽没反应」用这一条比读代码快一个数量级，同类库（react-beautiful-dnd 等）也有对应机制。

**模式九十六：「某个宿主要的样子」不要写成共享组件的唯一形态——「明细」两个字出现在需求里，就说明另一方不该动。**
> 例（#147）：用户 2026-10-12 在**明细侧栏**上提的要求（模型列表常开、动作让出宽度、字号小一点）落进了共享 `ModelListSection` / `ModelItem`，**列表布局跟着一起变**——用户 2026-10-13：「列表的样式不要变啊，列表样式的模型列表怎么也没有折叠了」。同一天里，同样的作用域误判在「设为主模型按钮常驻」上已经犯过一次（那是把明细的要求做成了全局行为），#145 的判据写下不到一天又被违反。
>
> **动作**：① **形态差异写成参数**（`appearance: 'card' | 'detail'`），默认值必须是**列表那一侧**——列表是既有行为，改动它需要理由，而明细侧的每一条都有理由；② 一个宿主读参数就够，值别分散在数据对象里（本例让开 caret 的 18px 缩进一度写在两个宿主共用的 `bodyStyle` 里，于是必然有一侧是错的——把它搬进区块自己按 `appearance` 施加）；③ 回归要**两侧都钉**：被点名的宿主用成对断言（静止/悬停），**没被点名的那一侧也要补「它没变」的断言**（本轮补「列表卡片仍折叠 + 点开有行 + 行是 13px / 把手在 / 带文字的动作占位」四条，其中三条正是被误改的那三个属性）；④ 判据——**「这句话里点是哪个宿主？另一个宿主的现状是什么？」** 答不出第二个问题就先别改共享组件。

**模式九十七：关掉一个异步卸载的东西（抽屉 / 弹窗 / 菜单）之后，「它存在」这个等待条件会立刻通过——因为上一个还没卸载完。**
> 例（#149）：Kimi 卡片视图检查里 `closeDetail()` + 固定 `delay(300)` 后点开另一个 provider 的侧栏，`waitFor(detailPanel())` 立即成立（旧面板还在关闭动画里），后面两条断言读的是**上一个 provider** 的面板，于是「官方侧栏里有账号块」判为 false——报错指向产品，实际是时序。
>
> **动作**：① 关掉之后**先等它消失**（`waitFor('!fixture.detailPanel()')`），再等下一个出现；② 更好的等待条件是**新内容自己的特征**（`detailTextIncludes('账号列表')`）：它同时排除「旧的还在」和「新的还没到」，而「元素存在」两个都排除不了；③ 固定 `delay` 在这里等于猜动画时长——同模式九十三：等待条件选「内容」，不选「控件/容器的存在」。

**模式九十八：列表里的「非 provider 成员」在网格布局里要么没有位置，要么被挤成一条读不了的窄栏。**
> ⚠️ **本条的动作（跨整行）已被 #150 / 模式九十九推翻**——位置那一步是对的，形状那一步不是：网格里一个成员唯一的形状是**格子**。规则仍成立（「它在列表里是谁？」），实例判错（成员不等于一整行）。
> 例（#148）：ZCode 的官方账号卡片是列表成员（可拖动排序、参与搜索过滤与空态）但不是 provider。网格按「一格一个 provider」排布时它无处安放：塞进一个 350px 格子会把账号行压成一栏，提到网格之外又会丢掉它的槽位（`officialAccountIndex`）；而且当时的列表布局还用 `providerIndex === officialAccountSlot` 单独实现了一遍「插在哪里」。
>
> **动作**：① 先问**「它在列表里是谁」**——是成员就留在网格里；② 两种布局的成员顺序必须来自**同一个表达式**（本例的 `listMembers`），否则「它放在哪」会有两个答案；③ **形状**那一步见模式九十九（网格里一个成员唯一的形状是格子，不是一整行）。

**模式九十九：一个列表成员的「形状」由宿主决定——为它发明一种形状（跨整行）和让它不穿形状（裸内容）都是同一个错误的两个方向。**

> 例（#150）：ZCode 的官方账号卡片在网格里错了两版。第一版保留了列表的 `CardShell`（全宽的描边卡，像第四种卡片样式），第二版干脆不套外壳（一块裸内容摆在格子之间）。两版都「有位置」、两版的断言（在网格里、跨整行）都过，用户两次都贴图说还是不对。定稿是**穿宿主里其他成员穿的那件**——同一个 `ProviderCompactCard`，同宽同高。
>
> **动作**：① 先问**「这个成员的形状由谁定」**：宿主定，就把它画成宿主里其他成员的形状——不要为它发明第三种（跨整行、独立外壳），也不要让它不穿衣服；② 「它的内容比一个格子大」不是形状问题，而是**内容去哪**的问题：管理器（一行一个条目、每行各有动作）进**这张格子自己的详情侧栏**（2026-10-14，与 Codex / Kimi 卡片模式同形），格子那行只说一件事；③ 正向断言（「在网格里」「跨整行」）对这两种错法都是绿的——**几何要量、形状要对比**（与邻居同宽 ±1px、穿同一个外壳组件），别只断言「它在网格里」；④ 同一个坑被用户报两次，说明第一次改的是**症状**（换了个位置/去掉了边框），要停下来问「它在两种宿主里本该是什么」。

**模式一百：同一段 chrome 在 ≥2 个调用点逐字重复时，它属于共享组件——n 份拷贝不会同时坏，所以没有测试会发现它们漂了。**
> 例（#151）：四个映射层各自渲染一份一模一样的 `<Drawer>`（placement / 宽度 / `destroyOnHidden` / `closable` / body styles 全同），差异只有面板内容；面板早已共享，抽屉却没有。用户看到「ZCode 的官方账号块也得进抽屉」时说「样式都封装到通用组件啊，不要每个单独改啊」。
>
> **动作**：① 判据是**调用点个数**，不是「它看起来算不算样式」——≥2 就搬；② 搬的是**外壳**，调用方给**内容**（`detail = { title, statusTags, sections, … }`），开合状态与副作用留在共享组件里；③ 搬完 `grep` 旧写法（`onOpenDetail`、`<Drawer`、`detailOpen`）确认残留为 0；④ 顺带把「什么不是控件」的过滤列表一起收——卡片里的把手是 `<span>`，漏了它会让「按住把手拖一下」顺带触发主体点击，补一条反向断言（「拖它不会打开侧栏」）。

**模式一百零一：推翻一条结论时，正文里的实例、模式行、孤儿段落、全仓的特征串要一起清——否则「写过了」读起来像「没写」。**
> 例（#152）：用户问「sop 里面没写吗？」，实际是一处写了、三处没清（跨整行的旧动作、`rowItem` 的孤儿块、`providerCardView/AGENTS.md` 里与同文件自相矛盾的那条）。
>
> **动作**：① 教训行加「结论已被 #NNN 推翻」（#106 的先例：规则仍成立、实例判错也要标）；② 模式行同样标注；③ 孤儿段落删掉——**回滚一个方案时把它留下的那一段一起删**，别只改结论；④ `grep` 被推翻做法的特征标识（`renderBody` / `rowItem` / `跨整行` / `onOpenDetail` / `bodyCustom`）扫全仓 `.md`。判据：**「读者照这份文档做，会做出哪一个？」**

**模式一百零四：位置是跨 CLI 的约定——「别的格子都在这个位置说它的控件」时，空着不是克制，是缺件。**
> 例（#154）：ZCode 没有 provider 级 应用，于是格子第二行的动作槽一直空着，而它唯一的 per-provider 控件（启用开关）折在「更多」菜单里；用户指着列表卡片头部那颗开关说「放出来，放到现在应用的地方」。
>
> **动作**：① 判据是把位置**横向比**——**「这个位置在我邻居那里是什么？」**（不是「我这个 CLI 有没有那个按钮」）；② 卡片因此要有「本 CLI 自己的控件」的插槽（`cellControl`），**插槽而不是布尔**：卡片不决定控件是什么；③ 控件本体先**收成同一个组件**再上槽——它已经在头部与菜单里各画了一份，第三份就是漂移的开始（本轮 `ProviderEnabledSwitch`：三处一组件，`checked` 的取反只在一处）；④ 上了槽就**从菜单撤掉**那一条（同一张卡片同一个控件不说两遍）；⑤ 回归要真点（回写的参数、卡片随后的陈述、回来的路、不误触发主体点击、菜单里没有第二条）。

**模式一百零二：「陈述一件事的一行」在每个状态下都要有输出——留空读起来是坏了，不是「无话可说」。**
> 例（#153）：卡片布局里官方渠道格子的第二行「当前登录: <账号>」只在**官方渠道真在运行时**才说话（取值只认 `isApplied` 的行）。用户那台机器上两家渠道都闲置（Codex 应用的是 AxonHub、Kimi 两个渠道都没应用），于是两行都是空的，他连着问「**为什么 codex 和 kimi 的卡片下面没有显示当前的账号啊**」。代码没错、断言全绿——**规则太窄**：那行的位置无论说不说都被共享卡片占着（`.bodyWithAction` 的 30px），空格子读起来就是坏卡片。
>
> **动作**：① 给「说一件事」的行**枚举状态**，每条状态都要有输出（在用 → 磁盘上那份登录 → 都没有就明说「未登录」）；② 报「为什么没显示」时先分清**「数据里没有」还是「规则没问」**——本轮两者都有（Kimi 是真没有登录，Codex 有但没被问），查法见模式一百零三；③ 断言把**三态**都钉住（`accounts=virtual` / `accounts=idle` 两个夹具模式），只断言「有值的那一态」等于没断言；④ 取舍口径由用户定：他选的是**「只写真正在用的登录」**（不把闲置账号说成当前登录），所以闲置态写「未登录」而不是写它保存的账号。

**模式一百零三：真机上的「为什么没显示」，答案在你机器里的那张表，不在代码里——先把它读出来。**
> 例（#153）：判断链条（官方渠道有没有被应用？CLI 的凭据文件里是哪份登录？那 `(2)` 是怎么来的？）全部落在**用户机器上的数据**里：`codex_provider.is_applied`（=AxonHub）、`codex_official_account.is_applied`（=0）、`~/.codex/auth.json`（`auth_mode=apikey` + 一份过期的官方 token）、`~/.kimi-code/config.toml`（未登录的默认桩）。读出来之后结论自明，且**每一步都能被用户核对**。
>
> **动作**：① 应用库 `ai-toolbox.db` 的 `data` 列是 **SQLite JSONB**，用 `sqlite3 -readonly <db> "select json_extract(data,'$.is_applied') from codex_official_account;"` 直接问（**别猜二进制格式**：本次先按 msgpack 解，字节对不上，白白绕了一圈——`sqlite3` 自带 `json_extract`）；② CLI 侧的事实读**磁盘上的真文件**（`auth.json` / `config.toml` / `credentials/`），它们才是「当前在跑什么」的证据；③ 报告时把**读到的值**摆出来（谁的 `is_applied`=1、哪一行 `(n)` 由什么组成），用户才能判断这是 bug 还是状态；④ `~/.codex/auth.json` 里的 token 可能属于**另一个账号**（本地行 = 磁盘上活着但库里没快照的那份登录），"官方渠道闲置" 与 "磁盘上有官方登录" 可以同时为真。

**模式一百零五：能编译、能跑 ≠ 形态正确——§4.0.2 的核对必须在写第一行前端代码之前做。**
> 例（#155）：两个新 tab 的页面写出来了，共享组件一个没用：手搓 `Modal`+`Form` 顶替 `ModelFormModal`、`Collapse` 顶替 `ProviderListSection`、`Alert` 堆提示。编译通过、`tsc` 干净，形却是自己的。
>
> **动作**：① 动手写页面前，先 `ls web/features/coding/shared/` 并按 §4.0.2 A–H 逐项核对**参照 CLI 的源码**，把「我这一页的哪块对应哪个共享件」列出来；② 「编译通过」只能证明类型对，不能证明形态对——SOP §12.3 已写明视觉核对不可用编译替代；③ 两个同引擎 tab 共用的一段 UI 落 `shared/<领域>/`，不要 A 的页面 import B 的组件目录。

**模式一百零六：逐字段相同的两个 TypeScript 类型就是一份类型——出现 `as unknown as` 时，先删重复定义，别消音。**
> 例（#156）：`WorkbuddyModelInput` 与 `CodebuddyModelInput` 一字不差；共享弹窗组件的 `form` / `channel` 类型对不上，于是代码里长出 `as unknown as` 和 `as never`。
>
> **动作**：① 判据——**「两个 type 的字段列表能否逐行对上？」** 能 → 下沉到共享文件，两侧 `export type { X as Y }` 再导出（调用方一行不用改）；② `as unknown as` / `as never` 是「这里有两套不兼容类型」的信号，先查来源；③ 结构上的共享比行为上的共享更重要——行为可以 Copy，类型一旦分叉就会用 `as` 掩盖。

**模式一百零七：托盘不是「其它 tab 都有」的装饰，而是「切换当前生效对象」的控件——没有可切换对象就不做，并把这个决定写下来。**
> 例（#157）：WorkBuddy / CodeBuddy 既无单一 active provider，也无可靠的默认自定义模型指针，托盘上没有可切换的东西。
>
> **动作**：① 判据——**「本 CLI 有没有一个『当前生效』指针？」** 没有 → 托盘不适用；② 不做也要**写进计划文档**（本轮 §10.1），否则下一轮 review 会把它当遗漏补上；③ 若将来出现可切换对象（如网关接管了多条渠道），再按当时的实际切换项补，而不是现在预埋空壳。

**模式一百一十三：共享文案里出现产品名，就不是共享文案。**
> 例（#163）：CodeBuddy 复用了 `workbuddy.availableModelsFree`，中文读作「WorkBuddy 会显示内置模型…」。键存在、检查通过、文案错人。
>
> **动作**：① 复用前读一遍目标语言的实际文本，找模块名；② 有产品名就下沉到 `common.*` 并改成中性表述（「会显示内置模型与本文件中的全部模型」）；③ 需要点名时用 `{{tool}}` 插值，而不是复制一份带名字的键。

**模式一百一十四：`TARGET_SCHEMA_VERSION` 是全局单调游标——分叉期间 main 往前走一步，你这边写的迁移就整段作废，而且**不报错**。**
> 例（#164）：本 worktree 从 `b61f7b9d` 分叉时基线是 v30，于是把「WorkBuddy/CodeBuddy 三张表」写成 `migrate_v31`。分叉后 main 上的 `0ca25b58` 也占用了 v31（给 `proxy_request_logs` 加 `native_tools_used`）。后果是**双向的、且两向都静默**：
> ① 老用户库的 `user_version` 已是 31 → `if current_version < 31` 永不成立 → 三张表**一张都不会建**，页面读到空配置；`get_settings` 也不报错，只是模块入口按「未配置」处理，看起来就是「设置里没有这个 tab」。
> ② 全新库跑到的是**我改写过的** v31 → main 的 `native_tools_used` 列**永远不会被创建** → 网关请求列表查询直接失败。这一向更危险：它在别人的功能上造成回归。
> 实测证据（用户库，只读 `mode=ro`）：`user_version=31`、`native_tools_used` 存在、76 张表里 workbuddy/codebuddy 相关 = `[]`。三个事实同时成立，才定位到「版本号被复用」。
>
> **动作**：① **合并前必查**：`git log --oneline $(git merge-base HEAD main)..main -- <目标迁移文件>`，看 main 有没有在这期间动过版本号；② 写迁移前先 `git merge-base` 确认分叉点，**新版本号 = max(分叉点版本, main 当前版本) + 1**，不要用「我的旧基线 + 1」；③ 发现撞号时，**恢复 main 的那一段原样**（此处 v31 逐字还原为 `add_column_if_missing(..., "native_tools_used", ...)`），把自己的内容顺延到新版本号（v32），**不要**两者合并进同一个 `migrate_vN`——合并后内容与 main 不同，git 会把它当冲突区，且语义上两个不相关的变更被绑死；④ 回归必须从**「`user_version` 已是旧迁移版本、但缺我新加的字段/表」**起步（`v32_migration_adds_workbuddy_tables_to_an_already_v31_database`），把版本号降回去再跑是**假通过**——真实用户库正是「版本已高、表却缺」的形状；⑤ 老库补建的表一定用 `CREATE TABLE IF NOT EXISTS`（`create_jsonb_table` 已是），使新老库殊途同归、可重复执行。
>
> **判据（一句话）**：**「这条迁移在别人的库上有没有可能『已经算跑过了』？」** 会 → 版本号必须比 main 更高，且新加的每个表/列都要能被「版本已到位但结构缺失」的库补上。

**模式一百一十五：接管类 CLI 只补 `From<CliKey> for UsageTool` 不够——`gateway_cli()` 是反向门，漏一组 arm 会让**每条请求都不落库**，且测试只报「记录数为 0」。**
> 例（#165）：WorkBuddy / CodeBuddy 加进 `GatewayCliKey` 后，`From<GatewayCliKey> for GatewayUsageTool` 补了，接管流程能跑、上游能转发、客户端拿到 200，但 `record_request_summary` 开头 `summary.cli_key.and_then(GatewayUsageTool::gateway_cli)` 用的是**另一张匹配表**（当时是 `_ => return None` 兜底）→ 直接 `Ok(Skipped)`。现象：请求成功、统计页空、`proxy_request_logs` 里一行都没有。`privacy_matrix_tests` 把它抓出来，报错却是 `left: 0, right: 1`（详情条数），看不到「gateway_cli」这个名字。
>
> **动作**：① 新增 `GatewayCliKey` 变体后，把**两张**匹配表都补上：`From<GatewayCliKey> for GatewayUsageTool`（正向，接管 → 记账）和 `GatewayUsageTool::gateway_cli()`（反向，记账行 → 可接管）；② **反向表用穷举 match，不要 `_ => return None`**——后者把「忘了加」和「本来就不可接管」（Pi/Dsh 等纯采集工具）混成同一种静默；③ 回归断言要落在**落库结果**上（`request_logs(...).data.len() == 1`），不是只断言 HTTP 200。

**模式一百一十六：一个产品两套 `app_type` 拼写时，「按 CLI 筛选」必须展开成 `IN (...)`——只比 `tool.as_str()` 会让代理行在统计页里凭空消失。**
> 例（#166）：WorkBuddy 家族原生转录用 `workbuddy` / `codebuddy`（`GatewayUsageTool::as_str()`），网关代理行用 `workbuddy_desktop` / `codebuddy_cli`（`GatewayCliKey::as_str()`，刻意区分两者）。写入侧和读取时的**反查**（`cli_key_from_app_type`）都处理了两种拼写，唯独**筛选条件**没有——`usage_stats.rs` 里 5 处 `WHERE app_type = ?` 全部只绑 `cli_key.as_str()`。现象：请求详情能列出该行，一选「WorkBuddy」筛选就全空。
>
> **动作**：① 加一个 `app_type_filter_values(tool) -> Vec<&'static str>` 作为筛选用的事实源，特殊产品返回两种拼写，其余返回单个；② 5 处筛选点（详情列表 / 项目统计 / 日聚合 / 趋势 / 事件）**全部**改用它 + `IN (?, ?)`，不要只修用户最先看到的那一处；③ 回归要正面覆盖「同一工具的两条不同 `app_type` 行，在筛选后都在列表里，且 `usage_summary` 的 `total_requests` 与之一致」。

**模式一百一十七：`pnpm test` 里出现与本轮改动无关的浏览器夹具失败时，先用**同一夹具连跑 5 次**判断是不是既有 flaky，再决定要不要二分——`git worktree` 基线对照会被 node_modules / Vite 缓存拖成伪证据。**
> 例（#167）：`verify-codex-fetched-model-picker` 失败在 `official failure exposes bundled fallback`。本轮对 Codex 目录**零改动**（`git diff --stat HEAD -- web/features/coding/codex/ tauri/src/coding/codex/` 为空）。做了两件错事才发现真相：① 复制 `web/constants/modules.tsx` 到基线 worktree 后行为翻转，一度判成「新增 tab 导致 Codex 夹具静默失败」——实际是每次 `git checkout` 都连带清了 Vite 预构建缓存，**变量一次动了两个**；② 在基线 worktree 里连跑时用 `tail -3` 取结果，把 `}` / `Node.js v24` 这类无关行当成失败，得出「基线也挂」的错误结论。
> 真相：**同一 worktree 连跑 5 次，4 次通过、1 次失败，失败点每轮不同**（`late cancelled fetch` / `late result from previous provider` / `official failure`），全是异步竞态断言，`actual: []` 表示那一轮的 `resolveNext` 没落地。这是夹具自身的时序脆弱，不是代码回归。
>
> **动作**：① 先跑 `git diff --stat HEAD -- <失败夹具的整个功能目录>`，为空就**强烈提示与改动无关**；② 同一夹具**连跑 5 次**，看失败点是否游走——游走即 flaky，固定才可能是回归；③ 需要基线对照时，**不要复制单个文件**（会连带改变构建图），要么整棵树 `git worktree add` + `pnpm install --offline`，要么干脆用「连跑 5 次」替代；④ **不要用 `tail -N` 判断通过与否**，要 grep `^Passed \d+ browser checks` 或 `AssertionError` 这样的确定性标记；⑤ 结论写进交付说明：跑了什么、哪条失败、是否为本轮新增、定位到的具体断言。

**模式一百一十八：新增枚举变体时，「序列化拼写」和「业务字符串」是两份契约——`rename_all` 与 `as_str()` 一致只是历史巧合，不是保证。**

> 例（#168）：`GatewayCliKey` 用 `rename_all = "snake_case"`，同时有手写的 `as_str()`。Claude/Codex/Grok/Gemini 这些变体两种拼写恰好相同，所以从没暴露过。新加的 `Workbuddy`/`Codebuddy` 需要 `workbuddy_desktop`/`codebuddy_cli`（必须与原生用量行的 `workbuddy`/`codebuddy` 区分），于是 `as_str()` 写成 `*_desktop`/`*_cli`、serde 仍出 `workbuddy`/`codebuddy`。后果链条：前端从 `proxy_gateway_supported_cli_keys` 拿到 `as_str()` 值 → 用它当 `app_configs` 的键写回 → `HashMap<GatewayCliKey, _>` 反序列化失败 → `settings_from_value` 的 `unwrap_or_else(|_| Default)` 把**整份**网关设置重置成默认值。用户感知只是「计费倍率没保存」，实际端口、日志保留期、超时、重试全丢了。
>
> **动作**：① 给任何与 `as_str()` 拼写不同的变体写显式 `#[serde(rename = "<as_str>", alias = "<旧/自然拼写>")]`，`alias` 保证存量行还能读；② 判断标准是**「这个枚举的值会不会被某个接口的返回值原样写回同一份存储？」** 会 → 两份拼写必须相同；③ 回归要断言**无关字段没被重置**（如 `listen_port` 仍是持久化值），只断言新字段读到了是漏测——`unwrap_or_default` 这个兜底会把「解析失败」伪装成「全是默认值」；④ 同类风险点还有 `GatewayUsageTool`、`SessionTool` 等带 `as_str()` + serde 的枚举，新增变体时逐个自查。

**模式一百一十九：带 applied 语义的提示词是「恢复后要重建」的状态——漏接 reapply，备份恢复会静默丢掉 `CODEBUDDY.md` / `AGENTS.md` 的生效内容。**

> 例（#169）：CodeBuddy 的全局提示词和 Hermes/ZCode 一样是 DB-backed applied preset（`codebuddy_prompt_config.is_applied`）。但 `reapply_applied_runtime_after_restore` 的标签清单里没有 `codebuddy`。恢复后 DB 记录回来了、`CODEBUDDY.md` 却停在恢复前的旧内容（备份包里没有 runtime 文件时），用户看到「配置都在、提示词没生效」。同时 `wsl_module_for_reapply_label` 缺 arm 会让即便有 reapply 也不触发后续 WSL 同步。
>
> **动作**：① 接入一个带 `is_applied` 提示词表的 CLI 时，同步查 `reapply_applied_runtime.rs` 三处：`reapply_<cli>` 函数、`reapply_applied_runtime_after_restore` 里的调用、`wsl_module_for_reapply_label` 的 arm；② **reapply 路径必须用 `*_without_events` 变体**——启动恢复期间 emit `config-changed` 会被托盘重建、WSL/SSH 自动同步当成一次用户编辑（新模块要顺手补这个无事件函数，参照 `apply_hermes_prompt_config_internal_without_events` / `apply_dsh_prompt_config_internal_without_events`）；③ **没有 applied 记录的 CLI（如 WorkBuddy 的身份文件）不要加 reapply arm**，否则 `changed_modules` 会声称同步了一个根本没发生的变化，让后续 WSL 跳过判断失真；④ 判据——**「这份配置在备份包里，还是在 runtime 文件里？」** 在 runtime 文件里 → 不需要 reapply；在 DB 里且有 applied 标记 → 必须 reapply。

**模式一百一十二：可隐藏的 UI 必须自带「恢复显示」的路——否则那个开关是单程票。**
> 例（#162）：两页都接了 `SectionSidebarLayout`，却没接 `SidebarSettingsModal`（全仓 14 个页面都有）。用户隐藏侧栏后再无入口。
>
> **动作**：① 新增 `SectionSidebarLayout` 页面时，同一条命令同时查 `SidebarSettingsModal` 覆盖率；② 任何「隐藏 X」的开关，问一句「关掉之后怎么打开」；③ 判据——**不可逆的界面状态比缺失的功能更糟**，缺功能用户会找，界面卡死用户只能重装。

**模式一百零八：coding 页面外壳是「全家 16/16 都在用」的约定，不是可选装饰——新页面不在 `pageRhythmFixture` 名单里就等于没被量过。**
> 例（#158）：WorkBuddy / CodeBuddy 第一版直接 `<Spin><div style={{padding:16}}>`，没有 `SectionSidebarLayout`、没有 `.pageContent { display: block }`、路由也没写 `contentPadding: 'compact'`。三处都不报错。
>
> **动作**：① 新建页面先 `grep -c SectionSidebarLayout web/features/coding/*/pages/*.tsx`——全仓 16 个页面都有，照做；② 容器只写 `display: block`，间距由区块 `margin-bottom: 16px` 承担；③ 每条 coding 路由都要 `contentPadding: 'compact'`；④ **把新页加进 `PageRhythmFixture` 的 `PAGES`**——守卫只量名单内的页面，名单外等于没检查。

**模式一百零九：预览必须读原文——任何 parse → serialize 往返都会把用户文件改成「一份不存在的文件」。**
> 例（#159）：`models.json` 预览用 `JSON.stringify` 现场拼视图，丢掉无 URL 条目、重排字段、不含产品自有键。
>
> **动作**：① 预览的数据源是 `read_to_string`，不是运行时视图；② 后端加一个只读命令（如 `read_workbuddy_models_file`）；③ 判据——**「`read_to_string` 能不能直接得到这份内容？」** 能就不要再序列化一遍。

**模式一百一十：i18n 动态前缀保护表漏登记时，`i18n:check` 是「通过」的——它只在补登记之后才暴露全部缺口。**
> 例（#160）：`useRootDirectoryConfig` 的保护表没有 `workbuddy` / `codebuddy`，补上后一次性爆出 72 条缺失（18 key × 2 语言 × 2 工具）。
>
> **动作**：① 用任何会拼 key 的共享组件前**先登记前缀**；② 补完必须重跑 `i18n:check`，红了就按参照模块措辞逐条补齐；③ 绝不用空串或复制工具名占位——那会让弹窗读出无意义文案而守卫全绿。

**模式一百一十一：守卫看不见的 DRY 就是不可验证的代码——不变量靠 grep 时，被检查的路径必须写成字面量。**
> 例（#161）：备份打包段用 `format!("external-configs/{tool}/")`，§12.1 的字面量核对完全看不见它。
>
> **动作**：① 「打包 ↔ 恢复」两个集合逐个工具直写 `"external-configs/<tool>/"`，宁可重复几行；② 改完立刻跑核对命令、输出必须为空；③ 判据——**「这条不变量有没有机械守卫？守卫抓的是哪种写法？」** 有守卫就按守卫看得见的写法写。

**模式九十一：网格里的「高度/间距」是整行的事，不是这张卡片的事——断言要拿形态不同的两张比。**
> 例（#142）：没有模型行的卡片矮 17px，网格把它拉高，差额落在分隔线上方，看起来像「这张卡片的间距错了」。逐张看数值都对，只有两张并排才看得出。
>
> **动作**：① 判据——**「同一行里形态不同的卡片，高度会不会跟着邻居变？」** 会 → 把「有的有、有的没有」的内容行按自身尺寸预留掉（`min-height`），**内边距要算进去**（`border-box`：17px 行 + 6px 下内边距 = 23px）；② 回归断言要**成对比较**（有模型行 vs 没有 → 等高；两张卡片「第二行到分隔线」的距离相同），而不是断言某一个绝对值；③ 间距类改动必须量几何，不能读代码（同 13.1 模式六十三）。

**模式九十二：`visibility` 隐藏的元素不占「视觉」但占「布局」——窄宿主里它会把邻居挤没。**
> 例（#143）：hover 才显形的动作按钮保留宽度（这是对的，避免 hover 抖动），但在 478px 的侧栏里它们吃掉 362px，模型名被折成四行；同一组件在 1176px 的列表卡片上完全正常。
>
> **动作**：① 判据——**「把容器缩到最窄的宿主，这行的文字还剩多少？」** 共享行组件（`ModelItem` / `ProviderCardMetaEntry` 这类）要在**每个宿主宽度**上看一眼，不能只在宽的那个上验收；② 修法是**给内容最小份额 + 让次要区整组换行**（`flex: 1 1 220px` + `flex-wrap: wrap`），不是把动作改成 `display: none`——那会重新引入 hover 抖动（DESIGN.md 禁止布局跳动）；③ 换行条件必须只依赖恒定尺寸，**不能依赖 hover 状态**，否则指针一动就重排。

**模式八十八：复用一个通用文案之前，先问它会不会在同一个屏幕上出现两次、指两件事。**
> 例（#139）：`common.model.title`（模型列表）既是 Codex 侧栏某个分区的标题，又是它下面模型目录区块自己的标题——一个侧栏里两个「模型列表」，其中一个只有一行。
>
> **动作**：① 判据——**「这段文案会不会在同一屏出现两次？」** 会 → 它在这个位置已经是专名，另起 key（按层命名，`<cli>.model.mappingTitle`），别复用 `common.*`；② 加了新 key 后 `grep` 一遍它的渲染点，确认没有第三处顺手复用；③ 选词与兄弟模块平行（Claude 的同一分区叫「模型映射」，Codex 叫「模型配置」，一眼看出是同一种东西、不是两份清单）。

**模式八十九：夹具的命令桩忽略参数 → 夹具渲染出一个真实应用不会产生的页面，而所有正向断言照样全绿。**
> 例（#140）：`list_codex_official_accounts` 桩不按 `providerId` 过滤，于是每个 provider 都拿到同一批账号（自定义 provider 的侧栏长出账号块 + 「仍有历史官方账号」提示）。53 项断言全过——它们只问「官方 provider 有没有账号块」，没有一条问「别人该不该有」。
>
> **动作**：① 桩按参数作答，复现后端语义（真后端不会跨 provider 返回同一批记录）；② **每条「有」的断言配一条「不该有」的反向断言**——反向断言是桩这类错误的唯一探测器，正反成对也是「迁移没漏功能」这类问题的通用验收（同 #114/#115）；③ 跑完之后**看截图**：断言是抽样，截图是全貌，几轮下来这条已被证明是发现「断言查不到的东西」的最快路径。

**模式九十：守卫的作用域清单与代码搬迁必须是同一个改动——假红和假绿是同一缺陷的两个方向。**
> 例（#141）：`modelSection.*` 的渲染点搬进共享 `ProviderModelSection.tsx` 后，`verify-provider-card-layout.mjs` 按旧的「渲染点文件表」判断，把 7 个活着的 prop 报成死的。反方向的 #124 / #125 是同一件事：清单里漏了**违规者**（假绿，三份手写区块从未被扫出）或漏了**合法者**（假红，本轮）。
>
> **动作**：① 守卫的清单（后缀集合、文件表、`EXEMPT_FILES`、`PENDING_MIGRATION`）在**移动代码的同一次改动里**更新；② 判据——**「被扫描的文件集，还覆盖这条规则关心的全部渲染点吗？」**；③ 清单里每一项都要写明它为什么在（该脚本的注释体系就是这样），下一个人才判断得出该不该跟着改。

**模式八十：教训 / 模式 / 清单 / 接入步骤是四种产物，一种不能顶替另一种——按「读者的问题」分流。** 同一个改变会同时在几处留下痕迹，但它们回答的是**不同的问题**：13 节教训回答「我们踩过什么坑」，13.1 模式回答「这类坑的规律」，附录 B 台账回答「这一项传了没有」，**正文章节（4.x）回答「我要接一个新 CLI，第一步做什么」**。前三样写满、正文那节空着，文档看起来「很全」（新增十几条教训、三个浏览器套件、两条守卫），但下一个接入者仍拼不出流程——他得从教训 #120–#130 里反推步骤。
> 例（#131）：方案写了「新增「官方账号区块」的接入步骤与最小验证（与卡片三样式那节并列）」，实际只交付了教训 / 模式 / 清单 bullet / 守卫表行；用户问「sop 里加上官方账号的处理方法了吗」才发现 §4.2 下没有这一节、附录 A.1 也没有这一行。
>
> **动作**：① 收尾时把方案里的**产物清单**逐条 `grep` 定位——看它出现在「正文的小节标题」还是只在「表格与教训」里（`grep -n "官方账号区块" docs/*.md`）；② 正文那一节要自足到**照着做就能接上**：判据（怎么选宿主）、字段表（每个字段填什么、从哪取）、props 表（传哪个渲染哪个）、后端不变量（前端补不回来的那条）、最小验证命令；③ 教训与正文**双向引用**，别把步骤抄成两份——正文是流程，教训是为什么。

**模式六十八：时间维度的交互（滚动 / 动画 / 过渡）只能断言「用户看见的那一刻」，不能断言「最后状态」。** 这类缺陷在截图里往往看不出来：卡片确实滚到位了，动画也确实跑过了——只是两者**时间上错开**。同理，「断言存在某个属性」也拦不住它：动画的 0% 可以写成 spread 0，那是「有 box-shadow、没有画面」。
> 例（#118、#119）：① 定位闪光原本点击即闪，而平滑滚动要几百毫秒 → 用户到达时看到的是一张毫无标记的卡片。改为等卡片停稳再开始（rAF 轮询位置，两帧不动 / 1.2s 兜底），keyframes 从「0 → 3px」改成「3px 满绘 → 淡出」，断言改成「**到达那一刻**环的 spread ≠ 0」。② 夹具缺 `main`（应用把 `html/body/#root` 锁 `overflow:hidden`，滚动容器是 `MainLayout` 的 `main`）会让位置类断言在「一个不可能滚动的页面」上跑；同一夹具的 query 自带 `&` 时还必须 `encodeURIComponent(runId)`，否则 run id 被 `URLSearchParams` 拆坏、表现为 10s 超时。
>
> **动作**：涉及滚动 / 动画 / 过渡时——① 用真浏览器（同 #112 / 模式六十）；② 断言点放在「元素已进入视口 / 用户看得见」之后，而不是点击之后立刻取样；③ 断言要有**画面证据**（spread、可见性、位置），不是「某个属性存在」；④ 先确认夹具里存在与应用一致的滚动容器。

**反向模式（不是坑但容易误判）：进程 stales。**
> 排查 UI 异常前，**先确认运行中的进程是当前构建**。曾出现：前端 Vite 热更新到最新代码，而后端进程是 14 小时前启动的旧二进制，DB 迁移也没跑 → 表现为「后端命令不存在/报错」，实际是代码根本没生效。
>
> **快速核对**：进程启动时间 vs `ai-toolbox.exe` 编译时间；`PRAGMA user_version` vs `TARGET_SCHEMA_VERSION`。

---

## 附录 A：ZCode 集成改动面（实证样本）

一个「最小完整集成」实际触达的文件分布（10 提交 / 63 文件）：

- **后端 30 个**：`tauri/src/coding/zcode/`（9 个模块文件）+ `runtime_location.rs` + `reapply_applied_runtime.rs` + `session_manager/` + `tools/builtin.rs` + `settings/{types,adapter}.rs` + `settings/backup/{utils,restore}.rs` + `tray.rs` + `lib.rs` + `db/{schema,migrations}.rs` + `wsl/`+`ssh/`（mcp_sync + commands）+ `mcp/`（3 个）
- **前端 33 个**：`web/features/coding/zcode/`（7 个）+ `web/services/`（3 个）+ `web/types/` + `constants/modules.tsx` + `app/routeConfig.ts` + `MainLayout` + `toolIcon` + `features/settings/`（7 个）+ i18n（2 个）

> ZCode **未做**：Gateway 接管、`CliManualPathSetting`（页面有「更多选项」但只是配置弹窗；ZCode 模块本身不 spawn CLI，故无 cli_resolver 需求）、`docs/plan` 文档。这些是可选阶段——反过来说，**如果新工具需要调用 CLI**，cli_resolver 与手动路径入口就是必需项。

### A.1 共享组件对照表

**参照 CLI 是 Codex，不是 ZCode**（见 4.0.1）。下表只说明「哪个关注点该用哪个共享组件」，具体形态必须按 §4.0.2 的清单对着 Codex 逐项核对。

| 关注点 | 用共享组件 | 备注 |
|--------|-----------|------|
| 页面头部 | `CodingPageHeader` | 无文案 props；所有标签走 `common.*` |
| 供应商列表 | `ProviderListSection` | `emptyTextHint` 是唯一的空态扩展点 |
| 供应商卡片 | 模块内卡片（参照 `CodexProviderCard`） | 字段区差异大，不强行用通用 `ProviderCard` |
| 模型列表 | `ModelListSection` | **枚举全部可选 prop 逐个决定传/不传**（4.0.2-F） |
| 供应商表单 | `ProviderFormSections` | 计费 / 自定义头 / 模型重写三块**只对网关 CLI 显示**（4.0.3） |
| 模型表单 | `ModelFormModal` | 差异走 `messageOverrides`（值是**已翻译文本**，不是 key） |
| 通用配置编辑 | `JsonEditor` | `onChange(parsed, isValid)` + `onRawChange(raw)` 配合使用 |
| 官方账号区块 | `shared/officialAccounts` | **有官方 OAuth 登录才有**。宿主由调用方挑：`embedded`（官方渠道就是一条 provider 行，挂在它卡片 `footer`）/ `standalone`（自持卡片，且必须是可排序的列表成员）。登录入口是 `loginAction` 插槽。见 §4.2.9 |

**「支持自定义模型」形态的供应商表单不含模型编辑**：模型在卡片上增删改，保存时要把已存模型原样带回去，否则会清空模型目录：

```tsx
// Models are edited on the provider card, not here. Reuse whatever the
// stored provider already has so saving the form does not wipe the catalog.
const existingModels = provider
  ? (parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [])
  : [];
```

**ZCode 有三个配置文件**（新 CLI 若也是多文件，照此分工）：

| 文件 | 内容 | 管理方式 |
|------|------|---------|
| `v2/provider_config.json` | 供应商 + 模型目录 | 结构化 UI（provider/模型 CRUD） |
| `cli/config.json` | MCP 服务器 / hooks / plugins / 权限 | `JsonEditor` 原文编辑（`ZcodeCommonConfigModal`） |
| `v2/setting.json` | 桌面端偏好 | 只读 |

> `cli/config.json` 里含明文凭据（Tavily / Firecrawl / GitHub token / Context7 key）。做演示或截图时注意遮挡。

### A.2 ZCode 的已知遗留

| 项 | 状态 |
|----|------|
| ~~`detection.rs` 4 个 `*_with_db*` 白名单漏 zcode~~ | ✅ 已修（2026-10-06）：白名单反转，见 7.2 |
| ~~模型行缺「复制」按钮（`onCopyModel` 未传）~~ | ✅ 已修（2026-10-06），见 4.0.2-F / 附录 B.1 |
| ~~页面缺「预览配置」（`onPreviewConfig` 未传）~~ | ✅ 已修（2026-10-06），见 4.1 / 附录 B.1 |
| ~~页面缺提示块与导入入口（`hint` / `footer` 未传）~~ | ✅ 已修（2026-10-06），三个导入源全部接上，见 4.2.1 |
| ~~「自定义根目录下 MCP/Skills 路径」无回归测试~~ | ✅ 已修（2026-10-06）：`tauri/tests/coding/tools/detection_paths.rs`，见 12.5 |
| ZCode 不在 `GatewayCliKey::supported_mvp()` | 设计如此（模块不 spawn CLI），故无 Gateway 接管与 cli_resolver |

---

## 附录 B：可选项台账（4.0.2-F 的核对底表）

**用法**：新增 CLI 时把下面三张表的每一行抄进 PR 描述，填「传 / 有意不传 + 理由」。这张表是**穷举**的——组件源码里改了 prop，这里要同步改（同步检查方式：`grep "?: " <组件>` 对比本表）。

「ZCode 现状」一列是实证，**用来提醒这些都是真会漏的**，不是「应该照抄 ZCode」。

### B.1 组件可选 prop

**`ProviderListSection`**（`shared/ProviderListSection.tsx`）

| prop | 渲染出的东西 | ZCode 现状 |
|------|-------------|-----------|
| `onBatchTest` / `batchTesting` | 「一键测试」 | ✅ 传 |
| `onOpenCommonConfig` | 「通用配置」 | ✅ 传 |
| `locateProviderId` | 标题右侧「定位」（滚到已应用的卡片并闪光）；**不传则完全没有这个按钮** | ✅ 传 |
| `headerExtra` | 标题旁插槽（Gateway 胶囊） | 有意不传（非网关 CLI） |
| `emptyTextHint` | 空态追加的一句补充 | 有意不传（无专属导入来源） |
| `hint` | 工具栏下方提示块 | ❌ **漏传** |
| `footer` | 底部导入按钮组（**无条件渲染**，与 `alwaysVisible` 不同） | ❌ **漏传** |
| `viewMode` / `onViewModeChange` | 布局切换（两个都传才渲染） | ✅ 传 |
| `alwaysVisible` | 空态 / 搜索无结果时仍要显示的列表成员 | 传（官方账号卡片与供应商同时进 `children`） |

> **`alwaysVisible` vs `footer`——选错会静默丢内容**：`alwaysVisible` 只在列表为空 / 搜索无结果时渲染（用于「空列表时也要能看到的区块」），`footer` 无条件渲染（用于「无论列表如何都要显示的区块」）。曾把「两种状态都要显示」的区块放进 `alwaysVisible`，于是**有供应商时整块消失**。判据：**「列表非空时它还需要显示吗？」** 需要 → `footer`。
>
> **第三个选项常被忘掉：两个都传。** 少见的区块（与供应商卡片平级的列表成员，如官方账号卡片）希望「正常时排在列表里的指定位置、空列表时也要能看到」——那时它**同时**进 `children`（槽位内渲染）和 `alwaysVisible`（兜底）。两个分支互斥，不会重复渲染。只在完全不在意外观次序时才退化成 `footer` 一个。
>
> **外壳插槽不是「有个地方放就行」**：`headerExtra`（标题旁）/ `footer`（列表下方）都只改变**位置**，不会让一个东西变成别的东西。若它本该是列表成员（要能拖动、要与卡片并排），唯一的正确落点是 `children` + `alwaysVisible`——放进工具栏或页底，用户会当它是坏掉的卡片。

**`ModelListSection`**（`shared/ModelListSection.tsx`）

| prop | 渲染出的东西 | ZCode 现状 |
|------|-------------|-----------|
| `onToggleBatchDeleteMode` / `onBatchDelete` | 「批量删除」入口与执行 | ✅ 传 |
| `selectionMode` / `selectedIds` / `onToggleSelection` | 多选状态 | ✅ 传 |
| `onTest`（+ `testDisabled` / `testDisabledTooltip`） | 「模型测试」 | ✅ 传 |
| `onFetchModels`（+ `fetchDisabled` / `fetchDisabledTooltip`） | 「获取模型」 | ✅ 传 |
| `onAddModel` | 「添加模型」 | ✅ 传 |
| `onEditModel` | 行级「编辑」 | ✅ 传 |
| **`onCopyModel`** | **行级「复制」** | ❌ **漏传** |
| `onDeleteModel` | 行级「删除」 | ✅ 传 |
| `onSetPrimaryModel` | 行级「设为主模型」+「当前主模型」标签 | ✅ 传 |
| `renderModelExtraActions` | 行级额外操作（Codex 的「设为自动审批模型」） | 有意不传（无此概念） |
| `aboveList` | 工具栏下方、列表上方（Codex 的自动审批行） | 有意不传（同上） |
| `modelsDraggable` / `onReorderModels` | 拖拽排序 | ✅ 传 |
| `rowKeyOf` | 行标识解析器 | ✅ 传（`model.id`） |
| `appearance` | `'card'`（默认，折叠）/ `'detail'`（明细侧栏，常开 + 侧栏口径的行） | 不传（默认 `'card'`） |
| `transparentRows` / `className` / `bodyStyle` | 样式适配（**缩进不要写在这里**，见 §4.2.3） | 传 `transparentRows` |

**`CodingPageHeader`**（`shared/CodingPageHeader.tsx`）

| prop | 渲染出的东西 | ZCode 现状 |
|------|-------------|-----------|
| `docsUrl` | 「官方文档」 | ✅ 传 |
| **`onPreviewConfig`** | **「预览配置」** | ❌ **漏传** |
| `configPathKind` | `'file'`（默认）/ `'directory'`：决定标签取 `common.configPath`「配置文件路径」还是 `common.configDir`「配置目录路径」 | 不传（ZCode 显示的是 `v2/provider_config.json`，是文件） |
| `onCustomizeConfig` / `customizeConfigDisabled` / `customizeConfigDisabledReason` | 「自定义配置目录」；禁用时悬停说明原因 | ✅ 传 |
| `onOpenFolder` | 「打开文件夹」 | ✅ 传 |
| `onRefresh` | 「刷新配置」 | ✅ 传 |
| `onMoreOptions` | 「更多选项」 | ✅ 传 |
| `extraActions` | 路径行追加文字按钮 | 有意不传（无「打开 Web UI」类入口） |

> **没有 `hint` prop**（2026-10-07 删除）：页面级提示块统一放 `ProviderListSection` 的 `hint`。

**`providerCardVariants`（`shared/providerCardVariants/types.ts`）** —— 三种样式共用同一套 props；样式只决定**位置**，不决定**内容**。CLI 侧的 `*ProviderCard.tsx` 是薄映射层（守卫：`pnpm run test:provider-card-layout`）。

| prop | 渲染出的东西 | 现有消费方 |
|------|-------------|-----------|
| `provider` | 名称 + baseUrl（名称链到端点 origin） | 全部 |
| `providerState.isDisabled` / `onToggleDisabled` | 已禁用态；启用开关（`onToggleDisabled` 不传则菜单里没有开关）。回调收到的是**新的启用状态**（同 antd `Switch.onChange`） | claudecode、codex、kimi、zcode、opencode、geminicli、grok、claudedesktop（omo_native 没有单渠道启用概念，不传） |
| `providerState.draggable` / `sortableId` | 卡片级拖拽把手（**不是** `modelSection.*`）；拖拽不可用时映射层传 `dragDisabled`，把手整个消失 | 14 个有供应商列表的 CLI，但**只有在该行有存储位置时才为真**：pi 限 `models_json` 来源、oh_my_pi 的 Codex 订阅卡片与 dsh 的内置目录渠道都没有把手 |
| `providerState.accent` | 卡片外框高亮：`applied` / `gatewayPrimary` | claudecode、codex、kimi、geminicli、grok、claudedesktop（`gatewayPrimary` 优先于 `applied`） |
| `providerState.connectivityStatus` | 名称左侧状态点 | claudecode、claudedesktop、codex、grok、kimi、omo_native、opencode、zcode（geminicli 没有连通性测试功能，不是「入口在别处」） |
| `providerState.selectable` / `selected` / `onSelectChange` | 多选复选框（取代拖拽手柄） | 全部 |
| `actions.onEdit` / `onCopy` / `onShare` / `onDelete` | 头部/菜单里的四个标准动作 | 全部 |
| `actions.deleteDisabledReason` / `deleteConfirm` | 删除的禁用原因 / 内置二次确认 | zcode（`deleteConfirm: false`，自己确认）、omo_native（`deleteDisabledReason`＝「默认渠道不可删」+ `deleteConfirm: false`）、opencode（`deleteConfirm: true` + 默认模型渠道 `deleteDisabledReason`；**详情侧栏的删除按钮要自己弹确认**，因为侧栏 footer 不是 `Popconfirm` 的宿主，且必须同样遵守 `deleteDisabledReason`） |
| **`actions.primaryAction`** | 头部文字链「应用」（含 `locked` 置灰态） | claudecode、codex、kimi、geminicli、grok、claudedesktop |
| **`actions.gatewayActions`** | 主操作**之前**的网关按钮组（代理/恢复直连/切换主渠道） | claudecode、codex、kimi、geminicli、grok、claudedesktop |
| `actions.enabledStateLabel` | 「更多」菜单里启用开关的副标题 | claudecode、codex、kimi、geminicli、grok、claudedesktop |
| `namePrefix` | 名称**之前**的节点（官方卡片的 🔗）；`nameTags` 只管名称之后 | codex、kimi、geminicli、grok、zcode（官方账号卡片） |
| `nameTags` | 名称右侧徽章（已应用/官方/代理/优先级/官方账号计数/来源标签） | claudecode、claudedesktop、codex、dsh（内置目录 + `apiKeyEnv`）、geminicli、grok、kimi、oh_my_pi、pi、zcode |
| `metaEntries` | 第二行（`text` / `code` / `tag` 有序项；`tag` 渲染成真 `Tag`） | claudecode、claudedesktop、codex、geminicli、grok、kimi |
| `inlineActions` | 第二行末尾（连通性测试、CLI 启动）；有 `inlineActions` 时这一行**不再要求**有 `metaEntries` | claudecode、codex、kimi、claudedesktop。**Grok 刻意不传**：模型工具栏已是唯一入口，再加就是同一张卡片说两遍 |
| `footer` | **头部两栏之下**、模型区之上（官方账号折叠区、只读模型目录、备注）；必须跨满卡片整宽 | codex / geminicli / grok / kimi（官方账号折叠区）、opencode（`auth.json` 合并的官方模型只读块）、claudecode / claudedesktop（备注） |
| `modelSection.*` | 见下方 `ModelListSection` 表 | codex、dsh、grok、hermes、kimi、oh_my_pi、omo_native、openclaw、opencode、pi、zcode（均经 `ProviderModelSection` 触达） |

> **加新 prop 的门槛**：先确认**至少两个**调用方需要它（判据与实例见 §4.2.1 的「样式能力不够时怎么办」）。**零消费方的 prop 当场删掉，不要留着「以后可能用到」**：2026-10-20 已删除 `actions.extraActions`（三张样式卡片都有渲染点，但九个映射层迁移完后零消费方；旧 bespoke 卡片的头部按钮已全部随迁移移除）；2026-10-07 已删除 `metaEntries` 的 `kind: 'id'` / `'sdk'`（OpenCode 式自己按 `provider.id` / `sdkName` / `baseUrl` 拼第二行，另两种样式由调用方给 `metaEntries`）与 `ProviderCardModels.modelSourceTag`（只有 `components/common/ProviderCard` 渲染它，那是另一条链）。查零消费方的命令：
> ```bash
> # 逐个候选 prop 数消费方；输出 0 或只在 shared/ 内部出现 → 是死 prop
> for prop in modelSourceTag officialModels onToggleModelDisabled; do
>   printf "%-26s %s\n" "$prop" "$(grep -rn "$prop" web/features/coding --include=*.tsx | grep -v providerCardVariants/ | wc -l)"
> done
> ```

**`providerCardView`（`shared/providerCardView/`）** —— 卡片布局（§4.2.2.1 是接入步骤）。三个组件，props 都只有「传了才渲染」的可选项：

| prop | 渲染出的东西 | 现有消费方 |
|------|-------------|-----------|
| `ProviderListSection.viewMode` / `onViewModeChange` | 工具栏的布局切换按钮；**两个都传才渲染**（见上表 `ProviderListSection` 段） | 14 个有供应商列表的 CLI 全部（claudecode、claudedesktop、codex、dsh、geminicli、grok、hermes、kimi、oh_my_pi、omo_native、openclaw、opencode、pi、zcode） |
| `ProviderCompactCard.subtitle` | 第二行 `Label: value` 对，空数组/不传即整行不渲染。**只说一件事**：普通 provider 说模型，官方渠道说「当前登录: <账号>」 | claudecode（全部角色模型）、claudedesktop（角色路由）、codex（主模型 / 官方渠道的登录）、geminicli（主模型 / 当前登录）、grok（主模型 / 当前登录）、kimi、zcode（官方账号成员）、opencode / omo_native / pi / oh_my_pi / dsh（默认模型，仅当该渠道拥有全局默认时；前面还有裸 npm 包名） |
| `ProviderCompactCard.namePrefix` | 名称**之前**的节点（官方卡片的 🔗；`nameTags` 只管名称之后） | codex、kimi、geminicli、grok、zcode（官方账号格子） |
| `ProviderCompactCard.launchAction` / `extraMenuItems` | footer 左侧的启动按钮；「更多」菜单里插在「分享」后的额外项 | claudecode（启动 + 模型测试）、codex / kimi / grok / claudedesktop / omo_native / opencode（模型测试） |
| `ProviderCompactCard.detail` | 这张格子的详情侧栏：`{ title, statusTags, primaryActions, secondaryActions, sections }`。**传了才有点击目标**（不传 = 卡片不可点，也不画 `data-provider-card-clickable`），**抽屉与面板由卡片自己渲染**（2026-10-14 起：四个映射层曾各写一份逐字相同的 `<Drawer>`，#151） | 14 个有供应商列表的 CLI 全部 |
| `ProviderCompactCard.actionPlacement` | 那组动作挂哪一行：`'footer'`（默认，底部隔断线下）或 `'subtitle'`（第二行末尾，`margin-left: auto`）。**两种形态渲染同一份动作节点**，只是位置不同；`'subtitle'` 时动作无条件渲染（第二行本身才是条件渲染的） | codex、opencode、geminicli、grok、kimi、omo_native（`subtitle`）；claudecode / claudedesktop 用默认 `footer`（动作多）。ZCode 的映射层与官方账号格子也传 `subtitle` |
| `ProviderCompactCard.cellControl` | 这张格子自己的控件，落在动作槽里、**主操作之前**（别的 CLI 放 应用 / 网关代理 的位置）。**插槽不是布尔**——卡片不决定一个 provider 的状态控件是什么，它的 CLI 决定；**传了它，「更多」菜单里那条启用项就让出来**（同一控件不在同一张卡片上说两遍） | zcode、opencode（`ProviderEnabledSwitch`；zcode 2026-10-15，opencode 2026-10-19） |
| `ProviderDetailPanel.title` / `statusTags` / `primaryActions` / `secondaryActions` | 侧栏的标题行与两排动作；**动作由映射层传同一份渲染结果**（卡片与侧栏因此不可能提供不同的能力） | title/sections 九家全部（含 `ZcodeOfficialAccountCard` 的侧栏）；`statusTags` 除 omo_native 外都传；`primaryActions` 仅六家有「应用/网关」语义的传（claudecode、claudedesktop、codex、geminicli、grok、kimi） |
| `ProviderDetailPanel.sections` | 侧栏正文的分区数组。`fields` 的 `emptyText` 有无决定「缺值显示占位」还是「整行消失」；`copyable` / `elideMiddle` 是连接字段的两个开关；`content` 给自带标题的组件（模型目录、官方账号块） | claudecode、claudedesktop、codex、geminicli、grok、kimi、omo_native、opencode、zcode（含官方账号格子） |

> **侧栏的分区标题用专名**：同一屏里出现两次、指两件事的文案必须另起 key（Codex 的模型绑定区叫「模型配置」而不是 `common.model.title`＝「模型列表」，后者是它下面目录区的标题，见 13 节 #139 / 模式八十八）。

**`ProviderFormSections`**（`shared/providerConfig/ProviderFormSections.tsx`）

| prop | 渲染出的东西 | ZCode 现状 |
|------|-------------|-----------|
| `advancedSettings` | 高级设置分区 | —（ZCode 表单结构不同） |
| `modelMapping` | 模型映射分区 | 有意不传（模型在卡片上管理） |
| `showBilling` + `billing` / `onBillingChange` | 计费分区 | `false`（非网关 CLI） |
| `showCustomHeaders` + `customHeaders` / `onCustomHeadersChange` | 自定义请求头分区 | `false`（同上） |
| `showModelRewrites` + `modelRewrites` / `onModelRewritesChange` | 模型改写分区 | `false`（同上） |
| `notesRows` / `notesResetKey` | 备注区 | 传 |
| `showNotes` | 是否渲染备注区（默认 `true`） | 不传（有 notes 字段）。**OpenCode 传 `false`**：provider 记录没有 `notes` 字段，渲染出来就是「写了也不保存」的假字段 |

**`ModelFormModal`**（`components/common/ModelFormModal/index.tsx`）—— 12 个 `show*` 开关 + 3 个校验强度开关，**逐个决定**：

`showOptions` / `showVariants` / `showModalities` / `showInputTypes` / `showApi` / `showReasoning` / `showThinkingLevel` / `showThinkingLevelMap` / `showOmpThinking` / `showCompat` / `showCost` / `showExtraParams`（以上 12 个 `show*`），`limitRequired` / `requireCompleteLimitPair` / `nameRequired`（校验强度），加 `apiOptions` / `thinkingLevelOptions` / `npmType` / `toolName` / `messageOverrides` / `onDuplicateId` / `existingIds` / `width`。

> **用模块内自建弹窗的 CLI**（codex / grok / kimi / openclaw / zcode 等）不适用这张 props 表，但**要做的决定是一样的**：逐字段确认「参照有我没有」「我有参照没有」，并按附录 B.3 补齐能力（尤其 **4.2.4 的预设模型选择器**）。

**`OfficialAccountsSection`**（`shared/officialAccounts/types.ts`）—— **有官方 OAuth 登录才适用**；接入步骤与最小验证见 §4.2.9。守卫：`pnpm run test:official-accounts-shared`（源码棘轮；2026-10-20 起 `PENDING_MIGRATION` 只剩 antigravity）。

| prop | 渲染出的东西 | ZCode 现状 |
|------|-------------|-----------|
| `variant` | 宿主：`embedded`（卡片内一段，带上分隔线、行缩进、纯文本空态）/ `standalone`（自持卡片，空态给插画） | `standalone`（官方登录不在 provider 表里） |
| `headingTitle` | 区块自己画的标题行 `🔗 官方账号 (n)`；**只有 `standalone` 传** | 传 |
| `listTitle` | 列表自己的标题行 `▾ 账号列表 (n)`；**每个宿主都传** | 传 |
| `hint` | 标题下方的说明句（副标题）；`standalone` 传，`embedded` 改由卡片第二行画 | 传 |
| `applyHint` | 「切换」按钮的 hover 文案 | 传 |
| `emptyText` | 空态文案 | 传 |
| `accounts` | 账号行（映射后的 `OfficialAccountRowView[]`） | 传 |
| `loginAction` | 登录入口，画在**账号列表标题行右端** | 传（下拉：两个 OAuth provider） |
| `collapsed` / `onToggleCollapsed` | 折叠能力；**两个都不传 = 不可折叠** | 传 |
| `pending` | 正在飞的那个动作（带种类，转圈落在真按下的按钮上） | 传 |
| `actionsDisabled` | 登录进行中置 true——登录与行动作改同一份凭据文件 | 传 |
| `onApply` / `onSaveLocal` / `onDelete` | 每行的切换 / 保存当前登录 / 删除 | 传 |
| `onRefresh` / `onViewDetails` | 每行的刷新（额度）/ 查看详情 | **有意不传**（ZCode / Kimi 的账号行没有这两个动作；Codex 五个全传） |
| `renderExtraActions` | CLI 专属的行内动作，渲染在「刷新」之后、「详情」之前；按钮形态与 loading 由调用方自带，按行 id 回查原始记录，返回 `null` 即该行不渲染 | 有意不传（ZCode 无此动作）；**Grok 传**（「刷新 Token」，2026-10-20） |

### B.2 页面级区块（不属于任何组件）

靠**打开参照页面逐块点**，不能靠读组件 props：

- [ ] 页面提示块（两行 `pageHint` + `pageWarning`）
- [ ] 导入入口：从 CC Switch 导入 / **导入我使用过的供应商** / 从 All API Hub 导入
- [ ] 页面级 `Alert`（未迁移提示、兼容性警告…）
- [ ] **空态与搜索无结果态**：把搜索框敲到无结果，再删光所有供应商，各看一遍——「和列表并列」的区块必须两种状态下都在（13.1 模式七）
- [ ] 会话管理面板、全局提示词区块
- [ ] 「更多选项」弹窗里的内容
- [ ] **官方账号区块**（Codex / Grok / Kimi / Gemini CLI / Antigravity / ZCode）：登录入口、账号列表、切换、保存当前登录。它在**参照 CLI 的对比里最容易被整块忽略**——因为它常被画在官方供应商卡片**内部**，而 ZCode 这类「官方登录独立于供应商表」的 CLI 必须自己另起一块。
  > **实现不要重写：用共享区块** `web/features/coding/shared/officialAccounts`（2026-10-09 抽成）。各 CLI 只写一个映射（自家账号记录 → `OfficialAccountRowView`）并挑宿主：`embedded`（官方供应商卡片内部，Codex / Kimi）或 `standalone`（自持卡片，ZCode）。登录入口走 `loginAction` 插槽——它吸收「一个按钮」与「下拉选 OAuth provider」的形态差异，不让共享组件知道一个 CLI 有几个官方 provider。守卫：`pnpm run test:official-accounts-shared`（禁止再手写行级按钮）。**接入步骤、字段 / props 台账、后端不变量与最小验证见 §4.2.9**（本节只回答「有没有这一块」）。
  >
  > **它到底是「卡片内的一段」还是「列表里的一个成员」，取决于官方登录在不在供应商表里**：Codex/Kimi 的官方渠道**就是**一条 provider 行，账号区挂在它卡片内部；ZCode 的官方登录在 `credentials.json` 里、供应商表里没有对应行，所以必须自持一张卡片。**判据是「这条官方渠道在 provider 表里有没有行」，不是观感。**
  >
  > **自持卡片的那种要在列表里能排序。** 它与供应商卡片视觉同级（同样的外壳、同样的容器），用户就会期望它能跟着拖。实现要点：占一个 sortable id、与其他卡片共用同一个 `DndContext` / `SortableContext`，位置按「它上面有几张供应商卡」记录（不是列表下标——下标会随搜索/排序模式变化）。

### B.3 能力（同 4.0.2-H）

见 4.0.2-H 的 8 项。B.1/B.2 是「组件有没有传」，B.3 是「能力有没有做」——**两层都要**：ZCode 的预设模型选择器属于 B.3（能力缺失），`onCopyModel` 属于 B.1（prop 漏传），两者在界面上都表现为「少了一块」。

---

## 何时更新本文件

- 新增 CLI 工具完成后，把新踩的坑补进第 13 节；若是**新类型**的坑，补进 13.1 的模式归纳
- 发现清单项过时或新增了硬编码清单，同步更新第 3 节并**同时**修正根 `AGENTS.md` 的对应章节
- 出现新的「编译通过但形态不对」返工，把漏掉的核对项补进 4.0.2
- **改动任何共享组件（`ProviderListSection` / `ModelListSection` / `CodingPageHeader` / `ProviderFormSections` / `ModelFormModal` / `providerCardVariants` / `shared/officialAccounts`）的 props 时，同步更新附录 B 的台账**——那份台账是穷举检查表，过期就失去意义
- **建了新的共享组件（或新的「固定 N 种」规则）时，同一任务内建机械守卫**，别只写文档：硬规则没有守卫就会漂移（13.1 模式四十六）。进度类规则用「棘轮」脚本，把未迁移的登记在案、只减不增
- 修掉一批「已知遗留」（附录 A.2）后，把对应行改成 ✅ 并把坑从 #13 表里标注为已修；**过期的「未修」记录比没有记录更有害**——它会让人以为问题还在而重复排查
- 本文件与根 `AGENTS.md` 的「Tab / Page-Key Allowlist Rules」是配套关系：后者是规则，前者是流程

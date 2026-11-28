# 插件中心 Magpie 清透柔靛风 UI 重构与渠道级自动签到规格书

> 状态说明：插件页强调色已统一为 AI Toolbox 全局主色蓝，`--pl-accent*` 现复用 `var(--ant-color-primary*)` / `var(--color-primary)` / `var(--color-bg-selected)`；下文「柔靛」色值仅作历史设计记录，运行时以 `web/features/coding/plugins/styles/pluginTheme.less` 为准。
>
> 工作目录：`D:\GitHub\ai-toolbox-plugins-ui`（分支：`feat/plugins-ui-magpie`）
> 核心约束：**视觉全面升级为「Magpie 清透柔靛风 + Unslop 4px 节奏」，但现有功能一项不可丢失；亮色、暗色、跟随系统主题均须精致协调。**

---

## 1. 视觉系统（Direction & Local Theme Tokens）

### 1.1 设计方向（Unslop）
- **Product**：AI Toolbox 插件工作台（管理 Magpie / OpenCode 兼容的 Provider 渠道、Gateway 中间件与自定义页面插件）。
- **Character**：清透、克制、精准、高密度但不拥挤（`calm, crisp, tactile`）。
- **World**：精密桌面开发控制台（参考 Magpie 的柔和靛蓝轻透感 + 现代石墨中性阶梯）。
- **Moments**：市场页的「三列清爽卡片 + 详情弹窗」、账号页的「内联短进度条 + 模型胶囊墙」、运行时页的「清爽分组设置行 + 终端日志流」。
- **Signature**：`1px` 安静细线边框 + `6px` 活状态圆点 + `tabular-nums` 等宽数字 + 严格 `4px` 间距网格（`4 / 8 / 12 / 16 / 20`），淘汰一切 `10px` 糊灰字、厚重灰阴影与刺眼荧光蓝 `#1890ff`。

### 1.2 局部 CSS 变量定义（挂在 `PluginsPage.module.less` 的 `.pluginsPage` 根节点，供全部插件子组件继承）

在 `PluginsPage.module.less` 中声明：

```less
.pluginsPage {
  /* Light theme: cool slate canvas so white cards lift cleanly without heavy shadows */
  --pl-bg: #f6f7fa;
  --pl-surface: #ffffff;
  --pl-surface-raised: #f2f4f8;
  --pl-surface-hover: #eceff5;
  --pl-border: rgba(15, 23, 42, 0.08);
  --pl-border-strong: rgba(15, 23, 42, 0.14);
  --pl-divider: rgba(15, 23, 42, 0.06);

  --pl-text-primary: #111827;
  --pl-text-secondary: #4b5563;
  --pl-text-muted: #6b7280;

  /* Soft indigo accent (Magpie feel, high contrast >= 7:1 on tint) */
  --pl-accent: #4f46e5;
  --pl-accent-hover: #4338ca;
  --pl-accent-text: #4338ca;
  --pl-accent-bg: #eef2ff;
  --pl-accent-bg-hover: #e0e7ff;
  --pl-accent-border: rgba(79, 70, 229, 0.18);

  /* Toned semantic colors */
  --pl-success: #15803d;
  --pl-success-bg: #ecfdf5;
  --pl-success-border: rgba(21, 128, 61, 0.2);
  --pl-warning: #b45309;
  --pl-warning-bg: #fffbeb;
  --pl-warning-border: rgba(180, 83, 9, 0.22);
  --pl-error: #dc2626;
  --pl-error-bg: #fef2f2;
  --pl-error-border: rgba(220, 38, 38, 0.22);

  /* Crisp minimal depth */
  --pl-shadow-card: 0 1px 2px rgba(15, 23, 42, 0.04);
  --pl-shadow-card-hover: 0 4px 12px rgba(15, 23, 42, 0.06);
  --pl-radius-card: 12px;
  --pl-radius-control: 8px;
  --pl-radius-tag: 6px;
  --pl-ease: cubic-bezier(0.22, 1, 0.36, 1);
}

:global([data-theme='dark']) .pluginsPage {
  --pl-bg: #0f1117;
  --pl-surface: #171923;
  --pl-surface-raised: #1f2230;
  --pl-surface-hover: #272b3b;
  --pl-border: rgba(255, 255, 255, 0.08);
  --pl-border-strong: rgba(255, 255, 255, 0.15);
  --pl-divider: rgba(255, 255, 255, 0.06);

  --pl-text-primary: #f1f5f9;
  --pl-text-secondary: #94a3b8;
  --pl-text-muted: #64748b;

  --pl-accent: #6366f1;
  --pl-accent-hover: #818cf8;
  --pl-accent-text: #a5b4fc;
  --pl-accent-bg: rgba(99, 102, 241, 0.16);
  --pl-accent-bg-hover: rgba(99, 102, 241, 0.24);
  --pl-accent-border: rgba(129, 140, 248, 0.26);

  --pl-success: #4ade80;
  --pl-success-bg: rgba(34, 197, 94, 0.14);
  --pl-success-border: rgba(74, 222, 128, 0.24);
  --pl-warning: #fbbf24;
  --pl-warning-bg: rgba(245, 158, 11, 0.14);
  --pl-warning-border: rgba(251, 191, 36, 0.24);
  --pl-error: #f87171;
  --pl-error-bg: rgba(248, 113, 113, 0.14);
  --pl-error-border: rgba(248, 113, 113, 0.26);

  /* Dark mode depth relies on surface steps + hairlines, not blurry shadows */
  --pl-shadow-card: none;
  --pl-shadow-card-hover: none;
}
```

> **重要**：由于 `Modal` 挂在 `document.body` Portal 里，弹窗（如插件市场详情弹窗 `PluginMarketDetailModal`）使用 `:root` / `[data-theme='dark']` 兜底或在其根容器也挂上同样的 token 类名，确保暗色模式下弹窗同样完美协调。

### 1.3 排版、间距与动效铁律
- 字体阶梯：标题 `15px–16px / 600`、分区/卡片标题 `14px / 600`、正文 `13px / 400–500`、次要说明与标签 **`11.5px–12px / 400–500`**（禁止大面积 `10px` 难认小字）；所有版本号、计数、百分比、时间、PID 均加 `font-variant-numeric: tabular-nums`。
- 间距严格 4px 倍数：`4 / 8 / 12 / 16 / 20 / 24`。
- 动效仅过渡 `color / background-color / border-color / opacity / transform`，使用 `150ms var(--pl-ease)`，禁止 `transition: all`、`hover:scale`、无意义脉冲动画。
- 所有图标统一用 `lucide-react` 线框图标（`14px` 或 `16px`，`strokeWidth={1.75}`），禁止 emoji 与 sparkle ✨ 装饰。

---

## 2. 侧边栏与整体舞台（`PluginsPage` & `PluginSidebar`）

- `.pluginsPage` 给主内容区赋予 `background: var(--pl-bg)` 与柔和内边距（`padding: 12px` 或紧贴内容区使舞台底色自然包裹右侧工作区，注意保留 `height: calc(100vh - var(--content-top-offset) - 48px)` 与现有滚动/KeepAlive 行为不变）。
- `PluginSidebar`：
  - 白底 `--pl-surface`，`border: 1px solid var(--pl-border)`，`border-radius: var(--pl-radius-card)`，`box-shadow: var(--pl-shadow-card)`；
  - 导航行高 `34px`，圆角 `8px`，选中态为柔靛淡底 `var(--pl-accent-bg)` + `var(--pl-accent-text)`（去掉生硬蓝色左条或重边框），右侧数量计数用静默等宽灰字（选中时柔靛字）；
  - 保留全部现有能力：宽 216px / 折叠 56px + `localStorage` 持久化、`< 860px` 自动折叠与浮层展开、管理项 + 自定义页面分组、无图标首字圆角方块降级、「安装插件」按钮（柔靛轻胶囊样式）、自定义页面保活与回收。

---

## 3. 新增共享功能 1：国内镜像（China Mirror）快捷开关

### 3.1 语义与持久化（零 DB 迁移）
- 复用现有后端 `PluginSettings.npm_registry`：
  - 官方源：`https://registry.npmjs.org`
  - 国内镜像：`https://registry.npmmirror.com`
- 判定逻辑：
  - `isChinaMirror(url)`：规范化去尾斜杠后等于 `https://registry.npmmirror.com`（或含 `npmmirror.com`）时为 `true`；
  - 开启开关：将 `npmRegistry` 设为 `https://registry.npmmirror.com` 并立即调 `savePluginSettings` 持久化；
  - 关闭开关：将 `npmRegistry` 设为 `https://registry.npmjs.org` 并立即持久化；
  - 若用户在「运行时」里手动填了私有 registry（非 npmjs 且非 npmmirror），开关显示关闭并在运行时页保持其自定义 URL。
- 状态共享：在 `PluginsPage` 维护一份共享的 `pluginSettings` 与 `onToggleChinaMirror(enabled)` 回调（切完后触发保存 + 轻 toast 提示，并同步更新运行时页的草稿与市场页开关，保证两处实时一致）。

---

## 4. 新增共享功能 2：自动签到改为「渠道级别」（前后端联动）

### 4.1 后端改动（`tauri/src/plugins/`）
1. **持久化存储**：
   - 在 `store` / `types.rs` 中持久化渠道级自动签到配置（按 `provider_id` 存储布尔值；推荐复用 `PluginProviderOptionRecord` 的保留字段或新增 `PluginEntityKind::ProviderConfig` / 在 `PluginSettings` 旁加 `provider_auto_checkin: HashMap<String, bool>`，选最简洁且零迁移破坏的方案——例如在 `PluginEntityKind` 或 `PluginSettings` 上加 `#[serde(default)] pub provider_auto_checkin: HashMap<String, bool>`，旧记录零迁移且与现有 `plugins_get_settings` / 独立命令无缝兼容）。
   - 提供专用 Tauri 命令 `plugins_set_provider_auto_checkin(provider_id: String, enabled: bool) -> Result<bool, String>`（注册在 `plugins::invoke_handler`，绝不塞回 `lib.rs` 根 `generate_handler!`），并在 `plugins_list_providers` 返回的每个 provider 对象上合并 `autoCheckin: bool` 字段！
2. **向后兼容迁移语义**：
   - 当某 `provider_id` 在新渠道级配置里尚未显式记录时，若该渠道下存在任意账号的旧 `PluginAccountMetaRecord.auto_checkin == true`，则**默认视作该渠道 `autoCheckin = true`**；一旦用户通过新命令显式切换（开或关），即以渠道级配置为准（同时顺手清掉旧账号级标记或让渠道级覆盖它）。
3. **后台调度改动（`background_tasks.rs`）**：
   - `checkin_due_accounts` 改为按渠道级开关判定：只要某账号所属 `provider` 的渠道级 `auto_checkin` 为 `true`，且该账号已启用（`enabled == true`）、对应插件已启用、且当日未结算（`checkin_due`），就为该账号执行签到并记录该账号各自的 `last_checkin_*` 结果；
   - 保持「无候选账号绝不启动 Bun」的懒启动不变量；
   - 补齐/更新 `background_tasks.rs` 单元测试（覆盖渠道级开启对多账号生效、停用账号跳过、渠道级关闭不启动 Bun、旧账号级兼容兜底）。

### 4.2 前端改动
- **从每个账号行移除「自动签到」Switch**；
- 在「渠道与账号」页面的 **选中渠道头部操作栏**（仅当 `selectedProvider.checkin === true` 时显示）放置渠道级「自动签到」Switch（紧邻「一键签到」按钮），切换调用 `setPluginProviderAutoCheckin(provider.id, checked)`；
- 每个账号行继续保留：该账号自己的「手动签到」小图标按钮 + 上次签到结果/时间的低调只读尾注。

---

## 5. 四大管理页精细视觉与功能规格

### 5.1 「已安装」页（`InstalledPluginsView` — 参考 Magpie 图 1）

**顶栏（sticky，background `var(--pl-bg)`）**：
- 左侧：轻量分段器（`全部 / 供应商 / 中间件 / 页面` + 安静数字计数）；
- 右侧：紧凑搜索框（`220px`）+ 排序切换（`A→Z` / `Z→A` 可选）+ 刷新按钮 + 「检查更新」（有多个可更新项时追加「全部更新」）。该页不再放 `国内镜像` 开关和「安装插件」按钮：安装入口统一在市场页、侧栏「安装插件」与空态「手动安装」。

**主体列表（取代原 2 列厚阴影卡片）**：
- 整块圆角列表容器：`background: var(--pl-surface)`，`border: 1px solid var(--pl-border)`，`border-radius: var(--pl-radius-card)`，`box-shadow: var(--pl-shadow-card)`，`overflow: hidden`；
- 列表项之间使用 `1px solid var(--pl-divider)` 分隔，hover 时轻微切至 `var(--pl-surface-raised)`；
- **单行结构（`padding: 14px 16px`，`gap: 12px`，左右对齐）**：
  - **左**：`36px × 36px` 圆角 `10px` 柔底图标（`PluginIcon`）；
  - **中（两行紧凑排版）**：
    - **第一行**：`插件名`（`14px / 600`）+ 等宽版本号 `v0.2.43`（`12px`，`var(--pl-text-muted)`）+ 来源轻标（本地/GitHub 时显示静默灰边标签）+ 若有 providers，显示柔靛淡底胶囊 `● 承载 XXX、YYY` + 静默灰字类型图标与文字（`供应商` / `中间件` / `页面` / `复合`）；
    - **第二行**：实时运行/登录摘要：
      - Provider 插件：逐个 provider 显示状态圆点（`● WorkBuddy AI: 已登录 · ○ WorkBuddy: 未登录`；已登录绿点 `var(--pl-success)`，未登录灰点 `var(--pl-text-muted)`）；
      - Middleware 插件：显示挂载钩子与调用/耗时摘要；
      - Page 插件：显示注册页面数与权限摘要；
      - 停用态整行名称轻微降对比度并显示「已停用」静默灰标。
  - **右（轻文字操作组，清晰不杂乱）**：
    - Provider 插件：柔靛轻文字按钮「登录 / 管理账号」（跳转到对应 provider 的渠道与账号页）；
    - Page 插件：「打开页面」（多页时带 Dropdown）；
    - 若需配置 options：「配置」轻文字按钮（打开 `PluginOptionsModal`）；
    - 「停用 / 启用」轻文字按钮（带 loading 防连点，调用 `setPluginEnabled`）；
    - 「移除」轻文字按钮（hover 显 `var(--pl-error)`，打开现有带「同时清除插件数据」勾选框的卸载确认 Modal）。
- **底部安静状态栏**（列表下方 `padding: 8px 4px`）：左 `插件运行在 Bun x.y.z 上`（未安装显示未就绪），右「查看运行时 / 刷新」。

**不可丢功能核对**：筛选、搜索、空态（引导逛市场/手动安装）、启用/禁用、卸载（默认保留数据 + Checkbox 可选 `purgeData`）、编辑插件 options、跳转管理账号、打开单页/多页、刷新。

---

### 5.2 「插件市场」页（`PluginMarketView` + 新建 `PluginMarketDetailModal` — 参考 Magpie 图 2 & 图 3）

**顶栏（sticky，background `var(--pl-bg)`）**：
- 一级可见：类型分段器（`全部 / 供应商 / 中间件 / 页面`）+ 市场源 Select + 搜索框 + 刷新图标按钮 + `MoreHorizontal` 更多菜单 + 安静总数 `N 个插件`。
- 更多菜单承载次级操作：`搜索 npm` 勾选项、**`国内镜像` 勾选项**、「管理市场源」。刷新是高频操作，保持一级可见；其余操作不得重新铺回工具栏，否则常见窗口宽度下会折到第二行。

**三列网格（桌面固定一行 3 个！）**：
```less
.grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
}
@media (max-width: 960px) {
  .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
@media (max-width: 640px) {
  .grid { grid-template-columns: minmax(0, 1fr); }
}
```

**三段式轻量卡片（高度约 `136px–148px`，点击整卡打开详情弹窗）**：
- 容器：`padding: 16px`，`border-radius: var(--pl-radius-card)`，`background: var(--pl-surface)`，`border: 1px solid var(--pl-border)`，`box-shadow: var(--pl-shadow-card)`，`cursor: pointer`；hover 边框提亮至 `var(--pl-border-strong)` + `var(--pl-shadow-card-hover)`（**不做位移跳动或缩放**）。
- **顶行（flex，align-items: center，gap: 12px）**：
  - 左：`36px × 36px` 图标（圆角 `10px` 柔底 `var(--pl-surface-raised)`）；
  - 中（flex: 1，min-width: 0）：
    - 标题：`14px / 600`，`var(--pl-text-primary)`，单行省略；
    - 来源行：`12px`，若为官方/社区市场显示绿盾图标 `ShieldCheck` + `magpie 社区`（`var(--pl-success)`），其他自定义市场显示市场名（`var(--pl-text-muted)`）；
  - 右（`e.stopPropagation()` 阻止冒泡触发弹窗）：
    - 未安装：Magpie 同款**柔靛药丸按钮**（`height: 30px`，`padding: 0 14px`，`border-radius: 999px`，`background: var(--pl-accent-bg)`，`color: var(--pl-accent-text)`，`border: 1px solid var(--pl-accent-border)`，hover `var(--pl-accent-bg-hover)`）；安装中显示旋转 `Loader2`；
    - 已安装：干净绿字无重框 `✓ 已安装`（`color: var(--pl-success)`，`font-size: 12px`，`font-weight: 500`）。
- **中段**：2 行简介（`12.5px`，`line-height: 1.55`，`color: var(--pl-text-secondary)`，`-webkit-line-clamp: 2`，`min-height: 39px` 保证同行卡片齐平）。
- **底行**：无顶部分隔线，清爽留白 `margin-top: auto`，单行元数据：`类型图标 + 供应商/中间件/页面`（`12px`，`var(--pl-text-muted)`）+ 等宽版本号 `v0.1.0`（`12px`，`var(--pl-text-muted)`，`tabular-nums`）；不再把长 `@scope/pkg` 和一排 providerTags 堆在卡片表面！

**插件详情弹窗（`PluginMarketDetailModal`，参考 Magpie 图 3）**：
- Ant Design 原生 Modal（`width={560}`，不叠多余 `.ant-modal-body` padding）；
- **头部**：`44px` 图标 + 名称（`16px / 600`）& 下行 `来源 · @scope/package-name` + 右侧柔靛药丸「安装」按钮（或 `✓ 已安装`）；
- **简介段落**：完整 `summary / description`（不截断）；
- **4 格横向属性栏**（1px 细边圆角容器内 4 等分竖线分隔）：`版本` / `类型` / `来源` / `提供渠道或页面`；
- **npm 外链按钮**：若为 npm 包，提供轻量圆角链接 `npm ↗`（用 `openUrl` 打开 `https://www.npmjs.com/package/<pkg>`）；
- **详细说明区**：展示完整描述、提供的 provider 列表（胶囊标签）及安装命令参考；
- **底部安全提示栏**：左侧灰字「插件是他人的代码：由它登录并发出请求，或作为中间件看到请求。」，右侧「关闭」按钮。

**不可丢功能核对**：类型过滤、市场源过滤、防抖搜索、`includeNpmSearch` 开关、管理市场源弹窗（`MarketRegistryModal`）、刷新市场、安装插件、空态一键重置筛选。

---

### 5.3 「渠道与账号」页（`PluginAccountsView` — 参考 Magpie 图 5 & 图 6 + 渠道级自动签到 + 单列紧凑）

#### 顶部渠道选择器（大幅瘦身）
- 将原来两层松散头部压成**紧凑单行工具栏**（sticky，background `var(--pl-bg)`）：
  - 左：横向可滚动的渠道药丸胶囊列表（`min-height: 32px`，选中态柔靛 `var(--pl-accent-bg)` + `var(--pl-accent-border)` + `var(--pl-accent-text)`；含图标 + 名称 + 健康小圆点 + 账号数）；
  - 右：紧凑搜索框（`180px`）+ 图标刷新按钮（`32×32`）。

#### 选中渠道信息头（两行卡片，所有操作平铺可见）
- `padding: 12px 16px`，`border-radius: var(--pl-radius-card)`，`background: var(--pl-surface)`，`border: 1px solid var(--pl-border)`，内部纵向 `gap: 8px`：
  - **第一行（身份 + 主操作）**：`32px` 图标 + `渠道名`（`15px / 600`）+ 下行轻灰信息 `@package-spec` · `已登录 N 个账号`（去掉冗余的 `渠道 Provider` 胶囊）；右侧只放唯一实心主按钮 `登录账号`（`32px` 高、`8px` 圆角），靠标题 `flex: 1` 自然推到最右。
  - **第二行（次级操作组，`1px solid var(--pl-divider)` 顶部分隔线 + `8px` 内边距）**：左侧依次为 `添加到 CLI`、`刷新模型`、（有 `usage` 时）`刷新额度`、（有 `checkin` 时）`一键签到`、插件动作菜单，均为 `var(--pl-surface)` + `1px` 细边的轻量按钮；**渠道级「自动签到」Switch**（仅当 `selectedProvider.checkin === true` 时显示）放在该行最右端（`margin-left: auto`），带简洁文字标签「自动签到」+ `Switch size="small"` + Tooltip 说明「开启后每天自动为该渠道下所有已启用账号签到」。窄屏（`< 760px`）取消该自动外边距，让按钮与开关按顺序换行。
  - 所有操作保持一级可见，**不折叠进「更多」菜单**；不要把它们塞回首行，否则常见宽度下会与标题争抢空间并折行。

#### 账号列表（单列紧凑，参考 Magpie 图 5）
- 容器：整块圆角列表卡（`var(--pl-surface)` + `1px solid var(--pl-border)` + `border-radius: var(--pl-radius-card)`），多账号之间用 `1px solid var(--pl-divider)` 分隔，**保持单列排布**；
- **每个账号行（`padding: 12px 16px`，高度约 `72–84px`，内部 2 行）**：
  - **第一行（身份 + 控制）**：
    - 左侧：`18px` 状态徽标（已启用为深色/绿色圆圈对勾 `CheckCircle2`，已停用为灰圆圈）+ **主标识（优先显示 `usage.user` 或可读 `displayName`，不再重复把和渠道同名的 `qoder` 放最显眼处；当有多号且有内部 key 时在旁以等宽灰字轻注）** + `Plan` 浅灰圆角小标（如 `Free plan` / `personal_standard`，`background: var(--pl-surface-raised)`）+ `OAuth / API` 静默轻标签 + 单账号刷新额度小图标按钮（`24×24`，无框 hover 显底）；
    - 右侧：`启用` 开关（标签 + `Switch size="small"`）+ 细竖分隔线 + 紧凑图标按钮组（`28×28`：`PlugZap` 连通性测试 / 若支持签到则 `CalendarCheck` 手动单号签到 / `LogOut` 退出登录 hover 红）。**账号行不再显示「自动签到」Switch！**
  - **第二行（内联短进度条额度 + 轻灰尾注）**：
    - 彻底告别横跨 1000px 的超大进度条！每个额度窗口渲染为紧凑内联行/项：
      - `窗口名（如 积分 / Credits / Weekly）`（`12px`，`var(--pl-text-secondary)`）
      - **内联短进度条**（`width: 96px`，`height: 6px`，`border-radius: 999px`，槽底 `var(--pl-surface-raised)`，填充色按健康度：正常 `var(--pl-accent)` / 警告 `var(--pl-warning)` / 耗尽 `var(--pl-error)`）
      - 紧凑等宽数值：`0 / 100 · 已用 0%`（`12px`，`var(--pl-text-primary)`，`tabular-nums`）
      - 重置倒计时：`3 天后重置`（`12px`，`var(--pl-text-muted)`）
    - 当有多个额度窗口时，用 `display: flex; flex-wrap: wrap; gap: 8px 20px` 横向流式排开，高密度而不拉高卡片；
    - 钱包余额、OAuth 过期提醒、上次签到结果（`上次签到: 已领取 · 11/27 08:56`）以简洁灰字尾注排列在同区或轻提示行，过期/失败时才用语义色强调；无额度数据时显示干净单行灰字。

#### 模型目录（流式药丸胶囊 + 极简能力标，参考 Magpie 图 6）
- 标题栏：`模型目录`（`14px / 600`）+ 等宽计数 `17` + 右侧紧凑搜索框（`200px`）；
- **内容区（去掉庞大的 4 列表格！）**：
  - 放在一块清爽圆角表面卡内（`padding: 14px 16px`，`background: var(--pl-surface)`，`border: 1px solid var(--pl-border)`，`border-radius: var(--pl-radius-card)`）；
  - `display: flex; flex-wrap: wrap; gap: 8px` 流式排列圆角药丸胶囊（`min-height: 32px`，`padding: 4px 12px`，`border-radius: 999px`，`border: 1px solid var(--pl-border)`，`background: var(--pl-surface)`，hover 切 `var(--pl-accent-bg)` + `var(--pl-accent-border)`，`cursor: pointer`）：
    - **胶囊内从左到右**：
      1. `模型显示名`（无显示名则用 `model.id`，`12.5px / 500`，`var(--pl-text-primary)`）；
      2. **极简能力微标**（`12px` 静默灰线 `lucide-react` 图标，不花哨，仅当模型具备该能力时显示）：
         - 推理：`Brain`（或 `Cpu`）
         - 视觉：`Eye`
         - 快速：`Zap`
      3. **倍率/免费微标**（`11px`，`padding: 1px 6px`，`border-radius: 6px`，`tabular-nums`）：
         - `model.free` → 柔和绿底 `Free`（`var(--pl-success-bg)` / `var(--pl-success)`）
         - `model.rate > 0` → 柔灰底 `${model.rate}×`（`var(--pl-surface-raised)` / `var(--pl-text-secondary)`）
      4. **上下文微标**（若有 `model.context`）：柔灰底 `1M` / `200K`（`var(--pl-surface-raised)` / `var(--pl-text-secondary)`）；
    - **Hover Tooltip**：完整展示 `model.id`、显示名、协议（`OpenAI Chat / Responses / Anthropic / Gemini`）、上下文窗口与全部能力文字；
    - **点击交互**：一键复制 `model.id` 到剪贴板（调用 `copyTextToClipboard` 并轻 toast 提示「已复制模型 ID」）。

#### 渠道选项（`selectedProvider.options`）
- 保留现有 `auth.options` 持久化开关区块（同样用整块圆角卡片 + 内部 `1px` 分隔线的左文右开关行，保留 `{ ...defaults, ...optionValues }` 合并逻辑与 `confirm` 二次确认弹窗，绝不丢失！）。

**不可丢功能核对**：URL `?provider=` 选中同步、无 Bun 报错与跳转运行时、登录弹窗（`PluginLoginModal`）、添加到 CLI 弹窗（`AddPluginToCliModal`）、刷新模型、刷新全部/单账号额度、一键/单号签到、渠道级自动签到、账号启用/禁用、账号连通性测试、退出登录二次确认、Provider 声明式 options 开关与刷新模型联动、模型搜索。

---

### 5.4 「运行时与日志」页（`PluginRuntimeView` — 根治左列挤压 + 接入国内镜像）

#### 1. Bun 运行时 & 插件宿主卡片
- 单张圆角卡（`padding: 16px`，`var(--pl-surface)`，`border: 1px solid var(--pl-border)`，`border-radius: var(--pl-radius-card)`）：
  - **上区（Bun 状态）**：`36px` 柔靛图标 + 标题 `Bun 运行时` + 等宽路径（单行省略 + title）+ 等宽元信息行（`来源 · 版本 · 期望版本`）；右侧按钮组：`刷新`、`检测 Bun`、（未就绪时实心柔靛 `下载 Bun`）。
  - `1px solid var(--pl-divider)` 分隔线。
  - **下区（插件宿主）**：`6px` 活状态圆点 + `插件宿主` + 状态词 + 等宽 `PID / 进行中 / 崩溃`；右侧「重启宿主」轻按钮。

#### 2. 运行时设置卡片（彻底修复 `108px` 窄列挤压！）
- 放弃 `grid-template-columns: 108px minmax(0, 1fr)`！改为现代清晰的 **Setting Rows 列表**（每行 `padding: 14px 0`，行间 `1px solid var(--pl-divider)`）：
  - **行 1：国内镜像（新增，开关行）**
    - 左（弹性宽，`min-width: 0`）：标题 `国内镜像`（`13px / 500`）+ 一行不折行的说明「启用后使用 npmmirror 镜像加速插件安装与市场拉取」（`12px`，`var(--pl-text-muted)`）；
    - 右：`Switch` 靠右对齐（开启即同步将下方 `npm Registry` 改为 `https://registry.npmmirror.com`，关闭切回 `https://registry.npmjs.org`）。
  - **行 2：npm Registry（输入行）**
    - 左（固定宽 `220px`，`< 760px` 堆叠上下）：标题 `npm Registry` + 简短说明「自定义插件包源地址」；
    - 右（flex，最大宽 `520px`）：输入框（与「国内镜像」开关双向联动）。
  - **行 3：Bun 路径（输入行）**
    - 左（固定宽 `220px`）：标题 `Bun 路径` + 简短说明「留空则依次查找应用托管与系统 PATH」；
    - 右（flex，最大宽 `520px`，`gap: 8px`）：输入框 + 同行「探测验证」轻按钮；探测结果贴齐右列下方展示（不再用硬编码 `margin-left: 120px`）。
  - **行 4：跟随应用代理（开关行）**
    - 左（弹性宽）：标题 `跟随应用代理` + 一行清晰说明「市场拉取与 npm 请求使用应用的网络代理设置」；
    - 右：`Switch` 靠右对齐。
  - **底部操作栏**：右对齐实心柔靛「保存设置」按钮（未修改时自然 disabled）。

#### 3. 中间件性能与诊断日志
- **中间件性能**：当 `middlewareStates.length === 0` 时，渲染紧凑单行安静空态卡（`padding: 16px`，不再画空表头撑大高度）；有数据时渲染高密度对齐表，数值列右对齐 `tabular-nums`。
- **诊断日志**：圆角容器 + 顶栏轻量级别分段（`全部 / 信息 / 警告 / 错误`）+ 右侧图标按钮（刷新 / 复制 / 清空）+ 等宽终端日志区（行高 `1.6`，清晰时间/级别/来源/正文对齐，保持前端二次脱敏 + 5 秒可见轮询切走即停）。

---

## 6. 验证与工程纪律

完成后必须在 `D:\GitHub\ai-toolbox-plugins-ui` 跑通全部校验，并按模块粒度 commit 到 `feat/plugins-ui-magpie`（不 push）：
1. `cd tauri && cargo test --lib plugins:: --jobs 2`
2. `cd tauri && cargo check`
3. `pnpm exec tsc --noEmit`
4. `pnpm run i18n:check`（严禁动态模板拼接 i18n 键）
5. `pnpm run test:css-module-classes`
6. `pnpm run test:provider-card-layout`
7. `node --test web/test/features/coding/shared/gateway/providerProtocol.test.ts web/test/features/coding/plugins/utils/pluginFormatters.test.ts`
8. `pnpm build`
9. 同步更新 `tauri/src/plugins/AGENTS.md` 与 `web/features/coding/plugins/AGENTS.md`（记录渠道级自动签到、国内镜像快捷开关、市场三列卡片+详情弹窗、Magpie 柔靛局部视觉体系等新事实）。

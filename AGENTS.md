# AGENTS.md - AI Toolbox Development Guide

This document provides essential information for AI coding agents working on this project.

## Communication Language

与用户的所有对话必须使用**中文**，包括问题澄清、方案说明、进度反馈和结果总结。代码注释和 commit message 仍使用英文。

## Project Overview

AI Toolbox is a cross-platform desktop application built with:
- **Frontend**: React 19 + TypeScript 5 + Ant Design 5 + Vite 7
- **Backend**: Tauri 2.x + Rust
- **Database**: SurrealDB 2.x (embedded SurrealKV)
- **Package Manager**: pnpm

## Directory Structure

```
ai-toolbox/
├── web/                    # Frontend source code
│   ├── app/                # App entry, routes, providers
│   ├── components/         # Shared components
│   ├── features/           # Feature modules
│   │   ├── coding/         # Coding tools (claudecode, codex, opencode, skills)
│   │   ├── daily/          # Daily notes
│   │   └── settings/       # App settings
│   ├── stores/             # Zustand state stores
│   ├── i18n/               # i18next localization
│   ├── constants/          # Module configurations
│   ├── hooks/              # Global hooks
│   ├── services/           # API services
│   └── types/              # Global type definitions
├── tauri/                  # Rust backend
│   ├── src/                # Rust source
│   │   ├── coding/         # Coding modules (claude_code, codex, open_code, skills)
│   │   └── settings/       # Settings modules
│   └── Cargo.toml          # Rust dependencies
└── package.json            # Frontend dependencies
```

## Build & Development Commands

### Frontend (pnpm)

```bash
# Install dependencies
pnpm install

# Start development server (frontend only)
pnpm dev

# Build frontend for production
pnpm build

# Type check
pnpm tsc --noEmit
```

### Tauri (Full App)

```bash
# Start full app in development mode
pnpm tauri dev

# Build production app
pnpm tauri build
```

### Rust (Backend)

```bash
# Check Rust code
cd tauri && cargo check

# Build Rust in release mode
cd tauri && cargo build --release

# Format Rust code
cd tauri && cargo fmt

# Lint Rust code
cd tauri && cargo clippy
```

### Testing (Not yet configured)

```bash
# Frontend tests (when configured)
pnpm test

# Run single test file
pnpm test -- path/to/test.ts

# Rust tests
cd tauri && cargo test

# Run single Rust test
cd tauri && cargo test test_name
```

## Code Style Guidelines

### TypeScript/React

#### Imports Order
1. React and React-related imports
2. Third-party libraries (antd, react-router-dom, etc.)
3. Internal aliases (`@/...`)
4. Relative imports
5. Style imports (`.less`, `.css`)

```typescript
// Example
import React from 'react';
import { Layout, Tabs } from 'antd';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { MODULES } from '@/constants';
import { useAppStore } from '@/stores';
import styles from './styles.module.less';
```

#### Naming Conventions
- **Components**: PascalCase (`MainLayout.tsx`)
- **Hooks**: camelCase with `use` prefix (`useAppStore.ts`)
- **Stores**: camelCase with `Store` suffix (`appStore.ts`)
- **Services**: camelCase with `Service` suffix (`noteService.ts`)
- **Types/Interfaces**: PascalCase (`interface AppState {}`)
- **Constants**: SCREAMING_SNAKE_CASE for values, PascalCase for configs

#### Component Structure
```typescript
import React from 'react';

interface Props {
  // Props interface
}

const ComponentName: React.FC<Props> = ({ prop1, prop2 }) => {
  // Hooks first
  const { t } = useTranslation();
  const navigate = useNavigate();
  
  // State and derived values
  const [state, setState] = React.useState();
  
  // Effects
  React.useEffect(() => {}, []);
  
  // Handlers
  const handleClick = () => {};
  
  // Render
  return <div />;
};

export default ComponentName;
```

#### Zustand Stores

Use Zustand without persistence middleware - all data must go through the service layer to SurrealDB:

```typescript
interface SettingsState {
  settings: AppSettings | null;
  initSettings: () => Promise<void>;
  updateSettings: (settings: AppSettings) => Promise<void>;
}

export const useSettingsStore = create<SettingsState>()((set) => ({
  settings: null,

  initSettings: async () => {
    const settings = await getSettings(); // Call service API
    set({ settings });
  },

  updateSettings: async (newSettings) => {
    await saveSettings(newSettings); // Save to database
    set({ settings: newSettings });
  },
}));
```

**Never use persist middleware** - all persistent data must be stored in SurrealDB via Tauri commands.

#### Path Aliases
Use `@/` for imports from `web/` directory:
```typescript
import { useAppStore } from '@/stores';
import { MODULES } from '@/constants';
```

### Rust

#### Naming Conventions
- **Functions/Methods**: snake_case
- **Structs/Enums**: PascalCase
- **Constants**: SCREAMING_SNAKE_CASE
- **Modules**: snake_case

#### Tauri Commands
```rust
#[tauri::command]
fn command_name(param: &str) -> Result<ReturnType, String> {
    // Implementation
    Ok(result)
}
```

#### Error Handling
- Use `thiserror` for custom errors
- Return `Result<T, String>` for Tauri commands
- Use `?` operator for error propagation

### Styling

- Use CSS Modules with Less (`.module.less`)
- Class naming: camelCase in Less files
- Use Ant Design's design tokens when possible

```less
.container {
  display: flex;

  &.active {
    background: rgba(24, 144, 255, 0.1);
  }
}
```

### Form & Modal Layout

**Modal forms should use horizontal (left-right) layout by default**, where labels are on the left and input fields are on the right. This provides better visual alignment and more efficient use of space.

#### Layout Guidelines

1. **Prefer Horizontal Layout**: Use Ant Design Form with `layout="horizontal"` for modal forms
2. **Label Placement**: Labels should be right-aligned and placed on the left side of inputs
3. **Consistent Label Width**: Use `labelCol` and `wrapperCol` to maintain consistent proportions

#### Implementation Pattern

```typescript
// ✅ Recommended: Horizontal layout for modal forms
<Form layout="horizontal" labelCol={{ span: 6 }} wrapperCol={{ span: 18 }}>
  <Form.Item label={t('name')} name="name">
    <Input />
  </Form.Item>
  <Form.Item label={t('description')} name="description">
    <Input.TextArea />
  </Form.Item>
</Form>

// ❌ Avoid: Vertical layout in modals (unless space is very limited)
<Form layout="vertical">
  <Form.Item label={t('name')} name="name">
    <Input />
  </Form.Item>
</Form>
```

#### When to Use Vertical Layout

Use vertical layout (`layout="vertical"`) only in these cases:
- Very narrow containers where horizontal layout would be cramped
- Forms with very long labels that don't fit well horizontally
- Single-field quick input forms

### Theme System (Dark Mode)

**IMPORTANT: The application supports full dark mode / light mode / system theme switching. ALL UI colors must use theme variables or Ant Design tokens - NEVER hardcode color values.**

#### Theme Architecture

The app uses a multi-layer theming system:

1. **Theme Store** (`web/stores/themeStore.ts`):
   - Manages theme mode: `'light'`, `'dark'`, or `'system'`
   - Automatically syncs with system theme when mode is `'system'`
   - Persists preference to database

2. **Theme Provider** (`web/app/providers.tsx`):
   - Applies Ant Design theme algorithm (`darkAlgorithm` or `defaultAlgorithm`)
   - Sets `data-theme` attribute on `document.documentElement`
   - Updates window background color for native titlebar

3. **CSS Variables** (`web/App.css`):
   - Defines theme-aware CSS variables
   - All custom variables automatically switch when `data-theme` attribute changes

#### Available CSS Variables

**Background Colors:**
- `--color-bg-base` - Base background color
- `--color-bg-container` - Container background
- `--color-bg-layout` - Layout background
- `--color-bg-elevated` - Elevated surface (dropdowns, modals)
- `--color-bg-hover` - Hover state background
- `--color-bg-selected` - Selected state background

**Text Colors:**
- `--color-text-primary` - Primary text (high emphasis)
- `--color-text-secondary` - Secondary text (medium emphasis)
- `--color-text-tertiary` - Tertiary text (low emphasis)

**Border Colors:**
- `--color-border` - Default border color
- `--color-border-secondary` - Secondary border (higher contrast)
- `--color-border-card` - Card border

**Other:**
- `--color-shadow` - Primary shadow
- `--color-shadow-secondary` - Secondary shadow
- `--color-scrollbar` - Scrollbar color

#### Usage Guidelines

**DO:**
```less
// ✅ Use CSS variables
.container {
  background: var(--color-bg-container);
  color: var(--color-text-primary);
  border: 1px solid var(--color-border);
}

// ✅ Use Ant Design tokens (via ConfigProvider)
.container {
  color: #1890ff; // OK for brand colors managed by Ant Design
}

// ✅ Dark mode specific overrides
.icon {
  opacity: 0.7;

  :global([data-theme="dark"]) & {
    filter: invert(1);
  }
}
```

**DON'T:**
```less
// ❌ Never hardcode colors
.container {
  background: #ffffff; // Wrong! Use var(--color-bg-container)
  color: rgba(0, 0, 0, 0.88); // Wrong! Use var(--color-text-primary)
}

// ❌ Don't use media queries for theme
@media (prefers-color-scheme: dark) { // Wrong! Use [data-theme="dark"]
  .container { ... }
}
```

#### Dark Mode Patterns

**Pattern 1: CSS Variables (Recommended)**
```less
.myComponent {
  background: var(--color-bg-container);
  color: var(--color-text-primary);
}
// Automatically adapts to theme changes
```

**Pattern 2: Attribute Selector Overrides**
```less
.myComponent {
  background-color: rgba(255, 255, 255, 0.2);

  :global([data-theme="dark"]) & {
    background-color: rgba(20, 20, 20, 0.2);
  }
}
```

**Pattern 3: Image/Icon Filters**
```less
.icon {
  // Default: black icon on light background

  :global([data-theme="dark"]) & {
    filter: invert(1); // Inverts to white icon
  }
}
```

#### Accessing Theme in TypeScript

```typescript
import { useThemeStore } from '@/stores/themeStore';

const MyComponent = () => {
  const { mode, resolvedTheme } = useThemeStore();
  // mode: 'light' | 'dark' | 'system'
  // resolvedTheme: 'light' | 'dark' (computed value)

  // Use resolvedTheme for conditional rendering
  const iconColor = resolvedTheme === 'dark' ? '#fff' : '#000';
};
```

#### Testing Theme Support

When implementing new components or features:

1. **Test both themes**: Switch between light and dark mode in Settings
2. **Test system theme**: Set to "System" and toggle OS theme
3. **Check all states**: Hover, active, disabled, selected
4. **Verify readability**: Ensure text contrast meets accessibility standards
5. **Review hardcoded colors**: Search for hex colors (`#`) in your styles

#### Common Mistakes to Avoid

1. **Hardcoding opacity values**: Use theme variables instead
   - ❌ `rgba(0, 0, 0, 0.88)` → ✅ `var(--color-text-primary)`

2. **Using media queries for theme**: Use `[data-theme]` attribute selector
   - ❌ `@media (prefers-color-scheme: dark)` → ✅ `[data-theme="dark"]`

3. **Inline styles with hardcoded colors**: Extract to CSS modules or use theme variables
   - ❌ `<div style={{ color: '#000' }}>` → ✅ Use CSS class with var()

4. **Forgetting images/icons**: Dark backgrounds require inverted icons
   - Add `filter: invert(1)` for dark mode when needed

### Internationalization

- All user-facing text must use i18next
- Translation keys in `web/i18n/locales/`
- Use nested keys: `modules.daily`, `settings.language`

```typescript
const { t } = useTranslation();
<span>{t('modules.daily')}</span>
```

## Feature Module Structure

Each feature in `web/features/` follows this pattern:

```
features/
└── feature-name/
    ├── components/     # Feature-specific components
    ├── hooks/          # Feature-specific hooks
    ├── services/       # Tauri command wrappers
    ├── stores/         # Feature state
    ├── types/          # Feature types
    ├── pages/          # Page components
    └── index.ts        # Public exports
```

## Key Configuration Files

| File | Purpose |
|------|---------|
| `tsconfig.json` | TypeScript config with path aliases |
| `vite.config.ts` | Vite build config, dev server on port 5173 |
| `tauri/tauri.conf.json` | Tauri app config |
| `tauri/Cargo.toml` | Rust dependencies |

## Important Notes

1. **Strict TypeScript**: `noUnusedLocals` and `noUnusedParameters` are enabled
2. **SurrealDB**: Uses embedded SurrealKV engine, data stored locally
3. **i18n**: Supports `zh-CN` and `en-US`
4. **Theme**: Full dark mode / light mode / system theme support implemented (see Theme System section in Code Style Guidelines)
5. **Dev Server**: Runs on `http://127.0.0.1:5173`

## Data Storage Architecture

**IMPORTANT**: All data storage and retrieval must go through the service layer API and interact directly with the backend database (SurrealDB). This is a local embedded database with very fast performance.

### DO NOT use localStorage

- **Never** use `localStorage` or `zustand/persist` for data that needs to be persisted
- **Never** sync data from localStorage to database - this pattern is not allowed
- All persistent data must be stored directly in SurrealDB via Tauri commands

### Correct Data Flow

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐     ┌──────────────┐
│  Component  │ ──► │  Service Layer   │ ──► │  Tauri Command  │ ──► │  SurrealDB   │
│  (React)    │ ◄── │  (web/services/) │ ◄── │  (Rust)         │ ◄── │  (Database)  │
└─────────────┘     └──────────────────┘     └─────────────────┘     └──────────────┘
```

### Service Layer Structure

All API services are located in `web/services/`:

```typescript
// web/services/settingsApi.ts
import { invoke } from '@tauri-apps/api/core';

export const getSettings = async (): Promise<AppSettings> => {
  return await invoke<AppSettings>('get_settings');
};

export const saveSettings = async (settings: AppSettings): Promise<void> => {
  await invoke('save_settings', { settings });
};
```

### Backend Command Pattern

All Tauri commands interacting with SurrealDB must follow the **Adapter Pattern** and use **Raw SQL** to ensure backward compatibility and avoid versioning issues.

#### 1. Database Naming Convention
- **Database Fields**: Must use `snake_case`.
- **Rust Structs**: Use `snake_case`.
- **Do NOT** use `#[serde(rename_all = "camelCase")]` for database records.

#### 2. Adapter Layer (Required)
Always implement an adapter layer to decouple Rust structs from database records. This handles missing fields and type mismatches robustly.

```rust
// adapter.rs
use serde_json::Value;
use super::types::AppSettings;

pub fn from_db_value(value: Value) -> AppSettings {
    AppSettings {
        // Robust extraction with defaults
        language: value.get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en-US")
            .to_string(),
        // ... other fields with default values
    }
}

pub fn to_db_value(settings: &AppSettings) -> Value {
    serde_json::to_value(settings).unwrap_or(json!({}))
}
```

#### 3. Persistence Pattern (Updates & ID Handling)
To avoid SurrealDB versioning conflicts (`Invalid revision` errors) and deserialization failures (`invalid type: map`):

1.  **Reads**: Handle the `Thing` ID type explicitly.
    *   **Best Practice**: Use **`type::string(id)`** in your query to convert the ID to a string before returning to Rust.
    *   **Why**: SurrealDB's default `id` is a `Thing` object (e.g., `{ tb: "table", id: "id" }`). Direct deserialization into a `String` field in Rust will fail. Explicit conversion ensures compatibility.
    *   **Code**: `SELECT *, type::string(id) as id FROM table:id`
    *   **IMPORTANT**: The converted ID includes the table prefix (e.g., `"claude_provider:abc123"`). When passing this ID to the frontend or using it in subsequent operations, **you must strip the table prefix** (e.g., `"abc123"`) in the adapter layer before returning to business logic.
    *   **Use Common Utility**: Always use the `db_id` module for ID handling:
        ```rust
        // In adapter.rs
        use crate::coding::db_id::db_extract_id;

        pub fn from_db_value_provider(value: Value) -> ClaudeCodeProvider {
            let id = db_extract_id(&value);
            // ...
        }
        ```
    *   **Available Functions** (`crate::coding::db_id`):
        *   `db_extract_id(record: &Value) -> String` - Extract and clean ID from a record
        *   `db_extract_id_opt(record: &Value) -> Option<String>` - Same but returns Option
        *   `db_clean_id(raw_id: &str) -> String` - Clean a raw ID string
        *   `db_build_id(table: &str, id: &str) -> String` - Build a record ID string
        *   `db_record_id(table: &str, id: &str) -> String` - Build backtick-escaped record reference for queries (e.g., `` table:`id` ``)
        *   `db_new_id() -> String` - Generate a new record ID (UUID v4, no hyphens)

2.  **Record ID Reference in Queries**: Use `db_record_id()` to build backtick-escaped record references.
    *   **Problem**: `type::thing('table', $id)` behavior changed across SurrealDB versions (e.g., 2.4 → 2.6), causing "not found" errors even for existing records.
    *   **Solution**: Use `db_record_id(table, id)` which generates `` table:`id` `` format. Backtick-escaped IDs are treated as literal strings regardless of content, avoiding version-specific parsing issues.
    *   **NEVER** use `type::thing()` in any query. Always use `db_record_id()` instead.
    *   **Code**:
        ```rust
        use crate::coding::db_id::db_record_id;

        // SELECT by ID
        let record_id = db_record_id("claude_provider", &id);
        db.query(&format!("SELECT *, type::string(id) as id FROM {} LIMIT 1", record_id))

        // UPDATE by ID
        let record_id = db_record_id("mcp_server", &server_id);
        db.query(&format!("UPDATE {} SET enabled = $enabled", record_id))
            .bind(("enabled", true))

        // DELETE by ID
        let record_id = db_record_id("mcp_server", server_id);
        db.query(&format!("DELETE {}", record_id))
        ```
    *   **Applies to**: All queries that target a specific record by ID:
        *   `SELECT ... FROM {record_id}`
        *   `UPDATE {record_id} SET ...` or `UPDATE {record_id} CONTENT $data`
        *   `DELETE {record_id}`
        *   `CREATE {record_id} CONTENT $data`
        *   `UPSERT {record_id} CONTENT $data` or `UPSERT {record_id} SET ...`

3.  **Record ID Generation**: Prefer SurrealDB auto-generated IDs. When manual IDs are needed, use `db_new_id()`.
    *   **Preferred**: Let SurrealDB auto-generate IDs via `CREATE table CONTENT $data` (no ID specified). This is how `claude_provider`, `codex_provider`, `oh_my_opencode_config`, etc. work.
    *   **When manual IDs are needed** (e.g., MCP servers, skills): Use the shared `db_new_id()` function which generates UUID v4 without hyphens.
    *   **NEVER** call `uuid::Uuid::new_v4()` directly in store/command files. Always use `db_new_id()` from the `db_id` module.
    *   **Code**:
        ```rust
        use crate::coding::db_id::{db_record_id, db_new_id};

        // Create with manual ID
        let id = db_new_id();
        let record_id = db_record_id("mcp_server", &id);
        db.query(&format!("CREATE {} CONTENT $data", record_id))
            .bind(("data", payload))
        ```

4.  **Updates**: Use **Blind Writes (Overwrite)** to bypass version checks.
    *   **Avoid**: Do NOT send the `version` or `revision` field back to the database in the `CONTENT` block. This triggers optimistic currency control checks which often fail.
    *   **Avoid**: Do NOT include the `id` field in the `CONTENT` block. It can cause type conflicts.
    *   **Pattern 1 (Update by ID)**: Use `db_record_id()` to target a specific record:
        ```rust
        let record_id = db_record_id("claude_provider", &id);
        db.query(&format!("UPDATE {} CONTENT $data", record_id))
            .bind(("data", payload))
        ```
    *   **Pattern 2 (Create or Update)**: Use `UPSERT` with `db_record_id()` for singleton or known-ID records:
        ```rust
        let record_id = db_record_id("settings", "app");
        db.query(&format!("UPSERT {} CONTENT $data", record_id))
            .bind(("data", payload))
        ```
        Or use hardcoded backtick format for fixed singleton IDs: `UPSERT settings:\`app\` CONTENT $data`
    *   **Pattern 3 (Single Field)**: `UPDATE {record_id} SET field = $value`
    *   **Pattern 4 (Batch by condition)**: `UPDATE table SET field = $value WHERE condition = true` (no ID targeting needed)

5.  **SurrealDB Wrapper Characters**: Handled automatically by `db_extract_id()` / `db_clean_id()`. No manual handling needed — these functions strip table prefixes and `⟨⟩` wrappers transparently.

```rust
// commands.rs
#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, DbState>) -> Result<AppSettings, String> {
    let db = state.0.lock().await;

    // CRITICAL: Convert `Thing` ID to string to match Rust struct types
    // This avoids "invalid type: map, expected a string" errors
    let mut result = db
        .query("SELECT *, type::string(id) as id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query settings: {}", e))?;

    let records: Vec<serde_json::Value> = result.take(0).map_err(|e| e.to_string())?;

    if let Some(record) = records.first() {
        Ok(adapter::from_db_value(record.clone()))
    } else {
        Ok(AppSettings::default())
    }
}

#[tauri::command]
pub async fn save_settings(
    state: tauri::State<'_, DbState>,
    settings: AppSettings,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Serialize settings but EXCLUDE sensitive system fields
    // Ensure `adapter::to_clean_payload` removes 'id' and 'version'/'revision'
    let json_payload = adapter::to_clean_payload(&settings);

    // CRITICAL for Updates:
    // 1. Use CONTENT with a clean payload (no version = no lock check).
    // 2. ID is used in the query target with native format, NOT in the content.
    // 3. Use UPSERT for singleton records to handle both create and update:
    //    UPSERT settings:`app` CONTENT $data
    db.query("UPSERT settings:`app` CONTENT $data")
        .bind(("data", json_payload)) // Clean data without ID/Version
        .await
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    Ok(())
}
```

### Benefits of Direct Database Access

1. **Performance**: SurrealDB with SurrealKV engine is embedded and extremely fast
2. **Consistency**: Single source of truth for all data
3. **Backup**: Database files can be backed up/restored as a whole
4. **No Sync Issues**: Avoids complex synchronization between localStorage and database

---

## System Tray Menu Integration

### Overview

The system tray menu provides quick access to configuration selections without opening the main window. When configurations are changed (either from the main window or the tray menu), the tray menu must stay in sync.

### Event-Driven Architecture

All configuration changes use the `config-changed` Tauri event to synchronize state:

| Source | Event Payload | Tray Refresh | Page Reload |
|--------|---------------|--------------|-------------|
| Main Window | `"window"` | ✅ | ❌ |
| Tray Menu | `"tray"` | ✅ | ✅ |

### Backend Implementation

#### 1. Internal Function Pattern

All modules should implement an internal function `apply_config_internal` that handles configuration saving and event emission:

```rust
// commands.rs
pub async fn apply_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle<R>,
    config: ModuleConfig,
    from_tray: bool,
) -> Result<(), String> {
    // 1. Save configuration to file/database
    save_config_to_file(state, &config).await?;

    // 2. Update database state if needed
    update_db_state(state, &config).await?;

    // 3. Emit event based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    Ok(())
}
```

#### 2. Tauri Command (Main Window)

The Tauri command called by the frontend passes `from_tray: false`:

```rust
#[tauri::command]
pub async fn save_module_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config: ModuleConfig,
) -> Result<(), String> {
    apply_config_internal(state, &app, config, false).await
}
```

#### 3. Tray Support Module

The tray support module calls with `from_tray: true`:

```rust
// tray_support.rs
pub async fn apply_module_selection<R: Runtime>(
    app: &AppHandle<R>,
    selection_id: &str,
) -> Result<(), String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    // Build config from selection
    let config = build_config_from_selection(&db, selection_id)?;

    // Apply with from_tray: true
    super::commands::apply_config_internal(&db, app, config, true).await?;

    Ok(())
}
```

#### 4. Global Event Listener (lib.rs)

The main entry point registers a global listener that refreshes the tray menu on any `config-changed` event:

```rust
// lib.rs
let app_handle_clone = app_handle.clone();
tauri::async_runtime::spawn(async move {
    let value = app_handle_clone.clone();
    let value_for_closure = value.clone();
    let listener = value.listen("config-changed", move |_event| {
        let app = value_for_closure.app_handle().clone();
        let _ = tauri::async_runtime::spawn(async move {
            let _ = tray::refresh_tray_menus(&app);
        });
    });
    let _ = listener;
});
```

### Frontend Implementation

#### 1. Event Listener (providers.tsx)

The app's main provider listens for `config-changed` events and triggers a page reload only for tray menu changes:

```typescript
// web/app/providers.tsx
use { listen } from '@tauri-apps/api/event';

React.useEffect(() => {
  const setupListener = async () => {
    unlisten = await listen<string>('config-changed', (event) => {
      const configType = event.payload;
      // Only reload page when change comes from tray menu
      if (configType === 'tray') {
        window.location.reload();
      }
      // Changes from main window only refresh the tray menu (handled by backend)
    });
  };
  setupListener();
  return () => { if (unlisten) unlisten(); };
}, []);
```

### Tray Support Module Structure

Each coding module with tray integration should have:

```
tauri/src/coding/{module_name}/
├── commands.rs          # Tauri commands + apply_config_internal
├── tray_support.rs      # Tray-specific functions
├── adapter.rs           # DB value adapters
└── types.rs             # Type definitions
```

### Tray Support Module Functions

The `tray_support.rs` must export:

```rust
// Data structures
pub struct TrayData {
    pub title: String,           // Section title
    pub items: Vec<TrayItem>,    // Selection items
}

pub struct TrayItem {
    pub id: String,              // Unique identifier
    pub display_name: String,    // Display text
    pub is_selected: bool,       // Current selection state
}

// Required functions
pub async fn get_{module}_tray_data<R: Runtime>(app: &AppHandle<R>)
    -> Result<TrayData, String>;

pub async fn apply_{module}_selection<R: Runtime>(app: &AppHandle<R>, id: &str)
    -> Result<(), String>;
```

### Menu Refresh Function

The `tray.rs` module exports:

```rust
pub async fn refresh_tray_menus<R: Runtime>(app: &AppHandle<R>)
    -> Result<(), String> {
    // 1. Fetch data from all modules
    let module_data = module_tray::get_module_tray_data(app).await?;

    // 2. Build menu items with checkmarks
    let items = build_menu_items(app, &module_data)?;

    // 3. Update tray menu
    let tray = app.state::<tauri::tray::TrayIcon>();
    tray.set_menu(Some(menu))?;

    Ok(())
}
```

### File Structure

```
tauri/src/
├── tray.rs                    # Main tray menu builder
├── lib.rs                     # Global event listener setup
└── coding/
    └── {module}/
        ├── commands.rs        # apply_config_internal + Tauri commands
        ├── tray_support.rs    # Tray data fetching + apply functions
        ├── adapter.rs
        └── types.rs

web/
├── app/
│   └── providers.tsx          # config-changed event listener
└── services/
    └── {module}Api.ts         # Backend API wrappers
```

### Implementation Checklist for New Tray Integration

1. **Backend** (`tauri/src/coding/{module}/`):
   - [ ] Add `apply_config_internal` function with `from_tray` parameter
   - [ ] Implement Tauri command for main window (calls with `false`)
   - [ ] Implement tray support functions:
     - `get_{module}_tray_data()` - returns current selections
     - `apply_{module}_selection()` - handles tray menu selection (calls with `true`)
   - [ ] Emit `config-changed` event with `"window"` or `"tray"` payload

2. **Frontend** (`web/app/providers.tsx`):
   - [ ] Ensure `config-changed` event listener reloads page only for `"tray"` payload

3. **Main Entry** (`tauri/src/lib.rs`):
   - [ ] Global listener already exists - no changes needed

---

## OpenCode Configuration Format

### Model Selection

OpenCode uses `provider_id/model_id` format for model configuration:

```typescript
// Main model: provider_id/model_id
config.model = Some("openai/gpt-4o");

// Small model: provider_id/model_id
config.small_model = Some("qwen/qwen3");
```

### Tray Menu Structure

The tray menu displays models with checkmarks:

```
──── OpenCode 模型 ────
主模型 (gpt-4o)
├── OpenAI / gpt-4o ✓
├── OpenAI / gpt-4o-mini
├── Qwen / qwen3 ✓
└── ...
小模型 (qwen3)
├── OpenAI / gpt-4o-mini
├── Qwen / qwen3 ✓
└── ...
```

When a user selects a model from the tray menu:
1. Parse `provider_id/model_id` from item ID
2. Update config with new selection
3. Emit `config-changed` event with `"tray"` payload
4. Frontend reloads page to reflect changes

---

## HTTP Client Guidelines

All HTTP requests in the Rust backend MUST use the unified `http_client` module to ensure proxy settings are respected.

### Usage

```rust
use crate::http_client;
use crate::db::DbState;

// Standard request (30s timeout, auto proxy)
let client = http_client::client(&state).await?;

// Custom timeout
let client = http_client::client_with_timeout(&state, 60).await?;

// Bypass proxy (special cases only)
let client = http_client::client_no_proxy(30)?;

// Get proxy URL directly (for non-HTTP use cases like git)
let proxy_url = http_client::get_proxy_from_settings(&state).await?;
// Returns empty string if not configured
```

### Rules

1. **NEVER** use `reqwest::Client::new()` or `reqwest::Client::builder()` directly
2. **ALWAYS** use `http_client::client()` for requests that should respect proxy settings
3. Use `http_client::client_no_proxy()` only when you explicitly need to bypass proxy
4. **For non-HTTP proxy needs** (e.g., git operations, external CLI tools): Use `http_client::get_proxy_from_settings()` to retrieve the proxy URL and apply it appropriately (e.g., set environment variables like `HTTP_PROXY`/`HTTPS_PROXY`)

### Supported Proxy Formats

- HTTP: `http://proxy.example.com:8080`
- HTTP with auth: `http://user:pass@proxy.example.com:8080`
- SOCKS5: `socks5://proxy.example.com:1080`
- SOCKS5 with auth: `socks5://user:pass@proxy.example.com:1080`

### Files Using http_client

- `tauri/src/update.rs` - Update checking
- `tauri/src/settings/backup/webdav.rs` - WebDAV operations
- `tauri/src/coding/open_code/models_api.rs` - Provider model fetching
- `tauri/src/skills/installer.rs` - Git operations proxy
- `tauri/src/skills/commands.rs` - Git operations proxy

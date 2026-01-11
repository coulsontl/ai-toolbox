# AGENTS.md - AI Toolbox Development Guide

This document provides essential information for AI coding agents working on this project.

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
│   ├── features/           # Feature modules (daily, coding, settings)
│   ├── stores/             # Zustand state stores
│   ├── i18n/               # i18next localization
│   ├── constants/          # Module configurations
│   ├── hooks/              # Global hooks
│   ├── services/           # API services
│   └── types/              # Global type definitions
├── tauri/                  # Rust backend
│   ├── src/                # Rust source
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
4. **Theme**: Dark mode interface is reserved but not yet implemented
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

#### 3. Persistence Pattern (DELETE + CREATE)
To avoid SurrealDB versioning conflicts (`Invalid revision` errors) and deserialization failures:

1.  **Reads**: Always use **`SELECT * OMIT id`** when `id` is SurrealDB's default `Thing` type.
    *   **Why**: SurrealDB's default `id` field is a complex `Thing` object (e.g., `table:id`). Most simple Rust structs or generic `serde_json::Value` expect a string or number, leading to serialization errors like `invalid type: map, expected a string`.
    *   **Exception**: If you explicitly define `id` as `String` or `Int` in your schema, `OMIT id` is not required.
2.  **Updates**: Always use **`DELETE`** followed by **`CREATE`**.
    *   **Why**: SurrealDB uses MVCC (Multi-Version Concurrency Control). Direct `UPDATE`s on `serde_json::Value` types often trigger `Invalid revision` errors due to internal version mismatches.

```rust
// commands.rs
#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, DbState>) -> Result<AppSettings, String> {
    let db = state.0.lock().await;
    
    // CRITICAL: Use `OMIT id` to prevent deserialization errors of Thing type
    // If you need the ID, select it explicitly as a string: `SELECT *, string::from(id) as id_str ...`
    let mut result = db
        .query("SELECT * OMIT id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query settings: {}", e))?;
        
    let records: Vec<serde_json::Value> = result.take(0).map_err(|e| e.to_string())?;
    
    if let Some(record) = records.first() {
        // Use adapter for fault-tolerant conversion
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
    let json = adapter::to_db_value(&settings);
    
    // CRITICAL: Delete then Create to bypass versioning checks
    db.query("DELETE settings:`app`")
        .await
        .map_err(|e| format!("Failed to delete old record: {}", e))?;
        
    db.query("CREATE settings:`app` CONTENT $data")
        .bind(("data", json))
        .await
        .map_err(|e| format!("Failed to create record: {}", e))?;
        
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
```

### Rules

1. **NEVER** use `reqwest::Client::new()` or `reqwest::Client::builder()` directly
2. **ALWAYS** use `http_client::client()` for requests that should respect proxy settings
3. Use `http_client::client_no_proxy()` only when you explicitly need to bypass proxy

### Supported Proxy Formats

- HTTP: `http://proxy.example.com:8080`
- HTTP with auth: `http://user:pass@proxy.example.com:8080`
- SOCKS5: `socks5://proxy.example.com:1080`
- SOCKS5 with auth: `socks5://user:pass@proxy.example.com:1080`

### Files Using http_client

- `tauri/src/update.rs` - Update checking
- `tauri/src/settings/backup/webdav.rs` - WebDAV operations
- `tauri/src/coding/open_code/models_api.rs` - Provider model fetching

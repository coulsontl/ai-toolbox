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
```typescript
import { create } from 'zustand';
import { persist } from 'zustand/middleware';

interface StoreState {
  value: string;
  setValue: (value: string) => void;
}

export const useStore = create<StoreState>()(
  persist(
    (set) => ({
      value: '',
      setValue: (value) => set({ value }),
    }),
    { name: 'store-name' }
  )
);
```

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

### Zustand Store Pattern (Without Persistence)

Stores should call the service layer for data operations, not use localStorage:

```typescript
// Correct: Call backend API
export const useSettingsStore = create<SettingsState>()((set, get) => ({
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

// WRONG: Do not use persist middleware for data storage
// export const useStore = create()(persist(...)); // ❌ Not allowed
```

### Backend Command Pattern

All Tauri commands should interact with the database state:

```rust
#[tauri::command]
async fn get_settings(state: tauri::State<'_, DbState>) -> Result<AppSettings, String> {
    let db = state.0.lock().await;
    let result: Option<AppSettings> = db
        .select(("settings", "app"))
        .await
        .map_err(|e| format!("Failed to get settings: {}", e))?;
    Ok(result.unwrap_or_default())
}

#[tauri::command]
async fn save_settings(
    state: tauri::State<'_, DbState>,
    settings: AppSettings,
) -> Result<(), String> {
    let db = state.0.lock().await;
    let _: Option<AppSettings> = db
        .upsert(("settings", "app"))
        .content(settings)
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

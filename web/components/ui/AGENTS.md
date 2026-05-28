# UI Compatibility Layer

## Source of Truth

- Application code should import shared controls from `@/components/ui` and shared icons from `@/components/ui/icons`.
- Do not import `antd`, `@ant-design/icons`, or `@lobehub/icons` in application code.
- Radix primitives belong inside this local UI layer unless a feature has a narrow, well-justified custom interaction that cannot fit the shared API.

## Why

The app keeps an AntD-shaped component API to minimize migration churn across existing feature modules, but the runtime implementation is local React + Radix UI + Tailwind/CSS. This lets feature code stay simple while removing AntD as a dependency.

The visual direction is consumer-facing, simple, and Apple-like liquid glass: restrained surfaces, soft translucency, clear focus states, and no heavy enterprise-dashboard chrome.

## Key Flow

- Prefer adapting broad API or styling differences in `web/components/ui/` rather than touching many feature call sites.
- Keep static feedback surfaces (`message`, `notification`, `Modal.confirm`, `Popconfirm`) on the same local styled layer; do not fall back to browser-native `alert` or `confirm`.
- Preserve existing `.ant-*` compatibility class names only where existing module CSS still depends on them. New UI-layer styling should use `.ui-*` classes.

## Gotchas

- `ConfigProvider` and `theme` are compatibility exports, not a reason to reintroduce AntD.
- Use theme variables from `web/App.css` and local UI tokens in `ui.css`; avoid hardcoded colors in component styles.
- When adding a control, implement the smallest subset used by the app first. Do not clone a full third-party component library.
- Compatibility is behavioral, not only visual. Existing AntD-shaped call sites depend on details such as built-in `Form` rules, searchable `Select`, image preview, drag-and-drop upload, and `.ant-*` active-state classes; preserve those semantics in this layer before changing feature call sites.

## Minimal Validation

- `./node_modules/.bin/tsc --noEmit`
- `pnpm test`
- `pnpm run test:ui` for local UI compatibility regressions while iterating on `web/components/ui/`
- `NODE_OPTIONS=--max-old-space-size=4096 pnpm build` when frontend entry, shared UI, Tailwind, or CSS processing changes.
- `grep -R "from 'antd'\|from \"antd\"\|@ant-design/icons\|@lobehub/icons" -n package.json pnpm-lock.yaml web`

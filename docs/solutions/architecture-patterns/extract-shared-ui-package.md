---
title: Extract Shared React UI Components into @senara-solutions/ui Package
category: architecture-patterns
date: 2026-03-18
tags: [npm-workspaces, react-components, typescript, vite, github-packages, shared-libraries]
related_modules: [packages/ui, dashboard, .github/workflows/publish-ui.yml]
---

# Extract Shared UI Package

## Problem

The Mika dashboard contained 6 reusable React components (StatusBadge, Pagination, EmptyState,
CopyButton, MarkdownContent, TaskStatusBadge), 3 utility modules (formatTime, badges,
agentColors), and shared design tokens (color palette, fonts, scrollbar styles) that were
tightly coupled to the dashboard application. These could not be reused by other projects in
the organization. Dependencies like `react-markdown` and `remark-gfm` were also only needed
by the `MarkdownContent` component but listed as dashboard-level dependencies.

## Root Cause

The dashboard was built as a standalone SPA before multi-project reuse was considered. All
components lived in `dashboard/src/components/` and `dashboard/src/utils/` with relative
imports, making extraction impossible without restructuring.

## Solution

Extracted shared components into `@senara-solutions/ui` — a Vite library-mode npm package
published to GitHub Packages via npm workspaces.

### 1. Workspace Setup

Root `package.json` declares npm workspaces:
```json
{
  "private": true,
  "workspaces": ["packages/*", "dashboard"]
}
```

### 2. UI Package Structure

```
packages/ui/
├── src/
│   ├── index.ts              # Barrel export (6 components, 3 utils)
│   ├── theme.css             # Design tokens (@theme variables)
│   ├── components/           # StatusBadge, Pagination, EmptyState,
│   │                         # CopyButton, MarkdownContent, TaskStatusBadge
│   └── utils/                # badges, formatTime, agentColors
├── package.json              # @senara-solutions/ui, peer deps, exports map
├── vite.config.ts            # Library mode + vite-plugin-dts
├── tsconfig.json
├── .npmrc                    # Scoped registry → npm.pkg.github.com
└── .gitignore                # dist/
```

### 3. Key Configuration

**`package.json` exports map** — dual entry points:
```json
{
  "exports": {
    ".": { "import": "./dist/index.js", "types": "./dist/index.d.ts" },
    "./theme.css": "./src/theme.css"
  },
  "peerDependencies": {
    "lucide-react": ">=0.400.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "tailwindcss": "^4.0.0"
  },
  "dependencies": {
    "react-markdown": "^10.1.0",
    "remark-gfm": "^4.0.1"
  }
}
```

**Vite library build** — ES-only output with externals:
```typescript
export default defineConfig({
  plugins: [react(), dts({ include: ['src'] })],
  build: {
    lib: {
      entry: resolve(__dirname, 'src/index.ts'),
      formats: ['es'],
      fileName: 'index',
    },
    rollupOptions: {
      external: ['react', 'react-dom', 'react/jsx-runtime', 'lucide-react'],
    },
  },
})
```

### 4. Theme CSS Extraction

Design tokens moved from `dashboard/src/index.css` to `packages/ui/src/theme.css`:
```css
@theme {
  --font-sans: "Plus Jakarta Sans", system-ui, -apple-system, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;
  --color-bg: #0d0f12;
  --color-bg-card: #151820;
  --color-accent: #7c6af7;
  --color-accent-light: #9d8fff;
  --color-heading: #e8ecf2;
  --color-muted: #a0a8b8;
}
```

Dashboard imports it:
```css
@import "tailwindcss";
@import '@senara-solutions/ui/theme.css';
```

### 5. Dashboard Import Migration

Before:
```typescript
import StatusBadge from '../components/StatusBadge.tsx'
import Pagination from '../components/Pagination.tsx'
import { formatTimestamp } from '../utils/formatTime.ts'
```

After:
```typescript
import { StatusBadge, Pagination, formatTimestamp } from '@senara-solutions/ui'
```

Dashboard depends on `"@senara-solutions/ui": "*"` — workspace resolution uses the local
package during development; published version from GitHub Packages in production.

### 6. CI/CD — Auto-Publish Workflow

`.github/workflows/publish-ui.yml` publishes on push to main when `packages/ui/**` changes:

1. Build the package (`npm run build -w packages/ui`)
2. Compare local version against published version on GitHub Packages
3. Skip if already published; publish if version bumped
4. Auth via `GITHUB_TOKEN` (no separate npm token needed)

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Peer deps for React/Tailwind | Avoids duplication in consumer bundles |
| Direct deps for react-markdown | Only MarkdownContent needs it; avoids consumer burden |
| GitHub Packages over npm.org | Internal distribution; scoped package |
| ES-only output | Modern tooling; smaller bundle; tree-shakeable |
| Separate theme.css export | Consumers can use components without theme |
| Wildcard version in dashboard | Workspace resolution handles local dev seamlessly |

## Prevention

### When Modifying the UI Package

- [ ] New components must be exported in `packages/ui/src/index.ts` barrel
- [ ] Run `npm run build -w packages/ui` and verify `dist/index.d.ts` contains new exports
- [ ] Peer dependency versions must be compatible with dashboard's versions
- [ ] Theme variables used by components must be defined in `theme.css`
- [ ] Bump `version` in `packages/ui/package.json` before merging to main
- [ ] Never import from `@senara-solutions/ui/src/...` — always use barrel

### Common Pitfalls

- **Workspace hoisting masks missing exports** — works locally but fails in CI. Always verify
  barrel exports match dashboard imports
- **Stale build artifacts** — after adding to `index.ts`, run full build (watch mode may not
  regenerate `.d.ts`)
- **Orphaned consumer dependencies** — when moving a component to UI, remove its deps from
  dashboard's `package.json`
- **GitHub Packages auth** — local dev requires PAT with `read:packages` scope in `~/.npmrc`
- **Theme CSS ordering** — consumer must import theme before or alongside Tailwind for
  `@theme` variables to resolve

### Related Code Review Findings

- Todo #689: Unused layout abstractions (AppShell/Sidebar exported but not consumed)
- Todo #690: publish-ui.yml action refs need SHA pinning
- Todo #692: Orphaned react-markdown/remark-gfm in dashboard deps
- Todo #693: Dual declaration generation (vite-plugin-dts + tsc)

## Related

- [Multi-Provider LLM Trait Abstraction](multi-provider-llm-trait-abstraction.md) — similar
  extraction pattern for Rust traits
- [Config Key Rename Across Layers](config-key-rename-across-layers.md) — checklist methodology
  reused for verifying import migration completeness

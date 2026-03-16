---
title: "feat: Add public documentation site with Starlight"
type: feat
status: completed
date: 2026-03-16
origin: docs/brainstorms/2026-03-16-starlight-docs-site-brainstorm.md
---

# feat: Add public documentation site with Starlight

## Overview

Add a public-facing documentation site for Mika at `mika-docs.senara-solutions.ai`, built with [Starlight](https://starlight.astro.build/) (Astro-based static site generator). The site auto-builds and deploys via AWS Amplify on push to `main`. Content is sourced from the existing `docs/` directory (single source of truth) via a build-time copy script.

## Problem Statement / Motivation

Mika has comprehensive markdown documentation in `docs/` (7 user guides + 6 ADRs), but no public-facing site. Users must navigate raw GitHub markdown. A branded documentation site improves discoverability, provides search (Pagefind), and presents Mika as a polished product — not just a codebase.

(see brainstorm: [docs/brainstorms/2026-03-16-starlight-docs-site-brainstorm.md](../brainstorms/2026-03-16-starlight-docs-site-brainstorm.md))

## Proposed Solution

### Architecture

```
mika/
├── docs/                          # Source of truth (unchanged)
│   ├── architecture.md
│   ├── configuration.md
│   ├── deployment.md
│   ├── getting-started.md
│   ├── runtime-structure.md
│   ├── skills.md
│   ├── slash-commands.md
│   ├── adr/
│   │   ├── 001-axum-http-server-architecture.md
│   │   ├── ...
│   │   └── 006-git-based-skills-marketplace.md
│   └── openapi/
├── docs-site/                     # NEW — Starlight project
│   ├── astro.config.mjs
│   ├── package.json
│   ├── package-lock.json
│   ├── tsconfig.json
│   ├── amplify.yml
│   ├── src/
│   │   ├── content.config.ts
│   │   ├── content/
│   │   │   └── docs/              # .gitignored — populated at build time
│   │   │       ├── *.md           # copies of docs/*.md
│   │   │       └── adr/           # copies of docs/adr/*.md
│   │   ├── assets/
│   │   │   └── mika-logo.svg      # (or png)
│   │   └── styles/
│   │       └── custom.css         # Dark theme overrides
│   └── public/
│       └── favicon.svg
└── ...
```

### Content Strategy: Build-Time Copy (not symlinks)

**Decision:** Use a `prebuild` script that copies `docs/*.md` and `docs/adr/*.md` into `docs-site/src/content/docs/` at build time. The copied directory is `.gitignore`d.

**Why not symlinks:**
- Symlinks break on Windows contributors (requires `core.symlinks=true` + NTFS permissions)
- Fragile: moving the `docs-site/` directory breaks relative symlink targets silently
- `git archive` does not follow symlinks (some CI systems use this)
- AWS Amplify symlink support is an assumption that could fail

**Why build-time copy works:**
- Robust across all platforms and CI environments
- The `prebuild` script can selectively copy only user docs + ADRs (excluding plans, brainstorms, solutions)
- Can inject/transform frontmatter during the copy step if needed
- `astro dev` runs `prebuild` first, so local development works identically

```json
// docs-site/package.json scripts
{
  "prebuild": "node scripts/sync-docs.mjs",
  "dev": "npm run prebuild && astro dev",
  "build": "npm run prebuild && astro build",
  "preview": "npm run prebuild && astro preview"
}
```

### Internal Link Strategy

**Problem:** Existing docs use relative `.md` links (e.g., `[Configuration](configuration.md)`, `[ADR-003](adr/003-layer3-hybrid-vector-search.md)`). These must work in both GitHub preview and Starlight.

**Solution:** Keep the `adr/` path name in Starlight (not rename to `decisions/`). Starlight resolves `.md` extension links when files are in the content collection. The `prebuild` copy script preserves the exact directory structure (`adr/` → `adr/`), so all existing relative links work without modification.

### Frontmatter Injection

**Problem:** Existing docs have no YAML frontmatter. Starlight requires at minimum a `title` field.

**Solution:** Add frontmatter directly to the source `docs/*.md` files. This is the simplest approach:
- GitHub renders frontmatter natively (hides it in preview)
- `build.rs` / `include_str!` will include frontmatter in the embedded string
- Add a frontmatter-stripping utility in `mika-agent` for the `get_documentation` handler

Minimal frontmatter to add:

```yaml
---
title: Getting Started
description: Install Mika and run your first conversation
---
```

### Sidebar Configuration

Explicit ordering in `astro.config.mjs` — "Getting Started" first, then logical reading order:

```js
sidebar: [
  { slug: 'getting-started', label: 'Getting Started' },
  { slug: 'architecture', label: 'Architecture' },
  { slug: 'configuration', label: 'Configuration' },
  { slug: 'runtime-structure', label: 'Runtime Structure' },
  { slug: 'deployment', label: 'Deployment' },
  { slug: 'skills', label: 'Skills' },
  { slug: 'slash-commands', label: 'Slash Commands' },
  {
    label: 'Architecture Decisions',
    collapsed: true,
    autogenerate: { directory: 'adr' },
  },
]
```

### Dark Theme

Pure CSS custom properties via `src/styles/custom.css` — no Tailwind dependency needed for the docs site. Match the Mika brand colors from `site/`.

### Search

Pagefind — zero configuration. Built-in to Starlight, runs at build time, instant client-side search.

### AWS Amplify Deployment

```yaml
# docs-site/amplify.yml
version: 1
applications:
  - appRoot: docs-site
    frontend:
      phases:
        preBuild:
          commands:
            - nvm install 22
            - nvm use 22
            - npm ci
        build:
          commands:
            - npm run build
      artifacts:
        baseDirectory: dist
        files:
          - '**/*'
      cache:
        paths:
          - node_modules/**/*
```

Domain: `mika-docs.senara-solutions.ai` — configured in Amplify Console with ACM cert + CNAME.

## Implementation Phases

### Phase 1: Scaffold Starlight project

**Files to create:**

- `docs-site/package.json` — Astro + Starlight dependencies
- `docs-site/astro.config.mjs` — Starlight config with sidebar, dark theme, site URL
- `docs-site/tsconfig.json` — Astro TypeScript config
- `docs-site/src/content.config.ts` — Content collection registration
- `docs-site/src/styles/custom.css` — Dark theme CSS custom properties
- `docs-site/src/content/docs/.gitkeep` — Placeholder (content is build-time generated)
- `docs-site/.gitignore` — Ignore `node_modules/`, `dist/`, `.astro/`, `src/content/docs/` (except `.gitkeep`)
- `docs-site/scripts/sync-docs.mjs` — Prebuild script: copies `../docs/*.md` and `../docs/adr/*.md` into `src/content/docs/`
- `docs-site/public/favicon.svg` — Mika favicon

**Acceptance criteria:**
- [x] `cd docs-site && npm install && npm run dev` serves docs locally
- [x] All 7 user docs render with correct content
- [x] All 6 ADRs render under "Architecture Decisions" sidebar group
- [x] Pagefind search works (type `/` or `Ctrl+K`)
- [x] Dark theme applied

### Phase 2: Add frontmatter to source docs

**Files to modify:**

- `docs/getting-started.md` — Add `title` + `description` frontmatter
- `docs/architecture.md` — Add `title` + `description` frontmatter
- `docs/configuration.md` — Add `title` + `description` frontmatter
- `docs/deployment.md` — Add `title` + `description` frontmatter
- `docs/runtime-structure.md` — Add `title` + `description` frontmatter
- `docs/skills.md` — Add `title` + `description` frontmatter
- `docs/slash-commands.md` — Add `title` + `description` frontmatter
- `docs/adr/001-axum-http-server-architecture.md` through `docs/adr/006-git-based-skills-marketplace.md` — Add `title` frontmatter

**Acceptance criteria:**
- [x] All docs have valid YAML frontmatter with `title`
- [x] GitHub preview still renders docs correctly (frontmatter hidden)
- [x] `cargo build -p mika-agent` succeeds (build.rs unchanged)
- [x] `cargo test -p mika-agent` passes

### Phase 3: Handle frontmatter in agent doc embedding

**Files to modify:**

- `crates/mika-agent/src/skills/builtin_handlers.rs` — Strip YAML frontmatter from `get_documentation` output before returning to user

**Implementation:** If the doc content starts with `---\n`, find the second `---\n` and strip everything before it (inclusive).

**Acceptance criteria:**
- [x] `get_documentation` returns content without frontmatter preamble
- [x] Existing tests pass
- [x] New test: doc with frontmatter → frontmatter stripped
- [x] New test: doc without frontmatter → unchanged

### Phase 4: AWS Amplify deployment config

**Files to create:**

- `docs-site/amplify.yml` — Amplify build spec (monorepo config, Node 22, `npm run build`, artifacts from `dist/`)

**Manual steps (not in code):**

- [ ] Connect GitHub repo in Amplify Console
- [ ] Set monorepo app root to `docs-site`
- [ ] Add custom domain `mika-docs.senara-solutions.ai`
- [ ] Configure DNS CNAME pointing to Amplify distribution
- [ ] Verify SSL cert provisioned by Amplify

**Acceptance criteria:**
- [ ] Push to `main` triggers Amplify build
- [ ] Site accessible at `mika-docs.senara-solutions.ai`
- [ ] All pages render correctly in production

### Phase 5: CI validation (optional but recommended)

**Files to modify:**

- `.github/workflows/ci.yml` — Add `docs-site` job (path-filtered to `docs/**` and `docs-site/**`)

**Pattern:** Follow existing `dashboard` job structure — pinned action SHAs, `defaults.run.working-directory: docs-site`, Node 22, `npm ci && npm run build`.

**Acceptance criteria:**
- [ ] PRs touching `docs/` or `docs-site/` trigger the Starlight build check
- [ ] PRs touching only Rust code do NOT trigger the docs build
- [ ] Build failure blocks merge

## Technical Considerations

### Frontmatter and `---` horizontal rules

The existing docs use `---` extensively as section dividers. When frontmatter is added, Starlight/Astro will parse the first `---...---` block as YAML. Subsequent `---` lines in the body are treated as markdown horizontal rules. This works correctly as long as the frontmatter block is well-formed. No action needed beyond ensuring valid frontmatter.

### Dual-use link compatibility

All internal links use relative paths with `.md` extensions (e.g., `[Skills](skills.md)`). This works in both GitHub preview and Starlight's content collection. By keeping the `adr/` directory name (not renaming to `decisions/`), the 4 cross-references in `architecture.md` to ADR files also work in both contexts.

### New doc process

When a contributor adds a new user-facing doc to `docs/`:
1. Add the file with YAML frontmatter (`title` required)
2. The `sync-docs.mjs` script auto-copies all `docs/*.md` files — new docs appear automatically
3. If sidebar ordering matters, add the slug to `astro.config.mjs` sidebar config
4. If the doc should be embedded in the agent, update `build.rs`, `builtin_handlers.rs`, and `sync-agent-docs.sh`

The Starlight site picks up new docs automatically (step 2), but sidebar placement requires a config change (step 3).

### Amplify build scope

Amplify rebuilds on every push to `main`, including pure Rust changes. At Mika's current merge frequency this is acceptable. If build minutes become a concern, Amplify supports path-based build filtering via `amplify.yml` `triggers` configuration.

## System-Wide Impact

- **`build.rs` (mika-agent):** Unchanged. Copies docs with frontmatter included — the embedded strings will contain frontmatter.
- **`builtin_handlers.rs`:** Modified to strip frontmatter before returning doc content.
- **`/mika-doc-audit`:** Unchanged. Continues updating `docs/*.md`; Amplify picks up changes on merge.
- **`scripts/sync-agent-docs.sh`:** Unchanged. Syncs to crate-local fallback for crates.io.
- **CI (`ci.yml`):** New path-filtered job for Starlight build validation.
- **No breaking changes** to existing functionality.

## Acceptance Criteria

- [ ] `docs-site/` directory with working Starlight project
- [ ] All 7 user docs + 6 ADRs render on the site
- [ ] Dark theme matching Mika brand
- [ ] Pagefind search functional
- [ ] Internal links between docs work (no 404s)
- [ ] `get_documentation` agent tool strips frontmatter
- [ ] `amplify.yml` configured for monorepo build
- [ ] CI job validates Starlight build on doc-related PRs
- [ ] Site live at `mika-docs.senara-solutions.ai` (post manual Amplify setup)

## Dependencies & Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Amplify symlink/path issues in monorepo | Low | Using build-time copy, not symlinks |
| Starlight `.md` link resolution fails | Low | Test in Phase 1; fallback: remark plugin to strip extensions |
| Frontmatter breaks `get_documentation` output | Certain | Phase 3 adds stripping logic |
| Contributors forget sidebar config for new docs | Medium | Document process in CLAUDE.md |
| Amplify build minutes on non-doc changes | Low | Monitor; add path filter if needed |

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-16-starlight-docs-site-brainstorm.md](../brainstorms/2026-03-16-starlight-docs-site-brainstorm.md) — Key decisions: Starlight over mdBook, user docs + ADRs scope, AWS Amplify hosting, Amplify auto-build

### Internal References

- Dashboard tooling pattern: `dashboard/package.json`, `dashboard/vite.config.ts`
- Doc embedding pipeline: `crates/mika-agent/build.rs:1-60`
- Doc serving handler: `crates/mika-agent/src/skills/builtin_handlers.rs`
- CI workflow: `.github/workflows/ci.yml`
- Doc audit command: `.claude/commands/mika-doc-audit.md`

### External References

- [Starlight documentation](https://starlight.astro.build/)
- [Starlight sidebar guide](https://starlight.astro.build/guides/sidebar/)
- [Starlight CSS & theming](https://starlight.astro.build/guides/css-and-tailwind/)
- [Astro AWS Amplify deployment](https://docs.astro.build/en/guides/deploy/aws/)
- [AWS Amplify monorepo configuration](https://docs.aws.amazon.com/amplify/latest/userguide/monorepo-configuration.html)

### Related Work

- GitHub issue #175: Interactive OpenAPI docs (deferred follow-up)

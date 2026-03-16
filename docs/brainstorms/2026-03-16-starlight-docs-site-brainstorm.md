# Brainstorm: Public Documentation Site with Starlight

**Date:** 2026-03-16
**Status:** Decided

## What We're Building

A public-facing documentation site for Mika at `mika-docs.senara-solutions.ai`, built with [Starlight](https://starlight.astro.build/) (Astro-based static site generator). The site will be auto-built and deployed via AWS Amplify on push to `main`.

## Why Starlight

- **Product feel over library feel.** Mika is a product with a brand, not a Rust library. mdBook looks like a Rust book; Starlight gives full CSS/Tailwind control to match the Mika website's dark theme.
- **Ecosystem alignment.** Already using Node.js + Tailwind for the React dashboard. Same toolchain, no new runtime dependencies.
- **Superior search.** Pagefind built-in with zero configuration — instant, static, low-bandwidth.
- **Future-proof.** i18n, versioning, component islands if needed later.

### Frameworks Eliminated

| Framework | Reason |
|---|---|
| mdBook | Looks generic, limited customization, basic search |
| MkDocs Material | Entering maintenance mode (Nov 2025), forced migration to Zensical |
| Docusaurus | Too heavy, React/MDX overhead, JSX clashes with Rust generics in code blocks |
| VitePress | Vue-ecosystem-specific, no advantage over Starlight |

## Key Decisions

1. **Framework:** Starlight (Astro)
2. **Content scope:** 7 user-facing docs + 6 ADRs (no brainstorms, plans, or solutions)
3. **Hosting:** AWS Amplify at `mika-docs.senara-solutions.ai`
4. **Build trigger:** Amplify auto-build on push to `main` (no GitHub Actions needed for docs)
5. **OpenAPI rendering:** Deferred — raw YAML reference for now, interactive Swagger/Redoc tracked as a separate issue
6. **Doc freshness:** `/mika-doc-audit` continues updating markdown source; Amplify rebuilds automatically

## Content Structure

```
docs-site/
├── astro.config.mjs
├── package.json
└── src/
    └── content/
        └── docs/
            ├── getting-started.md
            ├── architecture.md
            ├── configuration.md
            ├── deployment.md
            ├── runtime-structure.md
            ├── skills.md
            ├── slash-commands.md
            └── decisions/
                ├── 001-axum-http-server-architecture.md
                ├── 002-...
                └── ...
```

Starlight reads from `src/content/docs/`. The source markdown files in `docs/` remain the single source of truth — either symlinked or copied at build time.

## Integration Points

- **`/mika-doc-audit`** — Updates `docs/*.md` as today. No changes needed since Amplify watches the repo.
- **`/mika` workflow** — No changes needed. Doc audit already runs before commit; Amplify picks up merged changes.
- **CI (`ci.yml`)** — Optional: add a Starlight build check to catch broken docs on PRs.
- **`build.rs` (mika-agent)** — Unchanged. Continues embedding docs into the binary from `docs/`.

## Open Questions

None — all decisions resolved during brainstorming.

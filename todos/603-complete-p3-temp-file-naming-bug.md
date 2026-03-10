---
status: pending
priority: p3
issue_id: 603
tags: [code-review, quality]
dependencies: []
---

# Temp file naming produces .env.env.tmp instead of .env.tmp

## Problem Statement

`set_env_var` uses `env_path.with_extension("env.tmp")` which, for a path ending in `.env`, produces `.env.env.tmp` due to Rust's `Path` treating dotfiles as having no extension.

## Proposed Solutions

Replace with `env_path.with_file_name(".env.tmp")` for the intended filename.

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Temp file is named `.env.tmp`, not `.env.env.tmp`

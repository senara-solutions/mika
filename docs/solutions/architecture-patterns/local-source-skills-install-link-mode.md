---
title: "Local source support for mika skills install with --link mode"
category: architecture-patterns
date: 2026-03-19
tags: [skills, marketplace, symlink, local-install, cli]
modules: [mika-agent/skills, mika-cli/commands/skills]
issue: "#210"
---

# Local Source Support for mika skills install with --link Mode

## Problem

The marketplace install flow only accepted git URLs, forcing skill authors to commit, push, and run mika skills update for every edit during local development.

## Root Cause

resolve_url() in git.rs was hardcoded to only produce git URLs. The entire install pipeline assumed git: clone, scan, copy, lock. No code path existed for local filesystem sources.

## Solution

### 1. Source Resolution Refactor

Introduced SourceKind enum and resolve_source() in git.rs. Previous resolve_url() renamed to resolve_git_url() (private). Resolution: file:// URI to Local, absolute path to Local, else to Git.

### 2. Lock File Schema Extension

Added linked: bool to MarketplaceEntry with #[serde(default)] for backward compat. Local sources use url = "file:///..." and commit = "".

### 3. Three-Way Install Dispatch

CLI routes on SourceKind: Git to clone/scan/copy/lock, Local to scan in-place/copy/lock, Local+link to scan in-place/symlink/lock.

### 4. Symlink-Aware Operations

- Uninstall: symlink_metadata() to detect, remove_file() for symlinks
- Update: Linked to LinkedNoOp, local snapshot to re-copy, git to re-clone
- Scan: scan_skills_dir() detects broken symlinks with warnings

### 5. Security Guards

- Self-referential path rejection (source inside skills_dir)
- Absolute symlinks only (canonicalized)
- Enhanced exec handler warning for --link mode

## Key Design Decisions

1. linked: bool over source_type: String - simpler, backward-compatible, local vs git detectable from URL scheme
2. UpdateResult changed to enum - Updated and LinkedNoOp - forces explicit handling
3. Reuse scan_repo_for_skills() - same scanner for git clones and local dirs

## Prevention / Best Practices

- Use #[serde(default)] when adding fields to serialized structs for backward compat
- Canonicalize paths and create absolute symlinks to prevent CWD-dependent breakage
- Use symlink_metadata() not metadata() when entry might be a symlink

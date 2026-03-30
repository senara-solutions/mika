# Changelog

All notable changes to this project will be documented in this file.
## [0.2.0](https://github.com/senara-solutions/mika/releases/tag/v0.2.0) — 2026-03-25

### Added

- *(a2a)* implement A2A protocol v0.3 for agent-to-agent communication
- add /mika-issue and /mika-issues Claude Code commands
- add automated release system with GitHub binary downloads

### Changed

- **BREAKING** unify LLM API key — remove MIKA_ANTHROPIC_API_KEY

### Documentation

- add pre-1.0 breaking changes policy
- update documentation for recent changes
- update documentation for unified task engine completion
- update test count to ~837 after code review fixes
- update documentation for team TUI mode changes
- update documentation for skills marketplace changes
- add contributing guide with Claude Code workflow
- update documentation for shell-like autocompletion
- update documentation for agent management tools
- update documentation for release system and rustls migration

### Fixed

- *(a2a)* resolve code review findings from A2A protocol PR
- *(a2a)* add missing workspace metadata to fix cargo-deny license check

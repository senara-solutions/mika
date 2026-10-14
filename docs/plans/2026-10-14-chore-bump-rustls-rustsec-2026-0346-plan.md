---
date: 2026-10-14
type: chore
---

# Plan: bump rustls to 0.23.46 (RUSTSEC-2026-0346, -0347, -0348, -0349)

## Problem

The required Security job fails on every open PR since 2026-10-14: `cargo audit` flags `rustls 0.23.45` with four advisories published that day (medium: `Debug` impls expose TLS secrets; low: Brotli certificate decompression allocation, ServerHello downgrade, and a fourth). All four are fixed in 0.23.46.

## Fix

- `Cargo.lock`: `cargo update -p rustls --precise 0.23.46`. No other crate moves.

## Acceptance criteria

- AC1: `Cargo.lock` diff is the rustls version and checksum only; `cargo metadata --locked` succeeds.
- AC2: `cargo audit` reports no rustls advisory.
- AC3: `cargo check --workspace --locked` succeeds.
- AC4: The Security job is green on the PR.

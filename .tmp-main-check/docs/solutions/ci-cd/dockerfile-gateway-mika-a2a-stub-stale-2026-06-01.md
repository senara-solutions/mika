---
module: ci-cd
tags: [docker, mika-gateway, mika-a2a, workspace-stub, ci-drift]
problem_type: build-failure
category: ci-cd
---

# Dockerfile.gateway stubbed mika-a2a empty after gateway started using it

## Problem

`Docker Build (Dockerfile.gateway)` failed compiling the gateway:

```
error[E0432]: unresolved import `mika_a2a::jsonrpc`
error: could not compile `mika-gateway` (bin "mika-gateway") due to 1 previous error
```

Masked until mika#1366 (uppercase tag) was fixed — the tag error aborted before any build ran.

## Root cause

To speed builds, `Dockerfile.gateway` stubs workspace members the gateway "doesn't build" with
an empty lib:

```dockerfile
COPY crates/mika-a2a/Cargo.toml crates/mika-a2a/Cargo.toml
RUN mkdir -p crates/mika-a2a/src && echo "" > crates/mika-a2a/src/lib.rs   # empty stub
```

That stub was valid only while mika-gateway didn't reference mika-a2a's code. The gateway now
genuinely depends on `mika_a2a::jsonrpc` (the A2A proxy), so the empty `lib.rs` makes the import
unresolvable → E0432.

## Fix

Copy the real crate (Dockerfile.agent already does this for mika-a2a):

```dockerfile
COPY crates/mika-a2a/ crates/mika-a2a/
```

Verified with a real `docker build -f Dockerfile.gateway .` → mika-gateway compiles and the image
exports.

## Lesson

A "dummy stub for workspace members not being built" is a standing liability: it silently breaks
the moment the built crate adds a real dependency on the stubbed one. When a Dockerfile stubs a
workspace member, that stub must be revisited whenever inter-crate dependencies change. Prefer
copying the real crate unless the build-time win is measured and the dependency boundary is
enforced (e.g. a no-`mika-a2a`-import lint on mika-gateway). This was the third masked failure in
a chain behind the same broken Docker tag (mika#1366 → #1368 → #1370) — a reminder that a
broken-early CI step hides every failure downstream of it.

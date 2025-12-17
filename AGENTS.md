# AGENTS.md — DbMiru (Spec-driven development)

## What is DbMiru?

DbMiru is a lightweight desktop database client built with Rust + gpui.
It starts with PostgreSQL support, but must be designed to support multiple databases in the future.

## Product goals (MVP)

- Manage connection profiles (create/edit/delete)
- Connect to a database and run SQL
- Display results in a table
- Show errors clearly and never crash

## Non-goals (early milestones)

- Full pgAdmin feature parity
- Visual query builder, ER diagrams
- Administration features (backup/restore/roles/monitoring)
- Cloud sync and collaboration

## Key principles

- Spec-first: update docs before changing behavior.
- Small increments: every milestone must be demoable.
- Never block the UI thread.
- Security: never store plaintext passwords, never log secrets.

## Tech stack (initial)

- Rust (stable)
- UI: gpui
- Async: tokio
- PostgreSQL: tokio-postgres (initial)
- Config: serde + toml (or json)
- Logging: tracing + tracing-subscriber
- Errors: thiserror (anyhow allowed only at app boundaries)

## Architecture rules (must follow)

- UI must not call database driver APIs directly.
  UI → Core commands → DB adapter (async) → Core state → UI render
- Keep a single source of truth: AppState.
- Long tasks should be cancellable where practical.
- Do not block UI rendering with synchronous I/O.

## Workspace strategy

- Start as a single crate.
- Evaluate workspace split during M2 (when boundaries become clear).
- Introduce Cargo workspace in M3 if DB abstraction stabilizes.

## Quality gates

- cargo fmt
- cargo clippy (no warnings; allow-list only if justified)
- Basic unit tests for non-UI logic where feasible
- Manual smoke checklist per milestone (docs/milestones.md)
- Docs must be updated with behavior changes

## Agent (Codex) workflow

For each task:

1. Read AGENTS.md + relevant docs + current code.
2. Propose a short plan + list of files to change.
3. Implement minimal changes that satisfy acceptance criteria.
4. Self-review: fmt/clippy/tests; update docs if behavior changed.
5. Output a short "How to verify" checklist.

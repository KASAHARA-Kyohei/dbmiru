# Architecture — DbMiru

## High-level layers

- UI (gpui): rendering, user interaction
- Core: state + commands + view-model-ish logic
- DB: database adapters (initially PostgreSQL)
- Storage: persistence (profiles, history)

## Data flow

User action
→ UI event
→ Core command
→ async DB operation
→ Core state update
→ UI re-render

## State management

- Single source of truth: `AppState`
- UI renders from state; UI does not own business logic
- Avoid global mutable state outside `AppState`

## Database access

- Initial: PostgreSQL adapter using tokio-postgres
- Future: trait-based adapters for multiple databases

## Metadata + schema exploration (M2)

- DB worker must expose async commands for schemas, tables, columns, and previews.
- UI triggers these commands through `DbSessionHandle` (no direct Postgres calls in UI).
- Metadata responses flow back as `DbEvent` variants and update the schema browser state.

## Secret storage (M2)

- Wrap OS keychain / credential manager behind a `SecretStore`.
- Persist only encrypted/OS-managed secrets; `profiles.json` stores metadata (e.g., `remember_password`) but never raw passwords.
- Missing/failed keychain operations should degrade gracefully (prompt user to re-enter password).

## Config directory

- Use `directories::BaseDirs` to locate the OS-specific config root.
- Sub-directory names:
  - macOS: `~/Library/Application Support/DbMiru`
  - Linux: `~/.config/dbmiru`
  - Windows: `%APPDATA%/DbMiru`
- Initialize the directory on startup so future storage layers (profiles, history) have a known location.

## Error handling

- Use structured error types (thiserror)
- Map errors to user-friendly messages in UI
- Never panic on expected failures (connect timeout, bad SQL, etc.)

## Workspace plan

- Single crate until boundaries become clear (M0–M1)
- M2: document boundaries and decision
- M3: convert to workspace if DB abstraction is introduced

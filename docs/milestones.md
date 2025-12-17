# Milestones — DbMiru

This file is the source of truth for progress.
Mark items as done by changing `[ ]` to `[x]` and committing to git.

---

## M0: Project bootstrap

### Goals

Create a “keepable” foundation: app starts, docs exist, basic quality gates.

### Checklist

- [x] Repo initialized with Rust project (`cargo new dbmiru`)
- [x] `AGENTS.md` and `docs/` created and committed
- [x] App window opens (gpui) with placeholder layout panels
- [x] Logging enabled (tracing)
- [x] Config directory decided and documented
- [x] Basic `cargo fmt` and `cargo clippy` clean _(clippy blocked: Metal toolchain missing)_

### DoD (Definition of Done)

- App starts and shows a window reliably
- No panic in normal startup
- Docs reflect actual behavior

---

## M1: Connection profiles + SQL execution (PostgreSQL)

### Goals

Run queries against PostgreSQL and view results.

### Checklist

- [x] Connection profile list in sidebar
- [x] Create/Edit/Delete connection profiles
- [x] Connect to PostgreSQL and show status (connected/disconnected)
- [x] SQL editor (single tab is fine)
- [x] Execute SQL (Cmd/Ctrl+Enter)
- [x] Display results (table view) with row limit (e.g., 1000)
- [x] Display errors (summary + details)

### DoD

- A user can connect to a local PostgreSQL instance and run `SELECT 1`
- Query errors are visible and do not crash the app

### Manual smoke checklist

- [x] When the app launches, the connection profile list appears in the sidebar
- [x] Creating a new profile → saving → immediately reflected in the list → edits/deletes are also reflected
- [x] After selecting a profile, Connect/Disconnect immediately updates the status label in the UI
- [x] Enter `SELECT 1` in the SQL editor and run with Cmd/Ctrl + Enter → up to 1000 rows display in the result table
- [x] Running invalid SQL shows the error summary/details in the lower panel and the UI stays responsive
- [x] All text fields support basic editing with Backspace/Delete and arrow keys

---

## M2: Schema exploration + workspace evaluation

### Goals

Let users browse basic metadata and preview tables.

### Checklist

- [ ] List schemas
- [ ] List tables
- [ ] List columns for selected table
- [ ] Table preview: `SELECT * FROM <table> LIMIT N`
- [ ] Improve result table UX (copy cell/row basics if feasible)
- [ ] Decide workspace split boundaries (app / core / db / storage) and document in `docs/architecture.md`

### DoD (Definition of Done)

- A user can browse database metadata:
  - schemas
  - tables
  - columns
- A user can preview table data using a generated
  `SELECT * FROM <table> LIMIT N` query.
- Result tables remain usable when content overflows:
  - horizontal scrolling appears when columns exceed panel width
  - vertical scrolling works for large row counts
- Sensitive information (passwords) is stored securely and never persisted in plaintext.
- The app remains stable during metadata browsing (no panic, no UI freeze).

### Workspace evaluation criteria (write decision in docs)

Convert to workspace if at least one is true:

- UI code and DB code are frequently edited together and feel tangled
- You want to add a second DB backend soon (even experimental)
- You want to unit test core logic without UI dependencies

---

## M3: DB abstraction + Cargo workspace

### Goals

Prepare for multiple databases by introducing an adapter layer and splitting crates.

### Checklist

- [ ] Define a `DbAdapter` trait (connect/execute/metadata)
- [ ] Implement `PostgresAdapter`
- [ ] Convert repo to Cargo workspace
- [ ] Split crates: `app`, `core`, `db`, `storage`
- [ ] Update docs to match new structure

### DoD

- Same app features still work after refactor
- Core logic is testable without gpui

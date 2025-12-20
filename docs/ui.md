# UI — DbMiru

## Layout (M2)

- Left: Connection list (profiles, connection status)
- Center top: Tab bar (`Schema Browser`, `SQL Editor`)
- Tab `Schema Browser`: display schemas → tables → columns → preview vertically
- Tab `SQL Editor`: editor + Run button, results panel below the editor

## Interactions (MVP)

- Select a connection profile → connect
- Reorder connection profiles with per-item Up/Down controls (manual order, persisted)
- Write SQL → execute
- Results appear in the SQL tab result panel
- Errors appear inline (connection panel / editor panel / schema browser)

## Visual style (M3)

- Canvas background: deep navy (`#040715`) with generous padding; panels float as frosted cards (`#0a0f1d` / `#11182a`) with subtle borders.
- Accent color: violet (`#8B5CF6` → hover `#7C3AED`) for primary actions (New/Run/Connect) and tab focus.
- Text: base `#F4F5FB`, muted labels `#94A3C4`; table headers use the highlight surface and bright text for contrast.
- Buttons are pill-shaped with hover transitions; destructive actions reuse the coral danger color to stay consistent across the app.

## Schema browser (M2)

- After a successful connection, automatically fetch the schema list and auto-select the first schema/table pair
- Show up to 5 entries (roughly 25% of window height) for schema/table/column lists; beyond that, scroll within the list
- Right-click copies schema/table names; left-click copies column names
- When a table is selected, show both the column list and a preview (`SELECT * ... LIMIT 50`) in the same tab
- In preview tables, keep the column header visible while scrolling vertically (sticky header)
- Metadata fetch errors appear at the bottom of the schema browser without crashing the UI

## SQL editor tab

- Show the SQL input, Run button, and execution status
- Display query results and errors in the lower panel inside the tab

## Shortcuts

- Cmd/Ctrl + Enter: execute query
- Cmd/Ctrl + W: close tab (when tabs exist)

## UX rules

- Show a running indicator during connect/execute
- Disable execute while a query is running
- Always show feedback (success row count or error message)

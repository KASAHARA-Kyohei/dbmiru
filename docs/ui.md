# UI — DbMiru

## Layout (MVP)

- Left: Connection list (profiles, connection status)
- Center: SQL editor
- Bottom: Result / Error panel

## Interactions (MVP)

- Select a connection profile → connect
- Write SQL → execute
- Results appear as a table in the bottom panel
- Errors appear in the bottom panel with details

## Shortcuts

- Cmd/Ctrl + Enter: execute query
- Cmd/Ctrl + W: close tab (when tabs exist)

## UX rules

- Show a running indicator during connect/execute
- Disable execute while a query is running
- Always show feedback (success row count or error message)

# Requirements — DbMiru

## Target users

- Developers and data engineers who run SQL frequently
- Users who want a lightweight alternative to heavy admin tools

## Core use cases

1. Save and manage database connection profiles
2. Connect to a database
3. Write and execute SQL
4. View results clearly
5. Understand errors quickly

## User stories (MVP)

- As a user, I can create, edit, and delete connection profiles.
- As a user, I can connect to a database and see connection status.
- As a user, I can execute SQL and view results in a table.
- As a user, I can see a clear error summary and details when a query fails.

## Non-functional requirements

- The UI must remain responsive during DB operations.
- Errors must never crash the application.
- Credentials must not be logged or stored in plaintext.
- Reasonable defaults: limit displayed rows for large queries.

## Out of scope (early)

- Full admin features (roles, backups, monitoring)
- Visual query builder / ER diagrams
- Cloud sync

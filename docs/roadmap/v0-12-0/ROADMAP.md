# v0.12.0 — Data Operations and Cloud IAM

> **Theme**: Bulk data in, full data out, and passwordless cloud authentication.
> **Prerequisite**: v0.11.0

---

## Goal

Complete wellfeather's data-lifecycle story: import data from CSV/JSON files into existing
tables, dump full schemas to SQL files, and connect to AWS/GCP/Azure-hosted databases
without managing passwords manually.

---

## Exit Criteria

- [ ] CSV and JSON files can be imported into an existing table via a guided dialog
- [ ] Import column-mapping UI works; header detection is automatic
- [ ] Import supports skip-errors mode (INSERT OR IGNORE) and full-rollback mode, selectable per import
- [ ] Progress bar displayed during large imports; cancellable
- [ ] Full schema dump (DDL, data, or both) to `.sql` file via File → Dump Database
- [ ] `pg_dump` / `mysqldump` PATH auto-detected; manual override settable in preferences
- [ ] SQLite dump uses app-internal implementation (no external tool required)
- [ ] AWS RDS IAM token obtained via `aws-sdk-rust` credential chain; auto-renewed before expiry
- [ ] GCP Cloud SQL accessed via `cloud-sql-proxy` subprocess; proxy killed on disconnect
- [ ] Azure SQL accessed via `az account get-access-token` subprocess; helpful error if not logged in

---

## Key Risks

- **pg_dump/mysqldump on Windows** — tools may not be installed or on PATH; clear error messaging with install guidance is required
- **Cloud IAM token renewal** — tokens expire (AWS 15 min, Azure ~60 min); renewal must be transparent and must not interrupt in-flight queries
- **GCP cloud-sql-proxy process management** — proxy must be reliably killed on disconnect and on unexpected app exit

---

## Task List

See `docs/roadmap/tasks/v0-12-0.md` for details.

| Task ID | Title |
|---------|-------|
| T121 | CSV/JSON import: column-mapping dialog, skip/rollback mode, progress bar |
| T122 | Full schema dump: pg_dump / mysqldump subprocess + SQLite app-internal |
| T123 | Cloud IAM: AWS RDS token, GCP cloud-sql-proxy, Azure az CLI |

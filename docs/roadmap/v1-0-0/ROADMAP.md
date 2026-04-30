# v1.0.0 — Cloud Data Warehouses and AI SQL Generation

> **Theme**: Connect to cloud-scale analytics databases and generate SQL with natural language.
> **Prerequisite**: v0.12.0

---

## Goal

Add BigQuery, Snowflake, and Amazon Redshift drivers so wellfeather covers the full
spectrum of modern databases. Add an AI-assisted SQL generation side panel to lower the
barrier for complex queries and help new users onboard faster.

---

## Exit Criteria

- [ ] BigQuery connection via Service Account JSON; project/dataset/table browsing works
- [ ] BigQuery query results page correctly using `pageToken`; dry-run byte estimate shown optionally
- [ ] Snowflake connection via account/warehouse/database/schema parameters
- [ ] Redshift connection via PostgreSQL-compatible driver with Redshift catalog support
- [ ] AI side panel opens and closes via toolbar toggle; open/closed state persists across restarts
- [ ] Natural language prompt → SQL inserted into the active editor tab (streaming display)
- [ ] Full schema context (all MetadataCache tables/columns) sent automatically; truncated with user notice if too large
- [ ] API key stored in `config.toml` under `[ai]` with AES-256-GCM encryption
- [ ] Panel shows clear guidance when API key is not configured

---

## Key Risks

- **BigQuery REST client** — `gcp-bigquery-client` crate maturity; evaluate and fall back to direct reqwest-based REST implementation if needed
- **Snowflake Rust connector** — no official crate; JWT-based auth via REST API is feasible but requires custom implementation; budget extra time
- **AI token context window** — large schemas (>200 tables) may exceed token limits; implement top-N table truncation with clear notification
- **Streaming in Slint UI** — token-by-token preview updates must go through `slint::invoke_from_event_loop`; avoid holding locks across await points

---

## Performance Targets

| Metric | Target |
|--------|--------|
| BigQuery query round-trip (first page) | < 5 seconds |
| AI SQL generation (first token visible) | < 3 seconds |

---

## Task List

See `docs/roadmap/tasks/v1-0-0.md` for details.

| Task ID | Title |
|---------|-------|
| T200 | BigQuery driver: SA JSON auth, project/dataset browsing, pagination, dry-run |
| T201 | Snowflake driver: JWT/password auth, account params, schema browsing |
| T202 | Redshift driver: PG-compatible with Redshift catalog queries |
| T203 | AI SQL side panel: prompt UI, streaming preview, schema context, API key storage |

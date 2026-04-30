# v0.9.0 — Connection Security Phase 1 (SSH Tunnel, SSL/TLS, Connection Groups)

> **Theme**: Reach databases that are unreachable from the public internet.
> **Prerequisite**: v0.8.0

---

## Goal

Enable secure connections to private-subnet databases via SSH tunneling, add encrypted
transport via SSL/TLS certificates, and let users organize growing connection lists with
folder groups.

---

## Exit Criteria

- [ ] SSH Tunnel tab in connection dialog: both password and private-key auth work
- [ ] Host key fingerprint shown on first connect; saved to `known_hosts`; mismatch blocks
- [ ] SSL/TLS tab: CA cert, client cert, client key file pickers; sslmode dropdown for PG; TLS mode for MySQL
- [ ] Cert files are copied to `{config_dir}/certs/{conn_id}/` on save
- [ ] Connection groups (folders) can be created, renamed, and collapsed; state persists
- [ ] Connections can be moved into groups; color inherits from the group

---

## Key Risks

- **`ssh2` crate on Windows** — OpenSSL linkage may require the `vendored` feature; test CI early
- **SSH tunnel lifecycle** — the tunnel must be torn down on disconnect even if the app panics or is force-quit
- **SSL/TLS + sqlx** — sqlx's `rustls` feature must be enabled; PEM cert loading tested on all three DB drivers

---

## Task List

See `docs/roadmap/tasks/v0-9-0.md` for details.

| Task ID | Title |
|---------|-------|
| T091 | SSH tunnel: connection dialog tab, ssh2 port-forward, known_hosts verification |
| T092 | SSL/TLS: cert file picker, sslmode/TLS mode dropdown, cert copy to config dir |
| T093 | Connection groups: folder create/rename/collapse, color inheritance |

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rust_i18n::t;
use slint::{ComponentHandle, Model as _};
use tokio::sync::{mpsc, oneshot};
use wf_config::{crypto, models::DbTypeName};
use wf_db::models::{DbConnection, DbType, SshAuth, SshTunnelConfig, SslConfig, SslMode};

use crate::app::{
    command::{Command, ConfigUpdate},
    session::config_to_db_conn,
};
use crate::state::SharedState;

use super::{
    SidebarUiState, build_sidebar_tree, config_connections_to_entries, send_cmd, tabs_state,
    with_sidebar, with_sidebar_mut, with_ui,
};

pub(super) fn db_type_label_config(dt: &DbTypeName) -> &'static str {
    match dt {
        DbTypeName::PostgreSQL => "PostgreSQL",
        DbTypeName::MySQL => "MySQL",
        DbTypeName::SQLite => "SQLite",
    }
}

/// Returns a localized error string when the form state is invalid, or `None` when valid.
///
/// Currently catches: connection string mode + SSH host both set (remote_host would be empty).
fn validate_form(ui: &crate::UiState) -> Option<String> {
    let is_conn_string = ui.get_form_tab_index() == 0;
    let ssh_host_set = !ui.get_form_ssh_host().is_empty();
    if is_conn_string && ssh_host_set {
        return Some(t!("error.ssh_conn_string_unsupported").to_string());
    }
    None
}

/// Build a `DbConnection` from the current values in the connection form global,
/// and return the plaintext password separately (for immediate use in the connection URL).
///
/// The plaintext password is also AES-256-GCM encrypted with `enc_key` and stored in
/// `DbConnection.password_encrypted` so the session manager can persist it and
/// `main.rs` can decrypt it on the next startup for auto-reconnect.
fn build_conn_from_form(
    ui: &crate::UiState,
    enc_key: &[u8; 32],
) -> (DbConnection, Option<zeroize::Zeroizing<String>>) {
    let db_type = match ui.get_form_db_type() {
        0 => DbType::PostgreSQL,
        1 => DbType::MySQL,
        _ => DbType::SQLite,
    };

    let is_conn_string = ui.get_form_tab_index() == 0;
    let opt = |s: slint::SharedString| {
        let s = s.to_string();
        if s.is_empty() { None } else { Some(s) }
    };

    let password = if is_conn_string {
        None
    } else {
        opt(ui.get_form_password()).map(zeroize::Zeroizing::new)
    };

    // Encrypt the plaintext password for safe storage in config.toml.
    // Connection-string mode embeds the password in the URL, so no separate encryption needed.
    let password_encrypted = password.as_ref().map(|pw| crypto::encrypt(pw, enc_key));

    // Preserve the existing id when editing so save_connection upserts correctly.
    let edit_id = ui.get_form_edit_id().to_string();
    let id = if edit_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        edit_id
    };

    // SSH is active when SSH Host is filled in; `opt` returns None for empty strings.
    let ssh = opt(ui.get_form_ssh_host()).map(|host| {
        let auth = if ui.get_form_ssh_auth_method() == 1 {
            SshAuth::PrivateKey {
                key_path: ui.get_form_ssh_key_path().to_string(),
            }
        } else {
            SshAuth::Password
        };

        let ssh_password_raw = opt(ui.get_form_ssh_password()).map(zeroize::Zeroizing::new);
        let ssh_password_encrypted = ssh_password_raw
            .as_deref()
            .map(|p| crypto::encrypt(p, enc_key));

        let ssh_passphrase_raw = opt(ui.get_form_ssh_passphrase()).map(zeroize::Zeroizing::new);
        let ssh_passphrase_encrypted = ssh_passphrase_raw
            .as_deref()
            .map(|p| crypto::encrypt(p, enc_key));

        SshTunnelConfig {
            host,
            port: ui
                .get_form_ssh_port()
                .to_string()
                .parse::<u16>()
                .unwrap_or(22),
            user: ui.get_form_ssh_user().to_string(),
            auth,
            remote_host: if is_conn_string {
                String::new()
            } else {
                opt(ui.get_form_host()).unwrap_or_default()
            },
            remote_port: if is_conn_string {
                5432
            } else {
                ui.get_form_port()
                    .to_string()
                    .parse::<u16>()
                    .unwrap_or(5432)
            },
            ssh_password_encrypted,
            ssh_passphrase_encrypted,
        }
    });

    let conn = DbConnection {
        id,
        name: ui.get_form_name().to_string(),
        db_type,
        connection_string: if is_conn_string {
            opt(ui.get_form_conn_string())
        } else {
            None
        },
        host: if is_conn_string {
            None
        } else {
            opt(ui.get_form_host())
        },
        port: if is_conn_string {
            None
        } else {
            ui.get_form_port().to_string().parse::<u16>().ok()
        },
        user: if is_conn_string {
            None
        } else {
            opt(ui.get_form_user())
        },
        password_encrypted,
        database: if is_conn_string {
            None
        } else {
            opt(ui.get_form_database())
        },
        ssh,
        ssl: if ui.get_form_ssl_mode() != 0 && ui.get_form_db_type() != 2 {
            Some(SslConfig {
                mode: match ui.get_form_ssl_mode() {
                    2 => SslMode::VerifyCa,
                    3 => SslMode::VerifyFull,
                    _ => SslMode::Require,
                },
                ca_cert: opt_path(ui.get_form_ssl_ca_cert().as_str()),
                client_cert: opt_path(ui.get_form_ssl_client_cert().as_str()),
                client_key: opt_path(ui.get_form_ssl_client_key().as_str()),
            })
        } else {
            None
        },
    };

    (conn, password)
}

// ── Misc helpers ─────────────────────────────────────────────────────────────

/// Convert a non-empty string to a `PathBuf`, or `None` if the string is empty.
fn opt_path(s: &str) -> Option<std::path::PathBuf> {
    if s.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(s))
    }
}

// ── Connection string helpers ─────────────────────────────────────────────────

/// Percent-encode a string for use in a URL userinfo or path component.
/// Only unreserved characters (RFC 3986) are left as-is.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

/// Percent-decode a URL component back to a plain string.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Derive a connection URL from individual fields.
///
/// SQLite: returns the database path as-is.
/// PostgreSQL/MySQL: builds `scheme://[user[:pass]@][host[:port]][/database]`.
fn derive_conn_string(
    db_type: &DbType,
    host: &str,
    port: Option<u16>,
    user: &str,
    password: &str,
    database: &str,
) -> String {
    if matches!(db_type, DbType::SQLite) {
        return database.to_string();
    }
    let (scheme, default_port) = match db_type {
        DbType::PostgreSQL => ("postgres", 5432u16),
        DbType::MySQL => ("mysql", 3306u16),
        DbType::SQLite => unreachable!(),
    };
    let userinfo = match (user.is_empty(), password.is_empty()) {
        (true, _) => String::new(),
        (false, true) => format!("{}@", percent_encode(user)),
        (false, false) => format!("{}:{}@", percent_encode(user), percent_encode(password)),
    };
    let hostport = if host.is_empty() {
        String::new()
    } else {
        format!("{}:{}", host, port.unwrap_or(default_port))
    };
    let db_part = if database.is_empty() {
        String::new()
    } else {
        format!("/{}", database)
    };
    format!("{}://{}{}{}", scheme, userinfo, hostport, db_part)
}

/// Parse a URL-format connection string into individual fields.
///
/// Handles `postgres://`, `postgresql://`, and `mysql://` schemes.
/// SQLite: the entire string is treated as the database path.
///
/// Returns `(host, port, user, password, database)` or `None` if the string
/// cannot be recognised as a URL for the given db type.
fn parse_conn_string(
    s: &str,
    db_type: &DbType,
) -> Option<(String, Option<u16>, String, String, String)> {
    let s = s.trim();
    if matches!(db_type, DbType::SQLite) {
        return Some((
            String::new(),
            None,
            String::new(),
            String::new(),
            s.to_string(),
        ));
    }
    let rest = s
        .strip_prefix("postgres://")
        .or_else(|| s.strip_prefix("postgresql://"))
        .or_else(|| s.strip_prefix("mysql://"))?;
    // Strip query string / fragment
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    // Split userinfo from host
    let (userinfo, hostdb) = match rest.rfind('@') {
        Some(pos) => (&rest[..pos], &rest[pos + 1..]),
        None => ("", rest),
    };
    let (user, password) = match userinfo.find(':') {
        Some(pos) => (&userinfo[..pos], &userinfo[pos + 1..]),
        None => (userinfo, ""),
    };
    let (hostport, database) = match hostdb.find('/') {
        Some(pos) => (&hostdb[..pos], &hostdb[pos + 1..]),
        None => (hostdb, ""),
    };
    // Handle IPv6 brackets: [::1]:5432
    let (host, port) = if hostport.starts_with('[') {
        let bracket_end = hostport.find(']').unwrap_or(hostport.len());
        let host_raw = &hostport[1..bracket_end];
        let port = hostport[bracket_end + 1..]
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok());
        (host_raw, port)
    } else {
        match hostport.rfind(':') {
            Some(pos) => {
                let port = hostport[pos + 1..].parse::<u16>().ok();
                (&hostport[..pos], port)
            }
            None => (hostport, None),
        }
    };
    Some((
        percent_decode(host),
        port,
        percent_decode(user),
        percent_decode(password),
        percent_decode(database),
    ))
}

pub(super) fn register_sidebar_callbacks(
    window: &crate::AppWindow,
    state: SharedState,
    tx_cmd: mpsc::Sender<Command>,
    sidebar_state: Arc<Mutex<SidebarUiState>>,
    enc_key: [u8; 32],
    tabs_state: Rc<RefCell<tabs_state::TabsState>>,
) {
    let ui_state = window.global::<crate::UiState>();

    // open-connection-form: reset form fields then show the overlay
    {
        let window_weak = window.as_weak();
        ui_state.on_open_connection_form(move || {
            with_ui(&window_weak, |ui| {
                ui.set_form_name("".into());
                ui.set_form_conn_string("".into());
                ui.set_form_host("".into());
                ui.set_form_port("".into());
                ui.set_form_user("".into());
                ui.set_form_password("".into());
                ui.set_form_database("".into());
                ui.set_form_status("".into());
                ui.set_form_testing(false);
                ui.set_form_tab_index(0);
                ui.set_form_db_type(0);
                ui.set_form_test_ok(false);
                ui.set_show_test_result_popup(false);
                ui.set_show_add_confirm_popup(false);
                ui.set_form_edit_id("".into());
                ui.set_form_safe_dml(true);
                ui.set_form_read_only(false);
                // Reset SSH tunnel fields
                ui.set_form_section(0);
                ui.set_form_ssh_host("".into());
                ui.set_form_ssh_port("22".into());
                ui.set_form_ssh_user("".into());
                ui.set_form_ssh_auth_method(0);
                ui.set_form_ssh_password("".into());
                ui.set_form_ssh_key_path("".into());
                ui.set_form_ssh_passphrase("".into());
                // Reset SSL/TLS fields
                ui.set_form_ssl_mode(0);
                ui.set_form_ssl_ca_cert("".into());
                ui.set_form_ssl_client_cert("".into());
                ui.set_form_ssl_client_key("".into());
                ui.set_show_connection_form(true);
            });
        });
    }

    // edit-connection: open the connection form pre-filled for an existing connection.
    // Both tabs are filled: whichever tab was NOT used to save the connection is
    // derived via derive_conn_string / parse_conn_string.
    {
        let window_weak = window.as_weak();
        let sidebar_state = Arc::clone(&sidebar_state);
        ui_state.on_edit_connection(move |id| {
            let id = id.to_string();
            // Look up from config_connections — works even when the DB is not running.
            let conn_cfg = with_sidebar(&sidebar_state, |sb| {
                sb.config_connections.iter().find(|c| c.id == id).cloned()
            });
            let Some(conn_cfg) = conn_cfg else {
                return;
            };
            let conn = config_to_db_conn(&conn_cfg);
            let safe_dml = conn_cfg.safe_dml;
            let read_only = conn_cfg.read_only;

            // Decrypt stored password for pre-filling the UI form. Convert to plain
            // String at this point since it flows directly into SharedString UI fields.
            let stored_password: String = conn
                .password_encrypted
                .as_ref()
                .and_then(|enc| crypto::decrypt(enc, &enc_key).ok())
                .map(|z| z.as_str().to_owned())
                .unwrap_or_default();

            let is_conn_string = conn.connection_string.is_some();
            let db_type_idx: i32 = match conn.db_type {
                DbType::PostgreSQL => 0,
                DbType::MySQL => 1,
                DbType::SQLite => 2,
            };

            // Derive the connection string from individual fields (or use the stored one).
            let conn_string = if is_conn_string {
                conn.connection_string.clone().unwrap_or_default()
            } else {
                derive_conn_string(
                    &conn.db_type,
                    conn.host.as_deref().unwrap_or(""),
                    conn.port,
                    conn.user.as_deref().unwrap_or(""),
                    &stored_password,
                    conn.database.as_deref().unwrap_or(""),
                )
            };

            // Parse individual fields from the connection string (or use the stored ones).
            let (host, port, user, field_password, database) = if is_conn_string {
                parse_conn_string(&conn_string, &conn.db_type).unwrap_or_default()
            } else {
                (
                    conn.host.clone().unwrap_or_default(),
                    conn.port,
                    conn.user.clone().unwrap_or_default(),
                    stored_password,
                    conn.database.clone().unwrap_or_default(),
                )
            };

            // Decrypt SSH credentials for pre-filling the form.
            let (
                ssh_host,
                ssh_port,
                ssh_user,
                ssh_auth_method,
                ssh_password,
                ssh_key_path,
                ssh_passphrase,
            ) = match &conn.ssh {
                Some(ssh_cfg) => {
                    let pw = ssh_cfg
                        .ssh_password_encrypted
                        .as_ref()
                        .and_then(|enc| crypto::decrypt(enc, &enc_key).ok())
                        .map(|z| z.as_str().to_owned())
                        .unwrap_or_default();
                    let pp = ssh_cfg
                        .ssh_passphrase_encrypted
                        .as_ref()
                        .and_then(|enc| crypto::decrypt(enc, &enc_key).ok())
                        .map(|z| z.as_str().to_owned())
                        .unwrap_or_default();
                    let auth_idx = match &ssh_cfg.auth {
                        SshAuth::PrivateKey { .. } => 1,
                        SshAuth::Password => 0,
                    };
                    let key_path = match &ssh_cfg.auth {
                        SshAuth::PrivateKey { key_path } => key_path.clone(),
                        SshAuth::Password => String::new(),
                    };
                    (
                        ssh_cfg.host.clone(),
                        ssh_cfg.port,
                        ssh_cfg.user.clone(),
                        auth_idx,
                        pw,
                        key_path,
                        pp,
                    )
                }
                None => (
                    String::new(),
                    22u16,
                    String::new(),
                    0i32,
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            };

            with_ui(&window_weak, move |ui| {
                ui.set_form_edit_id(conn.id.clone().into());
                ui.set_form_name(conn.name.clone().into());
                ui.set_form_db_type(db_type_idx);
                ui.set_form_tab_index(if is_conn_string { 0 } else { 1 });
                ui.set_form_conn_string(conn_string.into());
                ui.set_form_host(host.into());
                ui.set_form_port(port.map(|p| p.to_string()).unwrap_or_default().into());
                ui.set_form_user(user.into());
                ui.set_form_password(field_password.into());
                ui.set_form_database(database.into());
                ui.set_form_status("".into());
                ui.set_form_testing(false);
                ui.set_form_test_ok(false);
                ui.set_form_safe_dml(safe_dml);
                ui.set_form_read_only(read_only);
                // SSH tunnel fields
                ui.set_form_section(0);
                ui.set_form_ssh_host(ssh_host.into());
                ui.set_form_ssh_port(ssh_port.to_string().into());
                ui.set_form_ssh_user(ssh_user.into());
                ui.set_form_ssh_auth_method(ssh_auth_method);
                ui.set_form_ssh_password(ssh_password.into());
                ui.set_form_ssh_key_path(ssh_key_path.into());
                ui.set_form_ssh_passphrase(ssh_passphrase.into());
                // SSL/TLS fields (0=Disabled, 1=Require, 2=VerifyCa, 3=VerifyFull)
                if let Some(ref ssl) = conn.ssl {
                    ui.set_form_ssl_mode(match ssl.mode {
                        SslMode::Require => 1,
                        SslMode::VerifyCa => 2,
                        SslMode::VerifyFull => 3,
                    });
                    ui.set_form_ssl_ca_cert(
                        ssl.ca_cert
                            .as_ref()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default()
                            .into(),
                    );
                    ui.set_form_ssl_client_cert(
                        ssl.client_cert
                            .as_ref()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default()
                            .into(),
                    );
                    ui.set_form_ssl_client_key(
                        ssl.client_key
                            .as_ref()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default()
                            .into(),
                    );
                } else {
                    ui.set_form_ssl_mode(0);
                    ui.set_form_ssl_ca_cert("".into());
                    ui.set_form_ssl_client_cert("".into());
                    ui.set_form_ssl_client_key("".into());
                }
                ui.set_show_test_result_popup(false);
                ui.set_show_add_confirm_popup(false);
                ui.set_show_connection_form(true);
            });
        });
    }

    // toggle-sidebar-node: expand/collapse a tree node; also switches active
    // connection when an inactive level-0 (connection) node is clicked.
    {
        // clone required: callback closure needs owned captures
        let tx_cmd = tx_cmd.clone();
        let state = state.clone();
        let sidebar_state = Arc::clone(&sidebar_state);
        let window_weak = window.as_weak();
        ui_state.on_toggle_sidebar_node(move |id| {
            let id = id.to_string();
            // For connection nodes, switch only when not already active.
            if let Some(conn_id) = id.strip_prefix("conn:") {
                let active_id = state
                    .conn
                    .active()
                    .map(|c| c.id.clone())
                    .unwrap_or_default();
                if conn_id != active_id {
                    let conn_cfg = with_sidebar(&sidebar_state, |sb| {
                        sb.config_connections
                            .iter()
                            .find(|c| c.id == conn_id)
                            .cloned()
                    });
                    if let Some(cc) = conn_cfg {
                        let conn = config_to_db_conn(&cc);
                        let password = conn
                            .password_encrypted
                            .as_ref()
                            .and_then(|enc| crypto::decrypt(enc, &enc_key).ok());
                        send_cmd(&tx_cmd, Command::Connect(conn, password));
                    }
                    // Return early — Event::Connected will auto-expand the newly active node.
                    return;
                }
            }
            // Toggle expanded state (active connection and category nodes).
            with_sidebar_mut(&sidebar_state, |sb| {
                if sb.expanded.contains(&id) {
                    sb.expanded.remove(&id);
                } else {
                    sb.expanded.insert(id.clone());
                }
            });
            // Rebuild and push the updated tree (already on UI thread)
            let nodes = with_sidebar(&sidebar_state, |sb| {
                let active_id = state
                    .conn
                    .active()
                    .map(|c| c.id.clone())
                    .unwrap_or_default();
                build_sidebar_tree(
                    &sb.config_connections,
                    &active_id,
                    &sb.metadata,
                    &sb.expanded,
                    &sb.read_only,
                )
            });
            with_ui(&window_weak, |ui| {
                let model = ui.get_sidebar_tree();
                if model.row_count() == nodes.len() {
                    for (i, node) in nodes.into_iter().enumerate() {
                        model.set_row_data(i, node);
                    }
                } else {
                    ui.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
                }
            });
        });
    }

    // connect-db: connect to a saved connection from the DB tab by id.
    // Mirrors the connection-switching logic in toggle-sidebar-node.
    {
        // clone required: callback closure needs owned captures
        let tx_cmd = tx_cmd.clone();
        let state = state.clone();
        let sidebar_state = Arc::clone(&sidebar_state); // clone required: lookup from config_connections
        ui_state.on_connect_db(move |id| {
            let id = id.to_string();
            let active_id = state
                .conn
                .active()
                .map(|c| c.id.clone())
                .unwrap_or_default();
            if id == active_id {
                return;
            }
            let conn_cfg = with_sidebar(&sidebar_state, |sb| {
                sb.config_connections.iter().find(|c| c.id == id).cloned()
            });
            if let Some(cc) = conn_cfg {
                let conn = config_to_db_conn(&cc);
                let password = conn
                    .password_encrypted
                    .as_ref()
                    .and_then(|enc| crypto::decrypt(enc, &enc_key).ok());
                send_cmd(&tx_cmd, Command::Connect(conn, password));
            }
        });
    }

    // open-db-manager: show the DB manager dialog.
    {
        let window_weak = window.as_weak();
        ui_state.on_open_db_manager(move || {
            with_ui(&window_weak, |ui| ui.set_show_db_manager(true));
        });
    }

    // disconnect: disconnect the active connection by id.
    // Clears AppState active_id, collapses the sidebar node, removes cached
    // metadata for that connection, and rebuilds the sidebar tree immediately.
    {
        let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
        let state = state.clone();
        let sidebar_state = Arc::clone(&sidebar_state);
        let window_weak = window.as_weak();
        ui_state.on_disconnect(move |id| {
            let id = id.to_string();
            if id.is_empty() {
                return;
            }
            // Clear active connection in AppState so toggle-sidebar-node can
            // reconnect when the collapsed node is clicked again.
            state.conn.clear_active();

            // Collapse the node and drop metadata for the disconnected connection.
            with_sidebar_mut(&sidebar_state, |sb| {
                sb.expanded.remove(&format!("conn:{}", id));
                sb.metadata.remove(&id);
            });

            // Rebuild tree (no active connection, no expanded node for id).
            let (nodes, entries) = with_sidebar(&sidebar_state, |sb| {
                let nodes = build_sidebar_tree(
                    &sb.config_connections,
                    "",
                    &sb.metadata,
                    &sb.expanded,
                    &sb.read_only,
                );
                let entries = config_connections_to_entries(&sb.config_connections, "");
                (nodes, entries)
            });

            // Already on the UI thread — update directly.
            with_ui(&window_weak, move |ui| {
                ui.set_active_connection_id("".into());
                ui.set_connection_list(Rc::new(slint::VecModel::from(entries)).into());
                ui.set_sidebar_tree(Rc::new(slint::VecModel::from(nodes)).into());
                ui.set_sidebar_loading(false);
            });

            send_cmd(&tx_cmd, Command::Disconnect(id));
        });
    }

    // table-double-clicked: open a Table View tab for the clicked table/view.
    // Saves the active editor text, opens or focuses the TV tab, then triggers
    // a FetchTableData command if the tab is new.
    {
        let window_weak = window.as_weak();
        let tx_cmd = tx_cmd.clone(); // clone required: callback closure needs owned tx_cmd
        let tabs_state = Rc::clone(&tabs_state); // clone required: callback closure needs owned tabs_state
        let sidebar_state = Arc::clone(&sidebar_state); // clone required: callback closure needs owned sidebar_state
        ui_state.on_table_double_clicked(move |name| {
            let name = name.to_string();
            let (conn_id, current_text, current_sub_tab) = {
                let Some(w) = window_weak.upgrade() else {
                    return;
                };
                let ui = w.global::<crate::UiState>();
                (
                    ui.get_active_connection_id().to_string(),
                    ui.get_editor_text().to_string(),
                    ui.get_tv_sub_tab() as usize,
                )
            };
            if conn_id.is_empty() {
                return;
            }
            let (tab_id, is_new, slint_tabs, active_idx, tv_sub_tab) = {
                let mut ts = tabs_state.borrow_mut();
                ts.save_current_text(&current_text);
                ts.save_tv_sub_tab(current_sub_tab);
                let (tab_id, _idx, is_new) = ts.open_table_view(&conn_id, &name);
                let tv_sub_tab = ts.active_tv_sub_tab();
                let slint_tabs = super::tabs::tabs_to_slint(&ts.tabs);
                let active_idx = ts.active_index as i32;
                (tab_id, is_new, slint_tabs, active_idx, tv_sub_tab)
            };
            let tv_cols = with_sidebar(&sidebar_state, |sb| {
                sb.metadata
                    .get(&conn_id)
                    .and_then(|meta| {
                        meta.tables
                            .iter()
                            .chain(meta.views.iter())
                            .find(|t| t.name == name)
                            .map(|ti| super::tabs::columns_to_slint(&ti.columns))
                    })
                    .unwrap_or_default()
            });
            with_ui(&window_weak, |ui| {
                ui.set_tabs(Rc::new(slint::VecModel::from(slint_tabs)).into());
                ui.set_active_tab_index(active_idx);
                ui.set_active_tab_kind_sql(false);
                ui.set_tv_table_name(name.clone().into());
                ui.set_tv_sub_tab(tv_sub_tab as i32);
                if is_new {
                    ui.set_tv_page_size(1000);
                }
                ui.set_tv_data_loading(is_new);
                ui.set_tv_data_error("".into());
                ui.set_tv_ddl("".into());
                ui.set_tv_ddl_loading(false);
                ui.set_tv_columns(Rc::new(slint::VecModel::from(tv_cols)).into());
            });
            if is_new {
                send_cmd(
                    &tx_cmd,
                    Command::FetchTableData {
                        tab_id,
                        conn_id,
                        table_name: name,
                        page_size: 1000,
                    },
                );
            }
        });
    }
}

// ── Connection form callbacks ─────────────────────────────────────────────────

pub(super) fn register_connection_form_callbacks(
    window: &crate::AppWindow,
    tx_cmd: mpsc::Sender<Command>,
    enc_key: [u8; 32],
    fp_approval_tx: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
) {
    let ui_state = window.global::<crate::UiState>();

    // close-connection-form
    {
        let window_weak = window.as_weak();
        ui_state.on_close_connection_form(move || {
            with_ui(&window_weak, |ui| {
                ui.set_show_connection_form(false);
                if ui.get_reopen_db_manager_on_form_close() {
                    ui.set_reopen_db_manager_on_form_close(false);
                    ui.set_show_db_manager(true);
                }
            });
        });
    }

    // test-connection: probe without saving — sends Command::TestConnection
    {
        let window_weak = window.as_weak();
        // clone required: callback closure needs owned tx_cmd
        let tx_cmd = tx_cmd.clone();
        ui_state.on_test_connection(move || {
            with_ui(&window_weak, |ui| {
                if let Some(err) = validate_form(ui) {
                    ui.set_form_status(err.into());
                    return;
                }
                ui.set_form_testing(true);
                ui.set_form_status("".into());
                ui.set_form_test_ok(false);
                let (conn, password) = build_conn_from_form(ui, &enc_key);
                send_cmd(&tx_cmd, Command::TestConnection(conn, password));
            });
        });
    }

    // add-connection: persist if test passed, else show confirm popup
    {
        let window_weak = window.as_weak();
        // clone required: callback closure needs owned tx_cmd
        let tx_cmd = tx_cmd.clone();
        ui_state.on_add_connection(move || {
            with_ui(&window_weak, |ui| {
                if let Some(err) = validate_form(ui) {
                    ui.set_form_status(err.into());
                    return;
                }
                if ui.get_form_test_ok() {
                    ui.set_form_testing(true);
                    let (conn, password) = build_conn_from_form(ui, &enc_key);
                    let conn_id = conn.id.clone();
                    let safe_dml = ui.get_form_safe_dml();
                    let read_only = ui.get_form_read_only();
                    send_cmd(&tx_cmd, Command::Connect(conn, password));
                    send_cmd(
                        &tx_cmd,
                        Command::UpdateConfig(ConfigUpdate::ConnectionFlags {
                            id: conn_id,
                            safe_dml,
                            read_only,
                        }),
                    );
                } else {
                    ui.set_show_add_confirm_popup(true);
                }
            });
        });
    }

    // confirm-add-connection: user chose "Yes" in confirm popup
    {
        let window_weak = window.as_weak();
        // clone required: callback closure needs owned tx_cmd
        let tx_cmd = tx_cmd.clone();
        ui_state.on_confirm_add_connection(move || {
            with_ui(&window_weak, |ui| {
                if let Some(err) = validate_form(ui) {
                    ui.set_form_status(err.into());
                    return;
                }
                ui.set_show_add_confirm_popup(false);
                ui.set_form_testing(true);
                let (conn, password) = build_conn_from_form(ui, &enc_key);
                let conn_id = conn.id.clone();
                let safe_dml = ui.get_form_safe_dml();
                let read_only = ui.get_form_read_only();
                send_cmd(&tx_cmd, Command::Connect(conn, password));
                send_cmd(
                    &tx_cmd,
                    Command::UpdateConfig(ConfigUpdate::ConnectionFlags {
                        id: conn_id,
                        safe_dml,
                        read_only,
                    }),
                );
            });
        });
    }

    // dismiss-test-popup: close the test-result popup
    {
        let window_weak = window.as_weak();
        ui_state.on_dismiss_test_popup(move || {
            with_ui(&window_weak, |ui| ui.set_show_test_result_popup(false));
        });
    }

    // dismiss-add-confirm: user chose "No" in confirm popup
    {
        let window_weak = window.as_weak();
        ui_state.on_dismiss_add_confirm(move || {
            with_ui(&window_weak, |ui| ui.set_show_add_confirm_popup(false));
        });
    }

    // delete-connection: remove from config, disconnect if active
    {
        let window_weak = window.as_weak();
        // clone required: callback closure needs owned tx_cmd
        let tx_cmd = tx_cmd.clone();
        ui_state.on_delete_connection(move || {
            with_ui(&window_weak, |ui| {
                let id = ui.get_form_edit_id().to_string();
                if !id.is_empty() {
                    send_cmd(&tx_cmd, Command::RemoveConnection(id));
                    ui.set_show_connection_form(false);
                }
            });
        });
    }

    // browse-ssh-key-file: open a file picker and set form-ssh-key-path.
    {
        let window_weak = window.as_weak();
        ui_state.on_browse_ssh_key_file(move || {
            let window_weak = window_weak.clone(); // clone required: async move
            tokio::spawn(async move {
                if let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter("Private key", &["pem", "key", "ppk", ""])
                    .pick_file()
                    .await
                {
                    let path = file.path().to_string_lossy().into_owned();
                    slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak.upgrade() {
                            w.global::<crate::UiState>()
                                .set_form_ssh_key_path(path.into());
                        }
                    })
                    .ok();
                }
            });
        });
    }

    // browse-ssl-ca-cert: open a file picker and set form-ssl-ca-cert.
    {
        let window_weak = window.as_weak();
        ui_state.on_browse_ssl_ca_cert(move || {
            let window_weak = window_weak.clone(); // clone required: async move
            tokio::spawn(async move {
                if let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter("PEM certificate", &["pem", "crt", "cer", ""])
                    .pick_file()
                    .await
                {
                    let path = file.path().to_string_lossy().into_owned();
                    slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak.upgrade() {
                            w.global::<crate::UiState>()
                                .set_form_ssl_ca_cert(path.into());
                        }
                    })
                    .ok();
                }
            });
        });
    }

    // browse-ssl-client-cert: open a file picker and set form-ssl-client-cert.
    {
        let window_weak = window.as_weak();
        ui_state.on_browse_ssl_client_cert(move || {
            let window_weak = window_weak.clone(); // clone required: async move
            tokio::spawn(async move {
                if let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter("PEM certificate", &["pem", "crt", "cer", ""])
                    .pick_file()
                    .await
                {
                    let path = file.path().to_string_lossy().into_owned();
                    slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak.upgrade() {
                            w.global::<crate::UiState>()
                                .set_form_ssl_client_cert(path.into());
                        }
                    })
                    .ok();
                }
            });
        });
    }

    // browse-ssl-client-key: open a file picker and set form-ssl-client-key.
    {
        let window_weak = window.as_weak();
        ui_state.on_browse_ssl_client_key(move || {
            let window_weak = window_weak.clone(); // clone required: async move
            tokio::spawn(async move {
                if let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter("PEM private key", &["pem", "key", ""])
                    .pick_file()
                    .await
                {
                    let path = file.path().to_string_lossy().into_owned();
                    slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak.upgrade() {
                            w.global::<crate::UiState>()
                                .set_form_ssl_client_key(path.into());
                        }
                    })
                    .ok();
                }
            });
        });
    }

    // approve-ssh-fingerprint: user approved the unknown host key.
    {
        let fp_approval_tx = Arc::clone(&fp_approval_tx); // clone required: callback closure
        let window_weak = window.as_weak();
        ui_state.on_approve_ssh_fingerprint(move || {
            if let Some(tx) = fp_approval_tx
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                let _ = tx.send(true);
            }
            with_ui(&window_weak, |ui| ui.set_show_ssh_fingerprint_dialog(false));
        });
    }

    // reject-ssh-fingerprint: user rejected the unknown host key.
    {
        let fp_approval_tx = Arc::clone(&fp_approval_tx); // clone required: callback closure
        let window_weak = window.as_weak();
        ui_state.on_reject_ssh_fingerprint(move || {
            if let Some(tx) = fp_approval_tx
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                let _ = tx.send(false);
            }
            with_ui(&window_weak, |ui| ui.set_show_ssh_fingerprint_dialog(false));
        });
    }
}

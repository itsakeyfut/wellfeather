//! Session persistence helpers.
//!
//! [`SessionManager`] wraps [`ConfigManager`] and persists page size, language,
//! theme, and reduce-motion settings to `config.toml`.
//!
//! Tab and last-query persistence has moved to [`wf_history::session::SessionService`]
//! (SQLite-backed via the shared `wellfeather.db` pool).
//!
//! Connection persistence (CRUD + last-used tracking) lives in
//! [`wf_config::ConnectionRepository`] (SQLite-backed).
//!
//! Conversion between `wf_db::models::DbConnection` and `wf_config::models::ConnectionConfig`
//! lives here because `app/` is the only crate that depends on both.

use anyhow::Context as _;
use tracing::info;
use wf_config::{
    manager::ConfigManager,
    models::{ConnectionConfig, DbTypeName, PageSize, SshAuthMethod, Theme},
};
use wf_db::models::{DbConnection, DbType, SshAuth, SshTunnelConfig, SslConfig, SslMode};

pub use wf_history::session::TabSessionEntry;

/// Persists and restores the last active database connection across app restarts.
///
/// Internally delegates all I/O to a [`ConfigManager`], which performs atomic
/// TOML file writes. `save_connection` is synchronous (blocking file I/O); this
/// is acceptable for a small desktop config file and avoids `spawn_blocking` noise.
pub struct SessionManager {
    config_manager: ConfigManager,
}

impl SessionManager {
    /// Create a `SessionManager` that reads from the default config path
    /// (`~/.config/wellfeather/config.toml` on Linux/macOS,
    /// `%APPDATA%\wellfeather\config.toml` on Windows).
    pub fn new() -> Self {
        Self {
            config_manager: ConfigManager::new(),
        }
    }

    /// Create a `SessionManager` backed by an arbitrary [`ConfigManager`].
    ///
    /// Used in tests to point at a temporary directory.
    #[cfg(test)]
    pub fn with_config_manager(cm: ConfigManager) -> Self {
        Self { config_manager: cm }
    }

    /// Persist `size` (100 / 500 / 1000) as `[editor].page_size` in `config.toml`.
    ///
    /// Silently ignores unknown values (not in the `PageSize` enum); they are
    /// replaced with the default (500).
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be loaded or written.
    pub fn save_page_size(&self, size: usize) -> anyhow::Result<()> {
        let mut config = self
            .config_manager
            .load()
            .context("failed to load config for page_size save")?;

        config.editor.page_size = PageSize::try_from(size as u32).unwrap_or_default();

        self.config_manager
            .save(&config)
            .context("failed to save page_size")?;
        info!(page_size = size, "page_size saved");
        Ok(())
    }

    /// Persist `lang` as `[ui].language` in `config.toml`.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be loaded or written.
    pub fn save_language(&self, lang: &str) -> anyhow::Result<()> {
        let mut config = self
            .config_manager
            .load()
            .context("failed to load config for language save")?;

        config.ui.language = lang.to_string();

        self.config_manager
            .save(&config)
            .context("failed to save language")?;
        info!(%lang, "language saved");
        Ok(())
    }

    /// Persist `theme` as `[appearance].theme` in `config.toml`.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be loaded or written.
    pub fn save_theme(&self, theme: &Theme) -> anyhow::Result<()> {
        let mut config = self
            .config_manager
            .load()
            .context("failed to load config for theme save")?;

        config.appearance.theme = theme.clone();

        self.config_manager
            .save(&config)
            .context("failed to save theme")?;
        info!(?theme, "theme saved");
        Ok(())
    }

    /// Persist `width` as `[editor].tab_width` in `config.toml`.
    pub fn save_tab_width(&self, width: u32) -> anyhow::Result<()> {
        let mut config = self
            .config_manager
            .load()
            .context("failed to load config for tab_width save")?;
        config.editor.tab_width = width;
        self.config_manager
            .save(&config)
            .context("failed to save tab_width")?;
        info!(tab_width = width, "tab_width saved");
        Ok(())
    }

    /// Persist `reduce_motion` as `[appearance].reduce_motion` in `config.toml`.
    pub fn save_reduce_motion(&self, value: bool) -> anyhow::Result<()> {
        let mut config = self
            .config_manager
            .load()
            .context("failed to load config for reduce_motion save")?;
        config.appearance.reduce_motion = value;
        self.config_manager
            .save(&config)
            .context("failed to save reduce_motion")?;
        Ok(())
    }

    /// Persist `secs` as `[editor].query_timeout_secs` in `config.toml`.
    pub fn save_query_timeout(&self, secs: u64) -> anyhow::Result<()> {
        let mut config = self
            .config_manager
            .load()
            .context("failed to load config for query_timeout_secs save")?;
        config.editor.query_timeout_secs = secs;
        self.config_manager
            .save(&config)
            .context("failed to save query_timeout_secs")?;
        info!(query_timeout_secs = secs, "query_timeout_secs saved");
        Ok(())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Conversion helpers (only in app/ — sees both wf-config and wf-db) ────────

/// Convert a stored [`ConnectionConfig`] to the runtime [`DbConnection`] model.
pub(crate) fn config_to_db_conn(cc: &ConnectionConfig) -> DbConnection {
    let ssh = if cc.ssh_enabled {
        cc.ssh_host.as_ref().map(|host| SshTunnelConfig {
            host: host.clone(),
            port: cc.ssh_port.unwrap_or(22),
            user: cc.ssh_user.clone().unwrap_or_default(),
            auth: match cc.ssh_auth_method {
                SshAuthMethod::Password => SshAuth::Password,
                SshAuthMethod::PrivateKey => SshAuth::PrivateKey {
                    key_path: cc.ssh_key_path.clone().unwrap_or_default(),
                },
            },
            remote_host: cc.host.clone().unwrap_or_default(),
            remote_port: cc.port.unwrap_or(5432),
            ssh_password_encrypted: cc.ssh_password_encrypted.clone(),
            ssh_passphrase_encrypted: cc.ssh_passphrase_encrypted.clone(),
        })
    } else {
        None
    };

    let ssl = if cc.ssl_enabled {
        Some(SslConfig {
            mode: match cc.ssl_mode {
                wf_config::models::SslMode::Require => SslMode::Require,
                wf_config::models::SslMode::VerifyCa => SslMode::VerifyCa,
                wf_config::models::SslMode::VerifyFull => SslMode::VerifyFull,
            },
            ca_cert: cc.ssl_ca_cert.as_deref().map(std::path::PathBuf::from),
            client_cert: cc.ssl_client_cert.as_deref().map(std::path::PathBuf::from),
            client_key: cc.ssl_client_key.as_deref().map(std::path::PathBuf::from),
        })
    } else {
        None
    };

    DbConnection {
        id: cc.id.clone(),
        name: cc.name.clone(),
        db_type: match cc.db_type {
            DbTypeName::PostgreSQL => DbType::PostgreSQL,
            DbTypeName::MySQL => DbType::MySQL,
            DbTypeName::SQLite => DbType::SQLite,
        },
        connection_string: cc.connection_string.clone(),
        host: cc.host.clone(),
        port: cc.port,
        user: cc.user.clone(),
        password_encrypted: cc.password_encrypted.clone(),
        database: cc.database.clone(),
        ssh,
        ssl,
    }
}

/// Convert a runtime [`DbConnection`] to the storable [`ConnectionConfig`] model.
pub(crate) fn db_to_config_conn(conn: &DbConnection) -> ConnectionConfig {
    let ssh_enabled = conn.ssh.is_some();
    let (
        ssh_host,
        ssh_port,
        ssh_user,
        ssh_auth_method,
        ssh_key_path,
        ssh_password_encrypted,
        ssh_passphrase_encrypted,
    ) = if let Some(ref ssh) = conn.ssh {
        let (auth_method, key_path) = match &ssh.auth {
            SshAuth::Password => (SshAuthMethod::Password, None),
            SshAuth::PrivateKey { key_path } => (SshAuthMethod::PrivateKey, Some(key_path.clone())),
        };
        (
            Some(ssh.host.clone()),
            Some(ssh.port),
            Some(ssh.user.clone()),
            auth_method,
            key_path,
            ssh.ssh_password_encrypted.clone(),
            ssh.ssh_passphrase_encrypted.clone(),
        )
    } else {
        (None, None, None, SshAuthMethod::Password, None, None, None)
    };

    let ssl_enabled = conn.ssl.is_some();
    let (ssl_mode, ssl_ca_cert, ssl_client_cert, ssl_client_key) = if let Some(ref ssl) = conn.ssl {
        let mode = match ssl.mode {
            SslMode::Require => wf_config::models::SslMode::Require,
            SslMode::VerifyCa => wf_config::models::SslMode::VerifyCa,
            SslMode::VerifyFull => wf_config::models::SslMode::VerifyFull,
        };
        (
            mode,
            ssl.ca_cert
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            ssl.client_cert
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            ssl.client_key
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        )
    } else {
        (wf_config::models::SslMode::Require, None, None, None)
    };

    ConnectionConfig {
        id: conn.id.clone(),
        name: conn.name.clone(),
        db_type: match conn.db_type {
            DbType::PostgreSQL => DbTypeName::PostgreSQL,
            DbType::MySQL => DbTypeName::MySQL,
            DbType::SQLite => DbTypeName::SQLite,
        },
        connection_string: conn.connection_string.clone(),
        host: conn.host.clone(),
        port: conn.port,
        user: conn.user.clone(),
        password_encrypted: conn.password_encrypted.clone(),
        database: conn.database.clone(),
        safe_dml: true,
        read_only: false,
        ssh_enabled,
        ssh_host,
        ssh_port,
        ssh_user,
        ssh_auth_method,
        ssh_key_path,
        ssh_password_encrypted,
        ssh_passphrase_encrypted,
        ssl_enabled,
        ssl_mode,
        ssl_ca_cert,
        ssl_client_cert,
        ssl_client_key,
        group_id: None,
        color: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use wf_config::{manager::ConfigManager, models::SshAuthMethod};
    use wf_db::models::{DbType, SshAuth, SshTunnelConfig, SslConfig, SslMode};

    use super::{SessionManager, config_to_db_conn, db_to_config_conn};
    use wf_config::models::{ConnectionConfig, DbTypeName};

    #[test]
    fn save_tab_width_should_persist_to_config() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        sm.save_tab_width(4).unwrap();

        let cfg = ConfigManager::with_path(dir.path().join("config.toml"))
            .load()
            .unwrap();
        assert_eq!(cfg.editor.tab_width, 4);
    }

    #[test]
    fn save_query_timeout_should_persist_to_config() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        sm.save_query_timeout(30).unwrap();

        let cfg = ConfigManager::with_path(dir.path().join("config.toml"))
            .load()
            .unwrap();
        assert_eq!(cfg.editor.query_timeout_secs, 30);
    }

    #[test]
    fn save_page_size_should_persist_to_config() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        sm.save_page_size(1000).unwrap();

        let cfg = ConfigManager::with_path(dir.path().join("config.toml"))
            .load()
            .unwrap();
        use wf_config::models::PageSize;
        assert_eq!(cfg.editor.page_size, PageSize::Rows1000);
    }

    #[test]
    fn config_to_db_conn_should_map_ssh_fields_when_enabled() {
        let cc = ConnectionConfig {
            id: "c1".into(),
            name: "c1".into(),
            db_type: DbTypeName::PostgreSQL,
            connection_string: None,
            host: Some("db.internal".into()),
            port: Some(5432),
            user: Some("admin".into()),
            password_encrypted: None,
            database: Some("mydb".into()),
            safe_dml: true,
            read_only: false,
            ssh_enabled: true,
            ssh_host: Some("bastion.example.com".into()),
            ssh_port: Some(22),
            ssh_user: Some("ec2-user".into()),
            ssh_auth_method: SshAuthMethod::PrivateKey,
            ssh_password_encrypted: None,
            ssh_key_path: Some("/home/user/.ssh/id_rsa".into()),
            ssh_passphrase_encrypted: Some("enc:xyz".into()),
            ssl_enabled: false,
            ssl_mode: wf_config::models::SslMode::Require,
            ssl_ca_cert: None,
            ssl_client_cert: None,
            ssl_client_key: None,
            group_id: None,
            color: None,
        };

        let conn = config_to_db_conn(&cc);
        let ssh = conn.ssh.expect("ssh should be Some when ssh_enabled");

        assert_eq!(ssh.host, "bastion.example.com");
        assert_eq!(ssh.port, 22);
        assert_eq!(ssh.user, "ec2-user");
        assert_eq!(ssh.remote_host, "db.internal");
        assert_eq!(ssh.remote_port, 5432);
        assert_eq!(ssh.ssh_passphrase_encrypted.as_deref(), Some("enc:xyz"));
        assert!(
            matches!(ssh.auth, SshAuth::PrivateKey { ref key_path } if key_path == "/home/user/.ssh/id_rsa")
        );
    }

    #[test]
    fn config_to_db_conn_should_return_no_ssh_when_disabled() {
        let cc = ConnectionConfig {
            id: "c2".into(),
            name: "c2".into(),
            db_type: DbTypeName::SQLite,
            connection_string: Some("sqlite::memory:".into()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            safe_dml: true,
            read_only: false,
            ssh_enabled: false,
            ssh_host: Some("should-be-ignored.example.com".into()),
            ssh_port: Some(22),
            ssh_user: None,
            ssh_auth_method: SshAuthMethod::Password,
            ssh_password_encrypted: None,
            ssh_key_path: None,
            ssh_passphrase_encrypted: None,
            ssl_enabled: false,
            ssl_mode: wf_config::models::SslMode::Require,
            ssl_ca_cert: None,
            ssl_client_cert: None,
            ssl_client_key: None,
            group_id: None,
            color: None,
        };

        let conn = config_to_db_conn(&cc);
        assert!(
            conn.ssh.is_none(),
            "ssh should be None when ssh_enabled=false"
        );
    }

    #[test]
    fn db_to_config_conn_should_round_trip_ssh() {
        use wf_db::models::DbConnection;

        let conn = DbConnection {
            id: "c3".into(),
            name: "SSH connection".into(),
            db_type: DbType::PostgreSQL,
            connection_string: None,
            host: Some("db.internal".into()),
            port: Some(5432),
            user: Some("admin".into()),
            password_encrypted: None,
            database: Some("mydb".into()),
            ssh: Some(SshTunnelConfig {
                host: "bastion.example.com".into(),
                port: 2222,
                user: "jumper".into(),
                auth: SshAuth::Password,
                remote_host: "db.internal".into(),
                remote_port: 5432,
                ssh_password_encrypted: Some("enc:abc".into()),
                ssh_passphrase_encrypted: None,
            }),
            ssl: None,
        };

        let cc = db_to_config_conn(&conn);
        assert!(cc.ssh_enabled);
        assert_eq!(cc.ssh_host.as_deref(), Some("bastion.example.com"));
        assert_eq!(cc.ssh_port, Some(2222));
        assert_eq!(cc.ssh_user.as_deref(), Some("jumper"));
        assert_eq!(cc.ssh_auth_method, SshAuthMethod::Password);
        assert_eq!(cc.ssh_password_encrypted.as_deref(), Some("enc:abc"));
        assert_eq!(cc.ssh_passphrase_encrypted, None);
    }

    #[test]
    fn config_to_db_conn_should_map_ssl_fields_when_enabled() {
        use wf_config::models::SslMode as ConfigSslMode;

        let cc = ConnectionConfig {
            id: "ssl-c1".into(),
            name: "ssl-c1".into(),
            db_type: DbTypeName::PostgreSQL,
            connection_string: None,
            host: Some("db.internal".into()),
            port: Some(5432),
            user: Some("admin".into()),
            password_encrypted: None,
            database: Some("mydb".into()),
            safe_dml: true,
            read_only: false,
            ssh_enabled: false,
            ssh_host: None,
            ssh_port: None,
            ssh_user: None,
            ssh_auth_method: SshAuthMethod::Password,
            ssh_password_encrypted: None,
            ssh_key_path: None,
            ssh_passphrase_encrypted: None,
            ssl_enabled: true,
            ssl_mode: ConfigSslMode::VerifyFull,
            ssl_ca_cert: Some("/certs/ca.pem".into()),
            ssl_client_cert: Some("/certs/client.pem".into()),
            ssl_client_key: Some("/certs/client.key".into()),
            group_id: None,
            color: None,
        };

        let conn = config_to_db_conn(&cc);
        let ssl = conn.ssl.expect("ssl should be Some when ssl_enabled");

        assert_eq!(ssl.mode, SslMode::VerifyFull);
        assert_eq!(
            ssl.ca_cert.as_deref(),
            Some(std::path::Path::new("/certs/ca.pem"))
        );
        assert_eq!(
            ssl.client_cert.as_deref(),
            Some(std::path::Path::new("/certs/client.pem"))
        );
        assert_eq!(
            ssl.client_key.as_deref(),
            Some(std::path::Path::new("/certs/client.key"))
        );
    }

    #[test]
    fn db_to_config_conn_should_round_trip_ssl() {
        use wf_db::models::DbConnection;

        let conn = DbConnection {
            id: "ssl-c2".into(),
            name: "SSL connection".into(),
            db_type: DbType::PostgreSQL,
            connection_string: None,
            host: Some("db.internal".into()),
            port: Some(5432),
            user: None,
            password_encrypted: None,
            database: None,
            ssh: None,
            ssl: Some(SslConfig {
                mode: SslMode::VerifyCa,
                ca_cert: Some(std::path::PathBuf::from("/certs/ca.pem")),
                client_cert: None,
                client_key: None,
            }),
        };

        let cc = db_to_config_conn(&conn);
        assert!(cc.ssl_enabled);
        assert_eq!(cc.ssl_mode, wf_config::models::SslMode::VerifyCa);
        assert_eq!(cc.ssl_ca_cert.as_deref(), Some("/certs/ca.pem"));
        assert_eq!(cc.ssl_client_cert, None);
        assert_eq!(cc.ssl_client_key, None);
    }
}

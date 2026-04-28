//! Session persistence for the last active database connection.
//!
//! [`SessionManager`] wraps [`ConfigManager`] and provides two operations:
//!
//! - [`SessionManager::save_connection`] — called after every successful connect to
//!   upsert the connection entry and record it as `last_connection_id` in `config.toml`.
//! - [`SessionManager::restore`] — called at startup to retrieve the connection that was
//!   active when the app last closed, enabling automatic reconnect.
//!
//! Conversion between `wf_db::models::DbConnection` and `wf_config::models::ConnectionConfig`
//! lives here because `app/` is the only crate that depends on both.

use anyhow::Context as _;
use tracing::{info, warn};
use wf_config::{
    manager::ConfigManager,
    models::{ConnectionConfig, DbTypeName, PageSize, Theme},
};
use wf_db::models::{DbConnection, DbType};

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
    /// Primarily used in tests to point at a temporary directory.
    pub fn with_config_manager(cm: ConfigManager) -> Self {
        Self { config_manager: cm }
    }

    /// Persist `conn` as the last active connection.
    ///
    /// Upserts the connection entry in `config.connections` (matched by `id`)
    /// and sets `config.session.last_connection_id`. The config file is then
    /// saved atomically.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be loaded or written.
    pub fn save_connection(&self, conn: &DbConnection) -> anyhow::Result<()> {
        let mut config = self
            .config_manager
            .load()
            .context("failed to load config for session save")?;

        let cc = db_to_config_conn(conn);
        match config.connections.iter_mut().find(|c| c.id == conn.id) {
            Some(existing) => *existing = cc,
            None => config.connections.push(cc),
        }
        config.session.last_connection_id = Some(conn.id.clone());

        self.config_manager
            .save(&config)
            .context("failed to save session")?;
        info!(conn_id = %conn.id, "session saved");
        Ok(())
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

    /// Write the current editor query to `last_query.sql` in the app config directory.
    ///
    /// An empty query writes an empty file (not an error); `restore_query_file`
    /// treats an empty or absent file as "no saved query".
    pub fn save_query_file(&self, query: &str) -> anyhow::Result<()> {
        let path = self.config_manager.dir().join("last_query.sql");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        std::fs::write(&path, query.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        info!(len = query.len(), "last_query.sql saved");
        Ok(())
    }

    /// Read `last_query.sql` from the app config directory.
    ///
    /// Returns `Ok(None)` when the file does not exist or is empty.
    pub fn restore_query_file(&self) -> anyhow::Result<Option<String>> {
        let path = self.config_manager.dir().join("last_query.sql");
        if !path.exists() {
            return Ok(None);
        }
        let query = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if query.is_empty() {
            return Ok(None);
        }
        info!(len = query.len(), "last_query.sql restored");
        Ok(Some(query))
    }

    /// Load the last session and return the connection to auto-connect to.
    ///
    /// Returns `Ok(None)` when:
    /// - no config file exists yet, or
    /// - `last_connection_id` is not set, or
    /// - the recorded id has no matching entry in `config.connections`.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file exists but cannot be parsed.
    pub fn restore(&self) -> anyhow::Result<Option<DbConnection>> {
        let config = self
            .config_manager
            .load()
            .context("failed to load config for session restore")?;

        let Some(last_id) = &config.session.last_connection_id else {
            return Ok(None);
        };

        let conn = config
            .connections
            .iter()
            .find(|c| &c.id == last_id)
            .map(config_to_db_conn);

        if conn.is_some() {
            info!(conn_id = %last_id, "session restore: found connection");
        } else {
            warn!(conn_id = %last_id, "session restore: id not found in connections list");
        }

        Ok(conn)
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Conversion helpers (only in app/ — sees both wf-config and wf-db) ────────

/// Convert a stored [`ConnectionConfig`] to the runtime [`DbConnection`] model.
fn config_to_db_conn(cc: &ConnectionConfig) -> DbConnection {
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
    }
}

/// Convert a runtime [`DbConnection`] to the storable [`ConnectionConfig`] model.
fn db_to_config_conn(conn: &DbConnection) -> ConnectionConfig {
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
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use wf_config::manager::ConfigManager;
    use wf_db::models::{DbConnection, DbType};

    use super::SessionManager;

    fn sqlite_conn(id: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: id.to_string(),
            db_type: DbType::SQLite,
            connection_string: Some("sqlite::memory:".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
        }
    }

    #[test]
    fn restore_should_return_none_when_config_absent() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        assert!(sm.restore().unwrap().is_none());
    }

    #[test]
    fn save_connection_should_persist_last_connection_id() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        sm.save_connection(&sqlite_conn("c1")).unwrap();

        let cfg = ConfigManager::with_path(dir.path().join("config.toml"))
            .load()
            .unwrap();
        assert_eq!(cfg.session.last_connection_id, Some("c1".to_string()));
        assert_eq!(cfg.connections.len(), 1);
        assert_eq!(cfg.connections[0].id, "c1");
    }

    #[test]
    fn restore_should_return_connection_when_id_exists() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        sm.save_connection(&sqlite_conn("c2")).unwrap();

        let restored = sm.restore().unwrap().expect("should restore connection");
        assert_eq!(restored.id, "c2");
        assert_eq!(restored.db_type, DbType::SQLite);
    }

    #[test]
    fn restore_should_return_none_when_id_not_in_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Write a config with a last_connection_id but no matching connection entry
        let mut cfg = wf_config::models::Config::default();
        cfg.session.last_connection_id = Some("ghost".to_string());
        ConfigManager::with_path(path.clone()).save(&cfg).unwrap();

        let sm = SessionManager::with_config_manager(ConfigManager::with_path(path));
        assert!(sm.restore().unwrap().is_none());
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
    fn save_connection_should_upsert_not_duplicate() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        // Save same id twice
        sm.save_connection(&sqlite_conn("c3")).unwrap();
        sm.save_connection(&sqlite_conn("c3")).unwrap();

        let cfg = ConfigManager::with_path(dir.path().join("config.toml"))
            .load()
            .unwrap();
        assert_eq!(cfg.connections.len(), 1, "should not duplicate");
    }

    #[test]
    fn save_query_file_should_persist_and_restore_query() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        sm.save_query_file("SELECT 1").unwrap();

        let restored = sm.restore_query_file().unwrap();
        assert_eq!(restored, Some("SELECT 1".to_string()));
        assert!(dir.path().join("last_query.sql").exists());
    }

    #[test]
    fn save_query_file_should_not_affect_config_toml() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        sm.save_connection(&sqlite_conn("c1")).unwrap();
        sm.save_query_file("SELECT 2").unwrap();

        let cfg = ConfigManager::with_path(dir.path().join("config.toml"))
            .load()
            .unwrap();
        assert_eq!(cfg.session.last_connection_id, Some("c1".to_string()));
        assert_eq!(cfg.session.last_query, None);
        assert_eq!(cfg.connections.len(), 1);
    }
}

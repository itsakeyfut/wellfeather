//! Session persistence helpers.
//!
//! [`SessionManager`] wraps [`ConfigManager`] and persists editor/tab state,
//! page size, language, and theme settings to `config.toml`.
//!
//! Connection persistence (CRUD + last-used tracking) has moved to
//! [`wf_config::ConnectionRepository`] (SQLite-backed).
//!
//! Conversion between `wf_db::models::DbConnection` and `wf_config::models::ConnectionConfig`
//! lives here because `app/` is the only crate that depends on both.

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use tracing::info;
use wf_config::{
    manager::ConfigManager,
    models::{ConnectionConfig, DbTypeName, PageSize, Theme},
};
use wf_db::models::{DbConnection, DbType};

/// Serializable record for a single SQL Editor tab (written to `tabs.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabSessionEntry {
    pub id: String,
    pub title: String,
    pub query_text: String,
}

#[derive(Serialize, Deserialize)]
struct TabsFile {
    active_index: usize,
    tabs: Vec<TabSessionEntry>,
}

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

    /// Persist the current set of SQL Editor tabs to `tabs.toml`.
    ///
    /// `active_index` is the index within the serialized list (sql-editor tabs only).
    /// Table View tabs are intentionally not persisted — they are easy to reopen.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_tabs(&self, active_index: usize, tabs: &[TabSessionEntry]) -> anyhow::Result<()> {
        let path = self.config_manager.dir().join("tabs.toml");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let file = TabsFile {
            active_index,
            tabs: tabs.to_vec(),
        };
        let s = toml::to_string_pretty(&file).context("failed to serialize tabs")?;
        std::fs::write(&path, s.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        info!(count = tabs.len(), "tabs.toml saved");
        Ok(())
    }

    /// Read `tabs.toml` and return `(active_index, tabs)`.
    ///
    /// Returns `Ok(None)` when the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn restore_tabs(&self) -> anyhow::Result<Option<(usize, Vec<TabSessionEntry>)>> {
        let path = self.config_manager.dir().join("tabs.toml");
        if !path.exists() {
            return Ok(None);
        }
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let file: TabsFile = toml::from_str(&s).context("failed to parse tabs.toml")?;
        info!(count = file.tabs.len(), "tabs.toml restored");
        Ok(Some((file.active_index, file.tabs)))
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
pub(crate) fn db_to_config_conn(conn: &DbConnection) -> ConnectionConfig {
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
        safe_dml: true, // default; overwritten by save_connection when updating existing entry
        read_only: false, // default; overwritten by save_connection when updating existing entry
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use wf_config::manager::ConfigManager;

    use super::SessionManager;

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
    fn save_tabs_should_persist_and_restore() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));

        let tabs = vec![
            super::TabSessionEntry {
                id: "t1".to_string(),
                title: "Query 1".to_string(),
                query_text: "SELECT 1".to_string(),
            },
            super::TabSessionEntry {
                id: "t2".to_string(),
                title: "Query 2".to_string(),
                query_text: "SELECT 2".to_string(),
            },
        ];
        sm.save_tabs(1, &tabs).unwrap();

        let (active, restored) = sm.restore_tabs().unwrap().expect("should restore");
        assert_eq!(active, 1);
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].id, "t1");
        assert_eq!(restored[1].query_text, "SELECT 2");
    }

    #[test]
    fn restore_tabs_should_return_none_when_absent() {
        let dir = tempdir().unwrap();
        let sm = SessionManager::with_config_manager(ConfigManager::with_path(
            dir.path().join("config.toml"),
        ));
        assert!(sm.restore_tabs().unwrap().is_none());
    }
}

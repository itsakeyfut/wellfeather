use std::{fs, path::PathBuf};

use anyhow::Context as _;

use crate::models::Config;

// ---------------------------------------------------------------------------
// ConfigManager
// ---------------------------------------------------------------------------

/// Manages loading and atomically saving `config.toml` from the OS-specific
/// application configuration directory.
pub struct ConfigManager {
    /// Full path to `config.toml`.
    path: PathBuf,
}

impl ConfigManager {
    /// Creates a `ConfigManager` that reads/writes the production config path
    /// (`app_dir()/config.toml`).
    pub fn new() -> Self {
        Self {
            path: Self::app_dir().join("config.toml"),
        }
    }

    /// Creates a `ConfigManager` pointed at an arbitrary path.
    /// Primarily used in tests and tooling.
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Returns the OS-specific application configuration directory.
    ///
    /// | Platform | Path                                              |
    /// |----------|---------------------------------------------------|
    /// | Windows  | `%APPDATA%\wellfeather`                           |
    /// | macOS    | `~/Library/Application Support/wellfeather`       |
    /// | Linux    | `~/.config/wellfeather`                           |
    pub fn app_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "wellfeather")
            .expect("cannot determine OS application configuration directory")
            .config_dir()
            .to_path_buf()
    }

    /// Returns the directory that contains `config.toml`.
    ///
    /// Equivalent to `app_dir()` for the default instance; when created with
    /// [`ConfigManager::with_path`] the directory of the supplied path is returned
    /// instead — useful for tests that point at a temp directory.
    pub fn dir(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf()
    }

    /// Loads configuration from `config.toml`.
    ///
    /// Returns `Config::default()` when the file does not exist.
    /// Propagates I/O and parse errors.
    pub fn load(&self) -> anyhow::Result<Config> {
        if !self.path.exists() {
            return Ok(Config::default());
        }
        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", self.path.display()))?;
        Ok(config)
    }

    /// Serializes `config` and writes it atomically to `config.toml`.
    ///
    /// Atomic write sequence:
    /// 1. Ensure the parent directory exists.
    /// 2. Write the serialized TOML to `config.toml.tmp` (same directory).
    /// 3. Rename `config.toml.tmp` → `config.toml` (atomic on both Unix and Windows).
    ///
    /// The `.toml.tmp` suffix is sufficient because wellfeather is a single-process
    /// desktop application; concurrent config writes are not expected.
    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }

        let serialized = toml::to_string_pretty(config).context("failed to serialize config")?;

        let tmp = self.path.with_extension("toml.tmp");
        fs::write(&tmp, &serialized)
            .with_context(|| format!("failed to write temp file {}", tmp.display()))?;

        fs::rename(&tmp, &self.path).with_context(|| {
            format!(
                "failed to rename {} → {}",
                tmp.display(),
                self.path.display()
            )
        })?;

        Ok(())
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AppearanceConfig, ConnectionConfig, DbTypeName, Theme};

    #[test]
    fn config_manager_should_return_default_when_file_absent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let mgr = ConfigManager::with_path(dir.path().join("config.toml"));

        let cfg = mgr.load().expect("load should succeed");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn config_manager_should_roundtrip_save_and_load() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let mgr = ConfigManager::with_path(dir.path().join("config.toml"));

        let original = Config {
            appearance: AppearanceConfig {
                theme: Theme::Light,
                font_family: "Cascadia Code".to_string(),
                font_size: 16,
            },
            connections: vec![ConnectionConfig {
                id: "test-id".to_string(),
                name: "local".to_string(),
                db_type: DbTypeName::SQLite,
                connection_string: None,
                host: None,
                port: None,
                user: None,
                password_encrypted: None,
                database: Some("local.db".to_string()),
                safe_dml: true,
            }],
            ..Config::default()
        };

        mgr.save(&original).expect("save should succeed");
        let loaded = mgr.load().expect("load should succeed");
        assert_eq!(original, loaded);
    }
}

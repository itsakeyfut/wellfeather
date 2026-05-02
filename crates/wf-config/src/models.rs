use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// DbTypeName — config-level DB type identifier
// Separate from wf-db::DbType so that wf-config has no dependency on wf-db.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DbTypeName {
    #[default]
    #[serde(rename = "postgresql")]
    PostgreSQL,
    #[serde(rename = "mysql")]
    MySQL,
    #[serde(rename = "sqlite")]
    SQLite,
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

// ---------------------------------------------------------------------------
// PageSize — serialised as a TOML integer (100 / 500 / 1000)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(try_from = "u32", into = "u32")]
pub enum PageSize {
    Rows100,
    #[default]
    Rows500,
    Rows1000,
}

impl From<PageSize> for u32 {
    fn from(p: PageSize) -> u32 {
        match p {
            PageSize::Rows100 => 100,
            PageSize::Rows500 => 500,
            PageSize::Rows1000 => 1000,
        }
    }
}

impl TryFrom<u32> for PageSize {
    type Error = String;

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        match v {
            100 => Ok(PageSize::Rows100),
            500 => Ok(PageSize::Rows500),
            1000 => Ok(PageSize::Rows1000),
            _ => Err(format!(
                "unknown page_size value: {v}; expected 100, 500, or 1000"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// AppearanceConfig  [appearance]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: Theme,
    pub font_family: String,
    pub font_size: u32,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            font_family: "JetBrains Mono".to_string(),
            font_size: 14,
        }
    }
}

// ---------------------------------------------------------------------------
// EditorConfig  [editor]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EditorConfig {
    pub page_size: PageSize,
}

// ---------------------------------------------------------------------------
// SessionConfig  [session]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionConfig {
    /// Preserved for migration only — read from old config.toml but never written back.
    #[serde(default, skip_serializing)]
    pub last_connection_id: Option<String>,
    pub last_query: Option<String>,
}

// ---------------------------------------------------------------------------
// UiConfig  [ui]  (spec §21 — language selection)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub language: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectionConfig  [[connections]]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub db_type: DbTypeName,
    /// Connection string mode: `postgres://user:pass@host:5432/dbname`
    #[serde(default)]
    pub connection_string: Option<String>,
    /// Individual field mode
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    /// AES-256-GCM encrypted password (see wf-config::crypto)
    #[serde(default)]
    pub password_encrypted: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    /// When true, UPDATE/DELETE without WHERE shows a confirmation dialog.
    #[serde(default = "default_safe_dml")]
    pub safe_dml: bool,
    /// When true, write statements (INSERT/UPDATE/DELETE/DDL) are blocked before execution.
    #[serde(default)]
    pub read_only: bool,
}

fn default_safe_dml() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Config — top-level structure mapping to the entire config.toml
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub appearance: AppearanceConfig,
    pub editor: EditorConfig,
    pub session: SessionConfig,
    pub ui: UiConfig,
    /// Preserved for migration only — read from old config.toml but never written back.
    #[serde(default, skip_serializing)]
    pub connections: Vec<ConnectionConfig>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_should_return_valid_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.appearance.theme, Theme::Dark);
        assert_eq!(cfg.appearance.font_family, "JetBrains Mono");
        assert_eq!(cfg.appearance.font_size, 14);
        assert_eq!(cfg.editor.page_size, PageSize::Rows500);
        assert_eq!(cfg.session.last_connection_id, None);
        assert_eq!(cfg.session.last_query, None);
        assert_eq!(cfg.ui.language, "en");
        assert!(cfg.connections.is_empty());
    }

    #[test]
    fn config_should_deserialize_from_full_toml() {
        let toml = r#"
[appearance]
theme = "light"
font_family = "Fira Code"
font_size = 16

[editor]
page_size = 1000

[session]
last_connection_id = "uuid-abc"
last_query = "SELECT * FROM users"

[ui]
language = "ja"

[[connections]]
id = "uuid-abc"
name = "my_postgres"
db_type = "postgresql"
host = "localhost"
port = 5432
user = "admin"
password_encrypted = "AES256GCM:abc123"
database = "mydb"

[[connections]]
id = "uuid-def"
name = "local_sqlite"
db_type = "sqlite"
database = "local.db"
"#;
        let cfg: Config = toml::from_str(toml).expect("failed to deserialize");

        assert_eq!(cfg.appearance.theme, Theme::Light);
        assert_eq!(cfg.appearance.font_family, "Fira Code");
        assert_eq!(cfg.appearance.font_size, 16);
        assert_eq!(cfg.editor.page_size, PageSize::Rows1000);
        assert_eq!(cfg.session.last_connection_id, Some("uuid-abc".to_string()));
        assert_eq!(
            cfg.session.last_query,
            Some("SELECT * FROM users".to_string())
        );
        assert_eq!(cfg.ui.language, "ja");

        assert_eq!(cfg.connections.len(), 2);
        let pg = &cfg.connections[0];
        assert_eq!(pg.id, "uuid-abc");
        assert_eq!(pg.name, "my_postgres");
        assert_eq!(pg.db_type, DbTypeName::PostgreSQL);
        assert_eq!(pg.host, Some("localhost".to_string()));
        assert_eq!(pg.port, Some(5432));
        assert_eq!(pg.user, Some("admin".to_string()));
        assert_eq!(pg.password_encrypted, Some("AES256GCM:abc123".to_string()));
        assert_eq!(pg.database, Some("mydb".to_string()));

        let sq = &cfg.connections[1];
        assert_eq!(sq.db_type, DbTypeName::SQLite);
        assert_eq!(sq.host, None);
        assert_eq!(sq.port, None);
    }

    #[test]
    fn config_should_deserialize_from_minimal_toml() {
        // Entirely empty config — all sections missing
        let cfg: Config = toml::from_str("").expect("failed to deserialize empty config");
        assert_eq!(cfg.appearance.theme, Theme::Dark);
        assert_eq!(cfg.editor.page_size, PageSize::Rows500);
        assert_eq!(cfg.ui.language, "en");
        assert!(cfg.connections.is_empty());
    }

    #[test]
    fn page_size_should_serialize_as_integer() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            page_size: PageSize,
        }
        let w = Wrapper {
            page_size: PageSize::Rows500,
        };
        let s = toml::to_string(&w).expect("failed to serialize");
        // Must contain the integer 500, not the string "500"
        assert!(s.contains("page_size = 500"), "got: {s}");
        assert!(
            !s.contains(r#"page_size = "500""#),
            "serialized as string: {s}"
        );

        // Round-trip: integer 500 → PageSize::Rows500
        let back: Wrapper = toml::from_str(&s).expect("failed to deserialize");
        assert_eq!(back.page_size, PageSize::Rows500);
    }

    #[test]
    fn config_should_roundtrip_serialize_deserialize() {
        // connections and session.last_connection_id are skip_serializing (migration-only
        // fields), so they are intentionally excluded from the roundtrip assertion.
        let original = Config {
            appearance: AppearanceConfig {
                theme: Theme::Light,
                font_family: "Cascadia Code".to_string(),
                font_size: 13,
            },
            editor: EditorConfig {
                page_size: PageSize::Rows100,
            },
            session: SessionConfig {
                last_connection_id: None,
                last_query: Some("SELECT 1".to_string()),
            },
            ui: UiConfig {
                language: "ja".to_string(),
            },
            connections: vec![],
        };

        let serialized = toml::to_string(&original).expect("failed to serialize");
        let deserialized: Config = toml::from_str(&serialized).expect("failed to deserialize");
        assert_eq!(original, deserialized);
    }
}

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
    /// When true, all UI animation durations collapse to 0ms.
    pub reduce_motion: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            font_family: "JetBrains Mono".to_string(),
            font_size: 14,
            reduce_motion: false,
        }
    }
}

// ---------------------------------------------------------------------------
// EditorConfig  [editor]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    pub page_size: PageSize,
    pub tab_width: u32,
    /// Query timeout in seconds. `0` means no timeout.
    pub query_timeout_secs: u64,
    /// Slow query warning threshold in milliseconds. `0` means disabled.
    pub slow_query_threshold_ms: u64,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            page_size: PageSize::default(),
            tab_width: 2,
            query_timeout_secs: 0,
            slow_query_threshold_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// SessionConfig  [session]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionConfig {
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
// GroupConfig  [[group]]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_group_color")]
    pub color: String,
    #[serde(default = "default_true")]
    pub expanded: bool,
}

fn default_group_color() -> String {
    "#6c7086".to_string()
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// SslMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SslMode {
    #[default]
    Require,
    VerifyCa,
    VerifyFull,
}

// ---------------------------------------------------------------------------
// SshAuthMethod
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMethod {
    #[default]
    Password,
    PrivateKey,
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
    // ── SSH Tunnel ────────────────────────────────────────────────────────────
    #[serde(default)]
    pub ssh_enabled: bool,
    #[serde(default)]
    pub ssh_host: Option<String>,
    #[serde(default)]
    pub ssh_port: Option<u16>,
    #[serde(default)]
    pub ssh_user: Option<String>,
    #[serde(default)]
    pub ssh_auth_method: SshAuthMethod,
    /// AES-256-GCM encrypted SSH password
    #[serde(default)]
    pub ssh_password_encrypted: Option<String>,
    #[serde(default)]
    pub ssh_key_path: Option<String>,
    /// AES-256-GCM encrypted SSH private-key passphrase
    #[serde(default)]
    pub ssh_passphrase_encrypted: Option<String>,
    // ── SSL/TLS ───────────────────────────────────────────────────────────────
    #[serde(default)]
    pub ssl_enabled: bool,
    #[serde(default)]
    pub ssl_mode: SslMode,
    /// Path to the CA certificate PEM file (copied into config_dir).
    #[serde(default)]
    pub ssl_ca_cert: Option<String>,
    /// Path to the client certificate PEM file (copied into config_dir).
    #[serde(default)]
    pub ssl_client_cert: Option<String>,
    /// Path to the client private key PEM file (copied into config_dir).
    #[serde(default)]
    pub ssl_client_key: Option<String>,
    // ── Group membership ─────────────────────────────────────────────────────
    /// ID of the group this connection belongs to, or `None` for ungrouped.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Per-connection color override (CSS hex, e.g. "#e74c3c").
    /// When `None`, the parent group's color is used; ungrouped connections have no color.
    #[serde(default)]
    pub color: Option<String>,
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
        assert!(!cfg.appearance.reduce_motion);
        assert_eq!(cfg.editor.page_size, PageSize::Rows500);
        assert_eq!(cfg.editor.tab_width, 2);
        assert_eq!(cfg.editor.query_timeout_secs, 0);
        assert_eq!(cfg.session.last_query, None);
        assert_eq!(cfg.ui.language, "en");
    }

    #[test]
    fn config_should_deserialize_from_full_toml() {
        let toml = r#"
[appearance]
theme = "light"
font_family = "Fira Code"
font_size = 16
reduce_motion = true

[editor]
page_size = 1000
tab_width = 4

[session]
last_query = "SELECT * FROM users"

[ui]
language = "ja"
"#;
        let cfg: Config = toml::from_str(toml).expect("failed to deserialize");

        assert_eq!(cfg.appearance.theme, Theme::Light);
        assert_eq!(cfg.appearance.font_family, "Fira Code");
        assert_eq!(cfg.appearance.font_size, 16);
        assert_eq!(cfg.editor.page_size, PageSize::Rows1000);
        assert_eq!(cfg.editor.tab_width, 4);
        assert_eq!(
            cfg.session.last_query,
            Some("SELECT * FROM users".to_string())
        );
        assert_eq!(cfg.ui.language, "ja");
        assert!(cfg.appearance.reduce_motion);
    }

    #[test]
    fn config_should_deserialize_from_minimal_toml() {
        // Entirely empty config — all sections missing
        let cfg: Config = toml::from_str("").expect("failed to deserialize empty config");
        assert_eq!(cfg.appearance.theme, Theme::Dark);
        assert_eq!(cfg.editor.page_size, PageSize::Rows500);
        assert_eq!(cfg.ui.language, "en");
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
    fn ssh_config_defaults_should_be_false_and_none() {
        let toml = r#"
            id = "c1"
            name = "c1"
            db_type = "sqlite"
        "#;
        let cc: ConnectionConfig = toml::from_str(toml).expect("should parse with SSH defaults");
        assert!(!cc.ssh_enabled);
        assert_eq!(cc.ssh_host, None);
        assert_eq!(cc.ssh_port, None);
        assert_eq!(cc.ssh_user, None);
        assert_eq!(cc.ssh_auth_method, SshAuthMethod::Password);
        assert_eq!(cc.ssh_password_encrypted, None);
        assert_eq!(cc.ssh_key_path, None);
        assert_eq!(cc.ssh_passphrase_encrypted, None);
    }

    #[test]
    fn ssh_config_fields_should_round_trip_through_toml() {
        let original = ConnectionConfig {
            id: "ssh-test".into(),
            name: "SSH Test".into(),
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
            ssh_passphrase_encrypted: Some("enc:abc123".into()),
            ssl_enabled: false,
            ssl_mode: SslMode::Require,
            ssl_ca_cert: None,
            ssl_client_cert: None,
            ssl_client_key: None,
            group_id: None,
            color: None,
        };

        let serialized = toml::to_string(&original).expect("failed to serialize");
        let deserialized: ConnectionConfig =
            toml::from_str(&serialized).expect("failed to deserialize");

        assert_eq!(original, deserialized);
        assert!(deserialized.ssh_enabled);
        assert_eq!(
            deserialized.ssh_host.as_deref(),
            Some("bastion.example.com")
        );
        assert_eq!(deserialized.ssh_port, Some(22));
        assert_eq!(deserialized.ssh_auth_method, SshAuthMethod::PrivateKey);
    }

    #[test]
    fn ssl_config_defaults_should_be_disabled_and_none() {
        let toml = r#"
            id = "c1"
            name = "c1"
            db_type = "postgresql"
        "#;
        let cc: ConnectionConfig = toml::from_str(toml).expect("should parse with SSL defaults");
        assert!(!cc.ssl_enabled);
        assert_eq!(cc.ssl_mode, SslMode::Require);
        assert_eq!(cc.ssl_ca_cert, None);
        assert_eq!(cc.ssl_client_cert, None);
        assert_eq!(cc.ssl_client_key, None);
    }

    #[test]
    fn ssl_config_fields_should_round_trip_through_toml() {
        let original = ConnectionConfig {
            id: "ssl-test".into(),
            name: "SSL Test".into(),
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
            ssl_mode: SslMode::VerifyFull,
            ssl_ca_cert: Some("/config/certs/ssl-test/ca.pem".into()),
            ssl_client_cert: Some("/config/certs/ssl-test/client.pem".into()),
            ssl_client_key: Some("/config/certs/ssl-test/client.key".into()),
            group_id: None,
            color: None,
        };

        let serialized = toml::to_string(&original).expect("failed to serialize");
        let deserialized: ConnectionConfig =
            toml::from_str(&serialized).expect("failed to deserialize");

        assert_eq!(original, deserialized);
        assert!(deserialized.ssl_enabled);
        assert_eq!(deserialized.ssl_mode, SslMode::VerifyFull);
        assert_eq!(
            deserialized.ssl_ca_cert.as_deref(),
            Some("/config/certs/ssl-test/ca.pem")
        );
    }

    #[test]
    fn group_config_defaults_should_be_applied() {
        let toml = "id = \"g1\"\nname = \"Production\"";
        let g: GroupConfig = toml::from_str(toml).expect("should parse with defaults");
        assert_eq!(g.color, "#6c7086");
        assert!(g.expanded);
    }

    #[test]
    fn group_config_fields_should_round_trip_through_toml() {
        let original = GroupConfig {
            id: "g1".into(),
            name: "Production".into(),
            color: "#e74c3c".into(),
            expanded: false,
        };
        let s = toml::to_string(&original).expect("failed to serialize");
        let back: GroupConfig = toml::from_str(&s).expect("failed to deserialize");
        assert_eq!(original, back);
    }

    #[test]
    fn connection_config_group_fields_should_default_to_none() {
        let toml = "id = \"c1\"\nname = \"c1\"\ndb_type = \"sqlite\"";
        let cc: ConnectionConfig = toml::from_str(toml).expect("should parse");
        assert_eq!(cc.group_id, None);
        assert_eq!(cc.color, None);
    }

    #[test]
    fn config_should_roundtrip_serialize_deserialize() {
        let original = Config {
            appearance: AppearanceConfig {
                theme: Theme::Light,
                font_family: "Cascadia Code".to_string(),
                font_size: 13,
                reduce_motion: false,
            },
            editor: EditorConfig {
                page_size: PageSize::Rows100,
                tab_width: 4,
                query_timeout_secs: 30,
                slow_query_threshold_ms: 0,
            },
            session: SessionConfig {
                last_query: Some("SELECT 1".to_string()),
            },
            ui: UiConfig {
                language: "ja".to_string(),
            },
        };

        let serialized = toml::to_string(&original).expect("failed to serialize");
        let deserialized: Config = toml::from_str(&serialized).expect("failed to deserialize");
        assert_eq!(original, deserialized);
    }
}

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, load_secret_key};
use tokio::net::TcpListener;

use crate::error::DbError;
use crate::models::{SshAuth, SshTunnelConfig};

// ---------------------------------------------------------------------------
// SshTunnel
// ---------------------------------------------------------------------------

/// An active SSH port-forwarding tunnel.
///
/// Backed by a tokio task that accepts local TCP connections and forwards
/// each through an SSH direct-tcpip channel.  Dropping aborts the task.
pub struct SshTunnel {
    /// OS-assigned local port; use as the database host port.
    pub local_port: u16,
    _task: tokio::task::JoinHandle<()>,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self._task.abort();
    }
}

// ---------------------------------------------------------------------------
// Known-hosts helpers
// ---------------------------------------------------------------------------

/// Result of comparing a host fingerprint against the known-hosts store.
#[derive(Debug)]
pub enum KnownHostStatus {
    Trusted,
    Unknown,
    Mismatch { expected: String, actual: String },
}

fn load_known_hosts(path: &Path) -> HashMap<String, String> {
    if !path.exists() {
        return HashMap::new();
    }
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    toml::from_str::<HashMap<String, String>>(&content).unwrap_or_default()
}

fn write_known_hosts(path: &Path, hosts: &HashMap<String, String>) -> Result<(), DbError> {
    let content = toml::to_string(hosts).map_err(|e| DbError::KnownHostsWrite(e.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DbError::KnownHostsWrite(e.to_string()))?;
    }
    std::fs::write(path, &content).map_err(|e| DbError::KnownHostsWrite(e.to_string()))?;
    // Restrict to owner-read/write on Unix so fingerprints aren't world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Check `host:port` against the known-hosts TOML file.
pub fn check_known_host(host: &str, port: u16, fingerprint: &str, path: &Path) -> KnownHostStatus {
    let hosts = load_known_hosts(path);
    let key = format!("{host}:{port}");
    match hosts.get(&key) {
        None => KnownHostStatus::Unknown,
        Some(stored) if stored == fingerprint => KnownHostStatus::Trusted,
        Some(stored) => KnownHostStatus::Mismatch {
            expected: stored.clone(),
            actual: fingerprint.to_string(),
        },
    }
}

/// Persist a trusted fingerprint for `host:port` into the known-hosts TOML file.
pub fn save_known_host(
    host: &str,
    port: u16,
    fingerprint: &str,
    path: &Path,
) -> Result<(), DbError> {
    let mut hosts = load_known_hosts(path);
    hosts.insert(format!("{host}:{port}"), fingerprint.to_string());
    write_known_hosts(path, &hosts)
}

// ---------------------------------------------------------------------------
// Probe handler — captures fingerprint, always rejects
// ---------------------------------------------------------------------------

struct ProbeHandler {
    fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for ProbeHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, russh::Error> {
        let fp = format!("{}", server_public_key.fingerprint(HashAlg::Sha256));
        *self.fingerprint.lock().unwrap_or_else(|p| p.into_inner()) = Some(fp);
        Ok(false) // reject — we only want the fingerprint
    }
}

// ---------------------------------------------------------------------------
// Tunnel handler — stores actual fingerprint for mismatch detection
// ---------------------------------------------------------------------------

struct TunnelHandler {
    trusted_fp: String,
    /// Populated in check_server_key so the caller can detect mismatches.
    actual_fp: Arc<Mutex<Option<String>>>,
}

impl client::Handler for TunnelHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, russh::Error> {
        let actual = format!("{}", server_public_key.fingerprint(HashAlg::Sha256));
        *self.actual_fp.lock().unwrap_or_else(|p| p.into_inner()) = Some(actual.clone());
        // Return false (reject) when fingerprint doesn't match; caller checks actual_fp.
        Ok(actual == self.trusted_fp)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Probe an SSH server and return its host-key fingerprint (SHA-256 format).
///
/// The connection is rejected immediately after the fingerprint is captured,
/// so any resulting I/O error is suppressed.  Returns `Err` only when the
/// server is unreachable.
pub async fn probe_fingerprint(host: &str, port: u16) -> Result<String, DbError> {
    let fp_store: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let handler = ProbeHandler {
        fingerprint: Arc::clone(&fp_store),
    };
    let config = Arc::new(client::Config::default());
    // connect returns Err because check_server_key returns Ok(false) — expected
    let _ = client::connect(config, (host, port), handler).await;
    fp_store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .ok_or_else(|| {
            DbError::SshTunnelFailed(format!(
                "could not reach SSH server at {host}:{port} or retrieve host key"
            ))
        })
}

/// Establish an SSH tunnel that port-forwards to the DB host in `config`.
///
/// Returns an [`SshTunnel`] whose `local_port` should replace the database
/// host port.  Dropping the tunnel aborts the forwarding task.
pub async fn connect_tunnel(
    config: &SshTunnelConfig,
    ssh_password: Option<&str>,
    ssh_passphrase: Option<&str>,
    trusted_fp: &str,
) -> Result<SshTunnel, DbError> {
    let actual_fp_store: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let handler = TunnelHandler {
        trusted_fp: trusted_fp.to_string(),
        actual_fp: Arc::clone(&actual_fp_store),
    };
    let ssh_config = Arc::new(client::Config::default());

    let mut session = client::connect(ssh_config, (config.host.as_str(), config.port), handler)
        .await
        .map_err(|e| {
            // Distinguish fingerprint mismatch from general connection failure.
            if let Some(actual) = actual_fp_store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .filter(|a| a != trusted_fp)
            {
                return DbError::SshFingerprintMismatch {
                    expected: trusted_fp.to_string(),
                    actual,
                };
            }
            DbError::SshTunnelFailed(e.to_string())
        })?;

    let auth_result = match &config.auth {
        SshAuth::Password => {
            let pw = ssh_password.unwrap_or("");
            session
                .authenticate_password(&config.user, pw)
                .await
                .map_err(|e| DbError::SshTunnelFailed(e.to_string()))?
        }
        SshAuth::PrivateKey { key_path } => {
            let key = load_secret_key(key_path, ssh_passphrase)
                .map_err(|e| DbError::SshTunnelFailed(e.to_string()))?;
            let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
            session
                .authenticate_publickey(&config.user, key_with_alg)
                .await
                .map_err(|e| DbError::SshTunnelFailed(e.to_string()))?
        }
    };

    if !auth_result.success() {
        return Err(DbError::SshTunnelFailed(
            "SSH authentication failed (wrong credentials)".to_string(),
        ));
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| DbError::SshTunnelFailed(format!("failed to bind local port: {e}")))?;
    let local_port = listener
        .local_addr()
        .map_err(|e| DbError::SshTunnelFailed(format!("failed to get local port: {e}")))?
        .port();

    let remote_host = config.remote_host.clone();
    let remote_port = config.remote_port;

    let task = tokio::spawn(async move {
        loop {
            let (mut tcp_stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "SSH tunnel: TCP accept failed, stopping");
                    break;
                }
            };
            let channel = match session
                .channel_open_direct_tcpip(remote_host.as_str(), remote_port as u32, "127.0.0.1", 0)
                .await
            {
                Ok(ch) => ch,
                Err(e) => {
                    tracing::warn!(error = %e, "SSH tunnel: failed to open channel, stopping");
                    break;
                }
            };
            tokio::spawn(async move {
                let mut ssh_stream = channel.into_stream();
                if let Err(e) =
                    tokio::io::copy_bidirectional(&mut tcp_stream, &mut ssh_stream).await
                {
                    tracing::debug!(error = %e, "SSH tunnel: forwarding ended");
                }
            });
        }
    });

    Ok(SshTunnel {
        local_port,
        _task: task,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn known_host_check_should_return_unknown_for_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.toml");
        let result = check_known_host("db.example.com", 22, "SHA256:abc", &path);
        assert!(matches!(result, KnownHostStatus::Unknown));
    }

    #[test]
    fn known_host_check_should_return_trusted_for_matching_entry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.toml");
        save_known_host("db.example.com", 22, "SHA256:abc123", &path).unwrap();
        let result = check_known_host("db.example.com", 22, "SHA256:abc123", &path);
        assert!(matches!(result, KnownHostStatus::Trusted));
    }

    #[test]
    fn known_host_check_should_return_mismatch_for_wrong_fingerprint() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.toml");
        save_known_host("db.example.com", 22, "SHA256:stored", &path).unwrap();
        let result = check_known_host("db.example.com", 22, "SHA256:different", &path);
        assert!(
            matches!(
                result,
                KnownHostStatus::Mismatch { ref expected, .. } if expected == "SHA256:stored"
            ),
            "expected Mismatch with stored fingerprint"
        );
    }

    #[test]
    fn known_host_save_should_persist_new_entry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.toml");
        save_known_host("myhost", 2222, "SHA256:xyz", &path).unwrap();
        let hosts = load_known_hosts(&path);
        assert_eq!(
            hosts.get("myhost:2222").map(|s| s.as_str()),
            Some("SHA256:xyz")
        );
    }

    #[test]
    fn known_host_save_should_overwrite_existing_entry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.toml");
        save_known_host("myhost", 22, "SHA256:old", &path).unwrap();
        save_known_host("myhost", 22, "SHA256:new", &path).unwrap();
        let hosts = load_known_hosts(&path);
        assert_eq!(
            hosts.get("myhost:22").map(|s| s.as_str()),
            Some("SHA256:new")
        );
    }

    #[test]
    fn known_host_check_should_distinguish_ports() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_hosts.toml");
        save_known_host("myhost", 22, "SHA256:for22", &path).unwrap();
        save_known_host("myhost", 2222, "SHA256:for2222", &path).unwrap();
        assert!(matches!(
            check_known_host("myhost", 22, "SHA256:for22", &path),
            KnownHostStatus::Trusted
        ));
        assert!(matches!(
            check_known_host("myhost", 2222, "SHA256:for2222", &path),
            KnownHostStatus::Trusted
        ));
        assert!(matches!(
            check_known_host("myhost", 22, "SHA256:for2222", &path),
            KnownHostStatus::Mismatch { .. }
        ));
    }
}

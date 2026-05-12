use tokio::sync::oneshot;
use tracing::{info, warn};
use wf_config::crypto;
use wf_db::{
    models::DbConnection,
    tunnel::{
        KnownHostStatus, check_known_host, connect_tunnel, probe_fingerprint, save_known_host,
    },
};
use zeroize::Zeroizing;

use crate::app::{LocalizedMessage, event::Event, session::db_to_config_conn};

use super::AppController;

impl AppController {
    pub(super) async fn handle_connect(
        &self,
        mut conn: DbConnection,
        password: Option<Zeroizing<String>>,
    ) {
        let id = conn.id.clone();
        info!(conn_id = %id, "handling Connect command");

        // ── SSH tunnel setup ──────────────────────────────────────────────────
        if let Some(ssh_cfg) = &conn.ssh {
            let known_hosts_path = self.config_dir.join("known_hosts.toml");

            // Phase 1: probe fingerprint
            let fingerprint = match probe_fingerprint(&ssh_cfg.host, ssh_cfg.port).await {
                Ok(fp) => fp,
                Err(e) => {
                    warn!(conn_id = %id, error = %e, "SSH fingerprint probe failed");
                    let _ = self
                        .tx_event
                        .send(Event::ConnectError(e.localized_message()))
                        .await;
                    return;
                }
            };

            // Phase 2: check known hosts
            let status =
                check_known_host(&ssh_cfg.host, ssh_cfg.port, &fingerprint, &known_hosts_path);
            match status {
                KnownHostStatus::Trusted => {
                    info!(conn_id = %id, "SSH host key trusted");
                }
                KnownHostStatus::Mismatch { expected, actual } => {
                    warn!(conn_id = %id, %expected, %actual, "SSH host key mismatch");
                    let err = wf_db::error::DbError::SshFingerprintMismatch { expected, actual };
                    let _ = self
                        .tx_event
                        .send(Event::ConnectError(err.localized_message()))
                        .await;
                    return;
                }
                KnownHostStatus::Unknown => {
                    // Ask the user to approve
                    let (approval_tx, approval_rx) = oneshot::channel::<bool>();
                    let _ = self
                        .tx_event
                        .send(Event::SshFingerprintRequired {
                            fingerprint: fingerprint.clone(),
                            approval_tx,
                        })
                        .await;

                    let approved = approval_rx.await.unwrap_or(false);
                    if !approved {
                        info!(conn_id = %id, "SSH host key rejected by user");
                        return;
                    }
                    // Persist trust
                    if let Err(e) = save_known_host(
                        &ssh_cfg.host,
                        ssh_cfg.port,
                        &fingerprint,
                        &known_hosts_path,
                    ) {
                        warn!(conn_id = %id, error = %e, "failed to save known host");
                    }
                }
            }

            // Phase 3: decrypt SSH credentials and establish tunnel
            let ssh_password = ssh_cfg
                .ssh_password_encrypted
                .as_deref()
                .and_then(|enc| crypto::decrypt(enc, &self.enc_key).ok());
            let ssh_passphrase = ssh_cfg
                .ssh_passphrase_encrypted
                .as_deref()
                .and_then(|enc| crypto::decrypt(enc, &self.enc_key).ok());

            let tunnel = match connect_tunnel(
                ssh_cfg,
                ssh_password.as_deref().map(|z| z.as_str()),
                ssh_passphrase.as_deref().map(|z| z.as_str()),
                &fingerprint,
            )
            .await
            {
                Ok(t) => t,
                Err(e) => {
                    warn!(conn_id = %id, error = %e, "SSH tunnel connect failed");
                    let _ = self
                        .tx_event
                        .send(Event::ConnectError(e.localized_message()))
                        .await;
                    return;
                }
            };

            let local_port = tunnel.local_port;
            // Store tunnel before mutating conn so lifetime is tied to DbService
            self.db.store_tunnel(id.clone(), tunnel);

            // Route DB connection through the local tunnel endpoint
            conn.host = Some("127.0.0.1".to_string());
            conn.port = Some(local_port);
        }

        match self
            .db
            .connect(&conn, password.as_ref().map(|z| z.as_str()))
            .await
        {
            Ok(()) => {
                let conn_cfg = db_to_config_conn(&conn);
                if let Err(e) = self.repo.upsert(&conn_cfg).await {
                    warn!(conn_id = %id, error = %e, "failed to upsert connection");
                }
                if let Err(e) = self.repo.touch_last_used(&id).await {
                    warn!(conn_id = %id, error = %e, "failed to touch last_used");
                }
                let already_saved = self.state.conn.all().iter().any(|c| c.id == id);
                if already_saved {
                    self.state.conn.update(conn);
                } else {
                    self.state.conn.add(conn);
                }
                self.state.conn.set_active(&id);
                info!(conn_id = %id, "connected successfully");
                let connections = self.repo.all().await.unwrap_or_default();
                let (safe_dml, read_only) = connections
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| (c.safe_dml, c.read_only))
                    .unwrap_or((true, false));
                let _ = self
                    .tx_event
                    .send(Event::Connected {
                        id: id.clone(),
                        connections,
                        safe_dml,
                        read_only,
                    })
                    .await;

                let db = self.db.clone(); // clone required: tokio::spawn needs 'static
                let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
                let cache = self.metadata_cache.clone(); // clone required: tokio::spawn needs 'static
                let fetch_id = id.clone(); // clone required: owned id for async block
                tokio::spawn(async move {
                    match db.fetch_metadata(&fetch_id).await {
                        Ok(meta) => {
                            if let Err(e) = cache.store(&fetch_id, meta.clone()).await {
                                warn!(conn_id = %fetch_id, error = %e, "failed to store metadata");
                            }
                            let _ = tx.send(Event::MetadataLoaded(fetch_id.clone(), meta)).await;
                        }
                        Err(e) => {
                            warn!(conn_id = %fetch_id, error = %e, "metadata fetch failed");
                            let _ = tx.send(Event::MetadataFetchFailed(e.to_string())).await;
                        }
                    }
                });
            }
            Err(e) => {
                warn!(conn_id = %id, error = %e, "connection failed");
                let _ = self
                    .tx_event
                    .send(Event::ConnectError(e.localized_message()))
                    .await;
            }
        }
    }

    pub(super) async fn handle_test_connection(
        &self,
        conn: DbConnection,
        password: Option<Zeroizing<String>>,
    ) {
        let id = conn.id.clone();
        info!(conn_id = %id, "handling TestConnection command");
        match self
            .db
            .connect(&conn, password.as_ref().map(|z| z.as_str()))
            .await
        {
            Ok(()) => {
                self.db.disconnect(&id);
                info!(conn_id = %id, "test connection succeeded");
                let _ = self.tx_event.send(Event::TestConnectionOk).await;
            }
            Err(e) => {
                warn!(conn_id = %id, error = %e, "test connection failed");
                let _ = self
                    .tx_event
                    .send(Event::TestConnectionFailed(e.localized_message()))
                    .await;
            }
        }
    }

    pub(super) async fn handle_disconnect(&self, id: String) {
        info!(conn_id = %id, "handling Disconnect command");
        self.db.disconnect(&id);
        let _ = self.tx_event.send(Event::Disconnected(id)).await;
    }

    pub(super) async fn handle_remove_connection(&self, id: String) {
        info!(conn_id = %id, "handling RemoveConnection command");
        self.db.disconnect(&id);
        self.state.conn.remove(&id);
        if let Err(e) = self.repo.delete(&id).await {
            warn!(conn_id = %id, error = %e, "failed to delete connection from repo");
        }
        let _ = self.tx_event.send(Event::ConnectionRemoved(id)).await;
    }
}

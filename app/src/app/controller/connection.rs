use tracing::{info, warn};
use wf_db::models::DbConnection;
use zeroize::Zeroizing;

use crate::app::{LocalizedMessage, event::Event, session::db_to_config_conn};

use super::AppController;

impl AppController {
    pub(super) async fn handle_connect(
        &self,
        conn: DbConnection,
        password: Option<Zeroizing<String>>,
    ) {
        let id = conn.id.clone();
        info!(conn_id = %id, "handling Connect command");
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

use tracing::info;
use wf_db::models::DbType;

use crate::app::{LocalizedMessage, event::Event};

use super::AppController;

impl AppController {
    /// Handle a `FetchCompletion` command.
    ///
    /// Looks up the active connection, calls [`CompletionService::complete`], and
    /// sends [`Event::CompletionReady`] with the (possibly empty) candidate list.
    /// Silently no-ops when there is no active connection.
    pub(super) async fn handle_fetch_completion(&self, sql: String, cursor_pos: usize) {
        let conn_id = match self.state.conn.active() {
            Some(c) => c.id.clone(),
            None => return,
        };
        let items = self.completion.complete(&conn_id, &sql, cursor_pos).await;
        let _ = self.tx_event.send(Event::CompletionReady(items)).await;
    }

    /// Handle a `FetchDdl` command.
    ///
    /// Fetches the DDL CREATE statement for `name` on `conn_id` and sends
    /// [`Event::DdlLoaded`] or [`Event::DdlFetchFailed`] to the UI.
    pub(super) async fn handle_fetch_ddl(
        &self,
        tab_id: String,
        conn_id: String,
        name: String,
        kind: String,
    ) {
        let db = self.db.clone(); // clone required: tokio::spawn needs 'static
        let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
        tokio::spawn(async move {
            match db.fetch_ddl(&conn_id, &name, &kind).await {
                Ok(ddl) => {
                    let _ = tx.send(Event::DdlLoaded { tab_id, ddl }).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Event::DdlFetchFailed {
                            tab_id,
                            msg: e.localized_message(),
                        })
                        .await;
                }
            }
        });
    }

    /// Handle a `FetchTableData` command.
    ///
    /// Executes `SELECT * FROM "{table_name}" LIMIT {page_size}` and sends
    /// [`Event::TableDataLoaded`] or [`Event::TableDataFailed`] to the UI.
    pub(super) async fn handle_fetch_table_data(
        &self,
        tab_id: String,
        conn_id: String,
        table_name: String,
        page_size: usize,
    ) {
        let quote = self
            .state
            .conn
            .all()
            .iter()
            .find(|c| c.id == conn_id)
            .map(|c| if c.db_type == DbType::MySQL { '`' } else { '"' })
            .unwrap_or('"');
        let escaped = table_name.replace(quote, &format!("{quote}{quote}"));
        let sql = if page_size > 0 {
            format!("SELECT * FROM {quote}{escaped}{quote} LIMIT {page_size}")
        } else {
            format!("SELECT * FROM {quote}{escaped}{quote}")
        };
        let db = self.db.clone(); // clone required: tokio::spawn needs 'static
        let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
        tokio::spawn(async move {
            match db.execute(&conn_id, &sql).await {
                Ok(result) => {
                    let _ = tx.send(Event::TableDataLoaded { tab_id, result }).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Event::TableDataFailed {
                            tab_id,
                            msg: e.localized_message(),
                        })
                        .await;
                }
            }
        });
    }

    /// Handle a `CancelQuery` command.
    ///
    /// Fires the stored [`CancellationToken`] (if any) and immediately sends
    /// [`Event::QueryCancelled`] to the UI so it can reset its loading state.
    pub(super) async fn handle_cancel_query(&self) {
        info!("handling CancelQuery command");
        self.state.query.cancel();
        let _ = self.tx_event.send(Event::QueryCancelled).await;
    }
}

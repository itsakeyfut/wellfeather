use std::time::Duration;

use chrono::Utc;
use rust_i18n::t;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use wf_db::error::DbError;

use crate::app::{LocalizedMessage, event::Event};

use super::AppController;

impl AppController {
    /// Handle a `RunQuery` command.
    ///
    /// Steps:
    /// 1. Cancel any in-flight query via `QueryState::cancel`.
    /// 2. Bail with [`Event::QueryError`] if there is no active connection.
    /// 3. Create a fresh [`CancellationToken`] and register it in `QueryState`.
    /// 4. Send [`Event::QueryStarted`] to the UI immediately.
    /// 5. Spawn a background task that calls [`DbService::execute_with_cancel`]
    ///    and sends [`Event::QueryFinished`] / [`Event::QueryCancelled`] /
    ///    [`Event::QueryError`] when done.
    pub(super) async fn handle_run_query(&self, sql: String) {
        info!("handling RunQuery command");
        self.state.query.cancel();

        let conn_id = match self.state.conn.active() {
            Some(c) => c.id.clone(),
            None => {
                warn!("RunQuery: no active connection");
                let _ = self
                    .tx_event
                    .send(Event::QueryError(
                        t!("error.no_active_connection").to_string(),
                    ))
                    .await;
                return;
            }
        };

        if self.is_read_only_blocked(&conn_id, &sql).await {
            return;
        }

        self.state.query.set_last_sql(sql.clone());

        let token = CancellationToken::new();
        self.state.query.set_cancel_token(token.clone());
        debug!("sending event: QueryStarted");
        let _ = self.tx_event.send(Event::QueryStarted).await;

        let page_size = self.state.ui.page_size();
        let timeout_secs = self.state.ui.query_timeout_secs();
        let sql_to_run = super::apply_limit(&sql, page_size);

        let db = self.db.clone(); // clone required: tokio::spawn needs 'static
        let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
        let history = self.history.clone(); // clone required: tokio::spawn needs 'static
        let sql_hist = sql.clone(); // clone required: history record needs owned sql
        let conn_id_hist = conn_id.clone(); // clone required: history record needs owned id
        tokio::spawn(async move {
            let now = Utc::now().timestamp();
            let result = if timeout_secs > 0 {
                match tokio::time::timeout(
                    Duration::from_secs(timeout_secs),
                    db.execute_with_cancel(&conn_id, &sql_to_run, token.clone()),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_elapsed) => {
                        token.cancel();
                        Err(DbError::Timeout)
                    }
                }
            } else {
                db.execute_with_cancel(&conn_id, &sql_to_run, token).await
            };
            match result {
                Ok(result) => {
                    let exec = wf_db::models::QueryExecution {
                        id: 0,
                        sql: sql_hist,
                        duration_ms: result.execution_time_ms,
                        success: true,
                        error_message: None,
                        timestamp: now,
                        connection_id: conn_id_hist,
                    };
                    if let Err(e) = history.insert(&exec).await {
                        warn!("failed to save history: {e}");
                    }
                    debug!("sending event: QueryFinished");
                    let _ = tx.send(Event::QueryFinished(result)).await;
                }
                Err(DbError::Cancelled) => {
                    debug!("sending event: QueryCancelled");
                    let _ = tx.send(Event::QueryCancelled).await;
                }
                Err(e) => {
                    error!(error = %e, "query execution failed");
                    let exec = wf_db::models::QueryExecution {
                        id: 0,
                        sql: sql_hist,
                        duration_ms: 0,
                        success: false,
                        error_message: Some(e.to_string()),
                        timestamp: now,
                        connection_id: conn_id_hist,
                    };
                    if let Err(he) = history.insert(&exec).await {
                        warn!("failed to save history: {he}");
                    }
                    debug!("sending event: QueryError");
                    let _ = tx.send(Event::QueryError(e.localized_message())).await;
                }
            }
        });
    }

    /// Handle a `RunAll` command.
    ///
    /// Splits the SQL on semicolons and executes each non-empty statement
    /// sequentially using a single cancellation token.  Only the result of the
    /// last statement is surfaced to the UI so the result panel is not spammed.
    pub(super) async fn handle_run_all(&self, sql: String) {
        self.state.query.cancel();

        let conn_id = match self.state.conn.active() {
            Some(c) => c.id.clone(),
            None => {
                warn!("RunAll: no active connection");
                let _ = self
                    .tx_event
                    .send(Event::QueryError(
                        t!("error.no_active_connection").to_string(),
                    ))
                    .await;
                return;
            }
        };

        if self.is_read_only_blocked(&conn_id, &sql).await {
            return;
        }

        let stmts: Vec<String> = wf_query::analyzer::extract_all_statements(&sql)
            .into_iter()
            .map(str::to_owned)
            .collect();

        if stmts.is_empty() {
            return;
        }

        self.state.query.set_last_sql(sql.clone());

        let token = CancellationToken::new();
        self.state.query.set_cancel_token(token.clone());
        let _ = self.tx_event.send(Event::QueryStarted).await;

        let page_size = self.state.ui.page_size();
        let timeout_secs = self.state.ui.query_timeout_secs();
        let db = self.db.clone(); // clone required: tokio::spawn needs 'static
        let tx = self.tx_event.clone(); // clone required: tokio::spawn needs 'static
        let history = self.history.clone(); // clone required: tokio::spawn needs 'static
        let conn_id_hist = conn_id.clone(); // clone required: history record needs owned id

        tokio::spawn(async move {
            let now = Utc::now().timestamp();

            for (i, stmt) in stmts.iter().enumerate() {
                let sql_to_run = super::apply_limit(stmt, page_size);
                let is_last = i == stmts.len() - 1;

                let stmt_result = if timeout_secs > 0 {
                    match tokio::time::timeout(
                        Duration::from_secs(timeout_secs),
                        db.execute_with_cancel(&conn_id, &sql_to_run, token.clone()),
                    )
                    .await
                    {
                        Ok(r) => r,
                        Err(_elapsed) => {
                            token.cancel();
                            Err(DbError::Timeout)
                        }
                    }
                } else {
                    db.execute_with_cancel(&conn_id, &sql_to_run, token.clone())
                        .await
                };

                match stmt_result {
                    Ok(result) => {
                        if is_last {
                            let exec = wf_db::models::QueryExecution {
                                id: 0,
                                sql: stmt.clone(),
                                duration_ms: result.execution_time_ms,
                                success: true,
                                error_message: None,
                                timestamp: now,
                                connection_id: conn_id_hist.clone(),
                            };
                            if let Err(e) = history.insert(&exec).await {
                                warn!("failed to save history: {e}");
                            }
                            debug!("sending event: QueryFinished (run-all last stmt)");
                            let _ = tx.send(Event::QueryFinished(result)).await;
                        }
                    }
                    Err(DbError::Cancelled) => {
                        debug!("sending event: QueryCancelled");
                        let _ = tx.send(Event::QueryCancelled).await;
                        return;
                    }
                    Err(e) => {
                        error!(error = %e, "run-all statement failed");
                        let exec = wf_db::models::QueryExecution {
                            id: 0,
                            sql: stmt.clone(),
                            duration_ms: 0,
                            success: false,
                            error_message: Some(e.to_string()),
                            timestamp: now,
                            connection_id: conn_id_hist.clone(),
                        };
                        if let Err(he) = history.insert(&exec).await {
                            warn!("failed to save history: {he}");
                        }
                        debug!("sending event: QueryError");
                        let _ = tx.send(Event::QueryError(e.localized_message())).await;
                        return;
                    }
                }
            }
        });
    }

    /// Returns `true` if the connection is read-only and `sql` contains a write statement.
    ///
    /// When `true`, sends `Event::QueryError` with a localized "blocked" message so the
    /// caller can return early without executing the query.
    pub(super) async fn is_read_only_blocked(&self, conn_id: &str, sql: &str) -> bool {
        let read_only = self
            .repo
            .find(conn_id)
            .await
            .ok()
            .flatten()
            .map(|c| c.read_only)
            .unwrap_or(false);
        if read_only && wf_query::analyzer::is_write_statement(sql) {
            warn!(conn_id = %conn_id, "RunQuery blocked: connection is read-only");
            let _ = self
                .tx_event
                .send(Event::QueryError(t!("error.read_only_blocked").to_string()))
                .await;
            return true;
        }
        false
    }
}

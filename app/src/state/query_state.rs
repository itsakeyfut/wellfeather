use std::sync::RwLock;

use tokio_util::sync::CancellationToken;
use wf_db::models::QueryResult;

// ---------------------------------------------------------------------------
// Internal data
// ---------------------------------------------------------------------------

#[derive(Default)]
struct QueryData {
    is_loading: bool,
    result: Option<QueryResult>,
    cancel_token: Option<CancellationToken>,
    last_sql: Option<String>,
}

// ---------------------------------------------------------------------------
// QueryState
// ---------------------------------------------------------------------------

/// Thread-safe state for the currently executing (or most recently finished) query.
///
/// All `RwLock` accesses use poison recovery (`unwrap_or_else(|p| p.into_inner())`).
pub struct QueryState {
    data: RwLock<QueryData>,
}

impl Default for QueryState {
    fn default() -> Self {
        Self {
            data: RwLock::new(QueryData::default()),
        }
    }
}

impl QueryState {
    /// Returns `true` while a query is executing.
    pub fn is_loading(&self) -> bool {
        self.data
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .is_loading
    }

    /// Sets the loading flag.  Call with `true` at query start, `false` on finish.
    pub fn set_loading(&self, v: bool) {
        self.data
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .is_loading = v;
    }

    /// Stores the result of the most recently completed query.
    pub fn set_result(&self, r: QueryResult) {
        self.data.write().unwrap_or_else(|p| p.into_inner()).result = Some(r);
    }

    /// Stores a `CancellationToken` so the query can be cancelled via [`cancel`].
    pub fn set_cancel_token(&self, t: CancellationToken) {
        self.data
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .cancel_token = Some(t);
    }

    /// Returns the SQL text of the most recently executed query, if any.
    pub fn last_sql(&self) -> Option<String> {
        self.data
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .last_sql
            .clone()
    }

    /// Stores the SQL text of the most recently executed query.
    pub fn set_last_sql(&self, sql: String) {
        self.data
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .last_sql = Some(sql);
    }

    /// Cancels the running query by calling `cancel()` on the stored token.
    /// No-op if no token is set.
    pub fn cancel(&self) {
        if let Some(token) = self
            .data
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .cancel_token
            .clone()
        {
            token.cancel();
        }
    }
}

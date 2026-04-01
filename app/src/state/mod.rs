#![allow(dead_code)]

pub mod connection_state;
pub mod query_state;
pub mod ui_state;

pub use connection_state::ConnectionState;
pub use query_state::QueryState;
pub use ui_state::UiState;

use std::sync::Arc;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Top-level shared application state.
///
/// Wrapped in `Arc` and cloned across the `AppController`, services, and UI
/// callbacks. Each sub-state holds an internal `RwLock`; callers must only
/// use the provided accessor methods — never access the locks directly.
pub struct AppState {
    pub conn: ConnectionState,
    pub query: QueryState,
    pub ui: UiState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            conn: ConnectionState::default(),
            query: QueryState::default(),
            ui: UiState::default(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience alias: the shared state handle passed throughout the app.
pub type SharedState = Arc<AppState>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use tokio_util::sync::CancellationToken;
    use wf_config::models::Theme;
    use wf_db::models::{DbConnection, DbType, QueryResult};

    use super::*;

    fn make_conn(id: &str, name: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: name.to_string(),
            db_type: DbType::SQLite,
            connection_string: None,
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: Some("test.db".to_string()),
        }
    }

    // -- ConnectionState --

    #[test]
    fn connection_state_should_add_and_list_connections() {
        let state = AppState::new();
        state.conn.add(make_conn("a", "A"));
        state.conn.add(make_conn("b", "B"));
        let all = state.conn.all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "a");
        assert_eq!(all[1].id, "b");
    }

    #[test]
    fn connection_state_should_set_active_and_retrieve() {
        let state = AppState::new();
        state.conn.add(make_conn("x", "X"));
        state.conn.set_active("x");
        let active = state.conn.active().expect("should have active conn");
        assert_eq!(active.id, "x");
    }

    #[test]
    fn connection_state_remove_should_drop_connection() {
        let state = AppState::new();
        state.conn.add(make_conn("del", "Del"));
        state.conn.remove("del");
        assert!(state.conn.all().is_empty());
    }

    // -- QueryState --

    #[test]
    fn query_state_should_track_loading_flag() {
        let state = AppState::new();
        assert!(!state.query.is_loading());
        state.query.set_loading(true);
        assert!(state.query.is_loading());
        state.query.set_loading(false);
        assert!(!state.query.is_loading());
    }

    #[test]
    fn query_state_cancel_should_trigger_cancellation_token() {
        let state = AppState::new();
        let token = CancellationToken::new();
        state.query.set_cancel_token(token.clone());
        assert!(!token.is_cancelled());
        state.query.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn query_state_should_store_result() {
        let state = AppState::new();
        let result = QueryResult {
            columns: vec!["id".to_string()],
            rows: vec![vec![Some("1".to_string())]],
            row_count: 1,
            execution_time_ms: 10,
        };
        state.query.set_result(result);
        // No getter exposed yet; test just confirms no panic.
    }

    // -- UiState --

    #[test]
    fn ui_state_should_get_and_set_theme() {
        let state = AppState::new();
        assert_eq!(state.ui.theme(), Theme::Dark);
        state.ui.set_theme(Theme::Light);
        assert_eq!(state.ui.theme(), Theme::Light);
    }

    #[test]
    fn ui_state_should_get_and_set_page_size() {
        let state = AppState::new();
        assert_eq!(state.ui.page_size(), 500);
        state.ui.set_page_size(1000);
        assert_eq!(state.ui.page_size(), 1000);
    }

    // -- Concurrent access --

    #[test]
    fn app_state_should_be_safe_under_concurrent_access() {
        let shared: SharedState = Arc::new(AppState::new());
        let mut handles = Vec::new();

        for i in 0..8 {
            // clone required: each thread needs owned Arc
            let s = shared.clone();
            let h = thread::spawn(move || {
                let id = format!("conn-{i}");
                s.conn.add(make_conn(&id, &id));
                let _ = s.conn.all();
                s.query.set_loading(true);
                let _ = s.query.is_loading();
                s.query.set_loading(false);
                s.ui.set_page_size(100 * (i + 1));
                let _ = s.ui.page_size();
            });
            handles.push(h);
        }

        for h in handles {
            h.join().expect("thread should not panic");
        }

        assert_eq!(shared.conn.all().len(), 8);
    }
}

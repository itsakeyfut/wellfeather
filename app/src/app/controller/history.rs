use crate::app::event::Event;

use super::AppController;

const HISTORY_PANEL_LIMIT: usize = 100;

impl AppController {
    pub(super) async fn handle_search_history(&self, keyword: String, conn_id: Option<String>) {
        let rows = self
            .history
            .search(&keyword, conn_id.as_deref(), HISTORY_PANEL_LIMIT)
            .await
            .unwrap_or_default();
        let _ = self.tx_event.send(Event::HistoryLoaded(rows)).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::SqlitePool;
    use tempfile::tempdir;
    use wf_completion::cache::MetadataCache;
    use wf_config::{ConnectionRepository, GroupRepository, manager::ConfigManager};
    use wf_db::models::QueryExecution;
    use wf_db::service::DbService;
    use wf_history::service::HistoryService;

    use crate::app::{command::Command, event::Event, session::SessionManager};
    use crate::state::AppState;

    use super::super::AppController;

    fn make_session() -> SessionManager {
        let dir = tempdir().unwrap();
        let path = dir.keep().join("config.toml");
        SessionManager::with_config_manager(ConfigManager::with_path(path))
    }

    async fn make_controller(
        history: HistoryService,
    ) -> (
        AppController,
        tokio::sync::mpsc::Sender<Command>,
        tokio::sync::mpsc::Receiver<Event>,
    ) {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let repo = Arc::new(ConnectionRepository::open_memory().await.unwrap());
        let group_repo = Arc::new(GroupRepository::open_memory().await.unwrap());
        let cache_pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let cache = MetadataCache::new(cache_pool).await.unwrap();
        AppController::new(
            state,
            db,
            make_session(),
            repo,
            group_repo,
            history,
            cache,
            tempdir().unwrap().keep(),
            [0u8; 32],
        )
    }

    fn make_exec(sql: &str, ts: i64) -> QueryExecution {
        QueryExecution {
            id: 0,
            sql: sql.to_string(),
            duration_ms: 10,
            success: true,
            error_message: None,
            timestamp: ts,
            connection_id: "c1".to_string(),
            params_json: None,
        }
    }

    #[tokio::test]
    async fn search_history_should_send_history_loaded_event() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let history = HistoryService::new(pool).await.unwrap();
        history.insert(&make_exec("SELECT 1", 1000)).await.unwrap();
        history.insert(&make_exec("SELECT 2", 2000)).await.unwrap();

        let (controller, tx_cmd, mut rx_event) = make_controller(history).await;
        tx_cmd
            .send(Command::SearchHistory {
                keyword: "SELECT".to_string(),
                conn_id: None,
            })
            .await
            .unwrap();
        drop(tx_cmd);
        controller.run().await;

        let event = rx_event.recv().await.unwrap();
        match event {
            Event::HistoryLoaded(rows) => assert_eq!(rows.len(), 2),
            _ => panic!("expected HistoryLoaded, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn search_history_should_return_empty_vec_when_no_rows() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let history = HistoryService::new(pool).await.unwrap();

        let (controller, tx_cmd, mut rx_event) = make_controller(history).await;
        tx_cmd
            .send(Command::SearchHistory {
                keyword: "SELECT".to_string(),
                conn_id: None,
            })
            .await
            .unwrap();
        drop(tx_cmd);
        controller.run().await;

        let event = rx_event.recv().await.unwrap();
        match event {
            Event::HistoryLoaded(rows) => assert!(rows.is_empty()),
            _ => panic!("expected HistoryLoaded, got {event:?}"),
        }
    }
}

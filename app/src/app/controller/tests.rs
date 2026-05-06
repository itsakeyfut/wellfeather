use std::sync::Arc;

use sqlx::SqlitePool;
use tempfile::tempdir;
use wf_completion::cache::MetadataCache;
use wf_config::{ConnectionRepository, manager::ConfigManager};
use wf_db::{
    models::{DbConnection, DbType},
    service::DbService,
};
use wf_history::service::HistoryService;

use crate::{
    app::{command::Command, event::Event, session::SessionManager},
    state::AppState,
};

use super::{AppController, apply_limit};

/// Build a [`SessionManager`] backed by a temporary directory.
/// `keep()` prevents cleanup so the path stays valid for the test lifetime.
fn test_session() -> SessionManager {
    let dir = tempdir().unwrap();
    let path = dir.keep().join("config.toml");
    SessionManager::with_config_manager(ConfigManager::with_path(path))
}

async fn test_repo() -> Arc<ConnectionRepository> {
    Arc::new(ConnectionRepository::open_memory().await.unwrap())
}

async fn test_history() -> HistoryService {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    HistoryService::new(pool).await.unwrap()
}

async fn test_metadata_cache() -> MetadataCache {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    MetadataCache::new(pool).await.unwrap()
}

fn sqlite_conn(id: &str) -> DbConnection {
    DbConnection {
        id: id.to_string(),
        name: id.to_string(),
        db_type: DbType::SQLite,
        connection_string: Some("sqlite::memory:".to_string()),
        host: None,
        port: None,
        user: None,
        password_encrypted: None,
        database: None,
    }
}

// ── apply_limit ───────────────────────────────────────────────────────────────

#[test]
fn apply_limit_should_append_limit_to_select() {
    assert_eq!(
        apply_limit("SELECT * FROM t", 500),
        "SELECT * FROM t LIMIT 500"
    );
}

#[test]
fn apply_limit_should_strip_trailing_semicolon_before_appending() {
    assert_eq!(
        apply_limit("SELECT * FROM t;", 100),
        "SELECT * FROM t LIMIT 100"
    );
}

#[test]
fn apply_limit_should_not_append_when_limit_already_present() {
    let sql = "SELECT * FROM t LIMIT 10";
    assert_eq!(apply_limit(sql, 500), sql);
}

#[test]
fn apply_limit_should_not_modify_dml_statements() {
    let insert = "INSERT INTO t VALUES (1)";
    assert_eq!(apply_limit(insert, 500), insert);
    let update = "UPDATE t SET x = 1";
    assert_eq!(apply_limit(update, 500), update);
    let delete = "DELETE FROM t";
    assert_eq!(apply_limit(delete, 500), delete);
}

#[test]
fn apply_limit_should_be_case_insensitive() {
    assert_eq!(
        apply_limit("select * from t", 1000),
        "select * from t LIMIT 1000"
    );
    let with_limit = "select * from t limit 5";
    assert_eq!(apply_limit(with_limit, 500), with_limit);
}

#[test]
fn apply_limit_should_not_apply_when_limit_is_zero() {
    assert_eq!(apply_limit("SELECT * FROM t", 0), "SELECT * FROM t");
}

#[test]
fn apply_limit_should_not_apply_to_multi_statement_sql() {
    let sql = "SELECT 1; SELECT 2";
    assert_eq!(apply_limit(sql, 500), sql);
}

// ── TestConnection ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_connection_should_send_ok_and_not_add_to_state() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd
        .send(Command::TestConnection(sqlite_conn("t1"), None))
        .await
        .unwrap();
    drop(tx_cmd);

    controller.run().await;

    let event = rx_event.recv().await.expect("expected event");
    assert!(matches!(event, Event::TestConnectionOk));
    assert!(state.conn.all().is_empty(), "test conn should not be saved");
}

#[tokio::test]
async fn test_connection_should_send_failed_on_invalid_url() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    let bad = DbConnection {
        id: "tbad".to_string(),
        name: "tbad".to_string(),
        db_type: DbType::SQLite,
        connection_string: Some("sqlite:///no/such/path/???invalid".to_string()),
        host: None,
        port: None,
        user: None,
        password_encrypted: None,
        database: None,
    };
    tx_cmd
        .send(Command::TestConnection(bad, None))
        .await
        .unwrap();
    drop(tx_cmd);

    controller.run().await;

    let event = rx_event.recv().await.expect("expected event");
    assert!(matches!(event, Event::TestConnectionFailed(_)));
    assert!(
        state.conn.all().is_empty(),
        "failed test conn should not be saved"
    );
}

#[tokio::test]
async fn connect_should_send_connected_event_on_success() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd
        .send(Command::Connect(sqlite_conn("c1"), None))
        .await
        .unwrap();
    drop(tx_cmd);

    controller.run().await;

    let event = rx_event.recv().await.expect("expected event");
    assert!(matches!(event, Event::Connected { ref id, .. } if id == "c1"));
    assert!(state.conn.active().is_some());
}

#[tokio::test]
async fn connect_should_send_connect_error_on_invalid_url() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    let bad = DbConnection {
        id: "bad".to_string(),
        name: "bad".to_string(),
        db_type: DbType::SQLite,
        connection_string: Some("sqlite:///no/such/path/???invalid".to_string()),
        host: None,
        port: None,
        user: None,
        password_encrypted: None,
        database: None,
    };
    tx_cmd.send(Command::Connect(bad, None)).await.unwrap();
    drop(tx_cmd);

    controller.run().await;

    let event = rx_event.recv().await.expect("expected event");
    assert!(matches!(event, Event::ConnectError(_)));
}

#[tokio::test]
async fn disconnect_should_send_disconnected_event() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd
        .send(Command::Connect(sqlite_conn("c2"), None))
        .await
        .unwrap();
    tx_cmd
        .send(Command::Disconnect("c2".to_string()))
        .await
        .unwrap();
    drop(tx_cmd);

    tokio::spawn(controller.run());

    let e1 = rx_event.recv().await.unwrap();
    assert!(matches!(e1, Event::Connected { .. }));
    // Drain any MetadataLoaded/MetadataFetchFailed from the background fetch.
    let e2 = loop {
        match rx_event.recv().await.unwrap() {
            Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
            e => break e,
        }
    };
    assert!(matches!(e2, Event::Disconnected(ref id) if id == "c2"));
}

// ── RunQuery ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn run_query_should_send_query_started_then_finished() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd
        .send(Command::Connect(sqlite_conn("q1"), None))
        .await
        .unwrap();
    tx_cmd
        .send(Command::RunQuery("SELECT 1 AS n".to_string()))
        .await
        .unwrap();
    drop(tx_cmd);

    tokio::spawn(controller.run());

    let e1 = rx_event.recv().await.unwrap();
    assert!(matches!(e1, Event::Connected { .. }));
    let e2 = loop {
        match rx_event.recv().await.unwrap() {
            Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
            e => break e,
        }
    };
    assert!(matches!(e2, Event::QueryStarted));
    let e3 = loop {
        match rx_event.recv().await.unwrap() {
            Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => continue,
            e => break e,
        }
    };
    assert!(matches!(e3, Event::QueryFinished(_)));
}

#[tokio::test]
async fn run_query_should_send_query_error_when_no_active_connection() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd
        .send(Command::RunQuery("SELECT 1".to_string()))
        .await
        .unwrap();
    drop(tx_cmd);

    controller.run().await;

    let event = rx_event.recv().await.unwrap();
    assert!(matches!(event, Event::QueryError(_)));
}

#[tokio::test]
async fn cancel_query_should_send_query_cancelled_event() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd.send(Command::CancelQuery).await.unwrap();
    drop(tx_cmd);

    controller.run().await;

    let event = rx_event.recv().await.unwrap();
    assert!(matches!(event, Event::QueryCancelled));
}

#[tokio::test]
async fn connect_should_send_metadata_loaded_after_connected() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd
        .send(Command::Connect(sqlite_conn("meta-1"), None))
        .await
        .unwrap();
    drop(tx_cmd);

    tokio::spawn(controller.run());

    let e1 = rx_event.recv().await.unwrap();
    assert!(matches!(e1, Event::Connected { .. }));
    let e2 = rx_event.recv().await.unwrap();
    assert!(
        matches!(
            e2,
            Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_)
        ),
        "expected MetadataLoaded or MetadataFetchFailed, got {e2:?}"
    );
}

#[tokio::test]
async fn connect_twice_should_not_duplicate_in_state() {
    let state = Arc::new(AppState::new());
    let db = DbService::new();
    let (controller, tx_cmd, mut rx_event) = AppController::new(
        state.clone(),
        db,
        test_session(),
        test_repo().await,
        test_history().await,
        test_metadata_cache().await,
    );

    tx_cmd
        .send(Command::Connect(sqlite_conn("c3"), None))
        .await
        .unwrap();
    tx_cmd
        .send(Command::Connect(sqlite_conn("c3"), None))
        .await
        .unwrap();
    drop(tx_cmd);

    tokio::spawn(controller.run());

    let mut connected_count = 0;
    while connected_count < 2 {
        match rx_event.recv().await.unwrap() {
            Event::Connected { .. } => connected_count += 1,
            Event::MetadataLoaded(_, _) | Event::MetadataFetchFailed(_) => {}
            _ => {}
        }
    }

    assert_eq!(state.conn.all().len(), 1, "conn should not be duplicated");
}

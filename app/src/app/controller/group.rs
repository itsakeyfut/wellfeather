use tracing::warn;
use uuid::Uuid;
use wf_config::models::GroupConfig;

use crate::app::{event::Event, group_undo::GroupOp};

use super::AppController;

impl AppController {
    pub(super) async fn handle_create_group(&self, name: String) {
        let id = Uuid::new_v4().to_string();
        let group = GroupConfig {
            id: id.clone(),
            name,
            color: "#6c7086".to_string(),
            expanded: true,
        };
        if let Err(e) = self.group_repo.upsert(&group).await {
            warn!(error = %e, "failed to create group");
            return;
        }
        self.group_undo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(
                GroupOp::DeleteGroup {
                    id: group.id.clone(),
                },
                GroupOp::CreateGroup {
                    group: group.clone(),
                },
            );
        let groups = self.group_repo.all().await.unwrap_or_default();
        let connections = self.repo.all().await.unwrap_or_default();
        let _ = self
            .tx_event
            .send(Event::GroupCreated {
                id,
                groups,
                connections,
            })
            .await;
    }

    pub(super) async fn handle_rename_group(&self, id: String, name: String) {
        let groups = self.group_repo.all().await.unwrap_or_default();
        let old_name = groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        if let Some(mut g) = groups.iter().find(|g| g.id == id).cloned() {
            g.name = name.clone();
            if let Err(e) = self.group_repo.upsert(&g).await {
                warn!(error = %e, "failed to rename group");
                return;
            }
        }
        self.group_undo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(
                GroupOp::RenameGroup {
                    id: id.clone(),
                    name: old_name,
                },
                GroupOp::RenameGroup {
                    id: id.clone(),
                    name: name.clone(),
                },
            );
        let groups = self.group_repo.all().await.unwrap_or_default();
        let connections = self.repo.all().await.unwrap_or_default();
        let _ = self
            .tx_event
            .send(Event::GroupsUpdated {
                groups,
                connections,
            })
            .await;
    }

    pub(super) async fn handle_delete_group(&self, id: String) {
        // Capture state before deletion for undo purposes.
        let all_groups = self.group_repo.all().await.unwrap_or_default();
        let group_config = all_groups.iter().find(|g| g.id == id).cloned();
        let connections = self.repo.all().await.unwrap_or_default();
        let member_conn_ids: Vec<String> = connections
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(&id))
            .map(|c| c.id.clone())
            .collect();

        // Ungroup all connections that belonged to this group.
        for mut cc in connections {
            if cc.group_id.as_deref() == Some(&id) {
                cc.group_id = None;
                if let Err(e) = self.repo.upsert(&cc).await {
                    warn!(conn_id = %cc.id, error = %e, "failed to ungroup connection");
                }
            }
        }
        if let Err(e) = self.group_repo.delete(&id).await {
            warn!(error = %e, "failed to delete group");
            return;
        }
        if let Some(gc) = group_config {
            self.group_undo
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(
                    GroupOp::RestoreGroup {
                        group: gc,
                        member_conn_ids,
                    },
                    GroupOp::DeleteGroup { id: id.clone() },
                );
        }
        let groups = self.group_repo.all().await.unwrap_or_default();
        let connections = self.repo.all().await.unwrap_or_default();
        let _ = self
            .tx_event
            .send(Event::GroupsUpdated {
                groups,
                connections,
            })
            .await;
    }

    pub(super) async fn handle_move_connection_to_group(
        &self,
        conn_id: String,
        group_id: Option<String>,
    ) {
        let connections = self.repo.all().await.unwrap_or_default();
        let old_group_id = connections
            .iter()
            .find(|c| c.id == conn_id)
            .and_then(|c| c.group_id.clone());
        if let Some(mut cc) = connections.iter().find(|c| c.id == conn_id).cloned() {
            cc.group_id = group_id.clone();
            if let Err(e) = self.repo.upsert(&cc).await {
                warn!(conn_id = %conn_id, error = %e, "failed to move connection to group");
                return;
            }
        }
        self.group_undo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(
                GroupOp::MoveConnectionToGroup {
                    conn_id: conn_id.clone(),
                    group_id: old_group_id,
                },
                GroupOp::MoveConnectionToGroup {
                    conn_id: conn_id.clone(),
                    group_id: group_id.clone(),
                },
            );
        let groups = self.group_repo.all().await.unwrap_or_default();
        let connections = self.repo.all().await.unwrap_or_default();
        let _ = self
            .tx_event
            .send(Event::GroupsUpdated {
                groups,
                connections,
            })
            .await;
    }

    pub(super) async fn handle_set_group_color(&self, group_id: String, color: String) {
        let groups = self.group_repo.all().await.unwrap_or_default();
        let old_color = groups
            .iter()
            .find(|g| g.id == group_id)
            .map(|g| g.color.clone())
            .unwrap_or_default();
        if let Some(mut g) = groups.iter().find(|g| g.id == group_id).cloned() {
            g.color = color.clone();
            if let Err(e) = self.group_repo.upsert(&g).await {
                warn!(error = %e, "failed to set group color");
                return;
            }
        }
        self.group_undo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(
                GroupOp::SetGroupColor {
                    id: group_id.clone(),
                    color: old_color,
                },
                GroupOp::SetGroupColor {
                    id: group_id.clone(),
                    color: color.clone(),
                },
            );
        let groups = self.group_repo.all().await.unwrap_or_default();
        let connections = self.repo.all().await.unwrap_or_default();
        let _ = self
            .tx_event
            .send(Event::GroupsUpdated {
                groups,
                connections,
            })
            .await;
    }

    pub(super) async fn handle_set_group_expanded(&self, id: String, expanded: bool) {
        if let Err(e) = self.group_repo.set_expanded(&id, expanded).await {
            warn!(error = %e, "failed to persist group expanded state");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::SqlitePool;
    use tempfile::tempdir;
    use wf_completion::cache::MetadataCache;
    use wf_config::{ConnectionRepository, GroupRepository, manager::ConfigManager};
    use wf_db::service::DbService;
    use wf_history::service::HistoryService;

    use crate::{
        app::{command::Command, event::Event, session::SessionManager},
        state::AppState,
    };

    use super::super::AppController;

    fn test_session() -> SessionManager {
        let dir = tempdir().unwrap();
        let path = dir.keep().join("config.toml");
        SessionManager::with_config_manager(ConfigManager::with_path(path))
    }

    async fn test_repo() -> Arc<ConnectionRepository> {
        Arc::new(ConnectionRepository::open_memory().await.unwrap())
    }

    async fn test_group_repo() -> Arc<GroupRepository> {
        Arc::new(GroupRepository::open_memory().await.unwrap())
    }

    async fn test_history() -> HistoryService {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        HistoryService::new(pool).await.unwrap()
    }

    async fn test_metadata_cache() -> MetadataCache {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MetadataCache::new(pool).await.unwrap()
    }

    fn make_conn(id: &str, group_id: Option<String>) -> wf_config::models::ConnectionConfig {
        wf_config::models::ConnectionConfig {
            id: id.to_string(),
            name: format!("conn-{id}"),
            db_type: wf_config::models::DbTypeName::SQLite,
            connection_string: Some("sqlite::memory:".to_string()),
            host: None,
            port: None,
            user: None,
            password_encrypted: None,
            database: None,
            safe_dml: true,
            read_only: false,
            ssh_enabled: false,
            ssh_host: None,
            ssh_port: None,
            ssh_user: None,
            ssh_auth_method: wf_config::models::SshAuthMethod::Password,
            ssh_password_encrypted: None,
            ssh_key_path: None,
            ssh_passphrase_encrypted: None,
            ssl_enabled: false,
            ssl_mode: wf_config::models::SslMode::Require,
            ssl_ca_cert: None,
            ssl_client_cert: None,
            ssl_client_key: None,
            group_id,
            color: None,
        }
    }

    #[tokio::test]
    async fn create_group_should_send_group_created_event() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state,
            db,
            test_session(),
            test_repo().await,
            test_group_repo().await,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::CreateGroup {
                name: "Production".to_string(),
            })
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        match event {
            Event::GroupCreated { groups, .. } => {
                assert_eq!(groups.len(), 1);
                assert_eq!(groups[0].name, "Production");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn rename_group_should_send_groups_updated_with_new_name() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let group_repo = test_group_repo().await;
        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state,
            db,
            test_session(),
            test_repo().await,
            group_repo.clone(),
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        // Pre-insert a group so rename has something to work with.
        let group = wf_config::models::GroupConfig {
            id: "g1".to_string(),
            name: "Old Name".to_string(),
            color: "#6c7086".to_string(),
            expanded: true,
        };
        group_repo.upsert(&group).await.unwrap();

        tx_cmd
            .send(Command::RenameGroup {
                id: "g1".to_string(),
                name: "New Name".to_string(),
            })
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        match event {
            Event::GroupsUpdated { groups, .. } => {
                let renamed = groups
                    .iter()
                    .find(|g| g.id == "g1")
                    .expect("group not found");
                assert_eq!(renamed.name, "New Name");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn delete_group_should_ungroup_member_connections() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let repo = test_repo().await;
        let group_repo = test_group_repo().await;

        // Pre-insert a group and a connection belonging to it.
        let group = wf_config::models::GroupConfig {
            id: "g1".to_string(),
            name: "ToDelete".to_string(),
            color: "#6c7086".to_string(),
            expanded: true,
        };
        group_repo.upsert(&group).await.unwrap();

        let conn = make_conn("c1", Some("g1".to_string()));
        repo.upsert(&conn).await.unwrap();

        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state,
            db,
            test_session(),
            repo,
            group_repo,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::DeleteGroup {
                id: "g1".to_string(),
            })
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        match event {
            Event::GroupsUpdated {
                groups,
                connections,
            } => {
                assert!(groups.is_empty(), "group should be deleted");
                let c = connections
                    .iter()
                    .find(|c| c.id == "c1")
                    .expect("conn not found");
                assert!(c.group_id.is_none(), "connection should be ungrouped");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn set_group_color_should_emit_groups_updated_with_new_color() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let group_repo = test_group_repo().await;

        let group = wf_config::models::GroupConfig {
            id: "g1".to_string(),
            name: "Prod".to_string(),
            color: "#6c7086".to_string(),
            expanded: true,
        };
        group_repo.upsert(&group).await.unwrap();

        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state,
            db,
            test_session(),
            test_repo().await,
            group_repo,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::SetGroupColor {
                group_id: "g1".to_string(),
                color: "#e74c3c".to_string(),
            })
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        match event {
            Event::GroupsUpdated { groups, .. } => {
                let g = groups
                    .iter()
                    .find(|g| g.id == "g1")
                    .expect("group not found");
                assert_eq!(g.color, "#e74c3c");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn move_connection_to_group_should_update_group_id() {
        let state = Arc::new(AppState::new());
        let db = DbService::new();
        let repo = test_repo().await;
        let group_repo = test_group_repo().await;

        let group = wf_config::models::GroupConfig {
            id: "g1".to_string(),
            name: "Staging".to_string(),
            color: "#6c7086".to_string(),
            expanded: true,
        };
        group_repo.upsert(&group).await.unwrap();

        let conn = make_conn("c1", None);
        repo.upsert(&conn).await.unwrap();

        let (controller, tx_cmd, mut rx_event) = AppController::new(
            state,
            db,
            test_session(),
            repo,
            group_repo,
            test_history().await,
            test_metadata_cache().await,
            tempdir().unwrap().keep(),
            [0u8; 32],
        );

        tx_cmd
            .send(Command::MoveConnectionToGroup {
                conn_id: "c1".to_string(),
                group_id: Some("g1".to_string()),
            })
            .await
            .unwrap();
        drop(tx_cmd);

        controller.run().await;

        let event = rx_event.recv().await.expect("expected event");
        match event {
            Event::GroupsUpdated { connections, .. } => {
                let c = connections
                    .iter()
                    .find(|c| c.id == "c1")
                    .expect("conn not found");
                assert_eq!(c.group_id.as_deref(), Some("g1"));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }
}

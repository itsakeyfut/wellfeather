use tracing::warn;

use crate::app::{event::Event, group_undo::GroupOp};

use super::AppController;

impl AppController {
    pub(super) async fn handle_undo_group_action(&self) {
        let op = self
            .group_undo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_undo();
        if let Some(op) = op {
            self.execute_group_op(op).await;
        }
    }

    pub(super) async fn handle_redo_group_action(&self) {
        let op = self
            .group_undo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_redo();
        if let Some(op) = op {
            self.execute_group_op(op).await;
        }
    }

    async fn execute_group_op(&self, op: GroupOp) {
        match op {
            GroupOp::CreateGroup { group } => {
                if let Err(e) = self.group_repo.upsert(&group).await {
                    warn!(error = %e, "execute_group_op: CreateGroup failed");
                    return;
                }
            }
            GroupOp::DeleteGroup { id } => {
                // Ungroup connections before deleting.
                let connections = self.repo.all().await.unwrap_or_default();
                for mut c in connections {
                    if c.group_id.as_deref() == Some(&id) {
                        c.group_id = None;
                        if let Err(e) = self.repo.upsert(&c).await {
                            warn!(error = %e, "execute_group_op: ungroup connection failed");
                        }
                    }
                }
                if let Err(e) = self.group_repo.delete(&id).await {
                    warn!(error = %e, "execute_group_op: DeleteGroup failed");
                    return;
                }
            }
            GroupOp::RenameGroup { id, name } => {
                let groups = self.group_repo.all().await.unwrap_or_default();
                if let Some(mut g) = groups.into_iter().find(|g| g.id == id) {
                    g.name = name;
                    if let Err(e) = self.group_repo.upsert(&g).await {
                        warn!(error = %e, "execute_group_op: RenameGroup failed");
                        return;
                    }
                }
            }
            GroupOp::SetGroupColor { id, color } => {
                let groups = self.group_repo.all().await.unwrap_or_default();
                if let Some(mut g) = groups.into_iter().find(|g| g.id == id) {
                    g.color = color;
                    if let Err(e) = self.group_repo.upsert(&g).await {
                        warn!(error = %e, "execute_group_op: SetGroupColor failed");
                        return;
                    }
                }
            }
            GroupOp::MoveConnectionToGroup { conn_id, group_id } => {
                let connections = self.repo.all().await.unwrap_or_default();
                if let Some(mut c) = connections.into_iter().find(|c| c.id == conn_id) {
                    c.group_id = group_id;
                    if let Err(e) = self.repo.upsert(&c).await {
                        warn!(error = %e, "execute_group_op: MoveConnectionToGroup failed");
                        return;
                    }
                }
            }
            GroupOp::RestoreGroup {
                group,
                member_conn_ids,
            } => {
                if let Err(e) = self.group_repo.upsert(&group).await {
                    warn!(error = %e, "execute_group_op: RestoreGroup (upsert) failed");
                    return;
                }
                let connections = self.repo.all().await.unwrap_or_default();
                for id in member_conn_ids {
                    if let Some(mut c) = connections.iter().find(|c| c.id == id).cloned() {
                        c.group_id = Some(group.id.clone());
                        if let Err(e) = self.repo.upsert(&c).await {
                            warn!(error = %e, "execute_group_op: RestoreGroup (reassign) failed");
                        }
                    }
                }
            }
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
}

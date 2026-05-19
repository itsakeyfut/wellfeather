use wf_config::models::GroupConfig;

/// A reversible group operation.
/// Every op can serve as both an undo and a redo entry.
#[derive(Clone, Debug)]
pub enum GroupOp {
    CreateGroup {
        group: GroupConfig,
    },
    DeleteGroup {
        id: String,
    },
    RenameGroup {
        id: String,
        name: String,
    },
    SetGroupColor {
        id: String,
        color: String,
    },
    MoveConnectionToGroup {
        conn_id: String,
        group_id: Option<String>,
    },
    /// Recreate a deleted group and reassign its former connections.
    RestoreGroup {
        group: GroupConfig,
        member_conn_ids: Vec<String>,
    },
}

const ACTION_UNDO_MAX: usize = 50;

pub struct GroupUndoStack {
    undo: Vec<(GroupOp, GroupOp)>,
    redo: Vec<(GroupOp, GroupOp)>,
}

impl GroupUndoStack {
    pub fn new() -> Self {
        Self {
            undo: vec![],
            redo: vec![],
        }
    }

    /// Record an action. `reverse` undoes it; `forward` re-applies it.
    /// Branching: discards the redo stack on every new action.
    pub fn push(&mut self, reverse: GroupOp, forward: GroupOp) {
        self.undo.push((reverse, forward));
        if self.undo.len() > ACTION_UNDO_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Returns the op to execute for undo, or `None` if the stack is empty.
    pub fn pop_undo(&mut self) -> Option<GroupOp> {
        let (reverse, forward) = self.undo.pop()?;
        self.redo.push((forward, reverse.clone()));
        if self.redo.len() > ACTION_UNDO_MAX {
            self.redo.remove(0);
        }
        Some(reverse)
    }

    /// Returns the op to execute for redo, or `None` if the stack is empty.
    pub fn pop_redo(&mut self) -> Option<GroupOp> {
        let (forward, reverse) = self.redo.pop()?;
        self.undo.push((reverse, forward.clone()));
        if self.undo.len() > ACTION_UNDO_MAX {
            self.undo.remove(0);
        }
        Some(forward)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_group(id: &str) -> GroupConfig {
        GroupConfig {
            id: id.to_string(),
            name: id.to_string(),
            color: "#6c7086".to_string(),
            expanded: true,
        }
    }

    #[test]
    fn group_undo_stack_should_pop_undo_and_return_reverse_op() {
        let mut s = GroupUndoStack::new();
        s.push(
            GroupOp::DeleteGroup {
                id: "g1".to_string(),
            },
            GroupOp::CreateGroup {
                group: dummy_group("g1"),
            },
        );
        let op = s.pop_undo().expect("should return op");
        assert!(matches!(op, GroupOp::DeleteGroup { ref id } if id == "g1"));
    }

    #[test]
    fn group_undo_stack_should_populate_redo_after_undo() {
        let mut s = GroupUndoStack::new();
        s.push(
            GroupOp::DeleteGroup {
                id: "g1".to_string(),
            },
            GroupOp::CreateGroup {
                group: dummy_group("g1"),
            },
        );
        s.pop_undo();
        assert_eq!(s.undo.len(), 0);
        assert_eq!(s.redo.len(), 1);
    }

    #[test]
    fn group_undo_stack_should_pop_redo_and_return_forward_op() {
        let mut s = GroupUndoStack::new();
        s.push(
            GroupOp::DeleteGroup {
                id: "g1".to_string(),
            },
            GroupOp::CreateGroup {
                group: dummy_group("g1"),
            },
        );
        s.pop_undo();
        let op = s.pop_redo().expect("should return forward op");
        assert!(matches!(op, GroupOp::CreateGroup { .. }));
    }

    #[test]
    fn group_undo_stack_should_repopulate_undo_after_redo() {
        let mut s = GroupUndoStack::new();
        s.push(
            GroupOp::DeleteGroup {
                id: "g1".to_string(),
            },
            GroupOp::CreateGroup {
                group: dummy_group("g1"),
            },
        );
        s.pop_undo();
        s.pop_redo();
        assert_eq!(s.undo.len(), 1);
        assert_eq!(s.redo.len(), 0);
    }

    #[test]
    fn group_undo_stack_should_clear_redo_on_new_push() {
        let mut s = GroupUndoStack::new();
        s.push(
            GroupOp::DeleteGroup {
                id: "g1".to_string(),
            },
            GroupOp::CreateGroup {
                group: dummy_group("g1"),
            },
        );
        s.pop_undo();
        s.push(
            GroupOp::DeleteGroup {
                id: "g2".to_string(),
            },
            GroupOp::CreateGroup {
                group: dummy_group("g2"),
            },
        );
        assert_eq!(s.redo.len(), 0);
    }

    #[test]
    fn group_undo_stack_should_return_none_when_empty() {
        let mut s = GroupUndoStack::new();
        assert!(s.pop_undo().is_none());
        assert!(s.pop_redo().is_none());
    }

    #[test]
    fn group_undo_stack_should_cap_undo_at_max_size() {
        let mut s = GroupUndoStack::new();
        for i in 0..=ACTION_UNDO_MAX {
            s.push(
                GroupOp::DeleteGroup {
                    id: format!("g{i}"),
                },
                GroupOp::CreateGroup {
                    group: dummy_group(&format!("g{i}")),
                },
            );
        }
        assert_eq!(s.undo.len(), ACTION_UNDO_MAX);
    }
}

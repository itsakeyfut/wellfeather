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
    /// Each entry: `(reverse_op, forward_op)` — reverse undoes the action, forward re-applies it.
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

use std::sync::RwLock;

use wf_db::models::DbConnection;

// ---------------------------------------------------------------------------
// Internal data
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ConnectionData {
    connections: Vec<DbConnection>,
    active_id: Option<String>,
}

// ---------------------------------------------------------------------------
// ConnectionState
// ---------------------------------------------------------------------------

/// Thread-safe store of all saved DB connections and the currently active one.
///
/// All `RwLock` accesses use poison recovery (`unwrap_or_else(|p| p.into_inner())`)
/// so that a panic in one thread never permanently wedges other threads.
pub struct ConnectionState {
    data: RwLock<ConnectionData>,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            data: RwLock::new(ConnectionData::default()),
        }
    }
}

impl ConnectionState {
    /// Returns a clone of the currently active connection, if any.
    pub fn active(&self) -> Option<DbConnection> {
        let d = self.data.read().unwrap_or_else(|p| p.into_inner());
        let id = d.active_id.as_deref()?;
        d.connections.iter().find(|c| c.id == id).cloned()
    }

    /// Returns clones of all saved connections.
    pub fn all(&self) -> Vec<DbConnection> {
        self.data
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .connections
            .clone()
    }

    /// Appends a connection to the store.
    pub fn add(&self, conn: DbConnection) {
        self.data
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .connections
            .push(conn);
    }

    /// Removes the connection with the given `id`. No-op if not found.
    pub fn remove(&self, id: &str) {
        self.data
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .connections
            .retain(|c| c.id != id);
    }

    /// Sets the active connection id.
    pub fn set_active(&self, id: &str) {
        self.data
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .active_id = Some(id.to_string());
    }
}

use tracing::warn;

use crate::app::{
    command::ConfigUpdate,
    event::{Event, StateEvent},
};

use super::AppController;

impl AppController {
    /// Handle an `UpdateConfig` command.
    ///
    /// Handles `Theme` and `PageSize` changes: updates shared state so they
    /// survive the current session, then persists the value to `config.toml`.
    pub(super) async fn handle_update_config(&self, update: ConfigUpdate) {
        match update {
            ConfigUpdate::Theme(t) => {
                self.state.ui.set_theme(t.clone());
                if let Err(e) = self.session.save_theme(&t) {
                    warn!(error = %e, "failed to persist theme to config");
                }
                let _ = self
                    .tx_event
                    .send(Event::StateChanged(StateEvent::ThemeChanged(t)))
                    .await;
            }
            ConfigUpdate::PageSize(ps) => {
                let n: u32 = ps.into();
                self.state.ui.set_page_size(n as usize);
                if let Err(e) = self.session.save_page_size(n as usize) {
                    warn!(error = %e, "failed to persist page_size to config");
                }
                let _ = self.tx_event.send(Event::ConfigUpdated).await;
            }
            ConfigUpdate::Language(lang) => {
                if let Err(e) = self.session.save_language(&lang) {
                    warn!(error = %e, "failed to persist language to config");
                }
            }
            ConfigUpdate::ConnectionFlags {
                id,
                safe_dml,
                read_only,
            } => {
                if let Err(e) = self.repo.update_flags(&id, safe_dml, read_only).await {
                    warn!(error = %e, "failed to update connection flags in repo");
                }
                let _ = self
                    .tx_event
                    .send(Event::ConnectionFlagsUpdated {
                        id,
                        safe_dml,
                        read_only,
                    })
                    .await;
            }
            ConfigUpdate::ReduceMotion(value) => {
                if let Err(e) = self.session.save_reduce_motion(value) {
                    warn!(error = %e, "failed to persist reduce_motion to config");
                }
            }
            ConfigUpdate::TabWidth(width) => {
                if let Err(e) = self.session.save_tab_width(width) {
                    warn!(error = %e, "failed to persist tab_width to config");
                }
            }
        }
    }
}

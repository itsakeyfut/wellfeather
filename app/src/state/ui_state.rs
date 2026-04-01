use std::sync::RwLock;

use wf_config::models::Theme;

// ---------------------------------------------------------------------------
// Internal data
// ---------------------------------------------------------------------------

struct UiData {
    theme: Theme,
    page_size: usize,
}

impl Default for UiData {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            page_size: 500,
        }
    }
}

// ---------------------------------------------------------------------------
// UiState
// ---------------------------------------------------------------------------

/// Thread-safe UI preferences (theme and page size).
///
/// Initialised from `Config` on startup; updated when the user changes settings.
/// All `RwLock` accesses use poison recovery (`unwrap_or_else(|p| p.into_inner())`).
pub struct UiState {
    data: RwLock<UiData>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            data: RwLock::new(UiData::default()),
        }
    }
}

impl UiState {
    /// Returns the current theme.
    pub fn theme(&self) -> Theme {
        self.data
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .theme
            .clone()
    }

    /// Updates the theme.
    pub fn set_theme(&self, t: Theme) {
        self.data.write().unwrap_or_else(|p| p.into_inner()).theme = t;
    }

    /// Returns the current result-table page size (row limit).
    pub fn page_size(&self) -> usize {
        self.data
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .page_size
    }

    /// Updates the page size.
    pub fn set_page_size(&self, size: usize) {
        self.data
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .page_size = size;
    }
}

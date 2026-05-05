use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform as CurrentPlatform;
#[cfg(target_os = "macos")]
pub use macos::MacOsPlatform as CurrentPlatform;
#[cfg(target_os = "windows")]
pub use windows::WindowsPlatform as CurrentPlatform;

#[cfg(target_os = "linux")]
pub use linux::enable_dpi_awareness;
#[cfg(target_os = "macos")]
pub use macos::enable_dpi_awareness;
#[cfg(target_os = "windows")]
pub use windows::enable_dpi_awareness;

pub(crate) const APP_NAME: &str = "wellfeather";

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("cannot determine OS application directory")]
    DirectoryNotFound,
}

/// Platform-specific directory resolution and system integration.
///
/// Each OS target has a concrete implementation (`WindowsPlatform`, `MacOsPlatform`,
/// `LinuxPlatform`) aliased to `CurrentPlatform` at compile time.
pub trait Platform {
    /// Returns the OS data directory for the application.
    fn get_data_dir() -> Result<PathBuf, PlatformError>;

    /// Returns the OS config directory for the application.
    fn get_config_dir() -> Result<PathBuf, PlatformError>;

    /// Returns the OS cache directory for the application.
    fn get_cache_dir() -> Result<PathBuf, PlatformError>;

    /// Returns `true` when the OS system-wide dark mode is active.
    ///
    /// Falls back to `false` (light mode) when detection is unavailable or fails.
    fn is_dark_mode_enabled() -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_get_config_dir_should_contain_app_name() {
        let path = CurrentPlatform::get_config_dir().expect("failed to get config directory");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "config dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn platform_get_data_dir_should_contain_app_name() {
        let path = CurrentPlatform::get_data_dir().expect("failed to get data directory");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "data dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn platform_get_cache_dir_should_contain_app_name() {
        let path = CurrentPlatform::get_cache_dir().expect("failed to get cache directory");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "cache dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn platform_is_dark_mode_enabled_should_return_bool() {
        // Smoke test: must not panic; actual value depends on system setting.
        let _ = CurrentPlatform::is_dark_mode_enabled();
    }
}

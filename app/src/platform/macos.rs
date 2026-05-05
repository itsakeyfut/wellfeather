use std::path::PathBuf;
use std::process::Command;

use directories::ProjectDirs;

use super::{APP_NAME, Platform, PlatformError};

pub struct MacOsPlatform;

fn project_dirs() -> Result<ProjectDirs, PlatformError> {
    ProjectDirs::from("", "", APP_NAME).ok_or(PlatformError::DirectoryNotFound)
}

/// No-op on macOS: the OS and Slint handle HiDPI scaling automatically.
pub fn enable_dpi_awareness() {
    tracing::debug!("macOS handles DPI scaling automatically");
}

impl Platform for MacOsPlatform {
    fn get_data_dir() -> Result<PathBuf, PlatformError> {
        Ok(project_dirs()?.data_dir().to_path_buf())
    }

    fn get_config_dir() -> Result<PathBuf, PlatformError> {
        Ok(project_dirs()?.config_dir().to_path_buf())
    }

    fn get_cache_dir() -> Result<PathBuf, PlatformError> {
        Ok(project_dirs()?.cache_dir().to_path_buf())
    }

    fn is_dark_mode_enabled() -> bool {
        // `defaults read -g AppleInterfaceStyle` prints "Dark" in dark mode
        // and exits with a non-zero status in light mode (key is absent).
        let output = Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let is_dark = text.trim().eq_ignore_ascii_case("dark");
                if is_dark {
                    tracing::debug!("macOS dark mode detected");
                }
                is_dark
            }
            Ok(_) => {
                // Key absent → light mode
                tracing::debug!("macOS AppleInterfaceStyle not set, using light mode");
                false
            }
            Err(e) => {
                tracing::debug!(error = ?e, "failed to run 'defaults' command, using light mode");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_platform_get_data_dir_should_contain_app_name() {
        let path = MacOsPlatform::get_data_dir().expect("failed to get data dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "data dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn macos_platform_get_config_dir_should_contain_app_name() {
        let path = MacOsPlatform::get_config_dir().expect("failed to get config dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "config dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn macos_platform_get_cache_dir_should_contain_app_name() {
        let path = MacOsPlatform::get_cache_dir().expect("failed to get cache dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "cache dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn macos_platform_is_dark_mode_enabled_should_return_bool() {
        let _ = MacOsPlatform::is_dark_mode_enabled();
    }
}

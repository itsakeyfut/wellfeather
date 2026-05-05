use std::path::PathBuf;
use std::process::Command;

use directories::ProjectDirs;

use super::{APP_NAME, Platform, PlatformError};

#[cfg(target_os = "windows")]
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

pub struct WindowsPlatform;

fn project_dirs() -> Result<ProjectDirs, PlatformError> {
    ProjectDirs::from("", "", APP_NAME).ok_or(PlatformError::DirectoryNotFound)
}

/// Enables per-monitor DPI awareness V2 on Windows.
///
/// Must be called before any window is created (i.e., before `slint::run_event_loop()`).
/// On Windows versions older than 1703 (Creators Update) this call may fail; a warning is
/// logged and the application continues with system-DPI scaling instead.
#[cfg(target_os = "windows")]
pub fn enable_dpi_awareness() {
    // SAFETY: SetProcessDpiAwarenessContext is safe to call during application initialisation
    // before any window has been created.
    unsafe {
        match SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) {
            Ok(()) => tracing::debug!("per-monitor DPI awareness V2 enabled"),
            Err(e) => tracing::warn!(
                error = ?e,
                "failed to enable per-monitor DPI awareness V2; \
                 display may not scale correctly on high-DPI monitors \
                 (expected on Windows versions older than 1703)"
            ),
        }
    }
}

impl Platform for WindowsPlatform {
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
        let output = Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    if line.contains("AppsUseLightTheme")
                        && line.contains("REG_DWORD")
                        && let Some(hex) = line.split_whitespace().last()
                    {
                        let val = if let Some(h) =
                            hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X"))
                        {
                            u32::from_str_radix(h, 16).unwrap_or(1)
                        } else {
                            hex.parse::<u32>().unwrap_or(1)
                        };
                        return val == 0;
                    }
                }
                tracing::debug!(
                    "failed to parse Windows AppsUseLightTheme registry value, defaulting to light"
                );
                false
            }
            _ => {
                tracing::debug!("failed to query Windows theme registry, defaulting to light");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_platform_get_data_dir_should_contain_app_name() {
        let path = WindowsPlatform::get_data_dir().expect("failed to get data dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "data dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn windows_platform_get_config_dir_should_contain_app_name() {
        let path = WindowsPlatform::get_config_dir().expect("failed to get config dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "config dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn windows_platform_get_cache_dir_should_contain_app_name() {
        let path = WindowsPlatform::get_cache_dir().expect("failed to get cache dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "cache dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn windows_platform_is_dark_mode_enabled_should_return_bool() {
        let _ = WindowsPlatform::is_dark_mode_enabled();
    }
}

use std::path::PathBuf;
use std::process::Command;

use directories::ProjectDirs;

use super::{APP_NAME, Platform, PlatformError};

pub struct LinuxPlatform;

fn project_dirs() -> Result<ProjectDirs, PlatformError> {
    ProjectDirs::from("", "", APP_NAME).ok_or(PlatformError::DirectoryNotFound)
}

/// No-op on Linux: DPI scaling is handled by the desktop environment.
pub fn enable_dpi_awareness() {
    tracing::debug!("Linux DPI scaling is handled by the desktop environment");
}

impl Platform for LinuxPlatform {
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
        // ── GNOME: color-scheme preference (preferred, GTK 4.x / GNOME 42+) ─────
        if let Ok(out) = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
        {
            if out.status.success() {
                let scheme = String::from_utf8_lossy(&out.stdout);
                if scheme.contains("prefer-dark") || scheme.contains("dark") {
                    tracing::debug!("GNOME dark mode detected via color-scheme");
                    return true;
                }
            }
        }

        // ── GNOME: gtk-theme fallback (GTK 3.x) ──────────────────────────────────
        if let Ok(out) = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
            .output()
        {
            if out.status.success() {
                let theme = String::from_utf8_lossy(&out.stdout).to_lowercase();
                if theme.contains("dark") {
                    tracing::debug!("GNOME dark mode detected via gtk-theme");
                    return true;
                }
            }
        }

        // ── KDE Plasma 6 (kreadconfig6) and Plasma 5 (kreadconfig5) ─────────────
        for kreadconfig in ["kreadconfig6", "kreadconfig5"] {
            if let Ok(out) = Command::new(kreadconfig)
                .args([
                    "--group",
                    "General",
                    "--key",
                    "ColorScheme",
                    "--file",
                    "kdeglobals",
                ])
                .output()
            {
                if out.status.success() {
                    let scheme = String::from_utf8_lossy(&out.stdout).to_lowercase();
                    if scheme.contains("dark") || scheme.contains("breezedark") {
                        tracing::debug!("KDE Plasma dark mode detected via {kreadconfig}");
                        return true;
                    }
                }
            }
        }

        // ── GTK_THEME environment variable (universal fallback) ───────────────────
        if let Ok(gtk_theme) = std::env::var("GTK_THEME") {
            if gtk_theme.to_lowercase().contains("dark") {
                tracing::debug!("dark mode detected via GTK_THEME environment variable");
                return true;
            }
        }

        tracing::debug!("no dark theme detected on Linux, defaulting to light");
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_platform_get_data_dir_should_contain_app_name() {
        let path = LinuxPlatform::get_data_dir().expect("failed to get data dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "data dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn linux_platform_get_config_dir_should_contain_app_name() {
        let path = LinuxPlatform::get_config_dir().expect("failed to get config dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "config dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn linux_platform_get_cache_dir_should_contain_app_name() {
        let path = LinuxPlatform::get_cache_dir().expect("failed to get cache dir");
        assert!(
            path.to_string_lossy().contains(APP_NAME),
            "cache dir {path:?} does not contain app name"
        );
    }

    #[test]
    fn linux_platform_is_dark_mode_enabled_should_return_bool() {
        let _ = LinuxPlatform::is_dark_mode_enabled();
    }
}

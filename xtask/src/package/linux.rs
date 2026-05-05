use anyhow::{Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{build_release_binary, prepare_dirs, print_output, workspace_version, PackageConfig};

pub fn build_appimage(config: &PackageConfig) -> Result<()> {
    let _ = config; // sign/notarize deferred for Linux

    println!("{}", "=== Linux AppImage Package ===".bold().blue());

    build_release_binary()?;

    let version = workspace_version()?;
    let temp = prepare_dirs("linux")?;
    let appdir = temp.join("AppDir");

    build_appdir(&appdir, &version)?;

    let output_dir = Path::new("packaging").join("output");
    let appimage_name = format!("wellfeather-{version}-x86_64.AppImage");
    let appimage_path = output_dir.join(&appimage_name);

    run_appimagetool(&appdir, &appimage_path)?;

    print_output(&appimage_path)
}

/// Assemble the AppDir layout required by appimagetool.
fn build_appdir(appdir: &Path, _version: &str) -> Result<()> {
    // Remove stale content from previous runs before populating
    if appdir.exists() {
        std::fs::remove_dir_all(appdir).context("cannot clean AppDir from previous build")?;
    }

    // Required subdirectories
    let app_share = appdir.join("usr").join("share");
    let icon_base = app_share.join("icons").join("hicolor");
    let apps_share = app_share.join("applications");
    std::fs::create_dir_all(&apps_share).context("cannot create AppDir/usr/share/applications")?;

    // Copy release binary
    let bin_src = Path::new("target").join("release").join("wellfeather");
    anyhow::ensure!(
        bin_src.exists(),
        "release binary not found at {}",
        bin_src.display()
    );
    let bin_dest = appdir.join("wellfeather");
    std::fs::copy(&bin_src, &bin_dest).context("cannot copy wellfeather binary")?;
    set_executable(&bin_dest)?;

    // Desktop entry — required at AppDir root AND usr/share/applications/
    let desktop_template = std::fs::read_to_string("packaging/linux/wellfeather.desktop")
        .context("cannot read packaging/linux/wellfeather.desktop")?;
    std::fs::write(appdir.join("wellfeather.desktop"), &desktop_template)
        .context("cannot write AppDir/wellfeather.desktop")?;
    std::fs::write(apps_share.join("wellfeather.desktop"), &desktop_template)
        .context("cannot write AppDir/usr/share/applications/wellfeather.desktop")?;

    // Icon assets
    let icon_src = Path::new("app").join("assets").join("icon.png");
    anyhow::ensure!(
        icon_src.exists(),
        "icon not found at {}.\n\
         Please place a 1024×1024 PNG at app/assets/icon.png before packaging.",
        icon_src.display()
    );
    generate_xdg_icons(&icon_src, appdir, &icon_base)?;

    Ok(())
}

/// Generate XDG icon sizes and place the 256px copy at the AppDir root.
fn generate_xdg_icons(icon_src: &Path, appdir: &Path, icon_base: &Path) -> Result<()> {
    let sizes: &[u32] = &[16, 32, 48, 128, 256, 512];

    let has_magick = Command::new("magick")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    for &size in sizes {
        let dir = icon_base.join(format!("{size}x{size}")).join("apps");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create icon dir {size}x{size}"))?;
        let dest = dir.join("wellfeather.png");

        if has_magick {
            let status = Command::new("magick")
                .args([
                    icon_src.to_str().unwrap(),
                    "-resize",
                    &format!("{size}x{size}"),
                    dest.to_str().unwrap(),
                ])
                .status()
                .context("failed to run magick")?;
            anyhow::ensure!(status.success(), "magick failed for {size}x{size}");
        } else {
            println!(
                "  {} ImageMagick not found; copying source PNG for {size}x{size}",
                "warning:".yellow().bold()
            );
            std::fs::copy(icon_src, &dest)
                .with_context(|| format!("cannot copy icon for {size}x{size}"))?;
        }

        // appimagetool requires a root-level icon; use the 256px version
        if size == 256 {
            std::fs::copy(&dest, appdir.join("wellfeather.png"))
                .context("cannot copy root icon to AppDir")?;
        }
    }
    Ok(())
}

/// Find `appimagetool` by searching PATH entries (`:` separated on Unix).
fn find_appimagetool() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("appimagetool");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn run_appimagetool(appdir: &Path, output: &Path) -> Result<()> {
    let tool = find_appimagetool().context(
        "appimagetool not found in PATH.\n\
         Download it from https://github.com/AppImage/AppImageKit/releases and add it to PATH.\n\
         On CI: see the build-linux job in .github/workflows/release.yml",
    )?;

    // Remove existing output so appimagetool does not prompt to overwrite
    if output.exists() {
        std::fs::remove_file(output).context("cannot remove existing AppImage")?;
    }

    println!("  Running appimagetool...");
    let status = Command::new(&tool)
        .arg(appdir)
        .arg(output)
        .env("ARCH", "x86_64")
        // Run without FUSE when appimagetool itself is an AppImage (common in CI)
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .status()
        .context("failed to run appimagetool")?;
    anyhow::ensure!(status.success(), "appimagetool failed");
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("cannot chmod +x {}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

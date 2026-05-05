use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;
use std::process::Command;

use super::{
    build_release_binary, inject_version, prepare_dirs, print_output, workspace_version,
    PackageConfig,
};

pub fn build_dmg(config: &PackageConfig) -> Result<()> {
    println!("{}", "=== macOS DMG Package ===".bold().blue());

    build_release_binary()?;

    let version = workspace_version()?;
    let temp = prepare_dirs("macos")?;

    // Build .app bundle
    let app_dir = temp.join("wellfeather.app");
    let contents = app_dir.join("Contents");
    let macos_dir = contents.join("MacOS");
    let resources_dir = contents.join("Resources");
    std::fs::create_dir_all(&macos_dir).context("cannot create .app/Contents/MacOS")?;
    std::fs::create_dir_all(&resources_dir).context("cannot create .app/Contents/Resources")?;

    // Inject version into Info.plist template
    let plist_template = std::fs::read_to_string("packaging/macos/Info.plist")
        .context("cannot read packaging/macos/Info.plist")?;
    let plist = inject_version(&plist_template, &version);
    std::fs::write(contents.join("Info.plist"), plist).context("cannot write Info.plist")?;

    // Copy the binary
    let bin_src = Path::new("target").join("release").join("wellfeather");
    anyhow::ensure!(
        bin_src.exists(),
        "release binary not found at {}",
        bin_src.display()
    );
    let bin_dest = macos_dir.join("wellfeather");
    std::fs::copy(&bin_src, &bin_dest).context("cannot copy wellfeather binary")?;
    set_executable(&bin_dest)?;

    // Convert icon.png → AppIcon.icns
    let icon_src = Path::new("app").join("assets").join("icon.png");
    let icns_path = resources_dir.join("AppIcon.icns");
    generate_icns(&icon_src, &icns_path)?;

    // Optional code signing
    if config.sign {
        let cert = config
            .certificate_name
            .as_deref()
            .context("--sign requires --certificate-name on macOS")?;
        codesign_app(&app_dir, cert)?;
    }

    // Create staging dir with .app + /Applications symlink for drag-install UX
    let staging = temp.join("staging");
    std::fs::create_dir_all(&staging).context("cannot create staging dir")?;

    // On macOS, create a symlink; on other OSes (cross-build CI) skip it
    #[cfg(unix)]
    {
        let app_staging = staging.join("wellfeather.app");
        // Copy (not move) so temp stays intact for re-runs
        copy_dir_all(&app_dir, &app_staging)?;
        let applications_link = staging.join("Applications");
        if applications_link.exists() {
            std::fs::remove_file(&applications_link)
                .context("cannot remove old Applications symlink")?;
        }
        std::os::unix::fs::symlink("/Applications", &applications_link)
            .context("cannot create /Applications symlink")?;
    }
    #[cfg(not(unix))]
    {
        let app_staging = staging.join("wellfeather.app");
        copy_dir_all(&app_dir, &app_staging)?;
        println!(
            "  {} /Applications symlink skipped (not on macOS/Unix)",
            "note:".yellow().bold()
        );
    }

    // Build DMG
    let output_dir = Path::new("packaging").join("output");
    let dmg_name = format!("wellfeather-{version}-macos.dmg");
    let dmg_path = output_dir.join(&dmg_name);
    create_dmg(&staging, &dmg_path)?;

    // Optional DMG signing
    if config.sign {
        let cert = config.certificate_name.as_deref().unwrap();
        codesign_dmg(&dmg_path, cert)?;
    }

    // Optional notarization
    if config.notarize {
        notarize(
            &dmg_path,
            config.apple_id.as_deref().unwrap(),
            config.team_id.as_deref().unwrap(),
        )?;
    }

    print_output(&dmg_path)
}

/// Convert app/assets/icon.png to AppIcon.icns via sips + iconutil.
fn generate_icns(icon_src: &Path, icns_dest: &Path) -> Result<()> {
    if !icon_src.exists() {
        anyhow::bail!(
            "icon not found at {}.\n\
             Please place a 1024×1024 PNG at app/assets/icon.png before packaging.",
            icon_src.display()
        );
    }

    let iconset = icns_dest.parent().unwrap().join("AppIcon.iconset");
    std::fs::create_dir_all(&iconset).context("cannot create iconset dir")?;

    // Required sizes for macOS iconset
    let sizes: &[(u32, &str)] = &[
        (16, "icon_16x16.png"),
        (32, "icon_16x16@2x.png"),
        (32, "icon_32x32.png"),
        (64, "icon_32x32@2x.png"),
        (128, "icon_128x128.png"),
        (256, "icon_128x128@2x.png"),
        (256, "icon_256x256.png"),
        (512, "icon_256x256@2x.png"),
        (512, "icon_512x512.png"),
        (1024, "icon_512x512@2x.png"),
    ];

    for (size, name) in sizes {
        let dest = iconset.join(name);
        let status = Command::new("sips")
            .args([
                "-z",
                &size.to_string(),
                &size.to_string(),
                icon_src.to_str().unwrap(),
                "--out",
                dest.to_str().unwrap(),
            ])
            .status()
            .context("failed to run sips (macOS only)")?;
        anyhow::ensure!(status.success(), "sips failed for {name}");
    }

    let status = Command::new("iconutil")
        .args([
            "--convert",
            "icns",
            iconset.to_str().unwrap(),
            "--output",
            icns_dest.to_str().unwrap(),
        ])
        .status()
        .context("failed to run iconutil")?;
    anyhow::ensure!(status.success(), "iconutil failed");
    Ok(())
}

fn codesign_app(app_dir: &Path, cert: &str) -> Result<()> {
    println!("  Signing .app bundle...");
    let status = Command::new("codesign")
        .args([
            "--deep",
            "--force",
            "--options",
            "runtime",
            "--sign",
            cert,
            app_dir.to_str().unwrap(),
        ])
        .status()
        .context("failed to run codesign")?;
    anyhow::ensure!(status.success(), "codesign failed for .app bundle");
    Ok(())
}

fn codesign_dmg(dmg: &Path, cert: &str) -> Result<()> {
    println!("  Signing DMG...");
    let status = Command::new("codesign")
        .args(["--sign", cert, dmg.to_str().unwrap()])
        .status()
        .context("failed to run codesign for DMG")?;
    anyhow::ensure!(status.success(), "codesign failed for DMG");
    Ok(())
}

fn create_dmg(staging: &Path, dmg_path: &Path) -> Result<()> {
    // Remove existing DMG so hdiutil -ov can overwrite
    if dmg_path.exists() {
        std::fs::remove_file(dmg_path).context("cannot remove existing DMG")?;
    }

    println!("  Creating DMG with hdiutil...");
    let status = Command::new("hdiutil")
        .args([
            "create",
            "-size",
            "200m",
            "-fs",
            "HFS+",
            "-volname",
            "wellfeather",
            "-srcfolder",
            staging.to_str().unwrap(),
            "-ov",
            "-format",
            "UDZO",
            dmg_path.to_str().unwrap(),
        ])
        .status()
        .context("failed to run hdiutil")?;
    anyhow::ensure!(status.success(), "hdiutil create failed");
    Ok(())
}

fn notarize(dmg: &Path, apple_id: &str, team_id: &str) -> Result<()> {
    println!("  Submitting to Apple Notary Service...");
    let status = Command::new("xcrun")
        .args([
            "notarytool",
            "submit",
            dmg.to_str().unwrap(),
            "--apple-id",
            apple_id,
            "--team-id",
            team_id,
            "--wait",
        ])
        .status()
        .context("failed to run xcrun notarytool")?;
    anyhow::ensure!(status.success(), "notarization failed");

    println!("  Stapling notarization ticket...");
    let status = Command::new("xcrun")
        .args(["stapler", "staple", dmg.to_str().unwrap()])
        .status()
        .context("failed to run xcrun stapler")?;
    anyhow::ensure!(status.success(), "stapler failed");
    Ok(())
}

/// Recursively copy a directory tree.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("cannot create {}", dst.display()))?;
    for entry in
        std::fs::read_dir(src).with_context(|| format!("cannot read dir {}", src.display()))?
    {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)
                .with_context(|| format!("cannot copy {}", entry.path().display()))?;
        }
    }
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

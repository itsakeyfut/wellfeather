use anyhow::{Context, Result};
use colored::Colorize;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use zip::write::SimpleFileOptions;

use super::{
    build_release_binary, inject_version, prepare_dirs, print_output, workspace_version,
    PackageConfig,
};

pub fn build_zip(config: &PackageConfig) -> Result<()> {
    println!("{}", "=== macOS ZIP Package ===".bold().blue());

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

    let plist_template = std::fs::read_to_string("packaging/macos/Info.plist")
        .context("cannot read packaging/macos/Info.plist")?;
    let plist = inject_version(&plist_template, &version);
    std::fs::write(contents.join("Info.plist"), plist).context("cannot write Info.plist")?;

    let bin_src = Path::new("target").join("release").join("wellfeather");
    anyhow::ensure!(
        bin_src.exists(),
        "release binary not found at {}",
        bin_src.display()
    );
    let bin_dest = macos_dir.join("wellfeather");
    std::fs::copy(&bin_src, &bin_dest).context("cannot copy wellfeather binary")?;
    set_executable(&bin_dest)?;

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

    // Optional notarization
    if config.notarize {
        let dmg_path = temp.join("sign_target.dmg");
        create_dmg_for_notarize(&app_dir, &dmg_path)?;
        notarize(
            &dmg_path,
            config.apple_id.as_deref().unwrap(),
            config.team_id.as_deref().unwrap(),
        )?;
    }

    // Zip the .app bundle
    let output_dir = Path::new("packaging").join("output");
    std::fs::create_dir_all(&output_dir).context("cannot create packaging/output/")?;
    let zip_name = format!("wellfeather-{version}-macos.zip");
    let zip_path = output_dir.join(&zip_name);

    println!("  Creating ZIP archive...");
    let zip_file =
        File::create(&zip_path).with_context(|| format!("cannot create {}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(zip_file);
    add_dir_to_zip(&mut zip, &app_dir, "wellfeather.app")?;
    zip.finish().context("cannot finalise zip")?;

    print_output(&zip_path)
}

/// Recursively add a directory tree into a ZipWriter.
fn add_dir_to_zip(zip: &mut zip::ZipWriter<File>, dir: &Path, prefix: &str) -> Result<()> {
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("cannot read dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let zip_path = format!("{}/{}", prefix, name.to_string_lossy());

        if path.is_dir() {
            add_dir_to_zip(zip, &path, &zip_path)?;
        } else {
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            // Preserve executable bit for the binary
            #[cfg(unix)]
            let options = {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path)?.permissions().mode();
                options.unix_permissions(mode)
            };
            zip.start_file(&zip_path, options)
                .with_context(|| format!("cannot add {zip_path} to zip"))?;
            let mut buf = Vec::new();
            File::open(&path)
                .with_context(|| format!("cannot open {}", path.display()))?
                .read_to_end(&mut buf)
                .with_context(|| format!("cannot read {}", path.display()))?;
            zip.write_all(&buf)
                .with_context(|| format!("cannot write {zip_path} to zip"))?;
        }
    }
    Ok(())
}

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

fn create_dmg_for_notarize(app_dir: &Path, dmg_path: &Path) -> Result<()> {
    if dmg_path.exists() {
        std::fs::remove_file(dmg_path).context("cannot remove existing temp DMG")?;
    }
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
            app_dir.to_str().unwrap(),
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

/// Recursively copy a directory tree (kept for potential future use).
#[allow(dead_code)]
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

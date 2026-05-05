use anyhow::{Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    build_release_binary, inject_version, prepare_dirs, print_output, workspace_version,
    PackageConfig,
};

pub fn build_msix(config: &PackageConfig) -> Result<()> {
    println!("{}", "=== Windows MSIX Package ===".bold().blue());

    build_release_binary()?;

    let version = workspace_version()?;
    let temp = prepare_dirs("windows")?;
    let pkg_dir = temp.join("package");
    let assets_dir = pkg_dir.join("Assets");
    std::fs::create_dir_all(&assets_dir).context("cannot create Assets dir")?;

    // Inject version into AppxManifest.xml template
    let manifest_template = std::fs::read_to_string("packaging/windows/AppxManifest.xml")
        .context("cannot read packaging/windows/AppxManifest.xml")?;
    let manifest = inject_version(&manifest_template, &version);
    std::fs::write(pkg_dir.join("AppxManifest.xml"), manifest)
        .context("cannot write AppxManifest.xml")?;

    // Copy the executable
    let exe_src = Path::new("target").join("release").join("wellfeather.exe");
    anyhow::ensure!(
        exe_src.exists(),
        "release binary not found at {}",
        exe_src.display()
    );
    std::fs::copy(&exe_src, pkg_dir.join("wellfeather.exe"))
        .context("cannot copy wellfeather.exe")?;

    // Generate icon assets (requires app/assets/icon.png)
    let icon_src = Path::new("app").join("assets").join("icon.png");
    generate_icon_assets(&icon_src, &assets_dir)?;

    // Run MakeAppx
    let output_dir = Path::new("packaging").join("output");
    let msix_name = format!("wellfeather-{version}-x86_64-windows.msix");
    let msix_path = output_dir.join(&msix_name);
    run_makeappx(&pkg_dir, &msix_path)?;

    // Optional code signing
    if config.sign {
        let cert = config
            .certificate_path
            .as_deref()
            .context("--sign requires --certificate-path")?;
        let pass = config.certificate_password.as_deref().unwrap_or("");
        run_signtool(&msix_path, cert, pass)?;
    }

    print_output(&msix_path)
}

/// Generate all required MSIX icon assets from a source PNG.
/// Uses ImageMagick `magick` if available; falls back to copying source (wrong size, but functional).
fn generate_icon_assets(icon_src: &Path, assets_dir: &Path) -> Result<()> {
    if !icon_src.exists() {
        anyhow::bail!(
            "icon not found at {}.\n\
             Please place a 1024×1024 PNG at app/assets/icon.png before packaging.",
            icon_src.display()
        );
    }

    // Required MSIX asset sizes: (filename, width, height)
    let assets: &[(&str, u32, u32)] = &[
        ("Square44x44Logo.png", 44, 44),
        ("Square150x150Logo.png", 150, 150),
        ("Wide310x150Logo.png", 310, 150),
        ("StoreLogo.png", 50, 50),
        ("SplashScreen.png", 620, 300),
    ];

    let has_magick = Command::new("magick")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    for (name, w, h) in assets {
        let dest = assets_dir.join(name);
        if has_magick {
            let status = Command::new("magick")
                .args([
                    icon_src.to_str().unwrap(),
                    "-resize",
                    &format!("{w}x{h}!"),
                    dest.to_str().unwrap(),
                ])
                .status()
                .context("failed to run magick")?;
            anyhow::ensure!(status.success(), "magick failed for {name}");
        } else {
            // Fallback: copy source PNG without resizing — MSIX will accept it
            println!(
                "  {} ImageMagick not found; copying source PNG for {name} (wrong size)",
                "warning:".yellow().bold()
            );
            std::fs::copy(icon_src, &dest)
                .with_context(|| format!("cannot copy icon to {name}"))?;
        }
    }
    Ok(())
}

/// Find MakeAppx.exe: PATH first, then Windows SDK directories.
fn find_makeappx() -> Option<PathBuf> {
    if which_tool("MakeAppx").is_some() {
        return Some(PathBuf::from("MakeAppx"));
    }

    // Search Windows SDK bin directories
    let sdk_base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = std::fs::read_dir(sdk_base) {
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        // Sort descending to prefer newest SDK
        versions.sort_by(|a, b| b.cmp(a));
        for ver in versions {
            let candidate = ver.join("x64").join("MakeAppx.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Find SignTool.exe the same way as MakeAppx.
fn find_signtool() -> Option<PathBuf> {
    if which_tool("SignTool").is_some() {
        return Some(PathBuf::from("SignTool"));
    }
    let sdk_base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = std::fs::read_dir(sdk_base) {
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        versions.sort_by(|a, b| b.cmp(a));
        for ver in versions {
            let candidate = ver.join("x64").join("signtool.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn which_tool(name: &str) -> Option<()> {
    Command::new("where")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| ())
}

fn run_makeappx(pkg_dir: &Path, output: &Path) -> Result<()> {
    let tool = find_makeappx().context(
        "MakeAppx.exe not found.\n\
         Install the Windows SDK (https://developer.microsoft.com/en-us/windows/downloads/windows-sdk/)\
         or add MakeAppx.exe to PATH.",
    )?;

    println!("  Running MakeAppx...");
    let status = Command::new(&tool)
        .args([
            "pack",
            "/d",
            pkg_dir.to_str().unwrap(),
            "/p",
            output.to_str().unwrap(),
            "/nv",
            "/o",
        ])
        .status()
        .context("failed to run MakeAppx")?;
    anyhow::ensure!(status.success(), "MakeAppx failed");
    Ok(())
}

fn run_signtool(msix: &Path, cert_path: &str, cert_pass: &str) -> Result<()> {
    let tool = find_signtool().context(
        "signtool.exe not found.\n\
         Install the Windows SDK or add signtool.exe to PATH.",
    )?;

    println!("  Signing MSIX with SignTool...");
    let mut args = vec!["sign", "/fd", "SHA256", "/f", cert_path];
    let pass_arg;
    if !cert_pass.is_empty() {
        pass_arg = cert_pass.to_owned();
        args.extend(["/p", &pass_arg]);
    }
    let msix_str = msix.to_str().unwrap().to_owned();
    args.push(&msix_str);

    let status = Command::new(&tool)
        .args(&args)
        .status()
        .context("failed to run SignTool")?;
    anyhow::ensure!(status.success(), "SignTool signing failed");
    Ok(())
}

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub mod linux;
pub mod macos;
pub mod windows;

pub struct PackageConfig {
    pub platform: String,
    pub sign: bool,
    pub certificate_path: Option<String>,
    pub certificate_password: Option<String>,
    pub certificate_name: Option<String>,
    pub notarize: bool,
    pub apple_id: Option<String>,
    pub team_id: Option<String>,
}

impl PackageConfig {
    pub fn resolved_platform(&self) -> &str {
        match self.platform.as_str() {
            "current" => {
                if cfg!(target_os = "windows") {
                    "windows"
                } else if cfg!(target_os = "macos") {
                    "macos"
                } else if cfg!(target_os = "linux") {
                    "linux"
                } else {
                    "all"
                }
            }
            other => other,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.notarize {
            anyhow::ensure!(self.sign, "--notarize requires --sign");
            anyhow::ensure!(self.apple_id.is_some(), "--notarize requires --apple-id");
            anyhow::ensure!(self.team_id.is_some(), "--notarize requires --team-id");
        }
        Ok(())
    }
}

pub fn run_package(config: &PackageConfig) -> Result<()> {
    config.validate()?;

    let platform = config.resolved_platform();
    println!(
        "{} {}",
        "Building distribution package for:".bold().blue(),
        platform.bold()
    );

    match platform {
        "windows" => windows::build_msix(config)?,
        "macos" => macos::build_dmg(config)?,
        "linux" => linux::build_appimage(config)?,
        "all" => {
            windows::build_msix(config)?;
            macos::build_dmg(config)?;
            linux::build_appimage(config)?;
        }
        other => anyhow::bail!("Unknown platform: {other}. Use: windows, macos, linux, or all"),
    }

    Ok(())
}

/// Read workspace version from root Cargo.toml [workspace.package].version.
pub fn workspace_version() -> Result<String> {
    let src = std::fs::read_to_string("Cargo.toml").context("cannot read workspace Cargo.toml")?;
    let doc: toml::Value = toml::from_str(&src).context("cannot parse workspace Cargo.toml")?;
    let version = doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .context("[workspace.package].version not found in Cargo.toml")?;
    Ok(version.to_owned())
}

/// Inject {VERSION} and other placeholders into a template string.
pub fn inject_version(template: &str, version: &str) -> String {
    template.replace("{VERSION}", version)
}

/// Build the release binary with cargo.
pub fn build_release_binary() -> Result<()> {
    println!("{}", "  Building release binary...".dimmed());
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to run cargo build --release")?;
    anyhow::ensure!(status.success(), "cargo build --release failed");
    Ok(())
}

/// Ensure `packaging/output/` and `packaging/temp/<sub>/` directories exist.
pub fn prepare_dirs(sub: &str) -> Result<PathBuf> {
    let output = Path::new("packaging").join("output");
    let temp = Path::new("packaging").join("temp").join(sub);
    std::fs::create_dir_all(&output).context("cannot create packaging/output/")?;
    std::fs::create_dir_all(&temp).context("cannot create packaging/temp/")?;
    Ok(temp)
}

/// Print a summary line with file size.
pub fn print_output(path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("cannot stat output file {}", path.display()))?;
    let mb = meta.len() as f64 / 1_048_576.0;
    println!(
        "\n{} {} ({:.1} MB)",
        "✓ Package ready:".green().bold(),
        path.display(),
        mb
    );
    Ok(())
}

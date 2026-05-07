use anyhow::{Context, Result};
use colored::Colorize;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use zip::write::SimpleFileOptions;

use super::{build_release_binary, prepare_dirs, print_output, workspace_version, PackageConfig};

pub fn build_zip(config: &PackageConfig) -> Result<()> {
    println!("{}", "=== Windows ZIP Package ===".bold().blue());

    build_release_binary()?;

    let version = workspace_version()?;
    let _temp = prepare_dirs("windows")?;

    let exe_src = Path::new("target").join("release").join("wellfeather.exe");
    anyhow::ensure!(
        exe_src.exists(),
        "release binary not found at {}",
        exe_src.display()
    );

    // Optional PE signing before packaging
    if config.sign {
        let cert = config
            .certificate_path
            .as_deref()
            .context("--sign requires --certificate-path")?;
        let pass = config.certificate_password.as_deref().unwrap_or("");
        run_signtool(&exe_src, cert, pass)?;
    }

    let output_dir = Path::new("packaging").join("output");
    std::fs::create_dir_all(&output_dir).context("cannot create packaging/output/")?;
    let zip_name = format!("wellfeather-{version}-x86_64-windows.zip");
    let zip_path = output_dir.join(&zip_name);

    println!("  Creating ZIP archive...");
    let zip_file =
        File::create(&zip_path).with_context(|| format!("cannot create {}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("wellfeather.exe", options)
        .context("cannot add wellfeather.exe to zip")?;
    let mut buf = Vec::new();
    File::open(&exe_src)
        .context("cannot open wellfeather.exe")?
        .read_to_end(&mut buf)
        .context("cannot read wellfeather.exe")?;
    zip.write_all(&buf).context("cannot write to zip")?;
    zip.finish().context("cannot finalise zip")?;

    print_output(&zip_path)
}

fn find_signtool() -> Option<std::path::PathBuf> {
    if Command::new("where")
        .arg("signtool")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(std::path::PathBuf::from("signtool"));
    }
    let sdk_base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = std::fs::read_dir(sdk_base) {
        let mut versions: Vec<_> = entries
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

fn run_signtool(exe: &Path, cert_path: &str, cert_pass: &str) -> Result<()> {
    let tool = find_signtool().context(
        "signtool.exe not found.\n\
         Install the Windows SDK or add signtool.exe to PATH.",
    )?;
    println!("  Signing executable with SignTool...");
    let mut args = vec!["sign", "/fd", "SHA256", "/f", cert_path];
    let pass_arg;
    if !cert_pass.is_empty() {
        pass_arg = cert_pass.to_owned();
        args.extend(["/p", &pass_arg]);
    }
    let exe_str = exe.to_str().unwrap().to_owned();
    args.push(&exe_str);
    let status = Command::new(&tool)
        .args(&args)
        .status()
        .context("failed to run SignTool")?;
    anyhow::ensure!(status.success(), "SignTool signing failed");
    Ok(())
}

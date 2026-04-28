fn main() {
    println!("cargo:rerun-if-changed=src/ui/app.slint");

    // Watch all .slint files directly under src/ui/ (e.g. globals.slint)
    if let Ok(entries) = std::fs::read_dir("src/ui") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "slint") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    // Watch all .slint files under src/ui/components/
    if let Ok(entries) = std::fs::read_dir("src/ui/components") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "slint") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    // Watch .po translation files
    println!("cargo:rerun-if-changed=lang");

    // Watch all .slint files under src/ui/components/dialogs/
    if let Ok(entries) = std::fs::read_dir("src/ui/components/dialogs") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "slint") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations(std::path::Path::new("lang"));
    slint_build::compile_with_config("src/ui/app.slint", config)
        .expect("Failed to compile Slint UI");
}

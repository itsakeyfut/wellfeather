fn main() {
    println!("cargo:rerun-if-changed=src/ui/app.slint");

    if let Ok(entries) = std::fs::read_dir("src/ui/components") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "slint") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    slint_build::compile("src/ui/app.slint").expect("Failed to compile Slint UI");
}

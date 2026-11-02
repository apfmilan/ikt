fn main() {
    // Rebuild when the configured origin changes.
    println!("cargo:rerun-if-env-changed=TC_ORIGIN");
    tauri_build::build()
}

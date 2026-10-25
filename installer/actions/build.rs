fn main() {
    let config =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../orbit-gui/tauri.conf.json");
    println!("cargo:rerun-if-changed={}", config.display());
    let bytes = std::fs::read(config).expect("GUI package metadata must be readable");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("valid Tauri metadata");
    let identifier = json["identifier"]
        .as_str()
        .expect("GUI application identifier");
    assert!(!identifier.contains(['/', '\\', '\n', '\r']));
    println!("cargo:rustc-env=ORBIT_GUI_IDENTIFIER={identifier}");
}

fn main() {
    // The localization proc macro reads these files during compilation, but
    // Cargo cannot infer that external input on incremental builds.
    println!("cargo:rerun-if-changed=locales");
}

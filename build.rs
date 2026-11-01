fn main() {
    println!("cargo:rerun-if-env-changed=CR_VERSION");
    let version = std::env::var("CR_VERSION")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=CR_BUILD_VERSION={version}");
}

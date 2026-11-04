// The Release workflow sets the app identity. Local builds use the Cargo version and `local`.
fn main() {
    for (name, fallback) in [
        ("SNIPLET_VERSION", env!("CARGO_PKG_VERSION")),
        ("SNIPLET_BUILD_SHA", "local"),
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        let value = std::env::var(name).unwrap_or_else(|_| fallback.into());
        println!("cargo:rustc-env={name}={value}");
    }
}

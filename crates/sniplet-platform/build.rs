fn main() {
    for name in ["SNIPLET_BUILD_SHA", "SNIPLET_BUILD_NUMBER"] {
        println!("cargo:rerun-if-env-changed={name}");
        let value = std::env::var(name).unwrap_or_else(|_| {
            if name == "SNIPLET_BUILD_NUMBER" {
                "0"
            } else {
                "local"
            }
            .into()
        });
        println!("cargo:rustc-env={name}={value}");
    }
}

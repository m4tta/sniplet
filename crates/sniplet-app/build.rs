fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Unoptimized GPUI element trees can exceed Windows' 1 MiB default.
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
}

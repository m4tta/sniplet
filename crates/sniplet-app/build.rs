fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=resources/sniplet.rc");
        println!("cargo:rerun-if-changed=../../assets/icons/sniplet.ico");
        embed_resource::compile("resources/sniplet.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Could not compile the Windows app icon");
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Unoptimized GPUI element trees can exceed Windows' 1 MiB default.
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
}

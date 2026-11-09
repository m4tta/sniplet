fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // The .rc file stays in the crate folder: rc.exe on Windows and llvm-rc on other
        // hosts resolve its relative icon path from different folders, and here they match.
        println!("cargo:rerun-if-changed=sniplet.rc");
        println!("cargo:rerun-if-changed=../../assets/icons/sniplet.ico");
        embed_resource::compile("sniplet.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Could not compile the Windows app icon");
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Unoptimized GPUI element trees can exceed Windows' 1 MiB default.
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
}

use std::env;

fn main() {
    let target = env::var("TARGET").unwrap_or_else(|e| panic!("{}", e));
    println!("cargo:rustc-check-cfg=cfg(lr_macos_legacy)");
    println!("cargo:rerun-if-env-changed=LR_MACOS_LEGACY");
    let legacy = target == "x86_64-apple-darwin"
        && env::var("LR_MACOS_LEGACY").as_deref() == Ok("1");
    if legacy {
        println!("cargo:rustc-cfg=lr_macos_legacy");
    }

    if !legacy && (target.contains("darwin") || target.contains("ios")) {
        println!("cargo:rustc-link-lib=framework=MetalKit");
    }
}

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=DOWNLOAD_CORE_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}

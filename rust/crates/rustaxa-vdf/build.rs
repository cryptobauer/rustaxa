use std::env;
use std::path::PathBuf;

fn main() {
    let lib_dir = env::var_os("TARAXA_VRF_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/build/deps/taraxa-vrf/lib"));

    println!("cargo:rerun-if-env-changed=TARAXA_VRF_LIB_DIR");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
}

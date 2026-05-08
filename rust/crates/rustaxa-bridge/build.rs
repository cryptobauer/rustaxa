fn main() {
    cxx_build::bridges(["src/ffi.rs"])
        .std("c++20")
        .flag("-fvisibility=hidden")
        .compile("rustaxa-bridge");

    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=src/dag.rs");
    println!("cargo:rerun-if-changed=src/final_chain.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/pbft_chain.rs");
    println!("cargo:rerun-if-changed=src/vdf.rs");
    println!("cargo:rerun-if-changed=src/storage.rs");
}

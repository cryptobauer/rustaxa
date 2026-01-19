fn main() {
    cxx_build::bridges(["src/vdf.rs", "src/storage.rs"])
        .std("c++17")
        .compile("rustaxa-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/vdf.rs");
    println!("cargo:rerun-if-changed=src/storage.rs");
}

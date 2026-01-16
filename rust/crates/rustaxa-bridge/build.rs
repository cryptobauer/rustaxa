fn main() {
    cxx_build::bridges(["src/vdf.rs"])
        .std("c++17")
        .compile("rustaxa-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/vdf.rs");
}

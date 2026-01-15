fn main() {
    cxx_build::bridge("src/bindings.rs")
        .std("c++17")
        .compile("rustaxa-vdf");

    println!("cargo:rerun-if-changed=src/bindings.rs");
}

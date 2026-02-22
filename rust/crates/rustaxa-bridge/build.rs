fn main() {
    cxx_build::bridges(["src/vdf.rs", "src/storage.rs"])
        .std("c++20")
        .flag("-fvisibility=hidden")
        .compile("rustaxa-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/vdf.rs");
    println!("cargo:rerun-if-changed=src/storage.rs");
}

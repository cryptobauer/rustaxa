fn main() {
    cxx_build::bridge("src/lib.rs")
        .file("native/selected_production_config.cpp")
        .include("include")
        .std("c++20")
        .flag("-fvisibility=hidden")
        .compile("rustaxa_native_config");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=include/rustaxa/selected_production_config.hpp");
    println!("cargo:rerun-if-changed=native/selected_production_config.cpp");
}

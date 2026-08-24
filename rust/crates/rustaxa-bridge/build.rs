fn main() {
    cxx_build::bridge("src/ffi.rs")
        .include("../../../libraries/core_libs/consensus/include")
        .std("c++20")
        .flag("-fvisibility=hidden")
        .compile("rustaxa-bridge");

    // Keep concrete application-host callbacks in a separate native archive.
    // Leaf users of the Rust staticlib must not select this CXX object merely
    // because it also contains generic CXX runtime support symbols.
    cxx_build::bridge("src/application_host_ffi.rs")
        .include("../../../libraries/core_libs/consensus/include")
        .std("c++20")
        .flag("-fvisibility=hidden")
        .cargo_metadata(false)
        .compile("rustaxa_application_host_bridge");

    if let Ok(destination) = std::env::var("RUSTAXA_APPLICATION_HOST_BRIDGE_OUT") {
        let source =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
                .join("librustaxa_application_host_bridge.a");
        std::fs::copy(&source, &destination).unwrap_or_else(|error| {
            panic!(
                "failed to copy application-host bridge from {} to {}: {error}",
                source.display(),
                destination
            )
        });
    }

    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=src/application_host_ffi.rs");
    println!("cargo:rerun-if-env-changed=RUSTAXA_APPLICATION_HOST_BRIDGE_OUT");
    println!(
        "cargo:rerun-if-changed=../../../libraries/core_libs/consensus/include/consensus/consensus_host_ports.hpp"
    );
    println!("cargo:rerun-if-changed=src/dag.rs");
    println!("cargo:rerun-if-changed=src/dag_transaction_service.rs");
    println!("cargo:rerun-if-changed=src/consensus_host_ports.rs");
    println!("cargo:rerun-if-changed=src/consensus_bootstrap.rs");
    println!("cargo:rerun-if-changed=src/final_chain.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/network.rs");
    println!("cargo:rerun-if-changed=src/network_slashing.rs");
    println!("cargo:rerun-if-changed=src/transaction_manager.rs");
    println!("cargo:rerun-if-changed=src/vdf.rs");
    println!("cargo:rerun-if-changed=src/storage.rs");
    println!("cargo:rerun-if-changed=src/pillar_votes.rs");
}

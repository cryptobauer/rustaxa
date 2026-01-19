# Rustaxa

Rustaxa is a port of the Taraxa node software written in Rust. Taraxa is a blockchain and this project is the validator node software.

- Most code is still in C++ and is being ported to Rust incrementally in this project.
- The Rust code is organized in a separate directory and is being developed alongside the existing C++.
- The goal is to eventually replace the C++ codebase with Rust.
- Whenever we replace any C++ code with Rust it always has to be backwards compatible with the C++ implementation to ensure the node can run against other C++ validator nodes.
- We are using shims to inject the Rust code activated with the define `RUSTAXA_ENABLED`.
- Any shim should be as small as possible so that merging C++ upstream changes doesn't lead to conflicts.
- The Rust code is organized in crates.
- Never change existing C++ code without asking me before.
- Only add comments when the code isn't self-explanatory.
- Whenever possible functions and structs should be ordered in reading direction, means that the caller should be above the callee and so on.

## Tools

- Always use `rg` ripgrep instead of `grep` for faster searches.
- Always use `fdfind` instead of `find` for faster file finding.

## Layout

All C++ code is located in the `libraries`, `programs`, and `submodules` directories. The new Rust code is in the `rust` directory.

## Building

- All code can be build with `make build`, which will build both C++ and Rust code.
- The Rust code can be built separately with `cargo build` from the `rust` directory.

## Rust Specifics

- We are using `tracing` for logging.
- We are using `anyhow` for error handling.

## Testing

- The C++ tests can be run by running the individual test executables in the `build/bin` directory.
- The Rust tests can be run with `cargo test` from the `rust` directory.

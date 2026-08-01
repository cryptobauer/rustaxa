#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_ROOT="${1:-/tmp/rustaxa-storage-conformance}"
CPP_BUILD_DIR="${BUILD_ROOT}/cpp"
RUST_BUILD_DIR="${BUILD_ROOT}/rust"
OUT_DIR="${BUILD_ROOT}/out"

mkdir -p "${CPP_BUILD_DIR}" "${RUST_BUILD_DIR}" "${OUT_DIR}"

detect_jobs() {
  if command -v nproc >/dev/null 2>&1; then
    nproc
    return
  fi
  if command -v sysctl >/dev/null 2>&1; then
    sysctl -n hw.logicalcpu
    return
  fi
  echo 4
}

ensure_toolchain() {
  local build_dir="$1"
  if [[ -f "${build_dir}/conan_toolchain.cmake" ]]; then
    return
  fi

  conan install "${ROOT_DIR}" \
    -s "build_type=Release" \
    -s "&:build_type=Release" \
    --profile:host=clang \
    --profile:build=clang \
    --build=missing \
    --output-folder="${build_dir}"
}

configure_and_build() {
  local build_dir="$1"
  shift

  ensure_toolchain "${build_dir}"
  cmake -S "${ROOT_DIR}" -B "${build_dir}" -DCMAKE_BUILD_TYPE=Release "$@"
  cmake --build "${build_dir}" --target storage_conformance_runner -j"$(detect_jobs)"
}

echo "[1/4] Configuring and building C++ reference mode..."
configure_and_build "${CPP_BUILD_DIR}" \
  -DRUSTAXA_ENABLE=OFF

echo "[2/4] Configuring and building Rust storage shim mode..."
configure_and_build "${RUST_BUILD_DIR}" \
  -DRUSTAXA_ENABLE=ON

echo "[3/4] Running conformance scenarios..."
"${CPP_BUILD_DIR}/bin/storage_conformance_runner" --output "${OUT_DIR}/cpp.json" >/dev/null
"${RUST_BUILD_DIR}/bin/storage_conformance_runner" --output "${OUT_DIR}/rust.json" >/dev/null

echo "[4/4] Diffing transcripts..."
python3 - "${OUT_DIR}/cpp.json" "${OUT_DIR}/rust.json" <<'PY'
import json
import sys
from pathlib import Path

cpp_path = Path(sys.argv[1])
rust_path = Path(sys.argv[2])

def load_entries(path: Path):
    data = json.loads(path.read_text())
    entries = data.get("entries", [])
    return sorted((entry["key"], entry["value"]) for entry in entries)

cpp_entries = load_entries(cpp_path)
rust_entries = load_entries(rust_path)

if cpp_entries != rust_entries:
    cpp_map = dict(cpp_entries)
    rust_map = dict(rust_entries)
    keys = sorted(set(cpp_map) | set(rust_map))
    print("Storage conformance mismatch detected:\n")
    for key in keys:
        c = cpp_map.get(key, "<missing>")
        r = rust_map.get(key, "<missing>")
        if c != r:
            print(f"- {key}\n  cpp : {c}\n  rust: {r}")
    sys.exit(1)

print("Storage conformance transcripts match.")
PY

echo "Outputs written to: ${OUT_DIR}"

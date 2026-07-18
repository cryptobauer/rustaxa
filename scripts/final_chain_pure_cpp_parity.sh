#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
BUILD_ROOT_INPUT="${1:-/tmp/rustaxa-final-chain-pure-cpp}"

case "${BUILD_ROOT_INPUT}" in
  /*) ;;
  *)
    echo "Error: FinalChain pure-C++ build root must be an absolute path: ${BUILD_ROOT_INPUT}" >&2
    exit 2
    ;;
esac

if [[ "${BUILD_ROOT_INPUT}" == "/" ]]; then
  echo "Error: refusing unsafe FinalChain pure-C++ build root: /" >&2
  exit 2
fi

case "${BUILD_ROOT_INPUT%/}/" in
  "${ROOT_DIR}/"*)
    echo "Error: FinalChain pure-C++ build root must be outside the repository: ${BUILD_ROOT_INPUT}" >&2
    exit 2
    ;;
esac

mkdir -p "${BUILD_ROOT_INPUT}"
BUILD_ROOT="$(cd "${BUILD_ROOT_INPUT}" && pwd -P)"
case "${BUILD_ROOT}/" in
  "${ROOT_DIR}/"*)
    echo "Error: FinalChain pure-C++ build root must be outside the repository: ${BUILD_ROOT}" >&2
    exit 2
    ;;
esac

CPP_BUILD_DIR="${BUILD_ROOT}/cpp"
CONAN_TOOLCHAIN="${CPP_BUILD_DIR}/conan_toolchain.cmake"
LOCK_FILE="${BUILD_ROOT}/.final-chain-pure-cpp.lock"

command -v flock >/dev/null 2>&1 || {
  echo "Error: flock is required to serialize the FinalChain pure-C++ cache" >&2
  exit 1
}
exec 9>"${LOCK_FILE}"
flock 9

mkdir -p "${CPP_BUILD_DIR}"

if [[ -f "${CPP_BUILD_DIR}/CMakeCache.txt" ]]; then
  cached_home="$(sed -n 's/^CMAKE_HOME_DIRECTORY:INTERNAL=//p' "${CPP_BUILD_DIR}/CMakeCache.txt")"
  if [[ -z "${cached_home}" || ! -d "${cached_home}" ]]; then
    echo "Error: existing cache has no valid CMAKE_HOME_DIRECTORY: ${CPP_BUILD_DIR}" >&2
    echo "Choose a different root, for example: make rewrite-validate-final-chain-parity FINAL_CHAIN_CPP_BUILD_ROOT=/tmp/rustaxa-final-chain-pure-cpp-alt" >&2
    exit 1
  fi
  cached_home="$(cd "${cached_home}" && pwd -P)"
  if [[ "${cached_home}" != "${ROOT_DIR}" ]]; then
    echo "Error: existing cache belongs to ${cached_home}, not ${ROOT_DIR}" >&2
    echo "Choose a different root, for example: make rewrite-validate-final-chain-parity FINAL_CHAIN_CPP_BUILD_ROOT=/tmp/rustaxa-final-chain-pure-cpp-alt" >&2
    exit 1
  fi
fi

RUSTAXA_OPTIONS=(
  RUSTAXA_ENABLE
  RUSTAXA_ENABLE_VDF
  RUSTAXA_ENABLE_STORAGE
  RUSTAXA_ENABLE_FINAL_CHAIN
  RUSTAXA_ENABLE_PBFT_CHAIN
  RUSTAXA_ENABLE_PROPOSED_BLOCKS
  RUSTAXA_ENABLE_VERIFIED_VOTES
  RUSTAXA_ENABLE_PILLAR_VOTES
  RUSTAXA_ENABLE_GAS_PRICER
  RUSTAXA_ENABLE_SLASHING_MANAGER
)

mapfile -t discovered_options < <(
  sed -nE 's/^option\((RUSTAXA_ENABLE[A-Z_]*)[[:space:]].*/\1/p' "${ROOT_DIR}/CMakeLists.txt"
)
expected_inventory="$(printf '%s\n' "${RUSTAXA_OPTIONS[@]}" | sort)"
discovered_inventory="$(printf '%s\n' "${discovered_options[@]}" | sort)"
if [[ "${discovered_inventory}" != "${expected_inventory}" ]]; then
  echo "Error: root CMake Rustaxa option inventory changed; update this Tier 3 gate before running it" >&2
  diff -u <(printf '%s\n' "${expected_inventory}") <(printf '%s\n' "${discovered_inventory}") >&2 || true
  exit 1
fi

source_fingerprint() {
  (
    cd "${ROOT_DIR}"
    git rev-parse HEAD
    git diff --no-ext-diff --binary HEAD --
    while IFS= read -r -d '' path; do
      printf 'untracked:%s\n' "${path}"
      sha256sum -- "${path}"
    done < <(git ls-files --others --exclude-standard -z | sort -z)
  ) | sha256sum | awk '{print $1}'
}

SOURCE_FINGERPRINT_BEFORE="$(source_fingerprint)"

echo "[1/5] Installing the Conan clang toolchain in ${CPP_BUILD_DIR}..."
conan install "${ROOT_DIR}" \
  -s "build_type=Release" \
  -s "&:build_type=Release" \
  --profile:host=clang \
  --profile:build=clang \
  --build=missing \
  --output-folder="${CPP_BUILD_DIR}"

if [[ ! -f "${CONAN_TOOLCHAIN}" ]]; then
  echo "Error: Conan did not generate ${CONAN_TOOLCHAIN}" >&2
  exit 1
fi

echo "[2/5] Configuring isolated pure-C++ FinalChain mode..."
cmake -S "${ROOT_DIR}" -B "${CPP_BUILD_DIR}" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE="${CONAN_TOOLCHAIN}" \
  -DRUSTAXA_ENABLE=OFF \
  -DRUSTAXA_ENABLE_VDF=OFF \
  -DRUSTAXA_ENABLE_STORAGE=OFF \
  -DRUSTAXA_ENABLE_FINAL_CHAIN=OFF \
  -DRUSTAXA_ENABLE_PBFT_CHAIN=OFF \
  -DRUSTAXA_ENABLE_PROPOSED_BLOCKS=OFF \
  -DRUSTAXA_ENABLE_VERIFIED_VOTES=OFF \
  -DRUSTAXA_ENABLE_PILLAR_VOTES=OFF \
  -DRUSTAXA_ENABLE_GAS_PRICER=OFF \
  -DRUSTAXA_ENABLE_SLASHING_MANAGER=OFF

for option in "${RUSTAXA_OPTIONS[@]}"; do
  if ! grep -q "^${option}:BOOL=OFF$" "${CPP_BUILD_DIR}/CMakeCache.txt"; then
    echo "Error: ${option} is not OFF in ${CPP_BUILD_DIR}/CMakeCache.txt" >&2
    exit 1
  fi
done

echo "[3/5] Building pure-C++ final_chain_test with 12 jobs..."
cmake --build "${CPP_BUILD_DIR}" --target final_chain_test --parallel 12

FINAL_CHAIN_TEST="${CPP_BUILD_DIR}/bin/final_chain_test"
if [[ ! -x "${FINAL_CHAIN_TEST}" ]]; then
  echo "Error: expected executable was not built: ${FINAL_CHAIN_TEST}" >&2
  exit 1
fi

echo "[4/5] Running focused native DPoS delegate/register parity fixtures..."
(
  cd "${CPP_BUILD_DIR}"
  "${FINAL_CHAIN_TEST}" \
    --gtest_filter=FinalChainTest.native_dpos_delegate_persists_receipt_and_state:FinalChainTest.native_dpos_delegate_to_missing_validator_rolls_back_state:FinalChainTest.native_dpos_register_validator_business_failures_roll_back_state:FinalChainTest.native_dpos_claim_rewards_from_sender_without_delegation_rolls_back_state
)

echo "[5/5] Running complete pure-C++ final_chain_test..."
(
  cd "${CPP_BUILD_DIR}"
  "${FINAL_CHAIN_TEST}"
)

SOURCE_FINGERPRINT_AFTER="$(source_fingerprint)"
if [[ "${SOURCE_FINGERPRINT_AFTER}" != "${SOURCE_FINGERPRINT_BEFORE}" ]]; then
  echo "Error: repository source changed while the FinalChain parity gate was running; results are invalid" >&2
  exit 1
fi

echo "Tier 3 FinalChain parity validation passed."
echo "Pure-C++ build retained at: ${CPP_BUILD_DIR}"

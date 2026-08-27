#!/bin/sh
set -eu

usage() {
  cat <<'EOF'
Usage: scripts/rewrite_storage_boundary_guard.sh [--base REV] [--self-test]

Checks newly added C++ lines for storage-boundary violations in Rust rewrite
code, including direct C++ FinalChain DPoS fact reads from consensus consumers.
Read-only RPC/GraphQL compatibility must be marked RUSTAXA_QUERY_COMPAT_READ.
Network/tarcap storage compatibility must be marked
RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY and kept out of Rust-enabled production
routing.
By default the guard checks staged and unstaged changes. With --base it checks
additions introduced since the merge base with REV.
EOF
}

base=""
self_test=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --base)
      shift
      if [ "$#" -eq 0 ]; then
        echo "missing revision after --base" >&2
        exit 2
      fi
      base="$1"
      ;;
    --self-test)
      self_test=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

scan_diff() {
  awk '
    function is_cpp_file(path) {
      return path ~ /\.(c|cc|cpp|cxx|h|hh|hpp|hxx)$/
    }

    function is_allowlisted(path) {
      return path ~ /^libraries\/core_libs\/storage\// ||
             path ~ /^tests\//
    }

    function is_query_compat_read(path, line) {
      return (path ~ /^libraries\/core_libs\/network\/(rpc|graphql)\//) &&
             line ~ /RUSTAXA_QUERY_COMPAT_READ/
    }

    function is_network_compat_route(path, line) {
      return (path ~ /^libraries\/core_libs\/network\/(include\/network\/tarcap|src\/tarcap)\//) &&
             line ~ /RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY/
    }

    function has_call(line, name) {
      return line ~ "(^|[^[:alnum:]_])" name "[[:space:]]*\\("
    }

    function is_final_chain_fact_provider(path) {
      return path ~ /^libraries\/core_libs\/consensus\/shims\/final_chain_shim\//
    }

    function is_forbidden_storage_route(line) {
      return line ~ /std::shared_ptr[[:space:]]*<DbStorage>/ ||
             line ~ /DbStorage[[:space:]]*[\*&]/ ||
             line ~ /(^|[^[:alnum:]_])db_->/ ||
             has_call(line, "createWriteBatch") ||
             has_call(line, "commitWriteBatch") ||
             has_call(line, "rustBatchId") ||
             has_call(line, "rustStorage") ||
             has_call(line, "getDB") ||
             line ~ /DbStorage::Columns/ ||
             line ~ /BridgeStorageBatch/ ||
             line ~ /storage_shim_[[:alnum:]_]+/
    }

    function is_forbidden_final_chain_fact_route(path, line) {
      return !is_final_chain_fact_provider(path) &&
             (has_call(line, "dposEligibleVoteCount") ||
              has_call(line, "dposEligibleTotalVoteCount"))
    }

    function report(line) {
      if (path != "" && is_cpp_file(path) && !is_allowlisted(path) &&
          !is_query_compat_read(path, line) &&
          !is_network_compat_route(path, line) &&
          (is_forbidden_storage_route(line) || is_forbidden_final_chain_fact_route(path, line))) {
        printf "%s:%d: %s\n", path, new_line, line
        found = 1
      }
    }

    /^\+\+\+ b\// {
      path = substr($0, 7)
      next
    }

    /^\+\+\+ \/dev\/null/ {
      path = ""
      next
    }

    /^@@ / {
      if (match($0, /\+[0-9]+/)) {
        new_line = substr($0, RSTART + 1, RLENGTH - 1) - 1
      }
      next
    }

    /^\+/ && !/^\+\+\+/ {
      new_line++
      report(substr($0, 2))
      next
    }

    /^ / {
      new_line++
      next
    }

    END {
      exit found ? 1 : 0
    }
  '
}

check_conformance_fixture_helper_scope() {
  root="${1:-.}"
  helper="consensus_application_run_storage_conformance_v1"
  violations="$(
    find "$root" \
      \( -path "$root/.git" -o -path "$root/rust/target" -o -path "$root/build" -o -path "$root/.cache" \) -prune -o \
      -type f \( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.cxx' -o \
                 -name '*.h' -o -name '*.hh' -o -name '*.hpp' -o -name '*.hxx' -o \
                 -name '*.rs' -o -name '*.sh' \) \
      -exec grep -Hn "$helper" {} + 2>/dev/null |
    sed "s#^$root/##" |
    awk -F: '
      $1 != "tests/storage_conformance/storage_conformance_runner.cpp" &&
      $1 != "tests/rust/storage/test_storage.cpp" &&
      $1 != "rust/crates/rustaxa-bridge/src/ffi.rs" &&
      $1 != "rust/crates/rustaxa-bridge/src/storage_admin.rs" &&
      $1 != "scripts/rewrite_storage_boundary_guard.sh" {
        print
      }
    '
  )"
  if [ -n "$violations" ]; then
    cat >&2 <<'EOF'
Rust storage-boundary guard failed.

The versioned production-root conformance transcript is intentionally limited
to the storage conformance runner, its focused bridge test, and the Rust bridge
adapter. Add a dedicated operation-shaped API instead of creating new callers.

Violations:
EOF
    printf '%s\n' "$violations" >&2
    return 1
  fi
}

if [ "$self_test" -eq 1 ]; then
  violations_file="$(mktemp)"
  fixture_scope_root="$(mktemp -d)"
  trap 'rm -f "$violations_file"; rm -rf "$fixture_scope_root"' EXIT

  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/network/rpc/Taraxa.cpp b/libraries/core_libs/network/rpc/Taraxa.cpp
--- a/libraries/core_libs/network/rpc/Taraxa.cpp
+++ b/libraries/core_libs/network/rpc/Taraxa.cpp
@@ -1,0 +1,1 @@
+auto bytes = app->getDB()->rustStorage().get_pillar_block_data_rlp(period);
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: rustStorage() addition was not rejected" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp b/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
--- a/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
+++ b/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
@@ -1,0 +1,1 @@
+auto db = app->getDB();
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: getDB() addition was not rejected" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/network/rpc/Taraxa.cpp b/libraries/core_libs/network/rpc/Taraxa.cpp
--- a/libraries/core_libs/network/rpc/Taraxa.cpp
+++ b/libraries/core_libs/network/rpc/Taraxa.cpp
@@ -1,0 +1,1 @@
+auto db = app->getDB();  // RUSTAXA_QUERY_COMPAT_READ: legacy RPC compatibility read.
EOF
  if [ -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: documented query compatibility read was rejected" >&2
    cat "$violations_file" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/network/graphql/src/query.cpp b/libraries/core_libs/network/graphql/src/query.cpp
--- a/libraries/core_libs/network/graphql/src/query.cpp
+++ b/libraries/core_libs/network/graphql/src/query.cpp
@@ -1,0 +1,1 @@
+auto blocks = db_->getDagBlocksAtLevel(level, 1);
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: unmarked GraphQL storage read was not rejected" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/network/graphql/src/query.cpp b/libraries/core_libs/network/graphql/src/query.cpp
--- a/libraries/core_libs/network/graphql/src/query.cpp
+++ b/libraries/core_libs/network/graphql/src/query.cpp
@@ -1,0 +1,1 @@
+auto blocks = db_->getDagBlocksAtLevel(level, 1);  // RUSTAXA_QUERY_COMPAT_READ: legacy GraphQL compatibility read.
EOF
  if [ -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: documented GraphQL compatibility read was rejected" >&2
    cat "$violations_file" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp b/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp
--- a/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp
+++ b/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp
@@ -1,0 +1,1 @@
+auto period_data = db_->getPeriodDataRaw(block_period);
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: unmarked network storage route was not rejected" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp b/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp
--- a/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp
+++ b/libraries/core_libs/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler.cpp
@@ -1,0 +1,1 @@
+auto period_data = db_->getPeriodDataRaw(block_period);  // RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY: legacy tarcap sync path.
EOF
  if [ -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: documented network compatibility route was rejected" >&2
    cat "$violations_file" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/consensus/shims/vote_manager_shim/src/vote_manager_shim.cpp b/libraries/core_libs/consensus/shims/vote_manager_shim/src/vote_manager_shim.cpp
--- a/libraries/core_libs/consensus/shims/vote_manager_shim/src/vote_manager_shim.cpp
+++ b/libraries/core_libs/consensus/shims/vote_manager_shim/src/vote_manager_shim.cpp
@@ -1,0 +1,1 @@
+auto weight = final_chain_->dposEligibleVoteCount(period, voter);
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: direct FinalChain DPoS fact read was not rejected" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/consensus/shims/final_chain_shim/src/final_chain_shim.cpp b/libraries/core_libs/consensus/shims/final_chain_shim/src/final_chain_shim.cpp
--- a/libraries/core_libs/consensus/shims/final_chain_shim/src/final_chain_shim.cpp
+++ b/libraries/core_libs/consensus/shims/final_chain_shim/src/final_chain_shim.cpp
@@ -1,0 +1,1 @@
+uint64_t FinalChain::dposEligibleVoteCount(EthBlockNumber blk_num, addr_t const& addr) const {
EOF
  if [ -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: FinalChain fact-provider implementation was rejected" >&2
    cat "$violations_file" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp b/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
--- a/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
+++ b/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
@@ -1,0 +1,1 @@
+rustaxa::storage_shim_save_period_data(batch, period, bytes);
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: direct storage_shim_* call was not rejected" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp b/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
--- a/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
+++ b/libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
@@ -1,0 +1,1 @@
+rustaxa::BridgeStorageBatch* batch = nullptr;
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: BridgeStorageBatch addition was not rejected" >&2
    exit 1
  fi

  : >"$violations_file"
  cat <<'EOF' | scan_diff >"$violations_file" || true
diff --git a/libraries/core_libs/consensus/src/application/new_storage_route.cpp b/libraries/core_libs/consensus/src/application/new_storage_route.cpp
--- a/libraries/core_libs/consensus/src/application/new_storage_route.cpp
+++ b/libraries/core_libs/consensus/src/application/new_storage_route.cpp
@@ -1,0 +1,1 @@
+rustaxa::BridgeStorageBatch* batch = nullptr;
EOF
  if [ ! -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: new direct storage route was not rejected" >&2
    exit 1
  fi

  mkdir -p "$fixture_scope_root/tests/storage_conformance" \
           "$fixture_scope_root/rust/crates/rustaxa-bridge/src" \
           "$fixture_scope_root/libraries/core_libs/consensus/shims/vote_manager_shim/src"
  echo 'rustaxa::consensus_application_run_storage_conformance_v1(*application);' \
    >"$fixture_scope_root/tests/storage_conformance/storage_conformance_runner.cpp"
  echo 'pub fn consensus_application_run_storage_conformance_v1() {}' \
    >"$fixture_scope_root/rust/crates/rustaxa-bridge/src/storage_admin.rs"
  echo 'pub fn consensus_application_run_storage_conformance_v1();' \
    >"$fixture_scope_root/rust/crates/rustaxa-bridge/src/ffi.rs"
  if ! check_conformance_fixture_helper_scope "$fixture_scope_root"; then
    echo "storage-boundary guard self-test failed: allowed conformance fixture scope was rejected" >&2
    exit 1
  fi

  echo 'rustaxa::consensus_application_run_storage_conformance_v1(*application);' \
    >"$fixture_scope_root/libraries/core_libs/consensus/shims/vote_manager_shim/src/vote_manager_shim.cpp"
  if check_conformance_fixture_helper_scope "$fixture_scope_root" >/dev/null 2>&1; then
    echo "storage-boundary guard self-test failed: new conformance fixture helper caller was not rejected" >&2
    exit 1
  fi

  echo "Rust storage-boundary guard self-test passed."
  exit 0
fi

violations_file="$(mktemp)"
trap 'rm -f "$violations_file"' EXIT

if [ -n "$base" ]; then
  merge_base="$(git merge-base "$base" HEAD)"
  git diff --unified=0 "$merge_base"..HEAD -- \
    "*.c" "*.cc" "*.cpp" "*.cxx" "*.h" "*.hh" "*.hpp" "*.hxx" | scan_diff >"$violations_file" || true
else
  {
    git diff --cached --unified=0 -- \
      "*.c" "*.cc" "*.cpp" "*.cxx" "*.h" "*.hh" "*.hpp" "*.hxx"
    git diff --unified=0 -- \
      "*.c" "*.cc" "*.cpp" "*.cxx" "*.h" "*.hh" "*.hpp" "*.hxx"
  } | scan_diff >"$violations_file" || true
fi

if [ -s "$violations_file" ]; then
  cat >&2 <<'EOF'
Rust storage-boundary guard failed.

In Rust mode, C++ consensus/final-chain code must not add new storage routes.
Move storage reads/writes into rustaxa-storage-backed Rust runtime APIs and keep
C++ limited to transport, external EVM execution, signing, timers, logging, and
legacy view translation. Consensus consumers must also use typed Rust fact ports
for FinalChain DPoS facts instead of adding new direct C++ FinalChain calls.
RPC/GraphQL query additions require RUSTAXA_QUERY_COMPAT_READ, and network/tarcap
compatibility additions require RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY plus legacy
guarding so they cannot become Rust-mode consensus storage routes.

Violations:
EOF
  cat "$violations_file" >&2
  exit 1
fi

check_conformance_fixture_helper_scope

echo "Rust storage-boundary guard passed."

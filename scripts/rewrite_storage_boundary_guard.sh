#!/bin/sh
set -eu

usage() {
  cat <<'EOF'
Usage: scripts/rewrite_storage_boundary_guard.sh [--base REV] [--self-test]

Checks newly added C++ lines for storage-boundary violations in Rust rewrite
code. By default the guard checks staged and unstaged changes. With --base it
checks additions introduced since the merge base with REV.
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
      return path ~ /^libraries\/core_libs\/consensus\/shims\/storage_shim\// ||
             path ~ /^libraries\/core_libs\/storage\// ||
             path ~ /^tests\//
    }

    function is_query_compat_read(path, line) {
      return (path ~ /^libraries\/core_libs\/network\/(rpc|graphql)\//) &&
             line ~ /RUSTAXA_QUERY_COMPAT_READ/
    }

    function has_call(line, name) {
      return line ~ "(^|[^[:alnum:]_])" name "[[:space:]]*\\("
    }

    function is_forbidden(line) {
      return line ~ /std::shared_ptr[[:space:]]*<DbStorage>/ ||
             line ~ /DbStorage[[:space:]]*[\*&]/ ||
             line ~ /(^|[^[:alnum:]_])db_->/ ||
             has_call(line, "createWriteBatch") ||
             has_call(line, "commitWriteBatch") ||
             has_call(line, "rustBatchId") ||
             has_call(line, "rustStorage") ||
             has_call(line, "getDB") ||
             line ~ /DbStorage::Columns/
    }

    function report(line) {
      if (path != "" && is_cpp_file(path) && !is_allowlisted(path) &&
          !is_query_compat_read(path, line) && is_forbidden(line)) {
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

if [ "$self_test" -eq 1 ]; then
  violations_file="$(mktemp)"
  trap 'rm -f "$violations_file"' EXIT

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
diff --git a/libraries/core_libs/consensus/shims/storage_shim/src/storage_shim.cpp b/libraries/core_libs/consensus/shims/storage_shim/src/storage_shim.cpp
--- a/libraries/core_libs/consensus/shims/storage_shim/src/storage_shim.cpp
+++ b/libraries/core_libs/consensus/shims/storage_shim/src/storage_shim.cpp
@@ -1,0 +1,1 @@
+auto batch_id = rust_storage_.value()->create_write_batch();
EOF
  if [ -s "$violations_file" ]; then
    echo "storage-boundary guard self-test failed: storage shim allowlist was rejected" >&2
    cat "$violations_file" >&2
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
legacy view translation.

Violations:
EOF
  cat "$violations_file" >&2
  exit 1
fi

echo "Rust storage-boundary guard passed."

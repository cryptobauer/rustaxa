#!/bin/sh
set -eu

usage() {
  cat <<'EOF'
Usage: scripts/rewrite_storage_boundary_guard.sh [--base REV]

Checks newly added C++ lines for storage-boundary violations in Rust rewrite
code. By default the guard checks staged and unstaged changes. With --base it
checks additions introduced since the merge base with REV.
EOF
}

base=""
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

    function is_forbidden(line) {
      return line ~ /std::shared_ptr[[:space:]]*<DbStorage>/ ||
             line ~ /DbStorage[[:space:]]*[\*&]/ ||
             line ~ /\bdb_->/ ||
             line ~ /\bcreateWriteBatch[[:space:]]*\(/ ||
             line ~ /\bcommitWriteBatch[[:space:]]*\(/ ||
             line ~ /\brustBatchId[[:space:]]*\(/ ||
             line ~ /\brustStorage[[:space:]]*\(/ ||
             line ~ /DbStorage::Columns/
    }

    function report(line) {
      if (path != "" && is_cpp_file(path) && !is_allowlisted(path) && is_forbidden(line)) {
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

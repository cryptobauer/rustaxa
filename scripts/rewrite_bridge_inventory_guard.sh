#!/bin/sh
set -eu

usage() {
  cat <<'EOF'
Usage: scripts/rewrite_bridge_inventory_guard.sh [--self-test]

Checks that every exported CXX `Bridge*` handle in
rust/crates/rustaxa-bridge/src/ffi.rs is documented in
doc/consensus_bridge_shim_audit.md.
EOF
}

self_test=0
while [ "$#" -gt 0 ]; do
  case "$1" in
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

extract_ffi_bridge_types() {
  sed -n 's/^[[:space:]]*type \(Bridge[A-Za-z0-9_]*\);[[:space:]]*$/\1/p' "$1" | sort -u
}

extract_audited_bridge_types() {
  awk '
    /^## Exported CXX Bridge Handles/ {
      in_section = 1
      next
    }
    /^## / && in_section {
      exit
    }
    in_section {
      if (match($0, /^\| `Bridge[A-Za-z0-9_]+` \|/)) {
        handle = substr($0, RSTART + 3, RLENGTH - 6)
        print handle
      }
    }
  ' "$1" | sort -u
}

check_inventory() {
  ffi_file="$1"
  audit_file="$2"
  missing_file="$3"
  stale_file="$4"

  : >"$missing_file"
  : >"$stale_file"
  ffi_types="$(mktemp)"
  audit_types="$(mktemp)"
  trap 'rm -f "$ffi_types" "$audit_types"' HUP INT TERM
  extract_ffi_bridge_types "$ffi_file" >"$ffi_types"
  extract_audited_bridge_types "$audit_file" >"$audit_types"

  comm -23 "$ffi_types" "$audit_types" >"$missing_file"
  comm -13 "$ffi_types" "$audit_types" >"$stale_file"

  rm -f "$ffi_types" "$audit_types"
  trap - HUP INT TERM
}

report_stale_entries() {
  stale_file="$1"
  if [ -s "$stale_file" ]; then
    cat >&2 <<'EOF'
Warning: audit entries exist for bridge handles that are not exported anymore.
Remove stale rows when deleting the corresponding compatibility surface.

Stale audit entries:
EOF
    cat "$stale_file" >&2
  fi
}

check_missing_exactly() {
  ffi_file="$1"
  audit_file="$2"
  expected_missing="$3"
  temp_dir="$4"

  missing_file="$temp_dir/missing"
  stale_file="$temp_dir/stale"
  check_inventory "$ffi_file" "$audit_file" "$missing_file" "$stale_file"
  if ! grep -qx "$expected_missing" "$missing_file"; then
    echo "bridge inventory guard self-test failed: missing type was not reported" >&2
    exit 1
  fi
}

check_no_missing() {
  ffi_file="$1"
  audit_file="$2"
  temp_dir="$3"

  missing_file="$temp_dir/missing"
  stale_file="$temp_dir/stale"
  check_inventory "$ffi_file" "$audit_file" "$missing_file" "$stale_file"
  if [ -s "$missing_file" ]; then
    echo "bridge inventory guard self-test failed: documented type was reported" >&2
    cat "$missing_file" >&2
    exit 1
  fi
}

check_stale_exactly() {
  ffi_file="$1"
  audit_file="$2"
  expected_stale="$3"
  temp_dir="$4"

  missing_file="$temp_dir/missing"
  stale_file="$temp_dir/stale"
  check_inventory "$ffi_file" "$audit_file" "$missing_file" "$stale_file"
  if ! grep -qx "$expected_stale" "$stale_file"; then
    echo "bridge inventory guard self-test failed: stale type was not reported" >&2
    exit 1
  fi
}

check_bridge_names_do_not_leak_from_other_sections() {
  ffi_file="$1"
  audit_file="$2"
  ignored_type="$3"
  temp_dir="$4"

  missing_file="$temp_dir/missing"
  stale_file="$temp_dir/stale"
  check_inventory "$ffi_file" "$audit_file" "$missing_file" "$stale_file"
  if ! grep -qx "$ignored_type" "$missing_file"; then
    echo "bridge inventory guard self-test failed: bridge mention outside handle table satisfied inventory" >&2
    exit 1
  fi
}

if [ "$self_test" -eq 1 ]; then
  temp_dir="$(mktemp -d)"
  trap 'rm -rf "$temp_dir"' EXIT

  cat >"$temp_dir/ffi.rs" <<'EOF'
#[cxx::bridge]
mod rustaxa_ffi {
    unsafe extern "Rust" {
        type BridgeDocumented;
        type BridgeMissing;
    }
}
EOF

  cat >"$temp_dir/audit.md" <<'EOF'
# Audit

| `BridgeMissing` | mentioned in the wrong section |

## Exported CXX Bridge Handles

| Handle | Implementing module | Current consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeDocumented` | `module.rs` | test | External boundary | keep |

## Next Section
EOF

  check_missing_exactly "$temp_dir/ffi.rs" "$temp_dir/audit.md" BridgeMissing "$temp_dir"
  check_bridge_names_do_not_leak_from_other_sections "$temp_dir/ffi.rs" "$temp_dir/audit.md" BridgeMissing "$temp_dir"

  # Insert the missing row before the next section.
  sed '/^## Next Section/i | `BridgeMissing` | `module.rs` | test | External boundary | keep |' "$temp_dir/audit.md" \
    >"$temp_dir/audit_with_missing.md"

  check_no_missing "$temp_dir/ffi.rs" "$temp_dir/audit_with_missing.md" "$temp_dir"

  cat >"$temp_dir/audit_with_stale.md" <<'EOF'
# Audit

## Exported CXX Bridge Handles

| Handle | Implementing module | Current consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeDocumented` | `module.rs` | test | External boundary | keep |
| `BridgeMissing` | `module.rs` | test | External boundary | keep |
| `BridgeStale` | `module.rs` | test | External boundary | keep |
EOF

  check_stale_exactly "$temp_dir/ffi.rs" "$temp_dir/audit_with_stale.md" BridgeStale "$temp_dir"

  echo "Rust bridge inventory guard self-test passed."
  exit 0
fi

missing_file="$(mktemp)"
stale_file="$(mktemp)"
trap 'rm -f "$missing_file" "$stale_file"' EXIT

check_inventory \
  rust/crates/rustaxa-bridge/src/ffi.rs \
  doc/consensus_bridge_shim_audit.md \
  "$missing_file" \
  "$stale_file"

report_stale_entries "$stale_file"

if [ -s "$missing_file" ]; then
  cat >&2 <<'EOF'
Rust bridge inventory guard failed.

Every exported CXX `Bridge*` handle must be classified in
doc/consensus_bridge_shim_audit.md before it is added or kept. Add an audit row
under "Exported CXX Bridge Handles" with current consumers, classification, and
a deletion or narrowing condition.

Missing audit entries:
EOF
  cat "$missing_file" >&2
  exit 1
fi

echo "Rust bridge inventory guard passed."

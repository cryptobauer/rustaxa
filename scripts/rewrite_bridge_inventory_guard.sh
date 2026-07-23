#!/bin/sh
set -eu

usage() {
  cat <<'EOF'
Usage: scripts/rewrite_bridge_inventory_guard.sh [--self-test]

Checks that every exported CXX `Bridge*` handle, Rust bridge module, and
consensus shim directory is documented in doc/consensus_bridge_shim_audit.md,
and reports stale inventory rows after deletion.
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

extract_bridge_modules() {
  sed -En 's#^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+([a-z0-9_]+)[[:space:]]*;([[:space:]]*(//.*|/\*.*\*/))?$#rust/crates/rustaxa-bridge/src/\3.rs#p' "$1" | sort -u
}

extract_audited_bridge_modules() {
  awk '
    /^## Rust Bridge Modules/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && match($0, /^\| `rust\/crates\/rustaxa-bridge\/src\/[a-z0-9_]+\.rs` \|/) {
      value = substr($0, RSTART + 3, RLENGTH - 6)
      print value
    }
  ' "$1" | sort -u
}

extract_shim_directories() {
  for shim_path in "$1"/*_shim; do
    [ -d "$shim_path" ] || continue
    basename "$shim_path"
  done | sort -u
}

extract_audited_shim_directories() {
  awk '
    /^## Consensus Shim Directories/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && match($0, /^\| `[a-z0-9_]+_shim` \|/) {
      value = substr($0, RSTART + 3, RLENGTH - 6)
      print value
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
  inventory_name="$1"
  stale_file="$2"
  if [ -s "$stale_file" ]; then
    echo "Error: stale $inventory_name audit entries exist; remove them with the retired surface." >&2
    cat "$stale_file" >&2
  fi
}

check_documented_inventory() {
  inventory_name="$1"
  live_file="$2"
  audit_file="$3"
  live_extractor="$4"
  audit_extractor="$5"
  temp_dir="$6"

  live_values="$temp_dir/${inventory_name}_live"
  audit_values="$temp_dir/${inventory_name}_audit"
  missing_values="$temp_dir/${inventory_name}_missing"
  stale_values="$temp_dir/${inventory_name}_stale"
  "$live_extractor" "$live_file" >"$live_values"
  "$audit_extractor" "$audit_file" >"$audit_values"
  comm -23 "$live_values" "$audit_values" >"$missing_values"
  comm -13 "$live_values" "$audit_values" >"$stale_values"
  report_stale_entries "$inventory_name" "$stale_values"
  if [ -s "$stale_values" ]; then
    return 1
  fi
  if [ -s "$missing_values" ]; then
    echo "Rust bridge inventory guard failed: undocumented $inventory_name entries:" >&2
    cat "$missing_values" >&2
    return 1
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

  cat >"$temp_dir/lib.rs" <<'EOF'
mod documented;
mod documented_block; /* retained block comment */
  pub mod documented_public;
pub(crate) mod documented_crate; // visible inside the crate
EOF
  mkdir -p "$temp_dir/shims/documented_shim"
  cat >"$temp_dir/full_audit.md" <<'EOF'
# Audit

## Rust Bridge Modules

| Module | Main exported handles or constructors | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/documented.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_block.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_crate.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_public.rs` | helper | test | Internal Rust route | keep |

## Consensus Shim Directories

| Shim directory | Current role | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `documented_shim` | helper | test | C++ public compatibility facade | keep |
EOF
  check_documented_inventory module "$temp_dir/lib.rs" "$temp_dir/full_audit.md" \
    extract_bridge_modules extract_audited_bridge_modules "$temp_dir"
  check_documented_inventory shim "$temp_dir/shims" "$temp_dir/full_audit.md" \
    extract_shim_directories extract_audited_shim_directories "$temp_dir"

  echo 'mod missing;' >>"$temp_dir/lib.rs"
  if check_documented_inventory module "$temp_dir/lib.rs" "$temp_dir/full_audit.md" \
    extract_bridge_modules extract_audited_bridge_modules "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: undocumented module was accepted" >&2
    exit 1
  fi
  cat >"$temp_dir/lib.rs" <<'EOF'
mod documented;
mod documented_block; /* retained block comment */
  pub mod documented_public;
pub(crate) mod documented_crate; // visible inside the crate
EOF
  mkdir "$temp_dir/shims/missing_shim"
  if check_documented_inventory shim "$temp_dir/shims" "$temp_dir/full_audit.md" \
    extract_shim_directories extract_audited_shim_directories "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: undocumented shim was accepted" >&2
    exit 1
  fi
  rm -rf "$temp_dir/shims/missing_shim"

  cat >"$temp_dir/stale_audit.md" <<'EOF'
# Audit

## Rust Bridge Modules

| Module | Main exported handles or constructors | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/documented.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_block.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_crate.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_public.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/stale.rs` | helper | test | Internal Rust route | remove |

## Consensus Shim Directories

| Shim directory | Current role | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `documented_shim` | helper | test | C++ public compatibility facade | keep |
| `stale_shim` | helper | test | Obsolete scaffold | remove |
EOF
  if check_documented_inventory module "$temp_dir/lib.rs" "$temp_dir/stale_audit.md" \
    extract_bridge_modules extract_audited_bridge_modules "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: stale module row was accepted" >&2
    exit 1
  fi
  if check_documented_inventory shim "$temp_dir/shims" "$temp_dir/stale_audit.md" \
    extract_shim_directories extract_audited_shim_directories "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: stale shim row was accepted" >&2
    exit 1
  fi

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

report_stale_entries "handle" "$stale_file"

if [ -s "$stale_file" ]; then
  exit 1
fi

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

inventory_temp_dir="$(mktemp -d)"
trap 'rm -f "$missing_file" "$stale_file"; rm -rf "$inventory_temp_dir"' EXIT

check_documented_inventory \
  module rust/crates/rustaxa-bridge/src/lib.rs doc/consensus_bridge_shim_audit.md \
  extract_bridge_modules extract_audited_bridge_modules "$inventory_temp_dir"
check_documented_inventory \
  shim libraries/core_libs/consensus/shims doc/consensus_bridge_shim_audit.md \
  extract_shim_directories extract_audited_shim_directories "$inventory_temp_dir"

echo "Rust bridge inventory guard passed."

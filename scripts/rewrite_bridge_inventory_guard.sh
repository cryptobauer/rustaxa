#!/bin/sh
set -eu

usage() {
  cat <<'EOF'
Usage: scripts/rewrite_bridge_inventory_guard.sh [--self-test] [--base-ref REF]

Checks that every exported CXX opaque handle, Rust bridge module, non-test C++
consumer, and consensus shim directory is documented in
doc/consensus_bridge_shim_audit.md, reports stale inventory rows after deletion,
and prevents checked surface budgets from increasing relative to the selected
base revision.
EOF
}

self_test=0
base_ref=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --self-test)
      self_test=1
      ;;
    --base-ref)
      shift
      if [ "$#" -eq 0 ]; then
        echo "--base-ref requires a revision" >&2
        exit 2
      fi
      base_ref="$1"
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
inventory_parser="$repo_root/scripts/rewrite_bridge_inventory.py"

extract_ffi_bridge_types() {
  python3 "$inventory_parser" ffi-handles "$1"
}

extract_audited_bridge_types() {
  awk '
    /^## Exported CXX Opaque Handles/ {
      in_section = 1
      next
    }
    /^## / && in_section {
      exit
    }
    in_section {
      if (match($0, /^\| `[A-Za-z][A-Za-z0-9_]+` \|/)) {
        handle = substr($0, RSTART + 3, RLENGTH - 6)
        print handle
      }
    }
  ' "$1" | sort
}

extract_non_test_cpp_consumers() {
  consumer_root="$1"
  {
    rg -l '#include [<"]rustaxa-bridge/(ffi|application_host_ffi)\.rs\.h[>"]' \
      "$consumer_root/libraries" "$consumer_root/programs" \
      --glob '*.cpp' --glob '*.cc' --glob '*.hpp' --glob '*.h' || true
  } | sed "s#^$consumer_root/##" | sort -u
}

extract_audited_non_test_cpp_consumers() {
  awk '
    /^## Non-Test C[+][+] Bridge Consumers/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && match($0, /^\| `(libraries|programs)\/[A-Za-z0-9_./-]+` \|/) {
      value = substr($0, RSTART + 3, RLENGTH - 6)
      print value
    }
  ' "$1" | sort
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
  ' "$1" | sort
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
  ' "$1" | sort
}

extract_cxx_functions() {
  python3 "$inventory_parser" ffi-functions "$1"
}

extract_partial_service_factories() {
  extract_audited_partial_service_factories "$2"
}

extract_cxx_box_factories() {
  python3 "$inventory_parser" ffi-factories "$1"
}

extract_audited_cxx_box_factories() {
  awk '
    /^## CXX Box Factory Inventory/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && match($0, /^\| `[a-z0-9_]+` \|/) {
      value = substr($0, RSTART + 3, RLENGTH - 6)
      print value
    }
  ' "$1" | sort
}

extract_audited_partial_service_factories() {
  awk -F '|' '
    /^## CXX Box Factory Inventory/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && $3 ~ /^[[:space:]]*Partial service[[:space:]]*$/ {
      value = $2
      gsub(/[ `]/, "", value)
      print value
    }
  ' "$1" | sort -u
}

extract_test_only_export_allowlist() {
  awk '
    /^## Test-Only CXX Export Allowlist/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && match($0, /^\| `[a-z0-9_]+` \|/) {
      value = substr($0, RSTART + 3, RLENGTH - 6)
      print value
    }
  ' "$1" | sort -u
}

extract_audited_partial_factory_sites() {
  awk -F '|' '
    /^## Partial-Service Factory Inventory/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && $2 ~ /`create_[a-z0-9_]+`/ {
      factory = $2
      path = $3
      gsub(/[ `]/, "", factory)
      gsub(/[ `]/, "", path)
      print factory "\t" path
    }
  ' "$1" | sort -u
}

extract_audited_partial_factory_counts() {
  awk -F '|' '
    /^## Partial-Service Factory Inventory/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && $2 ~ /`create_[a-z0-9_]+`/ {
      factory = $2
      count = $4
      gsub(/[ `]/, "", factory)
      gsub(/[ ]/, "", count)
      if (count !~ /^[0-9]+$/) {
        print "invalid partial factory call count for " factory >"/dev/stderr"
        failed = 1
      } else {
        print factory "\t" count
      }
    }
    END { exit failed }
  ' "$1" | sort -u
}

check_factory_inventory_rows() {
  audit_file="$1"
  if ! awk -F '|' '
    /^## CXX Box Factory Inventory/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && $2 ~ /`[a-z0-9_]+`/ {
      factory = $2
      classification = $3
      owner = $4
      condition = $5
      gsub(/[ `]/, "", factory)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", classification)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", owner)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", condition)
      if (++seen[factory] != 1) {
        print "duplicate factory row: " factory >"/dev/stderr"
        failed = 1
      }
      if (classification != "Supported boundary" &&
          classification != "Production root debt" &&
          classification != "Partial service" &&
          classification != "Compatibility facade") {
        print "invalid factory classification for " factory ": " classification >"/dev/stderr"
        failed = 1
      }
      if (owner == "" || condition == "") {
        print "factory row lacks a named owner or deletion condition: " factory >"/dev/stderr"
        failed = 1
      }
    }
    END { exit failed }
  ' "$audit_file"; then
    echo "Rust bridge inventory guard failed: malformed CXX Box factory inventory." >&2
    return 1
  fi
}

collect_cpp_call_names() {
  ffi_file="$1"
  root="$2"
  if [ ! -d "$root" ]; then
    return
  fi
  python3 "$inventory_parser" cpp-calls --ffi "$ffi_file" "$root"
}

check_cxx_export_callers() {
  ffi_file="$1"
  audit_file="$2"
  production_root_one="$3"
  production_root_two="$4"
  test_root="$5"
  temp_dir="$6"

  exports="$temp_dir/cxx_exports"
  production_identifiers="$temp_dir/production_identifiers"
  test_identifiers="$temp_dir/test_identifiers"
  non_production_exports="$temp_dir/non_production_exports"
  test_only_exports="$temp_dir/test_only_exports"
  no_caller_exports="$temp_dir/no_caller_exports"
  allowlisted_exports="$temp_dir/allowlisted_test_only_exports"
  missing_allowlist="$temp_dir/missing_test_only_allowlist"
  stale_allowlist="$temp_dir/stale_test_only_allowlist"

  extract_cxx_functions "$ffi_file" >"$exports"
  {
    collect_cpp_call_names "$ffi_file" "$production_root_one"
    collect_cpp_call_names "$ffi_file" "$production_root_two"
  } | sort -u >"$production_identifiers"
  collect_cpp_call_names "$ffi_file" "$test_root" >"$test_identifiers"
  extract_test_only_export_allowlist "$audit_file" >"$allowlisted_exports"

  comm -23 "$exports" "$production_identifiers" >"$non_production_exports"
  comm -12 "$non_production_exports" "$test_identifiers" >"$test_only_exports"
  comm -23 "$non_production_exports" "$test_identifiers" >"$no_caller_exports"
  comm -23 "$test_only_exports" "$allowlisted_exports" >"$missing_allowlist"
  comm -13 "$test_only_exports" "$allowlisted_exports" >"$stale_allowlist"

  failed=0
  if [ -s "$no_caller_exports" ]; then
    echo "Rust bridge inventory guard failed: CXX functions have no C++ caller:" >&2
    cat "$no_caller_exports" >&2
    failed=1
  fi
  if [ -s "$missing_allowlist" ]; then
    echo "Rust bridge inventory guard failed: test-only CXX functions are not allowlisted:" >&2
    cat "$missing_allowlist" >&2
    failed=1
  fi
  if [ -s "$stale_allowlist" ]; then
    echo "Rust bridge inventory guard failed: stale test-only CXX export allowlist entries exist:" >&2
    cat "$stale_allowlist" >&2
    failed=1
  fi
  if [ "$failed" -ne 0 ]; then
    return 1
  fi
}

check_partial_factory_sites() {
  ffi_file="$1"
  audit_file="$2"
  production_root_one="$3"
  production_root_two="$4"
  temp_dir="$5"

  partial_factories="$temp_dir/partial_factories"
  all_sites="$temp_dir/all_bridge_call_sites"
  live_sites="$temp_dir/live_partial_factory_sites"
  audited_sites="$temp_dir/audited_partial_factory_sites"
  live_counts="$temp_dir/live_partial_factory_counts"
  audited_counts="$temp_dir/audited_partial_factory_counts"
  missing_sites="$temp_dir/missing_partial_factory_sites"
  stale_sites="$temp_dir/stale_partial_factory_sites"

  extract_audited_partial_service_factories "$audit_file" >"$partial_factories"
  python3 "$inventory_parser" cpp-call-sites --ffi "$ffi_file" \
    "$production_root_one" "$production_root_two" >"$all_sites"
  : >"$live_sites"
  : >"$live_counts"
  while IFS= read -r factory; do
    [ -n "$factory" ] || continue
    awk -F '\t' -v wanted="$factory" '$1 == wanted { print $1 "\t" $2 }' "$all_sites" >>"$live_sites"
    count="$(awk -F '\t' -v wanted="$factory" '$1 == wanted { count += 1 } END { print count + 0 }' "$all_sites")"
    echo "$factory	$count" >>"$live_counts"
  done <"$partial_factories"
  sort -u -o "$live_sites" "$live_sites"
  sort -u -o "$live_counts" "$live_counts"
  extract_audited_partial_factory_sites "$audit_file" >"$audited_sites"
  extract_audited_partial_factory_counts "$audit_file" >"$audited_counts"
  comm -23 "$live_sites" "$audited_sites" >"$missing_sites"
  comm -13 "$live_sites" "$audited_sites" >"$stale_sites"

  if [ -s "$missing_sites" ]; then
    echo "Rust bridge inventory guard failed: partial-service factory call sites are undocumented:" >&2
    cat "$missing_sites" >&2
    return 1
  fi
  if [ -s "$stale_sites" ]; then
    echo "Rust bridge inventory guard failed: stale or incorrect partial-service factory call sites exist:" >&2
    cat "$stale_sites" >&2
    return 1
  fi
  if ! cmp -s "$live_counts" "$audited_counts"; then
    echo "Rust bridge inventory guard failed: partial-service factory call counts differ:" >&2
    diff -u "$audited_counts" "$live_counts" >&2 || true
    return 1
  fi
}

extract_surface_budget() {
  metric="$1"
  audit_file="$2"
  awk -F '|' -v wanted="$metric" '
    /^## Checked Surface Budgets/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section {
      name = $2
      value = $3
      gsub(/[ `]/, "", name)
      gsub(/[ ]/, "", value)
      if (name == wanted && value ~ /^[0-9]+$/) {
        print value
        found = 1
      }
    }
    END {
      if (!found) {
        exit 1
      }
    }
  ' "$audit_file"
}

count_shim_source_lines() {
  find "$1" -type f \( -name '*.hpp' -o -name '*.h' -o -name '*.cpp' -o -name '*.cc' \) \
    -exec wc -l {} + | awk '$2 != "total" { total += $1 } END { print total + 0 }'
}

count_cxx_carriers() {
  python3 "$inventory_parser" ffi-carriers "$1" | wc -l
}

count_cxx_function_declarations() {
  python3 "$inventory_parser" ffi-function-count "$1"
}

count_cxx_opaque_handles() {
  python3 "$inventory_parser" ffi-handles "$1" | wc -l
}

count_compatibility_constructor_calls() {
  ffi_file="$1"
  audit_file="$2"
  production_root_one="$3"
  production_root_two="$4"
  temp_dir="$(mktemp -d)"
  extract_partial_service_factories "$ffi_file" "$audit_file" >"$temp_dir/factories"
  python3 "$inventory_parser" cpp-call-sites --ffi "$ffi_file" \
    "$production_root_one" "$production_root_two" >"$temp_dir/sites"
  awk -F '\t' 'NR == FNR { wanted[$1] = 1; next } wanted[$1] { count += 1 } END { print count + 0 }' \
    "$temp_dir/factories" "$temp_dir/sites"
  rm -rf "$temp_dir"
}

check_surface_metric() {
  metric="$1"
  actual="$2"
  audit_file="$3"
  expected="$(extract_surface_budget "$metric" "$audit_file")"
  if [ "$actual" -ne "$expected" ]; then
    echo "Rust bridge inventory guard failed: $metric is $actual; checked budget is $expected." >&2
    echo "Delete compensating surface or lower the budget with the deletion slice." >&2
    return 1
  fi
  echo "$metric=$actual"
}

check_budget_ratchet() {
  audit_file="$1"
  base_audit_file="$2"
  ratchet_failed=0
  for metric in bridge_lines shim_lines cxx_functions cxx_carriers cxx_handles shim_directories \
    granular_flags partial_service_factories compatibility_constructor_calls non_test_cpp_consumers; do
    current="$(extract_surface_budget "$metric" "$audit_file")"
    if ! base="$(extract_surface_budget "$metric" "$base_audit_file" 2>/dev/null)"; then
      continue
    fi
    if [ "$current" -gt "$base" ]; then
      echo "Rust bridge inventory guard failed: $metric budget increased from $base to $current." >&2
      ratchet_failed=1
    fi
  done
  if [ "$ratchet_failed" -ne 0 ]; then
    return 1
  fi
}

check_budget_history() {
  history_current_audit="$1"
  history_repo="$2"
  history_tip="$3"
  stop_revision="$4"
  history_temp_dir="$5"

  revision_args="$history_tip"
  if [ -n "$stop_revision" ]; then
    revision_args="$revision_args ^$stop_revision"
  fi
  index=0
  seen_budget=0
  history_failed=0
  for revision in $(git -C "$history_repo" rev-list $revision_args); do
    historical_audit="$history_temp_dir/history_audit_$index.md"
    index=$((index + 1))
    if ! git -C "$history_repo" show \
      "$revision:doc/consensus_bridge_shim_audit.md" >"$historical_audit" 2>/dev/null; then
      if [ "$seen_budget" -eq 1 ] && [ -z "$stop_revision" ]; then
        break
      fi
      continue
    fi
    if extract_surface_budget bridge_lines "$historical_audit" >/dev/null 2>&1; then
      seen_budget=1
      if ! check_budget_ratchet "$history_current_audit" "$historical_audit"; then
        history_failed=1
      fi
    elif [ "$seen_budget" -eq 1 ] && [ -z "$stop_revision" ]; then
      break
    fi
  done
  if [ "$history_failed" -ne 0 ]; then
    return 1
  fi
}

check_surface_budgets() {
  repo_root="$1"
  audit_file="$2"
  ffi_file="$(mktemp)"
  cat "$repo_root/rust/crates/rustaxa-bridge/src/ffi.rs" \
    "$repo_root/rust/crates/rustaxa-bridge/src/application_host_ffi.rs" >"$ffi_file"
  bridge_root="$repo_root/rust/crates/rustaxa-bridge/src"
  shim_root="$repo_root/libraries/core_libs/consensus/shims"

  bridge_lines="$(find "$bridge_root" -type f -name '*.rs' -exec wc -l {} + |
    awk '$2 != "total" { total += $1 } END { print total + 0 }')"
  shim_lines="$(count_shim_source_lines "$shim_root")"
  cxx_functions="$(count_cxx_function_declarations "$ffi_file")"
  cxx_carriers="$(count_cxx_carriers "$ffi_file")"
  cxx_handles="$(count_cxx_opaque_handles "$ffi_file")"
  shim_directories="$(extract_shim_directories "$shim_root" | wc -l)"
  granular_flags="$(
    sed -n 's/^option(\(RUSTAXA_ENABLE_[A-Z_]*\).*/\1/p' "$repo_root/CMakeLists.txt" | sort -u | wc -l
  )"
  partial_service_factories="$(extract_partial_service_factories "$ffi_file" "$audit_file" | wc -l)"
  compatibility_constructor_calls="$(
    count_compatibility_constructor_calls "$ffi_file" "$audit_file" "$repo_root/libraries" "$repo_root/programs"
  )"
  non_test_cpp_consumers="$(
    {
      rg -l '#include [<"]rustaxa-bridge/(ffi|application_host_ffi)\.rs\.h[>"]' \
        "$repo_root/libraries" "$repo_root/programs" \
        --glob '*.cpp' --glob '*.cc' --glob '*.hpp' --glob '*.h' || true
    } | wc -l
  )"

  check_surface_metric bridge_lines "$bridge_lines" "$audit_file"
  check_surface_metric shim_lines "$shim_lines" "$audit_file"
  check_surface_metric cxx_functions "$cxx_functions" "$audit_file"
  check_surface_metric cxx_carriers "$cxx_carriers" "$audit_file"
  check_surface_metric cxx_handles "$cxx_handles" "$audit_file"
  check_surface_metric shim_directories "$shim_directories" "$audit_file"
  check_surface_metric granular_flags "$granular_flags" "$audit_file"
  check_surface_metric partial_service_factories "$partial_service_factories" "$audit_file"
  check_surface_metric compatibility_constructor_calls "$compatibility_constructor_calls" "$audit_file"
  check_surface_metric non_test_cpp_consumers "$non_test_cpp_consumers" "$audit_file"
  rm -f "$ffi_file"
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
  duplicate_types="$(mktemp)"
  trap 'rm -f "$ffi_types" "$audit_types" "$duplicate_types"' HUP INT TERM
  extract_ffi_bridge_types "$ffi_file" >"$ffi_types"
  extract_audited_bridge_types "$audit_file" >"$audit_types"
  uniq -d "$audit_types" >"$duplicate_types"
  if [ -s "$duplicate_types" ]; then
    echo "Rust bridge inventory guard failed: duplicate opaque handle rows:" >&2
    cat "$duplicate_types" >&2
    rm -f "$ffi_types" "$audit_types" "$duplicate_types"
    trap - HUP INT TERM
    return 1
  fi

  comm -23 "$ffi_types" "$audit_types" >"$missing_file"
  comm -13 "$ffi_types" "$audit_types" >"$stale_file"

  rm -f "$ffi_types" "$audit_types" "$duplicate_types"
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
  duplicate_values="$temp_dir/${inventory_name}_duplicates"
  "$live_extractor" "$live_file" >"$live_values"
  "$audit_extractor" "$audit_file" >"$audit_values"
  uniq -d "$audit_values" >"$duplicate_values"
  if [ -s "$duplicate_values" ]; then
    echo "Rust bridge inventory guard failed: duplicate $inventory_name audit entries:" >&2
    cat "$duplicate_values" >&2
    return 1
  fi
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
    echo "bridge inventory guard self-test failed: opaque handle mention outside handle table satisfied inventory" >&2
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

## Exported CXX Opaque Handles

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

## Exported CXX Opaque Handles

| Handle | Implementing module | Current consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeDocumented` | `module.rs` | test | External boundary | keep |
| `BridgeMissing` | `module.rs` | test | External boundary | keep |
| `BridgeStale` | `module.rs` | test | External boundary | keep |
EOF

  check_stale_exactly "$temp_dir/ffi.rs" "$temp_dir/audit_with_stale.md" BridgeStale "$temp_dir"

  sed '/BridgeDocumented.*module.rs/a | `BridgeDocumented` | `module.rs` | test | External boundary | keep |' \
    "$temp_dir/audit_with_missing.md" >"$temp_dir/audit_with_duplicate.md"
  if check_inventory "$temp_dir/ffi.rs" "$temp_dir/audit_with_duplicate.md" \
    "$temp_dir/missing" "$temp_dir/stale" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: duplicate opaque handle row was accepted" >&2
    exit 1
  fi

  cat >"$temp_dir/lib.rs" <<'EOF'
mod documented;
mod documented_block; /* retained block comment */
  pub mod documented_public;
pub(crate) mod documented_crate; // visible inside the crate
EOF
  mkdir -p "$temp_dir/shims/documented_shim"
  mkdir -p "$temp_dir/consumer_root/libraries" "$temp_dir/consumer_root/programs"
  cat >"$temp_dir/consumer_root/libraries/documented.cpp" <<'EOF'
#include "rustaxa-bridge/ffi.rs.h"
EOF
  cat >"$temp_dir/full_audit.md" <<'EOF'
# Audit

## Rust Bridge Modules

| Module | Main exported handles or constructors | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/documented.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_block.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_crate.rs` | helper | test | Internal Rust route | keep |
| `rust/crates/rustaxa-bridge/src/documented_public.rs` | helper | test | Internal Rust route | keep |

## Non-Test C++ Bridge Consumers

| Consumer path | Named client family | Removal condition |
| --- | --- | --- |
| `libraries/documented.cpp` | test | keep |

## Consensus Shim Directories

| Shim directory | Current role | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `documented_shim` | helper | test | C++ public compatibility facade | keep |
EOF
  check_documented_inventory module "$temp_dir/lib.rs" "$temp_dir/full_audit.md" \
    extract_bridge_modules extract_audited_bridge_modules "$temp_dir"
  check_documented_inventory consumer "$temp_dir/consumer_root" "$temp_dir/full_audit.md" \
    extract_non_test_cpp_consumers extract_audited_non_test_cpp_consumers "$temp_dir"
  check_documented_inventory shim "$temp_dir/shims" "$temp_dir/full_audit.md" \
    extract_shim_directories extract_audited_shim_directories "$temp_dir"

  sed '/libraries\/documented.cpp.*test/a | `libraries/documented.cpp` | test | keep |' \
    "$temp_dir/full_audit.md" >"$temp_dir/duplicate_consumer_audit.md"
  if check_documented_inventory consumer "$temp_dir/consumer_root" "$temp_dir/duplicate_consumer_audit.md" \
    extract_non_test_cpp_consumers extract_audited_non_test_cpp_consumers "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: duplicate consumer row was accepted" >&2
    exit 1
  fi

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
  cat >"$temp_dir/consumer_root/libraries/missing.cpp" <<'EOF'
#include <rustaxa-bridge/application_host_ffi.rs.h>
EOF
  if check_documented_inventory consumer "$temp_dir/consumer_root" "$temp_dir/full_audit.md" \
    extract_non_test_cpp_consumers extract_audited_non_test_cpp_consumers "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: undocumented consumer was accepted" >&2
    exit 1
  fi
  rm "$temp_dir/consumer_root/libraries/missing.cpp"

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

  mkdir "$temp_dir/production" "$temp_dir/tests"
  cat >"$temp_dir/caller_ffi.rs" <<'EOF'
#[cxx::bridge]
mod ffi {
    extern "Rust" {
        type ProductionHandle;
        pub fn production_export() -> bool;
        pub unsafe fn unsafe_production_export(
            pointer: *const u8,
        ) -> bool;
        pub fn test_only_export() -> bool;
        pub fn method_test_only_export(self: &ProductionHandle) -> bool;
    }
}
EOF
  cat >"$temp_dir/production/consumer.cpp" <<'EOF'
#include "rustaxa-bridge/ffi.rs.h"
void consume() {
  rustaxa::production_export();
  rustaxa::unsafe_production_export(nullptr);
}
struct FakeBridgeLookalike {
  bool method_test_only_export();
};
void unrelated(FakeBridgeLookalike& fake) { fake.method_test_only_export(); }
// rustaxa::test_only_export();
void test_only_export();
const char* ignored_call_text = "rustaxa::test_only_export()";
#if 0
void disabled() { rustaxa::test_only_export(); }
#endif
EOF
  cat >"$temp_dir/tests/consumer.cpp" <<'EOF'
#include "rustaxa-bridge/ffi.rs.h"
void test_consume(rustaxa::ProductionHandle& handle) {
  rustaxa::test_only_export();
  handle.method_test_only_export();
}
EOF
  cat >"$temp_dir/caller_audit.md" <<'EOF'
# Audit

## Test-Only CXX Export Allowlist

| Export | Named test client | Removal condition |
| --- | --- | --- |
| `method_test_only_export` | `tests/consumer.cpp` | remove |
| `test_only_export` | `tests/consumer.cpp` | remove |

## Checked Surface Budgets

| Metric | Exact budget |
| --- | ---: |
| `bridge_lines` | 3 |
EOF
  if [ "$(count_cxx_function_declarations "$temp_dir/caller_ffi.rs")" -ne 4 ]; then
    echo "bridge inventory guard self-test failed: multiline/unsafe CXX function was not counted" >&2
    exit 1
  fi
  if [ "$(count_cxx_opaque_handles "$temp_dir/caller_ffi.rs")" -ne 1 ]; then
    echo "bridge inventory guard self-test failed: non-Bridge opaque handle was not counted" >&2
    exit 1
  fi
  check_cxx_export_callers "$temp_dir/caller_ffi.rs" "$temp_dir/caller_audit.md" \
    "$temp_dir/production" "$temp_dir/missing_production_root" "$temp_dir/tests" "$temp_dir"
  sed '/test_only_export/d' "$temp_dir/caller_audit.md" >"$temp_dir/caller_audit_missing.md"
  if check_cxx_export_callers "$temp_dir/caller_ffi.rs" "$temp_dir/caller_audit_missing.md" \
    "$temp_dir/production" "$temp_dir/missing_production_root" "$temp_dir/tests" "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: unallowlisted test-only export was accepted" >&2
    exit 1
  fi
  check_surface_metric bridge_lines 3 "$temp_dir/caller_audit.md" >/dev/null
  if check_surface_metric bridge_lines 4 "$temp_dir/caller_audit.md" >/dev/null 2>&1; then
    echo "bridge inventory guard self-test failed: surface budget mismatch was accepted" >&2
    exit 1
  fi
  sed 's/| `bridge_lines` | 3 |/| `bridge_lines` | 4 |/' \
    "$temp_dir/caller_audit.md" >"$temp_dir/caller_audit_raised_budget.md"
  if check_budget_ratchet "$temp_dir/caller_audit_raised_budget.md" "$temp_dir/caller_audit.md" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: raised surface budget was accepted" >&2
    exit 1
  fi
  history_repo="$temp_dir/history_repo"
  mkdir -p "$history_repo/doc" "$temp_dir/history_checks"
  git -C "$history_repo" init -q
  git -C "$history_repo" config user.name "inventory self-test"
  git -C "$history_repo" config user.email "inventory-self-test@example.invalid"
  git -C "$history_repo" config commit.gpgsign false
  sed 's/| `bridge_lines` | 3 |/| `bridge_lines` | 100 |/' \
    "$temp_dir/caller_audit.md" >"$history_repo/doc/consensus_bridge_shim_audit.md"
  git -C "$history_repo" add doc/consensus_bridge_shim_audit.md
  git -C "$history_repo" commit -qm "budget 100"
  sed -i 's/| `bridge_lines` | 100 |/| `bridge_lines` | 50 |/' \
    "$history_repo/doc/consensus_bridge_shim_audit.md"
  git -C "$history_repo" commit -qam "budget 50"
  sed -i 's/| `bridge_lines` | 50 |/| `bridge_lines` | 60 |/' \
    "$history_repo/doc/consensus_bridge_shim_audit.md"
  git -C "$history_repo" commit -qam "budget 60"
  if check_budget_history "$history_repo/doc/consensus_bridge_shim_audit.md" \
    "$history_repo" HEAD "" "$temp_dir/history_checks" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: multi-commit budget re-increase was accepted" >&2
    exit 1
  fi

  cat >"$temp_dir/factory_ffi.rs" <<'EOF'
#[cxx::bridge]
mod ffi {
    extern "Rust" {
        type FactoryHandle;
        pub fn create_known_factory(input: &[u8; 32]) -> Box<FactoryHandle>;
        pub fn create_unknown_factory() -> Result<Box<FactoryHandle>>;
    }
}
EOF
  cat >"$temp_dir/factory_audit_missing.md" <<'EOF'
# Audit

## CXX Box Factory Inventory

| Factory | Classification | Named client or owner | Delete or narrow when |
| --- | --- | --- | --- |
| `create_known_factory` | Partial service | fixture | remove |
EOF
  if check_documented_inventory box_factory "$temp_dir/factory_ffi.rs" "$temp_dir/factory_audit_missing.md" \
    extract_cxx_box_factories extract_audited_cxx_box_factories "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: unknown Box factory was accepted" >&2
    exit 1
  fi
  cat >"$temp_dir/factory_audit_template.md" <<'EOF'
# Audit

## CXX Box Factory Inventory

| Factory | Classification | Named client or owner | Delete or narrow when |
| --- | --- | --- | --- |
| `create_known_factory` | Partial service | fixture | remove |
| `create_unknown_factory` | Production root debt | fixture | remove |

## Partial-Service Factory Inventory

| CXX factory | Compatibility constructor client path | Exact calls | Delete when |
| --- | --- | ---: | --- |
| `create_known_factory` | `production/consumer.cpp` | 1 | remove |
EOF
  sed "s#production/consumer.cpp#$temp_dir/production/consumer.cpp#" \
    "$temp_dir/factory_audit_template.md" >"$temp_dir/factory_audit.md"
  cat >"$temp_dir/production/consumer.cpp" <<'EOF'
#include "rustaxa-bridge/ffi.rs.h"
void consume() { rustaxa::create_known_factory({}); }
EOF
  check_documented_inventory box_factory "$temp_dir/factory_ffi.rs" "$temp_dir/factory_audit.md" \
    extract_cxx_box_factories extract_audited_cxx_box_factories "$temp_dir"
  check_partial_factory_sites "$temp_dir/factory_ffi.rs" "$temp_dir/factory_audit.md" \
    "$temp_dir/production" "$temp_dir/missing_production_root" "$temp_dir"
  cp "$temp_dir/production/consumer.cpp" "$temp_dir/production/consumer_once.cpp"
  echo 'void consume_again() { rustaxa::create_known_factory({}); }' >>"$temp_dir/production/consumer.cpp"
  if check_partial_factory_sites "$temp_dir/factory_ffi.rs" "$temp_dir/factory_audit.md" \
    "$temp_dir/production" "$temp_dir/missing_production_root" "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: duplicate partial-factory call was accepted" >&2
    exit 1
  fi
  mv "$temp_dir/production/consumer_once.cpp" "$temp_dir/production/consumer.cpp"
  sed 's#consumer.cpp#wrong.cpp#' \
    "$temp_dir/factory_audit.md" >"$temp_dir/factory_audit_wrong_site.md"
  if check_partial_factory_sites "$temp_dir/factory_ffi.rs" "$temp_dir/factory_audit_wrong_site.md" \
    "$temp_dir/production" "$temp_dir/missing_production_root" "$temp_dir" 2>/dev/null; then
    echo "bridge inventory guard self-test failed: wrong partial-factory call site was accepted" >&2
    exit 1
  fi

  echo "Rust bridge inventory guard self-test passed."
  exit 0
fi

missing_file="$(mktemp)"
stale_file="$(mktemp)"
ffi_inventory_file="$(mktemp)"
cat rust/crates/rustaxa-bridge/src/ffi.rs \
  rust/crates/rustaxa-bridge/src/application_host_ffi.rs >"$ffi_inventory_file"
trap 'rm -f "$missing_file" "$stale_file" "$ffi_inventory_file"' EXIT

check_inventory \
  "$ffi_inventory_file" \
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

Every exported CXX opaque handle must be classified in
doc/consensus_bridge_shim_audit.md before it is added or kept. Add an audit row
under "Exported CXX Opaque Handles" with current consumers, classification, and
a deletion or narrowing condition.

Missing audit entries:
EOF
  cat "$missing_file" >&2
  exit 1
fi

inventory_temp_dir="$(mktemp -d)"
trap 'rm -f "$missing_file" "$stale_file" "$ffi_inventory_file"; rm -rf "$inventory_temp_dir"' EXIT

check_documented_inventory \
  module rust/crates/rustaxa-bridge/src/lib.rs doc/consensus_bridge_shim_audit.md \
  extract_bridge_modules extract_audited_bridge_modules "$inventory_temp_dir"
check_documented_inventory \
  consumer "$repo_root" doc/consensus_bridge_shim_audit.md \
  extract_non_test_cpp_consumers extract_audited_non_test_cpp_consumers "$inventory_temp_dir"
check_documented_inventory \
  shim libraries/core_libs/consensus/shims doc/consensus_bridge_shim_audit.md \
  extract_shim_directories extract_audited_shim_directories "$inventory_temp_dir"
check_documented_inventory \
  box_factory "$ffi_inventory_file" doc/consensus_bridge_shim_audit.md \
  extract_cxx_box_factories extract_audited_cxx_box_factories "$inventory_temp_dir"
check_factory_inventory_rows doc/consensus_bridge_shim_audit.md
check_partial_factory_sites \
  "$ffi_inventory_file" doc/consensus_bridge_shim_audit.md \
  libraries programs "$inventory_temp_dir"
check_cxx_export_callers \
  "$ffi_inventory_file" doc/consensus_bridge_shim_audit.md \
  libraries programs tests "$inventory_temp_dir"
check_surface_budgets "$repo_root" "$repo_root/doc/consensus_bridge_shim_audit.md"

if ! git diff HEAD --quiet -- doc/consensus_bridge_shim_audit.md; then
  local_budget_base="HEAD"
else
  local_budget_base="HEAD^"
fi

target_ref="$base_ref"
if [ -z "$target_ref" ] && [ -n "${RUSTAXA_INVENTORY_BASE_REF:-}" ]; then
  target_ref="$RUSTAXA_INVENTORY_BASE_REF"
fi
if [ -z "$target_ref" ] && [ -n "${GITHUB_BASE_REF:-}" ]; then
  target_ref="origin/$GITHUB_BASE_REF"
fi
if [ -z "$target_ref" ] && git rev-parse --verify origin/main >/dev/null 2>&1; then
  target_ref="origin/main"
fi

target_budget_base=""
if [ -n "$target_ref" ]; then
  if ! target_budget_base="$(git merge-base HEAD "$target_ref" 2>/dev/null)"; then
    echo "Rust bridge inventory guard failed: cannot resolve target/base revision $target_ref." >&2
    exit 1
  fi
fi

if [ -n "$target_budget_base" ]; then
  target_audit_file="$inventory_temp_dir/target_base_audit.md"
  if git show "$target_budget_base:doc/consensus_bridge_shim_audit.md" >"$target_audit_file" 2>/dev/null; then
    check_budget_ratchet doc/consensus_bridge_shim_audit.md "$target_audit_file"
  fi
fi
check_budget_history \
  doc/consensus_bridge_shim_audit.md "$repo_root" "$local_budget_base" "$target_budget_base" "$inventory_temp_dir"

echo "Rust bridge inventory guard passed."

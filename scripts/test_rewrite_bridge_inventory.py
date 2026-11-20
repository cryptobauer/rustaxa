#!/usr/bin/env python3
"""Check complete CXX inventories and reject lexical decoys."""
import tempfile
import os
import shutil
import subprocess
import unittest
from pathlib import Path

import rewrite_bridge_inventory as inventory


class InventoryTests(unittest.TestCase):
    def module(self, body):
        return inventory.cxx_module("#[cxx::bridge] mod bridge {" + body + "}")

    def test_rust_exports_and_callbacks(self):
        module = self.module('''unsafe extern "Rust" {
            fn private_export(value_fn: u64);
            pub fn public_export();
            unsafe fn unsafe_export();
            pub unsafe fn factory(
                value: &'static str
            ) -> Box<Owner>;
            fn method(self: &Owner);
            fn borrowed<'a>(value: &'a str) -> &'a str;
            type Owner;
        }
        unsafe extern "C++" {
            fn callback();
            pub fn public_callback() -> Box<Callback>;
            type Callback;
        }
        struct Shared { value: u64 }
        ''')
        self.assertEqual([name for name, _ in inventory.ffi_functions(module)],
                         ["private_export", "public_export", "unsafe_export", "factory", "method", "borrowed"])
        self.assertEqual(inventory.ffi_method_receivers(module), {"method": "Owner"})
        self.assertIn("type Callback;", module)
        self.assertIn("struct Shared", module)

    def test_lexical_decoys(self):
        source = r'''/* outer /* #[cxx::bridge] mod fake {} */ extern "Rust" {} */
        const DECOY: &str = r###"#[cxx::bridge] mod fake { extern "Rust" { fn fake(); } }"###;
        const ESCAPED: &str = "escaped \" #[cxx::bridge] { }";
        #[cxx::bridge(namespace = "extern Rust { fn fake(); }")]
        mod bridge {
            extern /* comment */ "Rust" {
                #[doc = r#"fn fake(); } extern "Rust" {"#]
                fn actual(value: &'static str);
            }
        }
        '''
        self.assertEqual([name for name, _ in inventory.ffi_functions(inventory.cxx_module(source))],
                         ["actual"])

    def test_malformed_blocks(self):
        for source in [
            '#[cxx::bridge] mod bridge {',
            '#[cxx::bridge] mod bridge { extern "Rust" { fn missing() } }',
            '#[cxx::bridge] mod bridge { extern "Rust" { fn first() fn second(); } }',
            '#[cxx::bridge] mod bridge { extern "Other" { fn unknown(); } }',
            '#[cxx::bridge] mod bridge { extern { fn unknown(); } }',
            '#[cxx::bridge] mod bridge { extern "Rust" { fn unsupported<; } }',
            '#[cxx::bridge] mod bridge { extern "Rust" { fn unclosed(); }',
        ]:
            with self.subTest(source=source), self.assertRaises(ValueError):
                inventory.ffi_functions(inventory.cxx_module(source))

    def test_third_module_and_inline_bridge(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lib = root / "lib.rs"
            lib.write_text('mod first;\nmod second;\nmod third;\nmod ordinary;\n'
                           '#[cxx::bridge] mod inline { extern "Rust" { fn inline_export(); } }')
            for name in ("first", "second", "third", "unreferenced"):
                (root / (name + ".rs")).write_text(
                    '#[cxx::bridge] mod bridge { extern "Rust" {\n'
                    f'type {name.title()};\nfn {name}() -> Box<{name.title()}>;\n'
                    '}\nstruct ' + name.title() + 'Carrier { value: u64 }\n}')
            (root / "ordinary.rs").write_text('pub fn ordinary() {}')
            module = inventory.cxx_module(inventory.cxx_sources(lib))
            self.assertEqual({name for name, _ in inventory.ffi_functions(module)},
                             {"first", "second", "third", "inline_export"})
            self.assertIn("type Third;", module)
            self.assertIn("struct ThirdCarrier", module)
            self.assertNotIn("Unreferenced", module)
            (root / "third.rs").write_text('#[cxx::bridge] mod broken {')
            with self.assertRaises(ValueError):
                inventory.cxx_sources(lib)

    def test_consumer_includes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, source in {
                "first.cpp": '#include "rustaxa-bridge/third.rs.h"\n#include <rustaxa-bridge/third.rs.h>',
                "second.hpp": '  #  include <rustaxa-bridge/src/fourth.rs.h>',
                "decoy.cpp": '''// #include "rustaxa-bridge/hidden.rs.h"
                /* #include "rustaxa-bridge/hidden.rs.h" */
                const char* hidden = "#include <rustaxa-bridge/hidden.rs.h>";
                #if 0
                #include "rustaxa-bridge/hidden.rs.h"
                #endif
                #include "other-bridge/third.rs.h"
                const char* raw = R"marker(
                #include "rustaxa-bridge/hidden.rs.h"
                )marker";
                ''',
            }.items():
                (root / name).write_text(source)
            self.assertEqual(inventory.cpp_consumers([root]),
                             [(root / name).as_posix() for name in ("first.cpp", "second.hpp")])

    def test_mask_positions(self):
        source = '#if 0\nignored\n#endif\nvisible\n'
        masked = inventory.strip_inactive_if_zero(source)
        self.assertEqual(len(masked), len(source))
        self.assertEqual(masked.index("visible"), source.index("visible"))
        rust = 'const S: &str = r##"fake"##; /* nested /* x */ y */ mod actual;'
        self.assertEqual(len(inventory.rust_code(rust)), len(rust))
        self.assertEqual(inventory.rust_code(rust).index("mod actual"), rust.index("mod actual"))

    def test_complete_guard_collects_third_module(self):
        root = Path(tempfile.mkdtemp(prefix="bridge-inventory-fixture-"))
        print("Retained bridge inventory fixture:", root)
        scripts = root / "scripts"
        scripts.mkdir()
        for name in ("rewrite_bridge_inventory.py", "rewrite_bridge_inventory_guard.sh"):
            shutil.copyfile(Path(__file__).parent / name, scripts / name)
        bridge = root / "rust/crates/rustaxa-bridge/src"
        bridge.mkdir(parents=True)
        (bridge / "lib.rs").write_text("mod first;\nmod second;\nmod third;\n")
        for name in ("first", "second", "third"):
            (bridge / (name + ".rs")).write_text(
                '#[ cxx :: bridge ]\nmod bridge {\nextern "Rust" {\n'
                f'type {name.title()};\nfn {name}() -> Box<{name.title()}>;\n'
                '}\nstruct ' + name.title() + 'Carrier { value: u64 }\n}\n')
        (root / "libraries").mkdir()
        (root / "programs").mkdir()
        (root / "tests").mkdir()
        (root / "libraries/core_libs/consensus/shims").mkdir(parents=True)
        (root / "libraries/caller.cpp").write_text(
            '#include "rustaxa-bridge/src/third.rs.h"\n'
            'void caller() { rustaxa::first(); rustaxa::second(); rustaxa::third(); }\n')
        (root / "CMakeLists.txt").write_text("")
        (root / "doc").mkdir()
        audit = root / "doc/consensus_bridge_shim_audit.md"
        rows = ["## Exported CXX Opaque Handles"]
        rows += [f"| `{name.title()}` | fixture |" for name in ("first", "second", "third")]
        rows += ["## Rust Bridge Modules"]
        rows += [f"| `rust/crates/rustaxa-bridge/src/{name}.rs` | fixture |"
                 for name in ("first", "second", "third")]
        rows += ["## Non-Test C++ Bridge Consumers", "| `libraries/caller.cpp` | fixture |",
                 "## Consensus Shim Directories", "## CXX Box Factory Inventory"]
        rows += [f"| `{name}` | Supported boundary | fixture | keep |"
                 for name in ("first", "second", "third")]
        rows += ["## Partial-Service Factory Inventory", "## Test-Only CXX Export Allowlist",
                 "## Checked Surface Budgets"]
        metrics = dict(bridge_lines=27, shim_lines=0, cxx_functions=3, cxx_carriers=3,
                       cxx_handles=3, shim_directories=0, granular_flags=0,
                       partial_service_factories=0, compatibility_constructor_calls=0,
                       non_test_cpp_consumers=1)
        rows += [f"| `{name}` | {count} |" for name, count in metrics.items()]
        original = "\n".join(rows) + "\n"
        audit.write_text(original)
        env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        for name in list(env):
            if name.startswith("GIT_") and name not in {"GIT_CONFIG_NOSYSTEM", "GIT_CONFIG_GLOBAL"}:
                del env[name]
        def run(argv):
            result = subprocess.run(argv, cwd=root, env=env, text=True, capture_output=True,
                                    timeout=30)
            print("COMMAND", argv, "EXIT", result.returncode)
            print(result.stdout, end="")
            print(result.stderr, end="")
            return result
        for argv in [
            ["git", "init", "-q"], ["git", "add", "scripts", "rust", "libraries", "doc", "CMakeLists.txt"],
            ["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
             "-c", "commit.gpgsign=false", "commit", "-qm", "Inventory fixture"],
        ]:
            self.assertEqual(run(argv).returncode, 0)
        argv = ["sh", "scripts/rewrite_bridge_inventory_guard.sh", "--base-ref", "HEAD"]
        audit.write_text(original + "\n")
        result = run(argv)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("cxx_functions=3", result.stdout)
        self.assertIn("cxx_handles=3", result.stdout)
        self.assertIn("cxx_carriers=3", result.stdout)
        audit.write_text(original.replace("| `Third` | fixture |\n", ""))
        result = run(argv)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Third", result.stderr)
        audit.write_text(original.replace("| `cxx_functions` | 3 |", "| `cxx_functions` | 2 |"))
        result = run(argv)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cxx_functions is 3; checked budget is 2", result.stderr)


if __name__ == "__main__":
    unittest.main()

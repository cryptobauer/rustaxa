#!/usr/bin/env python3
"""Rust-mode closeout test overlay; original upstream C++ stays intact."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
root, output = args.source.resolve(), args.output.resolve()
assert output.is_relative_to(Path('/build')) and output != Path('/build')
rows = []
paths = [p for p in (root/'tests/test_util').rglob('*') if p.name != 'AI.md' and p.is_file()]
paths.append(root/'tests/rpc_test.cpp')
for path in paths:
    relative = path.relative_to(root/'tests')
    raw = path.read_bytes()
    effective = raw
    if relative.as_posix() in ('test_util/gtest.hpp', 'test_util/src/test_util.cpp', 'rpc_test.cpp'):
        text = raw.decode()
        if relative.as_posix() == 'test_util/gtest.hpp':
            text = text.replace('#pragma once', '#pragma once\n#include "'+str(root/'tests/rust/closeout_test_scratch.hpp')+'"', 1)
            old = 'std::filesystem::path data_dir = std::filesystem::temp_directory_path() / "taraxa_node_tests" /\n                                   current_test_info->test_suite_name() / current_test_info->name();'
            assert old in text
            text = text.replace(old, 'std::filesystem::path data_dir = rustaxa_closeout::active() ? rustaxa_closeout::path("taraxa_node_test") : std::filesystem::temp_directory_path() / "taraxa_node_tests" / current_test_info->test_suite_name() / current_test_info->name();')
            text = text.replace('std::filesystem::remove_all(data_dir);', 'if (!rustaxa_closeout::active()) std::filesystem::remove_all(data_dir);', 1)
        elif relative.as_posix() == 'test_util/src/test_util.cpp':
            assert 'void NodesTest::CleanupDirs() {' in text
            text = text.replace('void NodesTest::CleanupDirs() {', 'void NodesTest::CleanupDirs() {\n  if (rustaxa_closeout::active()) return;')
        else:
            old = 'const auto test_data_dir =\n        std::filesystem::temp_directory_path() / "taraxa_node_tests" / test_info->test_suite_name() / test_info->name();'
            assert old in text
            text = text.replace(old, 'const auto test_data_dir = rustaxa_closeout::active() ? rustaxa_closeout::path("rpc_isolated_node_test") : std::filesystem::temp_directory_path() / "taraxa_node_tests" / test_info->test_suite_name() / test_info->name();')
            text = text.replace('std::filesystem::remove_all(test_data_dir);', 'if (!rustaxa_closeout::active()) std::filesystem::remove_all(test_data_dir);')
        effective = text.encode()
    target = output/relative
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists() or target.read_bytes() != effective:
        target.write_bytes(effective)
    rows.append({'source':str(path),'source_sha256':hashlib.sha256(raw).hexdigest(),'effective':str(target),'effective_sha256':hashlib.sha256(effective).hexdigest(),'bytes':len(effective)})
manifest = output/'manifest.json'
encoded = (json.dumps(rows, sort_keys=True, indent=2)+'\n').encode()
if not manifest.exists() or manifest.read_bytes() != encoded:
    manifest.write_bytes(encoded)

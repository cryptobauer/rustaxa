#!/usr/bin/env python3
"""Parse Rust CXX declarations and concrete C++ bridge call syntax.

The inventory guard uses this helper instead of lexical identifier searches.
Inputs are repository source files or roots; outputs are sorted newline records.
Comments and string/character literals are removed before parsing, so prose,
forward declarations without a bridge qualifier, and unrelated same-named
functions do not establish a production bridge caller.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path
from typing import Iterable


CPP_SUFFIXES = frozenset({".cc", ".cpp", ".h", ".hpp"})


def strip_inactive_if_zero(source: str) -> str:
    """Blank code disabled by literal `#if 0` while preserving other branches.

    Other preprocessor conditions remain visible because they may be active in
    supported builds. Nested conditionals inside a disabled branch remain
    disabled; its `#else` branch becomes visible.
    """

    output: list[str] = []
    frames: list[tuple[bool, bool]] = []
    active = True
    for line in source.splitlines(keepends=True):
        directive = re.match(r"^\s*#\s*(if|ifdef|ifndef|else|elif|endif)\b(.*)", line)
        if directive:
            kind = directive.group(1)
            expression = directive.group(2).strip()
            if kind in {"if", "ifdef", "ifndef"}:
                literal_zero = kind == "if" and expression == "0"
                frames.append((active, literal_zero))
                active = active and not literal_zero
            elif kind in {"else", "elif"} and frames:
                parent_active, literal_zero = frames[-1]
                if literal_zero and kind == "elif":
                    active = parent_active and expression != "0"
                else:
                    active = parent_active
            elif kind == "endif" and frames:
                parent_active, _ = frames.pop()
                active = parent_active
            output.append("\n" if line.endswith("\n") else "")
            continue
        output.append(line if active else ("\n" if line.endswith("\n") else ""))
    return "".join(output)


def strip_comments_and_literals(source: str) -> str:
    """Return source code with comments and quoted literals replaced by spaces.

    Newlines are preserved so diagnostics and record paths stay stable. Nested
    block comments are supported for Rust; malformed trailing comments or
    literals consume the remaining input rather than exposing their contents as
    executable syntax.
    """

    source = strip_inactive_if_zero(source)
    output: list[str] = []
    index = 0
    block_depth = 0
    quote: str | None = None
    escaped = False
    while index < len(source):
        current = source[index]
        following = source[index + 1] if index + 1 < len(source) else ""
        if block_depth:
            if current == "/" and following == "*":
                block_depth += 1
                output.extend((" ", " "))
                index += 2
                continue
            if current == "*" and following == "/":
                block_depth -= 1
                output.extend((" ", " "))
                index += 2
                continue
            output.append("\n" if current == "\n" else " ")
            index += 1
            continue
        if quote is not None:
            if current == "\n" and quote != "`":
                quote = None
                escaped = False
                output.append("\n")
                index += 1
                continue
            output.append("\n" if current == "\n" else " ")
            if escaped:
                escaped = False
            elif current == "\\":
                escaped = True
            elif current == quote:
                quote = None
            index += 1
            continue
        if current == "/" and following == "/":
            while index < len(source) and source[index] != "\n":
                output.append(" ")
                index += 1
            continue
        if current == "/" and following == "*":
            block_depth = 1
            output.extend((" ", " "))
            index += 2
            continue
        if current in {'"', "'", "`"}:
            quote = current
            output.append(" ")
            index += 1
            continue
        output.append(current)
        index += 1
    return "".join(output)


def cxx_module(source: str) -> str:
    """Return the stripped bodies of all CXX bridge modules.

    Module boundaries are brace-matched after comments and literals are
    removed. Missing markers or unbalanced braces fail closed. Joining the
    bodies lets one inventory source concatenate independently generated CXX
    translation units without hiding later declarations.
    """

    stripped = strip_comments_and_literals(source)
    bodies: list[str] = []
    cursor = 0
    while (marker := stripped.find("#[cxx::bridge", cursor)) >= 0:
        opening = stripped.find("{", marker)
        if opening < 0:
            raise ValueError("CXX bridge module opening brace is missing")
        depth = 0
        for index in range(opening, len(stripped)):
            if stripped[index] == "{":
                depth += 1
            elif stripped[index] == "}":
                depth -= 1
                if depth == 0:
                    bodies.append(stripped[opening + 1 : index])
                    cursor = index + 1
                    break
        else:
            raise ValueError("CXX bridge module braces are unbalanced")
    if not bodies:
        raise ValueError("CXX bridge marker is missing")
    return "\n".join(bodies)


def ffi_functions(module: str) -> list[tuple[str, str]]:
    """Return every CXX Rust function name and its complete declaration.

    Declarations are collected through their terminating semicolon, preserving
    overloads as separate records. A missing semicolon is a hard parse error.
    """

    records: list[tuple[str, str]] = []
    pattern = re.compile(r"(?m)^\s*pub\s+(?:unsafe\s+)?fn\s+([A-Za-z_]\w*)\s*\(")
    for match in pattern.finditer(module):
        round_depth = 0
        square_depth = 0
        curly_depth = 0
        terminating = -1
        for index in range(match.end() - 1, len(module)):
            character = module[index]
            if character == "(":
                round_depth += 1
            elif character == ")":
                round_depth -= 1
            elif character == "[":
                square_depth += 1
            elif character == "]":
                square_depth -= 1
            elif character == "{":
                curly_depth += 1
            elif character == "}":
                curly_depth -= 1
            elif character == ";" and round_depth == square_depth == curly_depth == 0:
                terminating = index
                break
        if terminating < 0:
            raise ValueError(f"CXX function {match.group(1)} has no terminating semicolon")
        records.append((match.group(1), module[match.start() : terminating + 1]))
    return records


def cpp_files(roots: Iterable[Path]) -> Iterable[Path]:
    """Yield source/header files below roots in stable path order."""

    files: set[Path] = set()
    for root in roots:
        if not root.exists():
            continue
        if root.is_file() and root.suffix in CPP_SUFFIXES:
            files.add(root)
        elif root.is_dir():
            files.update(path for path in root.rglob("*") if path.suffix in CPP_SUFFIXES)
    yield from sorted(files)


def ffi_method_receivers(module: str) -> dict[str, str]:
    """Return generated C++ method names mapped to opaque receiver types."""

    receivers: dict[str, str] = {}
    receiver_pattern = re.compile(r"\bself\s*:\s*&(?:mut\s+)?([A-Za-z_]\w*)")
    for name, declaration in ffi_functions(module):
        receiver = receiver_pattern.search(declaration)
        if receiver:
            receivers[name] = receiver.group(1)
    return receivers


def cpp_bridge_calls(source: str) -> tuple[list[tuple[str, int]], list[tuple[str, int]]]:
    """Return bridge-shaped free-function and method-call candidates with offsets.

    Free functions must use the `rustaxa::name(` form. Methods must use
    `.name(` or `->name(`. Ordinary declarations and unqualified calls do not
    count, and comments/literals/inactive `#if 0` regions are removed.
    """

    stripped = strip_comments_and_literals(source)
    free_calls = [
        (match.group(1), match.start(1))
        for match in re.finditer(r"\brustaxa\s*::\s*([A-Za-z_]\w*)\s*\(", stripped)
    ]
    method_calls = [
        (match.group(1), match.start(1))
        for match in re.finditer(r"(?:\.|->)\s*([A-Za-z_]\w*)\s*\(", stripped)
    ]
    return free_calls, method_calls


def include_context(
    path: Path,
    source: str,
    files_by_name: dict[str, list[Path]],
) -> str:
    """Return source plus recursively included repository headers.

    Transitive header context proves that an opaque CXX receiver type is in
    scope for wrapper implementations while avoiding an unrelated same-named
    method elsewhere in the tree. Ambiguous overlay/upstream include suffixes
    are included conservatively.
    """

    context: list[str] = []
    pending: list[tuple[Path, str]] = [(path, source)]
    visited: set[Path] = set()
    while pending:
        current_path, current_source = pending.pop()
        if current_path in visited:
            continue
        visited.add(current_path)
        context.append(current_source)
        includes = re.findall(r'(?m)^\s*#\s*include\s*[<"]([^>"]+)[>"]', current_source)
        for include in includes:
            for match in files_by_name.get(Path(include).name, []):
                if match.as_posix().endswith(include) and match not in visited:
                    pending.append(
                        (match, match.read_text(encoding="utf-8", errors="replace"))
                    )
    return "\n".join(context)


def main() -> None:
    """Parse CLI arguments and emit the requested deterministic inventory."""

    parser = argparse.ArgumentParser()
    parser.add_argument(
        "mode",
        choices=(
            "ffi-functions",
            "ffi-function-count",
            "ffi-carriers",
            "ffi-handles",
            "ffi-factories",
            "cpp-calls",
            "cpp-call-sites",
        ),
    )
    parser.add_argument("--ffi", type=Path)
    parser.add_argument("paths", nargs="+", type=Path)
    args = parser.parse_args()

    if args.mode.startswith("ffi-"):
        if len(args.paths) != 1:
            parser.error(f"{args.mode} accepts exactly one FFI source")
        module = cxx_module(args.paths[0].read_text(encoding="utf-8"))
        functions = ffi_functions(module)
        if args.mode == "ffi-functions":
            print("\n".join(sorted({name for name, _ in functions})))
        elif args.mode == "ffi-function-count":
            print(len(functions))
        elif args.mode == "ffi-carriers":
            names = re.findall(r"(?m)^\s*(?:struct|enum)\s+([A-Za-z_]\w*)\b", module)
            print("\n".join(sorted(set(names))))
        elif args.mode == "ffi-handles":
            names = re.findall(r'(?m)^\s*type\s+([A-Za-z_]\w*)\s*;', module)
            print("\n".join(sorted(set(names))))
        else:
            factories = {name for name, declaration in functions if re.search(r"->[^;]*\bBox\s*<", declaration)}
            print("\n".join(sorted(factories)))
        return

    if args.ffi is None:
        parser.error(f"{args.mode} requires --ffi")
    module = cxx_module(args.ffi.read_text(encoding="utf-8"))
    exported_functions = {name for name, _ in ffi_functions(module)}
    method_receivers = ffi_method_receivers(module)
    files = list(cpp_files(args.paths))
    files_by_name: dict[str, list[Path]] = {}
    for path in files:
        files_by_name.setdefault(path.name, []).append(path)
    records: set[str] = set()
    for path in files:
        source = path.read_text(encoding="utf-8", errors="replace")
        free_candidates, method_candidates = cpp_bridge_calls(source)
        accepted: list[tuple[str, int]] = [
            candidate for candidate in free_candidates if candidate[0] in exported_functions
        ]
        context = strip_comments_and_literals(include_context(path, source, files_by_name))
        for name, offset in method_candidates:
            if name not in method_receivers:
                continue
            if re.search(rf"\b{re.escape(method_receivers[name])}\b", context):
                accepted.append((name, offset))
        if args.mode == "cpp-calls":
            records.update(name for name, _ in accepted)
        else:
            for name, offset in accepted:
                line = source.count("\n", 0, offset) + 1
                prior_newline = source.rfind("\n", 0, offset)
                column = offset - prior_newline
                records.add(f"{name}\t{path.as_posix()}\t{line}\t{column}")
    print("\n".join(sorted(records)))


if __name__ == "__main__":
    main()

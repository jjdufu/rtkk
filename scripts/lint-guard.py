#!/usr/bin/env python3
"""G1 Static Guard: Rust toolchain, Cargo.toml lint lock-table, and .rs zero-bypass scanner."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

REQUIRED_RUST_LINTS = {
    "unsafe_code": "forbid",
    "warnings": "deny",
    "missing_docs": "deny",
}

REQUIRED_CLIPPY_LINTS = {
    "all": "deny",
    "pedantic": "deny",
    "allow_attributes": "deny",
    "unwrap_used": "deny",
    "expect_used": "deny",
    "todo": "deny",
    "unimplemented": "deny",
    "panic": "deny",
    "as_conversions": "deny",
    "indexing_slicing": "deny",
}

PRINT_WHITELIST = {
    "src/main.rs",
    "src/cli.rs",
    "src/logging/mod.rs",
    "src/logging/progress.rs",
    "src/playbook/engine.rs",
}

FORBIDDEN_CODE_PATTERNS = [
    ("unsafe-keyword", re.compile(r"\bunsafe\b")),
    ("allow-attr", re.compile(r"#\s*!?\s*\[\s*allow\b")),
    ("expect-attr", re.compile(r"#\s*!?\s*\[\s*expect\b")),
    ("warn-attr", re.compile(r"#\s*!?\s*\[\s*warn\b")),
    ("dbg-macro", re.compile(r"(?<![A-Za-z0-9_:])(?:std::)?dbg\s*!")),
]

BARE_PRINT_PATTERN = re.compile(r"(?<![A-Za-z0-9_:])(?:std::)?e?println\s*!")


def strip_rust_comments_and_strings(src: str) -> str:
    """Replace Rust comments and string/char literals with spaces while preserving newlines."""
    out: list[str] = []
    n = len(src)
    i = 0
    while i < n:
        ch = src[i]
        nxt = src[i + 1] if i + 1 < n else ""

        # Line comment
        if ch == "/" and nxt == "/":
            while i < n and src[i] != "\n":
                out.append(" ")
                i += 1
            continue

        # Nested block comment
        if ch == "/" and nxt == "*":
            depth = 1
            out.extend((" ", " "))
            i += 2
            while i < n and depth > 0:
                c0 = src[i]
                c1 = src[i + 1] if i + 1 < n else ""
                if c0 == "/" and c1 == "*":
                    depth += 1
                    out.extend((" ", " "))
                    i += 2
                elif c0 == "*" and c1 == "/":
                    depth -= 1
                    out.extend((" ", " "))
                    i += 2
                else:
                    out.append("\n" if c0 == "\n" else " ")
                    i += 1
            continue

        # Raw byte/string literal: r#"..."#, br#"..."#, cr#"..."#
        if ch in ("r", "b", "c"):
            j = i
            if src[j] in ("b", "c") and j + 1 < n and src[j + 1] == "r":
                j += 1
            if src[j] == "r":
                k = j + 1
                hashes = 0
                while k < n and src[k] == "#":
                    hashes += 1
                    k += 1
                if k < n and src[k] == '"':
                    while i <= k:
                        out.append(" ")
                        i += 1
                    closing = '"' + ("#" * hashes)
                    while i < n:
                        if src.startswith(closing, i):
                            for _ in closing:
                                out.append(" ")
                            i += len(closing)
                            break
                        out.append("\n" if src[i] == "\n" else " ")
                        i += 1
                    continue

        # Normal string or byte string literal
        if ch == '"' or (ch in ("b", "c") and nxt == '"'):
            if ch in ("b", "c"):
                out.append(" ")
                i += 1
            out.append(" ")
            i += 1
            while i < n:
                c0 = src[i]
                if c0 == "\\":
                    out.append(" ")
                    i += 1
                    if i < n:
                        out.append("\n" if src[i] == "\n" else " ")
                        i += 1
                    continue
                if c0 == '"':
                    out.append(" ")
                    i += 1
                    break
                out.append("\n" if c0 == "\n" else " ")
                i += 1
            continue

        # Character literal (avoid confusing Rust lifetimes like 'a or 'static)
        if ch == "'":
            if i + 2 < n and src[i + 1] != "\\" and src[i + 2] == "'":
                out.extend((" ", " ", " "))
                i += 3
                continue
            if i + 3 < n and src[i + 1] == "\\":
                end_q = src.find("'", i + 2)
                if 0 < end_q <= i + 6:
                    while i <= end_q:
                        out.append(" ")
                        i += 1
                    continue

        out.append(ch)
        i += 1

    return "".join(out)


def parse_toml_sections(text: str) -> dict[str, dict[str, str]]:
    sections: dict[str, dict[str, str]] = {"": {}}
    current = ""
    for raw_line in text.splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("[") and line.endswith("]") and not line.startswith("[["):
            current = line[1:-1].strip()
            sections.setdefault(current, {})
            continue
        if "=" in line:
            k, v = line.split("=", 1)
            sections.setdefault(current, {})[k.strip()] = v.strip()
    return sections


def extract_lint_level(raw_val: str) -> str:
    m_direct = re.match(r'^"([^"]+)"$', raw_val)
    if m_direct:
        return m_direct.group(1)
    m_table = re.search(r'level\s*=\s*"([^"]+)"', raw_val)
    if m_table:
        return m_table.group(1)
    return raw_val


def check_manifests(errors: list[str]) -> None:
    tc_path = REPO_ROOT / "rust-toolchain.toml"
    cargo_path = REPO_ROOT / "Cargo.toml"
    if not tc_path.is_file():
        errors.append("rust-toolchain.toml: missing file")
        return
    if not cargo_path.is_file():
        errors.append("Cargo.toml: missing file")
        return

    tc_sections = parse_toml_sections(tc_path.read_text(encoding="utf-8"))
    channel = tc_sections.get("toolchain", {}).get("channel", "")
    if channel != '"1.100.0"':
        errors.append(f'rust-toolchain.toml: expected channel = "1.100.0", got {channel or "<missing>"}')

    cargo_sections = parse_toml_sections(cargo_path.read_text(encoding="utf-8"))
    ws_pkg = cargo_sections.get("workspace.package", {})
    if ws_pkg.get("edition") != '"2024"':
        errors.append(f'Cargo.toml [workspace.package]: expected edition = "2024", got {ws_pkg.get("edition")}')
    if ws_pkg.get("rust-version") != '"1.100.0"':
        errors.append(
            f'Cargo.toml [workspace.package]: expected rust-version = "1.100.0", got {ws_pkg.get("rust-version")}'
        )

    rust_lints = cargo_sections.get("workspace.lints.rust", {})
    for key, expected in REQUIRED_RUST_LINTS.items():
        actual = extract_lint_level(rust_lints.get(key, ""))
        if actual != expected:
            errors.append(f'Cargo.toml [workspace.lints.rust]: expected {key} = "{expected}", got {actual!r}')

    clippy_lints = cargo_sections.get("workspace.lints.clippy", {})
    for key, expected in REQUIRED_CLIPPY_LINTS.items():
        actual = extract_lint_level(clippy_lints.get(key, ""))
        if actual != expected:
            errors.append(f'Cargo.toml [workspace.lints.clippy]: expected {key} = "{expected}", got {actual!r}')


def scan_rust_sources(errors: list[str]) -> int:
    targets: list[Path] = []
    for sub in ("src", "test"):
        base = REPO_ROOT / sub
        if base.is_dir():
            targets.extend(sorted(base.rglob("*.rs")))
    build_rs = REPO_ROOT / "build.rs"
    if build_rs.is_file():
        targets.append(build_rs)

    for path in targets:
        rel = path.relative_to(REPO_ROOT).as_posix()
        raw = path.read_text(encoding="utf-8")
        sanitized = strip_rust_comments_and_strings(raw)
        for lineno, line in enumerate(sanitized.splitlines(), 1):
            for rule_name, regex in FORBIDDEN_CODE_PATTERNS:
                m = regex.search(line)
                if m:
                    errors.append(f"{rel}:{lineno}: [{rule_name}] forbidden token {m.group(0)!r}")
            if rel.startswith("src/") and rel not in PRINT_WHITELIST:
                m_print = BARE_PRINT_PATTERN.search(line)
                if m_print:
                    errors.append(
                        f"{rel}:{lineno}: [bare-println] forbidden {m_print.group(0)!r} outside CLI terminal whitelist"
                    )
    return len(targets)


def main() -> int:
    errors: list[str] = []
    check_manifests(errors)
    scanned_count = scan_rust_sources(errors)
    if errors:
        for err in errors:
            print(f"FAIL lint-guard: {err}", file=sys.stderr)
        return 1
    print(f"ok  lint-guard ({scanned_count} Rust files verified, toolchain 1.100.0 & lint table locked)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

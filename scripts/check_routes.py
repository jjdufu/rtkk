#!/usr/bin/env python3
"""G1 Static Guard: Two-way parity check between src/daemon/routes/**/*.rs and A4-API-CONTRACT-INDEX.md."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
A4_PATH = REPO_ROOT / "docs" / "standards" / "A4-API-CONTRACT-INDEX.md"
ROUTES_DIR = REPO_ROOT / "src" / "daemon" / "routes"

VALID_METHODS = {"GET", "POST", "PUT", "DELETE", "PATCH"}

# Matches Axum .route("/api/v1/...", get(h).post(h2)) or nested v1 .route("/health", get(h))
ROUTE_CALL_RE = re.compile(
    r'\.route\(\s*"([^"]+)"\s*,\s*([^;]+?)\)',
    re.S,
)
METHOD_HANDLER_RE = re.compile(r"\b(get|post|put|delete|patch)\s*\(")


def extract_section(text: str, start_marker: str, end_marker: str) -> str | None:
    s_idx = text.find(start_marker)
    e_idx = text.find(end_marker)
    if s_idx == -1 or e_idx == -1 or e_idx < s_idx:
        return None
    return text[s_idx + len(start_marker) : e_idx]


def parse_a4_table(section_text: str, section_label: str, errors: list[str]) -> set[tuple[str, str]]:
    routes: set[tuple[str, str]] = set()
    for raw_line in section_text.splitlines():
        line = raw_line.strip()
        if not line.startswith("|"):
            continue
        cells = [c.strip().strip("`") for c in line.strip("|").split("|")]
        if len(cells) < 2:
            continue
        method, path = cells[0], cells[1]
        if method in {"方法", "---", ":---"} or set(method) <= {":", "-"}:
            continue
        if method not in VALID_METHODS:
            errors.append(f"A4 ({section_label}): invalid HTTP method {method!r} in row {raw_line!r}")
            continue
        if not path.startswith("/api/v1/"):
            errors.append(f"A4 ({section_label}): route path must start with /api/v1/, got {path!r}")
            continue
        pair = (method, path)
        if pair in routes:
            errors.append(f"A4 ({section_label}): duplicate route entry {method} {path}")
        routes.add(pair)
    return routes


def normalize_route_path(raw_path: str) -> str:
    path = raw_path if raw_path.startswith("/api/") else f"/api/v1{raw_path if raw_path.startswith('/') else '/' + raw_path}"
    # Normalize Axum 0.7 ':id' syntax if ever used into Axum 0.8 '{id}'
    path = re.sub(r":([A-Za-z0-9_]+)", r"{\1}", path)
    return path


def extract_code_routes() -> set[tuple[str, str]]:
    if not ROUTES_DIR.is_dir():
        return set()
    discovered: set[tuple[str, str]] = set()
    for rs_file in sorted(ROUTES_DIR.rglob("*.rs")):
        content = rs_file.read_text(encoding="utf-8")
        for m in ROUTE_CALL_RE.finditer(content):
            raw_path = m.group(1)
            chain = m.group(2)
            norm_path = normalize_route_path(raw_path)
            for mm in METHOD_HANDLER_RE.finditer(chain):
                discovered.add((mm.group(1).upper(), norm_path))
    return discovered


def main() -> int:
    if not A4_PATH.is_file():
        print("FAIL check_routes: missing docs/standards/A4-API-CONTRACT-INDEX.md", file=sys.stderr)
        return 1

    text = A4_PATH.read_text(encoding="utf-8")
    errors: list[str] = []

    mounted_block = extract_section(text, "<!-- MOUNTED_ROUTES_START -->", "<!-- MOUNTED_ROUTES_END -->")
    if mounted_block is None:
        print("FAIL check_routes: missing MOUNTED_ROUTES_START/END markers in A4", file=sys.stderr)
        return 1

    planned_block = extract_section(text, "<!-- PLANNED_ROUTES_START -->", "<!-- PLANNED_ROUTES_END -->")
    if planned_block is None:
        print("FAIL check_routes: missing PLANNED_ROUTES_START/END markers in A4", file=sys.stderr)
        return 1

    a4_mounted = parse_a4_table(mounted_block, "Mounted", errors)
    a4_planned = parse_a4_table(planned_block, "Planned", errors)
    code_mounted = extract_code_routes()

    only_in_code = sorted(code_mounted - a4_mounted)
    only_in_a4 = sorted(a4_mounted - code_mounted)

    for method, path in only_in_code:
        errors.append(f"route mounted in src/daemon/routes/ but missing from A4 Mounted table: {method} {path}")
    for method, path in only_in_a4:
        errors.append(f"route listed in A4 Mounted table but missing from src/daemon/routes/: {method} {path}")

    if errors:
        for err in errors:
            print(f"FAIL check_routes: {err}", file=sys.stderr)
        return 1

    print(
        f"ok  check_routes ({len(code_mounted)} mounted routes in sync, {len(a4_planned)} Stage B planned contracts indexed)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

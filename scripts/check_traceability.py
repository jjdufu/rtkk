#!/usr/bin/env python3
"""G1 Static Guard: Verify A7-REQUIREMENTS-TRACEABILITY.md assertions exist in scripts/gate/stepNN.sh."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
A7_PATH = REPO_ROOT / "docs" / "standards" / "A7-REQUIREMENTS-TRACEABILITY.md"
GATE_DIR = REPO_ROOT / "scripts" / "gate"

CAPABILITY_ID_RE = re.compile(r"^`?(C\d{2})`?$")
STEP_ASSERTION_RE = re.compile(r'`?step(\d{2})`?\s+`"([a-z0-9][a-z0-9-]*)"`')


def extract_gate_assertions(script_path: Path) -> set[str]:
    text = script_path.read_text(encoding="utf-8")
    found: set[str] = set()
    for m in re.finditer(r'\brun_assertion\s+"([a-z0-9][a-z0-9-]*)"', text):
        found.add(m.group(1))
    return found


def main() -> int:
    if not A7_PATH.is_file():
        print("FAIL check_traceability: missing docs/standards/A7-REQUIREMENTS-TRACEABILITY.md", file=sys.stderr)
        return 1

    text = A7_PATH.read_text(encoding="utf-8")
    errors: list[str] = []
    seen_ids: set[str] = set()
    referenced_assertions: list[tuple[str, str, str]] = []

    for lineno, raw_line in enumerate(text.splitlines(), 1):
        line = raw_line.strip()
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) < 6:
            continue
        m_id = CAPABILITY_ID_RE.match(cells[0])
        if not m_id:
            continue
        cap_id = m_id.group(1)
        if cap_id in seen_ids:
            errors.append(f"A7:{lineno}: duplicate capability ID {cap_id}")
        seen_ids.add(cap_id)

        gate_col = cells[5]
        for m_step in STEP_ASSERTION_RE.finditer(gate_col):
            step_num = m_step.group(1)
            assertion_name = m_step.group(2)
            referenced_assertions.append((cap_id, f"step{step_num}", assertion_name))

    for idx in range(1, 15):
        req_id = f"C{idx:02d}"
        if req_id not in seen_ids:
            errors.append(f"A7: baseline capability {req_id} missing from matrix")

    script_cache: dict[str, set[str]] = {}
    for cap_id, step_name, assertion_name in referenced_assertions:
        script_path = GATE_DIR / f"{step_name}.sh"
        if not script_path.is_file():
            errors.append(
                f"A7 ({cap_id}): references {step_name} {assertion_name!r}, but scripts/gate/{step_name}.sh does not exist"
            )
            continue
        if step_name not in script_cache:
            script_cache[step_name] = extract_gate_assertions(script_path)
        if assertion_name not in script_cache[step_name]:
            errors.append(
                f"A7 ({cap_id}): assertion {assertion_name!r} not found via run_assertion in scripts/gate/{step_name}.sh"
            )

    if errors:
        for err in errors:
            print(f"FAIL check_traceability: {err}", file=sys.stderr)
        return 1

    print(
        f"ok  check_traceability ({len(seen_ids)} capabilities C01..C{len(seen_ids):02d}, "
        f"{len(referenced_assertions)} gate assertions verified)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

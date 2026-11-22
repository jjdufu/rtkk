#!/usr/bin/env python3
"""G1 Static Guard: Playbook/Compose YAML hygiene, quoted octal file modes, and 0600 credential permissions."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

MODE_FIELD_RE = re.compile(r"^\s*mode\s*:\s*(.+?)\s*$")
QUOTED_STRING_RE = re.compile(r"""^(?:"[^"]+"|'[^']+')$""")
PRIVATE_KEY_HEADER_RE = re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----")


def strip_inline_yaml_comment(val: str) -> str:
    in_single = False
    in_double = False
    for idx, ch in enumerate(val):
        if ch == "'" and not in_double:
            in_single = not in_single
        elif ch == '"' and not in_single:
            in_double = not in_double
        elif ch == "#" and not in_single and not in_double:
            if idx == 0 or val[idx - 1].isspace():
                return val[:idx].strip()
    return val.strip()


def scan_yaml_file(path: Path, errors: list[str]) -> None:
    rel = path.relative_to(REPO_ROOT).as_posix()
    text = path.read_text(encoding="utf-8")
    if not text.strip():
        errors.append(f"{rel}: empty YAML file")
        return

    for lineno, line in enumerate(text.splitlines(), 1):
        # Reject leading tab indentation in YAML
        leading = len(line) - len(line.lstrip("\t "))
        if "\t" in line[:leading]:
            errors.append(f"{rel}:{lineno}: tab character used in YAML indentation")

        if PRIVATE_KEY_HEADER_RE.search(line):
            errors.append(f"{rel}:{lineno}: inline private key PEM detected in repository config")

        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue

        m_mode = MODE_FIELD_RE.match(line)
        if m_mode:
            raw_val = strip_inline_yaml_comment(m_mode.group(1))
            if not QUOTED_STRING_RE.match(raw_val):
                errors.append(
                    f"{rel}:{lineno}: unquoted mode value {raw_val!r}; wrap octal modes in quotes (e.g. mode: \"0600\")"
                )


def check_private_registry_hosts_toml(errors: list[str]) -> None:
    deploy_path = REPO_ROOT / "configs" / "deploy.yaml"
    if not deploy_path.is_file():
        errors.append("configs/deploy.yaml: missing file")
        return

    text = deploy_path.read_text(encoding="utf-8")
    # Split tasks on '    - name:' boundaries in deploy.yaml
    task_blocks = re.split(r"\n(?=\s*-\s+name\s*:)", text)
    matched_private_hosts_toml = False
    for block in task_blocks:
        if "hosts.toml" in block and ".registry." in block:
            matched_private_hosts_toml = True
            if not re.search(r'^\s*mode\s*:\s*["\']0600["\']\s*$', block, re.M):
                errors.append(
                    'configs/deploy.yaml: private registry hosts.toml task must explicitly declare mode: "0600"'
                )
    if not matched_private_hosts_toml:
        errors.append("configs/deploy.yaml: could not locate private registry hosts.toml task block")


def check_sample_inventory_hygiene(errors: list[str]) -> None:
    inv_path = REPO_ROOT / "configs" / "inventory.yaml"
    if not inv_path.is_file():
        errors.append("configs/inventory.yaml: missing file")
        return
    text = inv_path.read_text(encoding="utf-8")
    for key in ("ansible_ssh_pass", "password"):
        for m in re.finditer(rf'^\s*{key}\s*:\s*["\']?([^"\'#\n]+)["\']?', text, re.M):
            val = m.group(1).strip()
            if val not in {"change-me", "<STRONG_PASSWORD>", "********", ""}:
                errors.append(
                    f"configs/inventory.yaml: {key} must use safe placeholder 'change-me', got {val!r}"
                )


def main() -> int:
    errors: list[str] = []
    yaml_files: list[Path] = []
    for folder in ("configs", "deploy"):
        root = REPO_ROOT / folder
        if root.is_dir():
            yaml_files.extend(sorted(root.rglob("*.yaml")))
            yaml_files.extend(sorted(root.rglob("*.yml")))

    for path in yaml_files:
        scan_yaml_file(path, errors)

    check_private_registry_hosts_toml(errors)
    check_sample_inventory_hygiene(errors)

    if errors:
        for err in errors:
            print(f"FAIL configs-guard: {err}", file=sys.stderr)
        return 1

    print(f"ok  configs-guard ({len(yaml_files)} YAML files verified, modes quoted & hosts.toml 0600 enforced)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

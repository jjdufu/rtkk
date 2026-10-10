#!/usr/bin/env python3
"""Set the rtkk package version in Cargo.lock to match Cargo.toml.

Release Please bumps Cargo.toml but not Cargo.lock. `cargo build --locked`
then refuses to run. This only rewrites our own package version.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
NAMES = {"rtkk"}


def package_version() -> str:
    data = tomllib.loads((ROOT / "Cargo.toml").read_text())
    return str(data["package"]["version"])


def sync_lock(text: str, version: str) -> str:
    chunks = text.split("[[package]]")
    out = [chunks[0]]
    for chunk in chunks[1:]:
        name = None
        for line in chunk.splitlines():
            if line.startswith("name = "):
                name = line.split("=", 1)[1].strip().strip('"')
                break
        if name in NAMES:
            lines = []
            replaced = False
            for line in chunk.splitlines(keepends=True):
                if not replaced and line.startswith("version = "):
                    nl = "\n" if line.endswith("\n") else ""
                    lines.append(f'version = "{version}"{nl}')
                    replaced = True
                else:
                    lines.append(line)
            chunk = "".join(lines)
        out.append(chunk)
    return "[[package]]".join(out)


def main() -> None:
    version = package_version()
    path = ROOT / "Cargo.lock"
    old = path.read_text()
    new = sync_lock(old, version)
    if new != old:
        path.write_text(new)
        print(f"updated Cargo.lock package version to {version}")
    else:
        print(f"Cargo.lock already at {version}")


if __name__ == "__main__":
    main()

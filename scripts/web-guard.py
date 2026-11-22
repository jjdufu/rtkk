#!/usr/bin/env python3
"""G1 Static Guard: Frontend TypeScript Strict, Park UI primitives, offline assets, and Clean Craft microcopy."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

TS_BYPASS_PATTERNS = [
    ("any-annotation", re.compile(r":\s*any\b")),
    ("as-any", re.compile(r"\bas\s+any\b")),
    ("generic-any", re.compile(r"<\s*any\s*>")),
    ("ts-ignore", re.compile(r"@ts-ignore\b")),
    ("ts-expect-error", re.compile(r"@ts-expect-error\b")),
    ("eslint-disable", re.compile(r"\beslint-disable\b")),
]

NATIVE_WIDGET_PATTERNS = [
    ("native-select", re.compile(r"<select[\s>]", re.I)),
    ("native-alert", re.compile(r"(?<![A-Za-z0-9_$])(?:window\.)?alert\s*\(")),
    ("native-confirm", re.compile(r"(?<![A-Za-z0-9_$])(?:window\.)?confirm\s*\(")),
    ("native-prompt", re.compile(r"(?<![A-Za-z0-9_$])(?:window\.)?prompt\s*\(")),
]

EXTERNAL_CDN_PATTERN = re.compile(
    r"https?://(?!(?:www\.w3\.org/2000/svg|127\.0\.0\.1|localhost)\b)[A-Za-z0-9.-]+",
    re.I,
)

UI_MICROCOPY_PATTERNS = [
    ("verbose-click", re.compile(r">\s*(?:点击|点此|按下?|选择)\S{1,12}\s*<")),
    ("verbose-en-click", re.compile(r">\s*(?:Click\s+to|Click\s+here|Press\s+to)\b", re.I)),
]


def parse_hex_rgb(val: str) -> tuple[float, float, float] | None:
    v = val.strip().lstrip("#")
    if len(v) == 3 and all(c in "0123456789abcdefABCDEF" for c in v):
        v = "".join(c * 2 for c in v)
    if len(v) != 6 or not all(c in "0123456789abcdefABCDEF" for c in v):
        return None
    r = int(v[0:2], 16) / 255.0
    g = int(v[2:4], 16) / 255.0
    b = int(v[4:6], 16) / 255.0
    return (r, g, b)


def relative_luminance(rgb: tuple[float, float, float]) -> float:
    def channel(c: float) -> float:
        return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4

    r, g, b = rgb
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def contrast_ratio(c1: tuple[float, float, float], c2: tuple[float, float, float]) -> float:
    l1 = relative_luminance(c1)
    l2 = relative_luminance(c2)
    lighter, darker = (l1, l2) if l1 >= l2 else (l2, l1)
    return (lighter + 0.05) / (darker + 0.05)


def extract_css_block_vars(css: str, selector_regex: str) -> dict[str, str]:
    m = re.search(selector_regex + r"\s*\{([^}]*)\}", css, re.S)
    if not m:
        return {}
    props: dict[str, str] = {}
    for prop_m in re.finditer(r"(--[A-Za-z0-9-]+)\s*:\s*([^;]+);", m.group(1)):
        props[prop_m.group(1).strip()] = prop_m.group(2).strip()
    return props


def verify_tokens_css(web_dir: Path, errors: list[str]) -> None:
    css_files = sorted((web_dir / "src").rglob("*.css"))
    combined_css = "\n".join(p.read_text(encoding="utf-8") for p in css_files)
    if not re.search(r"\[hidden\]\s*\{\s*display\s*:\s*none\s*!important\s*;?\s*\}", combined_css):
        errors.append("web/src CSS: missing mandatory '[hidden] { display: none !important; }' rule")

    light_vars = extract_css_block_vars(combined_css, r":root")
    dark_vars = extract_css_block_vars(combined_css, r'\[data-theme="dark"\]')
    if not light_vars:
        errors.append("web/src CSS: missing ':root' token block")
    if not dark_vars:
        errors.append('web/src CSS: missing \'[data-theme="dark"]\' token block')

    for mode_name, block in (("light (:root)", light_vars), ('dark ([data-theme="dark"])', dark_vars)):
        if not block:
            continue
        accent = block.get("--accent", "")
        accent_fg = block.get("--accent-fg", "")
        if not accent or not accent_fg:
            errors.append(f"web/src CSS {mode_name}: missing --accent or --accent-fg")
            continue
        if accent.lower() == accent_fg.lower():
            errors.append(f"web/src CSS {mode_name}: --accent and --accent-fg must not be identical ({accent})")
            continue
        rgb_a = parse_hex_rgb(accent)
        rgb_fg = parse_hex_rgb(accent_fg)
        if rgb_a and rgb_fg:
            ratio = contrast_ratio(rgb_a, rgb_fg)
            if ratio < 4.5:
                errors.append(
                    f"web/src CSS {mode_name}: contrast(--accent={accent}, --accent-fg={accent_fg}) = {ratio:.2f} < 4.5"
                )


def main() -> int:
    web_dir = REPO_ROOT / "web"
    web_src = web_dir / "src"
    if not web_src.is_dir():
        print("ok  web-guard (web/ not initialized yet)")
        return 0

    errors: list[str] = []
    files: list[Path] = []
    index_html = web_dir / "index.html"
    if index_html.is_file():
        files.append(index_html)
    for ext in ("*.ts", "*.tsx", "*.js", "*.jsx", "*.css"):
        files.extend(sorted(web_src.rglob(ext)))

    for path in files:
        rel = path.relative_to(REPO_ROOT).as_posix()
        lines = path.read_text(encoding="utf-8").splitlines()
        is_code = path.suffix in {".ts", ".tsx", ".js", ".jsx"}
        for lineno, line in enumerate(lines, 1):
            if is_code:
                for rule, rx in TS_BYPASS_PATTERNS + NATIVE_WIDGET_PATTERNS + UI_MICROCOPY_PATTERNS:
                    m = rx.search(line)
                    if m:
                        errors.append(f"{rel}:{lineno}: [{rule}] forbidden pattern {m.group(0)!r}")
            m_cdn = EXTERNAL_CDN_PATTERN.search(line)
            if m_cdn:
                errors.append(f"{rel}:{lineno}: [external-url] offline violation {m_cdn.group(0)!r}")

    verify_tokens_css(web_dir, errors)

    if errors:
        for err in errors:
            print(f"FAIL web-guard: {err}", file=sys.stderr)
        return 1

    print(f"ok  web-guard ({len(files)} frontend files verified)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

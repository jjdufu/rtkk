#!/usr/bin/env python3
"""G1 Static Guard: Memory file <24KB budget, Markdown relative link checker, and present-state copy hygiene."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MAX_MEMORY_BYTES = 24_000

MEMORY_FILES = [
    "IRON_RULES.md",
    "AGENTS.md",
    "GEMINI.md",
    "CLAUDE.md",
]

SKIP_DIRS = {".git", "target", "node_modules", ".gate", "dist", "playwright-report", "test-results"}

FENCE_RE = re.compile(r"^(\s*)(`{3,}|~{3,})")
INLINE_CODE_RE = re.compile(r"`[^`]+`")
MD_LINK_RE = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")

SKIP_SCHEMES = ("http://", "https://", "mailto:", "file://", "conversation://", "data:")

DOC_HYGIENE_PATTERNS = [
    (
        "audience-tag",
        re.compile(
            r"(?:适用(?:于)?|面向|专为).{0,8}(?:非技术|运营人员|新手|小白)(?:专用|使用|编写|设计)?"
            r"|(?:运营人员)" + r"(?:专用)"
        ),
    ),
    (
        "diff-leakage",
        re.compile(
            r"(?:现已|已经?)(?:统一)?(?:调整为|放宽到?|修改为|替换为)"
            r"|(?:旧配置)" + r"(?:里的)|(?:原方案)" + r"(?:为)"
        ),
    ),
    ("rhetorical-qa", re.compile(r"(?:怎么办|如何处理|怎么选)\s*[？?]")),
]


def iter_markdown_files() -> list[Path]:
    result: list[Path] = []
    for path in sorted(REPO_ROOT.rglob("*.md")):
        rel_parts = path.relative_to(REPO_ROOT).parts
        if any(part in SKIP_DIRS for part in rel_parts):
            continue
        result.append(path)
    return result


def check_memory_budgets(errors: list[str]) -> None:
    candidates = [REPO_ROOT / name for name in MEMORY_FILES]
    web_agents = REPO_ROOT / "web" / "AGENTS.md"
    if web_agents.is_file():
        candidates.append(web_agents)

    synced_bodies: dict[str, str] = {}
    for path in candidates:
        rel = path.relative_to(REPO_ROOT).as_posix()
        if not path.is_file():
            errors.append(f"{rel}: required memory file missing")
            continue
        size = path.stat().st_size
        if size >= MAX_MEMORY_BYTES:
            errors.append(f"{rel}: size {size} bytes exceeds strict < {MAX_MEMORY_BYTES} byte budget")
        if path.name in {"AGENTS.md", "GEMINI.md", "CLAUDE.md"}:
            lines = path.read_text(encoding="utf-8").splitlines()
            # Line 3 mentions the sibling filenames; everything from line 4 onward must stay identical
            synced_bodies[path.name] = "\n".join(lines[4:])

    if len(synced_bodies) == 3:
        ref_body = synced_bodies["AGENTS.md"]
        for mirror in ("GEMINI.md", "CLAUDE.md"):
            if synced_bodies[mirror] != ref_body:
                errors.append(f"{mirror}: out of sync with AGENTS.md body")


def strip_code_spans_from_markdown(lines: list[str]) -> list[tuple[int, str]]:
    """Yield (lineno, line_without_inline_code) outside fenced code blocks."""
    out: list[tuple[int, str]] = []
    active_fence: str | None = None
    for lineno, line in enumerate(lines, 1):
        m_fence = FENCE_RE.match(line)
        if m_fence:
            marker = m_fence.group(2)
            if active_fence is None:
                active_fence = marker[0] * len(marker)
                continue
            if marker[0] == active_fence[0] and len(marker) >= len(active_fence):
                active_fence = None
                continue
        if active_fence is not None:
            continue
        cleaned = INLINE_CODE_RE.sub("", line)
        out.append((lineno, cleaned))
    return out


def check_links_and_copy(md_files: list[Path], errors: list[str]) -> int:
    checked_links = 0
    for path in md_files:
        rel = path.relative_to(REPO_ROOT).as_posix()
        lines = path.read_text(encoding="utf-8").splitlines()
        is_historical = (
            rel == "CHANGELOG.md"
            or rel.startswith("changelog/")
            or rel.startswith("plans/completed/")
        )

        for lineno, cleaned in strip_code_spans_from_markdown(lines):
            for match in MD_LINK_RE.finditer(cleaned):
                raw_target = match.group(1).strip()
                if not raw_target:
                    continue
                # Strip optional Markdown link title: (path "title")
                target_path_str = raw_target.split()[0].strip("<>")
                if target_path_str.startswith("#") or target_path_str.startswith(SKIP_SCHEMES):
                    continue
                target_no_frag = target_path_str.split("#", 1)[0].split("?", 1)[0]
                if not target_no_frag:
                    continue
                checked_links += 1
                resolved = (path.parent / target_no_frag).resolve()
                if not resolved.exists():
                    errors.append(f"{rel}:{lineno}: broken relative link -> {raw_target!r}")

            if not is_historical:
                for rule_id, rx in DOC_HYGIENE_PATTERNS:
                    m_copy = rx.search(cleaned)
                    if m_copy:
                        errors.append(f"{rel}:{lineno}: [{rule_id}] anti-pattern {m_copy.group(0)!r}")

    return checked_links


def main() -> int:
    errors: list[str] = []
    check_memory_budgets(errors)
    md_files = iter_markdown_files()
    link_count = check_links_and_copy(md_files, errors)

    if errors:
        for err in errors:
            print(f"FAIL docs-and-plans-guard: {err}", file=sys.stderr)
        return 1

    print(
        f"ok  docs-and-plans-guard ({len(md_files)} Markdown files, {link_count} relative links verified, memory < 24KB)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

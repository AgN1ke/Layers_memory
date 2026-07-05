#!/usr/bin/env python3
"""Check internal Markdown links and optional wiki navigation coverage."""

from __future__ import annotations

import argparse
import re
import sys
import urllib.parse
from pathlib import Path


SKIP_DIRS = {".git", ".venv", ".pytest_cache", "target", "__pycache__"}
LINK_RE = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
EXTERNAL_PREFIXES = ("http://", "https://", "mailto:", "file:")


def markdown_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for path in root.rglob("*.md"):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        files.append(path)
    return sorted(files)


def normalize_target(raw: str) -> str:
    target = raw.split("#", 1)[0].strip()
    if target.startswith("<") and target.endswith(">"):
        target = target[1:-1]
    return urllib.parse.unquote(target)


def check_links(root: Path) -> list[str]:
    errors: list[str] = []
    for path in markdown_files(root):
        text = path.read_text(encoding="utf-8")
        for raw in LINK_RE.findall(text):
            target = normalize_target(raw)
            if not target or target.startswith(EXTERNAL_PREFIXES):
                continue
            resolved = (path.parent / target).resolve()
            try:
                resolved.relative_to(root)
            except ValueError:
                continue
            if not resolved.exists():
                errors.append(f"{path.relative_to(root)}: missing link target {raw}")
    return errors


def check_wiki_related_pages(root: Path) -> list[str]:
    errors: list[str] = []
    wiki_root = root / "wiki"
    if not wiki_root.exists():
        return errors
    for path in sorted(wiki_root.rglob("*.md")):
        if path.name == "index.md":
            continue
        text = path.read_text(encoding="utf-8")
        if "## Related pages" not in text:
            errors.append(f"{path.relative_to(root)}: missing '## Related pages'")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.cwd(),
        help="Repository root. Defaults to the current working directory.",
    )
    parser.add_argument(
        "--wiki-related-pages",
        action="store_true",
        help="Also require wiki pages, except wiki/index.md, to have Related pages.",
    )
    args = parser.parse_args()

    root = args.root.resolve()
    errors = check_links(root)
    if args.wiki_related_pages:
        errors.extend(check_wiki_related_pages(root))

    if errors:
        for error in errors:
            print(error)
        return 1

    checked = len(markdown_files(root))
    print(f"checked {checked} markdown files; wiki links ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())

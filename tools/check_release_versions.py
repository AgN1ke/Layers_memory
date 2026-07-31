#!/usr/bin/env python3
"""Verify that Memory Engine release versions and evidence agree."""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path


SEMVER_RE = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
SECTION_RE = re.compile(r"^\s*\[([^]]+)]\s*$")
STRING_VALUE_RE = re.compile(r'^\s*([A-Za-z0-9_.-]+)\s*=\s*"([^"]+)"\s*(?:#.*)?$')
BOOL_VALUE_RE = re.compile(
    r"^\s*([A-Za-z0-9_.-]+)\s*=\s*(true|false)\s*(?:#.*)?$",
    re.IGNORECASE,
)
WORKSPACE_MEMBERS_RE = re.compile(
    r"(?ms)^\s*members\s*=\s*\[(.*?)]",
)
QUOTED_ITEM_RE = re.compile(r'"([^"]+)"')


@dataclass(frozen=True)
class ReleaseState:
    version: str
    crate_manifests: tuple[Path, ...]
    python_manifest: Path


class ReleaseCheckError(ValueError):
    """Raised when the repository cannot produce a coherent release."""


def section_values(path: Path, wanted_section: str) -> dict[str, str | bool]:
    """Read the simple scalar TOML values used by the release manifests."""

    if not path.is_file():
        raise ReleaseCheckError(f"missing manifest: {path}")

    current_section: str | None = None
    values: dict[str, str | bool] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        section_match = SECTION_RE.match(line)
        if section_match:
            current_section = section_match.group(1)
            continue
        if current_section != wanted_section:
            continue

        string_match = STRING_VALUE_RE.match(line)
        if string_match:
            values[string_match.group(1)] = string_match.group(2)
            continue
        bool_match = BOOL_VALUE_RE.match(line)
        if bool_match:
            values[bool_match.group(1)] = bool_match.group(2).lower() == "true"

    return values


def validate_semver(version: str, source: Path) -> None:
    if not SEMVER_RE.fullmatch(version):
        raise ReleaseCheckError(
            f"{source}: version {version!r} must be stable MAJOR.MINOR.PATCH"
        )


def workspace_member_manifests(root: Path, cargo_manifest: Path) -> tuple[Path, ...]:
    text = cargo_manifest.read_text(encoding="utf-8")
    workspace_match = re.search(
        r"(?ms)^\s*\[workspace]\s*$\n(.*?)(?=^\s*\[|\Z)",
        text,
    )
    if workspace_match is None:
        raise ReleaseCheckError(f"{cargo_manifest}: [workspace] is missing")

    members_match = WORKSPACE_MEMBERS_RE.search(workspace_match.group(1))
    if members_match is None:
        raise ReleaseCheckError(f"{cargo_manifest}: [workspace].members is missing")

    member_patterns = QUOTED_ITEM_RE.findall(members_match.group(1))
    if not member_patterns:
        raise ReleaseCheckError(f"{cargo_manifest}: [workspace].members is empty")

    manifests: set[Path] = set()
    for member_pattern in member_patterns:
        matching_dirs = sorted(root.glob(member_pattern))
        if not matching_dirs:
            raise ReleaseCheckError(
                f"{cargo_manifest}: workspace member {member_pattern!r} does not exist"
            )
        for member_dir in matching_dirs:
            manifest = member_dir / "Cargo.toml"
            if not manifest.is_file():
                raise ReleaseCheckError(
                    f"{cargo_manifest}: workspace member {member_pattern!r} has no Cargo.toml"
                )
            manifests.add(manifest)

    return tuple(sorted(manifests))


def read_release_state(root: Path) -> ReleaseState:
    cargo_manifest = root / "Cargo.toml"
    workspace = section_values(cargo_manifest, "workspace.package")
    version = workspace.get("version")
    if not isinstance(version, str):
        raise ReleaseCheckError(
            f"{cargo_manifest}: [workspace.package].version is missing"
        )
    validate_semver(version, cargo_manifest)

    crate_manifests = workspace_member_manifests(root, cargo_manifest)

    for manifest in crate_manifests:
        package = section_values(manifest, "package")
        if package.get("version.workspace") is not True:
            explicit = package.get("version")
            detail = f"explicit version {explicit!r}" if explicit else "no version"
            raise ReleaseCheckError(
                f"{manifest}: {detail}; every workspace crate must use "
                "version.workspace = true"
            )

    python_manifest = root / "crates" / "python_adapter" / "pyproject.toml"
    python_project = section_values(python_manifest, "project")
    python_version = python_project.get("version")
    if python_version != version:
        raise ReleaseCheckError(
            f"{python_manifest}: Python version {python_version!r} does not match "
            f"Cargo workspace version {version!r}"
        )

    return ReleaseState(
        version=version,
        crate_manifests=crate_manifests,
        python_manifest=python_manifest,
    )


def normalize_tag(tag: str) -> tuple[str, str]:
    if not tag.startswith("v"):
        raise ReleaseCheckError(f"tag {tag!r} must use the vMAJOR.MINOR.PATCH form")
    version = tag[1:]
    if not SEMVER_RE.fullmatch(version):
        raise ReleaseCheckError(f"tag {tag!r} must use the vMAJOR.MINOR.PATCH form")
    return tag, version


def check_release_evidence(root: Path, tag: str) -> Path:
    acceptance = root / "wiki" / "pages" / "releases" / f"{tag}-acceptance.md"
    if not acceptance.is_file():
        raise ReleaseCheckError(
            f"missing exact release acceptance page: {acceptance.relative_to(root)}"
        )

    acceptance_text = acceptance.read_text(encoding="utf-8")
    if not re.search(rf"(?m)^Release:\s*`?{re.escape(tag)}`?\s*$", acceptance_text):
        raise ReleaseCheckError(
            f"{acceptance.relative_to(root)}: add an exact 'Release: {tag}' line"
        )
    if not re.search(r"(?m)^Status:\s*accepted\s*$", acceptance_text, re.IGNORECASE):
        raise ReleaseCheckError(
            f"{acceptance.relative_to(root)}: release evidence must say 'Status: accepted'"
        )

    history = root / "HISTORY.md"
    history_text = history.read_text(encoding="utf-8")
    if not re.search(rf"(?m)^##\s+.*\b{re.escape(tag)}\b.*$", history_text):
        raise ReleaseCheckError(
            f"HISTORY.md: add a release heading containing exact tag {tag}"
        )

    return acceptance


def check_repository(
    root: Path,
    *,
    tag: str | None = None,
    require_release_evidence: bool = False,
) -> ReleaseState:
    state = read_release_state(root)

    normalized_tag: str | None = None
    if tag is not None:
        normalized_tag, tag_version = normalize_tag(tag)
        if tag_version != state.version:
            raise ReleaseCheckError(
                f"tag {normalized_tag!r} does not match manifest version "
                f"{state.version!r}"
            )

    if require_release_evidence:
        if normalized_tag is None:
            raise ReleaseCheckError("--require-release-evidence requires --tag")
        check_release_evidence(root, normalized_tag)

    return state


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="Repository root. Defaults to the parent of tools/.",
    )
    parser.add_argument(
        "--tag",
        help="Requested Git tag in vMAJOR.MINOR.PATCH form.",
    )
    parser.add_argument(
        "--require-release-evidence",
        action="store_true",
        help="Also require exact accepted release evidence and a HISTORY heading.",
    )
    args = parser.parse_args(argv)

    root = args.root.resolve()
    try:
        state = check_repository(
            root,
            tag=args.tag,
            require_release_evidence=args.require_release_evidence,
        )
    except (OSError, ReleaseCheckError) as error:
        print(f"release consistency failed: {error}", file=sys.stderr)
        return 1

    summary = (
        f"release versions consistent: {state.version}; "
        f"{len(state.crate_manifests)} Rust crates + Python"
    )
    if args.tag:
        summary += f" + tag {args.tag}"
    if args.require_release_evidence:
        summary += " + accepted evidence + HISTORY"
    print(summary)
    return 0


if __name__ == "__main__":
    sys.exit(main())

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


TOOLS_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS_ROOT))

from check_release_versions import ReleaseCheckError, check_repository  # noqa: E402


class ReleaseVersionCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.root = Path(self.temp_dir.name)
        self.write(
            "Cargo.toml",
            "[workspace]\n"
            "members = [\n"
            '    "crates/memory_engine",\n'
            '    "crates/python_adapter",\n'
            "]\n\n"
            "[workspace.package]\n"
            'version = "0.4.0"\n',
        )
        self.write(
            "crates/memory_engine/Cargo.toml",
            "[package]\nname = \"memory_engine\"\nversion.workspace = true\n",
        )
        self.write(
            "crates/python_adapter/Cargo.toml",
            "[package]\nname = \"memory_engine_py\"\nversion.workspace = true\n",
        )
        self.write(
            "crates/python_adapter/pyproject.toml",
            '[project]\nname = "memory-engine"\nversion = "0.4.0"\n',
        )
        self.write("HISTORY.md", "# History\n")

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def write(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def test_accepts_consistent_workspace_and_python_versions(self) -> None:
        state = check_repository(self.root)

        self.assertEqual(state.version, "0.4.0")
        self.assertEqual(len(state.crate_manifests), 2)

    def test_rejects_python_version_mismatch(self) -> None:
        self.write(
            "crates/python_adapter/pyproject.toml",
            '[project]\nname = "memory-engine"\nversion = "0.3.0"\n',
        )

        with self.assertRaisesRegex(ReleaseCheckError, "does not match"):
            check_repository(self.root)

    def test_rejects_crate_that_does_not_inherit_workspace_version(self) -> None:
        self.write(
            "crates/memory_engine/Cargo.toml",
            '[package]\nname = "memory_engine"\nversion = "0.4.0"\n',
        )

        with self.assertRaisesRegex(ReleaseCheckError, "version.workspace = true"):
            check_repository(self.root)

    def test_rejects_tag_that_differs_from_manifests(self) -> None:
        with self.assertRaisesRegex(ReleaseCheckError, "does not match"):
            check_repository(self.root, tag="v0.5.0")

    def test_requires_exact_accepted_evidence_for_a_release_tag(self) -> None:
        self.write(
            "wiki/pages/releases/v0.4.0-acceptance.md",
            "# v0.4.0 acceptance\n\nRelease: `v0.4.0`\nStatus: accepted\n",
        )
        self.write("HISTORY.md", "# History\n\n## 2026-08-01 — v0.4.0 release\n")

        state = check_repository(
            self.root,
            tag="v0.4.0",
            require_release_evidence=True,
        )

        self.assertEqual(state.version, "0.4.0")

    def test_rejects_unaccepted_release_evidence(self) -> None:
        self.write(
            "wiki/pages/releases/v0.4.0-acceptance.md",
            "# v0.4.0 acceptance\n\nRelease: v0.4.0\nStatus: pending\n",
        )
        self.write("HISTORY.md", "# History\n\n## 2026-08-01 — v0.4.0 release\n")

        with self.assertRaisesRegex(ReleaseCheckError, "Status: accepted"):
            check_repository(
                self.root,
                tag="v0.4.0",
                require_release_evidence=True,
            )


if __name__ == "__main__":
    unittest.main()

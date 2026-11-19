"""Check the source identity embedded in an installed release wheel."""

import os
import re

import pytest

import memory_engine


def test_installed_release_wheel_build_identity():
    version = os.getenv("MEMORY_ENGINE_EXPECTED_VERSION")
    commit = os.getenv("MEMORY_ENGINE_EXPECTED_COMMIT")
    if version is None and commit is None:
        pytest.skip("Expected wheel identity was not supplied")

    assert version and commit, (
        "Set both MEMORY_ENGINE_EXPECTED_VERSION and MEMORY_ENGINE_EXPECTED_COMMIT"
    )
    assert re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit), (
        "Expected commit must be a full lowercase Git SHA"
    )
    assert memory_engine.__version__ == version
    assert memory_engine.build_info() == {
        "version": version,
        "commit": commit,
        "dirty": False,
    }

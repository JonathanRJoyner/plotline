"""Shared test setup."""

import pytest


@pytest.fixture(autouse=True)
def _private_model_cache(tmp_path, monkeypatch):
    """Point the model cache at a fresh folder, so no test reads or writes a user's real one."""
    monkeypatch.setenv("PLOTLINE_CACHE_DIR", str(tmp_path / "model-cache"))

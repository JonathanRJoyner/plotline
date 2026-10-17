"""Tests for kept models: the cache folder, accepted licences, downloads and loading into a session."""

import hashlib
import io
import shutil
import urllib.request
from pathlib import Path

import pytest

from plotline import Session, _core, models

FIXTURE = Path(__file__).parent.parent / "web" / "e2e" / "fixtures" / "tiny-conv.onnx"


def entry_for(data: bytes, **changes) -> dict:
    """A catalogue entry for a model whose file is ``data``."""
    entry = {
        "id": "demo-1",
        "url": "https://example.test/demo-1.onnx",
        "sha256": hashlib.sha256(data).hexdigest(),
        "bytes": len(data),
        "licence": "MIT",
        "attribution": "Demo authors",
    }
    return entry | changes


def serve(monkeypatch, data: bytes) -> None:
    """Make every ``urlopen`` answer with ``data``."""
    monkeypatch.setattr(urllib.request, "urlopen", lambda url, **kw: io.BytesIO(data))


def test_a_file_of_your_own_draws_with_learned():
    s = Session()
    model_id = s.add_model(FIXTURE)
    assert model_id.startswith("file-")
    s.set_image(_core.synthetic_radial(64, 64))
    s.set_style("learned")
    s.update("model", model=model_id, resolution=256)
    drawing = s.run_blocking()
    assert sum(len(layer.paths) for layer in drawing.layers) > 0


def test_kept_models_load_with_a_new_session():
    model_id = models.add_file(FIXTURE)
    assert [m["id"] for m in Session().models()] == [model_id]


def test_a_withdrawn_id_loads_as_a_file():
    models.cache_dir().mkdir(parents=True)
    shutil.copy(FIXTURE, models.cache_dir() / "gone-1.onnx")
    assert [m["id"][:5] for m in Session().models()] == ["file-"]


def test_a_damaged_kept_file_is_skipped_with_a_warning():
    models.cache_dir().mkdir(parents=True)
    (models.cache_dir() / "broken.onnx").write_bytes(b"not a model")
    with pytest.warns(UserWarning, match="broken"):
        s = Session()
    assert s.models() == []


def test_download_refuses_a_wrong_hash(monkeypatch):
    data = FIXTURE.read_bytes()
    serve(monkeypatch, data[::-1])
    with pytest.raises(ValueError, match="checksum"):
        models.download(entry_for(data))
    assert models.kept() == [] and not list(models.cache_dir().glob("*"))


def test_download_refuses_a_short_file(monkeypatch):
    data = FIXTURE.read_bytes()
    serve(monkeypatch, data[: len(data) // 2])
    with pytest.raises(ValueError, match="bytes"):
        models.download(entry_for(data))
    assert models.kept() == [] and not list(models.cache_dir().glob("*"))


def test_download_keeps_a_file_that_matches(monkeypatch):
    data = FIXTURE.read_bytes()
    serve(monkeypatch, data)
    path = models.download(entry_for(data))
    assert path == models.cache_dir() / "demo-1.onnx" and path.read_bytes() == data
    assert models.kept() == [("demo-1", path)]


def test_acceptance_is_per_licence_text():
    entry = entry_for(b"x")
    assert not models.accepted(entry)
    models.accept(entry)
    assert models.accepted(entry)
    assert not models.accepted(entry | {"licence": "CC-BY-NC-4.0"})
    assert not models.accepted(entry | {"id": "other-1"})


def test_remove_deletes_only_a_kept_model():
    model_id = models.add_file(FIXTURE)
    models.remove("../" + model_id)
    assert [i for i, _ in models.kept()] == [model_id]
    models.remove(model_id)
    assert models.kept() == []


def test_the_cache_folder_follows_the_platform(monkeypatch, tmp_path):
    monkeypatch.delenv("PLOTLINE_CACHE_DIR")
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "XDG_CACHE_HOME"))
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "LOCALAPPDATA"))
    # `Path.home()` reads HOME on POSIX and USERPROFILE on Windows.
    monkeypatch.setenv("HOME", str(tmp_path / "HOME"))
    monkeypatch.setenv("USERPROFILE", str(tmp_path / "HOME"))
    for platform, base in [
        ("linux", tmp_path / "XDG_CACHE_HOME"),
        ("win32", tmp_path / "LOCALAPPDATA"),
        ("darwin", tmp_path / "HOME" / "Library" / "Caches"),
    ]:
        monkeypatch.setattr(models.sys, "platform", platform)
        assert models.cache_dir() == base / "plotline" / "models"
    monkeypatch.delenv("XDG_CACHE_HOME")
    monkeypatch.setattr(models.sys, "platform", "linux")
    assert models.cache_dir() == tmp_path / "HOME" / ".cache" / "plotline" / "models"

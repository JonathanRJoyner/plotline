"""Learned line models kept on this machine: the cache folder, accepted licences, downloads.

The engine checks a model file (``plotline._core.Session.add_model``); this module is what a Python
host adds around it. The files live in one folder, ``<id>.onnx`` each, and the licences a person
has accepted sit in ``accepted.json`` beside them. A download goes through the steps of the design:
the caller shows the licence and asks (``accepted``, ``accept``), then ``download`` fetches the
file, checks its size and SHA-256 against the catalogue entry, and keeps it only when both match.
Only Python's standard library is used.
"""

import hashlib
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from plotline import _core

# Seconds to wait on the network for one read, so a stalled server fails instead of hanging.
_TIMEOUT = 60


def cache_dir() -> Path:
    """The folder that holds the kept models: ``PLOTLINE_CACHE_DIR`` itself when it is set, else
    ``plotline/models`` under ``XDG_CACHE_HOME`` (default ``~/.cache``) on Linux,
    ``~/Library/Caches`` on macOS or ``%LOCALAPPDATA%`` on Windows. It is not created here."""
    if override := os.environ.get("PLOTLINE_CACHE_DIR"):
        return Path(override)
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Caches"
    else:
        base = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache")
    return base / "plotline" / "models"


def kept() -> list[tuple[str, Path]]:
    """Every kept model as ``(id, path)``, in id order; the id is the file's name without
    ``.onnx``."""
    return sorted((p.stem, p) for p in cache_dir().glob("*.onnx"))


def _licence_hash(entry: dict[str, Any]) -> str:
    text = entry["licence"] + "\n" + entry["attribution"] + "\n" + entry["training"]
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _acceptances() -> dict[str, str]:
    """The id to licence hash map in ``accepted.json``; a missing or damaged file holds none."""
    try:
        data = json.loads((cache_dir() / "accepted.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def accepted(entry: dict[str, Any]) -> bool:
    """Whether the person has accepted this model's current licence, attribution and training
    note. A changed text asks again, because the hash of it is what is kept."""
    return _acceptances().get(entry["id"]) == _licence_hash(entry)


def accept(entry: dict[str, Any]) -> None:
    """Keep that the person accepted this model's licence, attribution and training note."""
    folder = cache_dir()
    folder.mkdir(parents=True, exist_ok=True)
    data = _acceptances() | {entry["id"]: _licence_hash(entry)}
    (folder / "accepted.json").write_text(
        json.dumps(data, indent=2) + "\n", encoding="utf-8", newline="\n"
    )


def _check_id(id: str) -> None:
    """Refuse an id that is not a plain file name stem, so it cannot name a path outside the
    cache folder."""
    if not re.fullmatch(r"[A-Za-z0-9._-]+", id):
        raise ValueError(
            f"{id!r} is not a model id: use letters, digits, dots, dashes and underscores"
        )


def _keep(id: str, data: bytes) -> Path:
    """Write ``<id>.onnx`` whole or not at all: the bytes go to a side file that is renamed over
    the real name, and a side file left by a failure is removed. Raises ``ValueError`` for an id
    that is not a plain file name stem."""
    _check_id(id)
    folder = cache_dir()
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / f"{id}.onnx"
    side = folder / f"{id}.part"
    try:
        side.write_bytes(data)
        os.replace(side, path)
    finally:
        side.unlink(missing_ok=True)
    return path


def intact(entry: dict[str, Any]) -> bool:
    """Whether the kept file of the catalogue ``entry`` is there and has the size and SHA-256 the
    entry lists."""
    try:
        data = (cache_dir() / f"{entry['id']}.onnx").read_bytes()
    except OSError:
        return False
    return len(data) == int(entry["bytes"]) and hashlib.sha256(data).hexdigest() == entry["sha256"]


def download(entry: dict[str, Any]) -> Path:
    """Fetch the model of a catalogue ``entry`` and keep it as ``<id>.onnx``; return the path.

    The caller has shown the licence and had it accepted. Nothing past ``entry["bytes"]`` is
    read, and the file is written only when its size and SHA-256 both match the entry. Raises
    ``ValueError`` for an id that is not a plain file name stem (before anything is fetched) and for
    a file of the wrong size or hash, and ``OSError`` when the fetch fails, with a message that
    names the model and the address and holds no library wording; either way nothing is kept.
    """
    _check_id(entry["id"])
    expected = int(entry["bytes"])
    try:
        with urllib.request.urlopen(entry["url"], timeout=_TIMEOUT) as response:
            data = response.read(expected + 1)
    except OSError as e:  # a URLError, a timeout, or a connection that dropped
        why = (
            f"the server answered {e.code}"
            if isinstance(e, urllib.error.HTTPError)
            else "check your connection"
        )
        raise OSError(f"could not download {entry['id']} from {entry['url']}: {why}") from e
    if len(data) != expected:
        raise ValueError(
            f"the download of {entry['id']} is {'at least ' if len(data) > expected else ''}"
            f"{len(data)} bytes, not the {expected} bytes the catalogue lists"
        )
    if hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise ValueError(
            f"the download of {entry['id']} does not match its published checksum; "
            "it may be damaged"
        )
    return _keep(entry["id"], data)


def add_file(path: Path) -> str:
    """Copy the ONNX file at ``path``, a model of your own, into the cache and return its id,
    ``file-`` and 12 hex digits of its SHA-256. The engine checks the file first, so ``ValueError``
    says why a file is not a usable model and nothing is kept; ``OSError`` is a file that cannot
    be read."""
    data = Path(path).read_bytes()
    info = _core.Session().add_model(None, data)
    _keep(info["id"], data)
    return str(info["id"])


def remove(id: str) -> None:
    """Delete the kept model called ``id``; one that is not kept changes nothing. Only the files
    ``kept`` lists can go, so an ``id`` cannot reach outside the cache folder."""
    for kept_id, path in kept():
        if kept_id == id:
            path.unlink(missing_ok=True)

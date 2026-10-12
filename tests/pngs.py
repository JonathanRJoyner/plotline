"""A PNG writer for tests, so that file loading can be tested on images made in code.

It writes 8-bit RGB without filtering, following the chunk layout of the PNG specification
(W3C, Portable Network Graphics Specification, second edition, 2003).
"""

import struct
import zlib
from pathlib import Path

import numpy as np


def write_png(path: Path, rgb: np.ndarray) -> None:
    """Write ``rgb``, an ``(H, W, 3)`` array of values in 0..1, to ``path`` as a PNG."""
    pixels = np.round(np.asarray(rgb) * 255).astype(np.uint8)
    h, w, _ = pixels.shape
    raw = b"".join(b"\x00" + row.tobytes() for row in pixels)  # filter type 0 on every row

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))

    header = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)  # 8 bits per sample, RGB
    png = chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + png)

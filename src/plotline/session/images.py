"""Reading image files as the float RGB arrays the engine works on.

Pillow decodes the file, and most modes then go through its conversion to 8-bit RGB. That
conversion clips 16-bit and float values to 0..255 instead of scaling them, so a 16-bit scan would
come out almost white, and those images are handled apart. 16-bit greyscale, which is how scanners
write it (modes ``I;16``, ``I;16B``, ``I;16L``, ``I;16N`` and ``I``), is scaled so that 65535 is
white, and any transparency in it is ignored. Float images (mode ``F``) are refused with a
``ValueError``, since nothing defines their white level.

The array is upright (the EXIF orientation is applied, so a phone photo is not sideways), opaque
(transparent pixels become white, the colour of blank paper, instead of whatever colour hides
under them) and at most ``MAX_EDGE_PX`` pixels on its longer edge.
"""

import os

import numpy as np
from numpy.typing import NDArray
from PIL import Image, ImageOps

MAX_EDGE_PX = 4000


def load_image(path: str | os.PathLike[str]) -> NDArray[np.float32]:
    """Load the image at ``path`` as an ``(H, W, 3)`` float32 array with values in 0..1.

    The steps run in this order: apply the EXIF orientation, composite any transparency over
    white, convert to RGB, then shrink with Lanczos resampling until neither edge is longer than
    ``MAX_EDGE_PX``. A smaller image is not enlarged. 16-bit greyscale skips the transparency and
    RGB steps: it is scaled to 0..1, shrunk as floats and repeated into three channels. A float
    image raises ``ValueError``. Errors from Pillow, such as a missing or unreadable file, are not
    caught.
    """
    with Image.open(path) as opened:
        if opened.mode == "F":
            raise ValueError(
                f"unsupported image mode {opened.mode!r}: float images have no defined white level"
            )
        im = ImageOps.exif_transpose(opened)
        if im.mode in ("I;16", "I;16B", "I;16L", "I;16N", "I"):
            # convert("RGB") would clip these to 255, so scale to 0..1 and shrink as floats.
            scaled = np.asarray(im, dtype=np.float32)
            scaled /= 65535.0
            np.clip(scaled, 0, 1, out=scaled)
            im = Image.fromarray(scaled)
            im.thumbnail((MAX_EDGE_PX, MAX_EDGE_PX), Image.Resampling.LANCZOS)
            grey = np.clip(np.asarray(im, dtype=np.float32), 0, 1)  # Lanczos can overshoot
            return np.repeat(grey[:, :, None], 3, axis=2)
        # True for an alpha channel, a palette with alpha and a PNG colour key alike.
        if im.has_transparency_data:
            rgba = im.convert("RGBA")
            im = Image.alpha_composite(Image.new("RGBA", rgba.size, "white"), rgba)
        im = im.convert("RGB")
        im.thumbnail((MAX_EDGE_PX, MAX_EDGE_PX), Image.Resampling.LANCZOS)
        rgb = np.asarray(im, dtype=np.float32)
    rgb /= 255.0
    return rgb

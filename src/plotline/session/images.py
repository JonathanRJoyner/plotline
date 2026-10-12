"""Reading image files as the float RGB arrays the engine works on.

Pillow decodes the file, so every format it reads works. The array is upright (the EXIF
orientation is applied, so a phone photo is not sideways), opaque (transparent pixels become
white, the colour of blank paper, instead of whatever colour hides under them) and at most
``MAX_EDGE_PX`` pixels on its longer edge.
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
    ``MAX_EDGE_PX``. A smaller image is not enlarged. Errors from Pillow, such as a missing or
    unreadable file, are not caught.
    """
    with Image.open(path) as opened:
        im = ImageOps.exif_transpose(opened)
        # True for an alpha channel, a palette with alpha and a PNG colour key alike.
        if im.has_transparency_data:
            rgba = im.convert("RGBA")
            im = Image.alpha_composite(Image.new("RGBA", rgba.size, "white"), rgba)
        im = im.convert("RGB")
        im.thumbnail((MAX_EDGE_PX, MAX_EDGE_PX), Image.Resampling.LANCZOS)
        rgb = np.asarray(im, dtype=np.float32)
    rgb /= 255.0
    return rgb

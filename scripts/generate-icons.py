#!/usr/bin/env python3
"""Create platform icons from assets/icons/sniplet-1024.png. Requires Pillow."""

import io
from pathlib import Path
import struct

from PIL import Image


def main():
    icons = Path(__file__).resolve().parents[1] / "assets" / "icons"
    source = Image.open(icons / "sniplet-1024.png").convert("RGBA")
    if source.size != (1024, 1024):
        raise ValueError("The source icon must be 1024 by 1024 pixels")

    sizes = [16, 24, 32, 48, 64, 128, 256, 512]
    for size in sizes:
        source.resize((size, size), Image.Resampling.LANCZOS).save(
            icons / f"sniplet-{size}.png"
        )
    source.save(
        icons / "sniplet.ico",
        sizes=[(size, size) for size in [16, 20, 24, 32, 40, 48, 64, 128, 256]],
    )

    # ICNS stores PNG images for normal and Retina display sizes.
    chunks = []
    for kind, size in [
        (b"icp4", 16), (b"icp5", 32), (b"icp6", 64),
        (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024),
        (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512),
    ]:
        png = io.BytesIO()
        source.resize((size, size), Image.Resampling.LANCZOS).save(png, format="PNG")
        data = png.getvalue()
        chunks.append(kind + struct.pack(">I", len(data) + 8) + data)
    data = b"".join(chunks)
    (icons / "sniplet.icns").write_bytes(b"icns" + struct.pack(">I", len(data) + 8) + data)


if __name__ == "__main__":
    main()

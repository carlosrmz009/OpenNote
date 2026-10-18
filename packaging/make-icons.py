#!/usr/bin/env python3
"""Makes the app icons (icon.png and icon.ico) from the artwork in packaging/glover-new.png.

    python3 packaging/make-icons.py [path/to/artwork.png]

The artwork is a rounded tile on a black backdrop. This crops to the tile and makes
everything outside it transparent, so the icon has no black corners on any system.
Needs Pillow (pip install pillow).
"""
import sys
from collections import deque
from pathlib import Path

from PIL import Image, ImageFilter

here = Path(__file__).parent
source = Path(sys.argv[1]) if len(sys.argv) > 1 else here / "glover-new.png"
art = Image.open(source).convert("RGBA")
w, h = art.size
px = art.load()

# The backdrop is pure black (or already transparent); the tile is a lighter dark grey.
# Flood in from the edges through anything that dark, so the black inside the face's eyes,
# which the tile surrounds, is never reached.
def backdrop(x, y):
    r, g, b, a = px[x, y]
    return a < 16 or max(r, g, b) < 9

outside = Image.new("L", (w, h), 0)
mask = outside.load()
queue = deque()
for x in range(w):
    queue.extend([(x, 0), (x, h - 1)])
for y in range(h):
    queue.extend([(0, y), (w - 1, y)])
while queue:
    x, y = queue.popleft()
    if not (0 <= x < w and 0 <= y < h) or mask[x, y] or not backdrop(x, y):
        continue
    mask[x, y] = 255
    queue.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)])

# Soft edge, so the corners are not jagged.
alpha = outside.point(lambda v: 255 - v).filter(ImageFilter.GaussianBlur(1.2))
art.putalpha(alpha)
art = art.crop(art.getbbox())

side = max(art.size)
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.alpha_composite(art, ((side - art.width) // 2, (side - art.height) // 2))
square = square.resize((1024, 1024), Image.LANCZOS)
square.save(here / "icon.png")
square.resize((256, 256), Image.LANCZOS).save(
    here / "icon.ico", sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)]
)
print(f"wrote {here / 'icon.png'} and {here / 'icon.ico'}")

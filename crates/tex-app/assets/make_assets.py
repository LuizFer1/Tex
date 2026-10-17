"""Builds the app icon and the mascot crops from the design handoff.

Usage: python make_assets.py <handoff assets folder>
Writes tex.ico next to this script and the PNGs to ../ui/assets.
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
UI_ASSETS = HERE.parent / "ui" / "assets"


def rounded(image, size, radius_ratio=0.22):
    image = image.resize((size, size), Image.LANCZOS)
    scale = 4
    mask = Image.new("L", (size * scale, size * scale), 0)
    radius = int(size * radius_ratio * scale)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, size * scale - 1, size * scale - 1), radius=radius, fill=255
    )
    image = image.convert("RGBA")
    image.putalpha(mask.resize((size, size), Image.LANCZOS))
    return image


def square(image, y_ratio):
    width, height = image.size
    side = min(width, height)
    left = (width - side) // 2
    top = int((height - side) * y_ratio)
    return image.crop((left, top, left + side, top + side))


def crop_ratio(image, ratio_w, ratio_h, y_ratio):
    width, height = image.size
    crop_height = int(width * ratio_h / ratio_w)
    top = int((height - crop_height) * y_ratio)
    return image.crop((0, top, width, top + crop_height))


def main(source):
    source = Path(source)
    UI_ASSETS.mkdir(parents=True, exist_ok=True)

    icon = square(Image.open(source / "tex-icon.png").convert("RGB"), 0.5)
    sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    frames = [rounded(icon, size) for size in sizes]
    frames[-1].save(HERE / "tex.ico", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
    rounded(icon, 144).save(UI_ASSETS / "app-icon.png")
    rounded(icon, 32).save(UI_ASSETS / "tray.png")

    search = square(Image.open(source / "tex-search.png").convert("RGB"), 0.4)
    search.resize((264, 264), Image.LANCZOS).save(UI_ASSETS / "mascot-search.png")
    clipboard = square(Image.open(source / "tex-clipboard.png").convert("RGB"), 0.3)
    clipboard.resize((264, 264), Image.LANCZOS).save(UI_ASSETS / "mascot-clipboard.png")
    banner = crop_ratio(Image.open(source / "tex-full.png").convert("RGB"), 132, 150, 0.12)
    banner.resize((264, 300), Image.LANCZOS).save(UI_ASSETS / "mascot-banner.png")


if __name__ == "__main__":
    main(sys.argv[1])

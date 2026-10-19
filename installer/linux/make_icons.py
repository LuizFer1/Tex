"""Writes the hicolor PNG icons of the Linux packages from the design handoff.

Usage: python installer/linux/make_icons.py <handoff assets folder>
Uses the same crop and rounded corners as crates/tex-app/assets/make_assets.py.
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent.parent / "crates" / "tex-app" / "assets"))

from PIL import Image
from make_assets import rounded, square

SIZES = [16, 24, 32, 48, 64, 128, 256]


def main(source):
    icon = square(Image.open(Path(source) / "tex-icon.png").convert("RGB"), 0.5)
    folder = HERE / "icons"
    folder.mkdir(exist_ok=True)
    for size in SIZES:
        rounded(icon, size).save(folder / f"{size}.png")


if __name__ == "__main__":
    main(sys.argv[1])

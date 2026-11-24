#!/usr/bin/env python3
"""Regenerate docs/shots/og-cover.png, the 1200x630 social card.

The card is the hero screenshot scaled to 1200 wide and cropped from the top,
faded into the page background over its lower part, with the wordmark, the
tagline and the engine line drawn on it.

Needs Pillow and the DejaVu fonts (Debian/Ubuntu: fonts-dejavu-core).

    python3 scripts/og-cover.py            # write the card
    python3 scripts/og-cover.py --out x.png
"""

import argparse
import pathlib
import sys

from PIL import Image, ImageDraw, ImageFont

ROOT = pathlib.Path(__file__).resolve().parent.parent
HERO = ROOT / "docs/shots/hero-session.png"
OUT = ROOT / "docs/shots/og-cover.png"
FONTS = pathlib.Path("/usr/share/fonts/truetype/dejavu")

SIZE = (1200, 630)
PAGE_BG = (13, 15, 18)  # --bg in docs/index.html
FADE_START = 252  # the fade begins at 40% of the height
FADE_MAX = 0.92
FADE_CURVE = 1.5

# (font file, size, position, colour, text)
LINES = [
    ("DejaVuSansMono-Bold.ttf", 76, (48, 478), (245, 247, 250), "AGENT CONSOLE"),
    (
        "DejaVuSans.ttf",
        30,
        (48, 563),
        (217, 221, 227),
        "Direct coding agents — approve, see the proof, keep the receipts.",
    ),
    (
        "DejaVuSans.ttf",
        22,
        (50, 600),
        (139, 148, 158),
        "Claude Code, Codex & OpenCode, native · agent-console.dev",
    ),
]


def render() -> Image.Image:
    width, height = SIZE
    hero = Image.open(HERO).convert("RGB")
    scaled = hero.resize((width, round(hero.height * width / hero.width)), Image.LANCZOS)
    card = scaled.crop((0, 0, width, height))

    column = Image.new("L", (1, height))
    for y in range(FADE_START, height):
        t = FADE_MAX * ((y - FADE_START) / (height - 1 - FADE_START)) ** FADE_CURVE
        column.putpixel((0, y), round(255 * t))
    card = Image.composite(Image.new("RGB", SIZE, PAGE_BG), card, column.resize(SIZE))

    draw = ImageDraw.Draw(card)
    for font_file, size, position, colour, text in LINES:
        font = ImageFont.truetype(str(FONTS / font_file), size)
        right = position[0] + draw.textlength(text, font=font)
        if right > width - position[0]:
            sys.exit(f"og-cover: line does not fit ({right:.0f}px of {width}): {text}")
        draw.text(position, text, font=font, fill=colour)
    return card


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=pathlib.Path, default=OUT)
    args = parser.parse_args()
    render().save(args.out, optimize=True)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()

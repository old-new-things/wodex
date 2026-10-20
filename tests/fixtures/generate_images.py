#!/usr/bin/env python3
"""Regenerate synthetic image fixtures; Pillow is needed only for regeneration."""

from pathlib import Path

from PIL import Image


def main():
  directory = Path(__file__).resolve().parent
  image = Image.new("RGB", (256, 128))
  colors = ((242, 30, 42), (24, 220, 66), (28, 50, 240), (242, 216, 30))
  for y in range(128):
    for x in range(256):
      image.putpixel((x, y), colors[(y >= 64) * 2 + (x >= 128)])
  image.save(
    directory / "image-quadrants.jpg", format="JPEG",
    quality=95, subsampling=0, optimize=False,
  )
  image.save(
    directory / "image-progressive.jpg", format="JPEG",
    quality=95, subsampling=0, optimize=False, progressive=True,
  )
  alpha = image.convert("RGBA")
  for y in range(48, 80):
    for x in range(112, 144):
      alpha.putpixel((x, y), (0, 0, 0, 0))
  alpha.save(directory / "image-quadrants.png", format="PNG", optimize=False)
  Image.new("L", (3, 2), 112).save(directory / "image-gray.png", format="PNG")
  Image.new("L", (3, 2), 112).save(directory / "image-gray.jpg", format="JPEG", quality=95)
  indexed = Image.new("P", (3, 2))
  indexed.putpalette([255, 0, 0, 0, 255, 0] + [0] * 762)
  indexed.putdata([0, 1, 0, 1, 0, 1])
  indexed.save(directory / "image-indexed.png", format="PNG", transparency=bytes([255, 0]))


if __name__ == "__main__":
  main()

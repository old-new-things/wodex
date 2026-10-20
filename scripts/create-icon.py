#!/usr/bin/env python3
"""Regenerate the small geometric app mark; requires Pillow only for JPEG."""
from pathlib import Path
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parent.parent
assets = root / "assets"
assets.mkdir(exist_ok=True)
points = [(67, 87), (89, 165), (128, 112), (165, 165), (187, 87)]
svg_points = " ".join(f"{x},{y}" for x, y in points)
(assets / "icon.svg").write_text(
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">\n'
  '  <rect width="256" height="256" rx="48" fill="#12161c"/>\n'
  '  <rect x="28" y="28" width="200" height="188" rx="44" fill="#bbef81"/>\n'
  '  <path d="M64 196v40l48-40" fill="#bbef81"/>\n'
  f'  <polyline points="{svg_points}" fill="none" stroke="#1b2819" '
  'stroke-width="16" stroke-linecap="round" stroke-linejoin="round"/>\n'
  '</svg>\n', encoding="utf-8")
scale = 4
image = Image.new("RGB", (256 * scale, 256 * scale), "#12161c")
draw = ImageDraw.Draw(image)
draw.rounded_rectangle((28*scale, 28*scale, 228*scale, 216*scale), radius=44*scale, fill="#bbef81")
draw.polygon([(64*scale, 196*scale), (64*scale, 236*scale), (112*scale, 196*scale)], fill="#bbef81")
draw.line([(x*scale, y*scale) for x, y in points], fill="#1b2819", width=16*scale, joint="curve")
for x, y in (points[0], points[-1]):
  draw.ellipse(((x-8)*scale, (y-8)*scale, (x+8)*scale, (y+8)*scale), fill="#1b2819")
image.resize((256, 256), Image.Resampling.LANCZOS).save(assets / "icon.jpg", quality=95)

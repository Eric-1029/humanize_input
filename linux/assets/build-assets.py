#!/usr/bin/env python3
"""Regenerates the compact assets embedded in the app from assets/src/.

- fonts/Saira-Latin.ttf: Saira subset to the characters the interface uses
  (Latin, Latin-1, general punctuation, arrows, ×). Both variation axes
  (wght, wdth) are kept. Saira declares no Reserved Font Name, so the
  subset may keep its name (SIL OFL 1.1).
- ground/contour-{a,b}-{light,dark}.svgz: contour lines tinted for each
  theme at 9 % opacity, coordinates rounded to whole pixels, gzipped.

Run from anywhere: python3 linux/assets/build-assets.py  (needs fonttools)
"""
import gzip
import re
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
UNICODES = "U+0020-007E,U+00A0-00FF,U+2010-2027,U+2030-203A,U+2190-2193,U+2212,U+00D7"

subprocess.run(
    ["pyftsubset", str(HERE / "src/Saira.ttf"), f"--unicodes={UNICODES}", "--layout-features=*",
     "--name-IDs=*", f"--output-file={HERE / 'fonts/Saira-Latin.ttf'}"],
    check=True,
)

for name in ["contour-a", "contour-b"]:
    svg = (HERE / f"src/{name}.svg").read_text()
    svg = re.sub(r"\d+\.\d+", lambda m: str(round(float(m.group(0)))), svg)
    for theme, colour in [("light", "#161616"), ("dark", "#EDEDED")]:
        tinted = svg.replace('stroke="#000"', f'stroke="{colour}" stroke-opacity="0.09"', 1)
        # mtime=0 keeps the output byte-identical between runs.
        (HERE / f"ground/{name}-{theme}.svgz").write_bytes(gzip.compress(tinted.encode(), 9, mtime=0))

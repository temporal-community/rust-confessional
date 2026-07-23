#!/usr/bin/env python3
"""Generate the dashboard confession QR as a self-contained SVG.

The QR encodes an ``sms:`` link so scanning it opens the phone's Messages app
addressed to the demo number. A 🦀 is drawn in the centre at error-correction
level H (~30% recovery), so the logo does not stop it scanning.

The committed ``static/confess-qr.svg`` intentionally uses a PLACEHOLDER number.
Generate your own before a live event and keep that copy out of git (see the
README): a real number committed to a public repo invites spam and per-message
charges, and lives in history forever.

Usage:
    pip install segno
    python tools/gen_qr.py "+15551234567"
"""

import os
import sys

import segno

PLACEHOLDER = "+15551234567"


def main() -> None:
    number = sys.argv[1] if len(sys.argv) > 1 else PLACEHOLDER
    out = os.path.join(os.path.dirname(__file__), "..", "static", "confess-qr.svg")

    qr = segno.make(f"sms:{number}", error="h")
    matrix = qr.matrix
    n = len(matrix)
    border = 4
    scale = 12
    size = (n + 2 * border) * scale

    rects = []
    for r, row in enumerate(matrix):
        for c, val in enumerate(row):
            if val:
                x = (c + border) * scale
                y = (r + border) * scale
                rects.append(f'<rect x="{x}" y="{y}" width="{scale}" height="{scale}"/>')
    dark = "".join(rects)

    logo = size * 0.20
    pad = logo * 1.20
    cx = cy = size / 2
    font = logo * 1.05

    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {size} {size}" role="img" '
        f'aria-label="Scan to text your confession">'
        f'<rect width="{size}" height="{size}" fill="#ffffff"/>'
        f'<g fill="#0b0b0f">{dark}</g>'
        f'<rect x="{cx - pad / 2:.1f}" y="{cy - pad / 2:.1f}" width="{pad:.1f}" '
        f'height="{pad:.1f}" rx="{pad * 0.18:.1f}" fill="#ffffff"/>'
        f'<text x="{cx:.1f}" y="{cy:.1f}" font-size="{font:.1f}" text-anchor="middle" '
        f'dominant-baseline="central">\U0001f980</text>'
        f"</svg>"
    )

    with open(os.path.abspath(out), "w", encoding="utf-8") as f:
        f.write(svg)

    tag = " (PLACEHOLDER)" if number == PLACEHOLDER else ""
    print(f"wrote static/confess-qr.svg for sms:{number}{tag} — QR v{qr.version}, {size}x{size}px, ECC=H")


if __name__ == "__main__":
    main()

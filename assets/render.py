#!/usr/bin/env python3
"""Rasterizes the brand SVGs into PNG with the same Chromium that shows the Desktop (cairosvg cannot do SVG masks).
Run inside a Playwright container: python3 render.py <directory with the svg files and render-manifest.json>."""
import base64, json, sys
from pathlib import Path
from playwright.sync_api import sync_playwright

d = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
jobs = json.loads((d / "render-manifest.json").read_text())
with sync_playwright() as p:
    b = p.chromium.launch()
    for svg, png, w, h in jobs:
        data = base64.b64encode((d / svg).read_bytes()).decode()
        pg = b.new_context(viewport={"width": w, "height": h}, device_scale_factor=1).new_page()
        pg.set_content(f'<body style="margin:0;background:transparent"><img id="i" src="data:image/svg+xml;base64,{data}" width="{w}" height="{h}" style="display:block"></body>')
        pg.wait_for_timeout(150)
        pg.locator("#i").screenshot(path=str(d / png), omit_background=True)
        pg.context.close()
        print("png:", png, f"{w}x{h}")
    b.close()

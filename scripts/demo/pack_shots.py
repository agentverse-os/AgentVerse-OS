#!/usr/bin/env python3
"""Сборка кадров README из PNG съёмки (shots.py) в assets/shots/*.webp: десктоп и планшет — как есть (retina → половина),
телефоны phone-home + phone-store склеиваются парой с зазором. Запуск: python3 scripts/demo/pack_shots.py <каталог PNG> [assets/shots]"""
import sys, pathlib
from PIL import Image

SRC = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "out")
DST = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else "assets/shots"); DST.mkdir(parents=True, exist_ok=True)
Q = 82

def save(im: Image.Image, name: str):
    im.convert("RGB").save(DST / f"{name}.webp", "WEBP", quality=Q, method=6); print("saved", name, im.size)

def half(name: str) -> Image.Image:
    im = Image.open(SRC / f"{name}.png"); return im.resize((im.width // 2, im.height // 2), Image.LANCZOS)

for n in ("desktop-dark", "desktop-light", "tablet-dark"):
    save(half(n), n)

# телефоны: два кадра 390×844 @3x → по 585 px шириной, зазор 40 px, фон прозрачный не нужен — берём цвет боковин из кадра
a = Image.open(SRC / "phone-home.png"); b = Image.open(SRC / "phone-store.png")
w = a.width // 2; h = a.height // 2
a = a.resize((w, h), Image.LANCZOS); b = b.resize((w, h), Image.LANCZOS)
gap = 40
pair = Image.new("RGB", (w * 2 + gap, h), a.convert("RGB").getpixel((2, 2)))
pair.paste(a, (0, 0)); pair.paste(b, (w + gap, 0))
save(pair, "phone")

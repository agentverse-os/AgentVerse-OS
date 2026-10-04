#!/usr/bin/env python3
"""Обои-«сцены» для темы Umbrel: живописные пейзажи в духе фотообоев umbrelOS (закат, тропические сумерки, дюны, сияние), нарисованные
растром — Pillow без внешних ассетов и без чужих фотографий. Каждая сцена несёт brand-цвет (акцент темы, как в umbrelOS: у
каждых обоев свой brandColorHsl). python3 assets/scenes.py [имя…] [--install] [--png] → assets/scene-<имя>-preview.jpg (и PNG при --png), при --install —
desktop/public/wallpaper-umbrel-<имя>.webp. Зависимость: Pillow."""
import colorsys, math, random, sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
W, H = 2560, 1440

def hsl(h, s, l):
    r, g, b = colorsys.hls_to_rgb(h / 360, l / 100, s / 100)
    return "#%02X%02X%02X" % (round(r * 255), round(g * 255), round(b * 255))

def rgb(c):
    c = c.lstrip("#"); return tuple(int(c[i:i + 2], 16) for i in (0, 2, 4))

def lerp(a, b, t): return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))

def vgrad(w, h, stops):
    """Вертикальный градиент по остановкам [(pos 0..1, '#rrggbb'), …]."""
    im = Image.new("RGB", (w, h)); d = ImageDraw.Draw(im)
    stops = [(p, rgb(c)) for p, c in stops]
    for y in range(h):
        t = y / (h - 1)
        for i in range(len(stops) - 1):
            p0, c0 = stops[i]; p1, c1 = stops[i + 1]
            if p0 <= t <= p1:
                d.line([(0, y), (w, y)], fill=lerp(c0, c1, (t - p0) / max(p1 - p0, 1e-6))); break
    return im

def layer_for(base, color):
    """Прозрачный слой, залитый цветом элемента: при размытии альфа-канал тянет цвет из прозрачных пикселей, и чёрная подложка
    дала бы тёмный ореол вокруг светлых пятен."""
    return Image.new("RGBA", base.size, rgb(color) + (0,))

def glow(base, cx, cy, rx, ry, color, alpha, blur):
    """Мягкое свечение: эллипс на слое, размытие, наложение."""
    layer = layer_for(base, color); d = ImageDraw.Draw(layer)
    d.ellipse([cx - rx, cy - ry, cx + rx, cy + ry], fill=rgb(color) + (round(alpha * 255),))
    layer = layer.filter(ImageFilter.GaussianBlur(blur))
    base.alpha_composite(layer)

def ridge(w, y0, amp, seg, rnd, smooth=0.55):
    """Линия хребта: срединное смещение — крупные формы плюс мелкая шероховатость."""
    pts = [y0 + rnd.uniform(-amp, amp) for _ in range(seg + 1)]
    for _ in range(3):
        pts = [pts[0]] + [(pts[i - 1] + pts[i] * 2 + pts[i + 1]) / 4 for i in range(1, seg)] + [pts[-1]]
    out = []
    for i in range(seg + 1):
        out.append((w * i / seg, pts[i]))
    fine = []
    for i in range(seg):
        (x0, ya), (x1, yb) = out[i], out[i + 1]
        n = 12
        for k in range(n):
            t = k / n; fine.append((x0 + (x1 - x0) * t, ya + (yb - ya) * t + rnd.uniform(-amp, amp) * (1 - smooth) * 0.12))
    fine.append(out[-1])
    return fine

def mountains(base, y0, amp, color, alpha, seed, seg=18, blur=0):
    rnd = random.Random(seed)
    layer = layer_for(base, color); d = ImageDraw.Draw(layer)
    pts = ridge(base.width, y0, amp, seg, rnd)
    d.polygon([(0, base.height), *pts, (base.width, base.height)], fill=rgb(color) + (round(alpha * 255),))
    if blur: layer = layer.filter(ImageFilter.GaussianBlur(blur))
    base.alpha_composite(layer)

def treeline(base, y0, height, color, alpha, seed, density=6, blur=0):
    """Ёлки: зубчатая кромка из треугольников разной высоты."""
    rnd = random.Random(seed)
    layer = layer_for(base, color); d = ImageDraw.Draw(layer)
    x = -20
    while x < base.width + 20:
        hgt = height * rnd.uniform(0.45, 1.0); wid = hgt * rnd.uniform(0.35, 0.6)
        yb = y0 + rnd.uniform(-height * 0.15, height * 0.15)
        d.polygon([(x - wid / 2, yb + 40), (x, yb - hgt), (x + wid / 2, yb + 40)], fill=rgb(color) + (round(alpha * 255),))
        x += wid * rnd.uniform(0.35, 0.7) * density / 6
    d.rectangle([0, y0 + 20, base.width, base.height], fill=rgb(color) + (round(alpha * 255),))
    if blur: layer = layer.filter(ImageFilter.GaussianBlur(blur))
    base.alpha_composite(layer)

def band(base, y, thickness, color, alpha, blur):
    layer = layer_for(base, color); d = ImageDraw.Draw(layer)
    d.rectangle([-blur * 2, y - thickness / 2, base.width + blur * 2, y + thickness / 2], fill=rgb(color) + (round(alpha * 255),))
    base.alpha_composite(layer.filter(ImageFilter.GaussianBlur(blur)))

def streaks(base, y_range, color, alpha, seed, n=9, blur=14):
    """Перистые облака: длинные тонкие эллипсы."""
    rnd = random.Random(seed)
    layer = layer_for(base, color); d = ImageDraw.Draw(layer)
    for _ in range(n):
        cx = rnd.uniform(-200, base.width + 200); cy = rnd.uniform(*y_range)
        rx = rnd.uniform(300, 900); ry = rnd.uniform(6, 22)
        d.ellipse([cx - rx, cy - ry, cx + rx, cy + ry], fill=rgb(color) + (round(alpha * rnd.uniform(0.5, 1) * 255),))
    base.alpha_composite(layer.filter(ImageFilter.GaussianBlur(blur)))

def vignette(base, strength=0.35):
    layer = Image.new("RGBA", base.size, (0, 0, 0, 0)); d = ImageDraw.Draw(layer)
    w, h = base.size
    d.rectangle([0, 0, w, h], fill=(0, 0, 0, round(strength * 255)))
    d.ellipse([-w * 0.1, -h * 0.25, w * 1.1, h * 1.25], fill=(0, 0, 0, 0))
    base.alpha_composite(layer.filter(ImageFilter.GaussianBlur(220)))

def grain(base, amount=6, seed=1):
    """Лёгкое зерно, чтобы градиенты не полосили."""
    rnd = random.Random(seed)
    noise = Image.effect_noise(base.size, amount).convert("L")
    layer = Image.merge("RGBA", (noise, noise, noise, Image.new("L", base.size, 18)))
    base.alpha_composite(Image.blend(Image.new("RGBA", base.size, (128, 128, 128, 18)), layer, 1.0))

# ---------------------------------------------------------------------------------------------------------------- сцены

def peak_ridge(w, y0, amp, seg, rnd, peak_x, peak_h, peak_w):
    """Хребет с доминантной вершиной: базовая линия плюс треугольный пик с шероховатыми склонами."""
    pts = ridge(w, y0, amp, seg, rnd)
    out = []
    for x, y in pts:
        d = abs(x - peak_x) / peak_w
        if d < 1:
            y -= peak_h * (1 - d) ** 1.3 * (1 + rnd.uniform(-0.08, 0.08))
        out.append((x, y))
    return out

def ridge_layer(base, pts, color, alpha, blur=0):
    layer = layer_for(base, color); d = ImageDraw.Draw(layer)
    d.polygon([(0, base.height), *pts, (base.width, base.height)], fill=rgb(color) + (round(alpha * 255),))
    if blur: layer = layer.filter(ImageFilter.GaussianBlur(blur))
    base.alpha_composite(layer)

def palm(layer_draw, x, yb, height, lean, color, rnd):
    """Силуэт пальмы: изогнутый ствол и веер листьев."""
    pts = []
    for i in range(41):
        t = i / 40
        pts.append((x + lean * t * t * height * 0.9, yb - t * height))
    for i in range(40):
        layer_draw.line([pts[i], pts[i + 1]], fill=color, width=max(3, int(14 * (1 - t) + 4)) if (t := i / 40) is not None else 6)
    tx, ty = pts[-1]
    for k in range(9):
        a = math.radians(-160 + k * 40 + rnd.uniform(-8, 8)); ln = height * rnd.uniform(0.28, 0.42); wd = ln * 0.16
        dx, dy = math.cos(a), math.sin(a) * 0.75 + 0.25
        end = (tx + dx * ln, ty + dy * ln)
        nx, ny = -dy, dx
        mid = (tx + dx * ln * 0.45 + nx * wd, ty + dy * ln * 0.45 + ny * wd)
        mid2 = (tx + dx * ln * 0.45 - nx * wd, ty + dy * ln * 0.45 - ny * wd)
        layer_draw.polygon([(tx, ty), mid, end, mid2], fill=color)

def scene_sunset():
    """Закат в горах: пылающее небо, слои хребтов в дымке с одной вершиной, туман в долине, ели на переднем плане."""
    im = vgrad(W, H, [(0, "#1B1030"), (0.2, "#4A1E4F"), (0.4, "#A0355A"), (0.55, "#E5643F"), (0.64, "#F5964B"), (0.72, "#F8B86A"), (1, "#F7C98A")]).convert("RGBA")
    glow(im, W * 0.56, H * 0.62, 1000, 300, "#FFC46B", 0.6, 140)
    glow(im, W * 0.56, H * 0.64, 420, 130, "#FFE9B0", 0.55, 60)
    streaks(im, (H * 0.1, H * 0.42), "#F2A3B0", 0.22, seed=3, n=6, blur=26)
    streaks(im, (H * 0.28, H * 0.5), "#FFCF9F", 0.25, seed=5, n=4, blur=18)
    rnd = random.Random(11)
    ridge_layer(im, peak_ridge(W, H * 0.7, 40, 16, rnd, W * 0.58, H * 0.27, W * 0.16), "#7A4173", 0.7, blur=3)   # дальний хребет с вершиной
    ridge_layer(im, ridge(W, H * 0.76, 70, 14, random.Random(23)), "#55305D", 0.85, blur=1)
    band(im, H * 0.815, 150, "#EBA8A5", 0.55, 70)                                                                # туман в долине
    ridge_layer(im, ridge(W, H * 0.84, 60, 18, random.Random(37)), "#2E1B3A", 0.95)
    treeline(im, H * 0.9, 170, "#170F20", 0.9, seed=41, density=7)
    treeline(im, H * 0.96, 120, "#0F0A15", 1.0, seed=43, density=9)
    vignette(im, 0.3); grain(im)
    return im

def scene_dusk():
    """Тропические сумерки над водой: розово-оранжевое небо, дорожка бликов, острова и пальмы на берегу."""
    hz = int(H * 0.62)
    sky = vgrad(W, hz, [(0, "#3B1560"), (0.3, "#8E2A78"), (0.55, "#D8456C"), (0.78, "#F47D4D"), (1, "#FFC15A")])
    sea = vgrad(W, H - hz, [(0, "#F9B65A"), (0.12, "#E0664C"), (0.4, "#7A2A5E"), (1, "#1A0C2E")])
    im = Image.new("RGBA", (W, H)); im.paste(sky, (0, 0)); im.paste(sea, (0, hz))
    glow(im, W * 0.6, hz - 40, 760, 240, "#FFD27A", 0.6, 120)
    glow(im, W * 0.6, hz + 30, 380, 100, "#FFF0B0", 0.55, 50)
    streaks(im, (H * 0.08, H * 0.42), "#FFB0C0", 0.22, seed=7, n=6, blur=24)
    rnd = random.Random(9); layer = layer_for(im, "#FFECB0"); d = ImageDraw.Draw(layer)
    for i in range(220):
        y = hz + 4 + (i / 220) ** 1.5 * (H - hz) * 0.92
        spread = 110 + (y - hz) * 1.1
        cx = W * 0.6 + rnd.uniform(-spread, spread); ln = rnd.uniform(30, 160) * (1 + (y - hz) / 350)
        d.line([(cx - ln / 2, y), (cx + ln / 2, y)], fill=(255, 236, 176, round(rnd.uniform(0.12, 0.55) * 255)), width=rnd.choice([2, 3, 4, 5]))
    im.alpha_composite(layer.filter(ImageFilter.GaussianBlur(1.4)))
    ridge_layer(im, peak_ridge(W, hz + 4, 22, 12, random.Random(51), W * 0.2, H * 0.1, W * 0.12), "#4B1D4F", 0.8, blur=2)   # остров слева
    ridge_layer(im, peak_ridge(W, hz + 6, 14, 10, random.Random(53), W * 0.86, H * 0.05, W * 0.09), "#5A2358", 0.7, blur=2)
    # берег и пальмы
    shore = Image.new("RGBA", im.size, (0, 0, 0, 0)); d = ImageDraw.Draw(shore)
    d.polygon([(0, H), (0, H * 0.9), (W * 0.18, H * 0.87), (W * 0.36, H * 0.93), (W * 0.62, H * 0.95), (W * 0.8, H * 0.9), (W, H * 0.92), (W, H)], fill=(18, 8, 26, 255))
    for x, hgt, lean in [(W * 0.09, H * 0.34, 0.35), (W * 0.15, H * 0.26, -0.25), (W * 0.9, H * 0.3, -0.4), (W * 0.83, H * 0.2, 0.3)]:
        palm(d, x, H * 0.9, hgt, lean, (18, 8, 26, 255), rnd)
    # кусты у воды
    for _ in range(40):
        x = rnd.uniform(0, W); r = rnd.uniform(30, 90); y = H * 0.9 + rnd.uniform(-10, 40)
        d.ellipse([x - r, y - r * 0.6, x + r, y + r * 0.6], fill=(18, 8, 26, 255))
    im.alpha_composite(shore)
    vignette(im, 0.28); grain(im)
    return im

def scene_dunes():
    """Дюны на рассвете: индиго переходит в персиковый, низкое солнце, гребни песка от дымчатых дальних до тёмных ближних."""
    im = vgrad(W, H, [(0, "#2A2F63"), (0.28, "#6A5C97"), (0.48, "#D9896E"), (0.58, "#F4B77E"), (0.66, "#FBD7A6"), (1, "#F7C98A")]).convert("RGBA")
    glow(im, W * 0.34, H * 0.6, 900, 260, "#FFE0A8", 0.7, 130)
    glow(im, W * 0.34, H * 0.61, 300, 110, "#FFF6DC", 0.8, 50)
    streaks(im, (H * 0.12, H * 0.4), "#F6C2B0", 0.2, seed=13, n=5, blur=30)
    layers = [(0.62, 110, 5, "#E7B37E", 0.75, 3), (0.68, 130, 6, "#D08C5B", 0.85, 2), (0.75, 150, 5, "#B36A3F", 0.92, 1), (0.83, 170, 6, "#8C4A2A", 0.97, 0), (0.92, 180, 5, "#5E2E1C", 1.0, 0)]
    for i, (y0, amp, seg, col, alpha, blur) in enumerate(layers):
        rnd = random.Random(140 + i)
        pts = ridge(W, H * y0, amp, seg, rnd, smooth=0.9)
        # гребень дюны: асимметрия — пологий наветренный склон и крутой подветренный
        pts = [(x, y + math.sin(x / W * math.pi * 2 + i) * amp * 0.25) for x, y in pts]
        ridge_layer(im, pts, col, alpha, blur=blur)
        # освещённая кромка вдоль гребня
        lit = layer_for(im, "#FFE7BF"); d = ImageDraw.Draw(lit)
        d.line(pts, fill=rgb("#FFE7BF") + (round(0.55 * 255),), width=max(3, 12 - i * 2))
        im.alpha_composite(lit.filter(ImageFilter.GaussianBlur(6 + i * 2)))
    vignette(im, 0.22); grain(im, 6)
    return im

def scene_aurora():
    """Северное сияние: тёмно-синяя ночь, звёзды, занавесы с вертикальной структурой, снежные хребты."""
    im = vgrad(W, H, [(0, "#040713"), (0.5, "#0A1533"), (0.8, "#0F1E44"), (1, "#0B1330")]).convert("RGBA")
    rnd = random.Random(101); layer = layer_for(im, "#FFFFFF"); d = ImageDraw.Draw(layer)
    for _ in range(1100):
        x, y = rnd.uniform(0, W), rnd.uniform(0, H * 0.8); r = rnd.choice([0.8, 1, 1, 1.2, 1.6, 2.2])
        d.ellipse([x - r, y - r, x + r, y + r], fill=(255, 255, 255, round(rnd.uniform(0.25, 1) * 255)))
    im.alpha_composite(layer)
    for k, (col, alpha, y0, amp, seed, thick) in enumerate([("#2CFFA6", 0.42, 0.3, 90, 3, 0.26), ("#22DD95", 0.3, 0.44, 110, 4, 0.22), ("#9A6BFF", 0.32, 0.2, 70, 5, 0.2), ("#5CE8FF", 0.2, 0.52, 60, 6, 0.16)]):
        rnd = random.Random(110 + seed)
        top = []
        for i in range(0, W + 41, 40):
            ph = i / W * math.pi * rnd.uniform(1.4, 2.2)
            top.append((i, H * y0 + math.sin(ph + seed) * amp + math.sin(ph * 2.7 + 1) * amp * 0.35))
        soft = layer_for(im, col); d = ImageDraw.Draw(soft)
        d.polygon(top + [(x, y + H * thick) for x, y in top][::-1], fill=rgb(col) + (round(alpha * 0.8 * 255),))
        im.alpha_composite(soft.filter(ImageFilter.GaussianBlur(46)))
        lines = layer_for(im, col); d = ImageDraw.Draw(lines)
        for x in range(-20, W + 20, 9):
            yt = top[min(max(x, 0) // 40, len(top) - 1)][1] + rnd.uniform(-20, 20)
            ln = H * thick * rnd.uniform(0.5, 1.1)
            d.line([(x, yt), (x + rnd.uniform(-14, 14), yt + ln)], fill=rgb(col) + (round(alpha * rnd.uniform(0.3, 1.0) * 255),), width=rnd.choice([1, 2, 2, 3]))
        im.alpha_composite(lines.filter(ImageFilter.GaussianBlur(5)))
    glow(im, W * 0.45, H * 0.74, 1200, 220, "#2CFFA6", 0.16, 120)
    ridge_layer(im, peak_ridge(W, H * 0.78, 80, 14, random.Random(121), W * 0.3, H * 0.12, W * 0.14), "#1A2B4E", 0.95, blur=1)
    ridge_layer(im, ridge(W, H * 0.86, 70, 16, random.Random(131)), "#0B1530", 1.0)
    band(im, H * 0.865, 40, "#E4EEFF", 0.12, 24)
    vignette(im, 0.25); grain(im, 5)
    return im

SCENES = {
    # имя → (функция, brand-цвет как в umbrelOS: hsl(h s% l%))
    "sunset": (scene_sunset, hsl(24, 90, 50)),
    "dusk": (scene_dusk, hsl(332, 84, 47)),
    "dunes": (scene_dunes, hsl(32, 100, 36)),
    "aurora": (scene_aurora, hsl(259, 100, 59)),
}

def main():
    install = "--install" in sys.argv
    only = [a for a in sys.argv[1:] if not a.startswith("--")]
    pub = HERE.parent / "desktop" / "public"
    for name, (fn, brand) in SCENES.items():
        if only and name not in only: continue
        im = fn().convert("RGB")
        if "--png" in sys.argv: im.save(HERE / f"scene-{name}.png", optimize=True)   # полноразмерный PNG в git не хранится
        prev = im.copy(); prev.thumbnail((960, 540)); prev.save(HERE / f"scene-{name}-preview.jpg", quality=85)
        if install:
            im.save(pub / f"wallpaper-umbrel-{name}.webp", quality=82, method=6)
        print(f"{name}: brand {brand}" + (f" → {pub / f'wallpaper-umbrel-{name}.webp'}" if install else ""))

if __name__ == "__main__":
    main()

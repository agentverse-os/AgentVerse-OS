#!/usr/bin/env python3
"""Бренд AgentVerse OS · знак Monolith. Генерирует SVG и PNG в assets/ (и иконки PWA в desktop/public при --install).
Запуск: python3 assets/build.py svg → SVG и render-manifest.json; затем PNG рендерит Chromium (assets/render.py в контейнере Playwright:
маски SVG cairosvg не умеет); python3 assets/build.py assemble [--install] → favicon.ico и копирование в desktop/public. Зависимость: Pillow.
Геометрия (viewBox 256): шпиль — треугольник apex (128,22), основание 92…164 на y=224, две грани по оси x=128;
кольцо — эллипс 196×56 с центром (128,170), наклон −16°; спереди кольцо отделено от шпиля прозрачным зазором (mask)."""
import os, sys, io
from pathlib import Path
from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
P = {"blue": "#5B8CFF", "violet": "#7A5CFF", "lilac": "#A2B4FF", "paper": "#F4F7FF", "night": "#0E1018", "deep": "#3B2FA8"}

def mark_parts(mono=None, simple=False, uid="m"):
    """Знак Monolith v2 (по референсу): широкий гранёный шпиль с вырезом снизу (силуэт A), внутренний клин между ножками
    и тонкое широкое кольцо под −10°. Вырез и зазор перед кольцом — маски, поэтому фон любой. Возвращает (defs, body)."""
    ring_w = 16 if simple else 9
    cut_w = ring_w + (10 if simple else 6)
    notch = "128,132 112,228 144,228" if simple else "128,118 108,228 148,228"
    wedge = "128,146 117,228 139,228"   # внутренний клин: в вырезе, с зазором до ножек
    if mono:
        fl = fr = fc = fw = rb = rf = mono; grads = ""
    else:
        fl, fr, fc, fw, rb, rf = f"url(#{uid}-gl)", f"url(#{uid}-gr)", f"url(#{uid}-gc)", f"url(#{uid}-gw)", f"url(#{uid}-rb)", f"url(#{uid}-rf)"
        grads = f"""
  <linearGradient id="{uid}-gl" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{P['paper']}"/><stop offset="0.35" stop-color="{P['lilac']}"/><stop offset="1" stop-color="{P['blue']}"/></linearGradient>
  <linearGradient id="{uid}-gr" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{P['lilac']}"/><stop offset="0.4" stop-color="{P['violet']}"/><stop offset="1" stop-color="{P['deep']}"/></linearGradient>
  <linearGradient id="{uid}-gc" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{P['paper']}"/><stop offset="1" stop-color="{P['lilac']}"/></linearGradient>
  <linearGradient id="{uid}-gw" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{P['paper']}"/><stop offset="0.5" stop-color="{P['lilac']}"/><stop offset="1" stop-color="{P['violet']}"/></linearGradient>
  <linearGradient id="{uid}-rb" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{P['deep']}"/><stop offset="1" stop-color="{P['violet']}"/></linearGradient>
  <linearGradient id="{uid}-rf" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{P['blue']}"/><stop offset="0.55" stop-color="{P['lilac']}"/><stop offset="1" stop-color="{P['violet']}"/></linearGradient>"""
    ring_cut = f'<g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="black" stroke-width="{cut_w}" stroke-linecap="round"/></g>'
    defs = f"""{grads}
  <mask id="{uid}-cut" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    <polygon points="{notch}" fill="black"/>
    {ring_cut}
  </mask>
  <mask id="{uid}-cut2" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    {ring_cut}
  </mask>"""
    if simple or mono:
        spire = f'<polygon points="128,18 62,228 194,228" fill="{fl}"/>'
    else:
        spire = (f'<polygon points="128,18 62,228 128,228" fill="{fl}"/>'
                 f'<polygon points="128,18 128,228 194,228" fill="{fr}"/>'
                 f'<polygon points="128,18 121,228 135,228" fill="{fc}" opacity="0.9"/>')
    inner = "" if simple else f'<g mask="url(#{uid}-cut2)"><polygon points="{wedge}" fill="{fw}"/></g>'
    body = f"""<g mask="url(#{uid}-cut)">
  <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 1 246 172" fill="none" stroke="{rb}" stroke-width="{ring_w}" stroke-linecap="round" opacity="0.85"/></g>
  {spire}
</g>
{inner}
<g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="{rf}" stroke-width="{ring_w}" stroke-linecap="round"/></g>"""
    return defs, body

def mark_svg(size=256, mono=None, simple=False, uid="m", bg=None, pad=0, radius=0):
    """Знак на прозрачном фоне (или на фоне bg с радиусом radius — для иконок PWA). pad — поле вокруг знака в единицах viewBox."""
    defs, body = mark_parts(mono, simple, uid)
    vb = f"{-pad} {-pad} {256 + 2 * pad} {256 + 2 * pad}"
    bg_rect = f'<rect x="{-pad}" y="{-pad}" width="{256 + 2 * pad}" height="{256 + 2 * pad}" rx="{radius}" fill="{bg}"/>' if bg else ""
    glow = f'<ellipse cx="128" cy="150" rx="120" ry="110" fill="url(#{uid}-glow)" opacity="0.55"/><radialGradient id="{uid}-glow"><stop offset="0" stop-color="{P["violet"]}" stop-opacity="0.55"/><stop offset="1" stop-color="{P["violet"]}" stop-opacity="0"/></radialGradient>' if bg else ""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" width="{size}" height="{size}" role="img" aria-label="AgentVerse OS">
<defs>{defs}</defs>
{bg_rect}{glow}
{body}
</svg>
"""

FONT = "'Space Grotesk','Segoe UI',system-ui,sans-serif"

def font_style():
    """Space Grotesk (OFL, assets/fonts/SpaceGrotesk-latin.woff2, вариативный 300…700) встраивается в SVG как data-URI:
    SVG внутри <img> внешние шрифты не грузит, а в контейнере рендера Space Grotesk нет (sans-serif там — WenQuanYi)."""
    import base64
    f = HERE / "fonts" / "SpaceGrotesk-latin.woff2"
    if not f.exists(): return ""
    return f"<style>@font-face{{font-family:'Space Grotesk';font-weight:300 700;src:url(data:font/woff2;base64,{base64.b64encode(f.read_bytes()).decode()}) format('woff2')}}</style>"

def lockup_parts(vertical=False, ink=P["paper"], accent=P["lilac"], muted="#8E97B8", uid="lk"):
    """Внутренности лок-апа (знак и надпись) и размер его системы координат: горизонтальный 760×256, вертикальный 420×420."""
    m = mark_svg(uid=uid).split("\n", 1)[1].rsplit("</svg>", 1)[0]  # внутренности знака без <svg>
    if vertical:
        body = f"""<g transform="translate(82 0)">{m}</g>
<text x="210" y="326" text-anchor="middle" font-family="{FONT}" font-weight="600" font-size="46" letter-spacing="-1" fill="{ink}">AgentVerse <tspan fill="{accent}">OS</tspan></text>
<text x="210" y="362" text-anchor="middle" font-family="{FONT}" font-weight="500" font-size="12" letter-spacing="3.2" fill="{muted}">ONE SYSTEM. MANY WORLDS.</text>"""
        return body, 420, 420
    body = f"""{m}
<text x="272" y="140" font-family="{FONT}" font-weight="600" font-size="64" letter-spacing="-1.3" fill="{ink}">AgentVerse <tspan fill="{accent}">OS</tspan></text>
<text x="274" y="180" font-family="{FONT}" font-weight="500" font-size="14" letter-spacing="3.4" fill="{muted}">ONE SYSTEM. MANY WORLDS.</text>"""
    return body, 760, 256

def lockup_svg(vertical=False, ink=P["paper"], accent=P["lilac"], muted="#8E97B8"):
    """Лок-ап с надписью; шрифт встроен (font_style), поэтому файл рендерится одинаково везде."""
    body, vw, vh = lockup_parts(vertical, ink, accent, muted)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {vw} {vh}" width="{vw}" height="{vh}" role="img" aria-label="AgentVerse OS">
{font_style()}
{body}
</svg>
"""

def stars(w, h, avoid, light, rnd):
    """Звёзды с редкими бликами-крестиками; avoid(x, y) — где звёзд не ставить (знак, надпись)."""
    star = "#2E3A6E" if light else "#DDE3FF"
    parts = []
    for _ in range(int(w * h / 9000)):
        x, y = rnd.uniform(0, w), rnd.uniform(0, h)
        if avoid(x, y): continue
        rad = rnd.choice([0.6, 0.8, 1.0, 1.2, 1.6, 2.0])
        op = rnd.uniform(0.18, 0.7) if not light else rnd.uniform(0.15, 0.45)
        parts.append(f'<circle cx="{x:.0f}" cy="{y:.0f}" r="{rad}" fill="{star}" opacity="{op:.2f}"/>')
        if rad >= 1.6 and rnd.random() < 0.45:   # блик-крестик у ярких
            L = rad * 6
            parts.append(f'<path d="M {x:.0f} {y - L:.0f} V {y + L:.0f} M {x - L:.0f} {y:.0f} H {x + L:.0f}" stroke="{star}" stroke-width="0.7" opacity="{op * 0.6:.2f}"/>')
    return parts

def constellation(pts, light, rnd):
    """Созвездие: точки со случайным сдвигом и тонкие связи между соседними."""
    star = "#2E3A6E" if light else "#DDE3FF"; line = "#7A5CFF" if light else "#A2B4FF"
    pts = [(x + rnd.uniform(-12, 12), y + rnd.uniform(-10, 10)) for x, y in pts]
    parts = [f'<line x1="{x1:.0f}" y1="{y1:.0f}" x2="{x2:.0f}" y2="{y2:.0f}" stroke="{line}" stroke-width="0.7" opacity="{0.22 if not light else 0.3}"/>' for (x1, y1), (x2, y2) in zip(pts, pts[1:])]
    parts += [f'<circle cx="{x:.0f}" cy="{y:.0f}" r="1.8" fill="{star}" opacity="0.85"/>' for x, y in pts]
    return parts

def orbits(cx, cy, specs, light, moon=None):
    """Тонкие орбиты с наклоном кольца знака (−10°); specs — (rx, ry, толщина, непрозрачность); moon=(номер орбиты, угол°) — «луна»."""
    import math
    line = "#7A5CFF" if light else "#A2B4FF"; dot = "#7A5CFF" if light else "#5B8CFF"
    parts = [f'<ellipse cx="{cx:.0f}" cy="{cy:.0f}" rx="{rx:.0f}" ry="{ry:.0f}" fill="none" stroke="{line}" stroke-width="{sw:.2f}" opacity="{op}" transform="rotate(-10 {cx:.0f} {cy:.0f})"/>' for rx, ry, sw, op in specs]
    if moon:
        k, deg = moon; rx, ry = specs[k][0], specs[k][1]; a = math.radians(deg); mx, my = cx + rx * math.cos(a), cy + ry * math.sin(a); r = max(4.0, ry * 0.06)
        parts.append(f'<g transform="rotate(-10 {cx:.0f} {cy:.0f})"><circle cx="{mx:.0f}" cy="{my:.0f}" r="{r:.1f}" fill="{dot}" opacity="0.9"/><circle cx="{mx:.0f}" cy="{my:.0f}" r="{r * 2.5:.1f}" fill="{dot}" opacity="0.18"/></g>')
    return parts

def cosmos(w, h, cx, cy, r, light=False, seed=7):
    """Космос вокруг знака: звёзды (не на знаке), три орбиты вокруг него с «луной», созвездие слева внизу."""
    import random
    rnd = random.Random(seed)
    parts = stars(w, h, lambda x, y: (x - cx) ** 2 + (y - cy) ** 2 < (r * 0.9) ** 2, light, rnd)
    parts += orbits(cx, cy + r * 0.28, [(r * 1.35, r * 0.34, 1.1, 0.16), (r * 1.85, r * 0.47, 0.85, 0.10), (r * 2.4, r * 0.6, 0.6, 0.06)], light, moon=(1, 215))
    pts = [(w * 0.10, h * 0.62), (w * 0.16, h * 0.70), (w * 0.23, h * 0.66), (w * 0.27, h * 0.76), (w * 0.34, h * 0.72), (w * 0.31, h * 0.84)]
    parts += constellation(pts, light, rnd)
    return "\n".join(parts)

def cosmos_lockup(w, h, box, light=False, seed=7, portrait=False):
    """Космос для обоев с лок-апом: звёзды не заходят на надпись, орбиты обнимают лок-ап целиком, созвездия по свободным углам."""
    import random
    rnd = random.Random(seed)
    x0, y0, x1, y1 = box; mx, my = w * 0.04, h * 0.05
    parts = stars(w, h, lambda x, y: x0 - mx < x < x1 + mx and y0 - my < y < y1 + my, light, rnd)
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2 + h * 0.03
    specs = [(w * 0.58, h * 0.27, 1.1, 0.16), (w * 0.72, h * 0.33, 0.85, 0.10), (w * 0.90, h * 0.40, 0.6, 0.06)] if portrait else [(w * 0.42, h * 0.26, 1.1, 0.16), (w * 0.52, h * 0.32, 0.85, 0.10), (w * 0.64, h * 0.40, 0.6, 0.06)]
    parts += orbits(cx, cy, specs, light, moon=(0, 205))
    if portrait: cs = [[(0.56, 0.06), (0.63, 0.11), (0.71, 0.08), (0.77, 0.15), (0.86, 0.12), (0.90, 0.20)], [(0.06, 0.36), (0.12, 0.42), (0.20, 0.38), (0.24, 0.46), (0.32, 0.43), (0.34, 0.48)]]
    else: cs = [[(0.22, 0.09), (0.28, 0.17), (0.34, 0.12), (0.37, 0.24), (0.43, 0.20), (0.41, 0.30)], [(0.56, 0.74), (0.62, 0.82), (0.68, 0.78), (0.72, 0.90), (0.78, 0.86), (0.80, 0.94)]]
    for c in cs: parts += constellation([(w * a, h * b) for a, b in c], light, rnd)
    return "\n".join(parts)

AB_C = ("#5B8CFF", "#7A5CFF", "#A2B4FF")
def ab_defs():
    c1, c2, c3 = AB_C
    return f"""<defs>
  <linearGradient id="ab-shard" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{c3}" stop-opacity="0.9"/><stop offset="1" stop-color="{c2}" stop-opacity="0.15"/></linearGradient>
  <radialGradient id="ab-planet" cx="0.35" cy="0.3" r="0.8"><stop offset="0" stop-color="{c3}"/><stop offset="0.6" stop-color="{c1}"/><stop offset="1" stop-color="#2A2470"/></radialGradient>
  <linearGradient id="ab-streak" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{c3}" stop-opacity="0"/><stop offset="0.5" stop-color="{c3}" stop-opacity="0.9"/><stop offset="1" stop-color="{c3}" stop-opacity="0"/></linearGradient>
  <linearGradient id="ab-orbit" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{c1}" stop-opacity="0"/><stop offset="0.5" stop-color="{c1}" stop-opacity="0.9"/><stop offset="1" stop-color="{c2}" stop-opacity="0"/></linearGradient>
</defs>"""

def planet(x, y, pr, op, ring=False, halo=2.6):
    c1, c2, c3 = AB_C
    out = []
    if ring: out.append(f'<ellipse cx="{x:.0f}" cy="{y:.0f}" rx="{pr * 2.3:.0f}" ry="{pr * 0.55:.0f}" fill="none" stroke="{c3}" stroke-width="1.1" opacity="{0.35 * op:.2f}" transform="rotate(-18 {x:.0f} {y:.0f})"/>')
    out.append(f'<circle cx="{x:.0f}" cy="{y:.0f}" r="{pr:.1f}" fill="url(#ab-planet)" opacity="{(0.9 if ring else 0.85) * op:.2f}"/>')
    out.append(f'<circle cx="{x:.0f}" cy="{y:.0f}" r="{pr * halo:.0f}" fill="{c1 if ring else c2}" opacity="{(0.06 if ring else 0.07) * op:.2f}"/>')
    return out

def graph(x0, y0, gw, gh, n, seed, op):
    """Граф: узлы со свечением и рёбра к двум ближайшим соседям — агенты и связи между ними."""
    import random, math
    c1, c2, c3 = AB_C
    g = random.Random(seed)
    pts = [(x0 + g.uniform(0, gw), y0 + g.uniform(0, gh)) for _ in range(n)]
    out = []
    for i, (x, y) in enumerate(pts):
        for _, j in sorted(((math.hypot(x - a, y - b), j) for j, (a, b) in enumerate(pts) if j != i))[:2]:
            a, b = pts[j]
            out.append(f'<line x1="{x:.0f}" y1="{y:.0f}" x2="{a:.0f}" y2="{b:.0f}" stroke="{c3}" stroke-width="1.3" opacity="{0.42 * op:.2f}"/>')
    for k, (x, y) in enumerate(pts):
        rr = 5.5 if k % 3 == 0 else 3.8
        out.append(f'<circle cx="{x:.0f}" cy="{y:.0f}" r="{rr * 2.8:.1f}" fill="{c1}" opacity="{0.14 * op:.2f}"/><circle cx="{x:.0f}" cy="{y:.0f}" r="{rr}" fill="{c3 if k % 3 else c1}" opacity="{0.95 * op:.2f}"/>')
    return out

def shards_and_streaks(w, h, avoid, rnd, op, n_shards=7, n_streaks=4, streak_x=(0.05, 0.7), streak_avoid=True):
    """Осколки-шпили в духе Monolith и тонкие световые штрихи; осколки не заходят в avoid, штрихи — если streak_avoid."""
    u = min(w, h)
    out = []
    for _ in range(n_shards):
        for _try in range(20):
            x, y = rnd.uniform(w * 0.05, w * 0.95), rnd.uniform(h * 0.08, h * 0.92)
            if not avoid(x, y): break
        hgt = rnd.uniform(u * 0.05, u * 0.16); wid = hgt * rnd.uniform(0.16, 0.26); rot = rnd.uniform(-35, 35); o = rnd.uniform(0.10, 0.28)
        out.append(f'<polygon points="{x:.0f},{y - hgt / 2:.0f} {x - wid / 2:.0f},{y + hgt / 2:.0f} {x + wid / 2:.0f},{y + hgt / 2:.0f}" fill="url(#ab-shard)" opacity="{o * op:.2f}" transform="rotate({rot:.0f} {x:.0f} {y:.0f})"/>')
    for _ in range(n_streaks):
        for _try in range(20 if streak_avoid else 1):
            x, y = rnd.uniform(w * streak_x[0], w * streak_x[1]), rnd.uniform(h * 0.1, h * 0.9)
            if not streak_avoid or not avoid(x, y): break
        L = rnd.uniform(w * 0.10, w * 0.24) if w >= h else rnd.uniform(w * 0.18, w * 0.40); ang = -20 + rnd.uniform(-8, 8)
        out.append(f'<rect x="{x:.0f}" y="{y:.0f}" width="{L:.0f}" height="1.2" fill="url(#ab-streak)" opacity="{rnd.uniform(0.25, 0.5) * op:.2f}" transform="rotate({ang:.0f} {x:.0f} {y:.0f})"/>')
    return out

def abstractions(w, h, cx, cy, r, light=False, seed=11, portrait=False):
    """Космические абстракции верхнего слоя (размываются вместе со знаком): обрывки больших орбит, планеты, осколки, штрихи, два графа.
    portrait — раскладка для телефона: знак по центру сверху, графы под ним."""
    import random
    rnd = random.Random(seed)
    c1, c2, c3 = AB_C
    op = 0.55 if light else 1.0; u = min(w, h)
    parts = [ab_defs()]
    # обрывки больших орбит: две широкие дуги с тем же наклоном −10°
    ocx, ocy = (w * 0.5, h * 0.30) if portrait else (w * 0.62, h * 0.58)
    for rx, ry, sw, o in ([(w * 0.62, h * 0.11, 1.4, 0.16), (w * 0.80, h * 0.15, 1.0, 0.10)] if portrait else [(w * 0.48, h * 0.20, 1.4, 0.16), (w * 0.62, h * 0.27, 1.0, 0.10)]):
        parts.append(f'<ellipse cx="{ocx:.0f}" cy="{ocy:.0f}" rx="{rx:.0f}" ry="{ry:.0f}" fill="none" stroke="url(#ab-orbit)" stroke-width="{sw}" opacity="{o * op:.2f}" transform="rotate(-10 {ocx:.0f} {ocy:.0f})"/>')
    # планеты: одна с кольцом, одна маленькая
    (px, py), (qx, qy) = ((w * 0.14, h * 0.10), (w * 0.86, h * 0.62)) if portrait else ((w * 0.17, h * 0.24), (w * 0.84, h * 0.80))
    parts += planet(px, py, u * 0.022, op, ring=True) + planet(qx, qy, u * 0.011, op, halo=3)
    # осколки и штрихи — не на знаке
    parts += shards_and_streaks(w, h, lambda x, y: (x - cx) ** 2 + (y - cy) ** 2 <= (r * 1.6) ** 2, rnd, op, streak_avoid=False)
    # графы: два скопления
    g1, g2 = ((w * 0.08, h * 0.44, w * 0.40, h * 0.16), (w * 0.50, h * 0.72, w * 0.42, h * 0.16)) if portrait else ((w * 0.27, h * 0.12, w * 0.26, h * 0.26), (w * 0.54, h * 0.62, w * 0.22, h * 0.24))
    parts += graph(*g1, 8, seed + 1, op) + graph(*g2, 6, seed + 2, op)
    return "\n".join(parts)

def abstractions_lockup(w, h, box, light=False, seed=11, portrait=False):
    """Абстракции верхнего слоя вокруг лок-апа: планеты и графы по углам, осколки и штрихи вне надписи, обрывки орбит по краям."""
    import random
    rnd = random.Random(seed)
    c1, c2, c3 = AB_C
    op = 0.55 if light else 1.0; u = min(w, h)
    x0, y0, x1, y1 = box; mx, my = w * 0.05, h * 0.06
    avoid = lambda x, y: x0 - mx < x < x1 + mx and y0 - my < y < y1 + my
    parts = [ab_defs()]
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2 + h * 0.03
    for rx, ry, sw, o in ([(w * 0.80, h * 0.36, 1.4, 0.14), (w * 1.0, h * 0.44, 1.0, 0.09)] if portrait else [(w * 0.58, h * 0.36, 1.4, 0.14), (w * 0.72, h * 0.46, 1.0, 0.09)]):
        parts.append(f'<ellipse cx="{cx:.0f}" cy="{cy:.0f}" rx="{rx:.0f}" ry="{ry:.0f}" fill="none" stroke="url(#ab-orbit)" stroke-width="{sw}" opacity="{o * op:.2f}" transform="rotate(-10 {cx:.0f} {cy:.0f})"/>')
    if portrait: ps = [((w * 0.14, h * 0.28), True), ((w * 0.86, h * 0.95), False), ((w * 0.90, h * 0.50), False)]
    else: ps = [((w * 0.12, h * 0.20), True), ((w * 0.86, h * 0.82), False), ((w * 0.93, h * 0.30), False)]
    for k, ((px, py), ring) in enumerate(ps): parts += planet(px, py, u * (0.022 if ring else 0.011 if k == 1 else 0.007), op, ring=ring, halo=2.6 if ring else 3)
    parts += shards_and_streaks(w, h, avoid, rnd, op, n_shards=9, n_streaks=5, streak_x=(0.05, 0.9))
    g1, g2 = ((w * 0.06, h * 0.06, w * 0.40, h * 0.14), (w * 0.52, h * 0.28, w * 0.42, h * 0.14)) if portrait else ((w * 0.64, h * 0.06, w * 0.28, h * 0.22), (w * 0.06, h * 0.70, w * 0.26, h * 0.22))
    parts += graph(*g1, 8, seed + 1, op) + graph(*g2, 6, seed + 2, op)
    return "\n".join(parts)

def wallpaper_svg(w, h, light=False, quiet=False, layer="all", portrait=False):
    """Обои: градиент неба, две туманности, звёзды и орбиты; знак справа сверху ~34 % высоты (portrait — по центру сверху, 42 % ширины).
    quiet — водяной знак вместо цветного. layer: all — целиком; bg — только космос (под размытие Desktop); mark — только знак на прозрачном."""
    bg1, bg2 = ("#F4F7FF", "#E4E9FF") if light else ("#0B0D14", "#141a2e")
    if portrait: size = int(w * 0.42); x, y = int((w - size) / 2), int(h * 0.12)
    else: size = int(h * (0.30 if quiet else 0.34)); x, y = int(w * 0.66), int(h * 0.14)
    cx, cy = x + size / 2, y + size * 0.55
    if quiet:
        defs, body = mark_parts(mono=("#5B8CFF" if light else "#A2B4FF"), uid="wp"); op = 0.10 if light else 0.14
    else:
        defs, body = mark_parts(uid="wp"); op = 0.92
    glow = "rgba(122,92,255,0.14)" if light else "rgba(91,140,255,0.22)"
    glow2 = "rgba(91,140,255,0.10)" if light else "rgba(122,92,255,0.14)"
    sky = f"""<rect width="{w}" height="{h}" fill="url(#sky)"/>
<rect width="{w}" height="{h}" fill="url(#g1)"/>
<rect width="{w}" height="{h}" fill="url(#g2)"/>
{cosmos(w, h, cx, cy, size / 2, light)}""" if layer in ("all", "bg") else ""
    mark = (abstractions(w, h, cx, cy, size / 2, light, portrait=portrait) + f'<g transform="translate({x} {y}) scale({size / 256:.4f})" opacity="{op}">{body}</g>') if layer in ("all", "mark") else ""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{bg1}"/><stop offset="1" stop-color="{bg2}"/></linearGradient>
  <radialGradient id="g1" cx="{cx / w:.3f}" cy="{cy / h:.3f}" r="0.42"><stop offset="0" stop-color="{glow}"/><stop offset="1" stop-color="{bg1}" stop-opacity="0"/></radialGradient>
  <radialGradient id="g2" cx="0.18" cy="0.80" r="0.40"><stop offset="0" stop-color="{glow2}"/><stop offset="1" stop-color="{bg1}" stop-opacity="0"/></radialGradient>
  {defs}
</defs>
{sky}
{mark}
</svg>
"""

def wallpaper_lockup_svg(w, h, light=False, layer="all", portrait=False):
    """Обои с лок-апом по центру и космосом по краям: горизонтальный лок-ап 48 % ширины (portrait — вертикальный, 60 % ширины).
    layer: all — целиком; bg — космос (под размытие Desktop); top — лок-ап и абстракции на прозрачном (верхний слой)."""
    bg1, bg2 = ("#F4F7FF", "#E4E9FF") if light else ("#0B0D14", "#141a2e")
    ink, accent, muted = (P["night"], P["blue"], "#5B6488") if light else (P["paper"], P["lilac"], "#8E97B8")
    body, vw, vh = lockup_parts(vertical=portrait, ink=ink, accent=accent, muted=muted, uid="wl")
    s = w * (0.56 if portrait else 0.48) / vw
    bw, bh = vw * s, vh * s
    bx, by = (w - bw) / 2, h * (0.72 if portrait else 0.50) - bh / 2   # портрет: верх экрана занят иконками и виджетами, лок-ап — в нижней трети
    box = (bx, by, bx + bw, by + bh)
    cx, cy = w / 2, by + bh / 2
    glow = "rgba(122,92,255,0.14)" if light else "rgba(91,140,255,0.20)"
    glow2 = "rgba(91,140,255,0.10)" if light else "rgba(122,92,255,0.14)"
    sky = f"""<rect width="{w}" height="{h}" fill="url(#sky)"/>
<rect width="{w}" height="{h}" fill="url(#g1)"/>
<rect width="{w}" height="{h}" fill="url(#g2)"/>
{cosmos_lockup(w, h, box, light, portrait=portrait)}""" if layer in ("all", "bg") else ""
    top = (abstractions_lockup(w, h, box, light, portrait=portrait) + f'\n<g transform="translate({bx:.1f} {by:.1f}) scale({s:.4f})" opacity="0.96">{body}</g>') if layer in ("all", "top") else ""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{bg1}"/><stop offset="1" stop-color="{bg2}"/></linearGradient>
  <radialGradient id="g1" cx="{cx / w:.3f}" cy="{cy / h:.3f}" r="0.46"><stop offset="0" stop-color="{glow}"/><stop offset="1" stop-color="{bg1}" stop-opacity="0"/></radialGradient>
  <radialGradient id="g2" cx="{0.20 if portrait else 0.15}" cy="{0.15 if portrait else 0.85}" r="0.40"><stop offset="0" stop-color="{glow2}"/><stop offset="1" stop-color="{bg1}" stop-opacity="0"/></radialGradient>
</defs>
{font_style() if layer != "bg" else ""}
{sky}
{top}
</svg>
"""

ICON_PNGS = [  # (svg-файл, png-файл, ширина, высота)
    ("favicon-16.svg", "favicon-16.png", 16, 16), ("favicon-32.svg", "favicon-32.png", 32, 32), ("favicon-48.svg", "favicon-48.png", 48, 48),
    ("icon-bg.svg", "icon-180.png", 180, 180), ("icon-bg.svg", "icon-192.png", 192, 192), ("icon-bg.svg", "icon-512.png", 512, 512),
    ("icon-maskable.svg", "icon-512-maskable.png", 512, 512),
    ("wallpaper-1440x900.svg", "wallpaper-1440x900.png", 1440, 900), ("wallpaper-2560x1440.svg", "wallpaper-2560x1440.png", 2560, 1440),
    ("wallpaper-light-1440x900.svg", "wallpaper-light-1440x900.png", 1440, 900),
    ("wallpaper-quiet-1440x900.svg", "wallpaper-quiet-1440x900.png", 1440, 900), ("wallpaper-quiet-2560x1440.svg", "wallpaper-quiet-2560x1440.png", 2560, 1440),
    # слои для Desktop: космос (под размытие) и знак (резкий, прозрачный фон)
    ("wallpaper-bg-2560x1440.svg", "wallpaper-bg-2560x1440.png", 2560, 1440), ("wallpaper-mark-2560x1440.svg", "wallpaper-mark-2560x1440.png", 2560, 1440),
    ("wallpaper-light-bg-1440x900.svg", "wallpaper-light-bg-1440x900.png", 1440, 900),
    # портретные слои пресета Monolith (телефон): знак по центру сверху
    ("wallpaper-bg-1440x2560.svg", "wallpaper-bg-1440x2560.png", 1440, 2560), ("wallpaper-mark-1440x2560.svg", "wallpaper-mark-1440x2560.png", 1440, 2560),
    ("wallpaper-light-bg-1440x2560.svg", "wallpaper-light-bg-1440x2560.png", 1440, 2560),
    # лок-ап по центру: композиты для превью и слои для Desktop (альбом и портрет, тёмный и светлый)
    ("wallpaper-lockup-2560x1440.svg", "wallpaper-lockup-2560x1440.png", 2560, 1440), ("wallpaper-lockup-light-2560x1440.svg", "wallpaper-lockup-light-2560x1440.png", 2560, 1440),
    ("wallpaper-lockup-bg-2560x1440.svg", "wallpaper-lockup-bg-2560x1440.png", 2560, 1440), ("wallpaper-lockup-top-2560x1440.svg", "wallpaper-lockup-top-2560x1440.png", 2560, 1440),
    ("wallpaper-lockup-light-bg-2560x1440.svg", "wallpaper-lockup-light-bg-2560x1440.png", 2560, 1440), ("wallpaper-lockup-light-top-2560x1440.svg", "wallpaper-lockup-light-top-2560x1440.png", 2560, 1440),
    ("wallpaper-lockup-bg-1440x2560.svg", "wallpaper-lockup-bg-1440x2560.png", 1440, 2560), ("wallpaper-lockup-top-1440x2560.svg", "wallpaper-lockup-top-1440x2560.png", 1440, 2560),
    ("wallpaper-lockup-light-bg-1440x2560.svg", "wallpaper-lockup-light-bg-1440x2560.png", 1440, 2560), ("wallpaper-lockup-light-top-1440x2560.svg", "wallpaper-lockup-light-top-1440x2560.png", 1440, 2560),
]

def write_svgs(out: Path):
    files = {
        "mark.svg": mark_svg(),
        "mark-small.svg": mark_svg(simple=True, uid="s"),
        "mark-mono-white.svg": mark_svg(mono=P["paper"], uid="w"),
        "mark-mono-black.svg": mark_svg(mono=P["night"], uid="b"),
        "lockup-horizontal-dark.svg": lockup_svg(),
        "lockup-horizontal-light.svg": lockup_svg(ink=P["night"], accent=P["blue"], muted="#5B6488"),
        "lockup-vertical-dark.svg": lockup_svg(vertical=True),
        "lockup-vertical-light.svg": lockup_svg(vertical=True, ink=P["night"], accent=P["blue"], muted="#5B6488"),
        "wallpaper-1440x900.svg": wallpaper_svg(1440, 900),
        "wallpaper-2560x1440.svg": wallpaper_svg(2560, 1440),
        "wallpaper-quiet-1440x900.svg": wallpaper_svg(1440, 900, quiet=True),
        "wallpaper-quiet-2560x1440.svg": wallpaper_svg(2560, 1440, quiet=True),
        "wallpaper-light-1440x900.svg": wallpaper_svg(1440, 900, light=True),
        "wallpaper-bg-2560x1440.svg": wallpaper_svg(2560, 1440, layer="bg"),
        "wallpaper-mark-2560x1440.svg": wallpaper_svg(2560, 1440, layer="mark"),
        "wallpaper-light-bg-1440x900.svg": wallpaper_svg(1440, 900, light=True, layer="bg"),
        "wallpaper-bg-1440x2560.svg": wallpaper_svg(1440, 2560, layer="bg", portrait=True),
        "wallpaper-mark-1440x2560.svg": wallpaper_svg(1440, 2560, layer="mark", portrait=True),
        "wallpaper-light-bg-1440x2560.svg": wallpaper_svg(1440, 2560, light=True, layer="bg", portrait=True),
        "wallpaper-lockup-2560x1440.svg": wallpaper_lockup_svg(2560, 1440),
        "wallpaper-lockup-light-2560x1440.svg": wallpaper_lockup_svg(2560, 1440, light=True),
        "wallpaper-lockup-bg-2560x1440.svg": wallpaper_lockup_svg(2560, 1440, layer="bg"),
        "wallpaper-lockup-top-2560x1440.svg": wallpaper_lockup_svg(2560, 1440, layer="top"),
        "wallpaper-lockup-light-bg-2560x1440.svg": wallpaper_lockup_svg(2560, 1440, light=True, layer="bg"),
        "wallpaper-lockup-light-top-2560x1440.svg": wallpaper_lockup_svg(2560, 1440, light=True, layer="top"),
        "wallpaper-lockup-bg-1440x2560.svg": wallpaper_lockup_svg(1440, 2560, layer="bg", portrait=True),
        "wallpaper-lockup-top-1440x2560.svg": wallpaper_lockup_svg(1440, 2560, layer="top", portrait=True),
        "wallpaper-lockup-light-bg-1440x2560.svg": wallpaper_lockup_svg(1440, 2560, light=True, layer="bg", portrait=True),
        "wallpaper-lockup-light-top-1440x2560.svg": wallpaper_lockup_svg(1440, 2560, light=True, layer="top", portrait=True),
        # источники PNG-иконок: favicon — упрощённый знак до 18 px, плашка с закруглением, maskable с безопасной зоной 80 %
        "favicon-16.svg": mark_svg(simple=True, uid="f"), "favicon-32.svg": mark_svg(uid="f"), "favicon-48.svg": mark_svg(uid="f"),
        "icon-bg.svg": mark_svg(uid="i", bg=P["night"], pad=28, radius=64),
        "icon-maskable.svg": mark_svg(uid="k", bg=P["night"], pad=52, radius=0),
    }
    for name, svg in files.items(): (out / name).write_text(svg)
    import json
    (out / "render-manifest.json").write_text(json.dumps(ICON_PNGS, ensure_ascii=False, indent=1))
    print("svg:", len(files), "файлов + render-manifest.json")

def assemble(out: Path, install: bool):
    missing = [png for _, png, _, _ in ICON_PNGS if not (out / png).exists()]
    if missing: print("нет PNG (сначала assets/render.py):", ", ".join(missing)); sys.exit(1)
    # Pillow отбрасывает размеры больше базового кадра — базой идёт 48 px, меньшие кадры добавляются как есть
    ims = [Image.open(out / f"favicon-{s}.png").convert("RGBA") for s in (48, 32, 16)]
    ims[0].save(out / "favicon.ico", format="ICO", sizes=[(16, 16), (32, 32), (48, 48)], append_images=ims[1:])
    if install:
        pub = ROOT / "desktop" / "public"
        def put(src, dst):
            """Обои кладём как WebP q=90 (звёзды и градиенты в PNG весят 1–1.7 МБ, в WebP — 150–350 КБ), остальное — как есть."""
            if dst.endswith(".webp"):
                im = Image.open(out / src); im.save(pub / dst, format="WEBP", quality=90, method=6)
            else: (pub / dst).write_bytes((out / src).read_bytes())
        for src, dst in [("icon-192.png", "icon-192.png"), ("icon-512.png", "icon-512.png"), ("icon-512-maskable.png", "icon-512-maskable.png"), ("icon-180.png", "apple-touch-icon.png"), ("favicon.ico", "favicon.ico"), ("mark.svg", "favicon.svg"), ("wallpaper-1440x900.png", "wallpaper-monolith.webp"), ("wallpaper-light-1440x900.png", "wallpaper-monolith-light.webp"), ("wallpaper-quiet-1440x900.png", "wallpaper-monolith-quiet.webp"), ("wallpaper-bg-2560x1440.png", "wallpaper-monolith-bg.webp"), ("wallpaper-mark-2560x1440.png", "wallpaper-monolith-mark.webp"), ("wallpaper-light-bg-1440x900.png", "wallpaper-monolith-light-bg.webp"),
                         ("wallpaper-bg-1440x2560.png", "wallpaper-monolith-bg-portrait.webp"), ("wallpaper-mark-1440x2560.png", "wallpaper-monolith-mark-portrait.webp"), ("wallpaper-light-bg-1440x2560.png", "wallpaper-monolith-light-bg-portrait.webp"),
                         ("wallpaper-lockup-bg-2560x1440.png", "wallpaper-monolith-lockup-bg.webp"), ("wallpaper-lockup-top-2560x1440.png", "wallpaper-monolith-lockup.webp"),
                         ("wallpaper-lockup-light-bg-2560x1440.png", "wallpaper-monolith-lockup-light-bg.webp"), ("wallpaper-lockup-light-top-2560x1440.png", "wallpaper-monolith-lockup-light.webp"),
                         ("wallpaper-lockup-bg-1440x2560.png", "wallpaper-monolith-lockup-bg-portrait.webp"), ("wallpaper-lockup-top-1440x2560.png", "wallpaper-monolith-lockup-portrait.webp"),
                         ("wallpaper-lockup-light-bg-1440x2560.png", "wallpaper-monolith-lockup-light-bg-portrait.webp"), ("wallpaper-lockup-light-top-1440x2560.png", "wallpaper-monolith-lockup-light-portrait.webp")]:
            put(src, dst)
        write_mark_svelte(ROOT / "desktop" / "src" / "lib" / "Mark.svelte")
        patch_boot_svg(ROOT / "desktop" / "index.html")
        print("установлено в desktop/public, Mark.svelte и index.html обновлены")
    print("ok:", ", ".join(sorted(p.name for p in out.iterdir() if p.suffix in (".svg", ".png", ".ico"))))

def write_mark_svelte(path: Path):
    """Компонент знака для Desktop генерируется из той же геометрии, что и файлы бренда."""
    defs_c, body_c = mark_parts(uid="U")
    defs_s, body_s = mark_parts(simple=True, uid="U")
    defs_m, body_m = mark_parts(mono="MONO", uid="U")
    def sv(t): return t.replace("U-", "{uid}-").replace("#U-", "#{uid}-").replace("MONO", "{mono}")
    path.write_text(f"""<script lang="ts">
  // Знак Monolith — сгенерировано assets/build.py --install из той же геометрии, что assets/mark.svg. Не править руками.
  // simple — версия для 16–18 px; mono — одноцветный силуэт указанного цвета.
  let {{ size = 24, simple = false, mono = '' }}: {{ size?: number; simple?: boolean; mono?: string }} = $props();
  const uid = `mk${{Math.random().toString(36).slice(2, 8)}}`;
</script>

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width={{size}} height={{size}} role="img" aria-label="AgentVerse OS" style="display:block; flex: none;">
  {{#if mono}}
    <defs>{sv(defs_m)}</defs>
    {sv(body_m)}
  {{:else if simple}}
    <defs>{sv(defs_s)}</defs>
    {sv(body_s)}
  {{:else}}
    <defs>{sv(defs_c)}</defs>
    {sv(body_c)}
  {{/if}}
</svg>
""")

def patch_boot_svg(path: Path):
    """Экран загрузки в index.html: SVG между маркерами <!-- mark --> и <!-- /mark -->."""
    html = path.read_text()
    a, b = html.find("<!-- mark -->"), html.find("<!-- /mark -->")
    if a < 0 or b < 0: print("index.html: маркеры знака не найдены, пропускаю"); return
    defs, body = mark_parts(uid="b")
    svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="112" height="112" role="img" aria-label="AgentVerse OS"><defs>{defs}</defs>{body}</svg>'
    path.write_text(html[:a] + "<!-- mark -->" + svg + html[b:])

if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("--") else "svg"
    if mode == "svg": write_svgs(HERE)
    elif mode == "assemble": assemble(HERE, "--install" in sys.argv)
    else: print("режимы: svg | assemble [--install]"); sys.exit(2)

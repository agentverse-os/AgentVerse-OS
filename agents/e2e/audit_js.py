# Screen audit for the phone/matrix checks (phone_check.py, composer_matrix_check.py): overflow, going past the edge,
# clipped nowrap text, small targets, covered elements; SCROLLER_JS — the largest scrollable container.
AUDIT_JS = """
(() => {
  const vw = window.innerWidth, vh = window.innerHeight;
  const out = { sw: document.documentElement.scrollWidth, w: vw, h: vh, wide: [], clipped: [], tiny: [] };
  const vis = (el) => { const cs = getComputedStyle(el); return cs.display !== 'none' && cs.visibility !== 'hidden' && parseFloat(cs.opacity) > 0.05; };
  const clipAbove = (el) => { for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) { const cs = getComputedStyle(p); if (/(auto|scroll|hidden|clip)/.test(cs.overflowX)) { const r = p.getBoundingClientRect(); if (r.right <= vw + 1 && r.left >= -1) return true; } } return false; };
  const sel = (el) => { let s = el.tagName.toLowerCase(); if (el.id) s += '#' + el.id; const c = typeof el.className === 'string' ? el.className.trim() : ''; if (c) s += '.' + c.split(/\\s+/).slice(0, 3).join('.'); return s; };
  for (const el of document.body.querySelectorAll('*')) {
    if (el.tagName === 'SCRIPT' || el.tagName === 'STYLE') continue;
    if (!vis(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0 || r.bottom < 0 || r.top > vh || r.right <= 0 || r.left >= vw) continue;
    if ((r.right > vw + 1 || r.left < -1) && !clipAbove(el)) out.wide.push({ s: sel(el), l: Math.round(r.left), r: Math.round(r.right), t: Math.round(r.top) });
    const cs = getComputedStyle(el);
    if (cs.whiteSpace === 'nowrap' && cs.overflowX !== 'visible' && cs.textOverflow !== 'ellipsis' && el.scrollWidth > el.clientWidth + 2 && el.children.length === 0 && (el.textContent || '').trim()) out.clipped.push({ s: sel(el), text: (el.textContent || '').trim().slice(0, 40) });
    if ((el.tagName === 'BUTTON' || el.getAttribute('role') === 'button') && (r.height < 26 || r.width < 26)) out.tiny.push({ s: sel(el), w: Math.round(r.width), h: Math.round(r.height) });
  }
  // касание: центр каждой видимой кнопки/переключателя/поля должен попадать в него же (не перекрыт другим элементом);
  // при открытом меню, листе, палитре или затемнении проверяются только элементы внутри них
  out.covered = [];
  const overlay = [...document.querySelectorAll('.menu, .sheet, .palette, .scrim, .popup, .modes, .effpop')].find(o => { const cs = getComputedStyle(o); return cs.display !== 'none' && cs.visibility !== 'hidden' && o.getBoundingClientRect().width > 0; });
  // точка внутри всех обрезающих предков (прокрутка под шапку/полосу вкладок и прокручиваемые меню — не «перекрытие»)
  const unclipped = (el, x, y) => { for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) { const cs = getComputedStyle(p); if (/(auto|scroll|hidden|clip)/.test(cs.overflowY) || /(auto|scroll|hidden|clip)/.test(cs.overflowX)) { const q = p.getBoundingClientRect(); if (x <= q.left + 0.5 || x >= q.right - 0.5 || y <= q.top + 0.5 || y >= q.bottom - 0.5) return false; } } return true; };
  for (const el of document.querySelectorAll('button, [role=button], input, select, textarea, a[href], .switch')) {
    if (!vis(el) || el.disabled) continue;
    if (overlay && !overlay.contains(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2 || r.bottom <= 0 || r.top >= vh || r.right <= 0 || r.left >= vw) continue;
    const cx = Math.min(vw - 1, Math.max(0, r.left + r.width / 2)), cy = Math.min(vh - 1, Math.max(0, r.top + r.height / 2));
    if (!unclipped(el, cx, cy)) continue;
    const hit = document.elementFromPoint(cx, cy);
    if (!hit) continue;
    const okHit = el.contains(hit) || hit.contains(el) || (el.tagName === 'INPUT' && hit.closest('label') && hit.closest('label').contains(el));
    if (!okHit) out.covered.push({ s: sel(el), by: sel(hit), x: Math.round(cx), y: Math.round(cy) });
  }
  for (const k of ['wide', 'clipped', 'tiny', 'covered']) out[k] = out[k].slice(0, 10);
  return out;
})()
"""
SCROLLER_JS = """
(() => { document.querySelectorAll('[data-pw]').forEach(e => delete e.dataset.pw);
  const els = [...document.querySelectorAll('main.main *, .sheet *, .palette *')].filter(e => { const cs = getComputedStyle(e); return /(auto|scroll)/.test(cs.overflowY) && e.scrollHeight > e.clientHeight + 24 && e.clientHeight > 160; });
  els.sort((a, b) => b.clientHeight * b.clientWidth - a.clientHeight * a.clientWidth); const e = els[0]; if (!e) return null; e.dataset.pw = 'scroller'; return { sh: e.scrollHeight, ch: e.clientHeight, top: e.scrollTop }; })()
"""

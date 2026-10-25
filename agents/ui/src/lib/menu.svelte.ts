// Popup menu: one per app, opens at an anchor, closes on an outside click, Esc or picking an item. `header` — a group
// heading, `sub` — a subheading inside a group (the provider in the model menu); `search` — a search field at the top: items
// are filtered by label and hint, headings stay if there are matches under them.
export type MenuItem = { label?: string; icon?: string; on?: () => void; checked?: boolean; danger?: boolean; header?: boolean; sub?: boolean; sep?: boolean; hint?: string; avatar?: string; disabled?: boolean };
export type MenuOpts = { search?: boolean; placeholder?: string };
class Menu {
  items = $state<MenuItem[]>([]);
  x = $state(0); y = $state(0);
  open = $state(false);
  search = $state(false);
  placeholder = $state('');
  query = $state('');
  label = $state('');
  anchor: HTMLElement | null = null;
  private showSeq = 0;
  show(anchor: HTMLElement, items: MenuItem[], opts: MenuOpts = {}) {
    const seq = ++this.showSeq;
    const r = anchor.getBoundingClientRect();
    this.anchor = anchor;
    this.label = anchor.getAttribute('aria-label') || anchor.getAttribute('title') || anchor.textContent?.trim() || items.find(it => it.header)?.label || '';
    this.items = items; this.x = r.left; this.y = r.bottom + 6; this.open = true;
    this.search = !!opts.search; this.placeholder = opts.placeholder ?? ''; this.query = '';
    // after rendering — adjust the position so the menu does not go off
    // screen
    requestAnimationFrame(() => {
      const el = document.querySelector('.menu') as HTMLElement | null; if (!this.open || seq !== this.showSeq || !el) return;
      const w = el.offsetWidth, h = el.offsetHeight;
      this.x = Math.max(8, Math.min(r.left, innerWidth - w - 8));
      this.y = r.bottom + 6 + h > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6;
      const first = el.querySelector<HTMLElement>(this.search ? 'input.ms' : '.mi:not(:disabled)');
      (first ?? el).focus({ preventScroll: true });
    });
  }
  close(restoreFocus = false) {
    this.open = false; ++this.showSeq;
    if (restoreFocus && this.anchor?.isConnected) this.anchor.focus({ preventScroll: true });
  }
  /** Items with the search applied: a match by label or hint; headings and subheadings — only above matches. */
  get visible(): MenuItem[] {
    // the query words are searched separately, "-" and "_" equal a dot: "opus 4.6" finds both "claude-opus-4-6" and "Claude
    // 4.6 Opus"
    const norm = (x: string) => x.toLowerCase().replace(/[-_]/g, '.');
    const words = norm(this.query.trim()).split(/\s+/).filter(Boolean);
    if (!this.search || !words.length) return this.items;
    const out: MenuItem[] = []; let head: MenuItem | null = null; let sub: MenuItem | null = null;
    for (const it of this.items) {
      if (it.header) { head = it; sub = null; continue; }
      if (it.sub) { sub = it; continue; }
      if (it.sep) continue;
      const hay = norm(`${it.label ?? ''} ${it.hint ?? ''}`);
      if (!words.every(w => hay.includes(w))) continue;
      if (head) { out.push(head); head = null; }
      if (sub) { out.push(sub); sub = null; }
      out.push(it);
    }
    return out;
  }
}
export const menu = new Menu();

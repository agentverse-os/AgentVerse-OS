// Всплывающее меню: одно на приложение, открывается у якоря, закрывается кликом вне, Esc или выбором пункта.
// `header` — заголовок группы, `sub` — подзаголовок внутри группы (провайдер в меню моделей); `search` — поле поиска сверху:
// пункты фильтруются по подписи и подсказке, заголовки остаются, если под ними есть совпадения.
export type MenuItem = { label?: string; icon?: string; on?: () => void; checked?: boolean; danger?: boolean; header?: boolean; sub?: boolean; sep?: boolean; hint?: string; avatar?: string; disabled?: boolean };
export type MenuOpts = { search?: boolean; placeholder?: string };
class Menu {
  items = $state<MenuItem[]>([]);
  x = $state(0); y = $state(0);
  open = $state(false);
  search = $state(false);
  placeholder = $state('');
  query = $state('');
  show(anchor: HTMLElement, items: MenuItem[], opts: MenuOpts = {}) {
    const r = anchor.getBoundingClientRect();
    this.items = items; this.x = r.left; this.y = r.bottom + 6; this.open = true;
    this.search = !!opts.search; this.placeholder = opts.placeholder ?? ''; this.query = '';
    // после отрисовки — поправить положение, чтобы меню не вылезло за экран
    requestAnimationFrame(() => {
      const el = document.querySelector('.menu') as HTMLElement | null; if (!el) return;
      const w = el.offsetWidth, h = el.offsetHeight;
      this.x = Math.max(8, Math.min(r.left, innerWidth - w - 8));
      this.y = r.bottom + 6 + h > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6;
      if (this.search) (el.querySelector('input.ms') as HTMLInputElement | null)?.focus();
    });
  }
  close() { this.open = false; }
  /** Пункты с учётом поиска: совпадение по подписи или подсказке; заголовки и подзаголовки — только над совпадениями. */
  get visible(): MenuItem[] {
    // слова запроса ищутся по отдельности, «-» и «_» равны точке: «opus 4.6» находит и «claude-opus-4-6», и «Claude 4.6 Opus»
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

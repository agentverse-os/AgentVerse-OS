// Toasts: short action confirmations in the bottom right
// corner.
export type Toast = { id: number; text: string; kind: 'ok' | 'bad' | 'info' };
class Toasts {
  list = $state<Toast[]>([]);
  private n = 0;
  push(text: string, kind: Toast['kind'] = 'ok') {
    const id = ++this.n;
    this.list.push({ id, text, kind });
    setTimeout(() => { this.list = this.list.filter(t => t.id !== id); }, 3400);
  }
}
export const toasts = new Toasts();
export const toast = (text: string, kind: Toast['kind'] = 'ok') => toasts.push(text, kind);

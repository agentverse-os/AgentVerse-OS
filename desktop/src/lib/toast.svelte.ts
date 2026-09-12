// Уведомления Desktop: ход долгих операций (установка, удаление, связи), результат с действием, ошибки.
import { t } from './i18n.svelte';
export type ToastKind = 'progress' | 'ok' | 'error' | 'info';
export interface Toast { id: number; kind: ToastKind; title: string; text?: string; action?: { label: string; run: () => void }; until?: number }
let seq = 0;
class ToastStore {
  list = $state<Toast[]>([]);
  push(t: Omit<Toast, 'id'>): number {
    const id = ++seq;
    const ttl = t.kind === 'progress' ? 0 : t.kind === 'error' ? 15000 : 7000;
    this.list = [...this.list, { ...t, id, until: ttl ? Date.now() + ttl : undefined }];
    if (ttl) setTimeout(() => this.dismiss(id), ttl);
    return id;
  }
  update(id: number, patch: Partial<Omit<Toast, 'id'>>) {
    const ttl = patch.kind === 'ok' || patch.kind === 'info' ? 7000 : patch.kind === 'error' ? 15000 : 0;
    this.list = this.list.map(t => t.id === id ? { ...t, ...patch, until: ttl ? Date.now() + ttl : t.until } : t);
    if (ttl) setTimeout(() => this.dismiss(id), ttl);
  }
  dismiss(id: number) { this.list = this.list.filter(t => t.id !== id); }
  /** Обёртка долгой операции: прогресс с этапами → готово/ошибка. */
  async track<T>(title: string, f: (progress: (stage: string) => void) => Promise<T>, done?: (v: T) => { text?: string; action?: Toast['action'] } | void): Promise<T> {
    const id = this.push({ kind: 'progress', title, text: t('common.starting') });
    try {
      const v = await f(stage => this.update(id, { text: stage }));
      const d = done?.(v) ?? undefined;
      this.update(id, { kind: 'ok', text: d?.text ?? t('common.ready'), action: d?.action });
      return v;
    } catch (e) {
      this.update(id, { kind: 'error', text: e instanceof Error ? e.message : String(e) });
      throw e;
    }
  }
}
export const toast = new ToastStore();

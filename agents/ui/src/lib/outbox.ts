// Outbox without a network (docs/agents-ux.md §9.9): messages sent while the connection was lost wait in the IndexedDB
// `agents-outbox` and go out in order when the network returns — from the app (the online event / a live socket) or from the
// service worker (Background Sync where available). The same database and the same record shape as in public/sw-extra.js.
import { api, OfflineError } from './api';

export type OutboxItem = { id: string; project: string; session: string; content: unknown[]; text: string; at: number; error?: string; sending?: number };

const DB = 'agents-outbox'; const STORE = 'items';
function open(): Promise<IDBDatabase> {
  return new Promise((res, rej) => {
    if (typeof indexedDB === 'undefined') { rej(new Error('IndexedDB недоступна')); return; }
    const rq = indexedDB.open(DB, 1);
    rq.onupgradeneeded = () => { const db = rq.result; if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE, { keyPath: 'id' }); };
    rq.onsuccess = () => res(rq.result); rq.onerror = () => rej(rq.error);
  });
}
function tx<T>(mode: IDBTransactionMode, run: (st: IDBObjectStore) => IDBRequest<T> | void): Promise<T> {
  return open().then(db => new Promise<T>((res, rej) => {
    const t = db.transaction(STORE, mode); const st = t.objectStore(STORE); const rq = run(st); let out: T;
    if (rq) rq.onsuccess = () => { out = rq.result; };
    t.oncomplete = () => { db.close(); res(out!); }; t.onerror = () => { db.close(); rej(t.error); };
  }));
}
export const listOutbox = () => tx<OutboxItem[]>('readonly', st => st.getAll() as IDBRequest<OutboxItem[]>).then(l => (l ?? []).sort((a, b) => a.at - b.at)).catch(() => [] as OutboxItem[]);
export const putOutbox = (item: OutboxItem) => tx<void>('readwrite', st => { st.put(item); }).catch(() => {});
export const delOutbox = (id: string) => tx<void>('readwrite', st => { st.delete(id); }).catch(() => {});
/** Claim a record in one transaction (the worker and the app must not send it twice); false — already taken by someone or failed */
function claimOutbox(id: string): Promise<boolean> {
  return open().then(db => new Promise<boolean>((res, rej) => {
    const t = db.transaction(STORE, 'readwrite'); const st = t.objectStore(STORE); const rq = st.get(id) as IDBRequest<OutboxItem | undefined>; let ok = false;
    rq.onsuccess = () => { const it = rq.result; if (!it || it.error || (it.sending && Date.now() - it.sending < 60000)) return; ok = true; st.put({ ...it, sending: Date.now() }); };
    t.oncomplete = () => { db.close(); res(ok); }; t.onerror = () => { db.close(); rej(t.error); };
  })).catch(() => false);
}
/** Outbox version for subscribers (Chat re-reads its list) */
export const outboxVersion = { n: 0, bump() { this.n++; window.dispatchEvent(new CustomEvent('agents:outbox')); } };

/** Put a message into the outbox and ask for background sync (Chromium); the online event is enough for the others */
export async function enqueueOutbox(project: string, session: string, content: unknown[], text: string): Promise<OutboxItem> {
  const item: OutboxItem = { id: `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`, project, session, content, text, at: Date.now() };
  await putOutbox(item);
  try { const reg = await navigator.serviceWorker?.ready; await (reg as unknown as { sync?: { register: (tag: string) => Promise<void> } }).sync?.register('agents-outbox'); } catch {}
  return item;
}

let flushing = false;
/** Send the queue from the app: in order, bringing up a finished session; 4xx — the record stays with an error */
export async function flushOutbox(onSent?: (item: OutboxItem) => void): Promise<number> {
  if (flushing) return 0; flushing = true; let sent = 0;
  try {
    for (const it of await listOutbox()) {
      if (it.error) continue;
      if (!(await claimOutbox(it.id))) continue;
      try {
        const info = await api.sessionGet(it.project, it.session).then(r => r.info).catch(() => null);
        if (!info) break;   // the core is unavailable — keep waiting
        if (info.alive === false || info.status === 'exited' || info.status === 'error') { await api.sessionCreate(it.project, { resume: it.session }); await new Promise(r => setTimeout(r, 1500)); }
        await api.sessionPrompt(it.project, it.session, it.content);
        await delOutbox(it.id); sent++; onSent?.(it);
      } catch (e) {
        if (e instanceof OfflineError) { await putOutbox({ ...it, sending: 0 }); break; }   // no network — retry later
        await putOutbox({ ...it, sending: 0, error: e instanceof Error ? e.message : String(e) }); continue;   // the server refused — stays with the error text
      }
    }
  } finally { flushing = false; outboxVersion.bump(); }
  return sent;
}

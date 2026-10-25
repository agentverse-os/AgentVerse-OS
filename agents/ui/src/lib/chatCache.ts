// Conversation feed cache on the device (docs/agents-ux.md §4.3.16): IndexedDB "agents-chat", one record per session —
// the last EVENTS events in lightweight form, the first and last numbers, the journal import_v and the schema version.
// The truth is the journal on the server: on open the feed is drawn from the cache, and the socket with
// `since=last_seq` sends the tail; an import_v mismatch or a last_seq smaller than the cache — the record is dropped.
// Writes are delayed, not per event; a ceiling on the number of conversations (LRU).
import type { SessionEvent } from './api';

export const SCHEMA = 1;
/** Cache window and page: 300 events ≈ 8–9 turns; no more than ~1 MB per session, no more than 40
 *  conversations */
export const PAGE = 300;
export const MAX_BYTES = 1_000_000;
export const MAX_SESSIONS = 40;
const DB = 'agents-chat'; const STORE = 'sessions';

export interface CachedChat { key: string; project: string; id: string; schema: number; import_v: number | null; first_seq: number; last_seq: number; events: SessionEvent[]; saved_at: number }

let dbp: Promise<IDBDatabase | null> | null = null;
function open(): Promise<IDBDatabase | null> {
  if (dbp) return dbp;
  dbp = new Promise((resolve) => {
    try {
      if (typeof indexedDB === 'undefined') { resolve(null); return; }
      const req = indexedDB.open(DB, 1);
      req.onupgradeneeded = () => { const db = req.result; if (!db.objectStoreNames.contains(STORE)) { const st = db.createObjectStore(STORE, { keyPath: 'key' }); st.createIndex('saved_at', 'saved_at'); } };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch { resolve(null); }
  });
  return dbp;
}
function tx<T>(mode: IDBTransactionMode, run: (st: IDBObjectStore) => IDBRequest<T> | undefined): Promise<T | undefined> {
  return open().then((db) => new Promise<T | undefined>((resolve) => {
    if (!db) { resolve(undefined); return; }
    try {
      const t = db.transaction(STORE, mode); const st = t.objectStore(STORE); const req = run(st);
      if (!req) { t.oncomplete = () => resolve(undefined); t.onerror = () => resolve(undefined); return; }
      req.onsuccess = () => resolve(req.result as T); req.onerror = () => resolve(undefined);
    } catch (e) { console.warn('chat cache:', e); resolve(undefined); }
  }));
}

export const cacheKey = (project: string, id: string) => `${project}/${id}`;

export async function getChat(project: string, id: string): Promise<CachedChat | null> {
  const r = await tx<CachedChat>('readonly', (st) => st.get(cacheKey(project, id)));
  if (!r || r.schema !== SCHEMA || !Array.isArray(r.events) || !r.events.length) return null;
  return r;
}

/** Event tail within the window and the weight ceiling: no more than PAGE and ~MAX_BYTES (estimated by JSON), but
 *  at least 40 events */
export function tail(events: SessionEvent[]): SessionEvent[] {
  let out = events.length > PAGE ? events.slice(events.length - PAGE) : events.slice();
  let bytes = 0; const sizes = out.map((e) => { const n = JSON.stringify(e).length; bytes += n; return n; });
  let from = 0;
  while (bytes > MAX_BYTES && out.length - from > 40) { bytes -= sizes[from]; from++; }
  if (from) out = out.slice(from);
  return out;
}

export async function putChat(project: string, id: string, importV: number | null, events: SessionEvent[]): Promise<void> {
  if (!events.length) return;
  const ev = tail(events);
  const rec: CachedChat = { key: cacheKey(project, id), project, id, schema: SCHEMA, import_v: importV ?? null, first_seq: ev[0].seq, last_seq: ev[ev.length - 1].seq, events: ev, saved_at: Date.now() };
  await tx('readwrite', (st) => st.put(rec));
  void evict();
}

export async function delChat(project: string, id: string): Promise<void> { await tx('readwrite', (st) => st.delete(cacheKey(project, id))); }

/** Conversations not opened for long beyond MAX_SESSIONS — out */
async function evict(): Promise<void> {
  const db = await open(); if (!db) return;
  try {
    const t = db.transaction(STORE, 'readwrite'); const idx = t.objectStore(STORE).index('saved_at');
    const countReq = idx.count();
    countReq.onsuccess = () => {
      let extra = countReq.result - MAX_SESSIONS; if (extra <= 0) return;
      const cur = idx.openCursor();
      cur.onsuccess = () => { const c = cur.result; if (!c || extra <= 0) return; c.delete(); extra--; c.continue(); };
    };
  } catch {}
}

export async function clearChatCache(): Promise<void> { await tx('readwrite', (st) => st.clear()); }

/** How many conversations are in the cache (for
 *  "About" and e2e) */
export async function cacheCount(): Promise<number> { return (await tx<number>('readonly', (st) => st.count())) ?? 0; }

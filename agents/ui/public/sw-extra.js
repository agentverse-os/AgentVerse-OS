/* Addition to the generated "Agents" service worker (docs/agents-ux.md §9.9): push notifications with actions,
   the icon badge, notification clicks and background sending of the outbox. Loaded from sw.js via importScripts. */
/* global self, clients */

const API = '/agents/api';
const ICON = '/agents/icon-192.png';

async function windowsOf() {
  try { return await clients.matchAll({ type: 'window', includeUncontrolled: true }); } catch { return []; }
}
function tell(list, msg) { for (const w of list) { try { w.postMessage(msg); } catch {} } }
async function setBadge(n) {
  try { if (!('setAppBadge' in navigator)) return; if (n > 0) await navigator.setAppBadge(n); else await navigator.clearAppBadge(); } catch {}
}

async function notify(title, opts) {
  try { await self.registration.showNotification(title, opts); return true; } catch (e) { return false; }   // no permission — the windows still get to know
}

/* Push payload → always to the windows, a notification — only if no window is visible (or force, e.g. a test) */
async function handlePush(d) {
  const wins = await windowsOf();
  if (typeof d.badge === 'number') await setBadge(d.badge);
  const visible = wins.some((w) => w.visibilityState === 'visible');
  tell(wins, { kind: 'push', data: d });
  if (visible && !d.force) return { shown: false, reason: 'visible' };
  const actions = Array.isArray(d.actions) ? d.actions.slice(0, 2) : [];
  const shown = await notify(d.title || 'AgentVerse Agents', {
    body: d.body || '', icon: ICON, badge: ICON, tag: d.tag || undefined, renotify: !!d.renotify && !!d.tag, data: d, actions,
    requireInteraction: d.kind === 'permission', silent: false,
  });
  return { shown };
}

self.addEventListener('push', (event) => {
  let d = {};
  try { d = event.data ? event.data.json() : {}; } catch { d = { title: 'AgentVerse Agents', body: event.data ? event.data.text() : '' }; }
  event.waitUntil(handlePush(d));
});

/* Notification action: "Allow/Deny" — an answer to the core from the worker, a tap on the body — focus or a new window on the conversation.
   Extracted into a function: the same code is run by the {kind:'notification-action'} message (checks without a real click). */
async function onNotificationAction(action, d) {
    if ((action === 'allow' || action === 'deny') && d.project && d.id && d.request_id) {
      try {
        const r = await fetch(`${API}/core/projects/${encodeURIComponent(d.project)}/sessions/${encodeURIComponent(d.id)}/permission`, {
          method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ request_id: d.request_id, behavior: action }),
        });
        if (!r.ok) await notify(action === 'allow' ? 'Не удалось разрешить' : 'Не удалось отклонить', { body: `ответ ядра ${r.status} — откройте беседу`, icon: ICON, tag: `perm-fail-${d.id}`, data: { url: d.url } });
        tell(await windowsOf(), { kind: 'permission-answered', data: d, behavior: action, ok: r.ok, status: r.status });
      } catch (e) {
        await notify('Нет связи с сервером', { body: 'решение не отправлено — откройте беседу', icon: ICON, tag: `perm-fail-${d.id}`, data: { url: d.url } });
        tell(await windowsOf(), { kind: 'permission-answered', data: d, behavior: action, ok: false, error: String(e) });
      }
      return;
    }
    const url = d.url || '/agents/';
    const wins = await windowsOf();
    const mine = wins.find((w) => { try { return new URL(w.url).pathname.startsWith('/agents'); } catch { return false; } });
    if (mine) {
      try { await mine.focus(); } catch {}
      tell([mine], { kind: 'open', data: d, url });
    } else {
      try { await clients.openWindow(url); } catch {}
    }
}

self.addEventListener('notificationclick', (event) => {
  const d = (event.notification && event.notification.data) || {};
  event.notification.close();
  event.waitUntil(onNotificationAction(event.action, d));
});

self.addEventListener('notificationclose', () => {});

/* The push service reissued the subscription (§9.10): subscribe anew with the server key, tell it the new endpoint, remove the old one —
   otherwise the server gets a 410, removes the device, and the phone quietly stops receiving notifications */
function b64ToBytes(b64) { const pad = '='.repeat((4 - (b64.length % 4)) % 4); const raw = atob((b64 + pad).replace(/-/g, '+').replace(/_/g, '/')); return Uint8Array.from(raw, (c) => c.charCodeAt(0)); }
self.addEventListener('pushsubscriptionchange', (event) => {
  event.waitUntil((async () => {
    try {
      const info = await (await fetch(`${API}/push`)).json();
      const sub = await self.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64ToBytes(info.public_key) });
      const j = sub.toJSON();
      await fetch(`${API}/push/subscribe`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ endpoint: sub.endpoint, keys: j.keys, ua: 'переподписка' }) });
      const old = event.oldSubscription && event.oldSubscription.endpoint;
      if (old && old !== sub.endpoint) await fetch(`${API}/push/subscribe`, { method: 'DELETE', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ endpoint: old }) });
    } catch (e) {}
  })());
});

/* ---- outbox without a network: IndexedDB agents-outbox, store items {id, project, session, content, text, at, error} ---- */
function outboxDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open('agents-outbox', 1);
    req.onupgradeneeded = () => { const db = req.result; if (!db.objectStoreNames.contains('items')) db.createObjectStore('items', { keyPath: 'id' }); };
    req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error);
  });
}
function allItems(db) {
  return new Promise((resolve, reject) => { const tx = db.transaction('items', 'readonly'); const rq = tx.objectStore('items').getAll(); rq.onsuccess = () => resolve(rq.result || []); rq.onerror = () => reject(rq.error); });
}
function putItem(db, item) { return new Promise((resolve, reject) => { const tx = db.transaction('items', 'readwrite'); tx.objectStore('items').put(item); tx.oncomplete = () => resolve(); tx.onerror = () => reject(tx.error); }); }
function delItem(db, id) { return new Promise((resolve, reject) => { const tx = db.transaction('items', 'readwrite'); tx.objectStore('items').delete(id); tx.oncomplete = () => resolve(); tx.onerror = () => reject(tx.error); }); }
/* Claim a record before sending in one transaction: the worker and the app itself may send at the same time, there must be no double send */
function claimItem(db, id) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction('items', 'readwrite'); const st = tx.objectStore('items'); const rq = st.get(id); let ok = false;
    rq.onsuccess = () => { const it = rq.result; if (!it || it.error || (it.sending && Date.now() - it.sending < 60000)) return; ok = true; st.put({ ...it, sending: Date.now() }); };
    tx.oncomplete = () => resolve(ok); tx.onerror = () => reject(tx.error);
  });
}

/* Send the queue in order: success — the record goes; 4xx — stays with the error text (there is a network, but the message was not accepted);
   a network error — stop, retry next time */
async function flushOutbox() {
  let db; try { db = await outboxDb(); } catch { return; }
  const items = (await allItems(db)).sort((a, b) => (a.at || 0) - (b.at || 0));
  let sent = 0;
  for (const it of items) {
    if (it.error) continue;
    if (!(await claimItem(db, it.id))) continue;
    let r;
    try {
      r = await fetch(`${API}/core/projects/${encodeURIComponent(it.project)}/sessions/${encodeURIComponent(it.session)}/prompt`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ content: it.content }),
      });
    } catch { await putItem(db, { ...it, sending: 0 }); break; }
    if (r.ok) { await delItem(db, it.id); sent++; continue; }
    if (r.status >= 400 && r.status < 500) { let msg = `HTTP ${r.status}`; try { msg = (await r.json()).error || msg; } catch {} await putItem(db, { ...it, sending: 0, error: msg }); continue; }
    await putItem(db, { ...it, sending: 0 }); break;
  }
  tell(await windowsOf(), { kind: 'outbox', sent });
}

self.addEventListener('sync', (event) => { if (event.tag === 'agents-outbox') event.waitUntil(flushOutbox()); });
self.addEventListener('message', (event) => {
  const m = event.data || {};
  if (m.kind === 'flush-outbox') event.waitUntil(flushOutbox());
  else if (m.kind === 'notification-action') event.waitUntil(onNotificationAction(m.action, m.data || {}));
  else if (m.kind === 'simulate-push') event.waitUntil(handlePush(m.data || {}).then((r) => tell([event.source].filter(Boolean), { kind: 'push-handled', ...r })));   // checks: the same path as a real push
});

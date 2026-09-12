// Экран загрузки (#boot в index.html): стоит до первого ответа ядра; если ядро не отвечает — показывает причину и продолжает пытаться.
let done = false;
export function bootMessage(text: string, error = false) {
  const el = document.getElementById('boot-msg');
  if (el) { el.textContent = text; el.classList.toggle('err', error); }
}
export function bootDone() {
  if (done) return;
  done = true;
  const b = document.getElementById('boot');
  if (!b) return;
  b.classList.add('out');
  setTimeout(() => b.remove(), 650);
}
export function bootActive() { return !done; }

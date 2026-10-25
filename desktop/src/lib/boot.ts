// Loading screen (#boot in index.html): stays until the core's first answer; if the core does not answer — shows the reason and keeps
// trying.
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

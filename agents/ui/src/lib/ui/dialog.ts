/** Modal keyboard navigation and focus return, shared by the palette and mobile sheet. */
const dialogs = new Map<HTMLElement, HTMLElement | null>();
let initialInert = false;
let lastClosedOpener: HTMLElement | null = null;
export function dialog(node: HTMLElement, close: () => void) {
  let opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  for (const [existing, parentOpener] of dialogs) if (opener && existing.contains(opener)) { opener = parentOpener; break; }
  if (lastClosedOpener && (!opener?.isConnected || opener === document.body || opener.closest('[aria-modal="true"]'))) opener = lastClosedOpener;
  const background = document.querySelector<HTMLElement>('.app');
  if (!dialogs.size) initialInert = background?.inert ?? false;
  dialogs.set(node, opener);
  if (background) background.inert = true;
  const targets = () => Array.from(node.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]')).filter(el => el.getClientRects().length > 0);
  queueMicrotask(() => { if (node.isConnected) (targets()[0] ?? node).focus(); });
  const key = (event: KeyboardEvent) => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); }
    if (event.key !== 'Tab') return;
    const list = targets(), first = list[0], last = list.at(-1);
    if (!first) { event.preventDefault(); node.focus(); return; }
    if (event.shiftKey && (document.activeElement === first || !node.contains(document.activeElement))) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && (document.activeElement === last || !node.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
  };
  node.addEventListener('keydown', key);
  return { destroy() {
    node.removeEventListener('keydown', key);
    dialogs.delete(node);
    lastClosedOpener = opener;
    if (background) background.inert = dialogs.size ? true : initialInert;
    // Opening another dialog in the same update takes precedence over returning focus.
    queueMicrotask(() => {
      if (!dialogs.size && opener?.isConnected) opener.focus();
      if (lastClosedOpener === opener) lastClosedOpener = null;
    });
  } };
}

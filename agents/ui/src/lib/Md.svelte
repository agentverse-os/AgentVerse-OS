<script module lang="ts">
  // Markdown of agent answers and command output: marked (GFM — tables, code, lists) + DOMPurify; links open in a new tab.
  // Slice 33 (docs/agents-ux.md §4.3.9): TeX formulas (KaTeX) and mermaid diagrams — parsed at the marked token stage,
  // rendered after insertion into the DOM; the libraries load lazily as separate chunks on the first formula/diagram. The "⧉"
  // button on code blocks.
  import { marked, type Tokens } from 'marked';
  import DOMPurify from 'dompurify';
  import { tick } from 'svelte';
  import { prefs } from './prefs.svelte';
  import { t } from './i18n';
  import { toast } from './toast.svelte';

  const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  // formulas: $$…$$ and \[…\] are blocks; $…$ and \(…\) are inline. Inline dollars are accepted only without spaces at the
  // boundaries and without a digit right after the closing $ ("$5 and $7" is not a formula).
  const mathBlock = {
    name: 'mathBlock', level: 'block' as const,
    start(src: string) { const m = /(^|\n)[ \t]*(\$\$|\\\[)/.exec(src); return m ? m.index + m[1].length : undefined; },
    tokenizer(src: string) {
      const m = /^[ \t]*\$\$[ \t]*\n?([\s\S]+?)\n?[ \t]*\$\$[ \t]*(?:\n+|$)/.exec(src) || /^[ \t]*\\\[[ \t]*\n?([\s\S]+?)\n?[ \t]*\\\][ \t]*(?:\n+|$)/.exec(src);
      if (m) return { type: 'mathBlock', raw: m[0], tex: m[1].trim() };
      return undefined;
    },
    renderer(tok: any) { return `<div class="math" data-tex="${esc(tok.tex)}" data-display="1">${esc(tok.tex)}</div>\n`; },
  };
  const mathInline = {
    name: 'mathInline', level: 'inline' as const,
    start(src: string) { const i = src.search(/\$|\\\(/); return i < 0 ? undefined : i; },
    tokenizer(src: string) {
      let m = /^\$\$([^\n]+?)\$\$/.exec(src);
      if (m) return { type: 'mathInline', raw: m[0], tex: m[1].trim(), display: true };
      m = /^\\\((.+?)\\\)/.exec(src);
      if (m) return { type: 'mathInline', raw: m[0], tex: m[1].trim(), display: false };
      m = /^\$(?!\s)((?:[^$\n\\]|\\.)+?)(?<!\s)\$(?!\d)/.exec(src);
      if (m) return { type: 'mathInline', raw: m[0], tex: m[1], display: false };
      return undefined;
    },
    renderer(tok: any) { return `<span class="math" data-tex="${esc(tok.tex)}"${tok.display ? ' data-display="1"' : ''}>${esc(tok.raw)}</span>`; },
  };
  marked.use({
    gfm: true, breaks: false, extensions: [mathBlock, mathInline],
    // the diagram source — only as text inside <pre><code>: in an attribute DOMPurify would cut it because of the `-->` arrows (looks like a
    // comment's `-->`)
    renderer: { code(tok: Tokens.Code): string | false { return tok.lang === 'mermaid' ? `<div class="mermaid-src"><pre><code class="language-mermaid">${esc(tok.text)}</code></pre></div>\n` : false; } } as any,
  });
  DOMPurify.addHook('afterSanitizeAttributes', (node) => { if (node.tagName === 'A') { node.setAttribute('target', '_blank'); node.setAttribute('rel', 'noopener noreferrer'); } });

  const svgCache = new Map<string, string>();
  let seq = 0;
  let katexP: Promise<any> | null = null; let mermaidP: Promise<any> | null = null;
  const loadKatex = () => (katexP ??= Promise.all([import('katex'), import('katex/dist/katex.min.css')]).then(([k]) => k.default));
  const loadMermaid = () => (mermaidP ??= import('mermaid').then((m) => m.default));
  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); }
    catch { const ta = document.createElement('textarea'); ta.value = text; ta.style.position = 'fixed'; ta.style.opacity = '0'; document.body.appendChild(ta); ta.select(); try { document.execCommand('copy'); } catch {} ta.remove(); }
    toast(t('md.copied'));
  }
  /** TeX formulas: from data-tex, and if DOMPurify removed the attribute — from the node text
   *  without delimiters. */
  const texOf = (m: HTMLElement) => m.dataset.tex ?? (m.textContent ?? '').trim().replace(/^(\$\$|\$|\\\(|\\\[)/, '').replace(/(\$\$|\$|\\\)|\\\])$/, '').trim();
  const btn = (cls: string, title: string, label: string, on: () => void) => { const b = document.createElement('button'); b.type = 'button'; b.className = cls; b.title = title; b.textContent = label; b.onclick = (e) => { e.stopPropagation(); on(); }; return b; };

  /** The nearest scrollable ancestor: a chat stuck to the bottom is scrolled further after the asynchronous render (a diagram
   *  above the text). */
  function scrollParent(el: HTMLElement): HTMLElement | null {
    for (let p = el.parentElement; p; p = p.parentElement) { const o = getComputedStyle(p).overflowY; if ((o === 'auto' || o === 'scroll') && p.scrollHeight > p.clientHeight) return p; }
    return null;
  }
  /** Finishing after the HTML insertion: formulas, copy buttons, diagrams (the latter — only for a finished message). */
  async function enhance(el: HTMLElement, theme: 'dark' | 'light', live: boolean) {
    const sc = scrollParent(el); const atBottom = !!sc && sc.scrollHeight - sc.scrollTop - sc.clientHeight < 48;
    // The asynchronous finishing must not cancel a jump to a request or reading history. Chats say explicitly
    // whether the scroll follows the answer right now; for the other panels the position is re-checked.
    const restick = () => {
      if (!atBottom || !sc || !el.isConnected || sc.dataset.following === 'false') return;
      if (sc.dataset.following === undefined && sc.scrollHeight - sc.scrollTop - sc.clientHeight >= 48) return;
      sc.scrollTop = sc.scrollHeight;
    };
    await enhanceInner(el, theme, live, restick);
    restick();
  }
  async function enhanceInner(el: HTMLElement, theme: 'dark' | 'light', live: boolean, restick: () => void) {
    const maths = el.querySelectorAll<HTMLElement>('.math:not([data-done])');
    if (maths.length) {
      const katex = await loadKatex();
      for (const m of maths) {
        if (!m.isConnected) continue;
        try { m.innerHTML = katex.renderToString(texOf(m), { displayMode: m.dataset.display === '1', throwOnError: false, output: 'html', trust: false }); } catch { /* the source text stays */ }
        m.dataset.done = '1';
      }
    }
    for (const pre of el.querySelectorAll<HTMLElement>('pre:not([data-copy])')) {
      pre.dataset.copy = '1';
      if (pre.closest('.mermaid-src')) continue;
      pre.appendChild(btn('cp', t('md.copy'), '⧉', () => copy(pre.querySelector('code')?.textContent ?? pre.textContent ?? '')));
    }
    if (live) return;
    const diags = Array.from(el.querySelectorAll<HTMLElement>('.mermaid-src')).filter((d) => d.dataset.theme !== theme);
    if (!diags.length) return;
    const mermaid = await loadMermaid();
    mermaid.initialize({ startOnLoad: false, theme: theme === 'dark' ? 'dark' : 'default', securityLevel: 'strict', fontFamily: 'inherit' });
    for (const d of diags) {
      if (!d.isConnected) continue;
      const code = (d.querySelector('code')?.textContent ?? '').replace(/\n$/, ''); const key = theme + '\n' + code;
      let svg = svgCache.get(key); let err: string | null = null;
      if (!svg) {
        try { await mermaid.parse(code); const r = await mermaid.render(`mmd${++seq}x${Math.random().toString(36).slice(2, 8)}`, code); svg = r.svg; if (svgCache.size > 200) svgCache.clear(); svgCache.set(key, r.svg); }
        catch (e: any) { err = String(e?.message ?? e).split('\n').find((l: string) => l.trim()) ?? 'error'; }
      }
      d.dataset.theme = theme;
      let card = d.querySelector<HTMLElement>(':scope > .diagram');
      if (!card) {
        card = document.createElement('div'); card.className = 'diagram';
        const head = document.createElement('div'); head.className = 'dh';
        const title = document.createElement('span'); title.textContent = t('md.diagram');
        const sp = document.createElement('span'); sp.className = 'sp';
        head.append(title, sp, btn('tg', t('md.code'), t('md.code'), () => d.classList.toggle('src')), btn('cp2', t('md.copy'), '⧉', () => copy(code)));
        const body = document.createElement('div'); body.className = 'dg';
        card.append(head, body); d.prepend(card);
      }
      const body = card.querySelector<HTMLElement>('.dg')!;
      card.querySelector('.derr')?.remove();
      if (svg) { body.innerHTML = svg; d.classList.remove('bad'); }
      else { body.innerHTML = ''; d.classList.add('src', 'bad'); const n = document.createElement('div'); n.className = 'derr'; n.textContent = t('md.diagramError', { msg: err ?? '' }); card.appendChild(n); }
      restick();
    }
  }
</script>

<script lang="ts">
  let { text, inline = false, rich = true, live = false }: { text: string; inline?: boolean; rich?: boolean; live?: boolean } = $props();
  let root = $state<HTMLElement | null>(null);
  const html = $derived.by(() => {
    try { const raw = inline ? (marked.parseInline(text) as string) : (marked.parse(text) as string); return DOMPurify.sanitize(raw, { USE_PROFILES: { html: true }, FORBID_TAGS: ['style', 'script', 'iframe', 'form', 'input'] }); }
    catch { return ''; }
  });
  $effect(() => {
    // dependencies: html (a new DOM), theme (redraw the diagrams), live (finish the diagrams when the answer is done)
    void html; const theme = prefs.effective; const isLive = live; const el = root;
    if (!el || !rich) return;
    let cancelled = false;
    tick().then(() => { if (!cancelled) void enhance(el, theme, isLive); });
    return () => { cancelled = true; };
  });
</script>

<div class="md" bind:this={root}>{@html html}</div>

<style>
  .md { line-height: 1.5; overflow-wrap: anywhere; min-width: 0; }
  .md :global(p) { margin: 0 0 8px; } .md :global(p:last-child) { margin-bottom: 0; }
  .md :global(h1), .md :global(h2), .md :global(h3), .md :global(h4) { margin: 10px 0 6px; line-height: 1.3; font-weight: 600; }
  .md :global(h1) { font-size: 15px; } .md :global(h2) { font-size: 14px; } .md :global(h3) { font-size: 13.5px; } .md :global(h4) { font-size: 13px; }
  .md :global(ul), .md :global(ol) { margin: 4px 0 8px; padding-left: 20px; } .md :global(li) { margin: 2px 0; }
  .md :global(code) { font-family: var(--mono); font-size: 12px; background: var(--bg-4); border: 1px solid var(--line-1); border-radius: 4px; padding: 1px 5px; }
  .md :global(pre) { position: relative; margin: 6px 0 8px; padding: 8px 10px; background: var(--bg-4); border: 1px solid var(--line-1); border-radius: var(--r-1); overflow: auto; max-width: 100%; }
  .md :global(pre code) { background: transparent; border: 0; padding: 0; white-space: pre; }
  .md :global(pre .cp) { position: absolute; top: 4px; right: 4px; opacity: 0; transition: opacity .15s; font-size: 12px; line-height: 1; padding: 3px 6px; border-radius: 4px; background: var(--bg-3); border: 1px solid var(--line-2); color: var(--fg-2); cursor: pointer; }
  .md :global(pre:hover .cp), .md :global(pre .cp:focus-visible) { opacity: 1; }
  @media (hover: none) { .md :global(pre .cp) { opacity: 1; } }
  .md :global(table) { display: block; width: max-content; max-width: 100%; overflow-x: auto; border-collapse: collapse; margin: 6px 0 8px; font-size: 12.5px; }
  .md :global(th), .md :global(td) { border: 1px solid var(--line-2); padding: 4px 8px; text-align: left; vertical-align: top; }
  .md :global(th) { background: var(--bg-3); font-weight: 600; }
  .md :global(blockquote) { margin: 6px 0; padding: 4px 10px; border-left: 3px solid var(--line-3); color: var(--fg-2); }
  .md :global(hr) { border: 0; border-top: 1px solid var(--line-2); margin: 8px 0; }
  .md :global(a) { color: var(--accent); text-decoration: underline; text-underline-offset: 2px; }
  .md :global(img) { max-width: 100%; border-radius: var(--r-1); }
  .md :global(strong) { font-weight: 600; }
  /* formulas */
  .md :global(.math[data-display="1"]) { display: block; text-align: center; margin: 8px 0; overflow-x: auto; overflow-y: hidden; padding: 2px 0; }
  .md :global(.math:not([data-done])) { font-family: var(--mono); font-size: 12px; color: var(--fg-2); }
  .md :global(.katex-display) { margin: 0; }
  .md :global(.katex) { font-size: 1.08em; }
  /* diagrams */
  .md :global(.mermaid-src) { margin: 6px 0 8px; }
  .md :global(.mermaid-src:not(.src) > pre) { display: none; }
  .md :global(.mermaid-src.src > .diagram > .dg) { display: none; }
  .md :global(.mermaid-src > pre) { margin: 4px 0 0; }
  .md :global(.diagram) { border: 1px solid var(--line-1); border-radius: var(--r-1); background: var(--bg-4); padding: 6px 8px 8px; }
  .md :global(.diagram .dh) { display: flex; align-items: center; gap: 6px; font-size: 11.5px; color: var(--fg-2); margin-bottom: 4px; }
  .md :global(.diagram .dh .sp) { flex: 1; }
  .md :global(.diagram .dh button) { font-size: 11.5px; line-height: 1.2; padding: 2px 6px; border-radius: 4px; background: var(--bg-3); border: 1px solid var(--line-2); color: var(--fg-2); cursor: pointer; }
  .md :global(.diagram .dg) { overflow-x: auto; display: flex; justify-content: center; }
  .md :global(.diagram .dg svg) { max-width: 100%; height: auto; }
  .md :global(.diagram .derr) { color: var(--bad); font-size: 12px; margin-top: 4px; }
</style>

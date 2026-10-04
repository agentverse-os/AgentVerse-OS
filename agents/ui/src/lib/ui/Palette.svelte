<script lang="ts">
  // Палитра команд (Ctrl+K): сессии, разделы, создание сессий; стрелки и Enter.
  import { data, ui, actions, SECTIONS, agentOf, type Ref } from '../state.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  let { onclose }: { onclose: () => void } = $props();
  let q = $state('');
  let sel = $state(0);
  let input: HTMLInputElement;
  type Item = { text: string; icon: string; kind: string; run: () => void };
  const items = $derived.by((): Item[] => {
    const list: Item[] = [
      ...data.sessions().map(s => ({ text: `${s.title} · ${s.ws}`, icon: agentOf({ project: s.ws, kind: 'session', id: s.id }) === 'shell' ? 'terminal' : s.agent, kind: t('pal.session'), run: () => ui.goto({ project: s.ws, kind: 'session', id: s.id }) })),
      ...data.running.flatMap(p => [
        { text: `${t('menu.chatClaude')} · ${p.name}`, icon: 'plus', kind: t('pal.create'), run: () => actions.createSession(p.name, 'claude') },
        { text: `${t('menu.chatCodex')} · ${p.name}`, icon: 'plus', kind: t('pal.create'), run: () => actions.createSession(p.name, 'codex') },
      ]),
      ...SECTIONS.map(s => ({ text: s.name, icon: s.icon, kind: t('pal.section'), run: () => { ui.section = s.id; } })),
    ];
    const needle = q.trim().toLowerCase();
    return (needle ? list.filter(i => i.text.toLowerCase().includes(needle)) : list).slice(0, 10);
  });
  $effect(() => { input?.focus(); });
  $effect(() => { q; sel = 0; });
  function run(i: Item) { onclose(); i.run(); }
  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { sel = Math.min(items.length - 1, sel + 1); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { sel = Math.max(0, sel - 1); e.preventDefault(); }
    else if (e.key === 'Enter') { const i = items[sel]; if (i) run(i); }
    else if (e.key === 'Escape') onclose();
  }
</script>

<div class="palette" role="dialog" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="box">
    <div class="q"><Icon name="search" /><input bind:this={input} bind:value={q} placeholder={t('pal.placeholder')} autocomplete="off" onkeydown={key} /></div>
    <div class="res">
      {#each items as i, idx (i.text + i.kind)}<button class="mi" class:sel={idx === sel} onclick={() => run(i)}><Icon name={i.icon} /><span>{i.text}</span><span class="k">{i.kind}</span></button>{/each}
      {#if !items.length}<div class="empty" style="padding: 24px"><span>{t('pal.none')}</span></div>{/if}
    </div>
    <div class="foot"><span><kbd>↑↓</kbd> {t('pal.up')}</span><span><kbd>↵</kbd> {t('pal.enter')}</span><span><kbd>esc</kbd> {t('pal.esc')}</span></div>
  </div>
</div>

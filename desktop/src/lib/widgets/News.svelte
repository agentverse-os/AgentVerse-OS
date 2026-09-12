<script lang="ts">
  // Виджет «Новости»: несколько RSS/Atom-лент через прокси ядра, склейка дубликатов, фильтр по источнику,
  // отметка прочитанного, картинки, чтение внутри Desktop (окно «Читалка»), настройки лент и интервала.
  import { NEWS_PRESETS } from '../widgets.svelte';
  import { wm } from '../windows.svelte';
  import { ago } from './util';
  import { t, tn } from '../i18n.svelte';
  import { newsCache, decode, shortSource, sourceColor, readSet, markRead, titleKey, cleanSummary, type NewsItem } from './news-util';
  let { config, onconfig }: { config: Record<string, any>; onconfig: (p: Record<string, any>) => void } = $props();
  let items = $state<NewsItem[]>([]);
  let loading = $state(false);
  let errors = $state<{ url: string; error: string }[]>([]);
  let lastAt = $state<number | null>(null);
  let filter = $state<string | null>(null);
  let settings = $state(false);
  let custom = $state('');
  let testing = $state('');
  let read = $state<Set<string>>(readSet());
  let tick = $state(0);
  const urls = $derived.by(() => {
    const preset = NEWS_PRESETS[config.preset] ?? NEWS_PRESETS.ai;
    const lang = config.lang ?? 'both';
    const base = config.preset === 'custom' ? [] : preset.feeds.filter(f => lang === 'both' || f.lang === lang).map(f => f.url);
    return [...base, ...((config.feeds ?? []) as string[])];
  });
  const count = $derived(Number(config.count ?? 10));
  const interval = $derived(Number(config.interval ?? 15));
  const showImages = $derived(config.images !== false);
  const compact = $derived(config.compact === true);
  async function load(force = false) {
    loading = true;
    const res = await Promise.all(urls.map(async u => {
      try { const r = await fetch(`/api/widgets/feed?url=${encodeURIComponent(u)}${force ? '&max_age=0' : ''}`); const j = await r.json(); return { ...j, url: u }; }
      catch (e) { return { items: [], error: String(e), url: u }; }
    }));
    errors = res.filter(r => r.error).map(r => ({ url: r.url, error: r.error }));
    const all: NewsItem[] = res.flatMap(r => (r.items ?? []).map((i: NewsItem) => ({ ...i, feed: r.url })));
    const seenLink = new Set<string>(); const seenTitle = new Set<string>();
    const merged = all.filter(i => {
      if (!i.link || !i.title) return false;
      const tk = titleKey(i.title);
      if (seenLink.has(i.link) || (tk && seenTitle.has(tk))) return false;
      seenLink.add(i.link); if (tk) seenTitle.add(tk); return true;
    }).sort((a, b) => new Date(b.published ?? 0).getTime() - new Date(a.published ?? 0).getTime());
    items = merged.slice(0, Math.max(count, 30)); // держим запас для фильтра по источнику
    for (const i of items) newsCache.set(i.link, i);
    lastAt = Date.now(); loading = false;
  }
  $effect(() => { urls; load(); });
  $effect(() => { const t = setInterval(() => load(), Math.max(1, interval) * 60 * 1000); const tk = setInterval(() => tick++, 30000); return () => { clearInterval(t); clearInterval(tk); }; });
  const sources = $derived([...new Set(items.map(i => shortSource(i.source)))]);
  const shown = $derived(items.filter(i => !filter || shortSource(i.source) === filter).slice(0, count));
  function openItem(i: NewsItem) { markRead(i.link); read = readSet(); wm.openSys('reader', { link: i.link, feed: i.feed }, decode(i.title)); }
  async function addFeed() {
    const u = custom.trim(); if (!u) return;
    testing = t('widgets.news.checking');
    try {
      const j = await (await fetch(`/api/widgets/feed?url=${encodeURIComponent(u)}`)).json();
      if (j.error) { testing = t('widgets.news.unreadable', { error: j.error }); return; }
      onconfig({ feeds: [...(config.feeds ?? []), u] }); custom = ''; testing = t('widgets.news.added', { title: j.title, items: tn('widgets.news.entries', (j.items ?? []).length) });
    } catch (e) { testing = t('widgets.news.unreadable', { error: e instanceof Error ? e.message : String(e) }); }
  }
  const presetLabel = $derived(t(NEWS_PRESETS[config.preset]?.label ?? 'widgets.news.presetOwn'));
  const updated = $derived.by(() => { tick; return lastAt ? ago(new Date(lastAt).toISOString()) : ''; });
</script>

{#if settings}
  <div class="cfg">
    <div class="row"><span class="muted">{t('widgets.news.presetLabel')}</span>{#each Object.entries(NEWS_PRESETS) as [k, p]}<button class:primary={config.preset === k} onclick={() => onconfig({ preset: k })}>{t(p.label)}</button>{/each}</div>
    <div class="row">
      <span class="muted">{t('widgets.news.langLabel')}</span>{#each [['both', t('widgets.news.both')], ['en', 'EN'], ['ru', 'RU']] as [v, l]}<button class:primary={(config.lang ?? 'both') === v} onclick={() => onconfig({ lang: v })}>{l}</button>{/each}
      <span class="muted">{t('widgets.news.showLabel')}</span>{#each [5, 10, 20, 30] as n}<button class:primary={count === n} onclick={() => onconfig({ count: n })}>{n}</button>{/each}
    </div>
    <div class="row">
      <span class="muted">{t('widgets.news.refreshLabel')}</span>{#each [5, 15, 30, 60] as n}<button class:primary={interval === n} onclick={() => onconfig({ interval: n })}>{t('time.min', { n })}</button>{/each}
      <button class:primary={showImages} onclick={() => onconfig({ images: !showImages })}>{t('widgets.news.images')}</button>
      <button class:primary={compact} onclick={() => onconfig({ compact: !compact })}>{t('widgets.news.compact')}</button>
    </div>
    <div class="row"><input placeholder={t('widgets.news.feedPlaceholder')} bind:value={custom} style="flex:1" onkeydown={(e) => e.key === 'Enter' && addFeed()} /><button onclick={addFeed}>{t('widgets.news.addFeed')}</button></div>
    {#if testing}<div class="muted small">{testing}</div>{/if}
    {#each config.feeds ?? [] as f}<div class="row muted small"><span style="flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap" title={f}>{f}</span><button title={t('widgets.news.removeFeed')} onclick={() => onconfig({ feeds: (config.feeds ?? []).filter((x: string) => x !== f) })}>✕</button></div>{/each}
    {#if errors.length}<div class="muted small">{t('widgets.news.unavailable')} {#each errors as e}<div title={e.error}>· {e.url} — {e.error.slice(0, 80)}</div>{/each}</div>{/if}
    <div class="row"><button class="primary" onclick={() => { settings = false; testing = ''; }}>{t('common.done')}</button></div>
  </div>
{:else}
  <div class="head row">
    <span class="muted small">{presetLabel} · {count}{#if updated} · {updated === t('time.now') ? t('widgets.news.updatedNow') : t('widgets.news.updatedAgo', { ago: updated })}{/if}</span>
    <span style="flex:1"></span>
    <button class="mini" title={t('widgets.news.refresh')} onclick={() => load(true)} disabled={loading}>{#if loading}<span class="spin"></span>{:else}↻{/if}</button>
    <button class="mini" title={t('widgets.news.feedSettings')} onclick={() => settings = true}>⚙</button>
  </div>
  {#if sources.length > 1}
    <div class="chips">
      <button class="chip" class:on={!filter} onclick={() => filter = null}>{t('widgets.news.all')}</button>
      {#each sources as s}<button class="chip" class:on={filter === s} title={s} onclick={() => filter = filter === s ? null : s}><span class="dot" style="background:{sourceColor(s)}"></span>{s}</button>{/each}
    </div>
  {/if}
  <div class="news" class:compact>
    {#each shown as i (i.link)}
      <article class="item" class:read={read.has(i.link)}>
        {#if showImages && !compact && i.image}<img class="thumb" src={i.image} alt="" loading="lazy" onerror={(e) => (e.currentTarget as HTMLImageElement).style.display = 'none'} />{/if}
        <div class="txt">
          <button class="title" onclick={() => openItem(i)}>{decode(i.title)}</button>
          {#if !compact && cleanSummary(i.summary)}<div class="muted sum">{cleanSummary(i.summary)}</div>{/if}
          <div class="meta muted small"><span class="dot" style="background:{sourceColor(shortSource(i.source))}"></span>{shortSource(i.source)} · {ago(i.published)}{#if i.author && !compact} · {i.author}{/if}<a class="ext" href={i.link} target="_blank" rel="noopener" title={t('widgets.news.openOriginal')}>↗</a></div>
        </div>
      </article>
    {/each}
    {#if !shown.length && !loading}<div class="muted">{t('widgets.news.none')}{errors.length ? ` · ${tn('widgets.news.feedsDown', errors.length)}` : ''}</div>{/if}
    {#if loading && !items.length}{#each Array(4) as _}<div class="item"><div class="txt"><div class="skeleton" style="height:16px;width:90%"></div><div class="skeleton" style="height:12px;width:40%;margin-top:6px"></div></div></div>{/each}{/if}
  </div>
{/if}

<style>
  .news { display: flex; flex-direction: column; gap: 8px; font-size: .9rem; overflow: auto; }
  .item { display: flex; gap: 10px; align-items: flex-start; }
  .item.read .title { color: var(--muted); }
  .thumb { width: 56px; height: 56px; border-radius: 8px; object-fit: cover; flex: none; background: var(--panel-2); }
  .txt { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .title { background: transparent; border: 0; padding: 0; text-align: left; color: var(--ink); font: inherit; line-height: 1.3; cursor: pointer; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .title:hover { color: var(--accent); }
  .sum { font-size: .8rem; line-height: 1.3; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .compact .item { gap: 6px; } .compact .title { -webkit-line-clamp: 1; }
  .meta { display: flex; align-items: center; gap: 5px; }
  .meta .ext { margin-left: auto; color: var(--muted); text-decoration: none; padding: 0 4px; } .meta .ext:hover { color: var(--accent); }
  .dot { width: 7px; height: 7px; border-radius: 50%; display: inline-block; flex: none; }
  .chips { display: flex; gap: 4px; overflow-x: auto; padding-bottom: 4px; margin-bottom: 4px; scrollbar-width: none; }
  .chip { display: inline-flex; align-items: center; gap: 5px; padding: 1px 8px; font-size: .75rem; border-radius: 999px; white-space: nowrap; background: transparent; }
  .chip.on { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 15%, transparent); }
  .small { font-size: .75rem; }
  .mini { padding: 0 6px; font-size: .8rem; min-width: 26px; }
  .cfg { display: flex; flex-direction: column; gap: 6px; font-size: .85rem; }
  .cfg .row { flex-wrap: wrap; gap: 4px; }
</style>

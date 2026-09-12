<script lang="ts">
  // «Читалка»: статья из RSS внутри Desktop — заголовок, источник, время, картинка, очищенный HTML или анонс, ссылка на оригинал.
  import DOMPurify from 'dompurify';
  import { newsCache, decode, shortSource, cleanSummary, type NewsItem } from './widgets/news-util';
  import { t, fmt } from './i18n.svelte';
  let { link, feed }: { link: string; feed?: string } = $props();
  let item = $state<NewsItem | null>(newsCache.get(link) ?? null);
  let error = $state('');
  $effect(() => {
    if (item || !feed) return;
    fetch(`/api/widgets/feed?url=${encodeURIComponent(feed)}`).then(r => r.json()).then(f => {
      const it = (f.items ?? []).find((x: NewsItem) => x.link === link);
      if (it) { const full: NewsItem = { ...it, feed }; item = full; newsCache.set(link, full); } else error = t('reader.gone');
    }).catch(e => error = e instanceof Error ? e.message : String(e));
  });
  const html = $derived.by(() => {
    if (!item?.content) return '';
    const clean = DOMPurify.sanitize(item.content, { FORBID_TAGS: ['script', 'style', 'iframe', 'form', 'input', 'object', 'embed'], ADD_ATTR: ['target', 'rel'] });
    return clean.replace(/<a\s/gi, '<a target="_blank" rel="noopener" ');
  });
  let copied = $state(false);
  // «5 мин назад», но «только что» — без «назад»
  const agoText = (iso: string) => { const a = fmt.ago(iso); return a === t('time.now') ? a : t('reader.ago', { ago: a }); };
</script>

<article class="reader">
  {#if item}
    <header>
      <h2>{decode(item.title)}</h2>
      <div class="muted meta">{shortSource(item.source)}{#if item.author} · {item.author}{/if}{#if item.published} · {agoText(item.published)} · {fmt.dateTime(item.published, { dateStyle: 'medium', timeStyle: 'short' })}{/if}</div>
    </header>
    {#if item.image && !/<img\s/i.test(html)}<img class="hero" src={item.image} alt="" onerror={(e) => (e.currentTarget as HTMLImageElement).style.display = 'none'} />{/if}
    {#if html}
      <div class="body">{@html html}</div>
    {:else if cleanSummary(item.summary)}
      <p class="lead">{cleanSummary(item.summary)}</p>
      <p class="muted small">{t('reader.summaryOnly')}</p>
    {/if}
    <footer class="row">
      <a href={link} target="_blank" rel="noopener"><button class="primary">{t('reader.openOriginal')}</button></a>
      <button onclick={() => { navigator.clipboard?.writeText(link); copied = true; setTimeout(() => copied = false, 1500); }}>{copied ? t('common.copied') : t('reader.copyLink')}</button>
    </footer>
  {:else if error}
    <div class="card" style="border-color: var(--bad)">{error}</div>
    <a href={link} target="_blank" rel="noopener"><button class="primary">{t('reader.openOriginal')}</button></a>
  {:else}
    <div class="muted">{t('reader.loading')}</div>
  {/if}
</article>

<style>
  .reader { display: flex; flex-direction: column; gap: 12px; max-width: 760px; margin: 0 auto; }
  h2 { margin: 0; line-height: 1.25; font-size: 1.35rem; }
  .meta { font-size: .85rem; }
  .hero { max-width: 100%; max-height: 360px; object-fit: cover; border-radius: var(--r-sm); }
  .lead { font-size: 1.05rem; line-height: 1.5; margin: 0; }
  .small { font-size: .8rem; }
  .body { line-height: 1.6; font-size: 1rem; overflow-wrap: anywhere; }
  .body :global(img) { max-width: 100%; height: auto; border-radius: var(--r-sm); }
  .body :global(pre) { overflow: auto; background: var(--panel-2); padding: 8px; border-radius: var(--r-sm); font-size: .85rem; }
  .body :global(a) { color: var(--accent); }
  .body :global(figure) { margin: 10px 0; }
  .body :global(h1), .body :global(h2), .body :global(h3) { line-height: 1.3; }
  footer { gap: 8px; padding-top: 6px; border-top: 1px solid var(--line); }
</style>

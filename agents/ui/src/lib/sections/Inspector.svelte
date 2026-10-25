<script lang="ts">
  // Inspector of the selected session: properties (model, account, mode — edited here), usage,
  // actions.
  import { ui, data, actions, titleOf, agentOf, agentName, dotOf, statusText, modeLabel, money, fmtK, ago, live, stopLabel, MODELS_FOR, MODES_FOR } from '../state.svelte';
  import { t, tn } from '../i18n';
  import Mark from '../Mark.svelte';
  import Icon from '../Icon.svelte';
  import Md from '../Md.svelte';
  import type { SessionInfo, PtyInfo } from '../api';
  const r = $derived(ui.sel);
  const info = $derived(data.infoOf(r));
  const s = $derived(r?.kind === 'session' ? (info as SessionInfo | null) : null);
  const p = $derived(r?.kind === 'pty' ? (info as PtyInfo | null) : null);
  const accountLabel = $derived(s?.account ? (data.accounts.find(a => a.id === s.account)?.label ?? s.account) : t('insp.gate'));
</script>

{#if !r || !info}
  <div class="empty" style="padding: 30px 16px"><b>{t('insp.none')}</b><span>{t('insp.noneHint')}</span></div>
{:else}
  <section>
    <div class="row"><span class="av {agentOf(r)} lg"><Icon name={agentOf(r) === 'shell' ? 'terminal' : agentOf(r)} /></span><div style="min-width: 0"><b class="md ell" style="display: block">{#if r.kind === 'session'}<Mark id={r.id} /> {/if}{titleOf(r)}</b><span class="muted sm">{r.project} · {agentName(agentOf(r))} · {ago(info.created_at)}</span></div></div>
  </section>
  {#if s}
    <section>
      <div class="kv">
        <span>{t('insp.status')}</span><span class="row"><span class="dot {dotOf(r)}"></span> {statusText(s)}</span>
        {#if s.label}<span>{t('insp.autoTitle')}</span><span class="ell" title={s.title}>{s.title}</span>{/if}
        <span>{t('insp.model')}</span><select class="sm" aria-label={t('insp.model')} value={s.model ?? ''} onchange={(e) => actions.setModel(r!, (e.target as HTMLSelectElement).value || null)}><option value="">{t('agents.modelDefault')}</option>{#each Array.from(new Set([...(MODELS_FOR[s.agent] ?? []), ...(s.model ? [s.model] : [])])) as m (m)}<option value={m}>{m}</option>{/each}</select>
        <span>{t('insp.account')}</span><select class="sm" aria-label={t('insp.account')} value={s.account ?? ''} onchange={(e) => actions.setAccount(r!, (e.target as HTMLSelectElement).value || null)}><option value="">{t('insp.gate')}</option>{#each data.accounts as a (a.id)}<option value={a.id}>{a.label}</option>{/each}</select>
        <span>{t('insp.route')}</span><span class="ell">{accountLabel}</span>
        <span>{t('insp.source')}</span><span class="ell">{s.imported ? t('insp.srcDisk') : t('insp.srcOwn')}{#if s.updated_at} · {t('insp.updated')} {ago(s.updated_at)}{/if}</span>
        <span>{t('insp.mode')}</span><select class="sm" aria-label={t('insp.mode')} value={s.permission_mode ?? (s.agent === 'codex' ? 'on-request' : 'default')} onchange={(e) => actions.setMode(r!, (e.target as HTMLSelectElement).value)}>{#each MODES_FOR[s.agent] ?? MODES_FOR.claude as m (m)}<option value={m}>{modeLabel(m)}</option>{/each}</select>
      </div>
    </section>
    <section>
      <div class="h-sec" style="margin: 0">{t('insp.usage')}</div>
      <div class="row"><b class="lg num">{money(s.usage?.cost_usd)}</b><span class="muted sm">· {tn('agents.turns', s.usage?.turns ?? 0)}</span></div>
      <div class="kv"><span>{t('insp.tokens')}</span><span class="num">{fmtK(s.usage?.input)}↑ {fmtK(s.usage?.output)}↓</span><span>{t('insp.cache')}</span><span class="num">{fmtK(s.usage?.cache_read)}</span></div>
      {#if s.last_text}<div class="muted sm snippet"><Md rich={false} text={s.last_text} /></div>{/if}
    </section>
  {:else if p}
    <section><div class="kv"><span>{t('insp.status')}</span><span class="row"><span class="dot {dotOf(r)}"></span> {p.alive ? t('insp.alive') : t('st.exited')}</span><span>{t('insp.viewers')}</span><span>{p.clients}</span><span>{t('insp.dir')}</span><code class="ell">{p.cwd}</code></div></section>
  {/if}
  <section>
    <div class="row wrap">
      {#if s && !live(s)}<button class="sm primary" onclick={() => actions.resume(r!)}>{t('sess.menu.resume')}</button>{/if}
      <button class="sm {live(info) ? '' : 'danger'}" onclick={() => actions.stopOrRemove(r!)}>{stopLabel(info, true)}</button>
      {#if ui.view === 'thread'}<button class="sm ghost" onclick={() => ui.toCanvas(r!)}><Icon name="grid" size="s" /> {t('top.toCanvas')}</button>{/if}
    </div>
    {#if data.project(r.project)?.apps.some(a => a.slug === 'code-server')}<button class="sm ghost" style="justify-content: flex-start" onclick={() => actions.ide(r!.project)}><Icon name="ext" size="s" /> {t('insp.ide')}</button>{/if}
    <!-- dev server preview (§4.9/4.10): enabled — open the sidebar tab, disabled — enable (on desktop it opens right away) -->
    <button class="sm ghost insp-preview" style="justify-content: flex-start" onclick={() => { if (data.browserOf(r!.project).preview) ui.openPreview(); else actions.togglePreview(r!.project); }}><Icon name="globe" size="s" /> {data.browserOf(r!.project).preview ? t('browser.tab') : t('browser.menuPreview')}</button>
  </section>
{/if}

<style>
  .snippet { max-height: 160px; overflow: auto; }
</style>

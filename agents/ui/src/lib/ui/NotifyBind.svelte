<!-- "Report to a channel" (docs/agents-ux.md §9.12.2): what a conversation, a workspace or a routine tells which channel.
     One dialog for the whole app, above everything; every change is saved at once — there is nothing to confirm. -->
<script lang="ts">
  import { notify, kindName, kindIcon, type BindTarget } from '../notify.svelte';
  import type { NotifyBinding, NotifyChannel } from '../api';
  import { ui } from '../state.svelte';
  import { t } from '../i18n';
  import { dialog } from './dialog';
  import Icon from '../Icon.svelte';
  let { target, onclose }: { target: BindTarget; onclose: () => void } = $props();
  let busy = $state('');
  let err = $state('');
  const mine = $derived(notify.of(target));
  const of = (c: NotifyChannel): NotifyBinding | undefined => mine.find(b => b.channel === c.id);
  const head = $derived(target.scope === 'session' ? t('ntf.bindSession', { title: target.title, project: target.project }) : target.scope === 'workspace' ? t('ntf.bindWorkspace', { project: target.project }) : t('ntf.bindRoutine', { title: target.title }));
  const base = (c: NotifyChannel) => ({ channel: c.id, scope: target.scope, project: target.project, session: target.session ?? '', routine: target.routine ?? '' });
  async function run(c: NotifyChannel, fn: () => Promise<void>) {
    if (busy) return; busy = c.id; err = '';
    try { await fn(); } catch (e) { err = e instanceof Error ? e.message : String(e); } finally { busy = ''; }
  }
  function setWhen(c: NotifyChannel, v: string) {
    const b = of(c);
    void run(c, async () => {
      if (v === 'off') { if (b) await notify.unbind(b.id); return; }
      await notify.bind({ ...base(c), when: v === 'once' ? 'once' : 'always', content: b?.content ?? 'fact', ...(b ? { reports: b.reports, away_only: b.away_only } : {}) });
    });
  }
  function patch(c: NotifyChannel, p: Partial<Pick<NotifyBinding, 'content' | 'reports' | 'away_only'>>) {
    const b = of(c); if (!b) return;
    void run(c, () => notify.bind({ ...base(c), when: b.when, content: b.content, reports: b.reports, away_only: b.away_only, ...p }));
  }
  const whenOf = (b?: NotifyBinding) => (!b ? 'off' : target.scope === 'session' ? b.when : 'on');
  function toSettings() { onclose(); ui.settingsTab = 'notify'; ui.section = 'settings'; }
</script>

<div class="ntf-scrim" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="ntf-dlg" use:dialog={onclose} role="dialog" aria-modal="true" aria-labelledby="ntf-bind-title" tabindex="-1">
    <h3 id="ntf-bind-title"><Icon name="bell" size="s" /> {t('ntf.bindTitle')}</h3>
    <p class="ntf-target">{head}</p>
    {#if notify.error}<p class="err" role="alert">{notify.error}</p>{/if}
    {#if !notify.loaded}
      <p class="muted sm" role="status">{t('common.loading')}</p>
    {:else if !notify.channels.length}
      <p class="muted sm ntf-none">{t('ntf.noChannels')}</p>
    {:else}
      <div class="ntf-rows">
        {#each notify.channels as c (c.id)}
          {@const b = of(c)}
          <fieldset class="ntf-row" class:on={!!b} disabled={busy === c.id} data-channel={c.id} aria-label={`${kindName(c.kind)} · ${c.name}`}>
            <div class="ntf-row-head">
              <span class="ntf-name"><Icon name={kindIcon(c.kind)} size="s" /> <b>{kindName(c.kind)}</b><span class="ntf-nm">· {c.name}</span>{#if !c.enabled}<span class="chip warn">{t('ntf.disabledCh')}</span>{/if}</span>
              <select class="sm ntf-when" aria-label={t('ntf.when')} value={whenOf(b)} onchange={(e) => setWhen(c, e.currentTarget.value)}>
                <option value="off">{t('ntf.whenOff')}</option>
                {#if target.scope === 'session'}<option value="once">{t('ntf.whenOnce')}</option><option value="always">{t('ntf.whenAlways')}</option>{:else}<option value="on">{t('ntf.whenOn')}</option>{/if}
              </select>
            </div>
            {#if b}
              <div class="ntf-opt" role="radiogroup" aria-label={t('ntf.what')}>
                <span class="muted sm">{t('ntf.what')}</span>
                {#each [['fact', t('ntf.cFact')], ['brief', t('ntf.cBrief')], ['full', t('ntf.cFull')]] as [k, l] (k)}<label><input type="radio" name={`ntf-content-${c.id}`} value={k} checked={b.content === k} onchange={() => patch(c, { content: k as NotifyBinding['content'] })} /> {l}</label>{/each}
              </div>
              {#if b.content !== 'fact'}<p class="muted xs ntf-outside">{t('ntf.outside')}</p>{/if}
              {#if target.scope === 'workspace'}
                <label class="ntf-opt-check"><input type="checkbox" class="ntf-away" checked={b.away_only} onchange={(e) => patch(c, { away_only: e.currentTarget.checked })} /> {t('ntf.awayOnly')}</label>
              {:else}
                <label class="ntf-opt-check"><input type="checkbox" class="ntf-reports" checked={b.reports} onchange={(e) => patch(c, { reports: e.currentTarget.checked })} /> {t('ntf.allowReports')}</label>
                {#if b.reports && !c.files}<p class="muted xs">{t('ntf.reportsNoFiles')}</p>{/if}
              {/if}
            {/if}
          </fieldset>
        {/each}
      </div>
      <p class="muted xs">{target.scope === 'routine' ? t('ntf.routineRule') : t('ntf.sessionAlso')}</p>
    {/if}
    {#if err}<p class="err ntf-bind-err" role="alert">{err}</p>{/if}
    <div class="ntf-dlg-acts"><button class="sm ghost ntf-to-settings" onclick={toSettings}><Icon name="settings" size="s" /> {t('ntf.openSettings')}</button><span class="sp"></span><button class="sm primary ntf-done" onclick={onclose}>{t('ntf.done')}</button></div>
  </div>
</div>

<style>
  .ntf-scrim { position: fixed; inset: 0; z-index: 110; background: rgb(0 0 0 / .5); backdrop-filter: blur(3px); display: grid; place-items: center; padding: 16px; }
  .ntf-dlg { width: min(540px, 100%); max-height: calc(100dvh - 32px); overflow: auto; padding: 16px; border-radius: var(--r-3, 14px); background: var(--bg-1); border: 1px solid var(--line-2); box-shadow: 0 18px 50px rgb(0 0 0 / .4); animation: pop var(--dur) ease-out; display: grid; gap: 10px; }
  .ntf-dlg h3 { margin: 0; font-size: 14px; display: flex; align-items: center; gap: 8px; }
  .ntf-dlg p { margin: 0; }
  /* titles of conversations and names of channels are the owner's own words of any length */
  .ntf-target, .ntf-name, .ntf-bind-err { overflow-wrap: anywhere; min-width: 0; }
  .ntf-target { color: var(--fg-2); font-size: 13px; }
  .ntf-rows { display: grid; gap: 8px; }
  .ntf-row { display: grid; gap: 8px; margin: 0; padding: 10px 12px; border: 1px solid var(--line-1); border-radius: var(--r-2); background: var(--bg-2); min-width: 0; }
  .ntf-row.on { border-color: rgb(var(--accent-rgb) / .45); }
  .ntf-row-head { display: flex; gap: 8px; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  .ntf-name { display: inline-flex; gap: 6px; align-items: center; font-size: 13px; }
  .ntf-name b, .ntf-name :global(.i), .ntf-name .chip { flex: none; }
  .ntf-nm { min-width: 0; overflow-wrap: anywhere; }
  .ntf-when { max-width: 100%; }
  .ntf-opt { display: flex; gap: 6px 14px; align-items: center; flex-wrap: wrap; font-size: 13px; }
  .ntf-opt label, .ntf-opt-check { display: inline-flex; gap: 6px; align-items: center; cursor: pointer; font-size: 13px; }
  .ntf-dlg-acts { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  @media (pointer: coarse) { .ntf-when, .ntf-dlg-acts button { min-height: 44px; } .ntf-opt label, .ntf-opt-check { min-height: 32px; } }
</style>

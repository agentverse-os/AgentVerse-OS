<script lang="ts">
  // Мастер установки (docs/userflow-apps.md, шаг 2.1): обязательные настройки до первого запуска, остальные — по желанию;
  // после установки родитель открывает «Готово, данные для входа».
  import { api, type AppView, type AppSetting } from './api';
  import { titleOf } from './store-utils';
  import Dropdown from './Dropdown.svelte';
  import { toast } from './toast.svelte';
  import { fade } from 'svelte/transition';
  import { t } from './i18n.svelte';
  let { app, onclose, ondone }: { app: AppView; onclose: () => void; ondone: (v: AppView) => void } = $props();
  const required = app.manifest.settings.filter(s => s.required);
  const optional = app.manifest.settings.filter(s => !s.required);
  const generated = Object.entries(app.manifest.env ?? {}).filter(([, v]) => typeof v === 'object').map(([k]) => k);
  let form = $state<Record<string, string>>({ ...app.settings_values });
  let showAll = $state(false);
  let busy = $state('');
  let err = $state('');
  function inputType(s: AppSetting) { return s.type === 'password' ? 'password' : s.type === 'number' ? 'number' : 'text'; }
  async function go() {
    const missing = required.filter(s => !(form[s.env] ?? '').trim());
    if (missing.length) { err = t('store.fillIn', { fields: missing.map(s => s.label ?? s.env).join(', ') }); return; }
    busy = t('store.savingSettings'); err = '';
    try {
      const values: Record<string, string> = {};
      for (const s of [...required, ...optional]) if (form[s.env] !== undefined && form[s.env] !== '••••••') values[s.env] = form[s.env];
      if (Object.keys(values).length) await api.settings(app.name, values);
      const v = await toast.track(t('store.installingTitle', { name: titleOf(app) }), progress => api.installWait(app.name, st => { busy = st; progress(st); }), () => ({ text: t('store.installed') }));
      ondone(v);
    } catch (e) { err = e instanceof Error ? e.message : String(e); } finally { busy = ''; }
  }
</script>

<div class="overlay" role="dialog" aria-label={t('store.installDialog')} tabindex="-1" transition:fade={{ duration: 120 }} onkeydown={(e) => e.key === 'Escape' && !busy && onclose()}>
  <div class="dlg">
    <h3>{t('store.installTitle', { name: titleOf(app) })}</h3>
    {#if required.length}
      <p class="muted small">{t('store.requiredHint')}</p>
      {#each required as s}
        <label class="fld"><span>{s.label ?? s.env}{#if s.hint}<span class="muted small"> · {s.hint}</span>{/if}</span>
          {#if s.type === 'select' && s.options}<Dropdown bind:value={form[s.env]} ariaLabel={s.label ?? s.env} options={s.options} />
          {:else}<input type={inputType(s)} bind:value={form[s.env]} placeholder={s.type} />{/if}</label>
      {/each}
    {/if}
    {#if generated.length}<p class="muted small">{t('store.generatedHint', { list: generated.join(', ') })}</p>{/if}
    {#if optional.length}
      <button onclick={() => showAll = !showAll}>{showAll ? t('store.hideOptional') : t('store.showOptional', { n: optional.length })}</button>
      {#if showAll}
        {#each optional as s}
          <label class="fld"><span>{s.label ?? s.env}{#if s.hint}<span class="muted small"> · {s.hint}</span>{/if}</span>
            {#if s.type === 'select' && s.options}<Dropdown bind:value={form[s.env]} ariaLabel={s.label ?? s.env} options={s.options} />
            {:else}<input type={inputType(s)} bind:value={form[s.env]} placeholder={s.type} />{/if}</label>
        {/each}
      {/if}
    {/if}
    {#if err}<div class="small" style="color: var(--bad)">{err}</div>{/if}
    <div class="row">
      <button class="primary" disabled={!!busy} onclick={go}>{#if busy}<span class="spin"></span> {busy}{:else}{t('common.install')}{/if}</button>
      <button onclick={onclose} disabled={!!busy}>{t('common.cancel')}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 6; background: rgb(0 0 0 / .45); display: grid; place-items: center; padding: 12px; overflow: auto; }
  .dlg { background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); box-shadow: var(--shadow); padding: 16px; width: min(560px, 100%); max-height: 90vh; overflow: auto; display: flex; flex-direction: column; gap: 10px; }
  .fld { display: flex; flex-direction: column; gap: 4px; }
  .fld input { width: 100%; }
  .small { font-size: .8rem; }
  h3, p { margin: 0; }
</style>

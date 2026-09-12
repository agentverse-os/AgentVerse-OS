<script lang="ts">
  // «Пароли и доступы»: все данные для входа установленных приложений в одном месте (docs/userflow-apps.md, шаг 5).
  import { api, type AppView } from './api';
  import { wm } from './windows.svelte';
  import { iconOf, isImg, titleOf } from './store-utils';
  import { accessOf } from './access';
  import AccessPanel from './AccessPanel.svelte';
  import { t, fmt } from './i18n.svelte';
  let apps = $state<AppView[]>([]);
  let q = $state('');
  let error = $state('');
  let copied = $state('');
  async function load() { try { apps = (await api.apps()).filter(a => a.installed); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } }
  $effect(() => { load(); });
  const shown = $derived(apps.filter(a => !q.trim() || titleOf(a).toLowerCase().includes(q.trim().toLowerCase()) || a.name.includes(q.trim().toLowerCase())));
  let status = $state<import('./api').SystemStatus | null>(null);
  let coderPw = $state<string | null>(null);
  $effect(() => { api.status().then(s => status = s).catch(() => {}); });
  const MODE_KEYS: Record<string, string> = { generated: 'vault.modeGenerated', default: 'vault.modeDefault', app: 'vault.modeApp', none: 'vault.modeNone', external: 'vault.modeExternal' };
  const modeShort = (m: string) => (MODE_KEYS[m] ? t(MODE_KEYS[m]) : m);
  async function exportAll() {
    if (!confirm(t('vault.exportConfirm'))) return;
    const lines: string[] = [t('vault.exportHeader', { date: fmt.dateTime(new Date()) }), ''];
    for (const a of shown) {
      const acc = accessOf(a);
      lines.push(`## ${titleOf(a)} (${a.name})`);
      if (acc.url) lines.push(`${t('vault.url')}: ${acc.url}`);
      for (const f of [acc.user, acc.password]) {
        if (!f) continue;
        let v = f.value ?? '';
        if (f.secret && f.env) { try { v = (await api.reveal(a.name, f.env)).value; } catch { v = '?'; } }
        lines.push(`${f.label}: ${v}`);
      }
      if (a.has_note) { try { lines.push(`${t('vault.note')}: ${(await api.note(a.name)).text}`); } catch { /* пропускаем */ } }
      lines.push('');
    }
    try { await navigator.clipboard?.writeText(lines.join('\n')); copied = t('vault.copiedAll'); } catch (e) { error = String(e); }
    setTimeout(() => copied = '', 2500);
  }
</script>

<div class="head row" style="justify-content: space-between; align-items: flex-start;">
  <div><h2>{t('sys.vault')}</h2><div class="muted small">{t('vault.subtitle')}</div></div>
  <div class="row"><input class="search" placeholder={t('vault.searchPlaceholder')} bind:value={q} /><button onclick={exportAll} disabled={!shown.length}>{t('vault.exportText')}</button>{#if copied}<span class="muted small">{copied}</span>{/if}</div>
</div>
{#if error}<div class="card" style="border-color: var(--bad); margin-bottom: 10px;">{error}</div>{/if}
{#if status?.coder_user && (!q.trim() || 'coder'.includes(q.trim().toLowerCase()))}
  <section class="card sys">
    <div class="row" style="justify-content: space-between;"><div class="row"><span class="ic emoji">🧩</span><b>Coder</b><span class="badge">{t('vault.coderBadge')}</span></div><a href={status.coder_url} target="_blank" rel="noopener"><button>{t('vault.openExt')}</button></a></div>
    <div class="row"><span class="k">{t('vault.url')}</span><a class="v" href={status.coder_url} target="_blank" rel="noopener">{status.coder_url.replace(/^https?:\/\//, '')}</a></div>
    <div class="row"><span class="k">{t('vault.login')}</span><code class="v">{status.coder_user}</code><button title={t('vault.copyTitle')} onclick={() => navigator.clipboard?.writeText(status?.coder_user ?? '')}>⧉</button></div>
    <div class="row"><span class="k">{t('vault.password')}</span><code class="v">{coderPw ?? '••••••••'}</code>
      {#if status.coder_password_set}<button title={coderPw ? t('vault.hide') : t('vault.show')} onclick={async () => { if (coderPw) { coderPw = null; return; } try { coderPw = (await api.coderCredentials()).password; } catch (e) { error = String(e); } }}>{coderPw ? '🙈' : '👁'}</button><button title={t('vault.copyTitle')} onclick={async () => { try { await navigator.clipboard?.writeText((await api.coderCredentials()).password); copied = t('vault.coderPwCopied'); setTimeout(() => copied = '', 1500); } catch (e) { error = String(e); } }}>⧉</button>{:else}<span class="muted small">{t('vault.coderPwUnknown')}</span>{/if}</div>
    <div class="muted small">{t('vault.coderHint')}</div>
  </section>
{/if}
<div class="list">
  {#each shown as a (a.name)}
    {@const ic = iconOf(a)}
    <section class="card vrow">
      <div class="row" style="justify-content: space-between;">
        <div class="row">{#if isImg(ic)}<img class="ic" src={ic} alt="" />{:else}<span class="ic emoji">{ic ?? '📦'}</span>{/if}<b>{titleOf(a)}</b><span class="badge">{modeShort(accessOf(a).mode)}</span>{#if a.has_note}<span class="badge">📝</span>{/if}</div>
        <button onclick={() => wm.openSys('appdetail', { name: a.name }, titleOf(a))}>{t('vault.card')}</button>
      </div>
      <AccessPanel app={a} compact manage onchange={(v) => { apps = apps.map(x => x.name === v.name ? v : x); }} />
    </section>
  {/each}
  {#if !shown.length && !error}<div class="card muted">{t('vault.notFound')}</div>{/if}
</div>

<style>
  .head { margin-bottom: 10px; }
  h2 { margin: 0; }
  .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(380px, 1fr)); gap: 10px; }
  .vrow { display: flex; flex-direction: column; gap: 8px; }
  .sys { display: flex; flex-direction: column; gap: 8px; margin-bottom: 10px; border-color: color-mix(in srgb, var(--accent) 40%, var(--line)); }
  .k { color: var(--muted); min-width: 52px; font-size: .85rem; }
  .v { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .95rem; background: var(--panel-2); padding: 3px 8px; border-radius: var(--r-sm); word-break: break-all; }
  .ic { width: 28px; height: 28px; border-radius: 8px; object-fit: cover; }
  .ic.emoji { font-size: 22px; display: grid; place-items: center; }
  .small { font-size: .8rem; }
  .search { min-width: 200px; }
</style>

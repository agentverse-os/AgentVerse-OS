<script lang="ts">
  import type { BackupStatus, BackupConfig } from './api';
  let bk = $state<BackupStatus | null>(null);
  let bkBusy = $state('');
  let bkMsg = $state('');
  async function loadBk() { try { bk = await api.backups(); } catch (e) { bkMsg = String(e); } }
  async function saveBk(c: BackupConfig) { try { await api.setBackups(c); bkMsg = t('common.saved'); await loadBk(); } catch (e) { bkMsg = String(e); } }
  async function runBk(label: string, f: () => Promise<unknown>) { bkBusy = label; bkMsg = ''; try { const r = await f() as Record<string, unknown>; bkMsg = r?.restic_error ? t('settings.bk.snapshotRestic', { error: String(r.restic_error) }) : t('common.ready'); } catch (e) { bkMsg = String(e); } finally { bkBusy = ''; await loadBk(); } }
  $effect(() => { loadBk(); });
  let coderPw = $state<string | null>(null);
  import { wm } from './windows.svelte';
  import { ACCENTS, BACKGROUNDS, DEFAULTS, THEMES, themeMatches, apply, saveLocal, saveRemote, type Appearance } from './theme';
  import { api, type SystemStatus } from './api';
  import Dropdown from './Dropdown.svelte';
  import { t, tn, fmt, LANGS, LANG_NAMES } from './i18n.svelte';
  let { appearance = $bindable() }: { appearance: Appearance } = $props();
  let status = $state<SystemStatus | null>(null);
  let saved = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => { api.status().then(s => status = s).catch(() => {}); });
  // любое изменение — применить сразу, сохранить локально и в ядре (с задержкой, чтобы не спамить при движении ползунка)
  $effect(() => {
    const a = $state.snapshot(appearance);
    apply(a); saveLocal(a);
    clearTimeout(timer); timer = setTimeout(async () => { await saveRemote(a); saved = t('common.saved') + ' ' + fmt.time(new Date(), { hour: '2-digit', minute: '2-digit', second: '2-digit' }); }, 500);
  });
  function reset() { appearance = { ...DEFAULTS }; }
  function set<K extends keyof Appearance>(k: K, v: Appearance[K]) { appearance = { ...appearance, [k]: v }; }
  function applyTheme(t: (typeof THEMES)[number]) { appearance = { ...appearance, ...t.set }; }
  const previewBg = (t: (typeof THEMES)[number]) => { const b = BACKGROUNDS.find(x => x.id === t.set.background); const m = t.set.mode === 'light' ? 'light' : 'dark'; return b ? (b.id.startsWith('monolith') ? (m === 'light' ? '#dfe6ff' : '#0f1424') : b[m]) : (m === 'light' ? '#f4f6f7' : '#121416'); };
</script>

<h2>{t('sys.settings')}</h2>
<div class="settings-grid">
  <section class="card themes">
    <h3>{t('settings.themes')}</h3>
    <div class="muted small" style="margin-bottom: 8px;">{t('settings.themesHint')}</div>
    <div class="thm">
      {#each THEMES as th (th.id)}
        <button class="thm-card" class:active={themeMatches(appearance, th)} onclick={() => applyTheme(th)} title={t(th.hint)}>
          <span class="thm-prev" style="background:{previewBg(th)}; color:{th.set.mode === 'light' ? '#14181b' : '#f2f4f4'}">
            <span class="thm-panel" style="background:{th.set.surface === 'glass' ? (th.set.mode === 'light' ? 'rgba(255,255,255,.55)' : 'rgba(255,255,255,.12)') : th.set.surface === 'oled' ? '#0a0a0d' : (th.set.mode === 'light' ? '#fff' : '#171c1f')}; border-color:{th.set.contrast === 'high' ? (th.set.mode === 'light' ? '#000' : '#fff') : 'rgba(255,255,255,.18)'}; border-radius:{th.set.radius === 'sharp' ? '2px' : th.set.radius === 'round' ? '10px' : '6px'}">
              <span class="thm-dot" style="background:{th.set.accent}"></span><span class="thm-line" style="background:currentColor; opacity:.7"></span><span class="thm-line" style="background:currentColor; opacity:.35; width:60%"></span>
            </span>
          </span>
          <span class="thm-name">{t(th.label)}</span>
          <span class="muted small">{t(th.hint)}</span>
        </button>
      {/each}
    </div>
  </section>

  <section class="card">
    <h3>{t('settings.theme')}</h3>
    <div class="row seg">
      {#each [['dark', t('settings.mode.dark')], ['light', t('settings.mode.light')], ['system', t('settings.mode.system')]] as [v, l]}
        <button class:primary={appearance.mode === v} onclick={() => set('mode', v as Appearance['mode'])}>{l}</button>
      {/each}
    </div>
    <div class="muted" style="margin-top:6px;">{t('settings.mode.systemHint')}</div>

    <h3 style="margin-top: 14px;">{t('settings.accent')}</h3>
    <div class="row">
      {#each ACCENTS as c}
        <button class="swatch" class:active={appearance.accent.toLowerCase() === c.toLowerCase()} style="background:{c}" title={c} aria-label={c} onclick={() => set('accent', c)}></button>
      {/each}
      <label class="muted">{t('settings.accentCustom')} <input type="color" value={appearance.accent} oninput={(e) => set('accent', (e.currentTarget as HTMLInputElement).value)} /></label>
    </div>

    <h3 style="margin-top: 14px;">{t('settings.contrast')}</h3>
    <div class="row seg">
      <button class:primary={appearance.contrast === 'normal'} onclick={() => set('contrast', 'normal')}>{t('settings.contrast.normal')}</button>
      <button class:primary={appearance.contrast === 'high'} onclick={() => set('contrast', 'high')}>{t('settings.contrast.high')}</button>
    </div>
  </section>

  <section class="card">
    <h3>{t('lang.title')}</h3>
    <div class="row seg">
      <button class:primary={(appearance.language ?? 'auto') === 'auto'} data-lang="auto" onclick={() => set('language', 'auto')}>{t('lang.auto')}</button>
      {#each LANGS as l}<button class:primary={appearance.language === l} data-lang={l} lang={l} onclick={() => set('language', l)}>{LANG_NAMES[l]}</button>{/each}
    </div>
    <div class="muted" style="margin-top:6px;">{t('lang.hint')}</div>
  </section>

  <section class="card">
    <h3>{t('settings.background')}</h3>
    <div class="bgs">
      {#each BACKGROUNDS as b}
        <button class="bg" class:active={appearance.background === b.id} onclick={() => set('background', b.id)}>
          <span class="bg-preview" style="background:{b.id === 'custom' ? 'repeating-linear-gradient(45deg, var(--panel-2) 0 6px, transparent 6px 12px)' : (b.top ? b.top[appearance.mode === 'light' ? 'light' : 'dark'] + ', ' : '') + b[appearance.mode === 'light' ? 'light' : 'dark']}"></span>
          <span>{t(b.label)}</span>
        </button>
      {/each}
    </div>
    {#if appearance.background === 'custom'}
      <input style="width:100%; margin-top: 8px;" placeholder={t('settings.bgUrlPlaceholder')} value={appearance.backgroundUrl} oninput={(e) => set('backgroundUrl', (e.currentTarget as HTMLInputElement).value)} />
    {/if}
    <label class="range"><span>{t('settings.blur')} <b>{appearance.backgroundBlur}px</b>{#if BACKGROUNDS.find(b => b.id === appearance.background)?.top} <span class="muted small">{t('settings.blurMarkHint')}</span>{/if}</span><input type="range" min="0" max="20" step="1" value={appearance.backgroundBlur} oninput={(e) => set('backgroundBlur', +(e.currentTarget as HTMLInputElement).value)} /></label>
    <label class="check"><input type="checkbox" checked={appearance.transparency} onchange={(e) => set('transparency', (e.currentTarget as HTMLInputElement).checked)} /> {t('settings.transparency')}</label>
  </section>

  <section class="card">
    <h3>{t('settings.text')}</h3>
    <label class="range"><span>{t('settings.textScale')} <b>{Math.round(appearance.textScale * 100)}%</b></span><input type="range" min="0.85" max="1.3" step="0.05" value={appearance.textScale} oninput={(e) => set('textScale', +(e.currentTarget as HTMLInputElement).value)} /></label>
    <div class="muted">{t('settings.density')}</div>
    <div class="row seg">
      {#each [['compact', t('settings.density.compact')], ['normal', t('settings.density.normal')], ['comfortable', t('settings.density.comfortable')]] as [v, l]}
        <button class:primary={appearance.density === v} onclick={() => set('density', v as Appearance['density'])}>{l}</button>
      {/each}
    </div>
    <div class="muted" style="margin-top:8px;">{t('settings.font')}</div>
    <Dropdown value={appearance.font} ariaLabel={t('settings.fontAria')} onchange={(v) => set('font', v as Appearance['font'])} options={[{ value: 'system', label: t('settings.font.system') }, { value: 'humanist', label: t('settings.font.humanist') }, { value: 'grotesk', label: t('settings.font.grotesk') }, { value: 'mono', label: t('settings.font.mono') }]} style="width: 100%" />
  </section>

  <section class="card">
    <h3>{t('settings.shape')}</h3>
    <div class="muted">{t('settings.saver')}</div>
    <div class="row" style="margin-bottom: 8px;"><Dropdown value={String(appearance.screensaver ?? 10)} ariaLabel={t('settings.saverAria')} onchange={(v) => set('screensaver', Number(v) as Appearance['screensaver'])} options={[{ value: '0', label: t('settings.saver.off') }, ...[1, 5, 10, 30].map(n => ({ value: String(n), label: tn('settings.saver.after', n) }))]} style="min-width: 14em" /><span class="muted small">{t('settings.saverHint')}</span></div>
    <div class="muted">{t('settings.radius')}</div>
    <div class="row seg">
      {#each [['sharp', t('settings.radius.sharp')], ['normal', t('settings.radius.normal')], ['round', t('settings.radius.round')]] as [v, l]}
        <button class:primary={appearance.radius === v} onclick={() => set('radius', v as Appearance['radius'])}>{l}</button>
      {/each}
    </div>
    <div class="muted" style="margin-top:8px;">{t('settings.iconSize')}</div>
    <div class="row seg">
      {#each [['small', t('settings.iconSize.small')], ['medium', t('settings.iconSize.medium')], ['large', t('settings.iconSize.large')]] as [v, l]}
        <button class:primary={appearance.iconSize === v} onclick={() => set('iconSize', v as Appearance['iconSize'])}>{l}</button>
      {/each}
    </div>
    <label class="check" style="margin-top:8px;"><input type="checkbox" checked={appearance.reduceMotion} onchange={(e) => set('reduceMotion', (e.currentTarget as HTMLInputElement).checked)} /> {t('settings.reduceMotion')}</label>
    <label class="check"><input type="checkbox" checked={appearance.showWorkspaceIps} onchange={(e) => set('showWorkspaceIps', (e.currentTarget as HTMLInputElement).checked)} /> {t('settings.showIps')}</label>
    <label class="check"><input type="checkbox" checked={!!appearance.taskbarPinned} onchange={(e) => set('taskbarPinned', (e.currentTarget as HTMLInputElement).checked)} /> {t('settings.taskbarPinned')} <span class="muted">{t('settings.taskbarPinnedHint')}</span></label>
  </section>

  <section class="card">
    <h3>{t('sys.vault')}</h3>
    <div class="muted">{t('settings.vaultHint')}</div>
    <div class="row" style="margin-top: 8px;"><button class="primary" onclick={() => wm.openSys('vault')}>{t('settings.vaultOpen')}</button></div>
  </section>
    <section class="card backups">
      <h3>{t('settings.bk.title')}</h3>
      {#if !bk}
        <div class="muted">{t('common.loading')}</div>
      {:else}
        <label class="row"><input type="checkbox" checked={bk.config.enabled} onchange={(e) => saveBk({ ...bk!.config, enabled: (e.currentTarget as HTMLInputElement).checked })} /> <b>{bk.config.restic ? t('settings.bk.scheduledRestic') : t('settings.bk.scheduled')}</b></label>
        <div class="muted small">{t('settings.bk.policy', { hourly: bk.config.hourly, daily: bk.config.daily, weekly: bk.config.weekly })} {bk.config.remote ? t('settings.bk.resticInRemote', { repo: bk.restic_repo, remote: bk.config.remote }) : t('settings.bk.resticIn', { repo: bk.restic_repo })}</div>
        <div class="row" style="flex-wrap: wrap; gap: 8px; margin-top: 8px;">
          <label class="muted small">{t('settings.bk.hourly')} <input type="number" min="0" max="168" style="width: 4.5em" value={bk.config.hourly} onchange={(e) => saveBk({ ...bk!.config, hourly: +(e.currentTarget as HTMLInputElement).value })} /></label>
          <label class="muted small">{t('settings.bk.daily')} <input type="number" min="0" max="60" style="width: 4.5em" value={bk.config.daily} onchange={(e) => saveBk({ ...bk!.config, daily: +(e.currentTarget as HTMLInputElement).value })} /></label>
          <label class="muted small">{t('settings.bk.weekly')} <input type="number" min="0" max="52" style="width: 4.5em" value={bk.config.weekly} onchange={(e) => saveBk({ ...bk!.config, weekly: +(e.currentTarget as HTMLInputElement).value })} /></label>
          <label class="muted small"><input type="checkbox" checked={bk.config.restic} onchange={(e) => saveBk({ ...bk!.config, restic: (e.currentTarget as HTMLInputElement).checked })} /> {t('settings.bk.resticNightly')}</label>
          <label class="muted small">{t('settings.bk.remote')} <input placeholder={t('settings.bk.remotePlaceholder')} style="width: 18em" value={bk.config.remote} onchange={(e) => saveBk({ ...bk!.config, remote: (e.currentTarget as HTMLInputElement).value.trim() })} /></label>
        </div>
        <table class="bktab">
          <thead><tr><th>{t('settings.bk.colDataset')}</th><th>{t('settings.bk.colData')}</th><th>{t('settings.bk.colSnapshots')}</th><th>{t('settings.bk.colSnapUsed')}</th><th>{t('settings.bk.colLast')}</th></tr></thead>
          <tbody>{#each bk.datasets as d}<tr><td><code>{d.name}</code> <span class="muted small">{d.mountpoint}</span></td><td>{fmt.bytes(d.used)}</td><td>{d.snapshots}</td><td>{fmt.bytes(d.snapshots_used)}</td><td>{d.last ? fmt.dateTime(d.last * 1000) : '—'}</td></tr>{/each}</tbody>
        </table>
        <div class="muted small" style="margin-top: 6px;">
          restic: {#if !bk.restic_installed}<b>{t('settings.bk.resticMissing')}</b> (apt install restic){:else}{tn('settings.bk.resticCopies', bk.restic_snapshots, { size: fmt.bytes(bk.restic_size) })}{/if}
          {#if bk.last?.restic}· {t('settings.bk.lastRestic', { at: bk.last.restic.at ? fmt.dateTime(bk.last.restic.at) : '' })} {bk.last.restic.ok === false ? t('settings.bk.resticError', { error: bk.last.restic.error ?? '' }) : ''}{/if}
          {#if bk.last?.snapshot}· {t('settings.bk.lastSnapshot', { at: fmt.dateTime(bk.last.snapshot.at), kind: bk.last.snapshot.kind })}{/if}
        </div>
        <div class="row" style="margin-top: 8px;">
          <button disabled={bkBusy !== '' || bk.running} onclick={() => runBk('snapshot', () => api.backupSnapshot())}>{bkBusy === 'snapshot' ? t('settings.bk.snapshotBusy') : t('settings.bk.snapshotNow')}</button>
          <button disabled={bkBusy !== '' || bk.running || !bk.restic_installed} onclick={() => runBk('restic', () => api.backupRun())}>{bkBusy === 'restic' ? t('settings.bk.resticBusy') : t('settings.bk.resticNow')}</button>
          {#if bk.running}<span class="muted small">{t('settings.bk.running')}</span>{/if}
          {#if bkMsg}<span class="muted small">{bkMsg}</span>{/if}
        </div>
        <div class="muted small" style="margin-top: 6px;">{t('settings.bk.note')}</div>
      {/if}
    </section>

  <section class="card">
    <h3>{t('sys.system')}</h3>
    <div class="muted">AgentVerse OS {status?.version ?? '…'}{#if status?.build && status.build !== 'dev'}{' · '}{t('settings.sys.build', { build: status.build })}{/if}
      {#if status?.update_available}<span class="badge ok" style="margin-left: 6px;">{t('settings.sys.updateAvailable')}</span>{/if}
      {#if status?.app_updates}<span class="badge" style="margin-left: 6px;">{t('settings.sys.appUpdates', { n: status.app_updates })}</span>{/if}</div>
    <div class="row" style="margin-top: 8px; gap: 8px; flex-wrap: wrap;">
      <button onclick={() => wm.openSys('updates')}>⬆️ {t('sys.updates')}</button>
      <button onclick={() => wm.openSys('setup')}>🧭 {t('settings.sys.wizard')}</button>
      <span class="muted small">{t('settings.sys.hint')}</span>
    </div>
  </section>

  <section class="card">
    <h3>{t('settings.devices')}</h3>
    {#if status}
      <div class="muted">Entry Point <code>https://{status.edge_host}</code>. {t('settings.dev.caIntro')} <a href="/api/ca.crt">{t('settings.dev.caDownload')}</a> {t('settings.dev.caTrust')}</div>
      <div class="muted" style="margin-top: 8px;"><b>Windows (Chrome, Edge)</b>: {t('settings.dev.win')} <code>certutil -addstore -f Root cloudos-ca.crt</code>. {t('settings.dev.winRestart')} <b>macOS</b>: {t('settings.dev.mac')} <b>iPhone</b>: {t('settings.dev.iphone')} <b>Android</b>: {t('settings.dev.android')}</div>
      <div class="muted">Coder: <a href={status.coder_url} target="_blank" rel="noopener">{status.coder_url}</a>{#if status.coder_user} · {t('settings.dev.login')} <code>{status.coder_user}</code>{/if}
        {#if status.coder_password_set}· {t('settings.dev.password')} <code>{coderPw ?? '••••••••'}</code> <button title={coderPw ? t('settings.dev.hide') : t('settings.dev.show')} onclick={async () => { if (coderPw) { coderPw = null; return; } try { coderPw = (await api.coderCredentials()).password; } catch (e) { alert(String(e)); } }}>{coderPw ? '🙈' : '👁'}</button> <button title={t('settings.dev.copyPassword')} onclick={async () => { try { navigator.clipboard?.writeText((await api.coderCredentials()).password); } catch (e) { alert(String(e)); } }}>⧉</button>
        {:else}· <span title={t('settings.dev.pwUnknownTitle')}>{t('settings.dev.pwUnknown')}</span>{/if}
        <span class="muted small"> {t('settings.dev.coderHint')}</span></div>
      <div class="muted" style="margin-top: 8px;"><b>Firefox</b> {t('settings.dev.ff1')} <a href="/api/ca.crt">cloudos-ca.crt</a> {t('settings.dev.ff2')} <code>security.enterprise_roots.enabled</code> {t('settings.dev.ff3')}</div>
      <div class="muted" style="margin-top: 8px;"><b>{t('settings.dev.noOpen')} <code>{status.edge_host}</code></b> {t('settings.dev.noOpenText')}
        {#if status.edge_alt_hosts?.length}{t('settings.dev.altHosts')} {#each status.edge_alt_hosts as h}<a href="https://{h}/">https://{h}/</a> {/each}{/if}</div>
    {/if}
    <div class="muted" style="margin-top: 8px;">{t('settings.dev.stored')}</div>
  </section>

  <section class="card">
    <h3>{t('settings.reset')}</h3>
    <div class="row"><button class="danger" onclick={reset}>{t('settings.resetBtn')}</button><span class="muted">{saved}</span></div>
  </section>
</div>

<style>
  .themes { grid-column: 1 / -1; }
  .thm { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 10px; }
  .thm-card { display: flex; flex-direction: column; align-items: stretch; gap: 6px; padding: 8px; text-align: left; background: var(--panel-2); border: 1px solid var(--line); border-radius: var(--r); }
  .thm-card.active { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent) inset; }
  .thm-prev { display: flex; align-items: flex-end; justify-content: flex-start; height: 72px; border-radius: 8px; padding: 8px; }
  .thm-panel { display: flex; flex-direction: column; gap: 4px; width: 68%; padding: 7px 8px; border: 1px solid; }
  .thm-dot { width: 10px; height: 10px; border-radius: 50%; }
  .thm-line { height: 3px; width: 90%; border-radius: 2px; }
  .thm-name { font-weight: 600; }

  .bktab { width: 100%; border-collapse: collapse; margin-top: 8px; font-size: .9rem; }
  .bktab th, .bktab td { text-align: left; padding: 4px 8px; border-bottom: 1px solid var(--line); }
  .bktab th { color: var(--muted); font-weight: 500; }

  .settings-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: calc(12px * var(--sp)); }
  .seg button { min-width: 6em; }
  .swatch { width: 28px; height: 28px; border-radius: 50%; border: 2px solid transparent; padding: 0; }
  .swatch.active { border-color: var(--ink); box-shadow: 0 0 0 2px var(--bg); }
  .bgs { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; }
  .bg { display: flex; flex-direction: column; gap: 4px; padding: 6px; font-size: .8rem; }
  .bg.active { border-color: var(--accent); }
  .bg-preview { display: block; height: 44px; border-radius: var(--r-sm); border: 1px solid var(--line); }
  .range { display: flex; flex-direction: column; gap: 4px; margin-top: 8px; font-size: .9rem; }
  .check { display: flex; gap: 8px; align-items: center; margin-top: 6px; font-size: .9rem; }
</style>

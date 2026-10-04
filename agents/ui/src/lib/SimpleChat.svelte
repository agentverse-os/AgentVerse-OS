<script lang="ts">
  // Простой чат (docs/agents-ux.md §6.1): сообщения с markdown и вложениями, вызовы инструментов свёрнутыми строками,
  // композер с моделью маршрута, переключателями поиска/страниц и скрепкой; ответ приходит потоком SSE (agents.chatSend).
  import { tick } from 'svelte';
  import { data, money, ago, msg, when, ui, phone } from './state.svelte';
  import { agents, byProvider, type SimpleChat, type ChatMessage, type ChatToolUse, type ChatEvent, type ChatPreset } from './api';
  import { menu } from './menu.svelte';
  import { toast } from './toast.svelte';
  import { t } from './i18n';
  import Icon from './Icon.svelte';
  import Md from './Md.svelte';
  let { id, onchange }: { id: string; onchange?: () => void } = $props();
  let chat = $state<SimpleChat | null>(null);
  let messages = $state<ChatMessage[]>([]);
  let text = $state('');
  let files = $state<{ name: string; mime: string; data: string; url: string }[]>([]);
  let working = $state(false);
  let live = $state<{ text: string; tools: ChatToolUse[]; running: string | null } | null>(null);
  let box = $state<HTMLElement | null>(null);
  let ta = $state<HTMLTextAreaElement | null>(null);
  let abort: AbortController | null = null;
  const route = $derived(data.chatRoutes?.routes.find(r => r.id === chat?.route) ?? null);
  const modelInfo = $derived(route?.models.find(m => m.id === chat?.model) ?? null);
  const searchReady = $derived(!!data.chatRoutes?.search.ready);
  async function load() {
    try { const r = await agents.chatGet(id); chat = r.chat; messages = r.messages; working = r.busy; await tick(); scroll(); } catch (e) { toast(msg(e), 'bad'); }
  }
  $effect(() => { id; live = null; text = ''; files = []; load(); if (!data.chatRoutes) data.loadChatRoutes(); });
  // переход к сообщению из поиска (срез 34): после загрузки — прокрутить и подсветить
  $effect(() => { const f = ui.focusMid; if (!f || f.chat !== id) return; const mid = f.mid; const h = setTimeout(() => { ui.focusMid = null; const el = box?.querySelector(`[data-mid="${mid}"]`) as HTMLElement | null; if (el) { el.scrollIntoView({ block: 'center' }); el.classList.add('hit'); setTimeout(() => el.classList.remove('hit'), 3000); } }, 700); return () => clearTimeout(h); });
  function scroll() { if (box) box.scrollTop = box.scrollHeight; }
  function autoGrow() { if (!ta) return; ta.style.height = 'auto'; ta.style.height = Math.min(180, ta.scrollHeight) + 'px'; }
  function onKey(e: KeyboardEvent) { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(); } }
  async function addFiles(list: FileList | File[]) {
    for (const f of Array.from(list)) {
      if (f.size > 8 * 1024 * 1024) { toast(t('chat.tooBig', { name: f.name }), 'bad'); continue; }
      const data64 = await new Promise<string>((res, rej) => { const r = new FileReader(); r.onload = () => res(String(r.result).split(',')[1] ?? ''); r.onerror = () => rej(r.error); r.readAsDataURL(f); });
      files.push({ name: f.name || 'image.png', mime: f.type || 'application/octet-stream', data: data64, url: f.type.startsWith('image/') ? URL.createObjectURL(f) : '' });
    }
  }
  function onPaste(e: ClipboardEvent) { const items = Array.from(e.clipboardData?.items ?? []).filter(i => i.kind === 'file'); if (!items.length) return; e.preventDefault(); addFiles(items.map(i => i.getAsFile()!).filter(Boolean)); }
  function onDrop(e: DragEvent) { e.preventDefault(); if (e.dataTransfer?.files?.length) addFiles(e.dataTransfer.files); }
  async function send() {
    if (!chat || working || (!text.trim() && !files.length)) return;
    const body = { text: text.trim(), attachments: files.map(f => ({ name: f.name, mime: f.mime, data: f.data })), tools: chat.tools };
    text = ''; files = []; await tick(); autoGrow();
    working = true; live = { text: '', tools: [], running: null }; abort = new AbortController();
    try {
      await agents.chatSend(chat.id, body, (ev: ChatEvent) => {
        if (ev.kind === 'user') { messages.push(ev.message); }
        else if (ev.kind === 'delta' && live) { live.text += ev.text; }
        else if (ev.kind === 'tool_start' && live) { live.running = ev.name; }
        else if (ev.kind === 'tool' && live) { live.tools.push({ name: ev.name, args: ev.args, result: ev.result, ok: ev.ok }); live.running = null; }
        else if (ev.kind === 'done' || ev.kind === 'error') { if (ev.message) messages.push(ev.message); if (ev.chat) chat = ev.chat; live = null; if (ev.kind === 'error') toast(ev.error, 'bad'); }
        scroll();
      }, abort.signal);
    } catch (e) { if (!(e instanceof DOMException && e.name === 'AbortError')) toast(msg(e), 'bad'); }
    finally { working = false; live = null; abort = null; await load(); onchange?.(); }
  }
  async function stop() { if (!chat) return; try { await agents.chatStop(chat.id); } catch (e) { toast(msg(e), 'bad'); } }
  function modelMenu(el: HTMLElement) {
    const rs = data.chatRoutes?.routes ?? [];
    if (!rs.length) { toast(t('chat.noRoutes'), 'bad'); return; }
    const win = (n?: number | null) => (n ? (n >= 1e6 ? `${Math.round(n / 1e5) / 10}M` : `${Math.round(n / 1000)}k`) : '');
    const items: Parameters<typeof menu.show>[1] = [];
    for (const r of rs) {
      items.push({ header: true, label: `${r.title}${r.error ? ' · ' + r.error : ''}` });
      // группы — каналы учёток gate (Claude Code, Codex, Antigravity…); одна модель может быть у нескольких каналов
      for (const g of byProvider(r.models)) {
        items.push({ sub: true, label: `${g.title} · ${g.models.length}` });
        for (const m of g.models) {
          const label = m.name && m.name !== m.id ? m.name : m.id;
          items.push({ icon: 'cpu', checked: r.id === chat?.route && m.id === chat?.model, label, hint: [label !== m.id ? m.id : '', win(m.context), m.price_in != null ? `$${m.price_in}/${m.price_out}` : ''].filter(Boolean).join(' · '), on: () => setModel(r.id, m.id) });
        }
      }
    }
    menu.show(el, items, { search: true, placeholder: t('chat.modelSearch') });
  }
  async function setModel(routeId: string, model: string) { if (!chat) return; try { chat = await agents.chatPatch(chat.id, { route: routeId, model }); onchange?.(); } catch (e) { toast(msg(e), 'bad'); } }
  async function toggleTool(k: 'search' | 'fetch') {
    if (!chat) return;
    if (k === 'search' && !searchReady && !chat.tools.search) { toast(t('chat.searchOff'), 'bad'); return; }
    try { chat = await agents.chatPatch(chat.id, { tools: { ...chat.tools, [k]: !chat.tools[k] } }); } catch (e) { toast(msg(e), 'bad'); }
  }
  // системный промпт и пресеты (срез 37, §6.5): попап вместо окна prompt()
  let sysOpen = $state(false); let sysText = $state(''); let presets = $state<ChatPreset[]>([]); let sysBusy = $state(false);
  async function openSystem() { if (!chat) return; sysText = chat.system; sysOpen = true; try { presets = (await agents.chatPresets()).presets; } catch {} }
  async function saveSystem() { if (!chat) return; sysBusy = true; try { chat = await agents.chatPatch(chat.id, { system: sysText.trim() }); toast(t('toast.saved')); sysOpen = false; } catch (e) { toast(msg(e), 'bad'); } finally { sysBusy = false; } }
  async function savePresets(list: ChatPreset[]) { try { presets = (await agents.chatPresetsSave(list)).presets; } catch (e) { toast(msg(e), 'bad'); } }
  function saveAsPreset() { const name = window.prompt(t('chat.presetName'), sysText.split('\n')[0].slice(0, 40)); if (!name || !sysText.trim()) return; savePresets([...presets, { id: '', name: name.trim(), text: sysText.trim() }]); }
  function imgPrefix() { if (!text.startsWith('/img ')) text = '/img ' + text; ta?.focus(); }
  const argText = (a: Record<string, unknown>) => Object.values(a).map(v => String(v)).join(' ').slice(0, 160);
  const toolLabel = (n: string) => (n === 'web_search' ? t('chat.toolSearch') : n === 'fetch_url' ? t('chat.toolFetch') : n);
</script>

<div class="chat" ondragover={(e) => e.preventDefault()} ondrop={onDrop} role="region" aria-label={t('chat.title')}>
  {#if chat?.incognito}<div class="incog"><Icon name="eyeOff" size="s" /> <span>{t('chat.incognitoBar')}{#if chat.expires_at} · {t('chat.expires', { t: when(chat.expires_at) })}{/if}</span></div>{/if}
  <div class="msgs" bind:this={box}>
    {#if !messages.length && !live}<div class="empty" style="padding: 40px 12px"><span class="ic"><Icon name="zap" size="l" /></span><b>{t('chat.emptySimple')}</b><span style="max-width: 460px">{t('chat.emptySimpleHint')}</span></div>{/if}
    {#each messages as m (m.id)}
      {#if m.role === 'user'}
        <div class="m user" data-mid={m.id}>{#if m.text}<div class="txt">{m.text}</div>{/if}
          {#if m.attachments.length}<div class="atts">{#each m.attachments as a (a.file)}{#if a.mime.startsWith('image/')}<a href={agents.chatFileUrl(chat!.id, a.file)} target="_blank" rel="noopener"><img src={agents.chatFileUrl(chat!.id, a.file)} alt={a.name} /></a>{:else}<a class="chip file" href={agents.chatFileUrl(chat!.id, a.file)} target="_blank" rel="noopener"><Icon name="clip" size="s" /> {a.name} · {Math.round(a.size / 1024)} KB{#if a.mime === 'application/pdf' || a.name.toLowerCase().endsWith('.pdf')} · {a.text ? t('chat.pdfText') : t('chat.pdfNoText')}{/if}</a>{/if}{/each}</div>{/if}</div>
      {:else}
        {#each m.tools as tu, i (m.id + i)}<details class="m tool" class:err={!tu.ok}><summary><b>{toolLabel(tu.name)}</b> <span class="muted mono">{argText(tu.args)}</span> <span class="badge {tu.ok ? 'ok' : 'bad'}">{tu.ok ? t('agents.toolDone') : t('agents.toolError')}</span></summary><pre class="mono res">{tu.result}</pre></details>{/each}
        {#if m.text}<div class="m agent" data-mid={m.id} class:wide={/(^|\n)\s*(\||```|#{1,4} |[-*] |\d+\. )/.test(m.text) || m.attachments?.length > 0}><Md text={m.text} />{#if m.attachments?.length}<div class="atts">{#each m.attachments as a (a.file)}{#if a.mime.startsWith('image/')}<a href={agents.chatFileUrl(chat!.id, a.file)} target="_blank" rel="noopener"><img src={agents.chatFileUrl(chat!.id, a.file)} alt={a.name} /></a>{/if}{/each}</div>{/if}<div class="meta muted xs">{m.tokens_in + m.tokens_out} {t('chat.tokens')} · {money(m.cost_usd)}{#if m.error} · <span class="bad">{m.error}</span>{/if}</div></div>
        {:else if m.error}<div class="m sys"><span class="bad">⚠ {m.error}</span></div>{/if}
      {/if}
    {/each}
    {#if live}
      {#each live.tools as tu, i (i)}<details class="m tool" class:err={!tu.ok}><summary><b>{toolLabel(tu.name)}</b> <span class="muted mono">{argText(tu.args)}</span> <span class="badge {tu.ok ? 'ok' : 'bad'}">{tu.ok ? t('agents.toolDone') : t('agents.toolError')}</span></summary><pre class="mono res">{tu.result}</pre></details>{/each}
      {#if live.running}<div class="m sys"><span class="spin"></span> {toolLabel(live.running)} · {t('chat.toolRunning')}</div>{/if}
      {#if live.text}<div class="m agent live"><Md text={live.text} live /></div>{:else if !live.running}<div class="m sys"><span class="spin"></span> {t('chat.thinkingDots')}</div>{/if}
    {/if}
  </div>
  <div class="composer" class:busy={working}>
    {#if files.length}<div class="thumbs">{#each files as f, i (i)}<span class="thumb">{#if f.url}<img src={f.url} alt={f.name} />{:else}<span class="chip"><Icon name="clip" size="s" /> {f.name}</span>{/if}<button class="x" onclick={() => files.splice(i, 1)}>×</button></span>{/each}</div>{/if}
    <div class="in">
      <textarea bind:this={ta} bind:value={text} rows="1" placeholder={t('chat.placeholder')} onkeydown={onKey} oninput={autoGrow} onpaste={onPaste}></textarea>
      <label class="icon ghost" title={t('chat.attach')}><Icon name="clip" /><input type="file" multiple hidden onchange={(e) => { const i = e.currentTarget as HTMLInputElement; if (i.files) addFiles(i.files); i.value = ''; }} /></label>
      {#if phone()}<label class="icon ghost cam" title={t('chat.camera')}><Icon name="camera" /><input type="file" accept="image/*" capture="environment" hidden onchange={(e) => { const i = e.currentTarget as HTMLInputElement; if (i.files) addFiles(i.files); i.value = ''; }} /></label>{/if}
      {#if working}<button class="sm danger" onclick={stop}><Icon name="square" size="s" /> {t('chat.stop')}</button>{:else}<button class="sm primary send" disabled={!text.trim() && !files.length} onclick={send}><Icon name="send" size="s" /> {t('chat.send')}</button>{/if}
    </div>
    {#if sysOpen}
      <div class="modes syspop" role="dialog" aria-label={t('chat.system')} onclick={(e) => e.stopPropagation()}>
        <div class="mh"><span>{t('chat.system')}</span><span class="sp"></span><button class="icon xs ghost" onclick={() => (sysOpen = false)}><Icon name="x" size="s" /></button></div>
        <textarea class="sys" rows="4" bind:value={sysText} placeholder={t('chat.systemHint')}></textarea>
        <div class="ph muted xs">{t('chat.presets')}</div>
        {#each presets as p (p.id || p.name)}
          <div class="preset" data-preset={p.id}><span class="mi-t"><b>{p.name}</b><span class="muted">{p.text.slice(0, 90)}</span></span><button class="xs ghost apply" onclick={() => (sysText = p.text)}>{t('chat.presetApply')}</button><button class="icon xs ghost" title={t('chat.presetDelete')} onclick={() => savePresets(presets.filter((x) => x !== p))}><Icon name="x" size="s" /></button></div>
        {/each}
        <div class="acts"><button class="xs ghost saveas" disabled={!sysText.trim()} onclick={saveAsPreset}><Icon name="plus" size="s" /> {t('chat.presetSaveAs')}</button><span class="sp"></span><button class="sm primary save" disabled={sysBusy} onclick={saveSystem}>{t('common.save')}</button></div>
      </div>
    {/if}
    <div class="tools">
      <button class="ctl mono" title={t('chat.model')} onclick={(e) => { e.stopPropagation(); modelMenu(e.currentTarget as HTMLElement); }}><Icon name="cpu" size="s" /> {chat?.model || t('chat.model')}{#if modelInfo?.context} · {Math.round(modelInfo.context / 1000)}k{/if}</button>
      <span class="muted hide-m" title={t('chat.route')}>{route?.title ?? chat?.route ?? ''}</span>
      <button class="ctl tsearch" class:on={chat?.tools.search} class:off={!searchReady} title={searchReady ? t('chat.toolSearch') : t('chat.searchOff')} onclick={() => toggleTool('search')}><Icon name="search" size="s" /> {t('chat.toolSearch')}</button>
      <button class="ctl tfetch" class:on={chat?.tools.fetch} title={t('chat.toolFetch')} onclick={() => toggleTool('fetch')}><Icon name="globe" size="s" /> {t('chat.toolFetch')}</button>
      <button class="ctl tsys" class:on={!!chat?.system} title={t('chat.systemHint')} onclick={(e) => { e.stopPropagation(); if (sysOpen) sysOpen = false; else openSystem(); }}><Icon name="edit" size="s" /> <span class="hide-m">{t('chat.system')}</span></button>
      <button class="ctl timg" title={t('chat.imgHint')} onclick={imgPrefix}><Icon name="image" size="s" /> <span class="hide-m">{t('chat.img')}</span></button>
      <span class="sp"></span>
      {#if chat}<span class="muted num">{money(chat.cost_usd)} · {chat.tokens_in + chat.tokens_out} {t('chat.tokens')}</span>{/if}
    </div>
  </div>
</div>

<svelte:window onclick={() => (sysOpen = false)} onkeydown={(e) => { if (e.key === 'Escape') sysOpen = false; }} />

<style>
  .chat { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; }
  .incog { display: flex; align-items: center; gap: 8px; padding: 6px 12px; border-radius: var(--r-2); background: rgb(var(--accent-rgb) / .08); border: 1px dashed rgb(var(--accent-rgb) / .4); font-size: 12px; color: var(--fg-2); }
  .msgs { flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column; gap: 8px; padding: 4px 2px; }
  .m { max-width: 76%; border-radius: var(--r-3); padding: 9px 13px; line-height: 1.5; font-size: 13.5px; background: var(--bg-3); border: 1px solid var(--line-1); }
  .m.user { align-self: flex-end; background: var(--accent-soft); border-color: rgb(var(--accent-rgb) / .25); border-bottom-right-radius: 4px; }
  .m.agent { align-self: flex-start; border-bottom-left-radius: 4px; }
  .m.agent.wide { max-width: 100%; }
  .m.agent .meta { margin-top: 6px; }
  .m.sys { align-self: center; background: transparent; border: 0; font-size: 11.5px; color: var(--fg-3); padding: 0 8px; display: flex; align-items: center; gap: 6px; }
  .m.tool { align-self: stretch; max-width: 100%; width: 100%; padding: 0; background: var(--bg-4); font-size: 12.5px; }
  .m.tool summary { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 4px 10px; cursor: pointer; list-style: none; color: var(--fg-2); }
  .m.tool summary::-webkit-details-marker { display: none; }
  .m.tool summary b { color: var(--fg-1); font-weight: 500; }
  .m.tool summary .mono { font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .m.tool.err { border-color: rgb(242 109 109 / .4); }
  .m.tool pre.res { margin: 0; border: 0; border-top: 1px solid var(--line-1); border-radius: 0; max-height: 260px; overflow: auto; white-space: pre-wrap; font-size: 12px; padding: 8px 10px; }
  .txt { white-space: pre-wrap; overflow-wrap: anywhere; }
  .atts { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 6px; }
  .atts img { max-width: 220px; max-height: 180px; border-radius: 8px; display: block; }
  .bad { color: var(--danger, #f26d6d); }
  .composer { border: 1px solid var(--line-2); border-radius: var(--r-3); background: var(--bg-4); padding: 6px 8px; display: flex; flex-direction: column; gap: 4px; position: relative; }
  .composer:focus-within { border-color: rgb(var(--accent-rgb) / .6); box-shadow: 0 0 0 3px rgb(var(--accent-rgb) / .14); }
  .composer.busy { border-color: rgb(var(--accent-rgb) / .35); }
  .in { display: flex; align-items: flex-end; gap: 6px; }
  .in textarea { flex: 1; border: 0; background: transparent; padding: 6px; min-height: 32px; max-height: 180px; resize: none; line-height: 1.45; }
  .in textarea:focus { box-shadow: none; }
  .in label.icon { display: inline-flex; align-items: center; justify-content: center; width: 30px; height: 30px; border-radius: 8px; cursor: pointer; color: var(--fg-2); }
  .in label.icon:hover { background: var(--bg-3); }
  .thumbs { display: flex; gap: 6px; flex-wrap: wrap; padding: 2px 4px; }
  .thumb { position: relative; display: inline-flex; align-items: center; }
  .thumb img { height: 56px; border-radius: 6px; }
  .thumb .x { position: absolute; top: -6px; right: -6px; width: 18px; height: 18px; border-radius: 50%; padding: 0; font-size: 12px; line-height: 1; }
  .tools { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 12px; color: var(--fg-3); padding: 0 2px; }
  .syspop { position: absolute; left: 8px; right: 8px; bottom: calc(100% + 6px); max-height: min(60vh, 520px); overflow: auto; z-index: 30; background: var(--bg-3); border: 1px solid var(--line-2); border-radius: var(--r-2); box-shadow: var(--shadow-3); padding: 8px 10px; display: flex; flex-direction: column; gap: 4px; }
  .syspop .mh { display: flex; align-items: center; gap: 8px; font-weight: 600; font-size: 12.5px; padding-bottom: 4px; }
  .syspop .sp { flex: 1; }
  .syspop textarea.sys { width: 100%; min-height: 84px; font-size: 13px; resize: vertical; }
  .syspop .ph { padding: 6px 2px 2px; }
  .syspop .preset { display: flex; align-items: center; gap: 8px; padding: 4px 2px; border-top: 1px solid var(--line-1); }
  .syspop .preset .mi-t { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 12.5px; }
  .syspop .preset .mi-t .muted { font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .syspop .acts { display: flex; align-items: center; gap: 8px; padding-top: 8px; }
  .atts .chip.file { text-decoration: none; color: inherit; }
  .tools .ctl.tsys.on { color: var(--accent); }
  .ctl { display: inline-flex; align-items: center; gap: 5px; height: 24px; padding: 0 8px; border-radius: 999px; border: 1px solid var(--line-2); background: transparent; color: var(--fg-2); font-size: 12px; cursor: pointer; }
  .ctl:hover { background: var(--bg-3); }
  .ctl.on { border-color: rgb(var(--accent-rgb) / .6); color: var(--accent); background: rgb(var(--accent-rgb) / .1); }
  .ctl.off { opacity: .55; }
  .spin { width: 10px; height: 10px; border: 2px solid var(--line-3); border-top-color: var(--accent); border-radius: 50%; display: inline-block; animation: sp 1s linear infinite; }
  @keyframes sp { to { transform: rotate(360deg); } }
  @media (max-width: 720px) { .m { max-width: 92%; } }
</style>

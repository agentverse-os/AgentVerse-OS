<script module lang="ts">
  type Attachment = { name: string; mime: string; data: string; url: string };
  // Keep unsent drafts when switching chats; incognito drafts stay in memory too.
  const drafts = new Map<string, { text: string; files: Attachment[] }>();
  function saveDraft(id: string, text: string, files: Attachment[]) {
    if (text || files.length) drafts.set(id, { text, files: [...files] });
    else drafts.delete(id);
  }
</script>

<script lang="ts">
  // Simple chat (docs/agents-ux.md §6.1): messages with markdown and attachments, tool calls as folded lines, a composer
  // with the route model, search/pages toggles and a paperclip; the answer arrives as an SSE stream (agents.chatSend).
  import { tick, untrack, onDestroy } from 'svelte';
  import { data, money, msg, when, ui, phone, touch } from './state.svelte';
  import { agents, byProvider, type SimpleChat, type ChatMessage, type ChatToolUse, type ChatEvent, type ChatPreset } from './api';
  import { menu } from './menu.svelte';
  import { toast } from './toast.svelte';
  import { t } from './i18n';
  import Icon from './Icon.svelte';
  import Md from './Md.svelte';
  import Mic from './Mic.svelte';
  import { insertAt, applyInterim, voice, parseCommand, type VoiceCommand, type Interim } from './voice.svelte';
  /** Answer read-aloud (§4.7, slice 64) — as in the agent
   *  conversation */
  async function speakMsg(key: string, text: string) {
    if (!voice.settings) { try { await voice.load(); } catch {} }
    if (!voice.ready) { toast(t('voice.speakNotReady'), 'bad'); ui.section = 'settings'; ui.settingsTab = 'voice'; return; }
    try { await voice.speak(key, text); } catch (e) { toast(msg(e), 'bad'); }
  }
  let { id, onchange }: { id: string; onchange?: () => void } = $props();
  let chat = $state<SimpleChat | null>(null);
  let messages = $state<ChatMessage[]>([]);
  let text = $state('');
  let files = $state<Attachment[]>([]);
  let working = $state(false);
  let sending = $state(false);
  let addingFiles = $state(false);
  let stopping = $state(false);
  let sendError = $state('');
  let live = $state<{ text: string; tools: ChatToolUse[]; running: string | null } | null>(null);
  let box = $state<HTMLElement | null>(null);
  let ta = $state<HTMLTextAreaElement | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);
  let cameraInput = $state<HTMLInputElement | null>(null);
  let following = $state(true);
  let active = true;
  let loadError = $state('');
  let loading = $state(false);
  let loadSeq = 0;
  let abort = $state<AbortController | null>(null);
  const route = $derived(data.chatRoutes?.routes.find(r => r.id === chat?.route) ?? null);
  const modelInfo = $derived(route?.models.find(m => m.id === chat?.model) ?? null);
  const searchReady = $derived(!!data.chatRoutes?.search.ready);
  async function load(initial = false) {
    const requestId = id, seq = ++loadSeq; loading = true;
    try {
      const r = await agents.chatGet(requestId);
      if (!active || requestId !== id || seq !== loadSeq) return;
      chat = r.chat; messages = r.messages; working = r.busy; loadError = '';
      await tick();
      if (initial || following) scroll();
    } catch (e) { if (active && requestId === id && seq === loadSeq) loadError = msg(e); }
    finally { if (active && seq === loadSeq) loading = false; }
  }
  // the composer reset and loading — only on a chat change. The `id` prop comes as `curSimple.id` from the parent's derived
  // object, which is recreated by every chat list poll (every 5 s): an effect reading the prop directly would fire on every poll
  // and wipe the typed text; a `$derived` of a string suppresses equal values, and untrack keeps it from depending on chatRoutes
  const chatId = $derived(id);
  $effect(() => { const currentId = chatId; untrack(() => { const draft = drafts.get(currentId); live = null; text = draft?.text ?? ''; files = draft?.files ?? []; load(true); tick().then(autoGrow); }); });
  $effect(() => { const draftText = text; const draftFiles = [...files]; untrack(() => saveDraft(id, draftText, draftFiles)); });
  onDestroy(() => { active = false; saveDraft(id, text, files); abort?.abort(); });
  $effect(() => { if (!data.chatRoutes && !data.chatRoutesLoading && !data.chatRoutesError) untrack(() => data.loadChatRoutes()); });
  // A reopened chat or interrupted stream can still be running on the server.
  $effect(() => { if (!working || abort) return; const timer = setInterval(() => { if (!loading) load(); }, 2500); return () => clearInterval(timer); });
  // jump to a message from search (slice 34): after loading — scroll and highlight
  $effect(() => { const f = ui.focusMid; if (!f || f.chat !== id) return; const mid = f.mid; const h = setTimeout(() => { ui.focusMid = null; const el = box?.querySelector(`[data-mid="${mid}"]`) as HTMLElement | null; if (el) { el.scrollIntoView({ block: 'center' }); el.classList.add('hit'); setTimeout(() => el.classList.remove('hit'), 3000); } }, 700); return () => clearTimeout(h); });
  function scroll() { if (box) { box.scrollTop = box.scrollHeight; following = true; } }
  function trackScroll() { if (box) following = box.scrollHeight - box.scrollTop - box.clientHeight < 64; }
  async function followResponse() { await tick(); if (active && following) scroll(); }
  function autoGrow() { if (!ta) return; ta.style.height = 'auto'; ta.style.height = Math.min(window.innerHeight < 480 ? 72 : 180, ta.scrollHeight) + 'px'; }   // short screen — up to three lines
  /** Dictation (§4.7): text at the cursor position; on touch the field is not
   *  focused */
  function insertText(piece: string) {
    const cur = text ?? ''; const s = ta?.selectionStart ?? cur.length, e = ta?.selectionEnd ?? cur.length;
    const r = insertAt(cur, [s, e], piece); text = r.value; sendError = '';
    tick().then(() => { if (ta) { try { ta.setSelectionRange(r.caret, r.caret); } catch {} if (!touch()) ta.focus(); } autoGrow(); });
  }
  /** Simple chat voice commands (§4.7, slice 69): send, stop, clear, read aloud; the rest is text */
  async function voiceCommand(cmd: VoiceCommand): Promise<void> {
    switch (cmd.kind) {
      case 'text': insertText(cmd.text); return;
      case 'send': if (!text.trim() && !files.length) { toast(t('vc.nothingToSend'), 'bad'); return; } await send(); toast(t('vc.sent')); return;
      case 'stop': if (!working) { toast(t('vc.notRunning'), 'bad'); return; } await stop(); toast(t('vc.stopped')); return;
      case 'clear': text = ''; toast(t('vc.cleared')); return;
      case 'read': { const last = [...messages].reverse().find(m => m.role === 'assistant' && m.text?.trim()); if (!last) { toast(t('vc.noReply'), 'bad'); return; } await speakMsg(`chat:${last.id}`, last.text); return; }
      default: toast(t('vc.unsupportedHere'), 'bad');
    }
  }
  let interim: Interim | null = null;
  function onPartial(piece: string) {
    const cur = text ?? ''; const s = ta?.selectionStart ?? cur.length, e = ta?.selectionEnd ?? cur.length;
    const r = applyInterim(cur, [s, e], interim, piece); text = r.value; interim = r.region;
    tick().then(() => { if (ta) { try { ta.setSelectionRange(r.caret, r.caret); } catch {} } autoGrow(); });
  }
  function onVoice(piece: string) {
    if (interim) onPartial('');
    const cmd = parseCommand(piece);
    if (cmd) { voiceCommand(cmd).catch(e => toast(msg(e), 'bad')); return; }
    insertText(piece);
  }
  function onKey(e: KeyboardEvent) { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { if (touch()) return; e.preventDefault(); send(); } }
  async function addFiles(list: FileList | File[]) {
    if (addingFiles) return;
    addingFiles = true;
    try {
      for (const f of Array.from(list)) {
        if (f.size > 8 * 1024 * 1024) { toast(t('chat.tooBig', { name: f.name }), 'bad'); continue; }
        const data64 = await new Promise<string>((res, rej) => { const r = new FileReader(); r.onload = () => res(String(r.result).split(',')[1] ?? ''); r.onerror = () => rej(r.error); r.readAsDataURL(f); });
        if (!active) return;
        files.push({ name: f.name || 'image.png', mime: f.type || 'application/octet-stream', data: data64, url: f.type.startsWith('image/') ? URL.createObjectURL(f) : '' });
      }
    } catch (e) { if (active) { sendError = msg(e); toast(sendError, 'bad'); } }
    finally { addingFiles = false; }
  }
  function removeFile(i: number) { const [file] = files.splice(i, 1); if (file?.url) URL.revokeObjectURL(file.url); }
  function onPaste(e: ClipboardEvent) { const items = Array.from(e.clipboardData?.items ?? []).filter(i => i.kind === 'file'); if (!items.length) return; e.preventDefault(); addFiles(items.map(i => i.getAsFile()!).filter(Boolean)); }
  function onDrop(e: DragEvent) { e.preventDefault(); if (e.dataTransfer?.files?.length) addFiles(e.dataTransfer.files); }
  async function send() {
    if (!chat || working || addingFiles || (!text.trim() && !files.length)) return;
    const sentText = text, sentFiles = [...files], requestId = chat.id;
    const body = { text: sentText.trim(), attachments: sentFiles.map(f => ({ name: f.name, mime: f.mime, data: f.data })), tools: chat.tools };
    let accepted = false, completed = false;
    sendError = ''; working = true; sending = true; live = { text: '', tools: [], running: null }; abort = new AbortController();
    scroll();
    try {
      await agents.chatSend(requestId, body, (ev: ChatEvent) => {
        if (!active || id !== requestId) return;
        if (ev.kind === 'user') {
          accepted = true; sending = false; messages.push(ev.message);
          // Clear only the accepted draft; typing during the request belongs to the next turn.
          if (text === sentText) text = '';
          files = files.filter(f => !sentFiles.includes(f));
          sentFiles.forEach(f => { if (f.url) URL.revokeObjectURL(f.url); });
          tick().then(autoGrow);
        }
        else if (ev.kind === 'delta' && live) { live.text += ev.text; }
        else if (ev.kind === 'tool_start' && live) { live.running = ev.name; }
        else if (ev.kind === 'tool' && live) { live.tools.push({ name: ev.name, args: ev.args, result: ev.result, ok: ev.ok }); live.running = null; }
        else if (ev.kind === 'done' || ev.kind === 'error') { completed = true; if (ev.message) messages.push(ev.message); if (ev.chat) chat = ev.chat; live = null; if (ev.kind === 'error') { sendError = ev.error; toast(ev.error, 'bad'); } }
        followResponse();
      }, abort.signal);
      if (active && !completed) sendError = t('chat.disconnected');
    } catch (e) {
      if (active && !(e instanceof DOMException && e.name === 'AbortError')) {
        sendError = accepted ? t('chat.disconnected') : `${t('chat.sendFailed')} ${msg(e)}`;
        toast(sendError, 'bad');
      }
    } finally {
      if (active && id === requestId) { working = false; sending = false; live = null; abort = null; await load(); onchange?.(); }
    }
  }
  async function stop() { if (!chat || stopping) return; stopping = true; try { await agents.chatStop(chat.id); } catch (e) { toast(msg(e), 'bad'); } finally { stopping = false; } }
  function modelMenu(el: HTMLElement) {
    const rs = data.chatRoutes?.routes ?? [];
    if (!rs.length) { toast(t('chat.noRoutes'), 'bad'); return; }
    const win = (n?: number | null) => (n ? (n >= 1e6 ? `${Math.round(n / 1e5) / 10}M` : `${Math.round(n / 1000)}k`) : '');
    const items: Parameters<typeof menu.show>[1] = [];
    for (const r of rs) {
      items.push({ header: true, label: `${r.title}${r.error ? ' · ' + r.error : ''}` });
      // groups are the gate account channels (Claude Code, Codex, Antigravity…); one model may belong to several
      // channels
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
  function parametersMenu(el: HTMLElement) {
    menu.show(el, [
      { icon: 'search', label: t('chat.toolSearch'), checked: chat?.tools.search, disabled: !chat || (!searchReady && !chat.tools.search), hint: searchReady ? undefined : t('chat.searchOff'), on: () => toggleTool('search') },
      { icon: 'globe', label: t('chat.toolFetch'), checked: chat?.tools.fetch, disabled: !chat, on: () => toggleTool('fetch') },
      { sep: true },
      { icon: 'edit', label: t('chat.system'), checked: !!chat?.system, disabled: !chat, on: openSystem },
      { icon: 'image', label: t('chat.img'), hint: '/img', disabled: !chat, on: imgPrefix },
      { sep: true },
      { header: true, label: `${t('chat.route')}: ${route?.title ?? chat?.route ?? '—'}` },
      ...(chat ? [{ header: true, label: `${money(chat.cost_usd)} · ${chat.tokens_in + chat.tokens_out} ${t('chat.tokens')}` }] : []),
    ]);
  }
  // system prompt and presets (slice 37, §6.5): a popup instead of a
  // prompt() window
  let sysOpen = $state(false); let sysText = $state(''); let presets = $state<ChatPreset[]>([]); let sysBusy = $state(false);
  async function openSystem() { if (!chat) return; sysText = chat.system; sysOpen = true; try { presets = (await agents.chatPresets()).presets; } catch {} }
  async function saveSystem() { if (!chat) return; sysBusy = true; try { chat = await agents.chatPatch(chat.id, { system: sysText.trim() }); toast(t('toast.saved')); sysOpen = false; } catch (e) { toast(msg(e), 'bad'); } finally { sysBusy = false; } }
  async function savePresets(list: ChatPreset[]) { try { presets = (await agents.chatPresetsSave(list)).presets; } catch (e) { toast(msg(e), 'bad'); } }
  function saveAsPreset() { const name = window.prompt(t('chat.presetName'), sysText.split('\n')[0].slice(0, 40)); if (!name || !sysText.trim()) return; savePresets([...presets, { id: '', name: name.trim(), text: sysText.trim() }]); }
  function imgPrefix() { if (!text.startsWith('/img ')) text = '/img ' + text; ta?.focus(); }
  function fitSystem(el: HTMLElement) {
    const fit = () => {
      const anchor = el.parentElement?.getBoundingClientRect(); if (!anchor) return;
      const top = document.querySelector('.main .top')?.getBoundingClientRect().bottom ?? 0;
      el.style.maxHeight = `${Math.max(44, Math.min(520, Math.floor(anchor.top - top - 12)))}px`;
    };
    fit(); const observer = new ResizeObserver(fit);
    if (el.parentElement) observer.observe(el.parentElement);
    window.addEventListener('resize', fit); window.visualViewport?.addEventListener('resize', fit);
    return { destroy() { observer.disconnect(); window.removeEventListener('resize', fit); window.visualViewport?.removeEventListener('resize', fit); } };
  }
  const argText = (a: Record<string, unknown>) => Object.values(a).map(v => String(v)).join(' ').slice(0, 160);
  const toolLabel = (n: string) => (n === 'web_search' ? t('chat.toolSearch') : n === 'fetch_url' ? t('chat.toolFetch') : n);
</script>

<div class="chat" ondragover={(e) => e.preventDefault()} ondrop={onDrop} role="region" aria-label={t('chat.title')}>
  {#if chat?.incognito}<div class="incog"><Icon name="eyeOff" size="s" /> <span>{t('chat.incognitoBar')}{#if chat.expires_at} · {t('chat.expires', { t: when(chat.expires_at) })}{/if}</span></div>{/if}
  <div class="msgs" bind:this={box} data-following={following} onscroll={trackScroll}>
    {#if loading && !messages.length}<div class="muted" role="status">{t('common.loading')}</div>{/if}
    {#if !loading && !loadError && !messages.length && !live}<div class="empty" style="padding: 40px 12px"><span class="ic"><Icon name="zap" size="l" /></span><b>{t('chat.emptySimple')}</b><span style="max-width: 460px">{t('chat.emptySimpleHint')}</span></div>{/if}
    {#each messages as m (m.id)}
      {#if m.role === 'user'}
        <div class="m user" data-mid={m.id}>{#if m.text}<div class="txt">{m.text}</div>{/if}
          {#if m.attachments.length}<div class="atts">{#each m.attachments as a (a.file)}{#if a.mime.startsWith('image/')}<a href={agents.chatFileUrl(chat!.id, a.file)} target="_blank" rel="noopener"><img src={agents.chatFileUrl(chat!.id, a.file)} alt={a.name} /></a>{:else}<a class="chip file" href={agents.chatFileUrl(chat!.id, a.file)} target="_blank" rel="noopener"><Icon name="clip" size="s" /> {a.name} · {Math.round(a.size / 1024)} KB{#if a.mime === 'application/pdf' || a.name.toLowerCase().endsWith('.pdf')} · {a.text ? t('chat.pdfText') : t('chat.pdfNoText')}{/if}</a>{/if}{/each}</div>{/if}</div>
      {:else}
        {#each m.tools as tu, i (m.id + i)}<details class="m tool" class:err={!tu.ok}><summary><b>{toolLabel(tu.name)}</b> <span class="muted mono">{argText(tu.args)}</span> <span class="badge {tu.ok ? 'ok' : 'bad'}">{tu.ok ? t('agents.toolDone') : t('agents.toolError')}</span></summary><pre class="mono res">{tu.result}</pre></details>{/each}
        {#if m.text}{@const sk = `chat:${m.id}`}<div class="m agent" data-mid={m.id} class:wide={/(^|\n)\s*(\||```|#{1,4} |[-*] |\d+\. )/.test(m.text) || m.attachments?.length > 0}><Md text={m.text} /><span class="macts"><button class="ma speak" class:on={voice.speaking === sk} class:busy={voice.speakLoading === sk} aria-pressed={voice.speaking === sk} title={voice.speaking === sk ? t('voice.stop') : t('voice.speak')} onclick={() => speakMsg(sk, m.text)}><Icon name={voice.speaking === sk ? 'square' : 'volume'} size="s" /></button></span>{#if m.attachments?.length}<div class="atts">{#each m.attachments as a (a.file)}{#if a.mime.startsWith('image/')}<a href={agents.chatFileUrl(chat!.id, a.file)} target="_blank" rel="noopener"><img src={agents.chatFileUrl(chat!.id, a.file)} alt={a.name} /></a>{/if}{/each}</div>{/if}<div class="meta muted xs">{m.tokens_in + m.tokens_out} {t('chat.tokens')} · {money(m.cost_usd)}{#if m.error} · <span class="bad">{m.error}</span>{/if}</div></div>
        {:else if m.error}<div class="m sys"><span class="bad">⚠ {m.error}</span></div>{/if}
      {/if}
    {/each}
    {#if live}
      {#each live.tools as tu, i (i)}<details class="m tool" class:err={!tu.ok}><summary><b>{toolLabel(tu.name)}</b> <span class="muted mono">{argText(tu.args)}</span> <span class="badge {tu.ok ? 'ok' : 'bad'}">{tu.ok ? t('agents.toolDone') : t('agents.toolError')}</span></summary><pre class="mono res">{tu.result}</pre></details>{/each}
      {#if live.running}<div class="m sys"><span class="spin"></span> {toolLabel(live.running)} · {t('chat.toolRunning')}</div>{/if}
      {#if live.text}<div class="m agent live"><Md text={live.text} live /></div>{:else if !live.running}<div class="m sys"><span class="spin"></span> {t('chat.thinkingDots')}</div>{/if}
    {/if}
  </div>
  {#if !following && messages.length}<button class="latest" onclick={scroll}><Icon name="chevD" size="s" /> {t('chat.latest')}</button>{/if}
  {#if loadError}<div class="load-warning" role="alert"><span>{chat ? t('ux.refreshFailed') : t('ux.loadFailed')} {loadError}</span><button disabled={loading} onclick={() => load(true)}>{t('common.retry')}</button></div>{/if}
  <div class="composer chat-composer" class:busy={working} aria-busy={addingFiles || sending}>
    {#if data.chatRoutesError}<div class="load-warning" role="alert"><span>{data.chatRoutes ? t('ux.refreshFailed') : t('ux.loadFailed')} {data.chatRoutesError}</span><button disabled={data.chatRoutesLoading} onclick={() => data.loadChatRoutes()}>{t('common.retry')}</button></div>{/if}
    {#if working || addingFiles}
      <div class="composer-state"><span class="composer-status" role="status"><span class="spin"></span> {addingFiles ? t('common.loading') : sending ? t('cmp.sending') : t('chat.thinkingDots')}</span></div>
    {/if}
    {#if files.length}<div class="thumbs">{#each files as f, i (i)}<span class="thumb">{#if f.url}<img src={f.url} alt={f.name} />{:else}<span class="chip"><Icon name="clip" size="s" /> {f.name}</span>{/if}<button class="x" onclick={() => removeFile(i)} aria-label={t('cmp.removeAttachment', { name: f.name })} title={t('cmp.removeAttachment', { name: f.name })}>×</button></span>{/each}</div>{/if}
    <div class="in composer-input">
      <textarea bind:this={ta} bind:value={text} rows="2" placeholder={t('cmp.placeholderPhone')} aria-label={t('cmp.messageLabel')} onkeydown={onKey} oninput={() => { sendError = ''; autoGrow(); }} onpaste={onPaste}></textarea>
    </div>
    {#if sendError}<div class="composer-error" role="alert">{sendError}</div>{/if}
    <div class="composer-toolbar">
      <div class="tools composer-controls">
        <button class="ctl mono composer-control composer-model" title={`${t('chat.model')}: ${chat?.model || '—'}${modelInfo?.context ? ' · ' + Math.round(modelInfo.context / 1000) + 'k' : ''}`} disabled={!chat} onclick={(e) => { e.stopPropagation(); modelMenu(e.currentTarget as HTMLElement); }}><Icon name="cpu" size="s" /><span class="lbl">{modelInfo?.name || chat?.model || t('chat.model')}</span></button>
        <button class="ctl composer-control composer-more" title={t('cmp.parameters')} aria-label={t('cmp.parameters')} aria-haspopup="menu" disabled={!chat} onclick={(e) => { e.stopPropagation(); parametersMenu(e.currentTarget as HTMLElement); }}><Icon name="settings" size="s" /><span class="more-label">{t('cmp.parameters')}</span></button>
        {#if chat?.tools.search}<button class="ctl tsearch on composer-control composer-secondary" title={t('chat.toolSearch')} aria-pressed="true" onclick={() => toggleTool('search')}><Icon name="search" size="s" /> {t('chat.toolSearch')}</button>{/if}
        {#if chat?.tools.fetch}<button class="ctl tfetch on composer-control composer-secondary" title={t('chat.toolFetch')} aria-pressed="true" onclick={() => toggleTool('fetch')}><Icon name="globe" size="s" /> {t('chat.toolFetch')}</button>{/if}
      </div>
      <div class="composer-actions">
        <input bind:this={fileInput} type="file" multiple hidden onchange={(e) => { const input = e.currentTarget; if (input.files) addFiles(input.files); input.value = ''; }} />
        <Mic onText={onVoice} {onPartial} cls="icon ghost composer-attach" />
        <button class="icon ghost composer-attach" title={t('chat.attach')} aria-label={t('chat.attach')} disabled={addingFiles} onclick={() => fileInput?.click()}><Icon name="clip" /></button>
        {#if phone()}
          <input bind:this={cameraInput} type="file" accept="image/*" capture="environment" hidden onchange={(e) => { const input = e.currentTarget; if (input.files) addFiles(input.files); input.value = ''; }} />
          <button class="icon ghost cam composer-attach" title={t('chat.camera')} aria-label={t('chat.camera')} disabled={addingFiles} onclick={() => cameraInput?.click()}><Icon name="camera" /></button>
        {/if}
        {#if working && !sending}<button class="danger composer-send composer-stop" disabled={stopping} onclick={stop} aria-label={t('chat.stop')} title={t('chat.stop')}><Icon name="square" size="s" /><span class="action-label">{t('chat.stop')}</span></button>{:else}<button class="primary send composer-send" disabled={!chat || addingFiles || sending || (!text.trim() && !files.length)} onclick={send} aria-label={sending ? t('cmp.sending') : t('chat.send')} title={t('chat.send')}><Icon name="send" size="s" /><span class="action-label">{sending ? t('cmp.sending') : t('chat.send')}</span></button>{/if}
      </div>
    </div>
    {#if !touch()}<div class="composer-hint">{t('cmp.enterHint')}</div>{/if}
    {#if sysOpen}
      <div class="modes syspop" role="dialog" tabindex="-1" aria-label={t('chat.system')} use:fitSystem onclick={(e) => e.stopPropagation()} onkeydown={(e) => { if (e.key === 'Escape') { sysOpen = false; e.stopPropagation(); ta?.focus(); } }}>
        <div class="mh"><span>{t('chat.system')}</span><span class="sp"></span><button class="icon xs ghost" aria-label={t('common.close')} onclick={() => (sysOpen = false)}><Icon name="x" size="s" /></button></div>
        <textarea class="sys" rows="4" bind:value={sysText} placeholder={t('chat.systemHint')} aria-label={t('chat.system')}></textarea>
        <div class="ph muted xs">{t('chat.presets')}</div>
        {#each presets as p (p.id || p.name)}
          <div class="preset" data-preset={p.id}><span class="mi-t"><b>{p.name}</b><span class="muted">{p.text.slice(0, 90)}</span></span><button class="xs ghost apply" onclick={() => (sysText = p.text)}>{t('chat.presetApply')}</button><button class="icon xs ghost" title={t('chat.presetDelete')} onclick={() => savePresets(presets.filter((x) => x !== p))}><Icon name="x" size="s" /></button></div>
        {/each}
        <div class="acts"><button class="xs ghost saveas" disabled={!sysText.trim()} onclick={saveAsPreset}><Icon name="plus" size="s" /> {t('chat.presetSaveAs')}</button><span class="sp"></span><button class="sm primary save" disabled={sysBusy} onclick={saveSystem}>{t('common.save')}</button></div>
      </div>
    {/if}
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
  .latest { align-self: center; display: flex; align-items: center; gap: 5px; min-height: 30px; height: auto; padding: 4px 12px; font-size: 12px; color: var(--accent); }
  .thumbs { display: flex; gap: 6px; flex-wrap: wrap; padding: 2px 4px; }
  .thumb { position: relative; display: inline-flex; align-items: center; max-width: 100%; }
  .thumb img { height: 56px; border-radius: 6px; }
  .thumb .chip { max-width: 100%; overflow: hidden; text-overflow: ellipsis; padding-right: 16px; }
  .thumb .x { position: absolute; top: -6px; right: -6px; width: 24px; height: 24px; border-radius: 50%; padding: 0; font-size: 14px; line-height: 1; }
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
  .spin { width: 10px; height: 10px; border: 2px solid var(--line-3); border-top-color: var(--accent); border-radius: 50%; display: inline-block; animation: sp 1s linear infinite; }
  @keyframes sp { to { transform: rotate(360deg); } }
  @media (max-width: 720px) { .m { max-width: 92%; } }
  @media (pointer: coarse) { .thumb .x { width: 32px; height: 32px; } }
</style>

// Provider adapters for the chat (modelled on CloudCLI): Claude Code and Codex name models, effort, modes, tools and
// commands differently — the UI asks the adapter instead of checking `agent` in place. What the installed CLI really can
// do comes from agentd as the session `caps` (Claude models and slash commands — from its `initialize`).
import { t } from './i18n';
import type { Caps, SessionEvent } from './api';

export type Opt = { value: string; label: string; hint?: string; context?: number; desc?: string; hidden?: boolean };
const fmtWin = (n?: number) => (n ? (n >= 1e6 ? `${n / 1e6}M` : `${Math.round(n / 1000)}k`) : '');
/** Context window of the selected model (from caps), otherwise the provider
 *  default. */
/** Context window by model family (the same numbers as in the service catalog and agentd): Codex itself does not know
 *  every model and for
 * GPT-6 Astra/Luna takes 272k — until its report has arrived or it is clearly the "default", count by
 * family */
export function familyWindow(id: string | null | undefined): number | null {
  const m = (id ?? '').toLowerCase();
  if (m.startsWith('gpt-6')) return 1_050_000;
  if (m.startsWith('gpt-5.6')) return 400_000;
  return null;
}
export function modelWindow(caps: Caps | null, id: string | null | undefined, fallback: number): number {
  const m = caps?.models?.find(x => x.value === (id ?? 'default') || x.resolved === id);
  return (m as any)?.context ?? (id && /\[1m\]|1m/i.test(id) ? 1_000_000 : fallback);
}
export type Command = { name: string; hint?: string; desc?: string; local?: boolean };
export type Ctx = { used: number; window: number; pct: number; source: 'provider' | 'estimate'; threshold?: number; categories?: any };

export interface Adapter {
  id: string;
  icon: string;
  name: string;
  models(caps: Caps | null): Opt[];
  modelLabel(id: string | null | undefined, caps: Caps | null): string;
  efforts(caps: Caps | null): Opt[];
  /** Everything the provider accepts via the /effort command (wider
   *  than the slider) */
  effortsAll: string[];
  effortLabel(v: string | null | undefined): string;
  modes(caps?: Caps | null): Opt[];
  modeLabel(v: string | null | undefined, caps?: Caps | null): string;
  /** Slash commands: `local` ones are intercepted by the UI (model, effort, mode), the rest go to the provider as is. */
  commands(caps: Caps | null): Command[];
  toolLabel(name: string): string;
  toolSummary(name: string, input: any): string;
  /** Current context: for Codex from the thread's tokenUsage (caps), for Claude — from the last usage in the journal (more precisely
   *  — get_context_usage). */
  context(caps: Caps | null, events: SessionEvent[], model?: string | null): Ctx | null;
  supportsFiles: boolean;
  supportsCompact: boolean;
}

// effort names are the originals, as in the CLI (low · medium · high · xhigh · max · ultracode), untranslated; empty —
// auto
const effortLabel = (v?: string | null) => v || 'auto';
const effortHint = (e: string) => t('eff.' + e);
// a function, not a constant: command labels are recomputed on a
// language change
const LOCAL = (): Command[] => [
  { name: 'model', hint: '<model>', desc: t('cmp.cmdModel'), local: true },
  { name: 'effort', hint: '<auto|low|medium|high|xhigh|max|ultracode>', desc: t('cmp.cmdEffort'), local: true },
  { name: 'mode', hint: '<mode>', desc: t('cmp.cmdMode'), local: true },
];
const shortModel = (m: string) => m.replace('claude-', '').replace('-medium', '');

function genericSummary(name: string, input: any): string {
  if (!input) return '';
  if (name === 'Bash') return input.command ?? '';
  if (['Write', 'Edit', 'Read', 'MultiEdit', 'NotebookEdit'].includes(name)) return input.file_path ?? '';
  if (name === 'Glob' || name === 'Grep') return input.pattern ?? '';
  if (name === 'WebFetch' || name === 'WebSearch') return input.url ?? input.query ?? '';
  if (name === 'Task' || name === 'Agent') return input.description ?? input.prompt?.slice(0, 80) ?? '';
  const s = JSON.stringify(input); return s.length > 160 ? s.slice(0, 160) + '…' : s;
}

/** Context estimate from the last API call in the journal: input + cache read + cache write = the context size at that
 *  moment. */
function estimateFromEvents(events: SessionEvent[], fallbackWindow: number): Ctx | null {
  // from the end of the journal: the last API call (an assistant with usage) gives the context size, the nearest
  // result — the model window; result comes after assistant, so we must not stop at it until the call itself is
  // found
  let used = 0, win = fallbackWindow, found = false, winFound = false;
  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i];
    if (!found && e.kind === 'assistant' && e.usage && !e.local) {
      const u = e.usage; used = (u.input_tokens ?? 0) + (u.cache_read_input_tokens ?? 0) + (u.cache_creation_input_tokens ?? 0); found = true;
    }
    if (!winFound && e.kind === 'result' && e.model_usage) { const w = Object.values(e.model_usage as Record<string, any>).map(m => m.contextWindow).find(Boolean); if (w) { win = w; winFound = true; } }
    if (found && (winFound || i < events.length - 80)) break;
  }
  return found ? { used, window: win, pct: Math.min(100, Math.round((used / win) * 100)), source: 'estimate' } : null;
}

export const claude: Adapter = {
  id: 'claude', icon: 'claude', name: 'Claude Code',
  models: (caps) => (caps?.models ?? []).map(m => ({ value: m.value ?? '', label: m.label ?? m.value ?? '', hint: fmtWin((m as any).context), desc: m.description ?? undefined, context: (m as any).context })),
  modelLabel(id, caps) {
    if (!id) { const d = caps?.models?.find(x => x.value === 'default'); const cur = d?.description?.match(/currently ([^)·]+\)?)/i)?.[1]; return cur ? `${t('agents.modelDefault')} · ${cur.trim()}` : t('agents.modelDefault'); }
    const m = caps?.models?.find(x => x.value === id || x.resolved === id); return m?.label ?? shortModel(id);
  },
  // as in Claude Code's own picker: low · medium · high · xhigh · max · ultracode (the top step is "xhigh + workflows")
  efforts: (caps) => (caps?.efforts ?? ['low', 'medium', 'high', 'xhigh', 'max', 'ultracode']).map(v => ({ value: v, label: v, hint: caps?.effort_hints?.[v] ?? effortHint(v) })),
  effortsAll: ['auto', 'low', 'medium', 'high', 'xhigh', 'max', 'ultracode'],
  effortLabel,
  // as in Claude Code itself (⇧+Tab): Manual · Edit automatically · Plan · Auto; bypass — per the workspace policy; dontAsk is rare, at
  // the end
  modes: () => [
    { value: 'default', label: t('mode.manual'), hint: t('mode.manualHint') }, { value: 'acceptEdits', label: t('mode.acceptEdits'), hint: t('mode.acceptEditsHint') },
    { value: 'plan', label: t('mode.plan'), hint: t('mode.planHint') }, { value: 'auto', label: t('mode.auto'), hint: t('mode.autoHint') },
    { value: 'bypassPermissions', label: t('mode.bypass'), hint: t('mode.bypassHint') }, { value: 'dontAsk', label: t('mode.dontAsk'), hint: t('mode.dontAskHint'), hidden: true },
  ],
  modeLabel: (v) => ({ default: t('mode.manual'), manual: t('mode.manual'), acceptEdits: t('mode.acceptEdits'), plan: t('mode.plan'), auto: t('mode.auto'), bypassPermissions: t('mode.bypass'), dontAsk: t('mode.dontAsk') } as Record<string, string>)[v ?? 'default'] ?? (v ?? ''),
  commands(caps) {
    const own = new Set(LOCAL().map(c => c.name));
    // before the first start under agentd (a conversation from disk) there is no list from the CLI yet — show the
    // built-in Claude Code commands
    const FALLBACK = [['compact', `<${t('cmd.desc')}>`, t('cmd.compact')], ['context', '', t('cmd.context')], ['cost', '', t('cmd.cost')], ['usage', '', t('cmd.usage')], ['clear', '', t('cmd.clear')], ['rename', `<${t('cmd.name')}>`, t('cmd.rename')], ['init', '', t('cmd.init')], ['review', '', t('cmd.review')]];
    const src = caps?.commands?.length ? caps.commands : FALLBACK.map(([name, argumentHint, description]) => ({ name, argumentHint, description }));
    const cli = src.filter(c => !own.has(c.name)).map(c => ({ name: c.name, hint: c.argumentHint || undefined, desc: c.description || undefined }));
    // the most needed first, as in the TUI
    const top = ['compact', 'context', 'cost', 'usage', 'clear', 'rename', 'review', 'init'];
    cli.sort((a, b) => (top.indexOf(a.name) + 1 || 99) - (top.indexOf(b.name) + 1 || 99) || a.name.localeCompare(b.name));
    return [...LOCAL(), ...cli];
  },
  toolLabel: (name) => name,
  toolSummary: genericSummary,
  context: (caps, events, model) => estimateFromEvents(events, (caps?.context_window as number | null | undefined) ?? modelWindow(caps, model, 200000)),
  supportsFiles: true, supportsCompact: true,
};

export const codex: Adapter = {
  id: 'codex', icon: 'codex', name: 'Codex',
  models: (caps) => (caps?.models ?? []).filter(m => !m.hidden).map(m => ({ value: m.value ?? '', label: m.label ?? m.value ?? '', desc: [m.description, (m as any).upgrade ? t('cmp.modelUpgrade', { m: (m as any).upgrade }) : ''].filter(Boolean).join(' · ') || undefined })),
  modelLabel(id, caps) { const m = caps?.models?.find(x => id ? x.value === id : x.default); return m?.label ?? (id || t('agents.modelDefault')); },
  // reasoning levels of the selected model from the app-server catalog (model/list) with its own descriptions:
  // low … max · ultra
  efforts: (caps) => (caps?.efforts ?? ['low', 'medium', 'high', 'xhigh']).map(v => ({ value: v, label: v, hint: caps?.effort_hints?.[v] ?? effortHint(v) })),
  effortsAll: ['minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'],
  effortLabel,
  modes: () => [
    { value: 'on-request', label: t('agents.modeOnRequest'), hint: t('cmp.modeOnRequestHint') }, { value: 'untrusted', label: t('agents.modeUntrusted'), hint: t('cmp.modeUntrustedHint') },
    { value: 'never', label: t('agents.modeNever'), hint: t('cmp.modeNeverHint') },
  ],
  modeLabel: (v) => ({ 'on-request': t('agents.modeOnRequest'), untrusted: t('agents.modeUntrusted'), never: t('agents.modeNever') } as Record<string, string>)[v ?? 'on-request'] ?? (v ?? ''),
  commands: () => LOCAL().map(c => (c.name === 'effort' ? { ...c, hint: '<low|medium|high|xhigh>' } : c.name === 'mode' ? { ...c, hint: '<on-request|untrusted|never>' } : c)),
  toolLabel: (name) => (name === 'Bash' ? t('cmp.toolCommand') : name === 'Edit' ? t('cmp.toolPatch') : name),
  toolSummary(name, input) { if (name === 'Edit' && input?.changes?.length) return (input.changes as string[]).join(', '); return genericSummary(name, input); },
  context(caps, events, model) {
    const tu = caps?.token_usage; const reported = caps?.context_window as number | null | undefined;
    // 258 400 = the Codex default for an unknown model (272k × 0.95): for families with a known window it lies (Astra: "284k of
    // 258k")
    const win = reported && reported !== 258400 ? reported : (familyWindow(model) ?? reported ?? 258400);
    const last = tu?.last ?? tu?.last_token_usage; const total = tu?.total ?? tu?.total_token_usage;
    const used = last ? (last.inputTokens ?? last.input_tokens ?? 0) + (last.cachedInputTokens ?? last.cached_input_tokens ?? 0) : total ? (total.totalTokens ?? total.total_tokens ?? 0) : 0;
    // the percentage is not clamped to 100: if more than the window is used, the window is wrong — that must
    // be visible
    if (used) return { used, window: win, pct: Math.round((used / win) * 100), source: 'provider' };
    return estimateFromEvents(events, win);
  },
  supportsFiles: true, supportsCompact: false,
};

/** An ACP engine (Gemini CLI, pi, …): models and modes are what the agent returned in session/new (caps), no efforts, commands — its slash
 *  commands. */
export function acpAdapter(id: string, name: string, icon: string): Adapter {
  const modeName = (m: { id: string; name: string }) => t('acpmode.' + m.id) !== 'acpmode.' + m.id ? t('acpmode.' + m.id) : m.name;
  return {
    id, icon, name,
    models: (caps) => (caps?.models ?? []).map(m => ({ value: m.value ?? '', label: m.label ?? m.value ?? '', desc: m.description ?? undefined })),
    modelLabel(id2, caps) { const m = caps?.models?.find(x => x.value === id2); return m?.label ?? (id2 || t('agents.modelDefault')); },
    efforts: () => [], effortsAll: [], effortLabel,
    modes: (caps) => (caps?.mode_options ?? []).map(m => ({ value: m.id, label: modeName(m), hint: m.description })),
    modeLabel: (v, caps) => { const m = caps?.mode_options?.find(x => x.id === v); return m ? modeName(m) : (v ?? ''); },
    commands: (caps) => [...LOCAL().filter(c => c.name !== 'effort'), ...(caps?.commands ?? []).map(c => ({ name: c.name, hint: c.argumentHint || undefined, desc: c.description || undefined }))],
    toolLabel: (n) => (n === 'Bash' ? t('cmp.toolCommand') : n),
    toolSummary: genericSummary,
    context: (caps) => { const u = caps?.acp_usage; if (u?.used && u?.size) return { used: u.used, window: u.size, pct: Math.min(100, Math.round((u.used / u.size) * 100)), source: 'provider' }; return null; },
    supportsFiles: false, supportsCompact: false,
  };
}
export const gemini = acpAdapter('gemini', 'Gemini CLI', 'gemini');
export const pi = acpAdapter('pi', 'pi', 'pi');
/** Antigravity (agentd agy.rs on top of headless agy): models — the gate catalog, modes and efforts — from caps (its own),
 *  no commands. */
export const antigravity: Adapter = {
  ...acpAdapter('antigravity', 'Antigravity', 'antigravity'),
  efforts: (caps) => (caps?.efforts ?? ['low', 'medium', 'high', 'xhigh', 'max']).map(v => ({ value: v, label: v, hint: caps?.effort_hints?.[v] ?? effortHint(v) })),
  effortsAll: ['low', 'medium', 'high', 'xhigh', 'max'],
  commands: () => LOCAL().filter(c => c.name !== 'effort'),
};
export const omp = acpAdapter('omp', 'Oh My Pi', 'omp');
export const ADAPTERS: Record<string, Adapter> = { claude, codex, gemini, pi, antigravity, omp };
export const adapterFor = (agent?: string | null): Adapter => ADAPTERS[agent ?? 'claude'] ?? (agent ? acpAdapter(agent, agent, 'cpu') : claude);

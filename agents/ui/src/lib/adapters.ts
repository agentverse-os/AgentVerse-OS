// Адаптеры провайдеров для чата (по образцу CloudCLI): Claude Code и Codex по-разному называют модели, усилие, режимы,
// инструменты и команды — интерфейс спрашивает адаптер, а не проверяет `agent` по месту. Что реально умеет установленный
// CLI, приходит из agentd как `caps` сессии (модели и слэш-команды Claude — из его `initialize`).
import { t } from './i18n';
import type { Caps, SessionEvent } from './api';

export type Opt = { value: string; label: string; hint?: string; context?: number; desc?: string; hidden?: boolean };
const fmtWin = (n?: number) => (n ? (n >= 1e6 ? `${n / 1e6}M` : `${Math.round(n / 1000)}k`) : '');
/** Окно контекста выбранной модели (из caps), иначе умолчание провайдера. */
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
  /** Всё, что принимает провайдер командой /effort (шире ползунка) */
  effortsAll: string[];
  effortLabel(v: string | null | undefined): string;
  modes(caps?: Caps | null): Opt[];
  modeLabel(v: string | null | undefined, caps?: Caps | null): string;
  /** Слэш-команды: `local` — перехватываются интерфейсом (модель, усилие, режим), остальные уходят провайдеру как есть. */
  commands(caps: Caps | null): Command[];
  toolLabel(name: string): string;
  toolSummary(name: string, input: any): string;
  /** Текущий контекст: у Codex из tokenUsage треда (caps), у Claude — по последнему usage в журнале (точнее — get_context_usage). */
  context(caps: Caps | null, events: SessionEvent[], model?: string | null): Ctx | null;
  supportsFiles: boolean;
  supportsCompact: boolean;
}

// названия усилий — оригинальные, как в CLI (low · medium · high · xhigh · max · ultracode), без перевода; пусто — auto
const effortLabel = (v?: string | null) => v || 'auto';
const effortHint = (e: string) => t('eff.' + e);
// функция, не константа: подписи команд пересчитываются при смене языка
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

/** Оценка контекста по последнему вызову API в журнале: вход + чтение кэша + запись кэша = размер контекста на тот момент. */
function estimateFromEvents(events: SessionEvent[], fallbackWindow: number): Ctx | null {
  // с конца журнала: последний вызов API (assistant с usage) даёт размер контекста, ближайший result — окно модели;
  // result идёт после assistant, поэтому останавливаться на нём нельзя, пока не найден сам вызов
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
  // как в пикере самого Claude Code: low · medium · high · xhigh · max · ultracode (верхняя ступень — «xhigh + workflows»)
  efforts: (caps) => (caps?.efforts ?? ['low', 'medium', 'high', 'xhigh', 'max', 'ultracode']).map(v => ({ value: v, label: v, hint: caps?.effort_hints?.[v] ?? effortHint(v) })),
  effortsAll: ['auto', 'low', 'medium', 'high', 'xhigh', 'max', 'ultracode'],
  effortLabel,
  // как в самом Claude Code (⇧+Tab): Manual · Edit automatically · Plan · Auto; обход — по политике workspace'а; dontAsk — редкий, в конце
  modes: () => [
    { value: 'default', label: t('mode.manual'), hint: t('mode.manualHint') }, { value: 'acceptEdits', label: t('mode.acceptEdits'), hint: t('mode.acceptEditsHint') },
    { value: 'plan', label: t('mode.plan'), hint: t('mode.planHint') }, { value: 'auto', label: t('mode.auto'), hint: t('mode.autoHint') },
    { value: 'bypassPermissions', label: t('mode.bypass'), hint: t('mode.bypassHint') }, { value: 'dontAsk', label: t('mode.dontAsk'), hint: t('mode.dontAskHint'), hidden: true },
  ],
  modeLabel: (v) => ({ default: t('mode.manual'), manual: t('mode.manual'), acceptEdits: t('mode.acceptEdits'), plan: t('mode.plan'), auto: t('mode.auto'), bypassPermissions: t('mode.bypass'), dontAsk: t('mode.dontAsk') } as Record<string, string>)[v ?? 'default'] ?? (v ?? ''),
  commands(caps) {
    const own = new Set(LOCAL().map(c => c.name));
    // до первого запуска под agentd (беседа с диска) списка от CLI ещё нет — показываем встроенные команды Claude Code
    const FALLBACK = [['compact', `<${t('cmd.desc')}>`, t('cmd.compact')], ['context', '', t('cmd.context')], ['cost', '', t('cmd.cost')], ['usage', '', t('cmd.usage')], ['clear', '', t('cmd.clear')], ['rename', `<${t('cmd.name')}>`, t('cmd.rename')], ['init', '', t('cmd.init')], ['review', '', t('cmd.review')]];
    const src = caps?.commands?.length ? caps.commands : FALLBACK.map(([name, argumentHint, description]) => ({ name, argumentHint, description }));
    const cli = src.filter(c => !own.has(c.name)).map(c => ({ name: c.name, hint: c.argumentHint || undefined, desc: c.description || undefined }));
    // самые нужные — первыми, как в TUI
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
  models: (caps) => (caps?.models ?? []).filter(m => !m.hidden).map(m => ({ value: m.value ?? '', label: m.label ?? m.value ?? '', desc: m.description ?? undefined })),
  modelLabel(id, caps) { const m = caps?.models?.find(x => id ? x.value === id : x.default); return m?.label ?? (id || t('agents.modelDefault')); },
  // уровни reasoning выбранной модели из каталога app-server (model/list) с его же описаниями: low … max · ultra
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
  context(caps, events) {
    const tu = caps?.token_usage; const win = caps?.context_window ?? 258400;
    const last = tu?.last ?? tu?.last_token_usage; const total = tu?.total ?? tu?.total_token_usage;
    const used = last ? (last.inputTokens ?? last.input_tokens ?? 0) + (last.cachedInputTokens ?? last.cached_input_tokens ?? 0) : total ? (total.totalTokens ?? total.total_tokens ?? 0) : 0;
    if (used) return { used, window: win, pct: Math.min(100, Math.round((used / win) * 100)), source: 'provider' };
    return estimateFromEvents(events, win);
  },
  supportsFiles: true, supportsCompact: false,
};

/** Движок по ACP (Gemini CLI, pi, …): модели и режимы — те, что агент отдал в session/new (caps), усилий нет, команды — его слэш-команды. */
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
export const ADAPTERS: Record<string, Adapter> = { claude, codex, gemini, pi };
export const adapterFor = (agent?: string | null): Adapter => ADAPTERS[agent ?? 'claude'] ?? (agent ? acpAdapter(agent, agent, 'cpu') : claude);

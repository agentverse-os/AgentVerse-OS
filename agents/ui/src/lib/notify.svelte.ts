// Notification channels (docs/agents-ux.md §9.12): the channels with their bindings, shared by Settings, the conversation
// menu, the routine form and the "to a channel" action of a message. Loaded once and re-read when the service says the
// lists have changed (a hub frame): a "once" binding goes away by itself after the turn it waited for. The binding dialog
// is one for the whole app.
import { agents, type NotifyBinding, type NotifyBindBody, type NotifyChannel, type NotifyKind } from './api';
import { t } from './i18n';

export type BindTarget = { scope: 'session' | 'workspace' | 'routine'; project: string; session?: string; routine?: string; title: string };

const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

class Notify {
  channels = $state<NotifyChannel[]>([]);
  loaded = $state(false);
  loading = $state(false);
  error = $state('');
  /** What the binding dialog is open for */
  target = $state<BindTarget | null>(null);
  private running: Promise<void> | null = null;
  private again = false;

  /** Read the channels. A call during a read asks for one more read after it: what changed meanwhile is not lost. */
  load(): Promise<void> {
    if (this.running) { this.again = true; return this.running; }
    this.loading = true;
    this.running = (async () => {
      try {
        do { this.again = false; this.channels = (await agents.notifyChannels()).channels; this.error = ''; this.loaded = true; } while (this.again);
      } catch (e) { this.error = msg(e); }
      finally { this.loading = false; this.running = null; }
    })();
    return this.running;
  }
  get bindings(): NotifyBinding[] { return this.channels.flatMap(c => c.bindings); }
  channel(id: string): NotifyChannel | undefined { return this.channels.find(c => c.id === id); }
  /** Bindings of one target: a conversation, a workspace, a routine */
  of(tg: BindTarget): NotifyBinding[] {
    return this.bindings.filter(b => b.scope === tg.scope && (tg.scope === 'routine' ? b.routine === tg.routine : b.project === tg.project && (tg.scope !== 'session' || b.session === tg.session)));
  }
  forSession(project: string, session: string): NotifyBinding[] { return this.bindings.filter(b => b.scope === 'session' && b.project === project && b.session === session); }
  /** "Telegram · Личный · один раз · отчёты" — one binding in words, for the conversation header and the inspector */
  words(b: NotifyBinding): string {
    const c = this.channel(b.channel);
    return [c ? `${kindName(c.kind)} · ${c.name}` : '—', ...(b.scope === 'session' && b.when === 'once' ? [t('ntf.once')] : []), ...(b.reports ? [t('ntf.reports')] : [])].join(' · ');
  }
  open(tg: BindTarget) { this.target = tg; void this.load(); }
  close() { this.target = null; }
  async bind(b: NotifyBindBody) { await agents.notifyBind(b); await this.load(); }
  async unbind(id: string) { await agents.notifyUnbind(id); await this.load(); }
}
export const notify = new Notify();

/** The kind of a channel in words (brand names are not translated) */
export function kindName(k: NotifyKind): string { return k === 'telegram' ? 'Telegram' : k === 'slack' || k === 'slack_bot' ? 'Slack' : k === 'discord' ? 'Discord' : t('ntf.kWebhookShort'); }
export function kindIcon(k: NotifyKind): string { return k === 'telegram' ? 'send' : k === 'webhook' ? 'globe' : 'chat'; }

/** The first line of the note the service puts after the owner's message when the agent is told about its report box
 *  (reports.rs, INTRO_HEAD): the feed folds the note away. */
export const REPORTS_HEAD = '# Отчёты владельцу';
/** A message of the owner's with that note after it: the note is taken off, the rest is the message itself. */
export function withoutReportsNote(text: string): { note: boolean; text: string } {
  if (text.startsWith(REPORTS_HEAD)) return { note: true, text: '' };
  const i = text.lastIndexOf(`\n---\n${REPORTS_HEAD}`);
  return i < 0 ? { note: false, text } : { note: true, text: text.slice(0, i).trimEnd() };
}

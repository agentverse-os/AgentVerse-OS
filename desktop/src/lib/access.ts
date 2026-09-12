// Данные для входа в приложение (docs/userflow-apps.md, раздел 4): манифест `login` ссылается на переменные env
// или содержит литералы; значения несекретных переменных приходят в settings_values, секреты — по reveal.
import type { AppView, LoginSpec } from './api';
import { sameHost } from './store-utils';
import { t } from './i18n.svelte';

export interface AccessField { label: string; env?: string; value?: string; secret: boolean }
export interface Access { mode: LoginSpec['mode']; user: AccessField | null; password: AccessField | null; note: string | null; url: string | null }

export function accessOf(a: AppView): Access {
  const l: LoginSpec = a.manifest.login ?? { mode: 'app' };
  const env = a.manifest.env ?? {};
  const field = (ref: string | null | undefined, label: string, secret: boolean): AccessField | null => {
    if (!ref) return null;
    if (ref in env) {
      const v = a.settings_values?.[ref];
      const masked = v === '••••••';
      return { label, env: ref, value: v && !masked ? v : undefined, secret: secret || masked || typeof env[ref] === 'object' };
    }
    return { label, value: ref, secret: false };
  };
  const path = l.path ? l.path.replace(/^\//, '') : '';
  return { mode: l.mode ?? 'app', user: field(l.user, t('access.user'), false), password: field(l.password, t('access.password'), true), note: l.note ?? null, url: a.url ? sameHost(a.url + path) : null };
}

export function modeText(mode: LoginSpec['mode']): string {
  switch (mode) {
    case 'generated': return t('access.modeGenerated');
    case 'default': return t('access.modeDefault');
    case 'none': return t('access.modeNone');
    case 'external': return t('access.modeExternal');
    default: return t('access.modeApp');
  }
}

/** Строка «Что понадобится» в карточке Store — из класса входа, обязательных настроек, capabilities и режима открытия. */
export function needsOf(a: AppView, catalog?: AppView[]): string[] {
  const out: string[] = [];
  const l = a.manifest.login;
  if (l?.mode === 'generated') out.push(t(l.user ? 'access.needGeneratedBoth' : 'access.needGenerated'));
  else if (l?.mode === 'default') out.push(t('access.needDefault'));
  else if (l?.mode === 'none') out.push(t('access.needNone'));
  else if (l?.mode === 'external') out.push(t('access.modeExternal'));
  else out.push(t('access.needApp'));
  const req = a.manifest.settings.filter(s => s.required);
  if (req.length) out.push(t('access.needSettings', { list: req.map(s => s.label ?? s.env).join(', ') }));
  for (const r of a.manifest.requires) {
    const prov = catalog?.find(c => c.manifest.provides.includes(r));
    out.push(t('access.needCap', { cap: r }) + (prov ? ` (${prov.manifest.title ?? prov.name})` : ''));
  }
  if (a.manifest.route.open === 'newtab') out.push(t('access.needNewtab'));
  if (a.manifest.host_docker_socket) out.push(t('access.needDocker'));
  return out;
}

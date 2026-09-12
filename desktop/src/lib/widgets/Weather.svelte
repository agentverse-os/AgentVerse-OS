<script lang="ts">
  import { t, i18n } from '../i18n.svelte';
  let { config, onconfig }: { config: Record<string, any>; onconfig: (p: Record<string, any>) => void } = $props();
  let data = $state<any>(null);
  let q = $state('');
  let found = $state<any[]>([]);
  let error = $state('');
  // коды погоды WMO: значок и ключ словаря с описанием
  const CODES: Record<number, [string, string]> = { 0: ['☀️', 'clear'], 1: ['🌤', 'mainlyClear'], 2: ['⛅', 'cloudy'], 3: ['☁️', 'overcast'], 45: ['🌫', 'fog'], 48: ['🌫', 'rime'], 51: ['🌦', 'drizzle'], 53: ['🌦', 'drizzle'], 55: ['🌧', 'drizzle'], 61: ['🌧', 'rain'], 63: ['🌧', 'rain'], 65: ['🌧', 'heavyRain'], 71: ['🌨', 'snow'], 73: ['🌨', 'snow'], 75: ['❄️', 'snowfall'], 80: ['🌦', 'showers'], 81: ['🌧', 'showers'], 82: ['⛈', 'showers'], 95: ['⛈', 'thunder'], 96: ['⛈', 'thunderHail'], 99: ['⛈', 'thunderHail'] };
  async function load() { if (!config.lat && !config.lon) return; try { data = await (await fetch(`/api/widgets/weather?lat=${config.lat}&lon=${config.lon}`)).json(); error = ''; } catch (e) { error = String(e); } }
  async function search() { try { const r = await (await fetch(`/api/widgets/geocode?q=${encodeURIComponent(q)}`)).json(); found = r.results ?? []; } catch (e) { error = String(e); } }
  $effect(() => { config.lat; config.lon; load(); const t = setInterval(load, 15 * 60 * 1000); return () => clearInterval(t); });
  const icon = (c: number) => { const e = CODES[c]; return e ? `${e[0]} ${t(`widgets.weather.${e[1]}`)}` : '🌡'; };
</script>
{#if !config.city || found.length}
  <div class="row"><input placeholder={t('widgets.weather.city')} bind:value={q} onkeydown={(e) => e.key === 'Enter' && search()} style="flex:1" /><button onclick={search}>{t('common.search')}</button></div>
  {#each found as f}<button class="pick" onclick={() => { onconfig({ city: `${f.name}${f.admin1 ? ', ' + f.admin1 : ''}, ${f.country}`, lat: f.latitude, lon: f.longitude }); found = []; }}>{f.name}{f.admin1 ? `, ${f.admin1}` : ''}, {f.country}</button>{/each}
{:else if data}
  <button class="w" ondblclick={() => found = [{ name: '' }]} title={t('widgets.weather.changeCity')}>
    <div class="now"><span class="t">{Math.round(data.current.temperature_2m)}°</span><span class="d">{icon(data.current.weather_code)}<br /><span class="muted small">{config.city}</span></span></div>
    <div class="days">{#each data.daily.time.slice(0, 5) as day, i}<div><span class="muted small">{new Date(day).toLocaleDateString(i18n.locale, { weekday: 'short' })}</span><span>{icon(data.daily.weather_code[i]).split(' ')[0]}</span><span class="small">{Math.round(data.daily.temperature_2m_max[i])}°/{Math.round(data.daily.temperature_2m_min[i])}°</span></div>{/each}</div>
    <div class="muted small">{t('widgets.weather.humidity', { n: data.current.relative_humidity_2m })} · {t('widgets.weather.wind', { n: Math.round(data.current.wind_speed_10m) })}</div>
  </button>
{:else}<div class="muted">{error || t('common.loading')}</div>{/if}
<style>
  .w { display: flex; flex-direction: column; gap: 8px; width: 100%; text-align: left; background: transparent; border-color: transparent; padding: 2px; }
  .now { display: flex; align-items: center; gap: 12px; } .t { font-size: 2.4rem; font-weight: 700; } .d { line-height: 1.3; }
  .days { display: grid; grid-template-columns: repeat(5, 1fr); gap: 4px; text-align: center; } .days div { display: flex; flex-direction: column; gap: 2px; }
  .small { font-size: .75rem; } .pick { text-align: left; }
</style>

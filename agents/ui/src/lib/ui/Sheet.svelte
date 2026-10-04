<script lang="ts">
  // Лист «Ещё» на телефоне: разделы, не поместившиеся в нижнюю панель, тема, палитра.
  import { ui } from '../state.svelte';
  import { prefs } from '../prefs.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  let { onclose, onpalette }: { onclose: () => void; onpalette: () => void } = $props();
</script>

<div class="sheet" role="dialog" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="box">
    <div class="grab"></div>
    <button class="mi" onclick={() => { ui.section = 'routines'; onclose(); }}><Icon name="repeat" /> {t('sec.routines')}</button>
    <button class="mi" onclick={() => { ui.section = 'settings'; onclose(); }}><Icon name="settings" /> {t('sec.settings')}</button>
    <button class="mi" onclick={() => { prefs.toggleTheme(); onclose(); }}><Icon name={prefs.effective === 'dark' ? 'sun' : 'moon'} /> {prefs.effective === 'dark' ? t('set.light') : t('set.dark')}</button>
    <button class="mi" onclick={() => { onclose(); onpalette(); }}><Icon name="search" /> {t('top.palette').split(' · ')[0]}</button>
  </div>
</div>

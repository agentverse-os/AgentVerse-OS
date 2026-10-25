<script lang="ts">
  import { SCHEMES, ACCENTS, TEXT_TONES, UI_FONTS, CODE_FONTS, type UiFont, type CodeFont, type TextWeight } from './appearance';
  import { prefs } from './prefs.svelte';
  import { t, LANGS, LANG_NAMES, type LangSetting } from './i18n';
  import Icon from './Icon.svelte';

  const MODES = ['dark', 'light', 'system'] as const;
  const DENSITIES = ['normal', 'compact'] as const;
  const selectedScheme = $derived(SCHEMES.find(s => s.id === prefs.scheme) ?? SCHEMES[0]);
  const uiFont = $derived(UI_FONTS.find(f => f.id === prefs.uiFont) ?? UI_FONTS[0]);
  const codeFont = $derived(CODE_FONTS.find(f => f.id === prefs.codeFont) ?? CODE_FONTS[0]);
  const storedTextColor = $derived(prefs.effective === 'dark' ? prefs.textDark : prefs.textLight);
  let textHex = $state('');
  let textHexError = $state(false);
  $effect(() => { textHex = storedTextColor; textHexError = false; });
  function saveTextColor(value: string) {
    prefs.set(prefs.effective === 'dark' ? 'textDark' : 'textLight', value.toLowerCase());
    textHex = storedTextColor; textHexError = false;
  }
  function commitTextHex() {
    const value = textHex.trim();
    if (!/^#[\da-f]{6}$/i.test(value)) { textHexError = true; return; }
    saveTextColor(value);
  }
</script>

<div class="card-s appearance" data-appearance>
  <div class="appearance-heading">
    <h3>{t('set.lookTitle')}</h3>
    <p>{t('look.intro')}</p>
  </div>

  <fieldset>
    <legend>{t('set.theme')}</legend>
    <div class="radio-options">
      {#each MODES as mode (mode)}
        <label class:selected={prefs.theme === mode}>
          <input type="radio" name="appearance-mode" value={mode} checked={prefs.theme === mode} onchange={() => prefs.set('theme', mode)} />
          {t(`set.${mode}`)}
        </label>
      {/each}
    </div>
  </fieldset>

  <fieldset>
    <legend>{t('look.scheme')}</legend>
    <p class="field-hint">{t('look.schemeHint')}</p>
    <div class="scheme-grid">
      {#each SCHEMES as scheme (scheme.id)}
        {@const colors = scheme[prefs.effective]}
        <button type="button" class="scheme-card" class:selected={prefs.scheme === scheme.id}
          data-scheme-card={scheme.id} aria-label={t('look.chooseScheme', { name: scheme.label })}
          aria-pressed={prefs.scheme === scheme.id} onclick={() => prefs.set('scheme', scheme.id)}>
          <span class="scheme-preview" aria-hidden="true"
            style="--preview-bg: {colors.bg}; --preview-panel: {colors.panel}; --preview-fg: {colors.fg}; --preview-accent: {colors.accent}">
            <span class="preview-sidebar"><i></i><i></i><i></i></span>
            <span class="preview-content"><i></i><i></i><i></i><i></i></span>
            <span class="preview-button"></span>
          </span>
          <span class="scheme-name"><span>{scheme.label}</span><span class="scheme-check" aria-hidden="true">{#if prefs.scheme === scheme.id}<Icon name="check" size="s" />{/if}</span></span>
          <span class="scheme-hint">{t(scheme.hint)}</span>
        </button>
      {/each}
    </div>
  </fieldset>

  <fieldset>
    <legend>{t('set.accent')}</legend>
    <div class="accent-options">
      <button type="button" class="accent-choice" class:selected={prefs.accent === 'scheme'}
        aria-pressed={prefs.accent === 'scheme'} onclick={() => prefs.set('accent', 'scheme')}>
        <span class="accent-dot" style:background={selectedScheme[prefs.effective].accent} aria-hidden="true"></span>
        {t('look.schemeAccent')}
      </button>
      {#each ACCENTS as accent (accent.id)}
        <button type="button" class="accent-choice" class:selected={prefs.accent === accent.id}
          aria-pressed={prefs.accent === accent.id} onclick={() => prefs.set('accent', accent.id)}>
          <span class="accent-dot" style:background={accent[prefs.effective]} aria-hidden="true"></span>
          {t(accent.key)}
        </button>
      {/each}
    </div>
  </fieldset>

  <fieldset class="text-settings">
    <legend>{t('look.textColor')}</legend>
    <div class="accent-options">
      {#each TEXT_TONES as tone (tone.id)}
        {@const color = tone.id === 'scheme' ? selectedScheme[prefs.effective].fg : tone.id === 'custom' ? storedTextColor : tone[prefs.effective]}
        <button type="button" class="accent-choice" class:selected={prefs.textTone === tone.id}
          data-text-tone={tone.id} aria-label={`${t('look.textColor')}: ${t(tone.key)}`}
          aria-pressed={prefs.textTone === tone.id} onclick={() => prefs.set('textTone', tone.id)}>
          <span class="accent-dot" style:background={color} aria-hidden="true"></span>
          {t(tone.key)}
        </button>
      {/each}
    </div>
    {#if prefs.textTone === 'custom'}
      <p class="field-hint custom-color-hint" id="appearance-text-mode">{t('look.textColorMode', { mode: t(`set.${prefs.effective}`) })}</p>
      <div class="custom-color-grid">
        <div class="color-field">
          <label for="appearance-text-color">{t('look.colorPicker')}</label>
          <input id="appearance-text-color" data-text-color type="color" value={storedTextColor}
            aria-describedby="appearance-text-mode" oninput={(event) => saveTextColor(event.currentTarget.value)} />
        </div>
        <div class="color-field">
          <label for="appearance-text-hex">{t('look.hexColor')}</label>
          <input id="appearance-text-hex" data-text-hex type="text" bind:value={textHex} maxlength="7"
            pattern="#[0-9a-fA-F]{6}" placeholder="#c7ccd6" autocomplete="off" spellcheck="false"
            aria-invalid={textHexError || undefined} aria-describedby={textHexError ? 'appearance-text-mode appearance-text-error' : 'appearance-text-mode'}
            oninput={() => { textHexError = false; }} onchange={commitTextHex} />
        </div>
      </div>
      {#if textHexError}<p class="hex-error" id="appearance-text-error" role="alert">{t('look.invalidHex')}</p>{/if}
    {/if}
    <p class="text-contrast" data-text-contrast>{t('look.contrast', { ratio: prefs.textContrast.toFixed(2) })}</p>
    {#if prefs.textTone === 'custom' && prefs.textAdjusted}
      <p class="field-hint adjusted-text" data-text-adjusted role="status">{t('look.textAdjusted', { color: prefs.appliedTextColor })}</p>
    {/if}
    <div class="text-preview">
      <strong>{t('look.readingTitle')}</strong>
      <p>{t('look.readingSample')}</p>
    </div>
  </fieldset>

  <div class="font-field weight-field">
    <label for="appearance-text-weight">{t('look.textWeight')}</label>
    <select id="appearance-text-weight" data-text-weight value={prefs.textWeight}
      onchange={(event) => prefs.set('textWeight', event.currentTarget.value as TextWeight)}>
      <option value="normal">{t('look.weightNormal')}</option>
      <option value="medium">{t('look.weightMedium')}</option>
    </select>
    <p class="field-hint">{t('look.readingHint')}</p>
  </div>

  <div class="font-grid">
    <div class="font-field">
      <label for="appearance-ui-font">{t('look.uiFont')}</label>
      <select id="appearance-ui-font" data-ui-font value={prefs.uiFont}
        onchange={(event) => prefs.set('uiFont', event.currentTarget.value as UiFont)}>
        {#each UI_FONTS as font (font.id)}<option value={font.id}>{t(font.label)}</option>{/each}
      </select>
      <p class="font-sample" style:font-family={uiFont.stack}>{t('look.textSample')}</p>
    </div>
    <div class="font-field">
      <label for="appearance-code-font">{t('look.codeFont')}</label>
      <select id="appearance-code-font" data-code-font value={prefs.codeFont}
        onchange={(event) => prefs.set('codeFont', event.currentTarget.value as CodeFont)}>
        {#each CODE_FONTS as font (font.id)}<option value={font.id}>{t(font.label)}</option>{/each}
      </select>
      <pre class="font-sample code-sample" style:font-family={codeFont.stack}><code>const message = 'Привет, world!';
console.log(message);</code></pre>
    </div>
  </div>
  <p class="field-hint">{t('look.fontHint')}</p>

  <fieldset>
    <legend>{t('set.density')}</legend>
    <div class="radio-options">
      {#each DENSITIES as density (density)}
        <label class:selected={prefs.density === density}>
          <input type="radio" name="appearance-density" value={density} checked={prefs.density === density} onchange={() => prefs.set('density', density)} />
          {t(`set.${density}`)}
        </label>
      {/each}
    </div>
  </fieldset>

  <fieldset>
    <legend>{t('set.lang')}</legend>
    <div class="radio-options language-options">
      <label class:selected={prefs.lang === 'auto'}>
        <input type="radio" name="appearance-language" value="auto" checked={prefs.lang === 'auto'} onchange={() => prefs.setLang('auto')} />
        {t('set.langAuto')}
      </label>
      {#each LANGS as language (language)}
        <label class:selected={prefs.lang === language}>
          <input type="radio" name="appearance-language" value={language} checked={prefs.lang === language} onchange={() => prefs.setLang(language as LangSetting)} />
          {LANG_NAMES[language]}
        </label>
      {/each}
    </div>
  </fieldset>

  <div class="appearance-footer">
    <p>{t('look.savedHere')}</p>
    <button type="button" class="reset-appearance" onclick={() => prefs.resetAppearance()}><Icon name="refresh" size="s" />{t('look.reset')}</button>
  </div>
</div>

<style>
  .appearance { min-width: 0; max-width: 820px; padding: 18px; display: grid; gap: 22px; }
  .appearance-heading { display: grid; gap: 6px; }
  .appearance-heading h3 { font-size: 16px; font-weight: 600; }
  .appearance-heading p, .field-hint, .appearance-footer p { color: var(--fg-3); font-size: 12.5px; }
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  legend, .font-field > label { padding: 0; font-size: 13px; font-weight: 600; color: var(--fg-1); margin-bottom: 8px; }
  .field-hint { margin: 0 0 10px; }
  .radio-options, .accent-options { display: flex; flex-wrap: wrap; gap: 8px; }
  .radio-options label { display: inline-flex; align-items: center; gap: 7px; min-height: 36px; padding: 6px 10px; cursor: pointer; border: 1px solid var(--line-2); border-radius: var(--r-2); background: var(--bg-3); }
  .radio-options label.selected { border-color: var(--accent); background: var(--accent-soft); }
  .radio-options input { margin: 0; flex: none; }
  .scheme-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .scheme-card { display: grid; height: auto; min-width: 0; padding: 8px; gap: 6px; align-content: start; text-align: left; white-space: normal; background: var(--bg-1); }
  .scheme-card.selected { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  .scheme-preview { position: relative; height: 68px; display: grid; grid-template-columns: 24px 1fr; gap: 8px; padding-right: 10px; border-radius: 5px; overflow: hidden; background: var(--preview-bg); }
  .preview-sidebar { display: flex; flex-direction: column; align-items: center; gap: 7px; padding: 10px 0; background: var(--preview-panel); }
  .preview-sidebar i { width: 6px; height: 6px; border-radius: 2px; background: var(--preview-fg); opacity: .5; }
  .preview-sidebar i:first-child { background: var(--preview-accent); opacity: 1; }
  .preview-content { display: grid; gap: 5px; align-content: start; padding-top: 10px; }
  .preview-content i { height: 4px; width: 82%; border-radius: 2px; background: var(--preview-fg); opacity: .6; }
  .preview-content i:first-child { width: 55%; height: 5px; background: var(--preview-accent); opacity: 1; }
  .preview-content i:last-child { width: 65%; }
  .preview-button { position: absolute; right: 10px; bottom: 8px; width: 20px; height: 9px; border-radius: 3px; background: var(--preview-accent); }
  .scheme-name { display: flex; align-items: center; justify-content: space-between; gap: 4px; min-width: 0; font-size: 13px; font-weight: 600; }
  .scheme-check { display: inline-flex; width: 14px; flex: none; color: var(--accent); }
  .scheme-hint { color: var(--fg-3); font-size: 11.5px; line-height: 1.4; overflow-wrap: anywhere; }
  .accent-choice { height: 36px; padding: 0 10px; white-space: normal; }
  .accent-choice.selected { border-color: var(--accent); background: var(--accent-soft); }
  .accent-dot { width: 12px; height: 12px; border-radius: 50%; flex: none; box-shadow: inset 0 0 0 1px rgb(0 0 0 / .12); }
  .text-settings .custom-color-hint { margin: 12px 0 8px; }
  .custom-color-grid { display: grid; grid-template-columns: 80px minmax(0, 1fr); gap: 12px; align-items: end; max-width: 360px; }
  .color-field { display: grid; gap: 6px; min-width: 0; }
  .color-field > label { font-size: 12px; color: var(--fg-2); overflow-wrap: anywhere; }
  .color-field input { width: 100%; }
  .color-field input[type="color"] { padding: 3px; cursor: pointer; }
  .color-field input[type="text"] { font-family: var(--mono); }
  .text-contrast { font-size: 12px; color: var(--fg-2); margin: 10px 0 0; }
  .hex-error { font-size: 12px; color: var(--bad); margin: 6px 0 0; }
  .adjusted-text { margin: 6px 0 0; }
  .text-preview { display: grid; gap: 6px; padding: 12px; margin-top: 10px; border: 1px solid var(--line-2); border-radius: var(--r-2); background: var(--bg-4); color: var(--fg-1); }
  .text-preview > strong { font-weight: 600; }
  .text-preview > p { font-size: 14px; line-height: 1.6; }
  .font-grid { display: grid; gap: 16px; }
  .font-field { display: grid; gap: 8px; min-width: 0; }
  .font-field > label { margin-bottom: 0; }
  .font-field select { width: 100%; }
  .weight-field select { max-width: 360px; }
  .weight-field .field-hint { margin-bottom: 0; }
  .font-sample { padding: 10px; border: 1px solid var(--line-2); border-radius: var(--r-2); background: var(--bg-4); color: var(--fg-1); font-size: 14px; overflow-wrap: anywhere; }
  .code-sample { margin: 0; font-size: 12px; line-height: 1.65; white-space: pre-wrap; }
  .code-sample code { font: inherit; }
  .font-grid + .field-hint { margin: -12px 0 0; }
  .appearance-footer { display: grid; gap: 10px; padding-top: 14px; border-top: 1px solid var(--line-2); }
  .reset-appearance { justify-self: start; height: auto; min-height: 32px; padding: 6px 10px; white-space: normal; text-align: left; }
  @media (min-width: 600px) { .scheme-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); } .font-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  @media (pointer: coarse) { .radio-options label, .accent-choice, .font-field select, .reset-appearance { min-height: 44px; } }
  @media (max-width: 599px) { .appearance { padding: 12px; gap: 20px; } .scheme-grid { gap: 8px; } .radio-options label { min-height: 44px; } }
</style>

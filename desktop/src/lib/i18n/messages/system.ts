// Окно «Система» (System.svelte): заголовки разделов. Заголовок окна и кнопки — sys.* из common.ts; статусы компонентов и события приходят из ядра и не переводятся.
import { defineMessages } from '../define';
export default defineMessages({
  en: { 'system.events': 'Events' },
  ru: { 'system.events': 'События' },
  uk: { 'system.events': 'Події' },
  es: { 'system.events': 'Eventos' },
});

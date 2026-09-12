// «Читалка»: подписи кнопок и состояний. Заголовок, источник, автор и текст статьи — контент ленты, не переводятся. Заголовок окна — sys.reader (common.ts).
import { defineMessages } from '../define';
export default defineMessages({
  en: {
    'reader.gone': 'this entry is no longer in the feed — open the original', 'reader.ago': '{ago} ago',
    'reader.summaryOnly': 'The feed carries only a summary; the full text is at the link below.',
    'reader.openOriginal': 'Open original ↗', 'reader.copyLink': 'Copy link', 'reader.loading': 'loading…',
  },
  ru: {
    'reader.gone': 'записи больше нет в ленте — откройте оригинал', 'reader.ago': '{ago} назад',
    'reader.summaryOnly': 'В ленте только анонс, полный текст по ссылке ниже.',
    'reader.openOriginal': 'Открыть оригинал ↗', 'reader.copyLink': 'Скопировать ссылку', 'reader.loading': 'загружаю…',
  },
  uk: {
    'reader.gone': 'запису більше немає в стрічці — відкрийте оригінал', 'reader.ago': '{ago} тому',
    'reader.summaryOnly': 'У стрічці лише анонс, повний текст за посиланням нижче.',
    'reader.openOriginal': 'Відкрити оригінал ↗', 'reader.copyLink': 'Скопіювати посилання', 'reader.loading': 'завантажую…',
  },
  es: {
    'reader.gone': 'la entrada ya no está en la fuente: abra el original', 'reader.ago': 'hace {ago}',
    'reader.summaryOnly': 'La fuente solo trae un resumen; el texto completo está en el enlace de abajo.',
    'reader.openOriginal': 'Abrir el original ↗', 'reader.copyLink': 'Copiar enlace', 'reader.loading': 'cargando…',
  },
});

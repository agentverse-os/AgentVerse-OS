import { defineMessages } from '../define';

// Own session name (docs/agents-ux.md §4.8): the menu item and the
// dialog
export default defineMessages({
  en: { 'sess.menu.rename': 'Rename…', 'rename.title': 'Session name', 'rename.auto': 'auto name: «{title}»', 'rename.hint': 'Leave it empty to go back to the automatic name. An emoji at the start makes the session easy to spot.', 'rename.reset': 'auto name', 'rename.cancel': 'Cancel', 'insp.autoTitle': 'auto name' },
  ru: { 'sess.menu.rename': 'Переименовать…', 'rename.title': 'Название сессии', 'rename.auto': 'автоназвание: «{title}»', 'rename.hint': 'Пусто — вернуть автоматическое название. Эмодзи в начале делает сессию заметной.', 'rename.reset': 'автоназвание', 'rename.cancel': 'Отмена', 'insp.autoTitle': 'автоназвание' },
  uk: { 'sess.menu.rename': 'Перейменувати…', 'rename.title': 'Назва сесії', 'rename.auto': 'автоназва: «{title}»', 'rename.hint': 'Порожньо — повернути автоматичну назву. Емодзі на початку робить сесію помітною.', 'rename.reset': 'автоназва', 'rename.cancel': 'Скасувати', 'insp.autoTitle': 'автоназва' },
  es: { 'sess.menu.rename': 'Renombrar…', 'rename.title': 'Nombre de la sesión', 'rename.auto': 'nombre automático: «{title}»', 'rename.hint': 'Déjalo vacío para volver al nombre automático. Un emoji al principio hace la sesión fácil de distinguir.', 'rename.reset': 'nombre automático', 'rename.cancel': 'Cancelar', 'insp.autoTitle': 'nombre automático' },
});

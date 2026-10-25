// The "System" window (System.svelte): section headings. The window title and buttons — sys.* from common.ts; component statuses and events come from the core and
// are not translated.
import { defineMessages } from '../define';
export default defineMessages({
  en: { 'system.usage': 'LLM spend', 'system.usage30': 'over 30 days', 'system.usageTurns': '{n} turns', 'system.usageTurnsShort': 'turns', 'system.usageProject': 'project', 'system.usageModel': 'model', 'system.usageDay': 'day', 'system.usageEmpty': 'No agent turns recorded yet — start a session in Agents.', 'system.events': 'Events' },
  ru: { 'system.usage': 'Расход LLM', 'system.usage30': 'за 30 дней', 'system.usageTurns': '{n} ходов', 'system.usageTurnsShort': 'ходов', 'system.usageProject': 'проект', 'system.usageModel': 'модель', 'system.usageDay': 'день', 'system.usageEmpty': 'Ходов агентов пока не записано — запусти сессию в «Агентах».', 'system.events': 'События' },
  uk: { 'system.usage': 'Витрати LLM', 'system.usage30': 'за 30 днів', 'system.usageTurns': '{n} ходів', 'system.usageTurnsShort': 'ходів', 'system.usageProject': 'проєкт', 'system.usageModel': 'модель', 'system.usageDay': 'день', 'system.usageEmpty': 'Ходів агентів ще не записано — запусти сесію в «Агентах».', 'system.events': 'Події' },
  es: { 'system.usage': 'Gasto LLM', 'system.usage30': 'en 30 días', 'system.usageTurns': '{n} turnos', 'system.usageTurnsShort': 'turnos', 'system.usageProject': 'proyecto', 'system.usageModel': 'modelo', 'system.usageDay': 'día', 'system.usageEmpty': 'Aún no hay turnos de agentes: inicia una sesión en Agentes.', 'system.events': 'Eventos' },
});

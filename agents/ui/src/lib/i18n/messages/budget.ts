import { defineMessages } from '../define';

// Spending limits in action (docs/agents-ux.md §8.3, slice 67): the bar in the composer and the button
// in "Models"
export default defineMessages({
  en: { 'budget.warn': '{pct}% of the daily limit for {scope} is spent: {spent} of {limit} over 24 h', 'budget.stopped': 'Daily limit for {scope} reached: {spent} of {limit} over 24 h — new turns are stopped until the spend drops', 'budget.open': 'Models', 'models.window24': 'over the last 24 hours' },
  ru: { 'budget.warn': 'израсходовано {pct}% дневного лимита {scope}: {spent} из {limit} за 24 ч', 'budget.stopped': 'дневной лимит {scope} исчерпан: {spent} из {limit} за 24 ч — новые ходы остановлены, пока расход не снизится', 'budget.open': 'Модели', 'models.window24': 'за последние 24 часа' },
  uk: { 'budget.warn': 'витрачено {pct}% денного ліміту {scope}: {spent} із {limit} за 24 год', 'budget.stopped': 'денний ліміт {scope} вичерпано: {spent} із {limit} за 24 год — нові ходи зупинені, доки витрати не знизяться', 'budget.open': 'Моделі', 'models.window24': 'за останні 24 години' },
  es: { 'budget.warn': 'gastado el {pct}% del límite diario de {scope}: {spent} de {limit} en 24 h', 'budget.stopped': 'límite diario de {scope} alcanzado: {spent} de {limit} en 24 h — los nuevos turnos están detenidos hasta que baje el gasto', 'budget.open': 'Modelos', 'models.window24': 'en las últimas 24 horas' },
});

import { defineMessages } from '../define';

// Honest runs of routines (docs/agents-ux.md §7.2): the owner's time zone, retries, overlap, unattended questions,
// skills proposed by an agent
export default defineMessages({
  en: {
    'rr.zone': 'time: {zone} (UTC{offset})', 'rr.zoneServer': 'server time (UTC{offset})', 'rr.zoneSwitch': 'switch to {zone}', 'rr.zoneSet': 'Schedules now run in {zone}',
    'rr.retries': 'on a provider failure', 'rr.retriesOn': 'retry in 5, 15 and 30 minutes', 'rr.overlap': 'previous run still going', 'rr.overlapSkip': 'skip the run', 'rr.overlapParallel': 'start next to it',
    'rr.question': 'the agent asks for a permission', 'rr.questionWait': 'wait for the answer in Inbox', 'rr.questionDeny': 'refuse and go on',
    'rr.attempt': 'retry {n}', 'rr.retryAt': 'retry at {t}', 'rr.rotate': 'new address', 'rr.rotateConfirm': 'Change the webhook address? The old one stops working at once.', 'rr.rotated': 'The webhook address was changed',
    'rr.accept': 'Accept the skill', 'rr.acceptHint': 'Proposed by the agent “{name}”. Until it is accepted, it works only in that agent’s own tasks.', 'rr.acceptHintGone': 'Proposed by an agent that no longer exists. Until it is accepted, nobody uses it.', 'rr.accepted': 'The skill is accepted',
  },
  ru: {
    'rr.zone': 'время: {zone} (UTC{offset})', 'rr.zoneServer': 'время сервера (UTC{offset})', 'rr.zoneSwitch': 'сменить на {zone}', 'rr.zoneSet': 'Расписания теперь идут по времени {zone}',
    'rr.retries': 'при сбое провайдера', 'rr.retriesOn': 'повторить через 5, 15 и 30 минут', 'rr.overlap': 'предыдущий ещё идёт', 'rr.overlapSkip': 'пропустить запуск', 'rr.overlapParallel': 'запускать рядом',
    'rr.question': 'агент спросил разрешение', 'rr.questionWait': 'ждать ответа во «Входящих»', 'rr.questionDeny': 'отказать и продолжить',
    'rr.attempt': 'повтор {n}', 'rr.retryAt': 'повтор в {t}', 'rr.rotate': 'сменить адрес', 'rr.rotateConfirm': 'Сменить адрес вебхука? Прежний перестанет работать сразу.', 'rr.rotated': 'Адрес вебхука сменён',
    'rr.accept': 'Принять навык', 'rr.acceptHint': 'Навык предложил агент «{name}». Пока навык не принят, он действует только в заданиях этого агента.', 'rr.acceptHintGone': 'Навык предложил агент, которого уже нет. Пока навык не принят, им никто не пользуется.', 'rr.accepted': 'Навык принят',
  },
  uk: {
    'rr.zone': 'час: {zone} (UTC{offset})', 'rr.zoneServer': 'час сервера (UTC{offset})', 'rr.zoneSwitch': 'змінити на {zone}', 'rr.zoneSet': 'Розклади тепер ідуть за часом {zone}',
    'rr.retries': 'у разі збою провайдера', 'rr.retriesOn': 'повторити через 5, 15 і 30 хвилин', 'rr.overlap': 'попередній ще триває', 'rr.overlapSkip': 'пропустити запуск', 'rr.overlapParallel': 'запускати поруч',
    'rr.question': 'агент запитав дозвіл', 'rr.questionWait': 'чекати відповіді у «Вхідних»', 'rr.questionDeny': 'відмовити й продовжити',
    'rr.attempt': 'повтор {n}', 'rr.retryAt': 'повтор о {t}', 'rr.rotate': 'змінити адресу', 'rr.rotateConfirm': 'Змінити адресу вебхука? Попередня перестане працювати одразу.', 'rr.rotated': 'Адресу вебхука змінено',
    'rr.accept': 'Прийняти навичку', 'rr.acceptHint': 'Навичку запропонував агент «{name}». Доки її не прийнято, вона діє лише в завданнях цього агента.', 'rr.acceptHintGone': 'Навичку запропонував агент, якого вже немає. Доки її не прийнято, нею ніхто не користується.', 'rr.accepted': 'Навичку прийнято',
  },
  es: {
    'rr.zone': 'hora: {zone} (UTC{offset})', 'rr.zoneServer': 'hora del servidor (UTC{offset})', 'rr.zoneSwitch': 'cambiar a {zone}', 'rr.zoneSet': 'Las programaciones ahora siguen la hora de {zone}',
    'rr.retries': 'si falla el proveedor', 'rr.retriesOn': 'reintentar en 5, 15 y 30 minutos', 'rr.overlap': 'la ejecución anterior sigue', 'rr.overlapSkip': 'omitir la ejecución', 'rr.overlapParallel': 'iniciar en paralelo',
    'rr.question': 'el agente pide un permiso', 'rr.questionWait': 'esperar la respuesta en Entrada', 'rr.questionDeny': 'denegar y continuar',
    'rr.attempt': 'reintento {n}', 'rr.retryAt': 'reintento a las {t}', 'rr.rotate': 'cambiar dirección', 'rr.rotateConfirm': '¿Cambiar la dirección del webhook? La anterior dejará de funcionar de inmediato.', 'rr.rotated': 'La dirección del webhook se cambió',
    'rr.accept': 'Aceptar la habilidad', 'rr.acceptHint': 'La propuso el agente «{name}». Hasta que se acepte, solo funciona en las tareas de ese agente.', 'rr.acceptHintGone': 'La propuso un agente que ya no existe. Hasta que se acepte, nadie la usa.', 'rr.accepted': 'La habilidad fue aceptada',
  },
});

import { defineMessages } from '../define';

// PWA to the max (docs/agents-ux.md §9.9): push to the device, share_target, outbox without a network, storage, "Share"
export default defineMessages({
  en: {
    'push.title': 'Push to this device', 'push.hint': 'The installed app receives permission requests, finished turns, errors and spent limits directly, with Allow/Deny right in the notification. ntfy keeps working as a second channel.',
    'push.unsupported': 'This browser cannot receive push. On iPhone it works only for the app added to the Home Screen (iOS 16.4+).', 'push.on': 'push is on for this device', 'push.off': 'push is off on this device', 'push.deniedShort': 'notifications are blocked in the browser',
    'push.enable': 'Enable on this device', 'push.disable': 'Disable', 'push.test': 'Send a test', 'push.denied': 'The browser refused notifications — allow them in the site settings and try again', 'push.enabled': 'Push enabled on this device', 'push.disabled': 'Push disabled on this device',
    'push.testSent': 'Test sent to {n} device(s)', 'push.testNone': 'Nothing sent', 'push.kinds': 'What to send', 'push.kindPermission': 'permission requests (with Allow / Deny)', 'push.kindDone': 'finished turns', 'push.kindError': 'errors', 'push.kindLimit': 'spent daily limits',
    'push.devices': 'devices subscribed: {n}', 'push.thisDevice': 'this device', 'push.fails': 'failures: {n}', 'push.drop': 'remove', 'push.ios': 'iPhone / iPad: install the app from the Home Screen first, then enable push here.',
    'share.pick': 'Shared content is waiting — open a conversation, it will land in the draft', 'share.gone': 'Nothing to share: the item was taken or expired', 'share.added': 'Shared content added to the draft',
    'outbox.waiting': 'waiting for network: {n} message(s)', 'outbox.sent': 'Sent from the outbox: {n}', 'outbox.drop': 'remove', 'outbox.queued': 'No connection — the message will go out when the network is back', 'outbox.failed': 'rejected: {err}',
    'set.storage': 'storage: {used} of {quota}', 'set.storagePersistent': 'persistent', 'set.storageBestEffort': 'may be cleared by the browser', 'set.storageAsk': 'make persistent',
    'chat.share': 'Share', 'chat.shared': 'Shared', 'set.colWindow': 'context window', 'api.timeout': 'The request timed out', 'cmp.sendUnconfirmed': 'The send was not confirmed in 30 s — check the conversation before sending again; the draft is kept.',
  },
  ru: {
    'push.title': 'Push на это устройство', 'push.hint': 'Установленное приложение само получает запросы разрешений, завершённые ходы, ошибки и исчерпанные лимиты — с кнопками «Разрешить / Отклонить» прямо в уведомлении. ntfy остаётся вторым каналом.',
    'push.unsupported': 'Этот браузер не принимает push. На iPhone это работает только у приложения с домашнего экрана (iOS 16.4+).', 'push.on': 'push включён на этом устройстве', 'push.off': 'push на этом устройстве выключен', 'push.deniedShort': 'уведомления запрещены в браузере',
    'push.enable': 'Включить на этом устройстве', 'push.disable': 'Отключить', 'push.test': 'Проверить', 'push.denied': 'Браузер отказал в уведомлениях — разрешите их в настройках сайта и повторите', 'push.enabled': 'Push включён на этом устройстве', 'push.disabled': 'Push на этом устройстве выключен',
    'push.testSent': 'Тест отправлен: устройств — {n}', 'push.testNone': 'Ничего не отправлено', 'push.kinds': 'Что присылать', 'push.kindPermission': 'запросы разрешений (с «Разрешить / Отклонить»)', 'push.kindDone': 'завершённые ходы', 'push.kindError': 'ошибки', 'push.kindLimit': 'исчерпанные дневные лимиты',
    'push.devices': 'подписанных устройств: {n}', 'push.thisDevice': 'это устройство', 'push.fails': 'неудач: {n}', 'push.drop': 'убрать', 'push.ios': 'iPhone / iPad: сначала установите приложение с домашнего экрана, потом включите push здесь.',
    'share.pick': 'Поделённое ждёт — откройте беседу, оно ляжет в черновик', 'share.gone': 'Нечем поделиться: запись забрана или устарела', 'share.added': 'Поделённое добавлено в черновик',
    'outbox.waiting': 'ждёт сети: {n} сообщ.', 'outbox.sent': 'Отправлено из исходящих: {n}', 'outbox.drop': 'убрать', 'outbox.queued': 'Нет связи — сообщение уйдёт, когда сеть вернётся', 'outbox.failed': 'не принято: {err}',
    'set.storage': 'хранилище: {used} из {quota}', 'set.storagePersistent': 'постоянное', 'set.storageBestEffort': 'браузер может вычистить', 'set.storageAsk': 'сделать постоянным',
    'chat.share': 'Поделиться', 'chat.shared': 'Отправлено', 'set.colWindow': 'окно контекста', 'api.timeout': 'Запрос не дождался ответа', 'cmp.sendUnconfirmed': 'Отправка не подтвердилась за 30 с — проверьте ленту, прежде чем отправлять снова; черновик сохранён.',
  },
  uk: {
    'push.title': 'Push на цей пристрій', 'push.hint': 'Встановлений застосунок сам отримує запити дозволів, завершені ходи, помилки та вичерпані ліміти — з кнопками «Дозволити / Відхилити» просто в сповіщенні. ntfy лишається другим каналом.',
    'push.unsupported': 'Цей браузер не приймає push. На iPhone це працює лише у застосунку з домашнього екрана (iOS 16.4+).', 'push.on': 'push увімкнено на цьому пристрої', 'push.off': 'push на цьому пристрої вимкнено', 'push.deniedShort': 'сповіщення заборонені в браузері',
    'push.enable': 'Увімкнути на цьому пристрої', 'push.disable': 'Вимкнути', 'push.test': 'Перевірити', 'push.denied': 'Браузер відмовив у сповіщеннях — дозвольте їх у налаштуваннях сайту і повторіть', 'push.enabled': 'Push увімкнено на цьому пристрої', 'push.disabled': 'Push на цьому пристрої вимкнено',
    'push.testSent': 'Тест надіслано: пристроїв — {n}', 'push.testNone': 'Нічого не надіслано', 'push.kinds': 'Що надсилати', 'push.kindPermission': 'запити дозволів (з «Дозволити / Відхилити»)', 'push.kindDone': 'завершені ходи', 'push.kindError': 'помилки', 'push.kindLimit': 'вичерпані денні ліміти',
    'push.devices': 'підписаних пристроїв: {n}', 'push.thisDevice': 'цей пристрій', 'push.fails': 'невдач: {n}', 'push.drop': 'прибрати', 'push.ios': 'iPhone / iPad: спочатку встановіть застосунок з домашнього екрана, потім увімкніть push тут.',
    'share.pick': 'Поділене чекає — відкрийте бесіду, воно ляже в чернетку', 'share.gone': 'Нічим ділитися: запис забрано або він застарів', 'share.added': 'Поділене додано до чернетки',
    'outbox.waiting': 'чекає мережі: {n} повід.', 'outbox.sent': 'Надіслано з вихідних: {n}', 'outbox.drop': 'прибрати', 'outbox.queued': 'Немає зв’язку — повідомлення піде, коли мережа повернеться', 'outbox.failed': 'не прийнято: {err}',
    'set.storage': 'сховище: {used} з {quota}', 'set.storagePersistent': 'постійне', 'set.storageBestEffort': 'браузер може вичистити', 'set.storageAsk': 'зробити постійним',
    'chat.share': 'Поділитися', 'chat.shared': 'Надіслано', 'set.colWindow': 'вікно контексту', 'api.timeout': 'Запит не дочекався відповіді', 'cmp.sendUnconfirmed': 'Надсилання не підтвердилося за 30 с — перевірте стрічку, перш ніж надсилати знову; чернетку збережено.',
  },
  es: {
    'push.title': 'Push a este dispositivo', 'push.hint': 'La app instalada recibe por sí sola las solicitudes de permiso, los turnos terminados, los errores y los límites agotados, con «Permitir / Rechazar» en la propia notificación. ntfy sigue como segundo canal.',
    'push.unsupported': 'Este navegador no recibe push. En iPhone solo funciona para la app añadida a la pantalla de inicio (iOS 16.4+).', 'push.on': 'push activado en este dispositivo', 'push.off': 'push desactivado en este dispositivo', 'push.deniedShort': 'las notificaciones están bloqueadas en el navegador',
    'push.enable': 'Activar en este dispositivo', 'push.disable': 'Desactivar', 'push.test': 'Enviar prueba', 'push.denied': 'El navegador rechazó las notificaciones: permítelas en los ajustes del sitio e inténtalo de nuevo', 'push.enabled': 'Push activado en este dispositivo', 'push.disabled': 'Push desactivado en este dispositivo',
    'push.testSent': 'Prueba enviada a {n} dispositivo(s)', 'push.testNone': 'No se envió nada', 'push.kinds': 'Qué enviar', 'push.kindPermission': 'solicitudes de permiso (con Permitir / Rechazar)', 'push.kindDone': 'turnos terminados', 'push.kindError': 'errores', 'push.kindLimit': 'límites diarios agotados',
    'push.devices': 'dispositivos suscritos: {n}', 'push.thisDevice': 'este dispositivo', 'push.fails': 'fallos: {n}', 'push.drop': 'quitar', 'push.ios': 'iPhone / iPad: instala primero la app desde la pantalla de inicio y luego activa el push aquí.',
    'share.pick': 'Lo compartido espera: abre una conversación y quedará en el borrador', 'share.gone': 'Nada que compartir: el elemento ya se tomó o caducó', 'share.added': 'Lo compartido se añadió al borrador',
    'outbox.waiting': 'esperando red: {n} mensaje(s)', 'outbox.sent': 'Enviado desde la bandeja de salida: {n}', 'outbox.drop': 'quitar', 'outbox.queued': 'Sin conexión: el mensaje saldrá cuando vuelva la red', 'outbox.failed': 'rechazado: {err}',
    'set.storage': 'almacenamiento: {used} de {quota}', 'set.storagePersistent': 'persistente', 'set.storageBestEffort': 'el navegador puede borrarlo', 'set.storageAsk': 'hacer persistente',
    'chat.share': 'Compartir', 'chat.shared': 'Compartido', 'set.colWindow': 'ventana de contexto', 'api.timeout': 'La solicitud no recibió respuesta a tiempo', 'cmp.sendUnconfirmed': 'El envío no se confirmó en 30 s: revisa la conversación antes de reenviar; el borrador se conserva.',
  },
});

// Сообщения клиента API (lib/api.ts): ошибка связи с ядром и этапы ожидания установки/удаления, которые формирует сам фронт.
// Тексты ядра (события, ошибки сервера, состояния) сюда не входят — показываются как есть.
import { defineMessages } from '../define';
export default defineMessages({
  en: {
    'api.offline': 'No connection to the cloudd core: check that the VPN client (Tailscale or NetBird) is connected and the host is reachable. Retrying automatically. If the site was opened with a certificate exception (the “not secure” badge), the exception may have been reset: reload the page and accept the certificate again, or install the AgentVerse OS root certificate (Settings → Devices).',
    'api.waitingCore': 'waiting for the core…', 'api.installing': 'installing…', 'api.startingState': 'starting ({state})',
    'api.installTimeout': 'installation did not finish in 5 minutes — check the app logs', 'api.removeTimeout': 'removal did not finish in 3 minutes — check the event feed',
  },
  ru: {
    'api.offline': 'Нет связи с ядром cloudd: проверьте, что подключён VPN-клиент (Tailscale или NetBird) и хост доступен. Повтор автоматически. Если сайт открыт с исключением для сертификата (значок «не защищено»), исключение могло сброситься: обновите страницу и подтвердите сертификат снова или установите корневой сертификат AgentVerse OS (Настройки → Устройства).',
    'api.waitingCore': 'жду ответ ядра…', 'api.installing': 'устанавливаю…', 'api.startingState': 'запускается ({state})',
    'api.installTimeout': 'установка не завершилась за 5 минут — смотрите логи приложения', 'api.removeTimeout': 'удаление не завершилось за 3 минуты — смотрите ленту событий',
  },
  uk: {
    'api.offline': 'Немає зв’язку з ядром cloudd: перевірте, що підключено VPN-клієнт (Tailscale або NetBird) і хост доступний. Повтор автоматично. Якщо сайт відкрито з винятком для сертифіката (значок «не захищено»), виняток міг скинутися: оновіть сторінку та підтвердьте сертифікат знову або встановіть кореневий сертифікат AgentVerse OS (Налаштування → Пристрої).',
    'api.waitingCore': 'чекаю відповіді ядра…', 'api.installing': 'встановлюю…', 'api.startingState': 'запускається ({state})',
    'api.installTimeout': 'встановлення не завершилося за 5 хвилин — дивіться логи застосунку', 'api.removeTimeout': 'видалення не завершилося за 3 хвилини — дивіться стрічку подій',
  },
  es: {
    'api.offline': 'Sin conexión con el núcleo cloudd: compruebe que el cliente VPN (Tailscale o NetBird) está conectado y que el host es accesible. Se reintenta automáticamente. Si el sitio se abrió con una excepción de certificado (el icono «no es seguro»), la excepción puede haberse restablecido: recargue la página y acepte el certificado de nuevo, o instale el certificado raíz de AgentVerse OS (Ajustes → Dispositivos).',
    'api.waitingCore': 'esperando al núcleo…', 'api.installing': 'instalando…', 'api.startingState': 'iniciando ({state})',
    'api.installTimeout': 'la instalación no terminó en 5 minutos — revise los registros de la aplicación', 'api.removeTimeout': 'la eliminación no terminó en 3 minutos — revise la lista de eventos',
  },
});

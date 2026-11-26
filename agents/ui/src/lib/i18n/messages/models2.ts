import { defineMessages } from '../define';

// Gate catalog grouping (2026-10-26): channels → families → models in the composer menu; speech, image and special-purpose
// models are folded behind one item; kind words for hints
export default defineMessages({
  en: {
    'cmp.gateSpecialShow': 'Special models · {n}', 'cmp.gateSpecialHide': 'Hide special models', 'cmp.gateSpecialHint': 'speech, images, video, embeddings, previews for robotics and computer use — not for coding agents',
    'cmp.kind.image': 'images', 'cmp.kind.video': 'video', 'cmp.kind.tts': 'speech synthesis', 'cmp.kind.audio': 'audio / transcription', 'cmp.kind.embedding': 'embeddings', 'cmp.kind.special': 'special purpose',
    'prices.noPriceN': 'no price: {n}',
  },
  ru: {
    'cmp.gateSpecialShow': 'Служебные модели · {n}', 'cmp.gateSpecialHide': 'Скрыть служебные модели', 'cmp.gateSpecialHint': 'речь, картинки, видео, эмбеддинги, превью для роботики и управления компьютером — не для агентов-кодеров',
    'cmp.kind.image': 'картинки', 'cmp.kind.video': 'видео', 'cmp.kind.tts': 'синтез речи', 'cmp.kind.audio': 'аудио / транскрипция', 'cmp.kind.embedding': 'эмбеддинги', 'cmp.kind.special': 'специального назначения',
    'prices.noPriceN': 'нет цены: {n}',
  },
  uk: {
    'cmp.gateSpecialShow': 'Службові моделі · {n}', 'cmp.gateSpecialHide': 'Сховати службові моделі', 'cmp.gateSpecialHint': 'мовлення, зображення, відео, ембедінги, прев’ю для робототехніки й керування комп’ютером — не для агентів-кодерів',
    'cmp.kind.image': 'зображення', 'cmp.kind.video': 'відео', 'cmp.kind.tts': 'синтез мовлення', 'cmp.kind.audio': 'аудіо / транскрипція', 'cmp.kind.embedding': 'ембедінги', 'cmp.kind.special': 'спеціального призначення',
    'prices.noPriceN': 'немає ціни: {n}',
  },
  es: {
    'cmp.gateSpecialShow': 'Modelos especiales · {n}', 'cmp.gateSpecialHide': 'Ocultar los modelos especiales', 'cmp.gateSpecialHint': 'voz, imágenes, vídeo, embeddings, vistas previas para robótica y uso del ordenador: no son para agentes de código',
    'cmp.kind.image': 'imágenes', 'cmp.kind.video': 'vídeo', 'cmp.kind.tts': 'síntesis de voz', 'cmp.kind.audio': 'audio / transcripción', 'cmp.kind.embedding': 'embeddings', 'cmp.kind.special': 'uso especial',
    'prices.noPriceN': 'sin precio: {n}',
  },
});

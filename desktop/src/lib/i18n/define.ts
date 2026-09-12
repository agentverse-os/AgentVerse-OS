// Контракт словарей локализации: один файл в messages/ — одно пространство имён с четырьмя языками.
// Английский задаёт набор ключей, русский/украинский/испанский проверяются по нему типом: лишний или пропущенный ключ — ошибка компиляции.
// Русские строки — исходные тексты интерфейса (по ним ищут элементы e2e-тесты), английский — язык по умолчанию для неизвестного языка браузера.
export type Lang = 'en' | 'ru' | 'uk' | 'es';
/** Настройка языка в оформлении: auto — по языку браузера. */
export type LangSetting = 'auto' | Lang;
export const LANGS: readonly Lang[] = ['en', 'ru', 'uk', 'es'];

export type Messages<T extends Record<string, string>> = { en: T; ru: { [K in keyof T]: string }; uk: { [K in keyof T]: string }; es: { [K in keyof T]: string } };
/** Ключи — `<пространство>.<имяCamelCase>`; подстановки — `{name}`; множественное число — четыре ключа `.one/.few/.many/.other` во всех языках (см. tn). */
export function defineMessages<T extends Record<string, string>>(m: Messages<T>): Messages<T> { return m; }

// Contract of the localization dictionaries: one file in messages/ is one namespace with four languages. English defines the key set,
// Russian/Ukrainian/Spanish are type-checked against it: an extra or missing key is a compile error. The Russian strings are the source texts
// of the UI (the e2e tests look up elements by them), English is the default for an unknown browser language.
export type Lang = 'en' | 'ru' | 'uk' | 'es';
/** Language setting in the appearance: auto — by the browser
 *  language. */
export type LangSetting = 'auto' | Lang;
export const LANGS: readonly Lang[] = ['en', 'ru', 'uk', 'es'];

export type Messages<T extends Record<string, string>> = { en: T; ru: { [K in keyof T]: string }; uk: { [K in keyof T]: string }; es: { [K in keyof T]: string } };
/** Keys — `<namespace>.<nameCamelCase>`; substitutions — `{name}`; plurals — four keys `.one/.few/.many/.other` in every language (see tn). */
export function defineMessages<T extends Record<string, string>>(m: Messages<T>): Messages<T> { return m; }
